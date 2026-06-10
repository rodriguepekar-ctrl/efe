// SPDX-License-Identifier: UNLICENSED
//! `SwarmGatewayState` — l'état partagé des handlers (clonable, axum). Fabrique un
//! [`Swarm`] **par tenant** (clé HKDF dérivée server-side), tient le [`RunStore`] et les
//! registres de métriques.

use std::str::FromStr;
use std::sync::Arc;

use efe_blackboard::SubstrateMetrics;
use efe_nano_sdk::TenantId;
use efe_swarm_dashboard::{DashboardMetrics, DashboardRegistry};
use efe_swarm_provenance::ProvenanceMetrics;
use efe_swarm_sdk::{ClientMode, HmacKeySource, Swarm, SwarmObserver};
use efe_swarm_topology::AgentFleet;

use crate::error::GatewayError;
use crate::store::{InMemoryRunStore, RunStore};

/// Plafond de runs live simultanés conservés dans le registre (drop-oldest au-delà).
const DASHBOARD_REGISTRY_CAP: usize = 256;

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
    /// **Inc.3** : registre des bus de dashboard live (par run, tenant-scopé, borné).
    pub dashboards: Arc<DashboardRegistry>,
    /// **Inc.3** : métriques du dashboard (events émis/perdus) — exposées en `/metrics`.
    pub dash_metrics: Arc<DashboardMetrics>,
    /// **Inc.4** : endpoint du **routeur frontière server-side** (`efe-llm-router`, M21) qui
    /// détient les clés provider (env/Vault). `None` ⇒ `ClientMode::Network` non câblé (501).
    /// La passerelle parle au routeur **sans clé applicative** (auth = infra : mTLS/NetworkPolicy) ;
    /// le routeur ajoute la clé provider **server-side**. **Aucune clé manipulée ici.**
    pub router_endpoint: Option<String>,
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
        Self {
            fleet,
            keys,
            store,
            prov_metrics,
            substrate_metrics,
            dashboards: Arc::new(DashboardRegistry::new(DASHBOARD_REGISTRY_CAP)),
            dash_metrics: DashboardMetrics::new(),
            router_endpoint: None,
        }
    }

    /// État **dev/standalone** : flotte standard, clé de test, store en mémoire, métriques
    /// neuves. Le mode démo souverain en une ligne. `Network` désactivé (pas de routeur).
    pub fn in_memory() -> Self {
        Self {
            fleet: Arc::new(AgentFleet::standard()),
            keys: HmacKeySource::test_key(),
            store: Arc::new(InMemoryRunStore::new()),
            prov_metrics: ProvenanceMetrics::new(),
            substrate_metrics: Arc::new(SubstrateMetrics::default()),
            dashboards: Arc::new(DashboardRegistry::new(DASHBOARD_REGISTRY_CAP)),
            dash_metrics: DashboardMetrics::new(),
            router_endpoint: None,
        }
    }

    /// **Inc.4** : câble le routeur frontière (`ClientMode::Network`). L'`endpoint` est une
    /// **URL non-secrète** (le routeur server-side détient les clés). À lire depuis l'env
    /// **côté binaire** (ex. `EFE_SWARM_ROUTER_ENDPOINT`), jamais ici dans la lib.
    pub fn with_router_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let ep = endpoint.into();
        self.router_endpoint = if ep.trim().is_empty() { None } else { Some(ep) };
        self
    }

    /// Résout un tenant de fil (chaîne) en [`TenantId`]. `"system"` est accepté comme alias
    /// du tenant système ; sinon un UUID est requis.
    pub fn tenant_id(&self, tenant: &str) -> Result<TenantId, GatewayError> {
        if tenant == "system" {
            return Ok(TenantId::system());
        }
        TenantId::from_str(tenant).map_err(|_| GatewayError::BadRequest(format!("tenant invalide : {tenant}")))
    }

    /// **Inc.4** : résout le mode de fil en mode SDK **exécutable**.
    /// - `Keyless`/`SelfHost` : inchangés, **souverains** (`allow_remote = false` → egress refusé).
    /// - `Network { provider }` : si un routeur est câblé, **traduit** en `SelfHost` vers le
    ///   **routeur server-side** (qui détient les clés) — `allow_remote = true` (Network fait de
    ///   l'egress **par conception**, l'opposé du mode souverain). Sinon `501`. **Aucune clé
    ///   manipulée ici** : la passerelle ne connaît que l'URL (non-secrète) du routeur ; les clés
    ///   provider vivent **dans le routeur** (`efe-llm-router`, env/Vault).
    ///
    /// Renvoie `(mode_exécutable, allow_remote_backend)`.
    pub fn resolve_mode(&self, mode: ClientMode) -> Result<(ClientMode, bool), GatewayError> {
        match mode {
            ClientMode::Network { provider } => match &self.router_endpoint {
                Some(endpoint) => Ok((ClientMode::SelfHost { endpoint: endpoint.clone(), model: provider }, true)),
                None => Err(GatewayError::NotImplemented(
                    "ClientMode::Network non câblé sur ce déploiement (routeur frontière absent ; définissez EFE_SWARM_ROUTER_ENDPOINT côté serveur)".into(),
                )),
            },
            other => Ok((other, false)), // keyless / self-host : souverain (egress refusé)
        }
    }

    /// Fabrique un [`Swarm`] **pour ce tenant**. `allow_remote` n'est `true` que pour le
    /// routeur Network (egress assumé) ; `false` garde la souveraineté (self-host local).
    pub fn swarm_for(&self, tenant: &str, mode: ClientMode, allow_remote: bool) -> Result<Swarm, GatewayError> {
        let tid = self.tenant_id(tenant)?;
        Swarm::builder()
            .fleet(self.fleet.clone())
            .tenant(tid)
            .key_source(self.keys.clone())
            .mode(mode)
            .with_metrics(self.prov_metrics.clone())
            .with_substrate_metrics(self.substrate_metrics.clone())
            .require_healthy_backend(true) // échec rapide si backend (self-host ou routeur) mort ; no-op keyless
            .allow_remote_backend(allow_remote)
            .build()
            .map_err(GatewayError::from)
    }

    /// **Inc.3** : comme [`Self::swarm_for`], mais **observé** (dashboard live).
    pub fn swarm_for_observed(
        &self,
        tenant: &str,
        mode: ClientMode,
        observer: Arc<dyn SwarmObserver>,
        allow_remote: bool,
    ) -> Result<Swarm, GatewayError> {
        let tid = self.tenant_id(tenant)?;
        Swarm::builder()
            .fleet(self.fleet.clone())
            .tenant(tid)
            .key_source(self.keys.clone())
            .mode(mode)
            .with_metrics(self.prov_metrics.clone())
            .with_substrate_metrics(self.substrate_metrics.clone())
            .with_observer(observer)
            .require_healthy_backend(true)
            .allow_remote_backend(allow_remote)
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
        assert!(s.swarm_for("system", ClientMode::Keyless, false).is_ok());
        // même clé pour le même tenant (déterministe) → re-preuve cohérente.
        let k1 = s.derive_key("system").unwrap();
        let k2 = s.derive_key("system").unwrap();
        assert_eq!(k1, k2);
        assert!(s.tenant_id("not-a-uuid").is_err());
    }

    #[test]
    fn resolve_mode_keeps_sovereign_modes_unchanged() {
        let s = SwarmGatewayState::in_memory();
        assert_eq!(s.resolve_mode(ClientMode::Keyless).unwrap(), (ClientMode::Keyless, false));
        let sh = ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() };
        assert_eq!(s.resolve_mode(sh.clone()).unwrap(), (sh, false), "self-host : souverain (egress refusé)");
    }

    #[test]
    fn resolve_mode_network_without_router_is_501() {
        let s = SwarmGatewayState::in_memory(); // pas de routeur
        let err = s.resolve_mode(ClientMode::Network { provider: "anthropic".into() });
        assert!(matches!(err, Err(GatewayError::NotImplemented(_))), "Network sans routeur → 501");
    }

    #[test]
    fn resolve_mode_network_with_router_routes_to_self_host() {
        // Inc.4 : routeur câblé (URL non-secrète) → Network devient SelfHost-vers-routeur,
        // egress autorisé (Network fait de l'egress par conception). AUCUNE clé ici.
        let s = SwarmGatewayState::in_memory().with_router_endpoint("http://efe-llm-router:8080/v1");
        let (mode, allow_remote) = s.resolve_mode(ClientMode::Network { provider: "anthropic".into() }).unwrap();
        assert!(allow_remote, "Network → egress autorisé (vers le routeur)");
        match mode {
            ClientMode::SelfHost { endpoint, model } => {
                assert_eq!(endpoint, "http://efe-llm-router:8080/v1");
                assert_eq!(model, "anthropic", "le provider devient le modèle routé");
            }
            other => panic!("attendu SelfHost-vers-routeur, eu {other:?}"),
        }
    }
}
