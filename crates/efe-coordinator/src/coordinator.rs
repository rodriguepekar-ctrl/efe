// SPDX-License-Identifier: UNLICENSED
//! `Coordinator` — pièce centrale : orchestre nano-robots, LLM gros, blackboard, bus, vote, speculator.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::blackboard::PheromoneBlackboard;
use crate::dag::DagExecutor;
use crate::error::{CoordError, CoordResult};
use crate::events::{filter_by_swarm, EventBus, InMemoryBus, StreamEvent, SubscriberToken};
use crate::llm::{MultiLlmBridge, ProviderRegistry};
use crate::ab_test::{ABTestConfig, ABTestRuntime};
use crate::metering::MetricsRegistry;
use crate::multi_tenant::{circuit_key, PolicyRegistry};
use crate::vote::VoteEngine;
use crate::DEFAULT_MAX_CONCURRENCY;

// P4.S1 — types pour les helpers `run_mission_*_streamed` (bridge SSE).
use crate::request::{
    MissionARequest, MissionAResponse, MissionBRequest, MissionBResponse,
    SpeculativeRequest, SpeculativeResponse,
};
use tokio::task::JoinHandle;
use uuid::Uuid;

use efe_nano_app_coordinator::{
    AnalyzerClient, CompressorClient, FilterClient, ReformulatorClient, RouterClient,
    SlmCircuitBreaker, TenantId, VerifierClient,
};

/// Bundle des 6 clients nano-robots Phase 1.
///
/// Tous optionnels au boot : un Coordinator sans Verifier marche encore
/// (mais sans fact-checking).  Un Coordinator sans Reformulator marche
/// aussi (compresseur reçoit le texte brut).
pub struct CoordinatorClients {
    /// Analyseur (NER + intent + sentiment + langue).
    pub analyzer: Option<Arc<AnalyzerClient>>,
    /// Reformulateur (dialecte → canonique).
    pub reformulator: Option<Arc<ReformulatorClient>>,
    /// Compresseur (texte long → court).
    pub compressor: Option<Arc<CompressorClient>>,
    /// Vérificateur (fact-check).
    pub verifier: Option<Arc<VerifierClient>>,
    /// Filtreur (routing pertinence).
    pub filter: Option<Arc<FilterClient>>,
    /// Routeur (sélection LLM Pareto).
    pub router: Option<Arc<RouterClient>>,
    /// **R3.S2 (fix L2)** — Circuit breakers per-SLM (analyzer/compressor/verifier).
    ///
    /// Si `HashMap` vide (default), les workflows n'appliquent **PAS** de
    /// circuit guard et fonctionnent en mode S8 (= comportement R2
    /// préservé pour backward-compat).
    ///
    /// Pour activer la résilience R3.S2, utiliser
    /// [`CoordinatorClients::default_slm_circuits`] ou
    /// [`CoordinatorBuilder::with_default_slm_circuits`].
    ///
    /// Keys typiques : `"analyzer"`, `"compressor"`, `"verifier"`.
    pub slm_circuits: HashMap<String, SlmCircuitBreaker>,
}

impl Default for CoordinatorClients {
    fn default() -> Self {
        Self {
            analyzer: None,
            reformulator: None,
            compressor: None,
            verifier: None,
            filter: None,
            router: None,
            slm_circuits: HashMap::new(),
        }
    }
}

