// SPDX-License-Identifier: UNLICENSED
//! **T4 — Gossip** : raffinement stigmergique par tours (ADR-SWARM-003, spec gelée).
//!
//! K agents [`Refiner`] partagent des estimations PARTIELLES via le Blackboard
//! (namespace transient `/gossip/<r>/<i>`) et convergent par tours : chacun lit la
//! **vérité figée** du tour précédent (ses pairs), produit une estimation affinée,
//! l'écrit **write-once**, publie l'indice. Arrêt : résiduel ≤ ε ou r_max (I-IT2).
//!
//! La sortie est l'**agrégat naturel** de la métrique (moyenne pour `Variance`,
//! majorité pour `Agreement`), accompagné du diagnostic de convergence — l'écart
//! résiduel est rendu, jamais masqué : un gossip qui n'a pas convergé le DIT.

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use efe_blackboard::{Blackboard, SwarmEvent, TopicRouter, WriteSink};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::round_driver::{RoundCell, RoundDriver, RoundWave};
use super::{num_of, ConvergenceMetric, Refiner};
use crate::topology::{answer_is_present, QualitySignals, SwarmContext, Topology, TopologyOutput};
use crate::traits::Stage;

// ─────────────────────────────────────────────────────────────────────────────
//  La topologie
// ─────────────────────────────────────────────────────────────────────────────

/// **T4 Gossip** — K refiners × tours, sur le substrat (cf. doc de module).
pub struct GossipTopology {
    refiner: Arc<dyn Refiner>,
    k: usize,
    max_rounds: usize,
    epsilon: f64,
    metric: ConvergenceMetric,
    max_concurrency: usize,
    write_sink: Option<Arc<dyn WriteSink>>,
}

impl std::fmt::Debug for GossipTopology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GossipTopology")
            .field("k", &self.k)
            .field("max_rounds", &self.max_rounds)
            .field("epsilon", &self.epsilon)
            .field("metric", &self.metric)
            .field("instrumented", &self.write_sink.is_some())
            .finish()
    }
}

impl GossipTopology {
    pub fn new(refiner: Arc<dyn Refiner>, k: usize, max_rounds: usize, epsilon: f64, metric: ConvergenceMetric) -> Self {
        Self {
            refiner,
            k: k.max(1),
            max_rounds: max_rounds.max(1),
            epsilon,
            metric,
            max_concurrency: 8,
            write_sink: None,
        }
    }
    /// **Phase 4** : exutoire d'écritures local (sinon celui du contexte) — ancrage
    /// 100 % des cellules `/gossip/*` pour la provenance.
    pub fn with_write_sink(mut self, sink: Arc<dyn WriteSink>) -> Self {
        self.write_sink = Some(sink);
        self
    }
    pub fn with_concurrency(mut self, c: usize) -> Self {
        self.max_concurrency = c.max(1);
        self
    }
}

/// La vague de gossip : K `refine()` en parallèle (bornés), chacun recevant SA
/// cellule du tour précédent + celles des pairs (vérité figée — I-IT3).
struct GossipWave {
    refiner: Arc<dyn Refiner>,
    task: Value,
    k: usize,
    max_concurrency: usize,
}

/// Injecte le contexte de vague dans la tâche (déterminisme Mode A par agent) +
/// la DÉCLARATION DE LECTURES `_reads` (sa cellule + celles des pairs, à version
/// exacte) — liftée par le Meter dans le record scellé, contrôlée par le DAG.
fn task_for(task: &Value, agent_index: usize, round: usize, reads: &[(&str, u64)]) -> Value {
    let mut m = match task {
        Value::Object(o) => o.clone(),
        other => {
            let mut m = Map::new();
            m.insert("task".into(), other.clone());
            m
        }
    };
    m.insert("gossip".into(), json!({ "agent_index": agent_index, "round": round }));
    if !reads.is_empty() {
        m.insert(
            "_reads".into(),
            Value::Array(
                reads.iter().map(|(p, v)| json!({ "pointer": p, "version": v })).collect(),
            ),
        );
    }
    Value::Object(m)
}

