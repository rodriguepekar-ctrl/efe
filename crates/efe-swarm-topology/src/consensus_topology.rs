// SPDX-License-Identifier: UNLICENSED
//! **T-Consensus** (Phase 3) — Generate puis CONSENSUS, câblé sur le substrat.
//!
//! Flux « les agents votent en se passant des infos » :
//! ```text
//! générateurs ──K candidats──► Blackboard (/cand/* = VÉRITÉ durable)
//!                                   │
//!   juges-agents (∥, bornés) ───────┤ lisent les candidats, ÉCRIVENT leur ballot
//!                                   │ /ballot/<j> + PUBLIENT sur "ballot.<j>" (TopicRouter)
//!                                   ▼
//!   collecte ──subscribe_prefix("ballot.")──► (bus = indice)  +  réconcilie depuis /ballot/* (vérité)
//!                                   ▼
//!   agrégateur (Vote/Borda/Condorcet, ADAPTATIF) ──► réponse consensuelle
//! ```
//! Keystone WS1 : le **bus** est l'indice (passage d'infos), le **Blackboard** est la
//! vérité (garantit que l'agrégateur voit TOUS les ballots — S1 l'exige).

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use efe_blackboard::{Blackboard, SwarmEvent, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::consensus::selector::{aggregator_for, Pattern, PatternSelector};
use crate::consensus::{Ballot, Judge};
use crate::topology::{QualitySignals, SwarmContext, Topology, TopologyOutput};
use crate::traits::Generator;

/// Politique de choix d'agrégateur.
#[derive(Debug, Clone)]
pub enum AggregatorPolicy {
    /// Pattern imposé.
    Fixed(Pattern),
    /// Choisi selon l'accord des candidats.
    Adaptive(PatternSelector),
}

/// **T-Consensus** : K générateurs + J juges + agrégation de consensus, sur le substrat.
pub struct ConsensusTopology {
    generator: Arc<dyn Generator>,
    k: usize,
    judges: Vec<Arc<dyn Judge>>,
    policy: AggregatorPolicy,
    max_concurrency: usize,
}

impl std::fmt::Debug for ConsensusTopology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsensusTopology")
            .field("k", &self.k)
            .field("judges", &self.judges.len())
            .field("policy", &self.policy)
            .finish()
    }
}

impl ConsensusTopology {
    /// Adaptatif par défaut (Vote si accord fort, sinon Borda).
    pub fn new(generator: Arc<dyn Generator>, k: usize, judges: Vec<Arc<dyn Judge>>) -> Self {
        Self {
            generator,
            k: k.max(1),
            judges,
            policy: AggregatorPolicy::Adaptive(PatternSelector::default()),
            max_concurrency: 8,
        }
    }
    pub fn with_pattern(mut self, p: Pattern) -> Self {
        self.policy = AggregatorPolicy::Fixed(p);
        self
    }
    pub fn with_selector(mut self, s: PatternSelector) -> Self {
        self.policy = AggregatorPolicy::Adaptive(s);
        self
    }
    pub fn with_concurrency(mut self, c: usize) -> Self {
        self.max_concurrency = c.max(1);
        self
    }

    /// Génère K candidats EN PARALLÈLE + les écrit au Blackboard (vérité durable).
    async fn generate(&self, task: &Value, bb: &Blackboard) -> Vec<Value> {
        let sem = Arc::new(Semaphore::new(self.max_concurrency));
        let mut set: JoinSet<(usize, Value)> = JoinSet::new();
        for variant in 0..self.k {
            let g = self.generator.clone();
            let task = task.clone();
            let sem = sem.clone();
            set.spawn(async move {
                let _p = sem.acquire_owned().await.expect("semaphore");
                (variant, g.generate(&task, variant).await)
            });
        }
        let mut cands = vec![Value::Null; self.k];
        while let Some(r) = set.join_next().await {
            if let Ok((v, c)) = r {
                cands[v] = c;
            }
        }
        let gid = AgentId::new("generator").unwrap();
        for (i, c) in cands.iter().enumerate() {
            let _ = bb.write(&format!("/cand/{i}"), c.clone(), gid.clone());
        }
        cands
    }
}

#[async_trait]
impl Topology for ConsensusTopology {
    fn name(&self) -> &str {
        "T-consensus"
    }

