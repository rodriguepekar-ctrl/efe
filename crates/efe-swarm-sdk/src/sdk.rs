// SPDX-License-Identifier: UNLICENSED
//! `Swarm` — l'entrée unique : `swarm.run(task, topology, budget)`.

use std::sync::Arc;

use efe_blackboard::{SubstrateMetrics, WriteSink};
use efe_nano_sdk::{SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, AgentKind, HmacKeySource, ProvenanceMetrics, ProvenanceRecorder, ProvenanceSink,
};
use efe_swarm_topology::{AgentFleet, SwarmContext};

use crate::agents::Meter;
use crate::backend::{build_local_backend, LocalBackend};
use crate::budget::{Budget, BudgetGuard, OnExceed};
use crate::n17::LlmProvenanceCollector;
use crate::client::{ClientMode, WidthSpec};
use crate::observer::{SwarmObserver, TeeSink};
use crate::resolver::Resolver;
use crate::result::{Accounting, ProvenanceSummary, RunOutcome, SwarmError, SwarmRun};
use crate::spec::TopologySpec;

/// L'essaim productisé. Construit via [`Swarm::builder`] ou [`Swarm::keyless`].
pub struct Swarm {
    fleet: Arc<AgentFleet>,
    mode: ClientMode,
    tenant: TenantId,
    key: Vec<u8>, // clé de provenance pré-dérivée (HKDF par tenant)
    provenance: bool,
    assumed_subtasks: usize,
    /// Métriques de provenance partagées (alimentées par chaque run) — pour `/metrics`.
    metrics: Option<Arc<ProvenanceMetrics>>,
    /// Métriques de substrat partagées (Blackboard/EventBus des topologies) — pour `/metrics`.
    substrate_metrics: Option<Arc<SubstrateMetrics>>,
    /// Observateur de cycle de vie (dashboard live, Inc.3) — `None` ⇒ comportement Inc.1/2.
    observer: Option<Arc<dyn SwarmObserver>>,
    /// **Garde-fou souveraineté (G2)** : autorise un backend self-host **non-local**
    /// (egress). `false` par défaut — un endpoint non-`127.0.0.1`/`localhost` est refusé.
    allow_remote_backend: bool,
    /// **Health-check (S.a)** : sonde le backend self-host au démarrage (échec rapide si
    /// injoignable). `false` par défaut (compatibilité).
    require_healthy_backend: bool,
    /// **ADR-014 (CP-1)** : largeur d'exécution des vagues — `Auto` = cœurs (Mode A) /
    /// capacité déclarée du backend (Mode C). Politique d'exécution, jamais une
    /// sémantique (I-CP1) : la racine de provenance n'en dépend pas.
    width: WidthSpec,
}

impl Swarm {
    pub fn builder() -> SwarmBuilder {
        SwarmBuilder::default()
    }

    /// Essaim **keyless** par défaut (flotte standard, provenance activée). Le mode
    /// démo/dev en une ligne.
    pub fn keyless() -> Self {
        SwarmBuilder::default().build().expect("clé de test toujours dérivable")
    }

    pub fn mode(&self) -> &ClientMode {
        &self.mode
    }
    pub fn fleet(&self) -> &Arc<AgentFleet> {
        &self.fleet
    }

    /// **`swarm.run(task, topology, budget)`** — exécute un essaim, plafonné en coût et
    /// **auto-tracé** : renvoie la sortie, le coût exact, et un `provenance_root`
    /// re-vérifiable par un tiers. C'est le wrapper Inc.1 (il jette la `SealedProvenance`).
    pub async fn run(
        &self,
        task: serde_json::Value,
        topology: TopologySpec,
        budget: Budget,
    ) -> Result<SwarmRun, SwarmError> {
        self.run_traced(task, topology, budget).await.map(|o| o.run)
    }

