// SPDX-License-Identifier: UNLICENSED
//! `SwarmGatewayState` — l'état partagé des handlers (clonable, axum). Fabrique un
//! [`Swarm`] **par tenant** (clé HKDF dérivée server-side), tient le [`RunStore`] et les
//! registres de métriques.

use std::str::FromStr;
use std::sync::Arc;

use efe_blackboard::SubstrateMetrics;
use efe_nano_sdk::TenantId;
use efe_swarm_provenance::ProvenanceMetrics;
use efe_swarm_sdk::{ClientMode, HmacKeySource, Swarm};
use efe_swarm_topology::AgentFleet;

use crate::error::GatewayError;
use crate::store::{InMemoryRunStore, RunStore};

/// État de la passerelle (clonable : tout est `Arc` ou `Clone` bon marché).
#[derive(Clone)]
pub struct SwarmGatewayState {
    /// Flotte d'agents **partagée** entre tous les `Swarm` (Mode A keyless).
    pub fleet: Arc<AgentFleet>,
    /// Source de clé de provenance (dev : `test_key` ; prod : `EnvHkdf` server-side).
    pub keys: HmacKeySource,
    /// Persistance des runs (la preuve).
    pub store: Arc<dyn RunStore>,
    /// Métriques de provenance **partagées** : chaque run les alimente (→ `/metrics`).
    pub prov_metrics: Arc<ProvenanceMetrics>,
    /// Métriques de substrat (Blackboard/EventBus) — exposées en `/metrics`.
    pub substrate_metrics: Arc<SubstrateMetrics>,
}

impl SwarmGatewayState {
    /// État complet (injection explicite). Prod : `keys = HmacKeySource::EnvHkdf`,
    /// `store = Arc<PgRunStore>`.
    pub fn new(
        fleet: Arc<AgentFleet>,
        keys: HmacKeySource,
        store: Arc<dyn RunStore>,
        prov_metrics: Arc<ProvenanceMetrics>,
        substrate_metrics: Arc<SubstrateMetrics>,
    ) -> Self {
        Self { fleet, keys, store, prov_metrics, substrate_metrics }
    }

    /// État **dev/standalone** : flotte standard, clé de test, store en mémoire, métriques
    /// neuves. Le mode démo souverain en une ligne.
    pub fn in_memory() -> Self {
        Self {
            fleet: Arc::new(AgentFleet::standard()),
            keys: HmacKeySource::test_key(),
            store: Arc::new(InMemoryRunStore::new()),
            prov_metrics: ProvenanceMetrics::new(),
            substrate_metrics: Arc::new(SubstrateMetrics::default()),
        }
    }

    /// Résout un tenant de fil (chaîne) en [`TenantId`]. `"system"` est accepté comme alias
    /// du tenant système ; sinon un UUID est requis.
    pub fn tenant_id(&self, tenant: &str) -> Result<TenantId, GatewayError> {
        if tenant == "system" {
            return Ok(TenantId::system());
        }
        TenantId::from_str(tenant).map_err(|_| GatewayError::BadRequest(format!("tenant invalide : {tenant}")))
    }

    /// Fabrique un [`Swarm`] **pour ce tenant** : flotte partagée, clé HKDF dérivée
    /// server-side, métriques partagées. Le mode (keyless/self-host/network) vient de la
    /// requête.
    pub fn swarm_for(&self, tenant: &str, mode: ClientMode) -> Result<Swarm, GatewayError> {
        let tid = self.tenant_id(tenant)?;
        Swarm::builder()
            .fleet(self.fleet.clone())
            .tenant(tid)
            .key_source(self.keys.clone())
            .mode(mode)
            .with_metrics(self.prov_metrics.clone())
            .build()
            .map_err(GatewayError::from)
    }

    /// Dérive la clé de provenance du tenant (server-side) pour re-prouver un run stocké.
    /// **La clé n'est jamais exposée** — seul le `root` l'est.
    pub fn derive_key(&self, tenant: &str) -> Result<Vec<u8>, GatewayError> {
        let tid = self.tenant_id(tenant)?;
        self.keys.derive(tid).map_err(|e| GatewayError::Internal(format!("clé tenant : {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_state_builds_swarms_and_keys() {
        let s = SwarmGatewayState::in_memory();
        assert!(s.swarm_for("system", ClientMode::Keyless).is_ok());
        // même clé pour le même tenant (déterministe) → re-preuve cohérente.
        let k1 = s.derive_key("system").unwrap();
        let k2 = s.derive_key("system").unwrap();
        assert_eq!(k1, k2);
        assert!(s.tenant_id("not-a-uuid").is_err());
    }
}
