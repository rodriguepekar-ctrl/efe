// SPDX-License-Identifier: UNLICENSED
//! Les **taps** — comment chaque seam alimente le [`DashboardBus`], sans toucher aux agents.
//!
//! - [`DashWriteSink`] (`WriteSink`) : writes stigmergiques `/cand`, `/ballot`, `/out` → trafic.
//! - [`DashEventTap`] (`EventSink`) : messages de bus inter-agent → trafic.
//! - [`DashboardObserver`] (`SwarmObserver`) : le pilote — fournit les deux taps au SDK et
//!   traduit le cycle de vie (start/appel/seal/finish) en événements (consensus + budget +
//!   provenance).

use std::sync::Arc;
use std::time::SystemTime;

use efe_blackboard::{BlackboardWrite, EventSink, SwarmEvent, WriteSink};
use efe_swarm_sdk::{AgentCall, AgentRole, SwarmObserver, SwarmRun};

use serde_json::Value;

use crate::bus::DashboardBus;
use crate::event::{preview, DashboardEvent, Lane};

/// Topic de bus **réservé** : l'agrégateur de consensus y annonce son verdict
/// (pattern + confiance). Le tap le traduit en [`DashboardEvent::Decided`] plutôt qu'en
/// `AgentMessage`. Émis par `efe-swarm-topology::ConsensusTopology` quand un tap est présent.
pub const DECIDED_TOPIC: &str = "consensus.decided";

/// **G9 — PII opt-in** : caviarde un aperçu textuel avant diffusion (previews du dashboard
/// live). Le dashboard reste **léger** (trait générique) ; la passerelle injecte une
/// implémentation `efe-pii` **locale** (sans réseau) quand un tenant l'active. Défaut : aucun.
pub trait PreviewRedactor: Send + Sync {
    fn redact(&self, text: &str) -> String;
}

/// Aperçu tronqué + caviardé (si un redactor est branché).
fn redacted_preview(redactor: &Option<Arc<dyn PreviewRedactor>>, value: &Value) -> String {
    let p = preview(value);
    match redactor {
        Some(r) => r.redact(&p),
        None => p,
    }
}

/// Tap d'écritures (`WriteSink`) → `AgentWrite`.
struct DashWriteSink {
    bus: Arc<DashboardBus>,
}
impl WriteSink for DashWriteSink {
    fn on_write(&self, w: &BlackboardWrite) {
        self.bus.emit(DashboardEvent::AgentWrite {
            pointer: w.pointer.clone(),
            version: w.version,
            agent_id: w.agent_id.as_str().to_string(),
            bytes: w.bytes,
            lane: Lane::classify(&w.pointer),
        });
    }
}

/// Tap d'événements de bus (`EventSink`) → `AgentMessage`.
struct DashEventTap {
    bus: Arc<DashboardBus>,
    redactor: Option<Arc<dyn PreviewRedactor>>,
}
impl EventSink for DashEventTap {
    fn on_event(&self, ev: &SwarmEvent) {
        // Verdicts d'agrégation (topics réservés) → événement structuré `Decided`.
        // Trois sources : consensus (historique), debate (itératif — même forme de
        // payload) et gossip (converged : la confiance est binaire, le résiduel fin
        // reste visible dans le flux générique).
        if ev.topic == DECIDED_TOPIC || ev.topic == "debate.decided" {
            let p = &ev.payload;
            self.bus.emit(DashboardEvent::Decided {
                pattern: p.get("pattern").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                confidence: p.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0),
                judges_used: p.get("judges_used").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                winner_preview: redacted_preview(&self.redactor, p.get("winner").unwrap_or(&Value::Null)),
            });
            return;
        }
        if ev.topic == "gossip.converged" {
            let p = &ev.payload;
            let converged = p.get("converged").and_then(|v| v.as_bool()).unwrap_or(false);
            self.bus.emit(DashboardEvent::Decided {
                pattern: "gossip".to_string(),
                confidence: if converged { 1.0 } else { 0.0 },
                judges_used: 0,
                winner_preview: redacted_preview(&self.redactor, p.get("winner").unwrap_or(&Value::Null)),
            });
            return;
        }
        let latency_ms = SystemTime::now()
            .duration_since(ev.published_at)
            .ok()
            .map(|d| d.as_secs_f64() * 1000.0);
        self.bus.emit(DashboardEvent::AgentMessage {
            topic: ev.topic.clone(),
            from: ev.published_by.as_str().to_string(),
            bytes: ev.payload.to_string().len(),
            latency_ms,
        });
    }
}