#[async_trait]
impl RoundWave for GossipWave {
    async fn produce(&self, round: usize, prev: &[RoundCell]) -> Vec<(String, AgentId, Value)> {
        let sem = Arc::new(Semaphore::new(self.max_concurrency));
        let mut set: JoinSet<(usize, Value)> = JoinSet::new();
        let prev = Arc::new(prev.to_vec());
        for i in 0..self.k {
            let r = self.refiner.clone();
            let prev = prev.clone();
            // l'agent DÉCLARE ce qu'il lit : sa cellule + celles des pairs (r−1).
            let reads: Vec<(&str, u64)> =
                prev.iter().map(|c| (c.pointer.as_str(), c.version)).collect();
            let task = task_for(&self.task, i, round, &reads);
            let sem = sem.clone();
            set.spawn(async move {
                let _p = sem.acquire_owned().await.expect("semaphore");
                let own = prev.get(i).map(|c| c.value.clone());
                let peers: Vec<Value> = prev
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, c)| c.value.clone())
                    .collect();
                (i, r.refine(&task, own.as_ref(), &peers).await)
            });
        }
        let mut cells = vec![Value::Null; self.k];
        while let Some(res) = set.join_next().await {
            if let Ok((i, v)) = res {
                cells[i] = v;
            }
        }
        let who = AgentId::new("refiner").expect("agent id valide");
        cells.into_iter().enumerate().map(|(i, v)| (i.to_string(), who.clone(), v)).collect()
    }
}

#[async_trait]
impl Topology for GossipTopology {
    fn name(&self) -> &str {
        "T4-gossip"
    }

