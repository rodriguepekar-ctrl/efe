// SPDX-License-Identifier: UNLICENSED
//! `RunStore` — la **preuve durable**. On persiste les `AgentCallRecord` bruts (serde,
//! Phase 4) : l'évidence que `GET /v1/swarm/verify/:id` re-prouve. Les records sont
//! **immuables** (write-once) ; de toute façon, toute mutation est détectée par
//! `verify_swarm`.
//!
//! - **Inc.2** : [`InMemoryRunStore`] (suffisant pour la démo + le standalone souverain).
//! - **Prod** : `PgRunStore` (table `swarm_runs { id, tenant, records JSONB, root, created_at }`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

use efe_swarm_sdk::{AgentCallRecord, LlmProvenanceRecord, RunOutcome};

use crate::error::GatewayError;

/// Un run persisté — **tenant-scopé**. `records` = l'évidence re-vérifiable.
#[derive(Debug, Clone)]
pub struct StoredRun {
    pub tenant: String,
    /// Les `AgentCallRecord` chaînés (l'évidence pour `verify_swarm`).
    pub records: Vec<AgentCallRecord>,
    pub root: Option<String>,
    /// **ADR-016 — forensique N17 durable** : les `LlmProvenanceRecord` (un par appel
    /// LLM Mode C). Vide pour un run keyless/Mode A. Sérialisés serde (JSONB en prod).
    pub llm_records: Vec<LlmProvenanceRecord>,
    /// **ADR-016** : les feuilles SHA-256 de contenu (hex, ordre canonique) — la
    /// localisation d'un record muté à la re-preuve.
    pub n17_leaves: Vec<String>,
    /// **ADR-016** : `n17_root` (hex) — la racine de la chaîne forensique scellée.
    pub n17_root: Option<String>,
    pub output: Value,
    pub topology: String,
    pub created_at: SystemTime,
}

impl StoredRun {
    /// Construit depuis un [`RunOutcome`] du SDK. La forensique N17 — déjà **scellée**
    /// par le SDK avec la clé tenant (`outcome.n17_sealed`) — est **conservée** (ADR-016 :
    /// on ne la jette plus). Pour un run keyless/Mode A, elle est vide (no-op).
    pub fn from_outcome(tenant: &str, outcome: &RunOutcome) -> Self {
        Self {
            tenant: tenant.to_string(),
            records: outcome.records().to_vec(),
            root: outcome.run.provenance.root.clone(),
            llm_records: outcome.llm_records.clone(),
            n17_leaves: outcome.n17_sealed.leaves_hex(),
            n17_root: outcome.n17_sealed.root_hex(),
            output: outcome.run.output.clone(),
            topology: outcome.run.topology.clone(),
            created_at: SystemTime::now(),
        }
    }

    /// **Couche 2 (M2)** : construit un [`StoredRun`] depuis une **mission multi-LLM**. Le
    /// graphe d'`AgentCallRecord` et les chaînes (`provenance_root`, N17) sont **déjà scellés**
    /// par `mission_graph::build_and_seal_mission` (la mission EST l'exécuteur — pas de
    /// `RunOutcome`/`SwarmRun`/`SealedProvenance`). Constructeur direct, jumeau de `from_outcome`.
    #[allow(clippy::too_many_arguments)]
    pub fn from_mission(
        tenant: &str,
        records: Vec<AgentCallRecord>,
        provenance_root: Option<String>,
        llm_records: Vec<LlmProvenanceRecord>,
        n17_leaves: Vec<String>,
        n17_root: Option<String>,
        output: Value,
        topology: String,
    ) -> Self {
        Self {
            tenant: tenant.to_string(),
            records,
            root: provenance_root,
            llm_records,
            n17_leaves,
            n17_root,
            output,
            topology,
            created_at: SystemTime::now(),
        }
    }

    /// **ADR-016** : les feuilles N17 décodées (`[u8;32]`) pour `verify_run_offline`.
    /// Une feuille hex malformée est ignorée (la chaîne cassera de toute façon).
    pub fn n17_leaf_bytes(&self) -> Vec<[u8; 32]> {
        self.n17_leaves.iter().filter_map(|h| unhex32(h)).collect()
    }

    /// **ADR-016** : `n17_root` décodé (`[u8;32]`), si présent et bien formé.
    pub fn n17_root_bytes(&self) -> Option<[u8; 32]> {
        self.n17_root.as_deref().and_then(unhex32)
    }

    /// Y a-t-il de la forensique LLM (run Mode C) ? (sinon la re-preuve N17 est triviale).
    pub fn has_n17(&self) -> bool {
        !self.llm_records.is_empty()
    }

