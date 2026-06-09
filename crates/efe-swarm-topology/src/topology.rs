// SPDX-License-Identifier: UNLICENSED
//! **Phase 2 — couche d'abstraction unifiée** (architecture §3).
//!
//! [`Topology`] : interface OBJECT-SAFE commune (Value → [`TopologyOutput`]) — un
//! `Vec<Box<dyn Topology>>` permet d'exécuter/comparer T1, T2, T3 uniformément.
//! [`SwarmContext`] injecte les services partagés (la [`AgentFleet`]).
//! [`TopologyOutput`] uniformise la comptabilité (coût, latence, agents, qualité).

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use efe_blackboard::{EventSink, SubstrateMetrics, WriteSink};
use serde_json::Value;

use crate::fleet::AgentFleet;
use crate::generate_verify::GenerateVerify;
use crate::hierarchical::Hierarchical;
use crate::pipeline::Pipeline;
use crate::traits::{Generator, Verifier};

/// Signaux de qualité d'un run de topologie (pour audit / provenance).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QualitySignals {
    pub candidates: usize,
    pub verified: usize,
    pub subtasks: usize,
}

/// Sortie UNIFORME d'une topologie : résultat + comptabilité.
#[derive(Debug, Clone)]
pub struct TopologyOutput {
    pub output: Value,
    pub agents_used: usize,
    /// Coût en unités d'appel générateur (cheap = 1).
    pub cost_units: f64,
    pub elapsed_ms: u64,
    pub quality: QualitySignals,
}

/// Contexte partagé injecté aux topologies (services : flotte d'agents + provenance).
#[derive(Clone)]
pub struct SwarmContext {
    pub fleet: Arc<AgentFleet>,
    /// **Phase 4 — provenance UNIFORME (§3)** : exutoire d'écritures optionnel, branché
    /// par CHAQUE topologie à blackboard (T2 Pipeline, T-Consensus) sur son substrat
    /// interne. Un seul `with_write_sink` sur le contexte instrumente toute la couche
    /// via l'interface unifiée [`Topology`] — sans que les topologies connaissent la
    /// provenance (type `efe_blackboard::WriteSink`, aucun couplage).
    pub write_sink: Option<Arc<dyn WriteSink>>,
    /// **Observabilité (Phase 5 Inc.2)** : registre de métriques de substrat **partagé**,
    /// branché par chaque topologie sur son Blackboard interne (T2 Pipeline, T-Consensus)
    /// → `swarm_blackboard_writes_total`, `swarm_events_published_total`, … agrégés sur
    /// tous les runs (exposés par la passerelle HTTP en `/metrics`). `None` ⇒ coût nul.
    pub substrate_metrics: Option<Arc<SubstrateMetrics>>,
    /// **Dashboard live (Phase 5 Inc.3)** : tap d'observation des **messages de bus**,
    /// branché par chaque topologie (T2 Pipeline, T-Consensus) sur son `TopicRouter` interne
    /// (au seam `publish`, fiable). `None` ⇒ coût nul. Le trafic inter-agent du dashboard.
    pub event_tap: Option<Arc<dyn EventSink>>,
}

impl std::fmt::Debug for SwarmContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmContext")
            .field("fleet", &self.fleet)
            .field("instrumented", &self.write_sink.is_some())
            .field("metered", &self.substrate_metrics.is_some())
            .field("tapped", &self.event_tap.is_some())
            .finish()
    }
}

impl SwarmContext {
    pub fn new(fleet: Arc<AgentFleet>) -> Self {
        Self { fleet, write_sink: None, substrate_metrics: None, event_tap: None }
    }
    /// Contexte avec la flotte standard (tout le catalogue Mode A).
    pub fn standard() -> Self {
        Self {
            fleet: Arc::new(AgentFleet::standard()),
            write_sink: None,
            substrate_metrics: None,
            event_tap: None,
        }
    }
    /// **Phase 4** : branche un exutoire de provenance partagé par toutes les topologies.
    pub fn with_write_sink(mut self, sink: Arc<dyn WriteSink>) -> Self {
        self.write_sink = Some(sink);
        self
    }
    /// **Phase 5 Inc.2** : branche un registre de métriques de substrat partagé. Chaque
    /// topologie l'attache à son Blackboard interne (`set_metrics`) → les écritures /
    /// événements de l'essaim sont comptés (pour `/metrics`).
    pub fn with_substrate_metrics(mut self, metrics: Arc<SubstrateMetrics>) -> Self {
        self.substrate_metrics = Some(metrics);
        self
    }
    /// **Phase 5 Inc.3** : branche un tap d'événements de bus (dashboard live). Chaque
    /// topologie l'attache à son `TopicRouter` (`set_event_sink`) → trafic inter-agent observé.
    pub fn with_event_tap(mut self, tap: Arc<dyn EventSink>) -> Self {
        self.event_tap = Some(tap);
        self
    }
}

/// Interface UNIFIÉE des topologies (object-safe).
#[async_trait]
pub trait Topology: Send + Sync {
    fn name(&self) -> &str;
    async fn run(&self, task: &Value, ctx: &SwarmContext) -> TopologyOutput;
}

// ─── Adaptateurs : T1 / T2 / T3 → Topology ───────────────────────────────────

/// **T1** comme topologie (Generate-Verify configurée avec ses agents).
pub struct GvTopology {
    pub gv: GenerateVerify,
    pub gen: Arc<dyn Generator>,
    pub ver: Arc<dyn Verifier>,
}
#[async_trait]
impl Topology for GvTopology {
    fn name(&self) -> &str {
        "T1:generate-verify"
    }
    async fn run(&self, task: &Value, _ctx: &SwarmContext) -> TopologyOutput {
        let t0 = Instant::now();
        let r = self.gv.run(task, self.gen.clone(), self.ver.as_ref()).await;
        let n = r.candidates.len();
        TopologyOutput {
            output: r.selected.unwrap_or(Value::Null),
            agents_used: n,
            cost_units: n as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals { candidates: n, verified: r.passed, subtasks: 0 },
        }
    }
}

