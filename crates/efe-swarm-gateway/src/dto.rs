// SPDX-License-Identifier: UNLICENSED
//! **DTO de fil** — le contrat HTTP **versionné**, découplé des types SDK (qui ne dérivent
//! pas `Deserialize`). On *désérialise* ici, puis on *mappe* vers `TopologySpec`/`Budget`/
//! `ClientMode`. Le SDK reste pur ; le contrat de fil évolue indépendamment.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use efe_swarm_sdk::{Budget, ClientMode, ConvergenceMetric, OnExceed, Pattern, SwarmRun, TopologySpec};

use crate::error::GatewayError;

fn default_k() -> usize {
    3
}

/// Corps de `POST /v1/swarm/run`.
#[derive(Debug, Clone, Deserialize)]
pub struct RunRequest {
    /// La tâche (JSON libre) passée à l'essaim.
    pub task: Value,
    /// La topologie à exécuter (enum tagué par `kind`).
    pub topology: TopologyDto,
    /// Le budget plafonné (défaut : illimité, politique `stop`).
    #[serde(default)]
    pub budget: BudgetDto,
    /// Le mode client (défaut : `keyless`).
    #[serde(default)]
    pub mode: ModeDto,
}

impl RunRequest {
    /// Démonte le DTO en types SDK prêts pour `swarm.run_traced(task, spec, budget)`.
    pub fn into_parts(self) -> Result<(Value, TopologySpec, Budget, ClientMode), GatewayError> {
        let spec = self.topology.into_spec()?;
        let budget = self.budget.into_budget();
        let mode = self.mode.into_mode();
        Ok((self.task, spec, budget, mode))
    }
}

/// Topologie de fil — enum **externe-tagué** par `kind` (snake_case).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopologyDto {
    /// T1 — Generate-Verify (K générateurs + vérificateur déterministe).
    GenerateVerify {
        generator: String,
        verifier: String,
        #[serde(default = "default_k")]
        k: usize,
    },
    /// T2 — Pipeline (étages nommés chaînés).
    Pipeline { stages: Vec<String> },
    /// T3 — Hiérarchique (décompose → résout → fusionne).
    Hierarchical { decomposer: String, solver: String, merger: String },
    /// T-Consensus (K générateurs + panel de juges → Vote/Borda/Condorcet).
    Consensus {
        generator: String,
        judges: Vec<String>,
        #[serde(default = "default_k")]
        k: usize,
        #[serde(default)]
        pattern: PatternDto,
        #[serde(default)]
        escalate: bool,
    },
    /// T4 — Gossip (itératif, ADR-003) : K refiners × tours, arrêt à ε ou r_max.
    Gossip {
        refiner: String,
        #[serde(default = "default_k")]
        k: usize,
        #[serde(default = "default_rounds")]
        max_rounds: usize,
        #[serde(default = "default_epsilon")]
        epsilon: f64,
        #[serde(default)]
        metric: MetricDto,
    },
    /// Debate (itératif, ADR-005) : consensus à dossier contradictoire.
    Debate {
        generator: String,
        judges: Vec<String>,
        #[serde(default = "default_pro")]
        pro: String,
        #[serde(default = "default_contra")]
        contra: String,
        #[serde(default = "default_k")]
        k: usize,
        #[serde(default)]
        pattern: PatternDto,
        #[serde(default = "default_rounds")]
        rounds: usize,
        /// Escalade à trois étages (Vote → jury → débat).
        #[serde(default)]
        escalate: bool,
    },
    /// Auto — choisit selon la tâche + le mode.
    Auto,
}

fn default_rounds() -> usize {
    2
}
fn default_epsilon() -> f64 {
    0.05
}
fn default_pro() -> String {
    "Argue FOR the candidate: cite its strongest evidence.".into()
}
fn default_contra() -> String {
    "Argue AGAINST the candidate: cite its weakest point.".into()
}

/// Métrique de convergence de fil (snake_case).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDto {
    #[default]
    Variance,
    Agreement,
    Stability,
}
impl From<MetricDto> for ConvergenceMetric {
    fn from(m: MetricDto) -> Self {
        match m {
            MetricDto::Variance => ConvergenceMetric::Variance,
            MetricDto::Agreement => ConvergenceMetric::Agreement,
            MetricDto::Stability => ConvergenceMetric::Stability,
        }
    }
}

