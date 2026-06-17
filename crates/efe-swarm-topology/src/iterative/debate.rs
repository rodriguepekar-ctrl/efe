// SPDX-License-Identifier: UNLICENSED
//! **Debate** — consensus à dossier contradictoire (ADR-SWARM-005, spec gelée).
//!
//! Le Debate n'est PAS un agrégateur de plus : c'est un **amplificateur de v** —
//! pour chaque candidat, des agents PRO et CONTRA argumentent sur r tours (chacun
//! lisant l'argument adverse du tour précédent, vérité figée I-IT3), puis le **jury
//! existant** note les candidats *à la lumière du dossier* ; l'agrégation
//! (Vote/Borda/Condorcet) est inchangée en aval — mêmes `Ballot`, même `AggOutcome`.
//!
//! `rounds = 0` dégrade proprement en consensus pur (jury sans dossier) — c'est la
//! borne basse de la dégradation budgétaire (« r d'abord », I-IT4). L'arrêt anticipé
//! utilise [`ConvergenceMetric::Stability`] : un débat qui se répète a fini de dire
//! ce qu'il savait.
//!
//! Namespace : `/cand/<i>` (candidats) · `/debate/<r>/{pro,contra}/<i>` (arguments)
//! · `/ballot/<j>` (jury) — tous ancrés via le `WriteSink` (provenance 100 %).

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use efe_blackboard::{Blackboard, SwarmEvent, TopicRouter, WriteSink};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Map, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::round_driver::{RoundCell, RoundDriver, RoundWave};
use super::ConvergenceMetric;
use crate::consensus::selector::aggregator_for;
use crate::consensus::{Aggregator, Ballot, Judge, MajorityVote};
use crate::consensus::Pattern;
use crate::topology::{answer_is_present, QualitySignals, SwarmContext, Topology, TopologyOutput};
use crate::traits::Generator;

// ─────────────────────────────────────────────────────────────────────────────
//  La topologie
// ─────────────────────────────────────────────────────────────────────────────

/// **Debate** : K candidats → r tours PRO/CONTRA → jury sur dossier → agrégation.
pub struct DebateTopology {
    generator: Arc<dyn Generator>,
    k: usize,
    pro: Arc<dyn Generator>,
    contra: Arc<dyn Generator>,
    judges: Vec<Arc<dyn Judge>>,
    pattern: Pattern,
    rounds: usize,
    max_concurrency: usize,
    /// **Escalade à trois étages (archi §5.3)** : `Some((s1, s2))` ⇒
    /// ① Vote GRATUIT (sans juge) — stop si confiance ≥ s1 ;
    /// ② jury SANS dossier (pattern) — stop si confiance ≥ s2 ;
    /// ③ débat (r tours) + re-jury SUR dossier — l'étage payé seulement quand
    ///   l'essaim, déjà escaladé, hésite ENCORE (« payer selon l'hésitation »).
    escalation: Option<(f64, f64)>,
    write_sink: Option<Arc<dyn WriteSink>>,
}

impl std::fmt::Debug for DebateTopology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DebateTopology")
            .field("k", &self.k)
            .field("rounds", &self.rounds)
            .field("judges", &self.judges.len())
            .field("pattern", &self.pattern)
            .finish()
    }
}

impl DebateTopology {
    #[allow(clippy::too_many_arguments)] // constructeur explicite : les rôles du débat sont nommés.
    pub fn new(
        generator: Arc<dyn Generator>,
        k: usize,
        pro: Arc<dyn Generator>,
        contra: Arc<dyn Generator>,
        judges: Vec<Arc<dyn Judge>>,
        pattern: Pattern,
        rounds: usize,
    ) -> Self {
        Self {
            generator,
            k: k.max(1),
            pro,
            contra,
            judges,
            pattern,
            rounds,
            max_concurrency: 8,
            escalation: None,
            write_sink: None,
        }
    }
    /// Active l'escalade à trois étages : Vote (≥ `s1` ⇒ stop) → jury sans dossier
    /// (≥ `s2` ⇒ stop) → débat + re-jury. Le débat ne se paie que sur les instances
    /// les plus dures du flux.
    pub fn with_escalation(mut self, s1: f64, s2: f64) -> Self {
        self.escalation = Some((s1, s2));
        self
    }
    pub fn with_write_sink(mut self, sink: Arc<dyn WriteSink>) -> Self {
        self.write_sink = Some(sink);
        self
    }
    pub fn with_concurrency(mut self, c: usize) -> Self {
        self.max_concurrency = c.max(1);
        self
    }

