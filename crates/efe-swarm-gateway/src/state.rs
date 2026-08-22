// SPDX-License-Identifier: UNLICENSED
//! `SwarmGatewayState` — l'état partagé des handlers (clonable, axum). Fabrique un
//! [`Swarm`] **par tenant** (clé HKDF dérivée server-side), tient le [`RunStore`] et les
//! registres de métriques.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use efe_blackboard::SubstrateMetrics;
use efe_nano_sdk::TenantId;
use efe_swarm_dashboard::{DashboardBus, DashboardMetrics, DashboardObserver, DashboardRegistry};
use efe_swarm_memory::{InMemoryMemoryStore, MemoryStore};
use efe_swarm_provenance::ProvenanceMetrics;
use efe_swarm_sdk::{ClientMode, HmacKeySource, Swarm, SwarmObserver, WidthSpec};
use efe_swarm_topology::AgentFleet;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::GatewayError;
use crate::router_map::RouterMap;
use crate::store::{InMemoryRunStore, RunStore};

/// Plafond de runs live simultanés conservés dans le registre (drop-oldest au-delà).
const DASHBOARD_REGISTRY_CAP: usize = 256;

/// **ADR-014 (CP-2)** : permits d'admission process — runs en vol simultanés.
/// Env `EFE_GATEWAY_MAX_INFLIGHT_RUNS` ; défaut `max(4, cœurs)`. Au-delà : 429 visible
/// (backpressure honnête) plutôt qu'une file invisible qui finit en timeouts.
fn admission_permits() -> usize {
    crate::profile::read::max_inflight_runs()
}

/// **ADR-014 (CP-2)** : plafond serveur du champ DTO `width` (un client ne dimensionne
/// pas la machine d'autrui). Env `EFE_GATEWAY_MAX_WIDTH` ; défaut 32.
fn max_width_cap() -> usize {
    crate::profile::read::max_width()
}

/// **ADR-018 (admission équitable)** : plafond de runs en vol simultanés **par tenant**,
/// superposé au plafond process global. Sans lui, l'admission ADR-014 est globale : un
/// seul tenant bavard peut occuper TOUS les permits et affamer les autres (le gap «
/// fairness par tenant » laissé ouvert en ADR-014). Le défaut `max(1, global − 1)` est un
/// **cap, pas une réservation** : il garantit qu'aucun tenant ne prend le dernier permit
/// disponible, laissant toujours ≥ 1 slot tourner pour autrui — sans pré-allouer par
/// tenant (impossible : le nombre de tenants est dynamique). Env
/// `EFE_GATEWAY_MAX_INFLIGHT_PER_TENANT` ; un opérateur peut l'égaler au global (désactive
/// l'équité) ou le baisser (équité stricte).
fn admission_per_tenant_permits(global: usize) -> usize {
    crate::profile::read::max_inflight_per_tenant(global)
}

/// **ADR-018** : pourquoi l'admission a refusé — distingue les deux plafonds pour que le
/// client (et l'opérateur via `/metrics`) sachent s'il faut *réessayer plus tard*
/// (passerelle pleine) ou *ralentir SON propre débit* (quota de tenant atteint). Même code
/// d'enveloppe HTTP (429) que les autres limites de l'API ; le champ `reason` précise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitReject {
    /// Plafond **process** (global) atteint — la passerelle entière est à capacité.
    GatewayFull,
    /// Plafond **par-tenant** atteint — l'appelant a déjà trop de runs en vol (équité) ;
    /// la passerelle a peut-être de la capacité globale, mais pas pour CE tenant.
    TenantQuota,
}

impl AdmitReject {
    /// Code machine stable (corps 429 `reason`, métrique).
    pub fn reason(&self) -> &'static str {
        match self {
            AdmitReject::GatewayFull => "gateway_full",
            AdmitReject::TenantQuota => "tenant_quota",
        }
    }
    /// Message humain (corps 429 `message`).
    pub fn message(&self) -> &'static str {
        match self {
            AdmitReject::GatewayFull => {
                "passerelle à capacité (admission process, ADR-014) : réessayez (Retry-After)"
            }
            AdmitReject::TenantQuota => {
                "quota de runs simultanés de votre tenant atteint (équité ADR-018) : \
                 attendez la fin d'un run en cours (Retry-After)"
            }
        }
    }
}