    /// **`swarm.run_traced(...)`** — identique à [`Swarm::run`], mais **expose la
    /// provenance scellée** ([`RunOutcome::sealed`]) : les `AgentCallRecord` bruts que la
    /// passerelle HTTP persiste pour servir `GET /v1/swarm/verify/:id` (le *moat* en HTTP).
    /// `run()` n'est qu'un wrapper qui jette `sealed`.
    pub async fn run_traced(
        &self,
        task: serde_json::Value,
        topology: TopologySpec,
        budget: Budget,
    ) -> Result<RunOutcome, SwarmError> {
        // ClientMode::Network nécessite la passerelle serveur (clés) → pas d'in-process.
        if matches!(self.mode, ClientMode::Network { .. }) {
            return Err(SwarmError::NetworkRequiresGateway);
        }
        let price = budget.price_per_call_cents.unwrap_or_else(|| self.mode.price_per_call_cents());

        // 0. concrétise Auto (selon tâche + mode) + applique la politique de PRÉ-VOL.
        let spec = topology.concretize(&task, &self.mode);
        let spec = match budget.on_exceed {
            OnExceed::Degrade => spec.degraded_to_fit(budget.max_cents, price, self.assumed_subtasks),
            OnExceed::Error => {
                // Revue ADR-014 : arithmétique u64 SANS cast tronquant — un produit
                // k×rounds énorme doit être REFUSÉ net, jamais admis par débordement.
                let est = (spec.estimated_calls(self.assumed_subtasks) as u64).saturating_mul(price as u64);
                if est > budget.max_cents as u64 {
                    return Err(SwarmError::BudgetTooLow {
                        needed_cents: u32::try_from(est).unwrap_or(u32::MAX),
                        max_cents: budget.max_cents,
                    });
                }
                spec
            }
            OnExceed::Stop => spec,
        };

        // 1. mode → backend d'agents + nature (Mode A flotte / Mode C self-host). En
        //    self-host, le garde-fou souveraineté (G2) REFUSE un endpoint non-local par
        //    défaut (egress interdit) ; le backend est enveloppé d'un circuit-breaker (G3).
        let (backend, kind): (Option<Arc<dyn LocalBackend>>, AgentKind) = match &self.mode {
            ClientMode::Keyless => (None, AgentKind::ModeA),
            ClientMode::SelfHost { endpoint, model } => {
                (Some(build_local_backend(endpoint, model, self.allow_remote_backend)?), AgentKind::ModeC)
            }
            ClientMode::Network { .. } => unreachable!("écarté en tête de run"),
        };
        // Health-check (S.a) : sonde rapide du backend local — échec net plutôt que N timeouts.
        if self.require_healthy_backend {
            if let Some(b) = &backend {
                if !b.health().await {
                    return Err(SwarmError::BackendUnavailable { endpoint: b.descriptor().endpoint });
                }
            }
        }
        // Forensique : descripteur du backend local (pont N17) — capté AVANT le move.
        let backend_info = backend.as_ref().map(|b| {
            let d = b.descriptor();
            crate::result::BackendInfo { engine: d.kind.as_str().to_string(), model: d.model, endpoint: d.endpoint }
        });

        // 2. provenance + budget : recorder, sink (→ contexte), garde de budget.
        let swarm_id = SwarmId::new();
        let recorder = Arc::new(ProvenanceRecorder::new(swarm_id, self.tenant));
        let guard = Arc::new(BudgetGuard::new(budget.max_cents, price));
        // Pont N17 par-appel : un collecteur de `LlmProvenanceRecord` pour les runs Mode C
        // (self-host). Mode A (keyless) ⇒ aucun appel LLM, aucun collecteur.
        let n17 = matches!(kind, AgentKind::ModeC).then(LlmProvenanceCollector::new);
        let meter = Meter {
            rec: recorder.clone(),
            guard: guard.clone(),
            kind,
            observer: self.observer.clone(),
            n17: n17.clone(),
            swarm_id,
            tenant: self.tenant,
        };

        // Inc.3 : si un observateur fournit un exutoire, on TEE provenance + observateur
        // (la provenance reste byte-identique) ; sinon, provenance seule (Inc.1/2 inchangé).
        let prov_sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));
        let sink: Arc<dyn WriteSink> = match self.observer.as_ref().and_then(|o| o.write_sink()) {
            Some(dash) => Arc::new(TeeSink(vec![prov_sink, dash])),
            None => prov_sink,
        };
        let mut ctx = SwarmContext::new(self.fleet.clone()).with_write_sink(sink);
        if let Some(sm) = &self.substrate_metrics {
            ctx = ctx.with_substrate_metrics(sm.clone());
        }
        if let Some(tap) = self.observer.as_ref().and_then(|o| o.event_tap()) {
            ctx = ctx.with_event_tap(tap); // Inc.3 : trafic inter-agent → dashboard
        }

        // 2.b — ADR-014 (CP-1) : résout la LARGEUR d'exécution du run, par mode
        // (Mode A : cœurs mesurés ; Mode C : capacité déclarée du backend ; env
        // EFE_SWARM_WIDTH prioritaire). Descendue dans chaque topologie au resolve.
        let width = self.width.resolve(&self.mode, backend.as_ref().map(|b| b.suggested_width()));

        // 3. assemble la topologie (agents flotte OU LLM, enveloppés) + exécute.
        let resolver = Resolver { fleet: &self.fleet, backend, meter, width };
        let topo = spec.resolve(&resolver)?;
        let topo_name = topo.name().to_string();
        if let Some(o) = &self.observer {
            o.on_run_start(&topo_name, &self.mode.label(), budget.max_cents, self.assumed_subtasks);
        }
        let out = topo.run(&task, &ctx).await;

        // 3. budget POST-VOL (politique Error : échec si épuisé en cours).
        if budget.on_exceed == OnExceed::Error && guard.exhausted() {
            return Err(SwarmError::BudgetExceeded { spent_cents: guard.spent_cents(), max_cents: guard.max_cents() });
        }

        // Pont N17 par-appel : les `LlmProvenanceRecord` forensiques collectés (Mode C).
        let llm_records = n17.as_ref().map(|c| c.records()).unwrap_or_default();

        // 4. scelle la provenance + re-prouve (verify_swarm) → provenance_root. La
        //    `SealedProvenance` est conservée (records bruts) pour la persistance HTTP ;
        //    les métriques partagées (si présentes) sont alimentées ici → `/metrics`.
        let (provenance, sealed, n17_summary) = if self.provenance {
            let sealed = recorder.seal_with_key(self.key.clone());
            let report = verify_swarm(&sealed.records, &self.key);
            if let Some(m) = &self.metrics {
                m.observe_sealed(&sealed);
                m.observe_verify(&report);
            }
            let summary = ProvenanceSummary {
                enabled: true,
                records: sealed.len(),
                traced_pct: report.traced_pct,
                verified: report.ok(),
                root: report.root_hex(),
            };
            if let Some(o) = &self.observer {
                o.on_seal(sealed.len(), report.traced_pct, summary.root.as_deref(), report.ok());
            }
            // N17 : vérifie le pont (chaque record Mode C lié à un LlmProvenanceRecord cohérent).
            let n17_summary = build_n17_summary(&sealed.records, &llm_records);
            (summary, Some(sealed), n17_summary)
        } else {
            (ProvenanceSummary::disabled(), None, None)
        };

        let run = SwarmRun {
            output: out.output,
            topology: topo_name,
            accounting: Accounting {
                agents_used: out.agents_used,
                cost_units: out.cost_units,
                elapsed_ms: out.elapsed_ms,
                width, // ADR-014 : la largeur résolue, rapportée par-run (jamais une sémantique)
            },
            budget: guard.snapshot(),
            provenance,
            quality: out.quality.into(),
            budget_stopped: budget.on_exceed == OnExceed::Stop && guard.exhausted(),
            backend: backend_info,
            n17: n17_summary,
        };
        if let Some(o) = &self.observer {
            o.on_finish(&run);
        }
        Ok(RunOutcome { run, sealed, llm_records })
    }
}