/// L'**observateur** SDK qui pilote un [`DashboardBus`] pour un run.
///
/// Branché via `SwarmBuilder::with_observer(Arc::new(DashboardObserver::new(bus)))`.
pub struct DashboardObserver {
    bus: Arc<DashboardBus>,
    redactor: Option<Arc<dyn PreviewRedactor>>,
}

impl DashboardObserver {
    pub fn new(bus: Arc<DashboardBus>) -> Self {
        Self { bus, redactor: None }
    }

    /// **G9** : caviarde les previews (candidats, verdict, sortie) via un redactor — la
    /// passerelle injecte un `efe-pii` **local** quand un tenant l'active. Défaut : aucun.
    pub fn with_redactor(mut self, redactor: Arc<dyn PreviewRedactor>) -> Self {
        self.redactor = Some(redactor);
        self
    }
}

impl SwarmObserver for DashboardObserver {
    fn write_sink(&self) -> Option<Arc<dyn WriteSink>> {
        Some(Arc::new(DashWriteSink { bus: self.bus.clone() }))
    }

    fn event_tap(&self) -> Option<Arc<dyn EventSink>> {
        Some(Arc::new(DashEventTap { bus: self.bus.clone(), redactor: self.redactor.clone() }))
    }

    fn on_run_start(&self, topology: &str, mode: &str, budget_cap_cents: u32, assumed_agents: usize) {
        self.bus.emit(DashboardEvent::RunStarted {
            topology: topology.to_string(),
            mode: mode.to_string(),
            budget_cap_cents,
            assumed_agents,
        });
    }

    fn on_agent_call(&self, call: AgentCall<'_>) {
        // budget + appel (panneaux budget & trafic).
        self.bus.emit(DashboardEvent::AgentCall {
            agent_id: call.agent_id.to_string(),
            role: format!("{:?}", call.role),
            agent_kind: format!("{:?}", call.kind),
            cost_cents: call.cost_cents,
        });
        // contenu de consensus, dérivé du rôle (le Meter porte la sortie réelle).
        match call.role {
            AgentRole::Generator => self.bus.emit(DashboardEvent::Candidate {
                variant: call.variant.unwrap_or(0) as usize,
                value_preview: redacted_preview(&self.redactor, call.output),
            }),
            AgentRole::Judge => {
                let scores = call
                    .output
                    .as_array()
                    .map(|a| a.iter().filter_map(|x| x.as_f64()).collect())
                    .unwrap_or_default();
                self.bus.emit(DashboardEvent::Ballot { judge: call.agent_id.to_string(), scores });
            }
            _ => {}
        }
        // tick de budget (jamais au-delà du plafond — I13).
        let b = call.budget;
        self.bus.emit(DashboardEvent::BudgetTick {
            spent_cents: b.spent_cents,
            max_cents: b.max_cost_cents,
            per_agent: b.per_llm.clone(),
            exhausted: b.remaining == 0 && b.max_cost_cents != u32::MAX,
        });
    }

    fn on_seal(&self, records: usize, traced_pct: f64, root: Option<&str>, verified: bool) {
        self.bus.emit(DashboardEvent::Sealed {
            records,
            traced_pct,
            root: root.map(|r| r.to_string()),
            verified,
        });
    }