impl CoordinatorClients {
    /// Builder vide.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Helper : tous les clients renseignés ?
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.analyzer.is_some()
            && self.reformulator.is_some()
            && self.compressor.is_some()
            && self.verifier.is_some()
            && self.filter.is_some()
            && self.router.is_some()
    }

    /// **R3.S2** — Helper pour créer un `HashMap` par défaut avec les 3 SLM
    /// circuits préconfigurés (threshold=3 échecs, reset_timeout=10s).
    ///
    /// Robots protégés : `analyzer`, `compressor`, `verifier` (les 3 SLM
    /// Phase 1 backed par `llama-server` Metal). Reformulator/filter/router
    /// NON protégés (cf. §1.4 architecture R3.S2).
    ///
    /// ## Exemple
    /// ```ignore
    /// use efe_coordinator::CoordinatorClients;
    ///
    /// let mut clients = CoordinatorClients::default();
    /// clients.slm_circuits = CoordinatorClients::default_slm_circuits();
    /// // 3 circuits : analyzer/compressor/verifier, threshold=3, timeout=10s.
    /// ```
    #[must_use]
    pub fn default_slm_circuits() -> HashMap<String, SlmCircuitBreaker> {
        let mut m = HashMap::new();
        for robot in ["analyzer", "compressor", "verifier"] {
            m.insert(
                robot.to_string(),
                SlmCircuitBreaker::new(3, Duration::from_secs(10)),
            );
        }
        m
    }

    /// **R3.S2** — Lookup helper pour récupérer un circuit par nom de robot.
    ///
    /// Retourne `None` si le circuit n'est pas configuré (= mode S8
    /// backward-compat → le workflow appelle directement le client sans
    /// guard).
    #[must_use]
    pub fn slm_circuit(&self, robot: &str) -> Option<&SlmCircuitBreaker> {
        self.slm_circuits.get(robot)
    }

    /// **R3.S5** — Pré-génère des circuits keyed `"{tenant}:{robot}"` pour une
    /// liste de tenants, afin d'**isoler le blast radius** (un tenant qui ouvre
    /// son circuit `analyzer` n'affecte pas les autres tenants — fix L5-d).
    ///
    /// Les 3 robots SLM (analyzer/compressor/verifier) sont déclarés pour
    /// chaque tenant avec le `threshold` fourni (reset_timeout 10 s, aligné
    /// R3.S2). Les déploiements mono-tenant n'en ont pas besoin : sans clé
    /// `"{tenant}:{robot}"`, [`Coordinator::slm_circuit_for`] retombe sur la
    /// clé `"{robot}"` seule (sémantique R3.S2 préservée).
    ///
    /// ## Exemple
    /// ```ignore
    /// let circuits = CoordinatorClients::tenant_slm_circuits(&[tenant_a, tenant_b], 3);
    /// let coord = Coordinator::builder().slm_circuits(circuits).build()?;
    /// ```
    #[must_use]
    pub fn tenant_slm_circuits(
        tenants: &[TenantId],
        threshold: u32,
    ) -> HashMap<String, SlmCircuitBreaker> {
        let mut m = HashMap::new();
        for tenant in tenants {
            for robot in ["analyzer", "compressor", "verifier"] {
                m.insert(
                    circuit_key(*tenant, robot, true),
                    SlmCircuitBreaker::new(threshold, Duration::from_secs(10)),
                );
            }
        }
        m
    }
}

/// Configuration du Coordinator.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// Parallélisme max du DAG executor.
    pub max_concurrency: usize,
    /// Deadline globale par défaut.
    pub default_deadline: Duration,
    /// Multi-tenant ID par défaut (peut être override per-call).
    pub default_tenant: uuid::Uuid,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            default_deadline: Duration::from_secs(30),
            default_tenant: uuid::Uuid::nil(),
        }
    }
}

/// Pièce centrale du Phase 2 — orchestre tout.
///
/// Construit via [`CoordinatorBuilder`].
#[derive(Clone)]
pub struct Coordinator {
    /// Configuration.
    pub config: CoordinatorConfig,
    /// Clients nano-robots Phase 1.
    pub clients: Arc<CoordinatorClients>,
    /// Bridge multi-LLM.
    pub bridge: MultiLlmBridge,
    /// Blackboard partagée (shared par toutes les missions concurrentes).
    pub blackboard: PheromoneBlackboard,
    /// Bus d'évènements (in-process par défaut).
    pub bus: Arc<dyn EventBus>,
    /// Executeur DAG (réutilisable).
    pub dag: DagExecutor,
    /// Moteur de vote (par défaut uniform).
    pub vote: VoteEngine,
    /// **R3.S5** — Politiques par tenant. Registre vide par défaut (=
    /// déploiement mono-tenant : tous les lookups → `None` ⇒ comportement
    /// global R3.S4, backward-compat). `Arc` pour préserver `Clone` cheap.
    pub policies: Arc<PolicyRegistry>,
    /// **R3.S7** — Registre de télémétrie Prometheus. `None` (défaut) ⇒
    /// télémétrie off, overhead nul (le hook RAII est inerte). `Arc` partagé
    /// avec le serveur `/metrics`.
    pub metrics: Option<Arc<MetricsRegistry>>,
    /// **R3.S8** — Runtime d'A/B test de tables de scoring. `None` (défaut) ⇒
    /// pas d'A/B (résolution R3.S5/S7 inchangée). `Arc` pour `Clone` cheap.
    pub ab_test: Option<Arc<ABTestRuntime>>,
}

