// SPDX-License-Identifier: UNLICENSED
//! Types I/O des 3 missions du Coordinator.
//!
//! Le Coordinator expose 3 entrées publiques :
//!
//! - [`MissionARequest`] / [`MissionAResponse`] — pré-mâcher pour 1 LLM.
//! - [`MissionBRequest`] / [`MissionBResponse`] — fan-out N LLMs + vote.
//! - [`SpeculativeRequest`] / [`SpeculativeResponse`] — drafter/verifier.

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

use crate::metrics::ChewingMetrics;

/// Requête utilisateur pour la Mission A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionARequest {
    /// Texte saisi par l'humain.
    pub user_text: String,

    /// Catégorie sémantique (utilisée par le Router pour choisir le LLM).
    pub task_class: TaskClass,

    /// LLMs candidats (au moins 1).
    pub available_llms: Vec<String>,

    /// Métadonnées par LLM (coût/latence/qualité — consommées par le Router).
    pub llm_metadata: HashMap<String, LlmMetadata>,

    /// Contraintes budgétaires.
    pub budget: Budget,

    /// Sources candidates pour le Verifier (≤ 50).  Optionnel : si absent,
    /// le Verifier est skippé (mode "no fact-check").
    #[serde(default)]
    pub candidate_sources: Vec<CandidateSourceRef>,

    /// Deadline globale de la mission.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

/// Réponse finale produite par la Mission A.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionAResponse {
    /// Texte généré par le LLM choisi (concaténation des tokens streamés).
    pub final_text: String,

    /// LLM réellement appelé.
    pub llm_used: String,

    /// Coût constaté (cents).
    pub cost_cents: u32,

    /// Latence end-to-end (ms).
    pub latency_ms: u64,

    /// Verdicts de fact-checking (vide si pas de sources).
    pub verdicts: Vec<FactCheck>,

    /// Hash BLAKE3 de la chaîne d'audit complète (preuve d'intégrité).
    pub audit_chain_hash: String,

    /// Métriques de pré-mâchage (Sprint S4 R2) — `None` si la mission n'a pas
    /// calculé les métriques (ex: tests offline avec mocks, ou versions
    /// antérieures de Mission A).
    ///
    /// `#[serde(default)]` garantit la backward-compat : les `MissionAResponse`
    /// sérialisés sans ce champ continuent à désérialiser correctement.
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,
}

/// Requête pour la Mission B (multi-LLM + vote).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionBRequest {
    /// Texte saisi.
    pub user_text: String,

    /// Catégorie sémantique.
    pub task_class: TaskClass,

    /// LLMs candidats (≥ 2 — sinon fallback Mission A).
    pub llms: Vec<String>,

    /// Métadonnées par LLM.
    pub llm_metadata: HashMap<String, LlmMetadata>,

    /// Poids `w_l` par LLM utilisés dans le vote ([`crate::vote::VoteEngine`]).
    /// Si absent, poids uniformes (1.0 partout).
    #[serde(default)]
    pub vote_weights: HashMap<String, f32>,

    /// Quorum requis (par défaut `ceil(|llms| / 2)`).
    #[serde(default)]
    pub quorum: Option<usize>,

    /// Sources candidates pour le verifier final.
    #[serde(default)]
    pub candidate_sources: Vec<CandidateSourceRef>,

    /// Deadline.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

/// Réponse de la Mission B.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionBResponse {
    /// Texte gagnant du vote (concaténation des tokens).
    pub final_text: String,

    /// LLM gagnant.
    pub winner_llm: String,

    /// Score du gagnant (somme pondérée).
    pub winner_score: f32,

    /// Score de chaque LLM ayant participé (ordre = paramètres).
    pub per_llm_scores: HashMap<String, f32>,

    /// Métriques de pré-mâchage **mutualisé** (Sprint S5 R2) — `None` si la
    /// mission n'a pas calculé les métriques (mocks offline anciens).
    ///
    /// En Mission B, le compressor tourne **1× seulement** mais son output est
    /// envoyé aux N LLMs en parallèle. Les économies sont donc multipliées :
    /// `savings_tokens × N` est l'économie cumulée sur l'ensemble des LLMs.
    ///
    /// - `llm_latency_ms` = `max` des N latences (fan-out parallèle, pas somme)
    /// - `llm_tokens_generated` = somme des tokens output des N LLMs
    /// - `selected_llm` = `winner_llm` (LLM gagnant du vote Symphony, pas du
    ///   router — Mission B n'utilise pas le router)
    /// - `router_*` = vides (Mission B appelle tous les LLMs sans choisir)
    ///
    /// `#[serde(default)]` garantit la backward-compat avec des
    /// `MissionBResponse` sérialisés pré-S5.
    #[serde(default)]
    pub chewing_metrics: Option<ChewingMetrics>,

    /// Coût agrégé (cents).
    pub total_cost_cents: u32,

    /// Latence end-to-end.
    pub latency_ms: u64,

    /// Fact-checks éventuels.
    pub verdicts: Vec<FactCheck>,

    /// Hash audit chain.
    pub audit_chain_hash: String,
}