/// **ADR-018** : permit d'admission **composite** — détient le permit global (process) ET
/// décrémente le compteur par-tenant à sa libération. Un seul objet RAII gouverne les deux
/// plafonds : tant qu'il vit, le run compte dans le global ET dans le quota de son tenant ;
/// son `Drop` libère atomiquement les deux (en sync au retour du handler, ou en fin de
/// tâche pour un job détaché — le permit y est déplacé). Jamais de fuite de compteur :
/// même un panic du handler droppe le permit et rééquilibre le quota.
#[derive(Debug)]
pub struct AdmissionPermit {
    /// Permit du sémaphore process global (libéré au `Drop`, ADR-014).
    _global: OwnedSemaphorePermit,
    /// Table des compteurs par-tenant (partagée avec l'état) — décrémentée au `Drop`.
    inflight: Arc<Mutex<HashMap<String, usize>>>,
    /// Tenant propriétaire de ce permit (clé du compteur à décrémenter).
    tenant: String,
}

impl Drop for AdmissionPermit {
    fn drop(&mut self) {
        // Le permit global se libère seul (Drop de `_global`). Ici : décrément du
        // compteur par-tenant, avec nettoyage de l'entrée à zéro (la table ne grossit pas
        // avec le nombre de tenants vus). Un mutex empoisonné est ignoré (best-effort —
        // on ne panique JAMAIS dans un Drop ; le pire cas est un compteur figé, borné par
        // le sémaphore global qui, lui, se libère toujours).
        if let Ok(mut m) = self.inflight.lock() {
            if let Some(c) = m.get_mut(&self.tenant) {
                *c = c.saturating_sub(1);
                if *c == 0 {
                    m.remove(&self.tenant);
                }
            }
        }
    }
}