/// Builder fluide pour `Coordinator`.
pub struct CoordinatorBuilder {
    config: CoordinatorConfig,
    clients: CoordinatorClients,
    registry: Option<ProviderRegistry>,
    bus: Option<Arc<dyn EventBus>>,
    vote: Option<VoteEngine>,
    /// **R3.S5** — registre de politiques tenant (défaut : vide).
    policies: Option<PolicyRegistry>,
    /// **R3.S7** — registre de télémétrie (défaut : None = off).
    metrics: Option<Arc<MetricsRegistry>>,
    /// **R3.S8** — runtime A/B test (défaut : None = off).
    ab_test: Option<Arc<ABTestRuntime>>,
}

impl Default for CoordinatorBuilder {
    fn default() -> Self {
        Self {
            config: CoordinatorConfig::default(),
            clients: CoordinatorClients::default(),
            registry: None,
            bus: None,
            vote: None,
            policies: None,
            metrics: None,
            ab_test: None,
        }
    }
}

impl CoordinatorBuilder {
    /// Configure.
    #[must_use]
    pub fn config(mut self, c: CoordinatorConfig) -> Self {
        self.config = c;
        self
    }

    /// Set des clients.
    #[must_use]
    pub fn clients(mut self, c: CoordinatorClients) -> Self {
        self.clients = c;
        self
    }

    /// Set du registre LLM (sinon vide).
    #[must_use]
    pub fn registry(mut self, r: ProviderRegistry) -> Self {
        self.registry = Some(r);
        self
    }

    /// Set du bus (sinon `InMemoryBus`).
    #[must_use]
    pub fn bus(mut self, b: Arc<dyn EventBus>) -> Self {
        self.bus = Some(b);
        self
    }

    /// Set du VoteEngine.
    #[must_use]
    pub fn vote(mut self, v: VoteEngine) -> Self {
        self.vote = Some(v);
        self
    }

    /// **R3.S2 (fix L2)** — Active les 3 SLM circuit breakers par défaut
    /// (analyzer/compressor/verifier, threshold=3, reset_timeout=10s).
    ///
    /// Sans cet appel, le `Coordinator` fonctionne en mode S8 (pas de
    /// circuit → comportement R2 préservé pour backward-compat).
    ///
    /// ## Exemple
    /// ```ignore
    /// let coord = Coordinator::builder()
    ///     .with_default_slm_circuits()
    ///     .build()?;
    /// ```
    #[must_use]
    pub fn with_default_slm_circuits(mut self) -> Self {
        self.clients.slm_circuits = CoordinatorClients::default_slm_circuits();
        self
    }

    /// **R3.S2** — Override avec une `HashMap` de circuits personnalisée.
    ///
    /// Utile pour les tests déterministes qui ont besoin de seuils
    /// différents (e.g. `threshold=1` pour ouverture immédiate, ou
    /// `reset_timeout=10ms` pour test temporel rapide).
    ///
    /// ## Exemple
    /// ```ignore
    /// use efe_nano_app_coordinator::SlmCircuitBreaker;
    /// use std::collections::HashMap;
    /// use std::time::Duration;
    ///
    /// let mut circuits = HashMap::new();
    /// circuits.insert(
    ///     "analyzer".to_string(),
    ///     SlmCircuitBreaker::new(1, Duration::from_millis(50)),
    /// );
    /// let coord = Coordinator::builder()
    ///     .slm_circuits(circuits)
    ///     .build()?;
    /// ```
    #[must_use]
    pub fn slm_circuits(mut self, circuits: HashMap<String, SlmCircuitBreaker>) -> Self {
        self.clients.slm_circuits = circuits;
        self
    }

