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
use efe_blackboard::{Blackboard, SwarmEvent, TopicRouter, WriteSink};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::consensus::selector::{aggregator_for, Pattern, PatternSelector};
use crate::consensus::{Aggregator, Ballot, Judge, MajorityVote};
use crate::topology::{QualitySignals, SwarmContext, Topology, TopologyOutput};
use crate::traits::Generator;

/// Politique de choix d'agrégateur.
#[derive(Debug, Clone)]
pub enum AggregatorPolicy {
    /// Pattern imposé.
    Fixed(Pattern),
    /// Choisi selon l'accord des candidats (avant de voir les ballots).
    Adaptive(PatternSelector),
    /// **Escalade pilotée par la CONFIANCE** (§5) : tente d'abord le Vote (GRATUIT,
    /// sans juges) ; si sa confiance ≥ seuil → terminé (juges ÉCONOMISÉS) ; sinon
    /// escalade vers un pattern plus cher (Borda/Condorcet → appelle les juges).
    Escalating {
        vote_confidence_threshold: f64,
        escalate_to: Pattern,
    },
}

/// **T-Consensus** : K générateurs + J juges + agrégation de consensus, sur le substrat.
pub struct ConsensusTopology {
    generator: Arc<dyn Generator>,
    k: usize,
    judges: Vec<Arc<dyn Judge>>,
    policy: AggregatorPolicy,
    max_concurrency: usize,
    /// **Phase 4 — provenance** : exutoire d'écritures optionnel, branché sur le
    /// Blackboard interne du run. Permet à la provenance par-agent d'ancrer 100 % des
    /// écritures (`/cand/*`, `/ballot/*`) **sans** que la topologie connaisse la
    /// provenance (type `efe_blackboard::WriteSink` — aucun couplage, aucun cycle).
    write_sink: Option<Arc<dyn WriteSink>>,
}

impl std::fmt::Debug for ConsensusTopology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsensusTopology")
            .field("k", &self.k)
            .field("judges", &self.judges.len())
            .field("policy", &self.policy)
            .field("instrumented", &self.write_sink.is_some())
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
            write_sink: None,
        }
    }

    /// **Phase 4** : branche un exutoire d'écritures (ex. `ProvenanceSink`) sur le
    /// Blackboard interne du run → ancrage 100 % des écritures pour la provenance.
    pub fn with_write_sink(mut self, sink: Arc<dyn WriteSink>) -> Self {
        self.write_sink = Some(sink);
        self
    }
    pub fn with_pattern(mut self, p: Pattern) -> Self {
        self.policy = AggregatorPolicy::Fixed(p);
        self
    }
    pub fn with_selector(mut self, s: PatternSelector) -> Self {
        self.policy = AggregatorPolicy::Adaptive(s);
        self
    }
    /// Escalade pilotée par la confiance : Vote d'abord, juges seulement si le vote
    /// est peu confiant.
    pub fn with_escalation(mut self, vote_confidence_threshold: f64, escalate_to: Pattern) -> Self {
        self.policy = AggregatorPolicy::Escalating { vote_confidence_threshold, escalate_to };
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

    /// Phase JUGES sur le substrat : juges-agents (∥, bornés) écrivent `/ballot/<j>`
    /// + publient sur `ballot.<j>` ; collecte via subscribe_prefix (bus = indice) +
    /// réconciliation depuis le Blackboard (vérité). Renvoie (ballots, #reçus via bus).
    async fn run_judges(
        &self,
        task: &Value,
        candidates: &[Value],
        bb: &Arc<Blackboard>,
        swarm_id: SwarmId,
    ) -> (Vec<Ballot>, usize) {
        let jn = self.judges.len();
        let cap = (jn * 4).max(256);
        let router = Arc::new(TopicRouter::new(cap));
        for j in 0..jn {
            let _ = router.subscribe(&format!("ballot.{j}"));
        }
        let mut merged = router.subscribe_prefix("ballot.");

        let cands = Arc::new(candidates.to_vec());
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

        drop(router); // bus = indice : draine puis ferme.
        let mut bus_count = 0usize;
        while merged.recv().await.is_some() {
            bus_count += 1;
        }

        // Blackboard = vérité : reconstruit tous les ballots (exhaustif).
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
        (ballots, bus_count)
    }
}

