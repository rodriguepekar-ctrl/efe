// SPDX-License-Identifier: UNLICENSED
//! Types I/O des 5 missions Phase 3.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use efe_nano_app_coordinator::types::router::TaskClass;

use crate::reformulator_ext::ReformulationHop;

/// Mode de vote pour Mission Vote 3-LLM.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VoteMode {
    /// Plurality pondéré (Phase 2 default).
    Plurality,
    /// Borda count — chaque LLM ranke les 3 outputs.
    Borda,
    /// Pairwise Condorcet — élimination face-à-face.
    Condorcet,
}

// ─── Mission Vote 3-LLM (raffinement Mission B Phase 2) ───────────

/// Requête pour Vote 3-LLM avec reformulation pré-vote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote3LlmRequest {
    /// Texte utilisateur.
    pub user_text: String,
    /// Catégorie de tâche (hint pour routing/compression).
    pub task_class: TaskClass,
    /// 3 LLMs cibles (canonical : claude, gpt, gemini).
    pub llms: Vec<String>,
    /// Mode de vote.
    #[serde(default = "default_vote_mode")]
    pub mode: VoteMode,
    /// Quorum requis (par défaut `ceil(n/2)`).
    #[serde(default)]
    pub quorum: Option<usize>,
    /// Budget hard-cap en cents.
    #[serde(default = "default_budget_cents")]
    pub max_budget_cents: u32,
    /// Deadline globale.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

fn default_vote_mode() -> VoteMode {
    VoteMode::Borda
}
const fn default_budget_cents() -> u32 {
    500
}
fn default_deadline() -> Duration {
    Duration::from_secs(30)
}

/// Réponse de Vote 3-LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote3LlmResponse {
    /// Texte gagnant (canonical synthesized).
    pub final_text: String,
    /// LLM gagnant.
    pub winner_llm: String,
    /// Mode utilisé.
    pub mode: VoteMode,
    /// Score gagnant.
    pub winner_score: f32,
    /// Scores par LLM.
    pub per_llm_scores: HashMap<String, f32>,
    /// Coût total (cents) — incluant reformulator + verifier nano-robots.
    pub total_cost_cents: u32,
    /// Latence ms.
    pub latency_ms: u64,
    /// Identifiant du swarm en hexadécimal canonique (`Uuid::simple`, 32 chiffres) — **ce n'est PAS
    /// un haché**, et rien n'est chaîné ici. Le champ a longtemps porté le nom `audit_chain_hash` en affirmant « Hash
    /// chaîne d'audit » : il est SÉRIALISÉ tel quel par la passerelle, donc SCELLÉ dans le reçu,
    /// donc lisible par un auditeur — un nom qui promet une chaîne d'audit là où il n'y a qu'un
    /// identifiant recopié est exactement la classe de bug « vert malhonnête » que ce dépôt
    /// traque. La VRAIE chaîne vit ailleurs : la racine HMAC par tenant de `efe-swarm-provenance`,
    /// exposée par `GET /v1/swarm/verify/{run_id}`. Ce champ ne sert qu'à corréler une réponse à
    /// son swarm. (Le `audit_chain_hash` de `efe-coordinator`, lui, est un vrai BLAKE3 sur les
    /// entrées d'audit : le nom y est mérité, il n'a pas été touché.)
    ///
    /// **Et il ne corrélait même pas.** Le rendu était `format!("{:016x}", swarm_id.as_u128())` :
    /// `016` borne la largeur MINIMALE, pas celle d'un u128. Un UUID v7 porte un horodatage 48 bits
    /// dont le premier quartet reste nul tout ce siècle, donc la valeur servie faisait **31**
    /// caractères là où la forme canonique en fait 32 — elle n'était égale à l'identifiant du swarm
    /// pour AUCUN run. Un auditeur joignant le reçu sur cet identifiant ne trouvait jamais rien, en
    /// silence. On sérialise désormais `swarm_id.simple()`, où la largeur ne peut plus être fausse.
    pub swarm_id_hex: String,
    /// LLMs cancellés pour early termination (I9).
    #[serde(default)]
    pub cancelled_llms: Vec<String>,
    /// **Fil B.3** — un hop estampillé par claim entrée au vote (moteur
    /// effectif + faits d'audit SLM). Scellé dans le reçu re-vérifiable par
    /// la gateway. Additif (`default` : rétro-compat des sorties stockées).
    #[serde(default)]
    pub reformulation_hops: Vec<ReformulationHop>,
}