    /// Date de création en secondes epoch (pour la sérialisation de fil).
    pub fn created_at_unix(&self) -> u64 {
        self.created_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Décode 32 octets depuis un hex de 64 caractères (`None` si malformé). Sans dépendance.
fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let b = s.as_bytes();
    let nib = |c: u8| -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    };
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = (nib(b[i * 2])? << 4) | nib(b[i * 2 + 1])?;
    }
    Some(out)
}

/// Persistance des runs (la preuve). **Tenant-scopé** : `get` ne rend un run que s'il
/// appartient au tenant demandeur (isolation).
#[async_trait]
pub trait RunStore: Send + Sync {
    async fn put(&self, id: Uuid, tenant: &str, outcome: &RunOutcome) -> Result<(), GatewayError>;
    /// **Couche 2 (M2)** : persiste un [`StoredRun`] **déjà construit** (mission multi-LLM) —
    /// court-circuite `from_outcome` (pas de `RunOutcome`). Write-once, comme `put`.
    async fn put_stored(&self, id: Uuid, stored: StoredRun) -> Result<(), GatewayError>;
    async fn get(&self, id: Uuid, tenant: &str) -> Result<Option<StoredRun>, GatewayError>;
}

/// Store en mémoire (borné par le process). Suffit pour la démo + le standalone souverain.
#[derive(Debug, Default)]
pub struct InMemoryRunStore {
    runs: Mutex<HashMap<Uuid, StoredRun>>,
}

impl InMemoryRunStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nombre de runs stockés (diagnostic/tests).
    pub fn len(&self) -> usize {
        self.runs.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// **Test-only (ADR-016)** : insère un [`StoredRun`] synthétique (un run Mode C
    /// fabriqué à la main, avec forensique N17), pour éprouver la persistance + la
    /// re-preuve forensique sans Ollama. Écrase un id existant.
    #[cfg(any(test, feature = "tamper-testkit"))]
    pub fn insert_stored(&self, id: Uuid, stored: StoredRun) {
        if let Ok(mut g) = self.runs.lock() {
            g.insert(id, stored);
        }
    }

    /// **Test-only** : mute une sortie d'un record stocké, pour prouver que `verify_swarm`
    /// le **détecte** (test de falsification). N'altère **que** la copie en mémoire ;
    /// la re-preuve casse de toute façon.
    #[cfg(any(test, feature = "tamper-testkit"))]
    pub fn tamper_first_output(&self, id: Uuid) -> bool {
        let mut g = match self.runs.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        if let Some(run) = g.get_mut(&id) {
            if let Some(r) = run.records.first_mut() {
                r.output_hash = efe_nano_sdk::Blake3Hash::from_bytes([0xff; 32]);
                return true;
            }
        }
        false
    }
}

#[async_trait]
impl RunStore for InMemoryRunStore {
    async fn put(&self, id: Uuid, tenant: &str, outcome: &RunOutcome) -> Result<(), GatewayError> {
        let stored = StoredRun::from_outcome(tenant, outcome);
        self.runs
            .lock()
            .map_err(|_| GatewayError::Internal("store: lock empoisonné".into()))?
            .insert(id, stored);
        Ok(())
    }

    async fn put_stored(&self, id: Uuid, stored: StoredRun) -> Result<(), GatewayError> {
        self.runs
            .lock()
            .map_err(|_| GatewayError::Internal("store: lock empoisonné".into()))?
            .insert(id, stored);
        Ok(())
    }

    async fn get(&self, id: Uuid, tenant: &str) -> Result<Option<StoredRun>, GatewayError> {
        let g = self
            .runs
            .lock()
            .map_err(|_| GatewayError::Internal("store: lock empoisonné".into()))?;
        // Isolation tenant : un run d'un autre tenant est INVISIBLE (comme inexistant).
        Ok(g.get(&id).filter(|r| r.tenant == tenant).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_swarm_sdk::{Budget, Swarm, TopologySpec};
    use serde_json::json;

    async fn one_run() -> RunOutcome {
        Swarm::keyless()
            .run_traced(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100))
            .await
            .unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn put_then_get_same_tenant() {
        let store = InMemoryRunStore::new();
        let id = Uuid::new_v4();
        store.put(id, "system", &one_run().await).await.unwrap();
        let got = store.get(id, "system").await.unwrap();
        assert!(got.is_some());
        assert!(!got.unwrap().records.is_empty());
        assert_eq!(store.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_is_tenant_scoped() {
        let store = InMemoryRunStore::new();
        let id = Uuid::new_v4();
        store.put(id, "tenant-a", &one_run().await).await.unwrap();
        // Un autre tenant ne voit RIEN (isolation).
        assert!(store.get(id, "tenant-b").await.unwrap().is_none());
        assert!(store.get(id, "tenant-a").await.unwrap().is_some());
    }
}