impl TopologyDto {
    fn into_spec(self) -> Result<TopologySpec, GatewayError> {
        Ok(match self {
            TopologyDto::GenerateVerify { generator, verifier, k } => {
                TopologySpec::GenerateVerify { generator, verifier, k }
            }
            TopologyDto::Pipeline { stages } => {
                if stages.is_empty() {
                    return Err(GatewayError::BadRequest("topology.pipeline : `stages` vide".into()));
                }
                TopologySpec::Pipeline { stages }
            }
            TopologyDto::Hierarchical { decomposer, solver, merger } => {
                TopologySpec::Hierarchical { decomposer, solver, merger }
            }
            TopologyDto::Consensus { generator, judges, k, pattern, escalate } => {
                if judges.is_empty() {
                    return Err(GatewayError::BadRequest("topology.consensus : `judges` vide".into()));
                }
                TopologySpec::Consensus { generator, judges, k, pattern: pattern.into(), escalate }
            }
            TopologyDto::Gossip { refiner, k, max_rounds, epsilon, metric } => {
                TopologySpec::Gossip { refiner, k, max_rounds, epsilon, metric: metric.into() }
            }
            TopologyDto::Debate { generator, judges, pro, contra, k, pattern, rounds, escalate } => {
                if judges.is_empty() {
                    return Err(GatewayError::BadRequest("topology.debate : `judges` vide".into()));
                }
                TopologySpec::Debate { generator, judges, pro, contra, k, pattern: pattern.into(), rounds, escalate }
            }
            TopologyDto::Auto => TopologySpec::Auto,
        })
    }
}

/// Pattern de consensus de fil.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternDto {
    Vote,
    #[default]
    Borda,
    Condorcet,
}

impl From<PatternDto> for Pattern {
    fn from(p: PatternDto) -> Self {
        match p {
            PatternDto::Vote => Pattern::Vote,
            PatternDto::Borda => Pattern::Borda,
            PatternDto::Condorcet => Pattern::Condorcet,
        }
    }
}

/// Budget de fil.
#[derive(Debug, Clone, Deserialize)]
pub struct BudgetDto {
    pub max_cents: u32,
    #[serde(default)]
    pub on_exceed: OnExceedDto,
    /// Prix par appel (cents). `None` ⇒ le prix du mode est utilisé.
    #[serde(default)]
    pub price_per_call_cents: Option<u32>,
}

impl Default for BudgetDto {
    fn default() -> Self {
        // Défaut sûr : illimité + arrêt propre (jamais de dépassement, jamais de refus).
        Self { max_cents: u32::MAX, on_exceed: OnExceedDto::Stop, price_per_call_cents: None }
    }
}

impl BudgetDto {
    fn into_budget(self) -> Budget {
        let mut b = Budget::cents(self.max_cents).with_policy(self.on_exceed.into());
        if let Some(p) = self.price_per_call_cents {
            b = b.with_price_per_call(p);
        }
        b
    }
}

/// Politique de dépassement de fil.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExceedDto {
    Error,
    #[default]
    Stop,
    Degrade,
}

impl From<OnExceedDto> for OnExceed {
    fn from(o: OnExceedDto) -> Self {
        match o {
            OnExceedDto::Error => OnExceed::Error,
            OnExceedDto::Stop => OnExceed::Stop,
            OnExceedDto::Degrade => OnExceed::Degrade,
        }
    }
}

/// Mode client de fil — **externe-tagué** : `"keyless"` (chaîne) **ou**
/// `{"self_host":{"endpoint":..,"model":..}}` **ou** `{"network":{"provider":..}}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeDto {
    #[default]
    Keyless,
    SelfHost { endpoint: String, model: String },
    Network { provider: String },
}