// ─── Mission Relay (séquentiel typé) ──────────────────────────────

/// Requête Relay — séquence forcée Plan → Code → Synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayRequest {
    /// Texte utilisateur.
    pub user_text: String,
    /// LLM pour phase α (Planning).
    pub planner_llm: String,
    /// LLM pour phase β (Implementation).
    pub implementer_llm: String,
    /// LLM pour phase γ (Synthesis).
    pub synthesizer_llm: String,
    /// Budget total.
    #[serde(default = "default_budget_cents")]
    pub max_budget_cents: u32,
    /// Deadline globale.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

/// Réponse Relay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayResponse {
    /// Texte final = synthesis du Synthesizer.
    pub final_text: String,
    /// Plan canonical produit phase α (JSON sérialisé).
    pub plan_json: String,
    /// Code canonical produit phase β (JSON sérialisé).
    pub code_json: String,
    /// Synthesis canonical produit phase γ.
    pub synthesis_json: String,
    /// LLMs effectivement utilisés (peut différer si fallback).
    pub llms_used: Vec<String>,
    /// Coût total.
    pub total_cost_cents: u32,
    /// Latence.
    pub latency_ms: u64,
    /// Identifiant du swarm en hexadécimal — **pas un haché** (cf. `VoteResponse::swarm_id_hex`).
    pub swarm_id_hex: String,
    /// **Fil B.3** — les 3 hops estampillés (`plan`/`code`/`synthesis`) :
    /// moteur effectif + faits d'audit SLM par hop. Scellés dans le reçu.
    #[serde(default)]
    pub reformulation_hops: Vec<ReformulationHop>,
}

// ─── Mission Debate (3 rounds + jury) ─────────────────────────────

/// Requête Debate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateRequest {
    /// Question débattue.
    pub user_text: String,
    /// LLM pro.
    pub pro_llm: String,
    /// LLM contra.
    pub contra_llm: String,
    /// LLM jury.
    pub jury_llm: String,
    /// Rounds (1..[`crate::DEBATE_MAX_ROUNDS`]).  Par défaut 2 (opening + rebut).
    #[serde(default = "default_rounds")]
    pub rounds: u8,
    /// Budget total.
    #[serde(default = "default_budget_cents")]
    pub max_budget_cents: u32,
    /// Deadline globale.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

const fn default_rounds() -> u8 {
    2
}

/// Réponse Debate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateResponse {
    /// Verdict final (synthesis du jury).
    pub verdict: String,
    /// Tous les arguments par round (round → [argument, argument, …]).
    pub arguments_per_round: Vec<Vec<DebateArgumentSummary>>,
    /// LLM jury qui a rendu le verdict.
    pub jury_llm: String,
    /// Coût total.
    pub total_cost_cents: u32,
    /// Latence.
    pub latency_ms: u64,
    /// Identifiant du swarm en hexadécimal — **pas un haché** (cf. `VoteResponse::swarm_id_hex`).
    pub swarm_id_hex: String,
    /// **Fil B.3** — 2 hops estampillés par round (`argument:rN:PRO|CONTRA`).
    /// Le verdict du jury n'est PAS reformulé (honnête : pas de hop jury).
    #[serde(default)]
    pub reformulation_hops: Vec<ReformulationHop>,
}

/// Résumé public d'un argument (pour la réponse Debate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebateArgumentSummary {
    /// Auteur.
    pub author: String,
    /// Position (`PRO`/`CONTRA`/`REBUTTAL`).
    pub stance: String,
    /// Thèse résumée.
    pub thesis: String,
}

// ─── Mission Specialist (decompose + N route + aggregate) ─────────

/// Requête Specialist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialistRequest {
    /// Texte utilisateur (peut contenir plusieurs sub-tasks).
    pub user_text: String,
    /// LLMs disponibles.
    pub available_llms: Vec<String>,
    /// Capability scores par LLM (TaskClass → 0..1).
    pub capability_scores: HashMap<String, HashMap<String, f32>>,
    /// Budget total.
    #[serde(default = "default_budget_cents")]
    pub max_budget_cents: u32,
    /// Deadline globale.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

/// Réponse Specialist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialistResponse {
    /// Synthèse finale (combine les N sub-results).
    pub final_text: String,
    /// Sub-tasks et assignations.
    pub assignments: Vec<SpecialistAssignment>,
    /// Coût total.
    pub total_cost_cents: u32,
    /// Latence.
    pub latency_ms: u64,
    /// Identifiant du swarm en hexadécimal — **pas un haché** (cf. `VoteResponse::swarm_id_hex`).
    pub swarm_id_hex: String,
}