/// Vérifie le **pont N17** sur un run : chaque `AgentCallRecord` Mode C doit référencer un
/// `LlmProvenanceRecord` cohérent (même tenant, même swarm). Renvoie `None` pour un run
/// keyless (Mode A — aucun appel LLM, aucun pont attendu).
fn build_n17_summary(
    records: &[efe_swarm_provenance::AgentCallRecord],
    llm_records: &[efe_nano_sdk::LlmProvenanceRecord],
) -> Option<crate::result::N17Summary> {
    let mode_c = records.iter().filter(|r| matches!(r.agent_kind, AgentKind::ModeC)).count();
    if mode_c == 0 && llm_records.is_empty() {
        return None; // run keyless : pas de pont N17
    }
    let resolver = efe_swarm_provenance::MapResolver::from_records(llm_records.iter());
    let bridge = efe_swarm_provenance::verify_bridge(records, &resolver);
    Some(crate::result::N17Summary {
        calls: llm_records.len(),
        mode_c_records: bridge.consistent + bridge.unlinked + bridge.broken,
        consistent: bridge.consistent,
        unlinked: bridge.unlinked,
        broken: bridge.broken,
        all_consistent: bridge.all_mode_c_consistent(),
    })
}

/// Constructeur de [`Swarm`].
pub struct SwarmBuilder {
    mode: ClientMode,
    tenant: TenantId,
    key_source: HmacKeySource,
    provenance: bool,
    assumed_subtasks: usize,
    fleet: Option<Arc<AgentFleet>>,
    metrics: Option<Arc<ProvenanceMetrics>>,
    substrate_metrics: Option<Arc<SubstrateMetrics>>,
    observer: Option<Arc<dyn SwarmObserver>>,
    allow_remote_backend: bool,
    require_healthy_backend: bool,
    width: WidthSpec,
}

