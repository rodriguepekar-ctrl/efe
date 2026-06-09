// SPDX-License-Identifier: UNLICENSED
//! **DTO de fil** — le contrat HTTP **versionné**, découplé des types SDK (qui ne dérivent
//! pas `Deserialize`). On *désérialise* ici, puis on *mappe* vers `TopologySpec`/`Budget`/
//! `ClientMode`. Le SDK reste pur ; le contrat de fil évolue indépendamment.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use efe_swarm_sdk::{Budget, ClientMode, OnExceed, Pattern, SwarmRun, TopologySpec};

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
    /// Auto — choisit selon la tâche + le mode.
    Auto,
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

/// Vue résumée d'un run stocké (`GET /v1/swarm/runs/:id`).
#[derive(Debug, Clone, Serialize)]
pub struct StoredRunView {
    pub run_id: Uuid,
    pub output: Value,
    pub topology: String,
    pub root: Option<String>,
    pub records: usize,
    /// Date de création (epoch, secondes).
    pub created_at_unix: u64,
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
}
