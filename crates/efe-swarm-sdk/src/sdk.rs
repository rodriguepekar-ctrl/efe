// SPDX-License-Identifier: UNLICENSED
//! `Swarm` — l'entrée unique : `swarm.run(task, topology, budget)`.

use std::sync::Arc;

use efe_blackboard::WriteSink;
use efe_nano_sdk::{SwarmId, TenantId};
use efe_swarm_provenance::{verify_swarm, AgentKind, HmacKeySource, ProvenanceRecorder, ProvenanceSink};
use efe_swarm_topology::{AgentFleet, SwarmContext};

use crate::agents::Meter;
use crate::budget::{Budget, BudgetGuard, OnExceed};
use crate::client::ClientMode;
use crate::llm::OllamaBackend;
use crate::resolver::Resolver;
use crate::result::{Accounting, ProvenanceSummary, SwarmError, SwarmRun};
use crate::spec::TopologySpec;

/// L'essaim productisé. Construit via [`Swarm::builder`] ou [`Swarm::keyless`].
pub struct Swarm {
    fleet: Arc<AgentFleet>,
    mode: ClientMode,
    tenant: TenantId,
    key: Vec<u8>, // clé de provenance pré-dérivée (HKDF par tenant)
    provenance: bool,
    assumed_subtasks: usize,
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
    /// re-vérifiable par un tiers.
    pub async fn run(
        &self,
        task: serde_json::Value,
        topology: TopologySpec,
        budget: Budget,
    ) -> Result<SwarmRun, SwarmError> {
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
                let est = spec.estimated_calls(self.assumed_subtasks).saturating_mul(price as usize) as u32;
                if est > budget.max_cents {
                    return Err(SwarmError::BudgetTooLow { needed_cents: est, max_cents: budget.max_cents });
                }
                spec
            }
            OnExceed::Stop => spec,
        };

        // 1. mode → backend d'agents + nature (Mode A flotte / Mode C self-host).
        let (backend, kind) = match &self.mode {
            ClientMode::Keyless => (None, AgentKind::ModeA),
            ClientMode::SelfHost { endpoint, model } => {
                (Some(OllamaBackend::new(endpoint.clone(), model.clone())), AgentKind::ModeC)
            }
            ClientMode::Network { .. } => unreachable!("écarté en tête de run"),
        };

        // 2. provenance + budget : recorder, sink (→ contexte), garde de budget.
        let swarm_id = SwarmId::new();
        let recorder = Arc::new(ProvenanceRecorder::new(swarm_id, self.tenant));
        let guard = Arc::new(BudgetGuard::new(budget.max_cents, price));
        let meter = Meter { rec: recorder.clone(), guard: guard.clone(), kind };
        let sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));
        let ctx = SwarmContext::new(self.fleet.clone()).with_write_sink(sink);

        // 3. assemble la topologie (agents flotte OU LLM, enveloppés) + exécute.
        let resolver = Resolver { fleet: &self.fleet, backend, meter };
        let topo = spec.resolve(&resolver)?;
        let topo_name = topo.name().to_string();
        let out = topo.run(&task, &ctx).await;

        // 3. budget POST-VOL (politique Error : échec si épuisé en cours).
        if budget.on_exceed == OnExceed::Error && guard.exhausted() {
            return Err(SwarmError::BudgetExceeded { spent_cents: guard.spent_cents(), max_cents: guard.max_cents() });
        }

        // 4. scelle la provenance + re-prouve (verify_swarm) → provenance_root.
        let provenance = if self.provenance {
            let sealed = recorder.seal_with_key(self.key.clone());
            let report = verify_swarm(&sealed.records, &self.key);
            ProvenanceSummary {
                enabled: true,
                records: sealed.len(),
                traced_pct: report.traced_pct,
                verified: report.ok(),
                root: report.root_hex(),
            }
        } else {
            ProvenanceSummary::disabled()
        };

        Ok(SwarmRun {
            output: out.output,
            topology: topo_name,
            accounting: Accounting {
                agents_used: out.agents_used,
                cost_units: out.cost_units,
                elapsed_ms: out.elapsed_ms,
            },
            budget: guard.snapshot(),
            provenance,
            quality: out.quality.into(),
            budget_stopped: budget.on_exceed == OnExceed::Stop && guard.exhausted(),
        })
    }
}

/// Constructeur de [`Swarm`].
pub struct SwarmBuilder {
    mode: ClientMode,
    tenant: TenantId,
    key_source: HmacKeySource,
    provenance: bool,
    assumed_subtasks: usize,
    fleet: Option<Arc<AgentFleet>>,
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
            .run(json!({ "text": "abc" }), TopologySpec::GenerateVerify { generator: "echo_generator".into(), verifier: "non_empty_text".into(), k: 3 }, Budget::cents(100))
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
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7, pattern: Pattern::Borda, escalate: false },
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
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7, pattern: Pattern::Borda, escalate: false },
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
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7, pattern: Pattern::Borda, escalate: false },
                Budget::cents(4).with_policy(OnExceed::Stop),
            )
            .await
            .unwrap();
        assert!(run.budget.spent_cents <= 4, "plafond JAMAIS dépassé (I13)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn unknown_agent_errors() {
        let swarm = Swarm::keyless();
        let err = swarm.run(json!({}), TopologySpec::GenerateVerify { generator: "does_not_exist".into(), verifier: "non_empty_text".into(), k: 1 }, Budget::unlimited()).await;
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
    async fn network_mode_requires_gateway() {
        let swarm = Swarm::builder().mode(ClientMode::Network { provider: "anthropic".into() }).build().unwrap();
        let err = swarm.run(json!({ "text": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::NetworkRequiresGateway)), "Network → passerelle serveur (clés)");
    }
}