/// **T2** comme topologie (Pipeline ; tâche unique = batch de 1).
pub struct PipelineTopology {
    pub pipeline: Pipeline,
    pub timeout: Duration,
}
impl PipelineTopology {
    pub fn new(pipeline: Pipeline) -> Self {
        Self { pipeline, timeout: Duration::from_secs(30) }
    }
}
#[async_trait]
impl Topology for PipelineTopology {
    fn name(&self) -> &str {
        "T2:pipeline"
    }
    async fn run(&self, task: &Value, ctx: &SwarmContext) -> TopologyOutput {
        let t0 = Instant::now();
        // Phase 4 : branche l'exutoire de provenance du contexte (s'il y en a un) sur le
        // substrat interne du pipeline → ancrage des écritures `/out/*` (seam A uniforme).
        let outs = self
            .pipeline
            .run_batch_instrumented(
                vec![task.clone()],
                self.timeout,
                ctx.write_sink.clone(),
                ctx.substrate_metrics.clone(),
                ctx.event_tap.clone(),
            )
            .await;
        let produced = outs.len();
        TopologyOutput {
            output: outs.into_iter().next().unwrap_or(Value::Null),
            agents_used: self.pipeline.len(),
            cost_units: self.pipeline.len() as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals { candidates: 0, verified: 0, subtasks: produced },
        }
    }
}

/// **T3** comme topologie (Hierarchical ; coût ≈ sous-tâches × coût unitaire).
pub struct HierTopology {
    pub inner: Arc<Hierarchical>,
    pub cost_per_subtask: f64,
}
impl HierTopology {
    pub fn new(inner: Arc<Hierarchical>, cost_per_subtask: f64) -> Self {
        Self { inner, cost_per_subtask }
    }
}
#[async_trait]
impl Topology for HierTopology {
    fn name(&self) -> &str {
        "T3:hierarchical"
    }
    async fn run(&self, task: &Value, _ctx: &SwarmContext) -> TopologyOutput {
        let t0 = Instant::now();
        let r = self.inner.run(task).await;
        TopologyOutput {
            output: r.output,
            agents_used: r.subtasks,
            cost_units: r.subtasks as f64 * self.cost_per_subtask,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals { candidates: 0, verified: 0, subtasks: r.subtasks },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{EqualsExpected, MajorityValue, WordSplit};
    use crate::hierarchical::{GvSolver, Hierarchical};
    use async_trait::async_trait;
    use serde_json::json;

    struct OkGen;
    #[async_trait]
    impl Generator for OkGen {
        async fn generate(&self, task: &Value, _v: usize) -> Value {
            json!({ "value": task.get("expected").cloned().unwrap_or(Value::Null) })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t1_as_topology_yields_output_and_accounting() {
        let topo = GvTopology {
            gv: GenerateVerify::new(5, 4),
            gen: Arc::new(OkGen),
            ver: Arc::new(EqualsExpected),
        };
        let ctx = SwarmContext::standard();
        let out = topo.run(&json!({ "expected": "tech" }), &ctx).await;
        assert_eq!(topo.name(), "T1:generate-verify");
        assert_eq!(out.output["value"], "tech");
        assert_eq!(out.agents_used, 5);
        assert_eq!(out.cost_units, 5.0);
        assert_eq!(out.quality.verified, 5);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn uniform_dispatch_over_box_dyn_topology() {
        // T1 + T3 derrière la MÊME interface → exécutables uniformément.
        let t1: Box<dyn Topology> = Box::new(GvTopology {
            gv: GenerateVerify::new(3, 3),
            gen: Arc::new(OkGen),
            ver: Arc::new(EqualsExpected),
        });
        let h = Arc::new(Hierarchical::new(
            Arc::new(WordSplit),
            Arc::new(GvSolver {
                gv: GenerateVerify::new(2, 2),
                gen: Arc::new(OkGen),
                ver: Arc::new(EqualsExpected),
            }),
            Arc::new(MajorityValue),
        ));
        let t3: Box<dyn Topology> = Box::new(HierTopology::new(h, 2.0));

        let ctx = SwarmContext::standard();
        let task = json!({ "expected": "x", "text": "a b c" });
        let topos: Vec<Box<dyn Topology>> = vec![t1, t3];
        let mut names = Vec::new();
        for t in &topos {
            let o = t.run(&task, &ctx).await;
            names.push(t.name().to_string());
            assert!(o.cost_units > 0.0);
        }
        assert_eq!(names, vec!["T1:generate-verify", "T3:hierarchical"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t2_pipeline_as_topology() {
        use crate::catalog::Uppercase;
        let pipe = Pipeline::new().stage("up", Arc::new(Uppercase));
        let topo = PipelineTopology::new(pipe);
        let ctx = SwarmContext::standard();
        let out = topo.run(&json!({ "text": "hi" }), &ctx).await;
        assert_eq!(topo.name(), "T2:pipeline");
        assert_eq!(out.output["text"], "HI");
        assert_eq!(out.agents_used, 1);
    }

    #[test]
    fn swarm_context_exposes_standard_fleet() {
        let ctx = SwarmContext::standard();
        assert_eq!(ctx.fleet.len(), 43); // 34 catalogue + 9 juges
        assert!(ctx.fleet.stage("uppercase").is_some());
        assert!(ctx.fleet.judge("consistency").is_some());
    }
}