    async fn run(&self, task: &Value, ctx: &SwarmContext) -> TopologyOutput {
        let t0 = Instant::now();
        let swarm_id = SwarmId::new();
        let bb = Arc::new(Blackboard::new(swarm_id));
        if let Some(sink) = self.write_sink.clone().or_else(|| ctx.write_sink.clone()) {
            bb.set_write_sink(sink);
        }
        if let Some(metrics) = ctx.substrate_metrics.clone() {
            bb.set_metrics(metrics);
        }
        let router = Arc::new(TopicRouter::new((self.k * self.max_rounds * 2).max(256)));
        if let Some(tap) = ctx.event_tap.clone() {
            router.set_event_sink(tap);
        }

        let driver = RoundDriver::new("/gossip", "gossip", self.max_rounds, self.epsilon, self.metric);
        let wave = GossipWave {
            refiner: self.refiner.clone(),
            task: task.clone(),
            k: self.k,
            max_concurrency: self.max_concurrency,
        };
        let rep = driver.run(&bb, &router, swarm_id, &wave).await;

        // L'agrégat final + le diagnostic (l'écart résiduel n'est jamais masqué).
        let agg = self.metric.aggregate(&rep.final_cells);
        let output = json!({
            "value": agg,
            "gossip": {
                "rounds": rep.rounds,
                "residual": rep.residual,
                "converged": rep.converged,
                "write_conflicts": rep.write_conflicts,
            }
        });

        // Inc.3 : verdict observable (même motif que "consensus.decided").
        if let Some(tap) = &ctx.event_tap {
            tap.on_event(&SwarmEvent {
                swarm_id,
                published_by: AgentId::new("gossip").expect("agent id valide"),
                topic: "gossip.converged".to_string(),
                payload: json!({
                    "rounds": rep.rounds, "residual": rep.residual, "converged": rep.converged,
                    "winner": agg.clone(),
                }),
                published_at: SystemTime::now(),
            });
        }

        let calls = self.k * rep.rounds;
        let acceptable = answer_is_present(&output); // MEM-2.c : réponse convergée présente ?
        TopologyOutput {
            output,
            agents_used: calls,
            cost_units: calls as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals {
                candidates: self.k,
                verified: usize::from(rep.converged),
                subtasks: rep.rounds,
            },
            decided_pattern: None, // gossip : convergence par raffinement, pas d'agrégation (ADR-019)
            escalated: false,
            produced_acceptable: acceptable,
            agent_contributions: Vec::new(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Refiners Mode A intégrés (déterministes — le harnais de gate + l'ancre CI)
// ─────────────────────────────────────────────────────────────────────────────

/// Estimation initiale déterministe d'un agent : `task.initial[i]` si fourni,
/// sinon `task.center` (défaut 0.0) — la diversité du tour 0 est PILOTÉE par la
/// tâche, donc reproductible (l'ancre Mode A, ADR-007).
fn initial_of(task: &Value) -> Value {
    let i = task
        .pointer("/gossip/agent_index")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    if let Some(v) = task.get("initial").and_then(|a| a.as_array()).and_then(|a| a.get(i)) {
        return if v.is_number() { json!({ "value": v }) } else { v.clone() };
    }
    json!({ "value": task.get("center").and_then(Value::as_f64).unwrap_or(0.0) })
}

/// `mean_pull` — tire son estimation vers la MOYENNE des pairs (α = 0,5).
pub struct MeanPull;
#[async_trait]
impl Refiner for MeanPull {
    async fn refine(&self, task: &Value, own: Option<&Value>, peers: &[Value]) -> Value {
        let Some(own) = own else { return initial_of(task) };
        let mine = num_of(own).unwrap_or(0.0);
        let xs: Vec<f64> = peers.iter().filter_map(num_of).collect();
        if xs.is_empty() {
            return own.clone();
        }
        let m = xs.iter().sum::<f64>() / xs.len() as f64;
        json!({ "value": 0.5 * mine + 0.5 * m })
    }
}

/// `median_pull` — comme `mean_pull`, vers la MÉDIANE (robuste aux extrêmes).
pub struct MedianPull;
#[async_trait]
impl Refiner for MedianPull {
    async fn refine(&self, task: &Value, own: Option<&Value>, peers: &[Value]) -> Value {
        let Some(own) = own else { return initial_of(task) };
        let mine = num_of(own).unwrap_or(0.0);
        let mut xs: Vec<f64> = peers.iter().filter_map(num_of).collect();
        if xs.is_empty() {
            return own.clone();
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let med = if xs.len() % 2 == 1 { xs[xs.len() / 2] } else { (xs[xs.len() / 2 - 1] + xs[xs.len() / 2]) / 2.0 };
        json!({ "value": 0.5 * mine + 0.5 * med })
    }
}

/// `hold` — ne révise JAMAIS (témoin de non-convergence / borne r_max).
pub struct Hold;
#[async_trait]
impl Refiner for Hold {
    async fn refine(&self, task: &Value, own: Option<&Value>, _peers: &[Value]) -> Value {
        own.cloned().unwrap_or_else(|| initial_of(task))
    }
}

/// `majority_adopt` — catégoriel : adopte la valeur MAJORITAIRE de (pairs ∪ soi).
pub struct MajorityAdopt;
#[async_trait]
impl Refiner for MajorityAdopt {
    async fn refine(&self, task: &Value, own: Option<&Value>, peers: &[Value]) -> Value {
        let Some(own) = own else { return initial_of(task) };
        let mut all: Vec<Value> = peers.to_vec();
        all.push(own.clone());
        super::majority(&all)
    }
}

/// Résolution des refiners Mode A intégrés (réservés — le Resolver du SDK s'en sert
/// avant de tenter l'adaptation d'une `Stage` de flotte).
pub fn builtin_refiner(name: &str) -> Option<Arc<dyn Refiner>> {
    match name {
        "mean_pull" => Some(Arc::new(MeanPull)),
        "median_pull" => Some(Arc::new(MedianPull)),
        "hold" => Some(Arc::new(Hold)),
        "majority_adopt" => Some(Arc::new(MajorityAdopt)),
        _ => None,
    }
}

/// Adapte une [`Stage`] de la flotte en [`Refiner`] : l'étage reçoit
/// `{task, own, peers, gossip}` et sa 1ʳᵉ sortie est l'estimation révisée.
pub struct StageRefiner(pub Arc<dyn Stage>);
#[async_trait]
impl Refiner for StageRefiner {
    async fn refine(&self, task: &Value, own: Option<&Value>, peers: &[Value]) -> Value {
        let input = json!({
            "task": task, "own": own.cloned().unwrap_or(Value::Null), "peers": peers,
        });
        self.0.process(&input).await.into_iter().next().unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_blackboard::BlackboardWrite;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountTap(AtomicUsize);
    impl efe_blackboard::EventSink for CountTap {
        fn on_event(&self, _e: &SwarmEvent) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    struct CountWrites(AtomicUsize);
    impl WriteSink for CountWrites {
        fn on_write(&self, _w: &BlackboardWrite) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn task4() -> Value {
        json!({ "initial": [90.0, 110.0, 95.0, 105.0] })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mean_pull_converges_to_the_mean() {
        // var₀ = 62,5 → ÷~9 par tour (α=0,5, lecture des pairs) → ε=0,1 atteint au tour 4.
        let topo = GossipTopology::new(Arc::new(MeanPull), 4, 10, 0.1, ConvergenceMetric::Variance);
        let out = topo.run(&task4(), &SwarmContext::standard()).await;
        let v = out.output.pointer("/value").and_then(Value::as_f64).unwrap();
        assert!((v - 100.0).abs() < 0.5, "la nuée converge vers la moyenne (100), eu {v}");
        assert_eq!(out.output.pointer("/gossip/converged"), Some(&json!(true)));
        let rounds = out.output.pointer("/gossip/rounds").and_then(Value::as_u64).unwrap() as usize;
        assert!((2..=6).contains(&rounds), "convergence en quelques tours, eu {rounds}");
        assert_eq!(out.agents_used, 4 * rounds, "comptabilité = K × tours");
        assert_eq!(out.quality.verified, 1, "converged → verified=1");
        assert_eq!(out.output.pointer("/gossip/write_conflicts"), Some(&json!(0)), "I-IT1");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn hold_hits_r_max_and_says_so() {
        let topo = GossipTopology::new(Arc::new(Hold), 4, 3, 1e-6, ConvergenceMetric::Variance);
        let out = topo.run(&task4(), &SwarmContext::standard()).await;
        assert_eq!(out.output.pointer("/gossip/converged"), Some(&json!(false)), "honnête : pas convergé");
        assert_eq!(out.output.pointer("/gossip/rounds"), Some(&json!(3)), "I-IT2 : borne dure");
        let res = out.output.pointer("/gossip/residual").and_then(Value::as_f64).unwrap();
        assert!(res > 1.0, "l'écart résiduel est rendu, pas masqué");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn majority_adopt_converges_categorically() {
        let task = json!({ "initial": ["a", "a", "b"] });
        let topo = GossipTopology::new(Arc::new(MajorityAdopt), 3, 5, 0.0, ConvergenceMetric::Agreement);
        let out = topo.run(&task, &SwarmContext::standard()).await;
        assert_eq!(out.output.pointer("/gossip/converged"), Some(&json!(true)));
        assert_eq!(out.output.pointer("/value"), Some(&json!("a")), "la majorité s'impose");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn seams_observe_cells_and_verdict() {
        let tap = Arc::new(CountTap(AtomicUsize::new(0)));
        let writes = Arc::new(CountWrites(AtomicUsize::new(0)));
        let ctx = SwarmContext::standard()
            .with_event_tap(tap.clone())
            .with_write_sink(writes.clone());
        let topo = GossipTopology::new(Arc::new(MeanPull), 4, 10, 0.1, ConvergenceMetric::Variance);
        let out = topo.run(&task4(), &ctx).await;
        let rounds = out.output.pointer("/gossip/rounds").and_then(Value::as_u64).unwrap() as usize;
        assert_eq!(writes.0.load(Ordering::Relaxed), 4 * rounds, "chaque cellule ancrée (provenance)");
        // indices par cellule + le verdict "gossip.converged".
        assert_eq!(tap.0.load(Ordering::Relaxed), 4 * rounds + 1, "tap : cellules + verdict");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runs_uniformly_as_dyn_topology() {
        let topo: Box<dyn Topology> =
            Box::new(GossipTopology::new(Arc::new(MeanPull), 3, 6, 0.5, ConvergenceMetric::Variance));
        let out = topo.run(&json!({ "initial": [1.0, 2.0, 3.0] }), &SwarmContext::standard()).await;
        assert_eq!(topo.name(), "T4-gossip");
        assert!(out.cost_units > 0.0);
    }
}
