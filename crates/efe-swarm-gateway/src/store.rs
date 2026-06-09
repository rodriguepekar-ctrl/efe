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

use efe_swarm_sdk::{AgentCallRecord, RunOutcome};

use crate::error::GatewayError;

/// Un run persisté — **tenant-scopé**. `records` = l'évidence re-vérifiable.
#[derive(Debug, Clone)]
pub struct StoredRun {
    pub tenant: String,
    /// Les `AgentCallRecord` chaînés (l'évidence pour `verify_swarm`).
    pub records: Vec<AgentCallRecord>,
    pub root: Option<String>,
    pub output: Value,
    pub topology: String,
    pub created_at: SystemTime,
}

impl StoredRun {
    /// Construit depuis un [`RunOutcome`] du SDK (copie des records scellés).
    pub fn from_outcome(tenant: &str, outcome: &RunOutcome) -> Self {
        Self {
            tenant: tenant.to_string(),
            records: outcome.records().to_vec(),
            root: outcome.run.provenance.root.clone(),
            output: outcome.run.output.clone(),
            topology: outcome.run.topology.clone(),
            created_at: SystemTime::now(),
        }
    }

    /// Date de création en secondes epoch (pour la sérialisation de fil).
    pub fn created_at_unix(&self) -> u64 {
        self.created_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Persistance des runs (la preuve). **Tenant-scopé** : `get` ne rend un run que s'il
/// appartient au tenant demandeur (isolation).
#[async_trait]
pub trait RunStore: Send + Sync {
    async fn put(&self, id: Uuid, tenant: &str, outcome: &RunOutcome) -> Result<(), GatewayError>;
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