#[async_trait]
impl Topology for ConsensusTopology {
    fn name(&self) -> &str {
        "T-consensus"
    }

    async fn run(&self, task: &Value, ctx: &SwarmContext) -> TopologyOutput {
        let t0 = Instant::now();
        let swarm_id = SwarmId::new();
        let bb = Arc::new(Blackboard::new(swarm_id));
        // Phase 4 : branche l'exutoire de provenance → ancrage 100 % des écritures
        // `/cand/*` + `/ballot/*` (seam A). Hook LOCAL prioritaire, sinon celui du
        // contexte (provenance UNIFORME via l'interface §3). Sans couplage provenance.
        if let Some(sink) = self.write_sink.clone().or_else(|| ctx.write_sink.clone()) {
            bb.set_write_sink(sink);
        }
        // Phase 5 Inc.2 : compte les écritures de substrat (`/cand/*`, `/ballot/*`) dans le
        // registre partagé du contexte → `swarm_blackboard_writes_total` (pour `/metrics`).
        if let Some(metrics) = ctx.substrate_metrics.clone() {
            bb.set_metrics(metrics);
        }

        // 1. génération + écriture des candidats au Blackboard (vérité durable).
        let candidates = self.generate(task, &bb).await;

        // 2. décide le pattern + si les juges sont nécessaires (escalade incluse).
        let (pattern, judges_needed) = match &self.policy {
            AggregatorPolicy::Fixed(p) => (*p, *p != Pattern::Vote),
            AggregatorPolicy::Adaptive(s) => {
                let p = s.select(&candidates);
                (p, p != Pattern::Vote)
            }
            AggregatorPolicy::Escalating { vote_confidence_threshold, escalate_to } => {
                // tente le Vote (GRATUIT) ; escalade seulement si peu confiant.
                let vote = MajorityVote.aggregate(task, &candidates, &[]);
                if vote.confidence >= *vote_confidence_threshold {
                    (Pattern::Vote, false) // confiant → juges ÉCONOMISÉS
                } else {
                    (*escalate_to, true) // escalade → on paie les juges
                }
            }
        };

        // 3. phase juges sur le substrat — UNIQUEMENT si nécessaire.
        let (ballots, bus_count) = if judges_needed && !self.judges.is_empty() {
            self.run_judges(task, &candidates, &bb, swarm_id).await
        } else {
            (Vec::new(), 0)
        };
        let judges_run = ballots.len();

        // 4. agrégation.
        let agg = aggregator_for(pattern);
        let outcome = agg.aggregate(task, &candidates, &ballots);

        TopologyOutput {
            output: outcome.selected,
            agents_used: self.k + judges_run,
            // coût = K générations + appels JUGES réellement effectués (récompense l'escalade).
            cost_units: self.k as f64 + judges_run as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals {
                candidates: candidates.len(),
                verified: bus_count,
                subtasks: judges_run,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn escalation_skips_judges_when_vote_is_confident() {
        // candidats UNANIMES → vote très confiant → escalade NON déclenchée → 0 juge.
        let topo = ConsensusTopology::new(
            Arc::new(TestGen { expected: json!("u"), correct_variants: (0..7).collect() }),
            7,
            discerning_judges(5),
        )
        .with_escalation(0.5, Pattern::Borda);
        let out = topo.run(&json!({ "expected": "u" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "u" }));
        assert_eq!(out.quality.subtasks, 0, "vote confiant → juges économisés");
        assert_eq!(out.cost_units, 7.0, "coût = K seulement (pas d'appel juge)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn escalation_runs_judges_when_vote_is_uncertain() {
        // leurre majoritaire (3 corrects, 4 leurres) → vote peu confiant → ESCALADE.
        let topo = ConsensusTopology::new(
            Arc::new(TestGen { expected: json!("e"), correct_variants: vec![0, 1, 2] }),
            7,
            discerning_judges(5),
        )
        .with_escalation(0.5, Pattern::Borda);
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.quality.subtasks, 5, "vote incertain → escalade, 5 juges appelés");
        assert_eq!(out.output, json!({ "value": "e" }), "Borda extrait le correct");
        assert_eq!(out.cost_units, 7.0 + 5.0, "coût = K + appels juges");
    }
}