    /// **R3.S4** — Enregistre un drafter SLM local + son circuit breaker.
    ///
    /// 1. Enregistre le `LocalSlmProvider` dans le registry (résolu ensuite
    ///    par `SpeculativeRequest.drafter_llm` = l'ID du provider).
    /// 2. Crée un `SlmCircuitBreaker` (threshold=3, reset=10s) pour cet ID,
    ///    que `run_speculative` passera au `Speculator` (skip drafter si le
    ///    llama-server local est mort → verifier-only).
    ///
    /// ## Exemple
    /// ```ignore
    /// use efe_coordinator::{Coordinator, LocalSlmProvider};
    ///
    /// let drafter = LocalSlmProvider::from_env()?;
    /// let coord = Coordinator::builder()
    ///     .with_local_slm_drafter(drafter)
    ///     .build()?;
    /// // SpeculativeRequest.drafter_llm = "local_gemma-2-1.5b"
    /// ```
    #[cfg(feature = "local-slm")]
    #[must_use]
    pub fn with_local_slm_drafter(
        mut self,
        provider: crate::llm::LocalSlmProvider,
    ) -> Self {
        // Appel trait-qualifié : `LlmProvider` n'est pas importé au niveau
        // module (éviterait un warning unused quand `local-slm` est off), et
        // `LocalSlmProvider` possède un champ privé `id` qui masquerait la
        // méthode du trait par simple `.id()`.
        let id = crate::llm::LlmProvider::id(&provider);
        self.registry
            .get_or_insert_with(ProviderRegistry::new)
            .register(Arc::new(provider));
        self.clients
            .slm_circuits
            .insert(id, SlmCircuitBreaker::new(3, Duration::from_secs(10)));
        self
    }

    /// **R3.S5** — Installe le registre de politiques multi-tenant.
    ///
    /// Sans cet appel, le `Coordinator` est mono-tenant : tous les lookups
    /// `policies.get()` renvoient `None` → comportement global R3.S4 préservé.
    ///
    /// ## Exemple
    /// ```ignore
    /// use efe_coordinator::{Coordinator, PolicyRegistry, TenantPolicies};
    ///
    /// let reg = PolicyRegistry::new()
    ///     .with(TenantPolicies::open(tenant_a))
    ///     .with(TenantPolicies { allowed_llms: Some(vec!["gemini".into()]),
    ///                            ..TenantPolicies::open(tenant_b) });
    /// let coord = Coordinator::builder().policies(reg).build()?;
    /// ```
    #[must_use]
    pub fn policies(mut self, reg: PolicyRegistry) -> Self {
        self.policies = Some(reg);
        self
    }

    /// **R3.S7** — Installe le registre de télémétrie Prometheus.
    ///
    /// Sans cet appel, `metrics = None` ⇒ aucune métrique enregistrée (overhead
    /// nul). Le même `Arc<MetricsRegistry>` est passé au serveur `/metrics`
    /// (`metrics_server::serve_metrics`).
    ///
    /// ## Exemple
    /// ```ignore
    /// use std::sync::Arc;
    /// use efe_coordinator::{Coordinator, MetricsRegistry};
    ///
    /// let reg = Arc::new(MetricsRegistry::new());
    /// let coord = Coordinator::builder().metrics(reg.clone()).build()?;
    /// // tokio::spawn(efe_coordinator::metrics_server::serve_metrics(addr, reg, coord.clients.clone()));
    /// ```
    #[must_use]
    pub fn metrics(mut self, reg: Arc<MetricsRegistry>) -> Self {
        self.metrics = Some(reg);
        self
    }

    /// **R3.S8** — Installe un A/B test de tables de scoring.
    ///
    /// La config (tables *owned*, sérialisable) est convertie une fois en
    /// [`ABTestRuntime`] (tables `Arc` → résolution O(1)/mission). En Mission A,
    /// si la requête n'épingle pas de table, l'A/B assigne control/experiment
    /// selon `blake3(swarm_id)` (précédence `request > A/B > tenant > global`).
    ///
    /// ## Exemple
    /// ```ignore
    /// use efe_coordinator::{Coordinator, ABTestConfig};
    ///
    /// let cfg = ABTestConfig { control_table, experiment_table,
    ///     experiment_traffic_ratio: 0.5, tenant_filter: None, fixture_filter: None };
    /// let coord = Coordinator::builder().ab_test(cfg).build()?;
    /// ```
    #[must_use]
    pub fn ab_test(mut self, cfg: ABTestConfig) -> Self {
        self.ab_test = Some(Arc::new(ABTestRuntime::from_config(cfg)));
        self
    }

