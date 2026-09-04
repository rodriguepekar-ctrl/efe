// SPDX-License-Identifier: UNLICENSED
//! Contexte d'appel + writer d'audit minimal.
//!
//! En Phase 1 on n'a pas encore branché `efe-server::audit::hmac_chain`
//! (qui requiert Postgres en backend, prévu Phase 7).  À la place on
//! fournit un `InMemoryAuditWriter` qui implémente le trait `AuditWriter` ;
//! Phase 2 substituera la vraie chaîne HMAC sans changer la signature.

use std::collections::VecDeque;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::{RobotError, RobotResult};

/// Identifie une exécution complète de DAG (l'équivalent d'un `request_id` HTTP).
pub type SwarmId = Uuid;

/// Identifie un tenant (multi-tenancy par défaut dans EFE).
pub type TenantId = Uuid;

/// Tout ce qui doit être propagé à travers une chaîne d'invocations de robots.
///
/// Construit en haut de la pile (typiquement par le gateway ou le coordinator)
/// et passé en référence à chaque client.
#[derive(Clone)]
pub struct CallContext {
    /// UUID v7 du swarm — préserve l'ordre temporel + uniqueness global.
    pub swarm_id: SwarmId,
    /// Tenant propriétaire de l'invocation (pour isolation audit).
    pub tenant_id: TenantId,
    /// Writer pour audit chain (HMAC-chained en Phase 2+).
    pub audit: Arc<dyn AuditWriter>,
}

impl CallContext {
    /// Construit un contexte avec le writer fourni.
    #[must_use]
    pub fn new(tenant_id: TenantId, audit: Arc<dyn AuditWriter>) -> Self {
        Self {
            swarm_id: Uuid::now_v7(),
            tenant_id,
            audit,
        }
    }

    /// Variante avec swarm_id fourni (utile pour resume / replay).
    #[must_use]
    pub fn with_swarm(tenant_id: TenantId, swarm_id: SwarmId, audit: Arc<dyn AuditWriter>) -> Self {
        Self {
            swarm_id,
            tenant_id,
            audit,
        }
    }

    /// Contexte de test (tenant et swarm aléatoires, audit in-memory).
    #[must_use]
    pub fn mock() -> Self {
        Self {
            swarm_id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            audit: Arc::new(InMemoryAuditWriter::default()),
        }
    }
}

/// Une ligne d'audit produite à chaque invocation de robot.
///
/// Phase 1 : version minimale.  Phase 2 ajoutera `prev_hash` + `entry_hmac`
/// (chaînage cryptographique HMAC) en réutilisant `efe-server::audit::hmac_chain`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// UUID v7 de cette entrée précise.
    pub entry_id: Uuid,
    /// Swarm parent.
    pub swarm_id: SwarmId,
    /// Tenant propriétaire.
    pub tenant_id: TenantId,
    /// Robot impliqué (`analyzer`, `reformulator`, ...).
    pub robot_id: String,
    /// Version du schéma I/O utilisée.
    pub schema_version: String,
    /// BLAKE3 hex de l'input canonicalisé.
    pub input_hash: String,
    /// BLAKE3 hex de l'output canonicalisé.
    pub output_hash: String,
    /// Tokens du prompt (côté llama.cpp, 0 pour les robots ONNX).
    pub tokens_prompt: u32,
    /// Tokens générés (0 pour les robots ONNX).
    pub tokens_completion: u32,
    /// Latence end-to-end en millisecondes.
    pub latency_ms: u64,
    /// Statut (`success`, `failed`, `timeout`, `circuit_open`, ...).
    pub status: AuditStatus,
    /// Description d'erreur si `status != Success`.
    pub error_detail: Option<String>,
    /// Horodatage de fin (UTC).
    pub finished_at: chrono::DateTime<chrono::Utc>,
}

