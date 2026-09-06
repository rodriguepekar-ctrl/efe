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
    /// **Couche 2 (M3)** : le résumé **scellé** de la requête de mission (`kind`, `k`, `rounds`,
    /// `user_text_hash`, `verifier_decision`…) — la source dont le checker out-of-band
    /// `/v1/llm/audit/:id` **re-dérive** l'arité déclarée (jamais l'output producteur). `{}`
    /// pour un run essaim/keyless (pas de mission). **Aucun secret** (hachés + paramètres).
    pub request: Value,
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
            request: Value::Null, // run essaim : pas de requête de mission LLM
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
        request: Value,
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
            request,
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

    /// **Test-only (P.0)** : **tronque la queue** d'un run — retire les `drop` derniers
    /// enregistrements en conservant la racine scellée.
    ///
    /// C'est l'attaque que l'intégrité seule **ne voit pas** : un préfixe d'une chaîne HMAC
    /// valide est lui-même une chaîne HMAC valide, donc `chain_intact` reste `true`. Seule la
    /// comparaison à la racine scellée la révèle. Ne touche **que** la copie en mémoire.
    ///
    /// Rend `false` si le run est inconnu, si `drop` vaut 0, ou s'il ne reste rien à retirer
    /// (on refuse de fabriquer un cas d'attaque vide qui rendrait le test vacant).
    #[cfg(any(test, feature = "tamper-testkit"))]
    pub fn truncate_tail(&self, id: Uuid, drop: usize) -> bool {
        if drop == 0 {
            return false;
        }
        let mut g = match self.runs.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        match g.get_mut(&id) {
            Some(run) if run.records.len() > drop => {
                let keep = run.records.len() - drop;
                run.records.truncate(keep);
                true
            }
            _ => false,
        }
    }

    /// **Test-only (P.0)** : **supprime tous** les enregistrements d'un run, en conservant la
    /// racine scellée. Cas-limite vicieux : `verify_swarm` déclare un run vide *trivialement*
    /// intègre (chaîne, contenu et DAG à `true`) et ne reconstruit **aucune** racine — un
    /// contrôle naïf le laisserait donc passer pour vérifié.
    #[cfg(any(test, feature = "tamper-testkit"))]
    pub fn drop_all_records(&self, id: Uuid) -> bool {
        let mut g = match self.runs.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        match g.get_mut(&id) {
            Some(run) if !run.records.is_empty() => {
                run.records.clear();
                true
            }
            _ => false,
        }
    }

    /// **Test-only (P.0)** : efface la **racine scellée** d'un run, simulant un enregistrement
    /// antérieur à sa persistance. Sert à prouver la **non-accusation rétroactive** : un tel run
    /// doit rendre `root_matches: None` et **conserver** son verdict.
    #[cfg(any(test, feature = "tamper-testkit"))]
    pub fn clear_root(&self, id: Uuid) -> bool {
        let mut g = match self.runs.lock() {
            Ok(g) => g,
            Err(_) => return false,
        };
        match g.get_mut(&id) {
            Some(run) => {
                run.root = None;
                true
            }
            None => false,
        }
    }
}

/// L'inventaire en mémoire — cf. [`crate::inventaire`] pour la raison du trait séparé.
///
/// Le tri se fait ici, à chaque appel : une `HashMap` n'a pas d'ordre, et une sauvegarde qui
/// lirait dans l'ordre de hachage ne serait pas reproductible d'une exécution à l'autre. Le coût
/// est sans objet — ce store est celui de la démo et du souverain autonome.
#[async_trait]
impl crate::inventaire::RunStoreInventaire for InMemoryRunStore {
    async fn inventorier(
        &self,
        apres: crate::inventaire::Curseur,
        limite: u32,
    ) -> Result<Vec<crate::inventaire::LigneExport>, GatewayError> {
        let mut lignes: Vec<crate::inventaire::LigneExport> = {
            let g = self
                .runs
                .lock()
                .map_err(|_| GatewayError::Internal("verrou du store empoisonné".into()))?;
            g.iter().map(|(id, r)| crate::inventaire::LigneExport::depuis(*id, r)).collect()
        };
        lignes.sort_by_key(crate::inventaire::LigneExport::cle);
        // `partition_point` : le premier indice STRICTEMENT après le curseur.
        let debut = match apres {
            None => 0,
            Some(c) => lignes.partition_point(|l| l.cle() <= c),
        };
        lignes.drain(..debut);
        lignes.truncate(limite as usize);
        Ok(lignes)
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