    /// Build final.
    pub fn build(self) -> CoordResult<Coordinator> {
        if self.config.max_concurrency == 0 {
            return Err(CoordError::Config(
                "max_concurrency must be >= 1".into(),
            ));
        }
        // R3.S2 (§9 R7 mitigation) — Warn au boot si l'opérateur a configuré
        // des SLM clients (analyzer/compressor/verifier) mais N'A PAS activé
        // les circuit breakers. C'est probablement un oubli en production :
        // sans circuits, les workflows fonctionnent en mode S8 (= comportement
        // R2 préservé pour backward-compat), MAIS la résilience aux SLM
        // crashes n'est pas activée → cascade de timeouts possible.
        let has_any_slm_client = self.clients.analyzer.is_some()
            || self.clients.compressor.is_some()
            || self.clients.verifier.is_some();
        if has_any_slm_client && self.clients.slm_circuits.is_empty() {
            tracing::warn!(
                target: "efe_coordinator::slm_circuit",
                "Coordinator built with SLM clients (analyzer/compressor/verifier) \
                 but WITHOUT slm_circuits configured. Workflows will run in S8 \
                 mode (no circuit protection). For production resilience, call \
                 `.with_default_slm_circuits()` on the builder (see R3.S2 \
                 architecture for fix L2)."
            );
        }
        let registry = self.registry.unwrap_or_default();
        let bridge = MultiLlmBridge::new(registry);
        let bus: Arc<dyn EventBus> = self.bus.unwrap_or_else(|| Arc::new(InMemoryBus::new()));
        Ok(Coordinator {
            dag: DagExecutor::new(self.config.max_concurrency),
            config: self.config,
            clients: Arc::new(self.clients),
            bridge,
            blackboard: PheromoneBlackboard::new(),
            bus,
            vote: self.vote.unwrap_or_else(VoteEngine::uniform),
            policies: Arc::new(self.policies.unwrap_or_default()),
            metrics: self.metrics,
            ab_test: self.ab_test,
        })
    }
}

impl Coordinator {
    /// Construit un builder.
    #[must_use]
    pub fn builder() -> CoordinatorBuilder {
        CoordinatorBuilder::default()
    }