/// Ce qu'un appel réussi a produit — l'entrée d'audit avant qu'elle ne soit datée
/// et identifiée.
///
/// # Pourquoi une structure, et pas huit arguments
///
/// La signature remplacée alignait, dans cet ordre : `robot_id`,
/// `schema_version` (deux `&str` **voisins**), `input_hash`, `output_hash` (deux
/// `String` **voisines**), `tokens_prompt`, `tokens_completion` (deux `u32`
/// **voisins**), puis `latency_ms`. **Trois paires transposables sans que le
/// compilateur ne bronche.**
///
/// Ce n'est pas une gêne de lecture, c'est une faute d'archivage. Une entrée dont
/// les empreintes d'entrée et de sortie sont ÉCHANGÉES est un enregistrement faux
/// et parfaitement plausible : rien en aval ne peut le rattraper, puisque c'est
/// LUI la preuve. Aux six sites d'appel, deux passaient `0, 0, latency_ms` et un
/// autre `10, 20, 50` — trois entiers nus dont seule la position disait le sens.
///
/// La feuille de route du MVP classait ce défaut parmi les « lints de forme ».
/// Le compte d'arguments est une forme ; ce que ce compte cachait ici ne l'est pas.
pub struct SuccessRecord<'a> {
    /// Identité du robot appelé.
    pub robot_id: &'a str,
    /// Version du schéma de sortie qu'il a respecté.
    pub schema_version: &'a str,
    /// Empreinte de l'ENTRÉE.
    pub input_hash: String,
    /// Empreinte de la SORTIE.
    pub output_hash: String,
    /// Tokens consommés par le prompt.
    pub tokens_prompt: u32,
    /// Tokens générés.
    pub tokens_completion: u32,
    /// Latence bout-en-bout (ms).
    pub latency_ms: u64,
}

impl AuditEntry {
    /// Construit l'instant t une entry "success" depuis les paramètres typiques.
    #[must_use]
    pub fn success(ctx: &CallContext, r: SuccessRecord<'_>) -> Self {
        Self {
            entry_id: Uuid::now_v7(),
            swarm_id: ctx.swarm_id,
            tenant_id: ctx.tenant_id,
            robot_id: r.robot_id.to_string(),
            schema_version: r.schema_version.to_string(),
            input_hash: r.input_hash,
            output_hash: r.output_hash,
            tokens_prompt: r.tokens_prompt,
            tokens_completion: r.tokens_completion,
            latency_ms: r.latency_ms,
            status: AuditStatus::Success,
            error_detail: None,
            finished_at: chrono::Utc::now(),
        }
    }

    /// Construit une entry pour un appel qui a échoué.
    #[must_use]
    pub fn failure(
        ctx: &CallContext,
        robot_id: &str,
        schema_version: &str,
        input_hash: String,
        latency_ms: u64,
        err: &RobotError,
    ) -> Self {
        let status = match err {
            RobotError::Timeout { .. } => AuditStatus::Timeout,
            RobotError::CircuitOpen { .. } => AuditStatus::CircuitOpen,
            _ => AuditStatus::Failed,
        };
        Self {
            entry_id: Uuid::now_v7(),
            swarm_id: ctx.swarm_id,
            tenant_id: ctx.tenant_id,
            robot_id: robot_id.to_string(),
            schema_version: schema_version.to_string(),
            input_hash,
            output_hash: String::new(),
            tokens_prompt: 0,
            tokens_completion: 0,
            latency_ms,
            status,
            error_detail: Some(err.to_string()),
            finished_at: chrono::Utc::now(),
        }
    }
}

/// État final d'une invocation, pour analytics + alerting.
#[allow(missing_docs)] // each variant is self-explanatory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Success,
    Failed,
    Timeout,
    CircuitOpen,
}