/// Requête pour la Mission speculative (drafter local + verifier gros LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeculativeRequest {
    /// Texte d'entrée.
    pub user_text: String,

    /// Catégorie.
    pub task_class: TaskClass,

    /// LLM "verifier" (gros — Claude/GPT-4/...).
    pub verifier_llm: String,

    /// Métadonnées du verifier.
    pub verifier_metadata: LlmMetadata,

    /// Budget.
    pub budget: Budget,

    /// Nombre max de tokens à drafter en spéculation.
    #[serde(default = "default_speculative_window")]
    pub max_draft_tokens: u32,

    /// Deadline globale.
    #[serde(default = "default_deadline")]
    pub max_total_latency: Duration,
}

/// Réponse spéculative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeculativeResponse {
    /// Texte final (le verifier a le dernier mot).
    pub final_text: String,

    /// Texte qui avait été drafté avant vérification.
    pub draft_text: String,

    /// Drafter accepté en l'état (= verifier validé sans changement) ?
    pub draft_accepted: bool,

    /// Nombre de tokens du draft acceptés tels quels.
    pub tokens_accepted: u32,

    /// Nombre de tokens regénérés par le verifier.
    pub tokens_regenerated: u32,

    /// Gain de latence vs. baseline (ms négatif si la spéculation a coûté plus cher).
    pub latency_saved_ms: i64,

    /// Latence end-to-end.
    pub latency_ms: u64,

    /// Hash audit chain.
    pub audit_chain_hash: String,
}

/// Verdict de fact-check résumé (alias UI-friendly).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactCheck {
    /// Claim verbatim qui a été vérifié.
    pub claim: String,
    /// Verdict (`SUPPORTED`, `UNSUPPORTED`, `CONTRADICTED`, `INSUFFICIENT_EVIDENCE`).
    pub verdict: String,
    /// Sévérité (`INFO`/`WARN`/`ERROR`).
    pub severity: String,
    /// ID de la source la mieux matchée (None si UNSUPPORTED).
    pub best_source_id: Option<String>,
    /// Similarité 0..1.
    pub similarity: f32,
}

/// Référence à une source candidate (utilisée par les Missions A et B).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSourceRef {
    /// ID unique.
    pub id: String,
    /// Texte de la source.
    pub text: String,
}

fn default_deadline() -> Duration {
    Duration::from_secs(30)
}

const fn default_speculative_window() -> u32 {
    128
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_app_coordinator::types::router::TaskClass;

    #[test]
    fn mission_a_roundtrips() {
        let req = MissionARequest {
            user_text: "what is the Fed rate?".into(),
            task_class: TaskClass::FactualQa,
            available_llms: vec!["claude".into()],
            llm_metadata: HashMap::new(),
            budget: Budget {
                max_cost_cents: 100,
                max_latency_ms: 5000,
                min_quality_score: 0.5,
            },
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(10),
        };
        let s = serde_json::to_string(&req).unwrap();
        let back: MissionARequest = serde_json::from_str(&s).unwrap();
        assert_eq!(back.user_text, "what is the Fed rate?");
    }

    #[test]
    fn mission_b_default_quorum_is_none() {
        let req = MissionBRequest {
            user_text: "x".into(),
            task_class: TaskClass::Analysis,
            llms: vec!["claude".into(), "gpt4".into(), "gemini".into()],
            llm_metadata: HashMap::new(),
            vote_weights: HashMap::new(),
            quorum: None,
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(10),
        };
        assert!(req.quorum.is_none());
    }

    #[test]
    fn speculative_default_window_is_128() {
        let json = r#"{"user_text":"x","task_class":"FACTUAL_QA","verifier_llm":"claude","verifier_metadata":{"cost_input_per_million":1000,"cost_output_per_million":1000,"p50_latency_ms":100,"p99_latency_ms":1000,"success_rate":0.99,"capability_scores":{},"circuit_open":false},"budget":{"max_cost_cents":100,"max_latency_ms":1000,"min_quality_score":0.5}}"#;
        let req: SpeculativeRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.max_draft_tokens, 128);
    }
}