    /// Génère K candidats en ∥ + les écrit au Blackboard (le motif T-Consensus).
    async fn generate(&self, task: &Value, bb: &Blackboard) -> (Vec<Value>, Vec<u64>) {
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
        let gid = AgentId::new("generator").expect("agent id valide");
        let mut versions = Vec::with_capacity(self.k);
        for (i, c) in cands.iter().enumerate() {
            let v = bb.write(&format!("/cand/{i}"), c.clone(), gid.clone()).unwrap_or(0);
            versions.push(v);
        }
        (cands, versions)
    }

    /// Le jury — identique au consensus, mais la tâche porte le DOSSIER.
    async fn run_jury(
        &self,
        task_with_dossier: &Value,
        candidates: &[Value],
        bb: &Arc<Blackboard>,
        router: &Arc<TopicRouter>,
        swarm_id: SwarmId,
    ) -> Vec<Ballot> {
        let cands = Arc::new(candidates.to_vec());
        let sem = Arc::new(Semaphore::new(self.max_concurrency));
        let mut handles = Vec::with_capacity(self.judges.len());
        for (j, judge) in self.judges.iter().enumerate() {
            let judge = judge.clone();
            let cands = cands.clone();
            let bb = bb.clone();
            let router = router.clone();
            let task = task_with_dossier.clone();
            let sem = sem.clone();
            let jid = AgentId::new(format!("judge_{j:03}")).expect("id de juge valide");
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
        // Vérité : reconstruit les ballots depuis le snapshot (keystone).
        let snap = bb.snapshot();
        let mut ballots = Vec::with_capacity(self.judges.len());
        for j in 0..self.judges.len() {
            if let Some(v) = snap.fields.get(&format!("/ballot/{j}")) {
                if let Some(arr) = v.value.get("scores").and_then(|s| s.as_array()) {
                    let sc: Vec<f64> = arr.iter().filter_map(|x| x.as_f64()).collect();
                    let name = v.value.get("judge").and_then(|x| x.as_str()).unwrap_or("judge");
                    ballots.push(Ballot::new(name, sc));
                }
            }
        }
        ballots
    }
}

/// La vague d'arguments : 2K générateurs (PRO_i, CONTRA_i), chacun lisant
/// l'argument ADVERSE du tour précédent (la contradiction qui fait le débat).
/// Ordre des cellules : `[pro/0 … pro/k−1, contra/0 … contra/k−1]` (stable).
struct DebateWave {
    pro: Arc<dyn Generator>,
    contra: Arc<dyn Generator>,
    task: Value,
    candidates: Arc<Vec<Value>>,
    cand_versions: Arc<Vec<u64>>,
    max_concurrency: usize,
}

#[allow(clippy::too_many_arguments)] // construction interne : les composantes du contexte d'argument sont nommées.
fn arguer_task(
    base: &Value,
    candidate: &Value,
    side: &str,
    round: usize,
    counter: Option<&Value>,
    reads: &[(String, u64)],
) -> Value {
    let mut m = match base {
        Value::Object(o) => o.clone(),
        other => {
            let mut m = Map::new();
            m.insert("task".into(), other.clone());
            m
        }
    };
    m.insert(
        "debate".into(),
        json!({ "side": side, "round": round, "candidate": candidate,
                "counter": counter.cloned().unwrap_or(Value::Null) }),
    );
    // `text` : le terrain commun des agents LLM (llm.rs lit task.text).
    let counter_txt = counter.and_then(|c| c.get("text")).and_then(Value::as_str).unwrap_or("");
    m.insert("text".into(), json!(format!("candidate: {candidate} | counter-argument: {counter_txt}")));
    if !reads.is_empty() {
        m.insert(
            "_reads".into(),
            Value::Array(reads.iter().map(|(p, v)| json!({ "pointer": p, "version": v })).collect()),
        );
    }
    Value::Object(m)
}

#[async_trait]
impl RoundWave for DebateWave {
    async fn produce(&self, round: usize, prev: &[RoundCell]) -> Vec<(String, AgentId, Value)> {
        let k = self.candidates.len();
        let sem = Arc::new(Semaphore::new(self.max_concurrency));
        let mut set: JoinSet<(usize, Value)> = JoinSet::new();
        for slot in 0..(2 * k) {
            let (side, i) = if slot < k { ("pro", slot) } else { ("contra", slot - k) };
            // l'adverse du tour précédent : PRO_i lit CONTRA_i (prev[k+i]) et vice-versa.
            let counter_cell = if prev.len() == 2 * k {
                Some(if slot < k { prev[k + i].clone() } else { prev[i].clone() })
            } else {
                None
            };
            // les lectures DÉCLARÉES de l'arguer : le candidat (à sa version d'écriture)
            // + l'argument adverse lu (r−1) — scellées, contrôlées par le DAG.
            let mut reads: Vec<(String, u64)> =
                vec![(format!("/cand/{i}"), self.cand_versions.get(i).copied().unwrap_or(0))];
            if let Some(c) = &counter_cell {
                reads.push((c.pointer.clone(), c.version));
            }
            let counter = counter_cell.as_ref().map(|c| c.value.clone());
            let arguer = if slot < k { self.pro.clone() } else { self.contra.clone() };
            let task =
                arguer_task(&self.task, &self.candidates[i], side, round, counter.as_ref(), &reads);
            let sem = sem.clone();
            set.spawn(async move {
                let _p = sem.acquire_owned().await.expect("semaphore");
                (slot, arguer.generate(&task, i).await)
            });
        }
        let mut cells = vec![Value::Null; 2 * k];
        while let Some(r) = set.join_next().await {
            if let Ok((s, v)) = r {
                cells[s] = v;
            }
        }
        let pro_id = AgentId::new("pro").expect("agent id valide");
        let con_id = AgentId::new("contra").expect("agent id valide");
        cells
            .into_iter()
            .enumerate()
            .map(|(s, v)| {
                if s < k {
                    (format!("pro/{s}"), pro_id.clone(), v)
                } else {
                    (format!("contra/{}", s - k), con_id.clone(), v)
                }
            })
            .collect()
    }
}

#[async_trait]
impl Topology for DebateTopology {
    fn name(&self) -> &str {
        "debate"
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
        let router = Arc::new(TopicRouter::new(((2 * self.k + self.judges.len()) * (self.rounds + 2)).max(256)));
        if let Some(tap) = ctx.event_tap.clone() {
            router.set_event_sink(tap);
        }

        // 1. candidats (vérité durable /cand/*), versions captées (graphe d'influence).
        let (candidates, cand_versions) = self.generate(task, &bb).await;

        // ── Escalade (archi §5.3) — étage ① : le Vote GRATUIT. ─────────────────
        // ── étage ② : jury SANS dossier — le consensus classique. ──────────────
        let mut judges_total = 0usize;
        if let Some((s1, s2)) = self.escalation {
            let vote = MajorityVote.aggregate(task, &candidates, &[]);
            if vote.confidence >= s1 {
                // confiant dès le vote : ni juge, ni débat — coût K.
                return self.finish(task, candidates.len(), 0, 0, vote, t0, swarm_id, &ctx.event_tap, "vote");
            }
            if !self.judges.is_empty() {
                let ballots = self.run_jury(task, &candidates, &bb, &router, swarm_id).await;
                judges_total += ballots.len();
                let outcome = aggregator_for(self.pattern).aggregate(task, &candidates, &ballots);
                if outcome.confidence >= s2 {
                    // le jury one-shot suffit : le débat est ÉCONOMISÉ.
                    return self.finish(task, candidates.len(), 0, judges_total, outcome, t0, swarm_id, &ctx.event_tap, "jury");
                }
            }
            // l'essaim hésite encore → étage ③ : on paie le débat.
        }

        // 2. le débat (rounds > 0) — arrêt anticipé si les arguments se répètent.
        let mut rounds_done = 0usize;
        if self.rounds > 0 {
            let driver = RoundDriver::new("/debate", "debate", self.rounds, 0.0, ConvergenceMetric::Stability);
            let wave = DebateWave {
                pro: self.pro.clone(),
                contra: self.contra.clone(),
                task: task.clone(),
                candidates: Arc::new(candidates.clone()),
                cand_versions: Arc::new(cand_versions.clone()),
                max_concurrency: self.max_concurrency,
            };
            let rep = driver.run(&bb, &router, swarm_id, &wave).await;
            rounds_done = rep.rounds;
        }

        // 3. le DOSSIER : le thread complet, relu de la vérité (transcript prouvable).
        let snap = bb.snapshot();
        let dossier: Vec<Value> = (0..self.k)
            .map(|i| {
                let collect = |role: &str| -> Vec<Value> {
                    (0..rounds_done)
                        .filter_map(|r| snap.fields.get(&format!("/debate/{r}/{role}/{i}")).map(|c| c.value.clone()))
                        .collect()
                };
                json!({ "candidate": candidates[i], "pro": collect("pro"), "contra": collect("contra") })
            })
            .collect();
        let task_for_jury = {
            let mut m = match task {
                Value::Object(o) => o.clone(),
                other => {
                    let mut m = Map::new();
                    m.insert("task".into(), other.clone());
                    m
                }
            };
            m.insert("debate".into(), Value::Array(dossier));
            Value::Object(m)
        };

        // 4. le jury (sur dossier) + 5. l'agrégation inchangée.
        let ballots = if self.judges.is_empty() {
            Vec::new()
        } else {
            self.run_jury(&task_for_jury, &candidates, &bb, &router, swarm_id).await
        };
        judges_total += ballots.len();
        let outcome = aggregator_for(self.pattern).aggregate(task, &candidates, &ballots);
        self.finish(task, candidates.len(), rounds_done, judges_total, outcome, t0, swarm_id, &ctx.event_tap, "debate")
    }
}

impl DebateTopology {
    /// Clôture commune des trois étages : verdict observable + comptabilité exacte
    /// (K + 2K×tours + TOUS les appels de juges — les deux jurys en cas d'escalade).
    #[allow(clippy::too_many_arguments)] // clôture interne : les composantes du verdict sont nommées.
    fn finish(
        &self,
        _task: &Value,
        k: usize,
        rounds_done: usize,
        judges_total: usize,
        outcome: crate::consensus::AggOutcome,
        t0: Instant,
        swarm_id: SwarmId,
        event_tap: &Option<Arc<dyn efe_blackboard::EventSink>>,
        stage: &str,
    ) -> TopologyOutput {
        if let Some(tap) = event_tap {
            tap.on_event(&SwarmEvent {
                swarm_id,
                published_by: AgentId::new("debate").expect("agent id valide"),
                topic: "debate.decided".to_string(),
                payload: json!({
                    "rounds": rounds_done, "pattern": format!("{:?}", self.pattern),
                    "stage": stage, "judges_used": judges_total,
                    "confidence": outcome.confidence, "winner": outcome.selected.clone(),
                }),
                published_at: SystemTime::now(),
            });
        }
        let calls = self.k + 2 * self.k * rounds_done + judges_total;
        let acceptable = answer_is_present(&outcome.selected); // MEM-2.c : réponse finale présente ?
        TopologyOutput {
            output: outcome.selected,
            agents_used: calls,
            cost_units: calls as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals { candidates: k, verified: rounds_done, subtasks: judges_total },
            decided_pattern: None, // débat : juge final, pas de pattern d'agrégation de vote (ADR-019)
            escalated: false,
            produced_acceptable: acceptable,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Arguers Mode A intégrés (déterministes — tests + harnais)
// ─────────────────────────────────────────────────────────────────────────────

/// Arguer déterministe : synthétise un argument à partir du candidat + du
/// contre-argument adverse. `stable = true` ⇒ texte identique d'un tour à l'autre
/// (le débat « se répète » → arrêt anticipé par Stability).
pub struct DeterministicArguer {
    side: &'static str,
    stable: bool,
}
#[async_trait]
impl Generator for DeterministicArguer {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let cand = task.pointer("/debate/candidate").cloned().unwrap_or(Value::Null);
        let round = task.pointer("/debate/round").and_then(Value::as_u64).unwrap_or(0);
        let tag = if self.stable { String::new() } else { format!(" r{round}") };
        json!({ "text": format!("{}[c{variant}{tag}]: position on {cand}", self.side) })
    }
}

/// Les arguers canoniques (PRO, CONTRA) — réservés côté SDK pour le mode keyless.
pub fn builtin_arguers(stable: bool) -> (Arc<dyn Generator>, Arc<dyn Generator>) {
    (
        Arc::new(DeterministicArguer { side: "pro", stable }),
        Arc::new(DeterministicArguer { side: "contra", stable }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::EqualsExpected;
    use crate::consensus::VerifierJudge;
    use crate::traits::Verifier;
    use std::sync::Mutex;

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

    fn debate(k: usize, correct: Vec<usize>, judges: usize, rounds: usize, stable: bool) -> DebateTopology {
        let (pro, contra) = builtin_arguers(stable);
        DebateTopology::new(
            Arc::new(TestGen { expected: json!("e"), correct_variants: correct }),
            k,
            pro,
            contra,
            discerning_judges(judges),
            Pattern::Borda,
            rounds,
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn borda_extracts_correct_after_debate() {
        // leurre majoritaire (3 corrects / 7) — le jury discernant + Borda extraient.
        let topo = debate(7, vec![0, 1, 2], 5, 2, false);
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "e" }));
        // comptabilité : K + 2K×rounds + J = 7 + 28 + 5.
        assert_eq!(out.agents_used, 7 + 2 * 7 * 2 + 5);
        assert_eq!(out.quality.verified, 2, "rounds effectués");
        assert_eq!(out.quality.subtasks, 5, "5 ballots");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn rounds_zero_is_plain_consensus() {
        let topo = debate(7, vec![0, 1, 2], 5, 0, false);
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "e" }));
        assert_eq!(out.agents_used, 7 + 5, "r=0 : dégradation propre en consensus pur");
        assert_eq!(out.quality.verified, 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn repeating_arguments_stop_early() {
        // arguers STABLES → le tour 2 répète le tour 1 → Stability arrête à 2 (< 4 demandés).
        let topo = debate(3, vec![0], 3, 4, true);
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.quality.verified, 2, "arrêt anticipé : le débat se répète");
        assert_eq!(out.agents_used, 3 + 2 * 3 * 2 + 3);
    }

    /// Juge-sonde : capture la tâche vue par le jury (pour vérifier le DOSSIER).
    struct ProbeJudge(Arc<Mutex<Option<Value>>>);
    #[async_trait]
    impl Judge for ProbeJudge {
        fn name(&self) -> &str {
            "probe"
        }
        async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
            *self.0.lock().unwrap() = Some(task.clone());
            vec![1.0; candidates.len()]
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn jury_receives_the_full_dossier() {
        let seen = Arc::new(Mutex::new(None));
        let (pro, contra) = builtin_arguers(false);
        let topo = DebateTopology::new(
            Arc::new(TestGen { expected: json!("e"), correct_variants: vec![0] }),
            3,
            pro,
            contra,
            vec![Arc::new(ProbeJudge(seen.clone())) as Arc<dyn Judge>],
            Pattern::Borda,
            2,
        );
        let _ = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        let task = seen.lock().unwrap().clone().expect("le jury a été appelé");
        let dossier = task.get("debate").and_then(Value::as_array).expect("dossier présent");
        assert_eq!(dossier.len(), 3, "un dossier par candidat");
        for d in dossier {
            assert_eq!(d["pro"].as_array().unwrap().len(), 2, "2 tours d'arguments PRO");
            assert_eq!(d["contra"].as_array().unwrap().len(), 2, "2 tours CONTRA");
        }
    }

    /// Juge SENSIBLE AU DOSSIER : plat (0,5 partout) sans dossier non vide —
    /// discernant (1/0 sur `expected`) avec. Modélise « les arguments élèvent v ».
    struct DossierSensitiveJudge {
        expected: Value,
    }
    #[async_trait]
    impl Judge for DossierSensitiveJudge {
        fn name(&self) -> &str {
            "dossier_sensitive"
        }
        async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
            let has_dossier = task
                .get("debate")
                .and_then(Value::as_array)
                .map(|d| {
                    !d.is_empty()
                        && d[0].get("pro").and_then(Value::as_array).map(|p| !p.is_empty()).unwrap_or(false)
                })
                .unwrap_or(false);
            candidates
                .iter()
                .map(|c| {
                    if !has_dossier {
                        0.5 // sans dossier : aucun signal → confiance d'agrégation nulle
                    } else if c.get("value") == Some(&self.expected) {
                        1.0
                    } else {
                        0.0
                    }
                })
                .collect()
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn escalation_stage1_free_vote_when_unanimous() {
        // candidats UNANIMES → le Vote gratuit suffit : ni juge, ni débat (coût K).
        let topo = debate(5, (0..5).collect(), 3, 4, false).with_escalation(0.5, 0.5);
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "e" }));
        assert_eq!(out.agents_used, 5, "étage ① : K seulement");
        assert_eq!(out.quality.subtasks, 0, "0 juge appelé");
        assert_eq!(out.quality.verified, 0, "0 tour de débat");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn escalation_stage2_jury_without_debate_when_discerning() {
        // UN correct face à 6 leurres : le vote (6/7 pour le leurre) reste sous s1=0,9
        // → étage ② ; le jury discernant donne au correct une marge Borda NETTE (pas
        // d'ex-æquo en tête → confiance ≈ 0,58 ≥ 0,3) → le débat est ÉCONOMISÉ.
        let topo = debate(7, vec![0], 5, 4, false).with_escalation(0.9, 0.3);
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "e" }), "Borda extrait le correct sans débat");
        assert_eq!(out.agents_used, 7 + 5, "étage ② : K + J (un seul jury)");
        assert_eq!(out.quality.verified, 0, "0 tour : le débat n'a pas été payé");
        assert_eq!(out.quality.subtasks, 5);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn escalation_stage3_debate_when_jury_hesitates() {
        // juges PLATS sans dossier (confiance 0) → escalade jusqu'au débat ; AVEC le
        // dossier ils discernent → le re-jury extrait le correct. Coût : K + 2J + 2K×r.
        let (pro, contra) = builtin_arguers(false);
        let judges: Vec<Arc<dyn Judge>> = (0..3)
            .map(|_| Arc::new(DossierSensitiveJudge { expected: json!("e") }) as Arc<dyn Judge>)
            .collect();
        let topo = DebateTopology::new(
            Arc::new(TestGen { expected: json!("e"), correct_variants: vec![0, 1, 2] }),
            7,
            pro,
            contra,
            judges,
            Pattern::Borda,
            2,
        )
        .with_escalation(0.9, 0.9);
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.output, json!({ "value": "e" }), "le dossier élève v → le re-jury tranche");
        assert_eq!(out.agents_used, 7 + 3 + 2 * 7 * 2 + 3, "K + jury① + 2K×r + jury②");
        assert_eq!(out.quality.verified, 2, "le débat a tourné (2 tours)");
        assert_eq!(out.quality.subtasks, 6, "les DEUX jurys sont comptés");
    }
}