/// Trait : tout ce qui peut récolter des `AuditEntry`.
///
/// Phase 1 : impl mémoire seul.  Phase 2 ajoutera `PostgresAuditWriter`
/// + `HmacChainWriter` qui chaîne cryptographiquement chaque entry.
#[async_trait]
pub trait AuditWriter: Send + Sync {
    /// Persiste l'entry.  Non-bloquant idéalement (queue interne).
    async fn record(&self, entry: AuditEntry) -> RobotResult<()>;

    /// Lecture (pour les tests + replay engine futur).
    async fn entries(&self) -> Vec<AuditEntry>;
}

/// Implémentation in-memory, suffisante pour Phase 1.
///
/// Stocke les entrées dans une `VecDeque` bornée (par défaut 100 K
/// pour éviter une OOM en stress test).
#[derive(Debug)]
pub struct InMemoryAuditWriter {
    inner: Mutex<VecDeque<AuditEntry>>,
    max_entries: usize,
}

impl InMemoryAuditWriter {
    /// Construit avec une capacité max donnée (default = `Self::default()`).
    #[must_use]
    pub fn with_capacity(max_entries: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(max_entries.min(1024))),
            max_entries,
        }
    }
}

impl Default for InMemoryAuditWriter {
    fn default() -> Self {
        Self::with_capacity(100_000)
    }
}

#[async_trait]
impl AuditWriter for InMemoryAuditWriter {
    async fn record(&self, entry: AuditEntry) -> RobotResult<()> {
        let mut guard = self.inner.lock().await;
        if guard.len() >= self.max_entries {
            guard.pop_front();
        }
        guard.push_back(entry);
        Ok(())
    }

    async fn entries(&self) -> Vec<AuditEntry> {
        let guard = self.inner.lock().await;
        guard.iter().cloned().collect()
    }
}

/// Helper : BLAKE3 hex d'un JSON canonicalisé.
///
/// Utilisé par tous les clients pour produire `input_hash` / `output_hash`.
#[must_use]
pub fn blake3_canonical_json<T: Serialize>(v: &T) -> String {
    let canonical = serde_json::to_vec(v).unwrap_or_default();
    blake3::hash(&canonical).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_writer_records_and_replays() {
        let writer = InMemoryAuditWriter::default();
        let ctx = CallContext::new(Uuid::now_v7(), Arc::new(InMemoryAuditWriter::default()));
        let entry = AuditEntry::success(
            &ctx,
            SuccessRecord {
                robot_id: "analyzer",
                schema_version: "1.0.0",
                input_hash: "abc".into(),
                output_hash: "def".into(),
                tokens_prompt: 10,
                tokens_completion: 20,
                latency_ms: 50,
            },
        );
        writer.record(entry.clone()).await.unwrap();

        let all = writer.entries().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].robot_id, "analyzer");
        assert_eq!(all[0].status, AuditStatus::Success);
    }

    #[tokio::test]
    async fn in_memory_writer_evicts_when_full() {
        let writer = InMemoryAuditWriter::with_capacity(3);
        let ctx = CallContext::mock();
        for i in 0..5 {
            let entry = AuditEntry::success(
                &ctx,
                SuccessRecord {
                    robot_id: "test",
                    schema_version: "1.0.0",
                    input_hash: format!("hash-{i}"),
                    output_hash: format!("out-{i}"),
                    tokens_prompt: 0,
                    tokens_completion: 0,
                    latency_ms: 0,
                },
            );
            writer.record(entry).await.unwrap();
        }
        let all = writer.entries().await;
        // capacité 3 → les 2 plus anciennes sont jetées.
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].input_hash, "hash-2");
        assert_eq!(all[2].input_hash, "hash-4");
    }

    #[test]
    fn blake3_canonical_is_deterministic() {
        let v1 = serde_json::json!({"a": 1, "b": 2});
        let v2 = serde_json::json!({"a": 1, "b": 2});
        assert_eq!(blake3_canonical_json(&v1), blake3_canonical_json(&v2));
        assert_eq!(blake3_canonical_json(&v1).len(), 64); // 32 bytes hex
    }
}
