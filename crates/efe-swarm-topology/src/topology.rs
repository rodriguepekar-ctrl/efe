// SPDX-License-Identifier: UNLICENSED
//! **Phase 2 — couche d'abstraction unifiée** (architecture §3).
//!
//! [`Topology`] : interface OBJECT-SAFE commune (Value → [`TopologyOutput`]) — un
//! `Vec<Box<dyn Topology>>` permet d'exécuter/comparer T1, T2, T3 uniformément.
//! [`SwarmContext`] injecte les services partagés (la [`AgentFleet`]).
//! [`TopologyOutput`] uniformise la comptabilité (coût, latence, agents, qualité).

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use efe_blackboard::{EventSink, SubstrateMetrics, SwarmEvent, WriteSink};
use efe_nano_sdk::{AgentId, SwarmId};
use serde_json::{json, Value};

use crate::fleet::AgentFleet;
use crate::generate_verify::GenerateVerify;
use crate::hierarchical::Hierarchical;
use crate::pipeline::Pipeline;
use crate::sizing::{run_waves, BudgetProbe, DeadlineProbe, KPlan, StopRule};
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
    /// **ADR-019 (MEM-2.b)** : le **pattern d'agrégation gagnant** (`"vote"`/`"borda"`/
    /// `"condorcet"`…), si la topologie en décide un (consensus). `None` pour les topologies
    /// sans notion de pattern (GV/T1, pipeline, hiérarchique, gossip, débat). C'est le signal
    /// que la mémoire replie dans le **prior de pattern** (`PatternPrior`) et que `SwarmRun`
    /// rend observable — auparavant non exposé sur `SwarmRun` (d'où le « vide » de MEM-2).
    pub decided_pattern: Option<String>,
    /// **ADR-019 (MEM-2.b)** : le run a-t-il **escaladé** (vote peu confiant → juges payés) ?
    /// `true` uniquement sur la branche d'escalade d'une politique `Escalating` ; `false` en
    /// pattern fixe/adaptatif et hors consensus. Replié dans `PatternPrior::escalations`.
    pub escalated: bool,
    /// **ADR-019 (MEM-2.c)** : la topologie a-t-elle produit une **réponse acceptable** ? Calculé
    /// PAR la topologie (qui seule connaît sa sémantique d'acceptation) : GV/T1 ⇒ au moins un
    /// candidat a **passé le vérificateur** (`passed > 0`) — sans quoi le `best_any` non-nul mais
    /// NON vérifié masquerait un échec ; consensus/pipeline/hiérarchique/gossip/débat ⇒ la
    /// **réponse est présente** ([`answer_is_present`], non nulle/vide, forme `value`|`text`|nue).
    /// `false` ⇒ la mémoire d'échec compte un échec (`RunSummary::failed`). Le SDK ne **devine**
    /// plus l'échec depuis la forme du gagnant (faux négatifs sur GV `best_any` + sorties `text` vides).
    pub produced_acceptable: bool,
    /// **ADR-019 (read-back B, B2)** : l'**attribution par-variante** d'un run consensus — qui a
    /// gagné (`won = i == selected_index`), qui a été vérifié (`verified = score_i ≥ barre`), sous
    /// l'id stable `generator_{i:03}`. Vide pour les topologies sans notion de variantes attribuables.
    /// Le SDK le replie en réputation (`RunSummary::agents`) — auparavant dégénéré (un seul agent).
    pub agent_contributions: Vec<AgentContribution>,
}

/// **ADR-019 (read-back B)** : la contribution d'UNE variante à un run — son id stable + a-t-elle
/// gagné / été vérifiée. Calculé PAR la topologie (qui seule connaît `selected_index` + les scores),
/// replié en réputation par le SDK. Jamais fabriqué (I-REP5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentContribution {
    pub agent_id: String,
    pub won: bool,
    pub verified: bool,
}