impl ModeDto {
    fn into_mode(self) -> ClientMode {
        match self {
            ModeDto::Keyless => ClientMode::Keyless,
            ModeDto::SelfHost { endpoint, model } => ClientMode::SelfHost { endpoint, model },
            ModeDto::Network { provider } => ClientMode::Network { provider },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Réponses de fil (Serialize)
// ─────────────────────────────────────────────────────────────────────────────

/// Réponse de `POST /v1/swarm/run`.
#[derive(Debug, Clone, Serialize)]
pub struct RunResponse {
    pub run_id: Uuid,
    /// Le `SwarmRun` (sortie + coût + budget + provenance{root,verified} + qualité).
    pub run: SwarmRun,
}

/// Réponse de `GET /v1/swarm/verify/:id` — **la re-preuve indépendante** (le *moat*).
#[derive(Debug, Clone, Serialize)]
pub struct VerifyResponse {
    pub run_id: Uuid,
    /// `verify_swarm` a-t-il **tout** re-prouvé (chaîne + contenu + DAG) ?
    pub verified: bool,
    /// `provenance_root` reconstruit depuis les records + la clé tenant.
    pub root: Option<String>,
    pub chain_intact: bool,
    pub content_intact: bool,
    pub dag_consistent: bool,
    pub traced_pct: f64,
    pub records: usize,
    /// Premier enregistrement en faute (forensique), si altération.
    pub first_break: Option<Uuid>,
}

/// Vue d'un run (`GET /v1/swarm/runs/:id`) — **terminé** (dans le store) **ou en cours**
/// (un job live encore en exécution). Tagué par `status`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunStatusView {
    /// Run terminé et persisté (records re-vérifiables via `verify/:id`).
    Finished {
        run_id: Uuid,
        output: Value,
        topology: String,
        root: Option<String>,
        records: usize,
        /// Date de création (epoch, secondes).
        created_at_unix: u64,
    },
    /// Job live encore en cours — suivez le flux SSE `events`.
    Running { run_id: Uuid, events: String },
}

/// Paramètres de requête de `POST /v1/swarm/run` (`?stream=true` → mode **job** live).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunParams {
    /// `true` ⇒ exécution **asynchrone** observée (dashboard live) : réponse 202 immédiate.
    #[serde(default)]
    pub stream: bool,
}

/// Réponse 202 d'un **job** live (`POST /v1/swarm/run?stream=true`).
#[derive(Debug, Clone, Serialize)]
pub struct JobAccepted {
    pub run_id: Uuid,
    pub status: &'static str,
    /// URL du flux SSE des événements live.
    pub events: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_consensus_with_defaults() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": { "text": "x" },
            "topology": { "kind": "consensus", "generator": "g", "judges": ["j1", "j2"] }
        }))
        .unwrap();
        let (_t, spec, budget, mode) = req.into_parts().unwrap();
        match spec {
            TopologySpec::Consensus { k, judges, pattern, .. } => {
                assert_eq!(k, 3, "k par défaut");
                assert_eq!(judges.len(), 2);
                assert_eq!(pattern, Pattern::Borda, "pattern par défaut");
            }
            other => panic!("attendu consensus, eu {other:?}"),
        }
        assert_eq!(budget.max_cents, u32::MAX, "budget par défaut illimité");
        assert_eq!(mode, ClientMode::Keyless, "mode par défaut keyless");
    }

    #[test]
    fn mode_keyless_is_a_bare_string() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "auto" },
            "mode": "keyless"
        }))
        .unwrap();
        assert_eq!(req.into_parts().unwrap().3, ClientMode::Keyless);
    }

    #[test]
    fn mode_self_host_is_externally_tagged() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "auto" },
            "mode": { "self_host": { "endpoint": "http://127.0.0.1:11434", "model": "gemma3:12b" } }
        }))
        .unwrap();
        match req.into_parts().unwrap().3 {
            ClientMode::SelfHost { endpoint, model } => {
                assert_eq!(endpoint, "http://127.0.0.1:11434");
                assert_eq!(model, "gemma3:12b");
            }
            other => panic!("attendu self_host, eu {other:?}"),
        }
    }

    #[test]
    fn empty_pipeline_is_rejected() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "pipeline", "stages": [] }
        }))
        .unwrap();
        assert!(matches!(req.into_parts(), Err(GatewayError::BadRequest(_))));
    }

    #[test]
    fn budget_maps_policy_and_price() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "auto" },
            "budget": { "max_cents": 50, "on_exceed": "degrade", "price_per_call_cents": 2 }
        }))
        .unwrap();
        let (_t, _s, budget, _m) = req.into_parts().unwrap();
        assert_eq!(budget.max_cents, 50);
        assert_eq!(budget.on_exceed, OnExceed::Degrade);
        assert_eq!(budget.price_per_call_cents, Some(2));
    }

    #[test]
    fn gossip_and_debate_wire_forms_parse() {
        // Gossip : défauts (k=3, r=2, ε=0.05, metric=variance).
        let req: RunRequest = serde_json::from_value(serde_json::json!({
            "task": { "initial": [1.0, 2.0] },
            "topology": { "kind": "gossip", "refiner": "mean_pull" }
        }))
        .unwrap();
        match req.topology.into_spec().unwrap() {
            TopologySpec::Gossip { refiner, k, max_rounds, epsilon, .. } => {
                assert_eq!(refiner, "mean_pull");
                assert_eq!((k, max_rounds), (3, 2));
                assert!((epsilon - 0.05).abs() < 1e-12);
            }
            other => panic!("attendu Gossip, eu {other:?}"),
        }
        // Debate : juges requis ; pro/contra ont des instructions par défaut.
        let req: RunRequest = serde_json::from_value(serde_json::json!({
            "task": { "text": "t" },
            "topology": { "kind": "debate", "generator": "g", "judges": ["j1"], "rounds": 1 }
        }))
        .unwrap();
        match req.topology.into_spec().unwrap() {
            TopologySpec::Debate { judges, rounds, pro, .. } => {
                assert_eq!(judges.len(), 1);
                assert_eq!(rounds, 1);
                assert!(pro.contains("FOR"), "instruction PRO par défaut");
            }
            other => panic!("attendu Debate, eu {other:?}"),
        }
        // Debate sans juge → 400.
        let req: RunRequest = serde_json::from_value(serde_json::json!({
            "task": {}, "topology": { "kind": "debate", "generator": "g", "judges": [] }
        }))
        .unwrap();
        assert!(req.topology.into_spec().is_err());
    }
}