    fn on_finish(&self, run: &SwarmRun) {
        self.bus.emit(DashboardEvent::RunFinished {
            elapsed_ms: run.accounting.elapsed_ms,
            agents_used: run.accounting.agents_used,
            output_preview: redacted_preview(&self.redactor, &run.output),
            verified: run.provenance.verified,
            root: run.provenance.root.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_swarm_sdk::{Budget, Swarm, TopologySpec};
    use serde_json::json;
    use uuid::Uuid;

    /// G9 : un redactor branché caviarde les previews (candidats + sortie).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn redactor_scrubs_previews() {
        struct Mask;
        impl PreviewRedactor for Mask {
            fn redact(&self, _t: &str) -> String {
                "[REDACTED]".into()
            }
        }
        let bus = DashboardBus::new(efe_nano_sdk::TenantId::system(), Uuid::from_u128(9), 8192);
        let swarm = Swarm::builder()
            .with_observer(Arc::new(DashboardObserver::new(bus.clone()).with_redactor(Arc::new(Mask))))
            .build()
            .unwrap();
        swarm
            .run(json!({ "text": "hello", "expected": "hello" }), TopologySpec::consensus("echo_generator", &["consistency"], 3), Budget::cents(1000))
            .await
            .unwrap();
        // tout preview (candidat / sortie) est caviardé ; aucun ne fuit le texte brut.
        let previews: Vec<String> = bus
            .replay()
            .into_iter()
            .filter_map(|e| match e.event {
                DashboardEvent::Candidate { value_preview, .. } => Some(value_preview),
                DashboardEvent::RunFinished { output_preview, .. } => Some(output_preview),
                _ => None,
            })
            .collect();
        assert!(!previews.is_empty());
        assert!(previews.iter().all(|p| p == "[REDACTED]"), "tous les previews caviardés : {previews:?}");
    }

    /// Un run keyless réel, observé : le bus reçoit la séquence attendue.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn observed_consensus_run_emits_expected_events() {
        let bus = DashboardBus::new(efe_nano_sdk::TenantId::system(), Uuid::from_u128(1), 8192);
        let swarm = Swarm::builder()
            .with_observer(Arc::new(DashboardObserver::new(bus.clone())))
            .build()
            .unwrap();
        let run = swarm
            .run(
                json!({ "text": "hello", "expected": "hello" }),
                TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        assert!(run.provenance.verified);

        let kinds: Vec<&str> = bus.replay().iter().map(|e| e.event.kind()).collect();
        // cycle de vie + chaque famille de panneau présente.
        assert_eq!(kinds.first(), Some(&"run_started"));
        assert_eq!(kinds.last(), Some(&"run_finished"));
        assert!(kinds.contains(&"agent_write"), "trafic : writes stigmergiques");
        assert!(kinds.contains(&"agent_call"), "budget/trafic : appels facturés");
        assert!(kinds.contains(&"candidate"), "consensus : candidats générés");
        assert!(kinds.contains(&"ballot"), "consensus : ballots des juges");
        assert!(kinds.contains(&"decided"), "consensus : verdict de l'agrégateur");
        assert!(kinds.contains(&"budget_tick"), "budget : ticks");
        assert!(kinds.contains(&"sealed"), "provenance : scellé");

        // le verdict porte un pattern + une confiance réels.
        let decided = bus.replay().into_iter().find_map(|e| match e.event {
            DashboardEvent::Decided { pattern, confidence, judges_used, .. } => Some((pattern, confidence, judges_used)),
            _ => None,
        });
        let (pattern, confidence, judges_used) = decided.expect("un verdict émis");
        assert!(!pattern.is_empty());
        assert!((0.0..=1.0).contains(&confidence), "confiance normalisée");
        assert!(judges_used <= 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn ballot_scores_are_real_from_meter() {
        let bus = DashboardBus::new(efe_nano_sdk::TenantId::system(), Uuid::from_u128(2), 8192);
        let swarm = Swarm::builder()
            .with_observer(Arc::new(DashboardObserver::new(bus.clone())))
            .build()
            .unwrap();
        swarm
            .run(
                json!({ "text": "abc", "expected": "abc" }),
                TopologySpec::consensus("echo_generator", &["consistency"], 3),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        let has_scored_ballot = bus.replay().iter().any(|e| {
            matches!(&e.event, DashboardEvent::Ballot { scores, .. } if !scores.is_empty())
        });
        assert!(has_scored_ballot, "les scores du ballot viennent du Meter (réels)");
    }

    /// Les verdicts ITÉRATIFS (debate.decided, gossip.converged) sont structurés en
    /// `Decided` — parité Mission-Control avec le consensus historique.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn iterative_verdicts_become_decided_events() {
        use efe_blackboard::{EventSink, SwarmEvent};
        use std::time::SystemTime;
        let bus = DashboardBus::new(efe_nano_sdk::TenantId::system(), Uuid::from_u128(7), 8192);
        let tap = DashEventTap { bus: bus.clone(), redactor: None };
        let mk = |topic: &str, payload: serde_json::Value| SwarmEvent {
            swarm_id: efe_nano_sdk::SwarmId::new(),
            published_by: efe_nano_sdk::AgentId::new("tap_test").unwrap(),
            topic: topic.into(),
            payload,
            published_at: SystemTime::now(),
        };
        tap.on_event(&mk(
            "debate.decided",
            json!({ "pattern": "Borda", "confidence": 0.75, "judges_used": 6, "winner": "e", "rounds": 2 }),
        ));
        tap.on_event(&mk(
            "gossip.converged",
            json!({ "rounds": 4, "residual": 0.01, "converged": true, "winner": 100.0 }),
        ));
        let decided: Vec<(String, f64, usize)> = bus
            .replay()
            .into_iter()
            .filter_map(|e| match e.event {
                DashboardEvent::Decided { pattern, confidence, judges_used, .. } => {
                    Some((pattern, confidence, judges_used))
                }
                _ => None,
            })
            .collect();
        assert_eq!(decided.len(), 2, "les deux verdicts structurés");
        assert_eq!(decided[0], ("Borda".into(), 0.75, 6));
        assert_eq!(decided[1], ("gossip".into(), 1.0, 0));
    }
}