/// **ADR-019 (MEM-2.c)** — la sortie d'une topologie porte-t-elle une **réponse présente** (non
/// nulle, non vide) ? On lit la réponse là où chaque grammaire la loge — `value` (consensus/GV),
/// `text` (pipeline/mergers), ou la valeur **nue** — puis on teste la vacuité (null, chaîne vide,
/// tableau/objet vide). Les topologies **sans** notion de vérification par-candidat s'en servent
/// pour renseigner [`TopologyOutput::produced_acceptable`] (le producteur connaît sa forme).
pub fn answer_is_present(output: &Value) -> bool {
    let w = output.get("value").or_else(|| output.get("text")).unwrap_or(output);
    match w {
        Value::Null => false,
        Value::String(s) => !s.trim().is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
        Value::Bool(_) | Value::Number(_) => true,
    }
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
            decided_pattern: None, // T1/GV : aucune notion de pattern d'agrégation (ADR-019)
            escalated: false,
            // MEM-2.c : ACCEPTABLE ⇔ au moins un candidat a passé le vérificateur. `best_any`
            // (le repli non vérifié) est non-nul mais NE compte PAS comme réponse acceptée.
            produced_acceptable: r.passed > 0,
            agent_contributions: Vec::new(), // GV : pas d'attribution par-variante (read-back B = consensus)
        }
    }
}

/// **T1 ADAPTATIF** (ADR-014) — la variante sœur de [`GvTopology`] : génération par
/// **vagues bornées** (`crate::sizing`), arrêt au **quota vérifié** (ou à toute autre
/// [`StopRule`]), sélection a-posteriori inchangée (I-AK5). C'est le cas-phare du K
/// adaptatif : sur la zone vérifiable, le coût attendu d'un « premier candidat
/// vérifié » est 1/p appels, là où le K fixe paie K quoi qu'il arrive.
///
/// Adaptateur **séparé** (et non des champs ajoutés à `GvTopology`) : zéro churn sur
/// les constructions littérales existantes — la déviation assumée façon ADR-013.
pub struct AdaptiveGvTopology {
    pub gen: Arc<dyn Generator>,
    pub ver: Arc<dyn Verifier>,
    pub plan: KPlan,
    /// Largeur d'exécution de la vague (I-CP1 : jamais une sémantique).
    pub width: usize,
    /// Sonde de budget coopérative (I-AK4) — injectée par le SDK.
    pub budget_probe: Option<BudgetProbe>,
    /// **ADR-018 (I-AD2)** : sonde de deadline coopérative — jumelle temporelle de
    /// `budget_probe`, injectée par le SDK. Même point d'arrêt (frontière de vague).
    pub deadline_probe: Option<DeadlineProbe>,
}