    async fn run(&self, task: &Value, _ctx: &SwarmContext) -> TopologyOutput {
        let t0 = Instant::now();
        let swarm_id = SwarmId::new();
        let bb = Arc::new(Blackboard::new(swarm_id));
        let jn = self.judges.len();
        let cap = ((self.k + jn) * 4).max(256);
        let router = Arc::new(TopicRouter::new(cap));

        // 1. génération + écriture des candidats (vérité).
        let candidates = self.generate(task, &bb).await;

        // 2. pré-crée les topics juges puis subscribe_prefix (collecte multi-topics).
        for j in 0..jn {
            let _ = router.subscribe(&format!("ballot.{j}"));
        }
        let mut merged = router.subscribe_prefix("ballot.");

        // 3. juges-agents (∥, bornés) : assess → /ballot/<j> + publient sur "ballot.<j>".
        let cands = Arc::new(candidates.clone());
        let sem = Arc::new(Semaphore::new(self.max_concurrency));
        let mut handles = Vec::with_capacity(jn);
        for (j, judge) in self.judges.iter().enumerate() {
            let judge = judge.clone();
            let cands = cands.clone();
            let bb = bb.clone();
            let router = router.clone();
            let task = task.clone();
            let sem = sem.clone();
            let jid = AgentId::new(format!("judge_{j:03}")).unwrap();
            handles.push(tokio::spawn(async move {
                let _p = sem.acquire_owned().await.expect("semaphore");
                let scores = judge.assess(&task, &cands).await;
                let payload = json!({ "judge": judge.name(), "scores": scores });
                let _ = bb.write(&format!("/ballot/{j}"), payload.clone(), jid.clone());
                let _ = router.publish(SwarmEvent {
                    swarm_id,
                    published_by: jid,
                    topic: format!("ballot.{j}"),
                    payload,
                    published_at: SystemTime::now(),
                });
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        // 4. bus = indice : draine les notifs (subscribe_prefix) — passage d'infos.
        drop(router);
        let mut bus_count = 0usize;
        while merged.recv().await.is_some() {
            bus_count += 1;
        }

        // 5. Blackboard = vérité : reconstruit les ballots (exhaustif, reliable).
        let snap = bb.snapshot();
        let mut ballots = Vec::with_capacity(jn);
        for j in 0..jn {
            if let Some(v) = snap.fields.get(&format!("/ballot/{j}")) {
                if let Some(arr) = v.value.get("scores").and_then(|s| s.as_array()) {
                    let sc: Vec<f64> = arr.iter().filter_map(|x| x.as_f64()).collect();
                    let name = v.value.get("judge").and_then(|x| x.as_str()).unwrap_or("judge");
                    ballots.push(Ballot::new(name, sc));
                }
            }
        }

        // 6. agrégation (pattern fixe ou adaptatif selon l'accord des candidats).
        let pattern = match &self.policy {
            AggregatorPolicy::Fixed(p) => *p,
            AggregatorPolicy::Adaptive(s) => s.select(&candidates),
        };
        let agg = aggregator_for(pattern);
        let outcome = agg.aggregate(task, &candidates, &ballots);

        TopologyOutput {
            output: outcome.selected,
            agents_used: self.k + jn,
            cost_units: self.k as f64, // K générations ; juges Mode A gratuits
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals {
                candidates: candidates.len(),
                verified: bus_count, // ballots reçus via le bus (démonstration du passage d'infos)
                subtasks: ballots.len(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::EqualsExpected;
    use crate::consensus::VerifierJudge;
    use crate::traits::Verifier;

    /// Générateur de test : renvoie l'attendu pour certains variants, un leurre sinon.
    struct TestGen {
        expected: Value,
        correct_variants: Vec<usize>,
    }
    #[async_trait]
    impl Generator for TestGen {
        async fn generate(&self, _task: &Value, variant: usize) -> Value {
            if self.correct_variants.contains(&variant) {
                json!({ "value": self.expected })
            } else {
                json!({ "value": "__lure__" })
            }
        }
    }

    fn discerning_judges(n: usize) -> Vec<Arc<dyn Judge>> {
        (0..n)
            .map(|i| {
                let v: Arc<dyn Verifier> = Arc::new(EqualsExpected);
                Arc::new(VerifierJudge::new(format!("j{i}"), v)) as Arc<dyn Judge>
            })
            .collect()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn consensus_beats_vote_with_discerning_judges() {
        // 3 candidats corrects, 4 leurres → le VOTE choisirait le leurre (majorité).
        // Mais les juges (EqualsExpected) favorisent les corrects → Borda choisit correct.
        let topo = ConsensusTopology::new(
            Arc::new(TestGen { expected: json!("tech"), correct_variants: vec![0, 1, 2] }),
            7,
            discerning_judges(5),
        )
        .with_pattern(Pattern::Borda);
        let out = topo.run(&json!({ "expected": "tech" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "tech" }), "Borda extrait le correct (vote échouerait)");
        assert_eq!(out.agents_used, 7 + 5);
        assert_eq!(out.quality.subtasks, 5, "5 ballots reconstruits depuis le Blackboard");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn adaptive_picks_vote_when_candidates_agree() {
        // tous les candidats identiques → accord = 1.0 → sélecteur choisit Vote.
        let topo = ConsensusTopology::new(
            Arc::new(TestGen { expected: json!("x"), correct_variants: (0..7).collect() }),
            7,
            discerning_judges(3),
        );
        let out = topo.run(&json!({ "expected": "x" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "x" }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runs_as_a_topology_uniformly() {
        let topo: Box<dyn Topology> = Box::new(
            ConsensusTopology::new(
                Arc::new(TestGen { expected: json!("a"), correct_variants: vec![0, 1, 2, 3] }),
                7,
                discerning_judges(3),
            )
            .with_pattern(Pattern::Condorcet),
        );
        let out = topo.run(&json!({ "expected": "a" }), &SwarmContext::standard()).await;
        assert_eq!(topo.name(), "T-consensus");
        assert!(out.cost_units > 0.0);
        assert_eq!(out.output, json!({ "value": "a" }));
    }
}