/// Assignation d'une sub-task à un LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecialistAssignment {
    /// ID 1-based de la sub-task.
    pub sub_task_id: u32,
    /// Texte de la sub-task.
    pub task: String,
    /// Catégorie inférée.
    pub task_class: TaskClass,
    /// LLM assigné.
    pub assigned_llm: String,
    /// Output canonique.
    pub output_text: String,
}

// ─── Mission Cascade (Speculative 3 niveaux) ──────────────────────

/// Requête Cascade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeRequest {
    /// Texte utilisateur.
    pub user_text: String,
    /// Catégorie sémantique (hint pour le verifier).
    pub task_class: TaskClass,
    /// LLM drafter local (cheap, fast).
    pub drafter_llm: String,
    /// LLM verifier (medium cost).
    pub verifier_llm: String,
    /// LLM validator final (cher, dernier ressort).
    pub validator_llm: String,
    /// Seuil de similarité au-dessus duquel le drafter est accepté tel quel.
    #[serde(default = "default_accept_threshold")]
    pub accept_threshold: f32,
    /// Budget total.
    #[serde(default = "default_budget_cents")]
    pub max_budget_cents: u32,
    /// Deadline globale.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

const fn default_accept_threshold() -> f32 {
    0.85
}

/// Réponse Cascade.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CascadeResponse {
    /// Texte final.
    pub final_text: String,
    /// Décision du verifier (`ACCEPT` | `REGEN_RANGES` | `REGEN_ALL`).
    pub verifier_decision: String,
    /// Validator a-t-il été appelé ?
    pub validator_used: bool,
    /// Texte drafté.
    pub draft_text: String,
    /// Coût total.
    pub total_cost_cents: u32,
    /// Latence.
    pub latency_ms: u64,
    /// Latence gagnée vs baseline (ms).
    pub latency_saved_ms: i64,
    /// Identifiant du swarm en hexadécimal — **pas un haché** (cf. `VoteResponse::swarm_id_hex`).
    pub swarm_id_hex: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vote_mode_serializes_screaming() {
        assert_eq!(serde_json::to_string(&VoteMode::Borda).unwrap(), "\"BORDA\"");
        assert_eq!(serde_json::to_string(&VoteMode::Condorcet).unwrap(), "\"CONDORCET\"");
        assert_eq!(serde_json::to_string(&VoteMode::Plurality).unwrap(), "\"PLURALITY\"");
    }

    #[test]
    fn default_vote_mode_is_borda() {
        let json = r#"{"user_text":"x","task_class":"FACTUAL_QA","llms":["a","b","c"]}"#;
        let req: Vote3LlmRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.mode, VoteMode::Borda);
        assert_eq!(req.max_budget_cents, 500);
    }

    #[test]
    fn relay_request_roundtrips() {
        let r = RelayRequest {
            user_text: "build server".into(),
            planner_llm: "claude".into(),
            implementer_llm: "gpt".into(),
            synthesizer_llm: "gemini".into(),
            max_budget_cents: 1000,
            max_total_latency: Duration::from_secs(10),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: RelayRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.planner_llm, "claude");
    }

    #[test]
    fn debate_default_rounds_is_2() {
        let json = r#"{"user_text":"x","pro_llm":"a","contra_llm":"b","jury_llm":"c"}"#;
        let req: DebateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.rounds, 2);
    }

    #[test]
    fn cascade_default_threshold_is_0_85() {
        let json = r#"{"user_text":"x","task_class":"FACTUAL_QA","drafter_llm":"d","verifier_llm":"v","validator_llm":"V"}"#;
        let req: CascadeRequest = serde_json::from_str(json).unwrap();
        assert!((req.accept_threshold - 0.85).abs() < 1e-6);
    }

    #[test]
    fn specialist_request_capability_scores_keyed_by_llm() {
        let mut caps = HashMap::new();
        let mut c = HashMap::new();
        c.insert("CODE".to_string(), 0.94);
        caps.insert("gpt".to_string(), c);
        let r = SpecialistRequest {
            user_text: "x".into(),
            available_llms: vec!["gpt".into(), "claude".into()],
            capability_scores: caps,
            max_budget_cents: 100,
            max_total_latency: Duration::from_secs(10),
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: SpecialistRequest = serde_json::from_str(&s).unwrap();
        assert!((back.capability_scores["gpt"]["CODE"] - 0.94).abs() < 1e-6);
    }
}