#[async_trait]
impl Topology for AdaptiveGvTopology {
    fn name(&self) -> &str {
        "T1:generate-verify"
    }
    async fn run(&self, task: &Value, ctx: &SwarmContext) -> TopologyOutput {
        let t0 = Instant::now();
        let plan = self.plan.clone().normalized();
        // L'observable : le QUOTA compte les candidats qui PASSENT le vérificateur
        // (a-posteriori, gratuit — cohérent avec le modèle de coût T1) ; les règles
        // pures (accord / erreur-type) restent disponibles telles quelles.
        let ver = self.ver.clone();
        let task_for_obs = task.clone();
        let stop = plan.stop.clone();
        let observe = move |cands: &[Value]| match &stop {
            StopRule::VerifiedQuota { .. } => {
                cands.iter().filter(|c| ver.passes(&task_for_obs, c)).count() as f64
            }
            pure => pure.observe_candidates(cands).unwrap_or(f64::NAN),
        };
        let (cands, trace) = run_waves(
            &self.gen,
            task,
            &plan,
            self.width,
            self.budget_probe.as_ref(),
            self.deadline_probe.as_ref(),
            &observe,
        )
        .await;

        // Le sizer annonce sa décision (même topic réservé que le consensus adaptatif).
        if let Some(tap) = &ctx.event_tap {
            tap.on_event(&SwarmEvent {
                swarm_id: SwarmId::new(),
                published_by: AgentId::new("sizer").expect("agent id valide"),
                topic: "sizer.decided".to_string(),
                payload: trace.to_json(&plan),
                published_at: SystemTime::now(),
            });
        }

        let k_used = cands.len();
        let r = GenerateVerify::select_from(task, cands, self.ver.as_ref());
        TopologyOutput {
            // ADR-014 : sortie enveloppée `{value, sizing}` — la grammaire gossip.
            output: json!({
                "value": r.selected.clone().unwrap_or(Value::Null),
                "sizing": trace.to_json(&plan),
            }),
            agents_used: k_used,
            cost_units: k_used as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals { candidates: k_used, verified: r.passed, subtasks: 0 },
            decided_pattern: None, // T1 adaptatif : pas de pattern non plus (ADR-019)
            escalated: false,
            produced_acceptable: r.passed > 0, // MEM-2.c : ≥ 1 candidat vérifié
            agent_contributions: Vec::new(),
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
        let output = outs.into_iter().next().unwrap_or(Value::Null);
        let acceptable = answer_is_present(&output); // MEM-2.c : sortie présente (≠ {text:""} vide)
        TopologyOutput {
            output,
            agents_used: self.pipeline.len(),
            cost_units: self.pipeline.len() as f64,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals { candidates: 0, verified: 0, subtasks: produced },
            decided_pattern: None, // T2 pipeline : pas d'agrégation par pattern (ADR-019)
            escalated: false,
            produced_acceptable: acceptable,
            agent_contributions: Vec::new(),
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
        let acceptable = answer_is_present(&r.output); // MEM-2.c : réponse présente (≠ décompo vide)
        TopologyOutput {
            output: r.output,
            agents_used: r.subtasks,
            cost_units: r.subtasks as f64 * self.cost_per_subtask,
            elapsed_ms: t0.elapsed().as_millis() as u64,
            quality: QualitySignals { candidates: 0, verified: 0, subtasks: r.subtasks },
            decided_pattern: None, // T3 hiérarchique : pas de pattern d'agrégation (ADR-019)
            escalated: false,
            produced_acceptable: acceptable,
            agent_contributions: Vec::new(),
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

    /// Générateur qui ne réussit qu'à partir d'un variant donné (difficulté réglable).
    struct LateGen {
        first_ok: usize,
    }
    #[async_trait]
    impl Generator for LateGen {
        async fn generate(&self, task: &Value, variant: usize) -> Value {
            if variant >= self.first_ok {
                json!({ "value": task.get("expected").cloned().unwrap_or(Value::Null) })
            } else {
                json!({ "value": format!("__wrong_{variant}__") })
            }
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn adaptive_t1_stops_at_first_verified_quota() {
        use crate::sizing::{KPlan, StopRule};
        // tâche FACILE (OkGen réussit dès le variant 0) : quota 1 atteint dans la
        // première vague → K_used = k_min = 2 au lieu de k_max = 8 (coût ÷4).
        let topo = AdaptiveGvTopology {
            gen: Arc::new(OkGen),
            ver: Arc::new(EqualsExpected),
            plan: KPlan { k_min: 2, k_max: 8, batch: 2, stop: StopRule::VerifiedQuota { quota: 1 } },
            width: 4,
            budget_probe: None,
            deadline_probe: None,
        };
        let out = topo.run(&json!({ "expected": "tech" }), &SwarmContext::standard()).await;
        assert_eq!(out.output["value"]["value"], "tech", "le vérifié est sélectionné");
        assert_eq!(out.output["sizing"]["k_used"], json!(2), "arrêt au plancher");
        assert_eq!(out.output["sizing"]["stop_reason"], json!("target"));
        assert_eq!(out.agents_used, 2);
        assert_eq!(out.quality.verified, 2, "OkGen : tous les candidats passent");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn adaptive_t1_escalates_waves_until_difficulty_yields() {
        use crate::sizing::{KPlan, StopRule};
        // tâche DURE (premier succès au variant 5) : vagues 2, 4, 6 → quota atteint
        // à K_used = 6 — l'adaptatif paie exactement ce que la difficulté exige.
        let topo = AdaptiveGvTopology {
            gen: Arc::new(LateGen { first_ok: 5 }),
            ver: Arc::new(EqualsExpected),
            plan: KPlan { k_min: 2, k_max: 8, batch: 2, stop: StopRule::VerifiedQuota { quota: 1 } },
            width: 4,
            budget_probe: None,
            deadline_probe: None,
        };
        let out = topo.run(&json!({ "expected": "x" }), &SwarmContext::standard()).await;
        assert_eq!(out.output["sizing"]["k_used"], json!(6), "2 → 4 → 6 : le succès tombe au variant 5");
        assert_eq!(out.output["value"]["value"], "x", "sélection a-posteriori inchangée (I-AK5)");
        assert_eq!(out.quality.verified, 1, "un seul passant (variant 5)");
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
        assert_eq!(ctx.fleet.len(), 44); // 34 catalogue + 9 juges + null_generator (B0.2)
        assert!(ctx.fleet.stage("uppercase").is_some());
        assert!(ctx.fleet.judge("consistency").is_some());
    }
}