impl Default for SwarmBuilder {
    fn default() -> Self {
        Self {
            mode: ClientMode::default(), // Keyless
            tenant: TenantId::system(),
            key_source: HmacKeySource::test_key(), // dev/keyless ; prod : EnvHkdf (server-side)
            provenance: true,
            assumed_subtasks: 8,
            fleet: None,
            metrics: None,
            substrate_metrics: None,
            observer: None,
            allow_remote_backend: false, // souverain par défaut : egress interdit
            require_healthy_backend: false,
            width: WidthSpec::Auto, // ADR-014 : cœurs (Mode A) / backend déclaré (Mode C)
        }
    }
}

impl SwarmBuilder {
    pub fn mode(mut self, mode: ClientMode) -> Self {
        self.mode = mode;
        self
    }
    pub fn tenant(mut self, tenant: TenantId) -> Self {
        self.tenant = tenant;
        self
    }
    /// Source de clé de provenance (prod : `HmacKeySource::EnvHkdf` server-side).
    pub fn key_source(mut self, src: HmacKeySource) -> Self {
        self.key_source = src;
        self
    }
    pub fn provenance(mut self, on: bool) -> Self {
        self.provenance = on;
        self
    }
    pub fn assumed_subtasks(mut self, n: usize) -> Self {
        self.assumed_subtasks = n.max(1);
        self
    }
    /// Flotte custom (par défaut : `AgentFleet::standard()`).
    pub fn fleet(mut self, fleet: Arc<AgentFleet>) -> Self {
        self.fleet = Some(fleet);
        self
    }