/// État de la passerelle (clonable : tout est `Arc` ou `Clone` bon marché).
#[derive(Clone)]
pub struct SwarmGatewayState {
    /// Flotte d'agents **partagée** entre tous les `Swarm` (Mode A keyless).
    pub fleet: Arc<AgentFleet>,
    /// Source de clé de provenance (dev : `test_key` ; prod : `EnvHkdf` server-side).
    pub keys: HmacKeySource,
    /// Persistance des runs (la preuve).
    pub store: Arc<dyn RunStore>,
    /// **ADR-019 (MEM-3)** : la mémoire/apprentissage inter-runs, tenant-scopée. Dev :
    /// `InMemoryMemoryStore` ; prod : `PgMemoryStore` (feature `postgres`). Le recall/learn se
    /// branchent par-requête sur l'essaim (selon le DTO `memory`) ; les endpoints `/memory/*`
    /// lisent + re-prouvent ce store.
    pub memory: Arc<dyn MemoryStore>,
    /// **ADR-019** : recalls mémoire cumulés (compteur `/metrics`) — un run a lu un instantané.
    pub memory_recalls: Arc<AtomicU64>,
    /// **ADR-019** : replis mémoire cumulés (compteur `/metrics`) — un run propre a appris.
    pub memory_learns: Arc<AtomicU64>,
    /// **M2** : ce que la passerelle fait quand la PERSISTANCE d'un run échoue. Défaut
    /// [`crate::capture::CapturePolicy::Bloquante`] — comportement d'avant M2, à l'octet près.
    /// La posture d'observation est un opt-in déclaré au registre M0, jamais un défaut silencieux.
    pub capture_policy: crate::capture::CapturePolicy,
    /// **M2** : le journal des brèches — les runs servis qu'on n'a PAS pu persister. Une brèche
    /// invisible serait pire que pas de chaîne du tout : la valeur d'une chaîne EST d'être complète.
    pub gaps: Arc<crate::capture::GapJournal>,
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
    /// **Fil P (P.2)** : l'export du **bundle de provenance** (pile B) est-il servi ?
    ///
    /// `false` par défaut, y compris via [`Self::in_memory`] et [`Self::new`] : sans opt-in
    /// explicite, la route n'est **pas montée** et le comportement servi est byte-identique.
    /// Câblé depuis l'env par le binaire (`EFE_SWARM_BUNDLE_ENABLED=true`), ou par
    /// [`Self::with_bundle_export`] — ce dernier évite toute manipulation d'environnement dans
    /// les tests, qui serait une course en exécution parallèle.
    ///
    /// L'export ne révèle **aucune clé** : le bundle ne contient que des empreintes, et la clé
    /// tenant de la pile B voyage hors bande vers l'auditeur.
    pub bundle_export: bool,
    /// **Fil P (P.3)** : la clé privée d'**attestation de racine**, si l'opérateur en a déployé une.
    ///
    /// `None` ⟹ les bundles sont servis **sans champ d'attestation** — jamais avec un champ vide
    /// ou nul, qu'un consommateur pourrait prendre pour une signature. Absent veut dire absent.
    ///
    /// La clé ne fuit pas : `BundleSigner` rédige son `Debug`, et `SwarmGatewayState` ne dérive pas
    /// `Debug`. Elle ne quitte **jamais** le processus — seule l'**empreinte** (`key_id`) est
    /// servie, et la clé **publique** se publie hors bande.
    pub bundle_signer: Option<Arc<provenance_bridge::BundleSigner>>,
    /// **Fil P (P.4)** : le support de publication de l'**ancre** — la racine attendue, écrite
    /// **hors** du bundle.
    ///
    /// `None` ⟹ aucune ancre n'est publiée, et la réponse porte `anchored: false`. C'est la
    /// vérité : sans référence externe, un bundle tronqué re-prouve parfaitement, et le bundle ne
    /// prouve sa cohérence que contre lui-même.
    ///
    /// Le support est délibérément **abstrait** (`AnchorSink`) : l'architecture (§11.2) laisse la
    /// question ouverte entre journal interne, horodatage RFC-3161 et journal de transparence.
    /// Seul le contrat est imposé — hors bundle, append-only, canal distinct.
    pub anchor_sink: Option<Arc<dyn provenance_bridge::AnchorSink>>,
    /// **Couche 2 (feature `frontier-llm`)** : moteur de coordination multi-LLM
    /// (Claude/Gemini/GPT) — fan-out + vote + provenance par appel. `None` ⇒ frontière
    /// non câblée (les futures routes `/v1/llm/*` répondront 501). Câblé server-side depuis
    /// l'env (`EFE_LLM_FRONTIER_ENABLED` + clés provider) ; les clés vivent **dans les
    /// providers du coordinator**, jamais copiées dans cet état ni scellées dans un bundle.
    #[cfg(feature = "frontier-llm")]
    pub coordinator: Option<Arc<efe_coordinator::Coordinator>>,
    /// **Couche 2 (M4 §2.4)** : compteurs d'egress frontière (appels, appels R4, coût µ¢, coût
    /// par tenant) exposés sur `/metrics`. Alimentés au scellement de mission — le chemin
    /// PAYANT devient observable (alerting coût/egress). Toujours présent sous la feature.
    #[cfg(feature = "frontier-llm")]
    pub frontier_metrics: Arc<crate::frontier_metrics::FrontierMetrics>,
    /// **Prod (feature `oidc`)** : vérificateur de JWT OIDC (clé PUBLIQUE server-side).
    /// `None` ⇒ repli sur l'extracteur d'en-têtes dev. Jamais une clé privée.
    #[cfg(feature = "oidc")]
    pub oidc: Option<Arc<crate::oidc::OidcVerifier>>,
    /// **Couche 2 (M3, feature `oidc`)** : vérificateur OIDC **DÉDIÉ au chemin frontière** —
    /// audience épinglée `efe-llm` (cf. [`crate::oidc::OidcVerifier::llm_from_env`]). Séparé de
    /// `oidc` pour qu'un jeton d'essaim souverain NE déverrouille PAS l'egress tiers coûteux
    /// non-souverain (§4). Consommé par l'extracteur [`crate::auth::LlmCaller`] des routes
    /// `/v1/llm/*` ; `None` ⇒ 401 fail-closed sur ce chemin (jamais de repli keyless).
    #[cfg(feature = "oidc")]
    pub oidc_llm: Option<Arc<crate::oidc::OidcVerifier>>,
    /// **G9 (feature `pii`)** : politique PII par tenant (défaut `Off`). Caviarde sorties +
    /// previews. Détection LOCALE (souverain).
    #[cfg(feature = "pii")]
    pub pii: crate::pii::PiiConfig,
    /// **ADR-014 (CP-2)** : sémaphore d'**admission process** (FIFO) — borne les runs
    /// en vol (sync ET jobs). `try_acquire` en tête de handler ; refus = 429 +
    /// `Retry-After` (backpressure VISIBLE). Ferme l'étage process du gap §17
    /// « admission-control » (les tiers/files par tenant restent ouverts — ADR-014).
    pub admission: Arc<Semaphore>,
    /// Permits configurés (pour la jauge `inflight = max − disponibles`).
    pub admission_max: usize,
    /// Refus d'admission cumulés (compteur `/metrics`).
    pub admission_rejected: Arc<AtomicU64>,
    /// **ADR-018 (équité)** : plafond de runs en vol **par tenant**, superposé au global.
    /// Ferme le gap « fairness par tenant » laissé ouvert en ADR-014.
    pub admission_per_tenant_max: usize,
    /// **ADR-018** : compteur de runs en vol **par tenant** (clé = tenant). Incrémenté à
    /// l'admission, décrémenté au `Drop` de l'[`AdmissionPermit`]. `Mutex` std (section
    /// critique = quelques instructions, jamais tenue à travers un `await`).
    pub admission_tenant_inflight: Arc<Mutex<HashMap<String, usize>>>,
    /// Plafond serveur du champ DTO `width` (`EFE_GATEWAY_MAX_WIDTH`, défaut 32).
    pub max_width: usize,
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
        let permits = admission_permits();
        // ADR-019 : le store mémoire par défaut scelle son `memory_root` avec la MÊME source de
        // clé tenant que le reste (sinon, en prod EnvHkdf, la mémoire serait scellée sous
        // test_key — racine incohérente avec `derive_key`). Clone AVANT le move de `keys`.
        let memory_keys = keys.clone();
        Self {
            fleet,
            keys,
            store,
            memory: Arc::new(InMemoryMemoryStore::with_keys(memory_keys)),
            memory_recalls: Arc::new(AtomicU64::new(0)),
            memory_learns: Arc::new(AtomicU64::new(0)),
            capture_policy: crate::capture::CapturePolicy::default(),
            gaps: Arc::new(crate::capture::GapJournal::new()),
            prov_metrics,
            substrate_metrics,
            dashboards: Arc::new(DashboardRegistry::new(DASHBOARD_REGISTRY_CAP)),
            dash_metrics: DashboardMetrics::new(),
            router_endpoint: None,
            router_map: RouterMap::default(),
            // Fil P (P.2) : OFF par défaut — la route d'export n'est pas montée.
            bundle_export: false,
            // Fil P (P.3) : aucune clé d'attestation par défaut — les bundles sortent SANS
            // champ d'attestation, ce qui est la vérité plutôt qu'un champ vide.
            bundle_signer: None,
            // Fil P (P.4) : aucune publication d'ancre par défaut.
            anchor_sink: None,
            #[cfg(feature = "frontier-llm")]
            coordinator: None,
            #[cfg(feature = "frontier-llm")]
            frontier_metrics: crate::frontier_metrics::FrontierMetrics::new(),
            #[cfg(feature = "oidc")]
            oidc: None,
            #[cfg(feature = "oidc")]
            oidc_llm: None,
            #[cfg(feature = "pii")]
            pii: crate::pii::PiiConfig::default(),
            admission: Arc::new(Semaphore::new(permits)),
            admission_max: permits,
            admission_rejected: Arc::new(AtomicU64::new(0)),
            admission_per_tenant_max: admission_per_tenant_permits(permits),
            admission_tenant_inflight: Arc::new(Mutex::new(HashMap::new())),
            max_width: max_width_cap(),
        }
    }

    /// État **dev/standalone** : flotte standard, clé de test, store en mémoire, métriques
    /// neuves. Le mode démo souverain en une ligne. `Network` désactivé (pas de routeur).
    pub fn in_memory() -> Self {
        Self::new(
            Arc::new(AgentFleet::standard()),
            HmacKeySource::test_key(),
            Arc::new(InMemoryRunStore::new()),
            ProvenanceMetrics::new(),
            Arc::new(SubstrateMetrics::default()),
        )
    }

    /// **ADR-014 (CP-2, tests/déploiements)** : remplace les permits d'admission process.
    /// Recalcule aussi le plafond par-tenant (dérivé du global, ADR-018) sauf override env.
    pub fn with_admission_permits(mut self, permits: usize) -> Self {
        let permits = permits.max(1);
        self.admission = Arc::new(Semaphore::new(permits));
        self.admission_max = permits;
        self.admission_per_tenant_max = admission_per_tenant_permits(permits);
        self
    }

    /// **ADR-018 (tests/déploiements)** : fixe explicitement le plafond par-tenant (borné
    /// au global — un quota de tenant > capacité process n'aurait aucun sens).
    pub fn with_admission_per_tenant(mut self, per_tenant: usize) -> Self {
        self.admission_per_tenant_max = per_tenant.clamp(1, self.admission_max.max(1));
        self
    }

    /// **ADR-018 — ADMISSION ÉQUITABLE**. Tente d'admettre un run pour `tenant`, sous les
    /// **deux** plafonds : quota par-tenant (équité) PUIS capacité process (global). L'ordre
    /// est délibéré — on teste l'équité AVANT de consommer un permit global, sinon un tenant
    /// bavard épuiserait le global avant même que son quota ne le freine. Renvoie un
    /// [`AdmissionPermit`] composite (RAII : libère les deux plafonds à sa chute) ou un
    /// [`AdmitReject`] qualifié. **Tout** refus incrémente le compteur unifié
    /// `admission_rejected` (→ `/metrics`) ; le `reason` distingue la cause côté client.
    /// Sync, non bloquant (`try_acquire`) : aucune file invisible — backpressure VISIBLE.
    pub fn admit(&self, tenant: &str) -> Result<AdmissionPermit, AdmitReject> {
        // Mutex empoisonné (un Drop a paniqué ailleurs — ne devrait jamais arriver) : refus
        // prudent côté capacité, jamais de panique en cascade.
        let mut inflight = self.admission_tenant_inflight.lock().map_err(|_| {
            self.admission_rejected.fetch_add(1, Ordering::Relaxed);
            AdmitReject::GatewayFull
        })?;
        // ① Équité par-tenant : le tenant a-t-il déjà atteint SON quota ?
        let count = inflight.entry(tenant.to_string()).or_insert(0);
        if *count >= self.admission_per_tenant_max {
            self.admission_rejected.fetch_add(1, Ordering::Relaxed);
            return Err(AdmitReject::TenantQuota);
        }
        // ② Capacité process : reste-t-il un permit global ? (consommé SEULEMENT si ①
        //    passe — l'équité garde la porte avant le global).
        let global = self.admission.clone().try_acquire_owned().map_err(|_| {
            self.admission_rejected.fetch_add(1, Ordering::Relaxed);
            AdmitReject::GatewayFull
        })?;
        // Les deux plafonds accordés : on engage le compteur de tenant et on rend le permit
        // composite (son Drop décrémentera ce même compteur).
        *count += 1;
        Ok(AdmissionPermit {
            _global: global,
            inflight: self.admission_tenant_inflight.clone(),
            tenant: tenant.to_string(),
        })
    }

    /// **ADR-018** : nombre de tenants ayant ≥ 1 run en vol (jauge `/metrics`).
    pub fn admission_tenants_active(&self) -> usize {
        self.admission_tenant_inflight.lock().map(|m| m.len()).unwrap_or(0)
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

    /// **Couche 2 (M3, feature `oidc`)** : branche le vérificateur OIDC **frontière** (audience
    /// `efe-llm`). L'extracteur [`crate::auth::LlmCaller`] des routes `/v1/llm/*` valide alors un
    /// JWT `aud=efe-llm` (401 sinon, sans repli keyless). À construire côté binaire via
    /// [`crate::oidc::OidcVerifier::llm_from_env`]. Orthogonal à [`Self::with_oidc`] (essaim).
    #[cfg(feature = "oidc")]
    pub fn with_oidc_llm(mut self, verifier: Arc<crate::oidc::OidcVerifier>) -> Self {
        self.oidc_llm = Some(verifier);
        self
    }

    /// **M2** : pose la politique de capture. Réservé au binaire et aux tests — comme
    /// `with_bundle_export`, ce constructeur évite toute lecture d'environnement depuis l'état,
    /// de sorte qu'un test puisse construire une posture déterministe sans course.
    #[must_use]
    pub fn with_capture_policy(mut self, policy: crate::capture::CapturePolicy) -> Self {
        self.capture_policy = policy;
        self
    }

    /// Remplace le store des runs (ex. `PgRunStore` en prod via la feature `postgres`).
    pub fn with_store(mut self, store: Arc<dyn RunStore>) -> Self {
        self.store = store;
        self
    }

    /// **ADR-019 (MEM-3)** : remplace le store de **mémoire** (ex. `PgMemoryStore` en prod).
    pub fn with_memory_store(mut self, memory: Arc<dyn MemoryStore>) -> Self {
        self.memory = memory;
        self
    }

    /// **Prod** : remplace la source de clé de **provenance**. En production, le binaire
    /// injecte `HmacKeySource::EnvHkdf` (master HKDF lu **server-side** via
    /// `EFE_NANO_AUDIT_HMAC_MASTER`) ; sans ça, le repli `test_key()` signerait avec un
    /// master **public** → la re-preuve (`verify`) serait **forgeable**. Cette clé **signe**
    /// la chaîne ; elle n'est **jamais** transmise ni une clé d'auth provider.
    pub fn with_keys(mut self, keys: HmacKeySource) -> Self {
        // ADR-019 : le store mémoire par défaut HÉRITE de la nouvelle source de clé — sinon, le
        // chemin prod `in_memory().with_keys(EnvHkdf)` laisserait la mémoire scellée sous le
        // test_key d'in_memory() (memory_root incohérent avec `derive_key`). Un
        // `with_memory_store(custom)` APRÈS `with_keys` reste souverain (override explicite).
        self.memory = Arc::new(InMemoryMemoryStore::with_keys(keys.clone()));
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

    /// **Couche 2 (feature `frontier-llm`)** : câble le moteur de coordination multi-LLM
    /// (Claude/Gemini/GPT). À construire depuis l'env **côté binaire** (clés provider
    /// server-side, via `frontier::coordinator_from_env`) puis injecter ici ; `None`
    /// (défaut) ⇒ frontière non servie (les routes `/v1/llm/*` de M3 répondront 501).
    #[cfg(feature = "frontier-llm")]
    pub fn with_coordinator(mut self, coordinator: Arc<efe_coordinator::Coordinator>) -> Self {
        self.coordinator = Some(coordinator);
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

    /// **ADR-014 (CP-2)** : largeur demandée par le client → [`WidthSpec`], plafonnée
    /// serveur (`max_width`). Sans demande client : l'override opérateur
    /// `EFE_SWARM_WIDTH` (lu ICI, côté binaire — le SDK ne lit jamais l'env, garde-fou
    /// G1), sinon `Auto` (cœurs / backend déclaré).
    fn width_spec(&self, requested: Option<usize>) -> WidthSpec {
        match requested {
            Some(w) => WidthSpec::Fixed(w.clamp(1, self.max_width)),
            None => match crate::profile::read::swarm_width() {
                Some(w) => WidthSpec::Fixed(w.clamp(1, self.max_width)),
                None => WidthSpec::Auto,
            },
        }
    }

    /// Fabrique un [`Swarm`] **pour ce tenant**. `allow_remote` n'est `true` que pour le
    /// routeur Network (egress assumé) ; `false` garde la souveraineté (self-host local).
    /// `width` : largeur d'exécution demandée (DTO), plafonnée serveur — I-CP1.
    pub fn swarm_for(
        &self,
        tenant: &str,
        mode: ClientMode,
        allow_remote: bool,
        width: Option<usize>,
    ) -> Result<Swarm, GatewayError> {
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
            .width(self.width_spec(width))
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
        width: Option<usize>,
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
            .width(self.width_spec(width))
            .build()
            .map_err(GatewayError::from)
    }

    /// Dérive la clé de provenance du tenant (server-side) pour re-prouver un run stocké.
    /// **La clé n'est jamais exposée** — seul le `root` l'est.
    pub fn derive_key(&self, tenant: &str) -> Result<Vec<u8>, GatewayError> {
        let tid = self.tenant_id(tenant)?;
        self.keys.derive(tid).map_err(|e| GatewayError::Internal(format!("clé tenant : {e}")))
    }

    /// **Fil P (P.2)** : la clé tenant de la **pile B**, dérivée de celle de la pile A.
    ///
    /// Ce sont **deux clés distinctes**, et ce n'est pas une précaution de façade : réutiliser les
    /// mêmes octets à travers deux schémas de scellement différents est la classe d'erreur qui les
    /// casse tous les deux à la fois. La dérivation passe par le contexte versionné publié par la
    /// pile B (`efe-prov-dag:v1:tenant:`), via `provenance_bridge::stack_b_key` — source unique de
    /// vérité, jamais recopiée ici.
    ///
    /// Propriétés : octets différents de la pile A ; **sens unique** (confier un bundle à un
    /// auditeur ne lui donne pas le pouvoir de forger un reçu de la pile A) ; déterministe ; et
    /// **aucun secret nouveau à garder**.
    pub fn derive_bundle_key(
        &self,
        tenant: &str,
    ) -> Result<provenance_dag::TenantKey, GatewayError> {
        let a = self.derive_key(tenant)?;
        Ok(provenance_bridge::stack_b_key(&a, tenant))
    }

    /// **Fil P (P.2)** : active l'export du bundle de provenance.
    ///
    /// Réservé au câblage explicite (binaire depuis l'env, ou test). Ne lit **pas** l'environnement
    /// lui-même : un test qui manipulerait `EFE_SWARM_BUNDLE_ENABLED` créerait une course avec les
    /// autres tests du même processus.
    #[must_use]
    pub fn with_bundle_export(mut self, enabled: bool) -> Self {
        self.bundle_export = enabled;
        self
    }

    /// L'export du bundle est-il servi ? (Faux par défaut ⟹ route **non montée**.)
    #[must_use]
    pub fn bundle_export_enabled(&self) -> bool {
        self.bundle_export
    }

    /// **Fil P (P.3)** : installe la clé privée d'attestation de racine.
    ///
    /// Sans elle, les bundles sont servis **non attestés** — et le disent. Avec elle, un tiers qui
    /// a épinglé la clé **publique** hors bande peut conclure que la racine servie a bien été
    /// émise ici, sans détenir le moindre secret.
    #[must_use]
    pub fn with_bundle_signer(mut self, signer: provenance_bridge::BundleSigner) -> Self {
        self.bundle_signer = Some(Arc::new(signer));
        self
    }

    /// **Fil P (P.4)** : installe le support de publication de l'ancre.
    ///
    /// Avec lui, chaque bundle servi publie sa racine attendue **ailleurs**, et la réponse porte
    /// `anchored: true`. C'est le seul niveau où une troncature devient détectable par quelqu'un
    /// qui ne détient **aucune clé** — à condition qu'il reçoive le journal par un canal distinct
    /// de celui du bundle.
    #[must_use]
    pub fn with_anchor_sink(mut self, sink: Arc<dyn provenance_bridge::AnchorSink>) -> Self {
        self.anchor_sink = Some(sink);
        self
    }
}

/// **Fil P (P.2)** : l'opt-in d'environnement de l'export de bundle, lu par le **binaire**.
///
/// Fonction libre et non méthode : l'état ne lit jamais l'environnement de lui-même, de sorte
/// qu'un test puisse construire un état déterministe sans course. Convention identique à
/// `EFE_LLM_FRONTIER_ENABLED` : seule la chaîne `"true"` active.
#[must_use]
pub fn bundle_export_from_env() -> bool {
    crate::profile::read::bundle_enabled()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_state_builds_swarms_and_keys() {
        let s = SwarmGatewayState::in_memory();
        assert!(s.swarm_for("system", ClientMode::Keyless, false, None).is_ok());
        assert!(s.swarm_for("system", ClientMode::Keyless, false, Some(4)).is_ok());
        // même clé pour le même tenant (déterministe) → re-preuve cohérente.
        let k1 = s.derive_key("system").unwrap();
        let k2 = s.derive_key("system").unwrap();
        assert_eq!(k1, k2);
        assert!(s.tenant_id("not-a-uuid").is_err());
    }

    #[test]
    fn width_is_capped_server_side_and_admission_is_bounded() {
        // ADR-014 (CP-2) : le plafond serveur mord (un client ne dimensionne pas la
        // machine d'autrui), et l'admission est bornée + FIFO.
        let s = SwarmGatewayState::in_memory().with_admission_permits(2);
        assert_eq!(s.width_spec(Some(10_000)), WidthSpec::Fixed(s.max_width), "plafonné");
        assert_eq!(s.width_spec(Some(0)), WidthSpec::Fixed(1), "plancher 1 (I-CP3)");
        assert_eq!(s.width_spec(None), WidthSpec::Auto);
        assert_eq!(s.admission_max, 2);
        let p1 = s.admission.clone().try_acquire_owned().unwrap();
        let _p2 = s.admission.clone().try_acquire_owned().unwrap();
        assert!(s.admission.clone().try_acquire_owned().is_err(), "3ᵉ run refusé à 2 permits");
        drop(p1);
        assert!(s.admission.clone().try_acquire_owned().is_ok(), "permit libéré → admis");
    }

    #[test]
    fn admit_enforces_per_tenant_fairness_then_global_capacity() {
        // ADR-018 — ÉQUITÉ : global = 3 permits, plafond par-tenant = 2. Un tenant bavard
        // est capé à 2 (même s'il reste de la capacité globale) ; un autre tenant peut
        // toujours obtenir le permit restant — PAS de famine. Le global borne le total.
        let s = SwarmGatewayState::in_memory().with_admission_permits(3).with_admission_per_tenant(2);
        assert_eq!(s.admission_max, 3);
        assert_eq!(s.admission_per_tenant_max, 2);

        // Tenant A prend ses 2 slots (quota), puis se heurte à SON plafond — alors que le
        // global a encore 1 permit libre : c'est l'équité, pas la capacité, qui le freine.
        let a1 = s.admit("alice").unwrap();
        let _a2 = s.admit("alice").unwrap();
        assert_eq!(s.admit("alice").unwrap_err(), AdmitReject::TenantQuota, "A capé par l'équité, pas le global");
        assert_eq!(s.admission_tenants_active(), 1);

        // Tenant B obtient le dernier permit global (équité OK : B est à 0 < 2)…
        let _b1 = s.admit("bob").unwrap();
        assert_eq!(s.admission_tenants_active(), 2, "deux tenants en vol");
        // …puis B se heurte au plafond GLOBAL (3/3), pas à son quota (1 < 2).
        assert_eq!(s.admit("bob").unwrap_err(), AdmitReject::GatewayFull, "global plein");

        // A libère un slot → A peut ré-admettre (quota 1 < 2 ET global a de la place).
        drop(a1);
        let _a3 = s.admit("alice").expect("slot rendu → ré-admission");
        // Tout refus a alimenté le compteur unifié /metrics (2 refus : 1 quota + 1 global).
        assert_eq!(s.admission_rejected.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn per_tenant_cap_defaults_below_global_to_guarantee_headroom() {
        // ADR-018 : le défaut `max(1, global − 1)` laisse toujours ≥ 1 permit pour autrui
        // (aucun tenant ne prend le DERNIER) — sauf à 1 permit, où cap = global = 1.
        assert_eq!(SwarmGatewayState::in_memory().with_admission_permits(8).admission_per_tenant_max, 7);
        assert_eq!(SwarmGatewayState::in_memory().with_admission_permits(2).admission_per_tenant_max, 1);
        assert_eq!(SwarmGatewayState::in_memory().with_admission_permits(1).admission_per_tenant_max, 1);
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