    /// Coordinator minimal : aucun nano-robot, aucun provider, bus in-mem.
    /// Utile pour les tests qui ciblent un sous-composant.
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            config: CoordinatorConfig::default(),
            clients: Arc::new(CoordinatorClients::default()),
            bridge: MultiLlmBridge::new(ProviderRegistry::new()),
            blackboard: PheromoneBlackboard::new(),
            bus: Arc::new(InMemoryBus::new()),
            dag: DagExecutor::default(),
            vote: VoteEngine::uniform(),
            policies: Arc::new(PolicyRegistry::new()),
            metrics: None,
            ab_test: None,
        }
    }

    /// **R3.S5** — Résout le circuit SLM d'un robot pour un tenant donné.
    ///
    /// - Si `has_policy` (le tenant a une policy), on cherche d'abord la clé
    ///   isolée `"{tenant}:{robot}"`. Si absente (circuits tenant non
    ///   pré-déclarés), on retombe sur la clé partagée `"{robot}"`.
    /// - Si `!has_policy` (mono-tenant), on utilise directement `"{robot}"`
    ///   (sémantique R3.S2 inchangée).
    ///
    /// Cette dégradation propre garantit que l'absence de circuits tenant
    /// pré-déclarés n'introduit pas de panne : on retrouve la garde partagée.
    #[must_use]
    pub fn slm_circuit_for(
        &self,
        tenant: TenantId,
        robot: &str,
        has_policy: bool,
    ) -> Option<&SlmCircuitBreaker> {
        if has_policy {
            let keyed = circuit_key(tenant, robot, true);
            if let Some(cb) = self.clients.slm_circuits.get(&keyed) {
                return Some(cb);
            }
        }
        self.clients.slm_circuit(robot)
    }

    /// Souscrit au bus.  Helper pour les tests.
    pub fn subscribe(
        &self,
        name: Option<&str>,
    ) -> (
        crate::events::SubscriberToken,
        tokio::sync::mpsc::Receiver<crate::events::StreamEvent>,
    ) {
        self.bus.subscribe(name)
    }

    /// **P4.S1** — Démarre une **Mission A** et renvoie un flux d'événements
    /// live ([`MissionStream`]), sans course.
    ///
    /// Séquence : génère le `swarm_id`, s'abonne au bus **filtré** sur ce
    /// `swarm_id` AVANT de lancer la mission, `tokio::spawn` la mission (id
    /// injecté), puis renvoie le [`MissionStream`] (events + join + cleanup RAII).
    ///
    /// L'abonnement précédant le spawn, `SwarmStarted` (premier publish de la
    /// mission) est **toujours** capté. Le receiver ne reçoit QUE les événements
    /// de ce swarm (`filter_by_swarm`). Requiert un `Arc<Coordinator>` (la
    /// mission est `tokio::spawn`-ée → `'static`).
    #[must_use]
    pub fn run_mission_a_streamed(
        self: &Arc<Self>,
        req: MissionARequest,
    ) -> MissionStream<MissionAResponse> {
        let swarm_id = Uuid::now_v7();
        // `None` ⇒ token UUID unique : deux missions concurrentes ne
        // collisionnent jamais dans la DashMap du bus (un nom fixe le ferait).
        let (token, events) = self
            .bus
            .subscribe_filtered(None, filter_by_swarm(swarm_id));
        let coord = Arc::clone(self);
        let join = tokio::spawn(async move {
            crate::workflows::run_mission_a_with_swarm_id(&coord, req, swarm_id).await
        });
        MissionStream {
            swarm_id,
            events,
            join,
            _unsub: Unsubscriber {
                bus: Arc::clone(&self.bus),
                token,
            },
        }
    }

    /// **P4.S1** — Idem pour une **Mission B** (fan-out N LLMs + vote).
    #[must_use]
    pub fn run_mission_b_streamed(
        self: &Arc<Self>,
        req: MissionBRequest,
    ) -> MissionStream<MissionBResponse> {
        let swarm_id = Uuid::now_v7();
        let (token, events) = self
            .bus
            .subscribe_filtered(None, filter_by_swarm(swarm_id));
        let coord = Arc::clone(self);
        let join = tokio::spawn(async move {
            crate::workflows::run_mission_b_with_swarm_id(&coord, req, swarm_id).await
        });
        MissionStream {
            swarm_id,
            events,
            join,
            _unsub: Unsubscriber {
                bus: Arc::clone(&self.bus),
                token,
            },
        }
    }

    /// **P4.S1** — Idem pour une **Mission speculative** (drafter + verifier).
    #[must_use]
    pub fn run_speculative_streamed(
        self: &Arc<Self>,
        req: SpeculativeRequest,
    ) -> MissionStream<SpeculativeResponse> {
        let swarm_id = Uuid::now_v7();
        let (token, events) = self
            .bus
            .subscribe_filtered(None, filter_by_swarm(swarm_id));
        let coord = Arc::clone(self);
        let join = tokio::spawn(async move {
            crate::workflows::run_speculative_with_swarm_id(&coord, req, swarm_id).await
        });
        MissionStream {
            swarm_id,
            events,
            join,
            _unsub: Unsubscriber {
                bus: Arc::clone(&self.bus),
                token,
            },
        }
    }
}

