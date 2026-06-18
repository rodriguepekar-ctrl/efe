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
use efe_blackboard::{Blackboard, EventSink, SwarmEvent, TopicRouter, WriteSink};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::consensus::selector::{aggregator_for, Pattern, PatternSelector};
use crate::consensus::{AggOutcome, Aggregator, Ballot, Judge, MajorityVote};
use crate::sizing::{run_waves_pure_scheduled, BudgetProbe, DeadlineProbe, KPlan, SizingTrace};
use crate::topology::{answer_is_present, AgentContribution, QualitySignals, SwarmContext, Topology, TopologyOutput};
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
    /// **ADR-014 — taille adaptative** : `Some(plan)` ⇒ la génération échantillonne par
    /// vagues bornées `[k_min, k_max]` et s'arrête à la cible (`self.k` sert alors de
    /// plafond de référence = `plan.k_max`). `None` ⇒ chemin K fixe, inchangé.
    k_plan: Option<KPlan>,
    /// **I-AK4** : sonde de budget coopérative (« n appels de plus, payables ? ») —
    /// injectée par le SDK ; la topologie ignore le modèle de prix.
    budget_probe: Option<BudgetProbe>,
    /// **ADR-018 (I-AD2)** : sonde de deadline coopérative (« la fenêtre temporelle
    /// reste-t-elle ouverte ? ») — jumelle temporelle de `budget_probe`, injectée par le
    /// SDK ; la topologie ignore l'horloge. Consultée au MÊME point de la boucle de
    /// vagues : l'arrêt tombe sur une frontière de vague, le partiel reste scellable.
    deadline_probe: Option<DeadlineProbe>,
    /// **ADR-019 (read-back B, R-a)** : `Some(weights)` ⇒ poids de routage par-variante (indice
    /// `i` = variante `i` = `generator_{i:03}`), pré-calculés par le SDK depuis l'instantané
    /// mémoire (bornés, jamais zéro). L'agrégation pondère alors les candidats SOUS VETO
    /// (`aggregate_weighted`). `None` ⇒ agrégation uniforme (chemin historique, byte-identique).
    /// Longueur ≥ `k_max` (le SDK pré-calcule pour `0..k_max`) ; on tranche `[0..k_used]` au vol.
    reputation_weights: Option<Vec<f64>>,
    /// **ADR-019 (read-back B, R-b/R-c)** : `Some(schedule)` ⇒ **ordonnancement de profils** du sizer
    /// adaptatif — une permutation de `[0, k_max)` où `schedule[p]` = le profil exécuté à la position
    /// `p` (pré-calculé par le SDK depuis l'instantané pinné, cf. `Reputation::profile_schedule`).
    /// La génération adaptative passe alors les bons profils en premier (R-b) sous rotation
    /// d'exploration ; l'attribution crédite `generator_{schedule[i]:03}` (I-REP7) et le poids R-a de
    /// la position `i` devient `w[schedule[i]]` (I-REP6). **L'auteur d'écriture reste "generator"** —
    /// la provenance enregistre le variant réel, le moat est transparent à la permutation (I-REP8).
    /// `None` ⇒ identité (`variant = position`, chemin historique byte-identique, I-AK3 / I-REP10).
    /// N'agit qu'en ADAPTATIF (le gain est l'arrêt anticipé) ; le chemin K-fixe l'ignore.
    reputation_schedule: Option<Vec<usize>>,
}

