// SPDX-License-Identifier: UNLICENSED
//! `SwarmGatewayState` — l'état partagé des handlers (clonable, axum). Fabrique un
//! [`Swarm`] **par tenant** (clé HKDF dérivée server-side), tient le [`RunStore`] et les
//! registres de métriques.

use std::str::FromStr;
use std::sync::Arc;

use efe_blackboard::SubstrateMetrics;
use efe_nano_sdk::TenantId;
use efe_swarm_dashboard::{DashboardBus, DashboardMetrics, DashboardObserver, DashboardRegistry};
use efe_swarm_provenance::ProvenanceMetrics;
use efe_swarm_sdk::{ClientMode, HmacKeySource, Swarm, SwarmObserver};
use efe_swarm_topology::AgentFleet;

use crate::error::GatewayError;
use crate::router_map::RouterMap;
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
    /// **Inc.4.b** : table de routage **provider→modèle** du mode `Network`. Traduit la
    /// famille (`"anthropic"`) en identifiant de modèle concret (`"anthropic/claude-…"`)
    /// attendu par le routeur. **Aucun secret** (noms de modèles = politique de routage).
    pub router_map: RouterMap,
    /// **Prod (feature `oidc`)** : vérificateur de JWT OIDC (clé PUBLIQUE server-side).
    /// `None` ⇒ repli sur l'extracteur d'en-têtes dev. Jamais une clé privée.
    #[cfg(feature = "oidc")]
    pub oidc: Option<Arc<crate::oidc::OidcVerifier>>,
    /// **G9 (feature `pii`)** : politique PII par tenant (défaut `Off`). Caviarde sorties +
    /// previews. Détection LOCALE (souverain).
    #[cfg(feature = "pii")]
    pub pii: crate::pii::PiiConfig,
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
            router_map: RouterMap::default(),
            #[cfg(feature = "oidc")]
            oidc: None,
            #[cfg(feature = "pii")]
            pii: crate::pii::PiiConfig::default(),
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
            router_map: RouterMap::default(),
            #[cfg(feature = "oidc")]
            oidc: None,
            #[cfg(feature = "pii")]
            pii: crate::pii::PiiConfig::default(),
        }
    }

    /// **G9 (feature `pii`)** : branche une politique PII par tenant (caviardage souverain).
    #[cfg(feature = "pii")]
    pub fn with_pii(mut self, config: crate::pii::PiiConfig) -> Self {
        self.pii = config;
        self
    }

    /// Caviarde la sortie d'un run selon la politique PII du tenant (no-op sans feature `pii`
    /// ou politique `Off`). Ne touche QUE la présentation — records/hashs intacts.
    pub fn redact_output(&self, tenant: &str, run: &mut efe_swarm_sdk::SwarmRun) {
        #[cfg(feature = "pii")]
        {
            let policy = self.pii.policy_for(tenant);
            run.output = crate::pii::redact_json(&run.output, policy);
        }
        #[cfg(not(feature = "pii"))]
        {
            let _ = (tenant, run);
        }
    }

    /// Crée l'observateur dashboard d'un run, avec un **redactor PII** des previews si le
    /// tenant l'active (feature `pii`). Sans la feature : observateur nu.
    pub fn dashboard_observer(&self, tenant: &str, bus: Arc<DashboardBus>) -> Arc<dyn SwarmObserver> {
        let observer = DashboardObserver::new(bus);
        #[cfg(feature = "pii")]
        let observer = {
            let policy = self.pii.policy_for(tenant);
            if matches!(policy, crate::pii::PolicyKind::Redact | crate::pii::PolicyKind::Strict) {
                observer.with_redactor(Arc::new(crate::pii::GatewayRedactor::new(policy)))
            } else {
                observer
            }
        };
        #[cfg(not(feature = "pii"))]
        let _ = tenant;
        Arc::new(observer)
    }

    /// **Inc.2.d (feature `oidc`)** : branche un vérificateur OIDC (clé publique server-side).
    /// L'auth `Caller` valide alors un JWT ES256 au lieu des en-têtes dev.
    #[cfg(feature = "oidc")]
    pub fn with_oidc(mut self, verifier: Arc<crate::oidc::OidcVerifier>) -> Self {
        self.oidc = Some(verifier);
        self
    }

    /// Remplace le store des runs (ex. `PgRunStore` en prod via la feature `postgres`).
    pub fn with_store(mut self, store: Arc<dyn RunStore>) -> Self {
        self.store = store;
        self
    }

    /// **Prod** : remplace la source de clé de **provenance**. En production, le binaire
    /// injecte `HmacKeySource::EnvHkdf` (master HKDF lu **server-side** via
    /// `EFE_NANO_AUDIT_HMAC_MASTER`) ; sans ça, le repli `test_key()` signerait avec un
    /// master **public** → la re-preuve (`verify`) serait **forgeable**. Cette clé **signe**
    /// la chaîne ; elle n'est **jamais** transmise ni une clé d'auth provider.
    pub fn with_keys(mut self, keys: HmacKeySource) -> Self {
        self.keys = keys;
        self
    }

    /// **Inc.4** : câble le routeur frontière (`ClientMode::Network`). L'`endpoint` est une
    /// **URL non-secrète** (le routeur server-side détient les clés). À lire depuis l'env
    /// **côté binaire** (ex. `EFE_SWARM_ROUTER_ENDPOINT`), jamais ici dans la lib.
    pub fn with_router_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        let ep = endpoint.into();
        self.router_endpoint = if ep.trim().is_empty() { None } else { Some(ep) };
        self
    }

    /// **Inc.4.b** : remplace la table de routage **provider→modèle** du mode `Network`
    /// (par défaut : `RouterMap::default()`). À alimenter depuis l'env **côté binaire**
    /// (ex. `EFE_SWARM_ROUTER_MODELS="anthropic=…,openai=…"`). **Aucun secret.**
    pub fn with_router_models(mut self, map: RouterMap) -> Self {
        self.router_map = map;
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
    /// Renvoie `(mode_exécutable, allow_remote_backend, tarif_à_appliquer_cents)`.
    ///
    /// **Inc.4.b (cost-governance)** : le 3ᵉ champ est un **tarif par appel à forcer**. La
    /// traduction `Network → SelfHost` ferait tomber le prix à **0¢** (self-host = gratuit),
    /// vidant la gouvernance de budget de son mordant (`spent_cents` resterait à 0). On renvoie
    /// donc `Some(tarif_Network)` pour le mode `Network` afin de **préserver** le tarif à travers
    /// la traduction ; `None` pour les modes souverains (leur tarif intrinsèque suffit). Le coût
    /// **réel** reste chez le provider (via le routeur) ; ce tarif est l'unité de **gouvernance**.
    pub fn resolve_mode(&self, mode: ClientMode) -> Result<(ClientMode, bool, Option<u32>), GatewayError> {
        match mode {
            ClientMode::Network { provider } => match &self.router_endpoint {
                // Inc.4.b : la **famille** (`provider`) est traduite en **modèle concret**
                // attendu par le routeur via la table provider→modèle (passthrough si déjà qualifié).
                Some(endpoint) => {
                    let model = self.router_map.resolve(&provider);
                    // Préserve le tarif Network (sinon la traduction → SelfHost le mettrait à 0¢).
                    let network_price = ClientMode::Network { provider }.price_per_call_cents();
                    Ok((ClientMode::SelfHost { endpoint: endpoint.clone(), model }, true, Some(network_price)))
                }
                None => Err(GatewayError::NotImplemented(
                    "ClientMode::Network non câblé sur ce déploiement (routeur frontière absent ; définissez EFE_SWARM_ROUTER_ENDPOINT côté serveur)".into(),
                )),
            },
            other => Ok((other, false, None)), // keyless / self-host : tarif intrinsèque du mode
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
        // souverains : pas de surcharge de tarif (None → tarif intrinsèque du mode).
        assert_eq!(s.resolve_mode(ClientMode::Keyless).unwrap(), (ClientMode::Keyless, false, None));
        let sh = ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() };
        assert_eq!(s.resolve_mode(sh.clone()).unwrap(), (sh, false, None), "self-host : souverain (egress refusé)");
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
        // Inc.4.b : la famille `anthropic` est traduite en modèle concret (table par défaut).
        let s = SwarmGatewayState::in_memory().with_router_endpoint("http://efe-llm-router:8080/v1");
        let (mode, allow_remote, price) = s.resolve_mode(ClientMode::Network { provider: "anthropic".into() }).unwrap();
        assert!(allow_remote, "Network → egress autorisé (vers le routeur)");
        // Inc.4.b : le tarif Network survit à la traduction → SelfHost (sinon budget = 0¢).
        assert_eq!(price, Some(ClientMode::Network { provider: "x".into() }.price_per_call_cents()),
                   "tarif Network préservé pour la gouvernance de budget");
        match mode {
            ClientMode::SelfHost { endpoint, model } => {
                assert_eq!(endpoint, "http://efe-llm-router:8080/v1");
                assert_eq!(model, "anthropic/claude-3-5-sonnet-latest", "famille → modèle concret routé");
            }
            other => panic!("attendu SelfHost-vers-routeur, eu {other:?}"),
        }
    }

    #[test]
    fn resolve_mode_network_applies_model_map_and_passthrough() {
        // Inc.4.b : table surchargée (famille connue) + passthrough d'un modèle déjà qualifié.
        let s = SwarmGatewayState::in_memory()
            .with_router_endpoint("http://efe-llm-router:8080/v1")
            .with_router_models(RouterMap::empty().with("openai", "openai/gpt-4o-mini"));
        // famille mappée explicitement.
        let (m1, _, _) = s.resolve_mode(ClientMode::Network { provider: "openai".into() }).unwrap();
        assert!(matches!(m1, ClientMode::SelfHost { ref model, .. } if model == "openai/gpt-4o-mini"));
        // modèle déjà qualifié (`family:model`) → passthrough intact.
        let (m2, _, _) = s.resolve_mode(ClientMode::Network { provider: "anthropic:claude-3-opus".into() }).unwrap();
        assert!(matches!(m2, ClientMode::SelfHost { ref model, .. } if model == "anthropic:claude-3-opus"));
    }

    #[test]
    fn network_budget_uses_network_tariff_and_respects_client_override() {
        // Inc.4.b (cost-governance) : reproduit le câblage de `routes::run` — le tarif renvoyé
        // par `resolve_mode` est injecté au budget (sinon Network facturerait 0¢/appel).
        use efe_swarm_sdk::Budget;
        let s = SwarmGatewayState::in_memory().with_router_endpoint("http://efe-llm-router:8080/v1");
        let net_price = ClientMode::Network { provider: "x".into() }.price_per_call_cents();

        // (a) sans tarif client : le tarif Network est appliqué.
        let mut budget = Budget::cents(1000); // price_per_call_cents = None
        let (_m, _a, price_override) = s.resolve_mode(ClientMode::Network { provider: "anthropic".into() }).unwrap();
        if let Some(p) = price_override { budget.price_per_call_cents.get_or_insert(p); }
        assert_eq!(budget.price_per_call_cents, Some(net_price), "Network → budget facture au tarif Network (≠ 0¢)");

        // (b) tarif explicite du client : respecté (jamais écrasé).
        let mut budget2 = Budget::cents(1000).with_price_per_call(2);
        if let Some(p) = price_override { budget2.price_per_call_cents.get_or_insert(p); }
        assert_eq!(budget2.price_per_call_cents, Some(2), "tarif explicite du client préservé");

        // (c) modes souverains : aucune surcharge → tarif intrinsèque du mode.
        let (_m, _a, none_price) = s.resolve_mode(ClientMode::Keyless).unwrap();
        assert_eq!(none_price, None, "keyless/self-host : pas de surcharge de tarif");
    }
}