/// **P4.S1** — Flux d'événements live d'une mission + sa réponse finale.
///
/// Retourné par `Coordinator::run_mission_{a,b,speculative}_streamed`. Conçu
/// pour le bridge SSE (Phase 4) :
///
/// - **`events`** : `Receiver` filtré sur le `swarm_id` (ne reçoit QUE cette
///   mission). À consommer en boucle (`events.recv().await`) jusqu'à
///   `SwarmFinished`/`SwarmFailed`.
/// - **`join`** : `JoinHandle` de la mission → réponse typée finale (audit/log
///   serveur). S'attend via `&mut self.join` (`JoinHandle` est `Unpin`).
/// - **`swarm_id`** : corrélation cross-événements.
///
/// ## Cleanup RAII (indispensable)
///
/// `Drop` appelle `bus.unsubscribe(token)`. C'est **obligatoire** : `InMemoryBus`
/// ne prune PAS les receivers fermés au `publish` (il attend un `unsubscribe`
/// explicite — cf. `events/inmem.rs`). Sans ce Drop, **chaque mission fuiterait
/// un subscriber** dans la `DashMap` du bus.
///
/// Si le `MissionStream` est droppé tôt (client SSE déconnecté), la mission
/// spawnée **continue** côté serveur (cohérence audit/billing) ; ses publishes
/// sont simplement ignorés (subscriber retiré).
pub struct MissionStream<R> {
    /// `swarm_id` de la mission (corrélation).
    pub swarm_id: Uuid,
    /// Canal des `StreamEvent` filtrés sur ce `swarm_id`. **Movable** : le
    /// consumer peut le déplacer (ex. boucle de stream SSE).
    pub events: tokio::sync::mpsc::Receiver<StreamEvent>,
    /// Handle de la mission spawnée → réponse finale (ou erreur). **Movable** :
    /// `let resp = stream.join.await;` est permis (cf. garde interne ci-dessous).
    pub join: JoinHandle<CoordResult<R>>,
    /// **Garde de cleanup RAII** (champ privé). C'est CE champ — et non
    /// `MissionStream` lui-même — qui implémente `Drop` : ainsi `events` et
    /// `join` restent *movables* (on ne peut pas `move` hors d'un type qui impl
    /// `Drop`), tout en garantissant le `unsubscribe` au scope-end.
    _unsub: Unsubscriber,
}

/// Garde RAII : retire le subscriber du bus au `Drop`.
///
/// **Indispensable** : `InMemoryBus` ne prune PAS les receivers fermés au
/// `publish` (il attend un `unsubscribe` explicite — cf. `events/inmem.rs`).
/// Sans cette garde, chaque mission fuiterait un subscriber dans la `DashMap`.
/// Isolée dans un champ dédié pour ne pas rendre `MissionStream` non-movable.
struct Unsubscriber {
    bus: Arc<dyn EventBus>,
    token: SubscriberToken,
}

impl Drop for Unsubscriber {
    fn drop(&mut self) {
        self.bus.unsubscribe(&self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_default_works() {
        let c = Coordinator::builder().build().unwrap();
        assert_eq!(c.config.max_concurrency, DEFAULT_MAX_CONCURRENCY);
        assert!(c.blackboard.is_empty());
        assert_eq!(c.bridge.registry().len(), 0);
    }

    #[test]
    fn builder_with_zero_concurrency_errors() {
        let cfg = CoordinatorConfig {
            max_concurrency: 0,
            ..Default::default()
        };
        let r = Coordinator::builder().config(cfg).build();
        assert!(matches!(r, Err(CoordError::Config(_))));
    }

    #[test]
    fn for_tests_returns_valid_coordinator() {
        let c = Coordinator::for_tests();
        assert!(c.clients.analyzer.is_none());
        assert!(c.blackboard.is_empty());
    }

    #[test]
    fn clients_empty_is_default() {
        let cs = CoordinatorClients::empty();
        assert!(!cs.is_complete());
        assert!(cs.analyzer.is_none());
    }

    #[tokio::test]
    async fn subscribe_then_publish_via_bus() {
        let c = Coordinator::for_tests();
        let (_t, mut rx) = c.subscribe(Some("test"));
        let swarm = uuid::Uuid::now_v7();
        let ev = crate::events::StreamEvent::Heartbeat {
            ts: chrono::Utc::now(),
        };
        c.bus.publish(ev).await.unwrap();
        let _ = swarm;
        let got = rx.recv().await.unwrap();
        assert_eq!(got.tag(), "HEARTBEAT");
    }
}