impl std::fmt::Debug for ConsensusTopology {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsensusTopology")
            .field("k", &self.k)
            .field("judges", &self.judges.len())
            .field("policy", &self.policy)
            .field("instrumented", &self.write_sink.is_some())
            .field("adaptive", &self.k_plan.is_some())
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
            k_plan: None,
            budget_probe: None,
            deadline_probe: None,
            reputation_weights: None,
            reputation_schedule: None,
        }
    }

    /// **ADR-019 (read-back B, R-a)** : branche les poids de routage par-variante (pré-calculés par
    /// le SDK depuis l'instantané mémoire, indice = variante). L'agrégation pondère alors sous veto.
    /// Vides/absents ⇒ uniforme (byte-identique, I-REP2).
    pub fn with_reputation_weights(mut self, weights: Vec<f64>) -> Self {
        self.reputation_weights = Some(weights);
        self
    }

    /// **ADR-019 (read-back B, R-b/R-c)** : branche l'**ordonnancement de profils** (permutation de
    /// `[0, k_max)`, pré-calculée par le SDK depuis l'instantané pinné). La génération adaptative
    /// exécute alors `schedule[p]` à la position `p` (priorisation R-b + rotation R-c) ; l'attribution
    /// et les poids R-a suivent (I-REP6/7). Absent ⇒ identité (byte-identique, I-AK3 / I-REP10).
    /// N'a d'effet qu'avec un `k_plan` adaptatif (le chemin K-fixe ignore l'ordonnancement).
    pub fn with_reputation_schedule(mut self, schedule: Vec<usize>) -> Self {
        self.reputation_schedule = Some(schedule);
        self
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
    /// **ADR-014** : active la **taille adaptative** — la génération échantillonne par
    /// vagues (`plan.batch` ∈ spec, jamais la machine — §2.3 de l'architecture) et
    /// s'arrête à la cible. Le plan est normalisé (I-AK1 : bornes dures).
    pub fn with_k_plan(mut self, plan: KPlan) -> Self {
        let plan = plan.normalized();
        self.k = plan.k_max; // cohérence : le plafond de référence suit le plan
        self.k_plan = Some(plan);
        self
    }
    /// **I-AK4** : branche la sonde de budget coopérative (le SDK enveloppe son
    /// `BudgetGuard`) — aucune vague impayable n'est émise, zéro sentinelle.
    pub fn with_budget_probe(mut self, probe: BudgetProbe) -> Self {
        self.budget_probe = Some(probe);
        self
    }
    /// **ADR-018 (I-AD2)** : branche la sonde de deadline coopérative (le SDK enveloppe
    /// son `DeadlineGuard` à horloge) — aucune vague n'est émise hors de la fenêtre
    /// temporelle ; le partiel produit est scellé sur une frontière de vague nette.
    pub fn with_deadline_probe(mut self, probe: DeadlineProbe) -> Self {
        self.deadline_probe = Some(probe);
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

    /// **ADR-014** : génération par **vagues bornées** (le sizer, `crate::sizing`) —
    /// vague → observation **pure** (I-AK2) → décision dans `[k_min, k_max]` (I-AK1),
    /// sous sonde de budget (I-AK4). Les cellules `/cand/<i>` sont écrites en indices
    /// **continus de POSITION**. **ADR-019 (R-b/R-c)** : sous `reputation_schedule`, le profil
    /// exécuté à la position `i` est `schedule[i]` (I-AK3′) — sans ordonnancement, `variant = i`
    /// (I-AK3 historique). Le point CLÉ du moat (I-REP8) : **l'auteur d'écriture reste l'unique
    /// `"generator"`** quel que soit le profil — la provenance enregistre le variant réel via le
    /// métré, l'ancrage se fait par valeur ; juges, agrégation et DAG ne voient aucune différence.
    async fn generate_adaptive(&self, task: &Value, bb: &Blackboard, plan: &KPlan) -> (Vec<Value>, SizingTrace) {
        let (cands, trace) = run_waves_pure_scheduled(
            &self.generator,
            task,
            plan,
            self.max_concurrency,
            self.budget_probe.as_ref(),
            self.deadline_probe.as_ref(),
            self.reputation_schedule.as_deref(), // R-b/R-c : profil[position] ; None ⇒ identité (I-AK3)
        )
        .await;
        // I-REP8 (linchpin) : auteur INVARIANT "generator" — jamais par-variante. Le variant réel
        // (schedule[i]) est capté par le métré du générateur ; l'ancrage matche par valeur, pas par
        // position. La permutation est donc transparente à la provenance (R0/R1 inchangés).
        let gid = AgentId::new("generator").unwrap();
        for (i, c) in cands.iter().enumerate() {
            let _ = bb.write(&format!("/cand/{i}"), c.clone(), gid.clone());
        }
        (cands, trace)
    }

    /// Phase JUGES sur le substrat : juges-agents (∥, bornés) écrivent `/ballot/<j>`
    /// + publient sur `ballot.<j>` ; collecte via subscribe_prefix (bus = indice) +
    /// réconciliation depuis le Blackboard (vérité). Renvoie (ballots, #reçus via bus).
    #[allow(clippy::too_many_arguments, clippy::doc_lazy_continuation)] // phase interne : params explicites (substrat + bus + tap)
    async fn run_judges(
        &self,
        task: &Value,
        candidates: &[Value],
        bb: &Arc<Blackboard>,
        swarm_id: SwarmId,
        event_tap: Option<Arc<dyn EventSink>>,
    ) -> (Vec<Ballot>, usize) {
        let jn = self.judges.len();
        let cap = (jn * 4).max(256);
        let router = Arc::new(TopicRouter::new(cap));
        if let Some(es) = event_tap {
            router.set_event_sink(es); // Inc.3 : observe les ballots publiés (ballot.<j>).
        }
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
            let jid = AgentId::new(format!("judge_{j:03}")).expect("id de juge valide (judge_NNN, alphanumérique)");
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
        //    ADR-014 : chemin fixe (K d'un bloc) OU adaptatif (vagues bornées + trace).
        let (candidates, sizing) = match &self.k_plan {
            None => (self.generate(task, &bb).await, None),
            Some(plan) => {
                let (c, t) = self.generate_adaptive(task, &bb, plan).await;
                (c, Some((plan.clone(), t)))
            }
        };
        // Le sizer annonce sa décision (topic réservé) : K_used + trajectoire + raison —
        // non dérivables des seuls ballots. Coût nul si aucun tap n'est branché.
        if let (Some((plan, trace)), Some(tap)) = (&sizing, &ctx.event_tap) {
            tap.on_event(&SwarmEvent {
                swarm_id,
                published_by: AgentId::new("sizer").expect("agent id valide"),
                topic: "sizer.decided".to_string(),
                payload: trace.to_json(plan),
                published_at: SystemTime::now(),
            });
        }

        // 2. décide le pattern + si les juges sont nécessaires + si l'on a ESCALADÉ. Le 3ᵉ
        //    booléen (ADR-019) est l'escalade STRICTE : `true` seulement quand une politique
        //    `Escalating` a basculé du Vote gratuit vers les juges payés (vote peu confiant) ;
        //    un pattern fixe/adaptatif non-Vote n'est PAS une escalade (c'est un choix posé).
        let (pattern, judges_needed, escalated) = match &self.policy {
            AggregatorPolicy::Fixed(p) => (*p, *p != Pattern::Vote, false),
            AggregatorPolicy::Adaptive(s) => {
                let p = s.select(&candidates);
                (p, p != Pattern::Vote, false)
            }
            AggregatorPolicy::Escalating { vote_confidence_threshold, escalate_to } => {
                // tente le Vote (GRATUIT) ; escalade seulement si peu confiant.
                let vote = MajorityVote.aggregate(task, &candidates, &[]);
                if vote.confidence >= *vote_confidence_threshold {
                    (Pattern::Vote, false, false) // confiant → juges ÉCONOMISÉS, pas d'escalade
                } else {
                    (*escalate_to, true, true) // escalade → on paie les juges
                }
            }
        };

        // 3. phase juges sur le substrat — UNIQUEMENT si nécessaire.
        let (ballots, bus_count) = if judges_needed && !self.judges.is_empty() {
            self.run_judges(task, &candidates, &bb, swarm_id, ctx.event_tap.clone()).await
        } else {
            (Vec::new(), 0)
        };
        let judges_run = ballots.len();

        // 4. agrégation — pondérée par la réputation (read-back B, R-a) si des poids sont branchés,
        //    SINON uniforme (chemin historique, byte-identique). Les poids sont indexés par variante
        //    (`generator_{i:03}` = candidat i) ; on tranche `[0..k_used]` (le SDK pré-calcule 0..k_max).
        let agg = aggregator_for(pattern);
        // ADR-019 (read-back B) : poids R-a **par POSITION**. Sous ordonnancement (R-b/R-c), la
        // position `i` a exécuté le profil `schedule[i]` → son poids est `w[schedule[i]]` (I-REP6) ;
        // sans ordonnancement, `w[i]` (identité). UNE seule source — le même `reputation_schedule`
        // pilote attribution ET poids, impossible à désynchroniser. Hors-borne (jamais en pratique :
        // |w| = k_max ≥ k_used, schedule ∈ [0,k_max)) ⇒ poids neutre 1,0 (repli sûr, I-REP2).
        let pos_weights: Option<Vec<f64>> = self.reputation_weights.as_ref().map(|w| {
            (0..candidates.len())
                .map(|i| {
                    let profil = self.reputation_schedule.as_deref().and_then(|s| s.get(i).copied()).unwrap_or(i);
                    w.get(profil).copied().unwrap_or(1.0)
                })
                .collect()
        });
        let outcome = match &pos_weights {
            Some(w) => agg.aggregate_weighted(task, &candidates, &ballots, w),
            None => agg.aggregate(task, &candidates, &ballots),
        };

        // Inc.3 : l'agrégateur annonce son verdict à l'observateur (topic réservé) — pattern
        // + confiance, non dérivables des seuls ballots. Coût nul si aucun tap n'est branché.
        if let Some(tap) = &ctx.event_tap {
            tap.on_event(&SwarmEvent {
                swarm_id,
                published_by: AgentId::new("aggregator").expect("agent id valide"),
                topic: "consensus.decided".to_string(),
                payload: json!({
                    "pattern": format!("{pattern:?}"),
                    "confidence": outcome.confidence,
                    "judges_used": judges_run,
                    "winner": outcome.selected.clone(),
                }),
                published_at: SystemTime::now(),
            });
        }

        // read-back B (B2) : l'ATTRIBUTION par-variante, depuis la décision (`selected_index` +
        // `scores`). Calculée AVANT que `output` ne déplace `outcome.selected`. Le SDK la replie en
        // réputation (auparavant dégénérée : un seul agent « generator »).
        let agent_contributions = variant_contributions(&candidates, &outcome, self.reputation_schedule.as_deref());

        // ADR-014 : en adaptatif, la sortie est ENVELOPPÉE `{value, sizing}` — la même
        // grammaire de diagnostic que le gossip (`{value, gossip:{…}}`) : on rend
        // l'observable, jamais seulement la décision. Chemin fixe : forme historique.
        let k_used = candidates.len();
        let output = match &sizing {
            None => outcome.selected,
            Some((plan, trace)) => json!({ "value": outcome.selected, "sizing": trace.to_json(plan) }),
        };
        let acceptable = answer_is_present(&output); // MEM-2.c : le consensus a-t-il une réponse présente ?
        TopologyOutput {
            output,
            // K réellement dépensé (= self.k en fixe ; = K_used ≤ k_max en adaptatif).
            agents_used: k_used + judges_run,
            // coût = générations réelles + appels JUGES réellement effectués
            // (récompense l'escalade ET l'arrêt anticipé du sizer).
            cost_units: k_used as f64 + judges_run as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals {
                candidates: k_used,
                verified: bus_count,
                subtasks: judges_run,
            },
            // ADR-019 (MEM-2.b) : EXPOSE le pattern décidé (forme canonique minuscule —
            // `"vote"`/`"borda"`/`"condorcet"`…) + l'escalade, pour que la mémoire replie un
            // VRAI prior de pattern (auparavant `None`, d'où le vide en MEM-2). C'est le même
            // verdict que l'event `consensus.decided`, désormais porté aussi par `SwarmRun`.
            decided_pattern: Some(format!("{pattern:?}").to_lowercase()),
            escalated,
            produced_acceptable: acceptable,
            agent_contributions,
        }
    }
}

/// **ADR-019 (read-back B, B2 + R-b/R-c)** : l'attribution par-variante d'une décision consensuelle.
/// `won` = la variante a produit le candidat retenu (`selected_index`, exact) ; `verified` = son
/// score consensuel atteint la **barre** (la moyenne des scores du run) — un signal pris des juges
/// RÉELS, jamais fabriqué (I-REP5). Le gagnant (score max ≥ moyenne) est toujours « vérifié ».
/// Scores incohérents (repli sans juge dégénéré) ⇒ vide.
///
/// **I-REP7 (cohérence attribution↔ordre)** : sous `schedule`, la position `i` a été produite par le
/// profil `schedule[i]` ; le crédit va donc à `generator_{schedule[i]:03}`. Les signaux `won` /
/// `verified` restent lus sur la décision **position-indexée** (`outcome`) — seule l'**étiquette** du
/// contributeur suit l'ordonnancement. Sans `schedule` (`None`) ⇒ `generator_{i:03}` (identité).
fn variant_contributions(
    candidates: &[Value],
    outcome: &AggOutcome,
    schedule: Option<&[usize]>,
) -> Vec<AgentContribution> {
    if candidates.is_empty() || outcome.scores.len() != candidates.len() {
        return Vec::new();
    }
    let bar = outcome.scores.iter().sum::<f64>() / candidates.len() as f64;
    (0..candidates.len())
        .map(|i| {
            // le profil RÉEL exécuté à la position i (identité si pas d'ordonnancement, I-AK3).
            let profil = schedule.and_then(|s| s.get(i).copied()).unwrap_or(i);
            AgentContribution {
                agent_id: format!("generator_{profil:03}"),
                won: i == outcome.selected_index,
                verified: outcome.scores[i] >= bar,
            }
        })
        .collect()
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

    #[test]
    fn variant_contributions_credit_the_scheduled_profile_not_the_position() {
        // ADR-019 R-b/R-c (I-REP7) : sous ordonnancement, la position `i` a exécuté le profil
        // `schedule[i]` → le crédit va à `generator_{schedule[i]:03}`, PAS `generator_{i:03}`.
        // Les signaux won/verified restent lus sur la décision position-indexée.
        let cands = vec![json!("a"), json!("a"), json!("b")]; // scores Vote [2,2,1], gagnant index 0
        let outcome = MajorityVote.aggregate(&json!({}), &cands, &[]);
        let schedule = [5usize, 3, 7];
        let c = variant_contributions(&cands, &outcome, Some(&schedule));
        assert_eq!(c[0].agent_id, "generator_005", "position 0 (gagnante) → profil schedule[0]=5");
        assert!(c[0].won, "la position gagnante gagne, étiquetée par son profil");
        assert_eq!(c[1].agent_id, "generator_003", "position 1 → profil schedule[1]=3");
        assert_eq!(c[2].agent_id, "generator_007", "position 2 → profil schedule[2]=7");
        // identité (None) ⇒ generator_{position} (comportement historique).
        let id = variant_contributions(&cands, &outcome, None);
        assert_eq!(id[0].agent_id, "generator_000");
        assert_eq!(id[2].agent_id, "generator_002");
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
    async fn adaptive_k_stops_at_k_min_and_wraps_output_with_sizing() {
        use crate::sizing::{KPlan, StopRule};
        // tous les variants corrects → candidats UNANIMES → accord 1.0 dès k_min :
        // le sizer s'arrête au plancher, et la sortie porte la trace (ADR-014).
        let topo = ConsensusTopology::new(
            Arc::new(TestGen { expected: json!("a"), correct_variants: (0..24).collect() }),
            24,
            discerning_judges(3),
        )
        .with_pattern(Pattern::Borda)
        .with_k_plan(KPlan { k_min: 4, k_max: 24, batch: 4, stop: StopRule::Agreement { threshold: 0.9 } });
        let out = topo.run(&json!({ "expected": "a" }), &SwarmContext::standard()).await;
        assert_eq!(out.output["value"], json!({ "value": "a" }), "le gagnant sous `value`");
        assert_eq!(out.output["sizing"]["k_used"], json!(4), "arrêt au plancher k_min");
        assert_eq!(out.output["sizing"]["stop_reason"], json!("target"));
        assert_eq!(out.quality.candidates, 4);
        assert_eq!(out.agents_used, 4 + 3, "K_used + juges (le sizer économise 20 appels)");
        assert_eq!(out.cost_units, 7.0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn adaptive_k_runs_to_k_max_when_candidates_disagree() {
        use crate::sizing::{KPlan, StopRule};
        // 1 seul variant correct → candidats jamais d'accord à 0.9 → plafond k_max (I-AK1),
        // et Borda + juges discriminants extraient quand même le correct (I-AK5 :
        // l'agrégation est inchangée — le sizer a choisi COMBIEN, pas QUOI).
        let topo = ConsensusTopology::new(
            Arc::new(TestGen { expected: json!("e"), correct_variants: vec![0] }),
            7,
            discerning_judges(5),
        )
        .with_pattern(Pattern::Borda)
        .with_k_plan(KPlan { k_min: 3, k_max: 7, batch: 2, stop: StopRule::Agreement { threshold: 0.9 } });
        let out = topo.run(&json!({ "expected": "e" }), &SwarmContext::standard()).await;
        assert_eq!(out.output["sizing"]["k_used"], json!(7), "plafond dur atteint");
        assert_eq!(out.output["sizing"]["stop_reason"], json!("k_max"));
        assert_eq!(out.output["value"], json!({ "value": "e" }), "Borda extrait le correct");
        assert_eq!(out.agents_used, 7 + 5);
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