    /// Registre de métriques de provenance **partagé** : chaque run l'alimente
    /// (`observe_sealed`/`observe_verify`). La passerelle HTTP l'expose en `/metrics`
    /// (agrégé sur tous les runs d'un tenant).
    pub fn with_metrics(mut self, metrics: Arc<ProvenanceMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Registre de métriques de **substrat** partagé (Blackboard/EventBus des topologies
    /// T2/T-Consensus) : chaque run l'alimente → `swarm_blackboard_*`, exposé par la
    /// passerelle en `/metrics`.
    pub fn with_substrate_metrics(mut self, metrics: Arc<SubstrateMetrics>) -> Self {
        self.substrate_metrics = Some(metrics);
        self
    }

    /// **Phase 5 Inc.3** : branche un observateur de cycle de vie (dashboard live, journal…).
    /// Tee l'exutoire de provenance + tap d'événements + notifications appel/seal/finish.
    /// **Non-breaking** : sans observateur, le run est inchangé.
    pub fn with_observer(mut self, observer: Arc<dyn SwarmObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// **Garde-fou souveraineté (G2)** : autorise un backend self-host **non-local** (egress).
    /// Par défaut `false` — seul `127.0.0.1`/`localhost`/`::1` est accepté (souverain). À
    /// n'activer **qu'explicitement**, pour un backend LAN de confiance.
    pub fn allow_remote_backend(mut self, allow: bool) -> Self {
        self.allow_remote_backend = allow;
        self
    }

    /// **Health-check (S.a)** : sonde le backend self-host au démarrage du run ; renvoie
    /// `SwarmError::BackendUnavailable` s'il est injoignable (échec rapide). `false` par défaut.
    pub fn require_healthy_backend(mut self, require: bool) -> Self {
        self.require_healthy_backend = require;
        self
    }

    /// **ADR-014 (CP-1)** : la **largeur d'exécution** des vagues. `Auto` (défaut) =
    /// `min(vague, P_eff)` — cœurs mesurés en Mode A, capacité déclarée du backend en
    /// Mode C. `Fixed(w)` impose. Env `EFE_SWARM_WIDTH` prioritaire sur tout (l'opérateur
    /// a le dernier mot). **I-CP1** : ce réglage ne change ni cellules, ni records, ni
    /// racine — seules les durées.
    pub fn width(mut self, w: WidthSpec) -> Self {
        self.width = w;
        self
    }

    /// Construit le `Swarm` (dérive la clé de provenance par tenant).
    pub fn build(self) -> Result<Swarm, SwarmError> {
        let key = self.key_source.derive(self.tenant).map_err(|e| SwarmError::KeyError(e.to_string()))?;
        Ok(Swarm {
            fleet: self.fleet.unwrap_or_else(|| Arc::new(AgentFleet::standard())),
            mode: self.mode,
            tenant: self.tenant,
            key,
            provenance: self.provenance,
            assumed_subtasks: self.assumed_subtasks,
            metrics: self.metrics,
            substrate_metrics: self.substrate_metrics,
            observer: self.observer,
            allow_remote_backend: self.allow_remote_backend,
            require_healthy_backend: self.require_healthy_backend,
            width: self.width,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientMode;
    use efe_swarm_topology::Pattern;
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn keyless_consensus_runs_and_is_verifiable() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(
                json!({ "text": "hello", "expected": "hello" }),
                TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        assert_eq!(run.topology, "T-consensus");
        assert!(run.backend.is_none(), "keyless Mode A : aucun backend local");
        assert!(run.n17.is_none(), "keyless Mode A : aucun appel LLM → aucun pont N17");
        // provenance auto : chaque run renvoie un root re-vérifiable.
        assert!(run.provenance.enabled);
        assert!(run.provenance.verified, "verify_swarm a re-prouvé le run");
        assert_eq!(run.provenance.root.as_ref().unwrap().len(), 64);
        assert!(run.budget.spent_cents <= run.budget.max_cost_cents);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t1_generate_verify_runs() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(json!({ "text": "abc" }), TopologySpec::GenerateVerify { generator: "echo_generator".into(), verifier: "non_empty_text".into(), k: 3.into() }, Budget::cents(100))
            .await
            .unwrap();
        assert_eq!(run.topology, "T1:generate-verify");
        assert!(run.provenance.verified);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t2_pipeline_runs() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(json!({ "text": "  Hello   World  " }), TopologySpec::Pipeline { stages: vec!["normalize_whitespace".into(), "uppercase".into()] }, Budget::cents(100))
            .await
            .unwrap();
        assert_eq!(run.topology, "T2:pipeline");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t3_hierarchical_runs_with_stage_solver() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(json!({ "text": "alpha beta gamma" }), TopologySpec::Hierarchical { decomposer: "word_split".into(), solver: "uppercase".into(), merger: "join_lines".into() }, Budget::cents(1000))
            .await
            .unwrap();
        assert_eq!(run.topology, "T3:hierarchical");
        assert!(run.provenance.verified);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn auto_picks_a_topology_and_runs() {
        let swarm = Swarm::keyless();
        let run = swarm.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100)).await.unwrap();
        assert_eq!(run.topology, "T1:generate-verify"); // verifiable → T1
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_error_policy_rejects_when_too_low() {
        let swarm = Swarm::keyless();
        // Keyless = 1¢/appel ; consensus k=7 + 3 juges = 10 appels = 10¢ > plafond 3¢.
        let err = swarm
            .run(
                json!({ "text": "x" }),
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7.into(), pattern: Pattern::Borda, escalate: false },
                Budget::cents(3).with_policy(OnExceed::Error),
            )
            .await;
        assert!(matches!(err, Err(SwarmError::BudgetTooLow { .. })));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_degrade_policy_shrinks_to_fit() {
        let swarm = Swarm::keyless();
        // plafond 3¢ @ 1¢/appel → 3 appels max ; consensus dégradé tient dans le budget.
        let run = swarm
            .run(
                json!({ "text": "x", "expected": "x" }),
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7.into(), pattern: Pattern::Borda, escalate: false },
                Budget::cents(3).with_policy(OnExceed::Degrade),
            )
            .await
            .unwrap();
        assert!(run.budget.spent_cents <= 3, "jamais au-delà du plafond (I13)");
        assert!(!run.budget_stopped, "dégradé pour tenir, pas arrêté");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_stop_policy_caps_spend() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(
                json!({ "text": "x" }),
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7.into(), pattern: Pattern::Borda, escalate: false },
                Budget::cents(4).with_policy(OnExceed::Stop),
            )
            .await
            .unwrap();
        assert!(run.budget.spent_cents <= 4, "plafond JAMAIS dépassé (I13)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn unknown_agent_errors() {
        let swarm = Swarm::keyless();
        let err = swarm.run(json!({}), TopologySpec::GenerateVerify { generator: "does_not_exist".into(), verifier: "non_empty_text".into(), k: 1.into() }, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::UnknownAgent { role: "generator", .. })));
    }

    #[test]
    fn self_host_mode_builds_keyless() {
        // build seul (pas de run : SelfHost ferait de VRAIS appels Ollama — cf. l'exemple
        // selfhost_ollama). On vérifie : keyless + prix local nul.
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() })
            .build()
            .unwrap();
        assert!(swarm.mode().is_keyless());
        assert_eq!(swarm.mode().price_per_call_cents(), 0, "self-host local = gratuit");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn self_host_remote_endpoint_is_denied_by_default() {
        // Garde-fou souveraineté (G2) : un backend self-host non-local est REFUSÉ avant tout
        // appel réseau (le garde-fou mord avant la résolution des agents) → egress=0.
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://example.com:11434".into(), model: "gemma3:12b".into() })
            .build()
            .unwrap();
        let err = swarm.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::RemoteBackendDenied { .. })), "egress refusé par défaut (souverain)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn require_healthy_backend_errors_on_dead_local_endpoint() {
        // Health-check (S.a) : endpoint LOCAL (passe le garde-fou egress) mais MORT (port 1,
        // connexion refusée instantanément) → BackendUnavailable, sans réseau externe.
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:1".into(), model: "m".into() })
            .require_healthy_backend(true)
            .build()
            .unwrap();
        let err = swarm.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::BackendUnavailable { .. })), "health-check : échec rapide si injoignable");
    }

    #[test]
    fn allow_remote_backend_is_opt_in_and_off_by_default() {
        // build seul (pas de run) : on vérifie que le flag existe et est opt-in.
        let _ = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://10.0.0.5:11434".into(), model: "m".into() })
            .allow_remote_backend(true)
            .build()
            .unwrap();
    }

    /// **Pont N17 par-appel RÉEL** (Ollama local). Ignoré par défaut (CI sans modèle).
    /// Lancer : `cargo test -p efe-swarm-sdk --lib -- --ignored n17_real`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
    async fn n17_per_call_bridge_links_real_self_host_calls() {
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() })
            .build()
            .unwrap();
        let outcome = swarm
            .run_traced(
                json!({ "text": "The court ruled the clause unenforceable.", "expected": "legal" }),
                TopologySpec::Consensus {
                    generator: "Classify the topic in one word from {technology,finance,health,legal}.".into(),
                    judges: vec!["Judge how well the label fits.".into()],
                    k: 3.into(),
                    pattern: Pattern::Borda,
                    escalate: false,
                },
                Budget::cents(1000),
            )
            .await
            .unwrap();

        // chaque appel LLM a un record forensique ; le pont est cohérent.
        assert!(!outcome.llm_records.is_empty(), "des LlmProvenanceRecord par-appel");
        let n17 = outcome.run.n17.as_ref().expect("résumé N17 présent (Mode C)");
        assert!(n17.calls >= 4, "≥ 3 générateurs + ≥ 1 juge → ≥ 4 appels LLM");
        assert!(n17.all_consistent, "tous les Mode C liés à un N17 cohérent : {n17:?}");
        assert_eq!(n17.unlinked, 0);
        assert_eq!(n17.broken, 0);
        // le forensique porte le vrai modèle.
        assert!(outcome.llm_records.iter().all(|r| r.model_identity.model_id == "gemma3:12b"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn network_mode_requires_gateway() {
        let swarm = Swarm::builder().mode(ClientMode::Network { provider: "anthropic".into() }).build().unwrap();
        let err = swarm.run(json!({ "text": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::NetworkRequiresGateway)), "Network → passerelle serveur (clés)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_traced_exposes_sealed_records_matching_summary() {
        // Inc.2 : run_traced expose les AgentCallRecord bruts (la preuve persistable).
        let swarm = Swarm::keyless();
        let outcome = swarm
            .run_traced(
                json!({ "text": "hello", "expected": "hello" }),
                TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        // les records existent et leur compte == le résumé.
        let sealed = outcome.sealed.as_ref().expect("provenance activée → records scellés");
        assert!(!sealed.records.is_empty());
        assert_eq!(sealed.len(), outcome.run.provenance.records);
        assert_eq!(outcome.records().len(), sealed.len());
        // un tiers re-prouve depuis les SEULS records + une clé tenant.
        let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
        let report = verify_swarm(outcome.records(), &key);
        assert!(report.ok(), "records bruts re-prouvés indépendamment");
        assert_eq!(report.root_hex(), outcome.run.provenance.root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn with_metrics_accumulates_across_runs() {
        // Inc.2 : un registre partagé agrège la provenance de TOUS les runs (→ /metrics).
        let metrics = ProvenanceMetrics::new();
        let swarm = Swarm::builder().with_metrics(metrics.clone()).build().unwrap();
        for _ in 0..3 {
            swarm
                .run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100))
                .await
                .unwrap();
        }
        let snap = metrics.snapshot();
        assert!(snap.records_total >= 3, "records agrégés sur les 3 runs");
        assert_eq!(snap.chain_breaks_total, 0, "runs sains → aucune rupture");
        assert!(metrics.render_prometheus().contains("efe_swarm_prov_records_total"));
    }
}
