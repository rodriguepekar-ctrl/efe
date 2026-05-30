// SPDX-License-Identifier: UNLICENSED
//! DTOs d'entrée du bridge — couche anti-corruption mince.
//!
//! Le front n'envoie PAS un [`efe_coordinator::MissionARequest`] brut (qui
//! expose des champs internes : `task_class_scoring_table`, `tenant_id`, …). Il
//! envoie un DTO minimal qu'on mappe ici vers la requête moteur.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use efe_coordinator::{MissionARequest, MissionBRequest, SpeculativeRequest};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

/// Corps de `POST /v1/missions/a`.
#[derive(Debug, Clone, Deserialize)]
pub struct MissionADto {
    /// Le texte utilisateur (obligatoire, non vide après trim).
    pub user_text: String,
    /// Catégorie (`"FactualQa"`, `"Code"`, …). Défaut `FactualQa`.
    #[serde(default)]
    pub task_class: Option<String>,
    /// LLMs candidats. Défaut : `config.default_llms`.
    #[serde(default)]
    pub available_llms: Option<Vec<String>>,
}

impl MissionADto {
    /// Mappe le DTO → [`MissionARequest`].
    ///
    /// Validation : `user_text` non vide, `task_class` parseable, au moins 1
    /// LLM (du DTO ou des défauts). Les métadonnées LLM sont des placeholders
    /// de preview (le coût RÉEL affiché vient de la réponse provider, pas de
    /// ces métadonnées — cf. architecture §6).
    ///
    /// # Errors
    /// Retourne un message si `user_text` est vide, `task_class` invalide, ou
    /// aucun LLM disponible.
    pub fn into_request(self, default_llms: &[String]) -> Result<MissionARequest, String> {
        let user_text = self.user_text.trim().to_string();
        if user_text.is_empty() {
            return Err("user_text vide".to_string());
        }
        let task_class = parse_task_class(self.task_class.as_deref().unwrap_or("FactualQa"))?;

        let llms = self
            .available_llms
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default_llms.to_vec());
        if llms.is_empty() {
            return Err("aucun LLM disponible (DTO vide + aucun défaut configuré)".to_string());
        }

        let mut llm_metadata = HashMap::new();
        for id in &llms {
            llm_metadata.insert(id.clone(), preview_meta());
        }

        Ok(MissionARequest {
            user_text,
            task_class,
            available_llms: llms,
            llm_metadata,
            budget: Budget {
                max_cost_cents: 0,
                max_cost_microcents: 1_000_000,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(30),
            task_class_scoring_table: None,
            tenant_id: None,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Mission B (fan-out N LLMs + vote)
// ─────────────────────────────────────────────────────────────────────────

/// Corps de `POST /v1/missions/b`.
#[derive(Debug, Clone, Deserialize)]
pub struct MissionBDto {
    /// Texte utilisateur (obligatoire).
    pub user_text: String,
    /// Catégorie (défaut `FactualQa`).
    #[serde(default)]
    pub task_class: Option<String>,
    /// LLMs candidats (≥ 2). Défaut : `config.default_llms`.
    #[serde(default)]
    pub llms: Option<Vec<String>>,
    /// Poids de vote par LLM (défaut : uniformes).
    #[serde(default)]
    pub vote_weights: Option<std::collections::HashMap<String, f32>>,
    /// Quorum requis (défaut : `ceil(|llms| / 2)`).
    #[serde(default)]
    pub quorum: Option<usize>,
}

impl MissionBDto {
    /// Mappe le DTO → [`MissionBRequest`].
    ///
    /// # Errors
    /// `user_text` vide, `task_class` invalide, ou < 2 LLMs.
    pub fn into_request(self, default_llms: &[String]) -> Result<MissionBRequest, String> {
        let user_text = self.user_text.trim().to_string();
        if user_text.is_empty() {
            return Err("user_text vide".to_string());
        }
        let task_class = parse_task_class(self.task_class.as_deref().unwrap_or("FactualQa"))?;
        let llms = self
            .llms
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| default_llms.to_vec());
        if llms.len() < 2 {
            return Err(format!("Mission B requiert ≥ 2 LLMs (reçu {})", llms.len()));
        }
        let mut llm_metadata = HashMap::new();
        for id in &llms {
            llm_metadata.insert(id.clone(), preview_meta());
        }
        Ok(MissionBRequest {
            user_text,
            task_class,
            llms,
            llm_metadata,
            vote_weights: self.vote_weights.unwrap_or_default(),
            quorum: self.quorum,
            candidate_sources: vec![],
            max_total_latency: Duration::from_secs(30),
            tenant_id: None,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Mission Speculative (drafter cheap + verifier premium)
// ─────────────────────────────────────────────────────────────────────────

/// Corps de `POST /v1/missions/speculative`.
#[derive(Debug, Clone, Deserialize)]
pub struct SpecDto {
    /// Texte utilisateur (obligatoire).
    pub user_text: String,
    /// Catégorie (défaut `FactualQa`).
    #[serde(default)]
    pub task_class: Option<String>,
    /// LLM drafter (cheap, e.g. `"mock-gemini"`).
    pub drafter_llm: String,
    /// LLM verifier (premium, e.g. `"mock-claude"`).
    pub verifier_llm: String,
}

impl SpecDto {
    /// Mappe le DTO → [`SpeculativeRequest`] (via le constructeur moteur qui
    /// dérive `verifier_metadata` de la table S6 par défaut).
    ///
    /// # Errors
    /// `user_text` vide, `task_class` invalide, drafter/verifier vides.
    pub fn into_request(self) -> Result<SpeculativeRequest, String> {
        let user_text = self.user_text.trim().to_string();
        if user_text.is_empty() {
            return Err("user_text vide".to_string());
        }
        if self.drafter_llm.trim().is_empty() {
            return Err("drafter_llm vide".to_string());
        }
        if self.verifier_llm.trim().is_empty() {
            return Err("verifier_llm vide".to_string());
        }
        let task_class = parse_task_class(self.task_class.as_deref().unwrap_or("FactualQa"))?;
        Ok(SpeculativeRequest::with_drafter_and_verifier(
            user_text,
            task_class,
            self.drafter_llm,
            self.verifier_llm,
        ))
    }
}

/// Métadonnées LLM placeholder pour le routing en preview.
fn preview_meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 0,
        cost_output_per_million: 0,
        cost_input_per_million_microcents: 1_000,
        cost_output_per_million_microcents: 1_000,
        p50_latency_ms: 200,
        p99_latency_ms: 800,
        success_rate: 1.0,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

/// Parse une string `task_class` (accepte CamelCase et SCREAMING_SNAKE_CASE).
fn parse_task_class(s: &str) -> Result<TaskClass, String> {
    let upper = match s {
        "FactualQa" | "FACTUAL_QA" => "FACTUAL_QA",
        "Code" | "CODE" => "CODE",
        "Creative" | "CREATIVE" => "CREATIVE",
        "Analysis" | "ANALYSIS" => "ANALYSIS",
        "Reasoning" | "REASONING" => "REASONING",
        "Translation" | "TRANSLATION" => "TRANSLATION",
        other => other,
    };
    serde_json::from_value::<TaskClass>(serde_json::json!(upper))
        .map_err(|_| format!("task_class invalide: {s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_user_text_rejected() {
        let dto = MissionADto {
            user_text: "   ".into(),
            task_class: None,
            available_llms: None,
        };
        assert!(dto.into_request(&["mock".into()]).is_err());
    }

    #[test]
    fn defaults_to_factual_qa_and_default_llms() {
        let dto = MissionADto {
            user_text: "Capitale ?".into(),
            task_class: None,
            available_llms: None,
        };
        let req = dto.into_request(&["mock".into()]).expect("ok");
        assert_eq!(req.task_class, TaskClass::FactualQa);
        assert_eq!(req.available_llms, vec!["mock".to_string()]);
        assert!(req.llm_metadata.contains_key("mock"));
    }

    #[test]
    fn parses_screaming_and_camel_task_class() {
        assert_eq!(parse_task_class("Code").unwrap(), TaskClass::Code);
        assert_eq!(parse_task_class("CODE").unwrap(), TaskClass::Code);
        assert!(parse_task_class("Nonsense").is_err());
    }

    #[test]
    fn mission_b_requires_two_llms() {
        let one = MissionBDto {
            user_text: "x".into(),
            task_class: None,
            llms: Some(vec!["a".into()]),
            vote_weights: None,
            quorum: None,
        };
        assert!(one.into_request(&[]).is_err(), "1 LLM → refus");

        let two = MissionBDto {
            user_text: "x".into(),
            task_class: None,
            llms: Some(vec!["a".into(), "b".into()]),
            vote_weights: None,
            quorum: None,
        };
        let req = two.into_request(&[]).expect("2 LLMs ok");
        assert_eq!(req.llms.len(), 2);
        assert!(req.llm_metadata.contains_key("a") && req.llm_metadata.contains_key("b"));
    }

    #[test]
    fn mission_b_defaults_to_config_llms() {
        let dto = MissionBDto {
            user_text: "x".into(),
            task_class: None,
            llms: None,
            vote_weights: None,
            quorum: None,
        };
        let defaults = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let req = dto.into_request(&defaults).expect("défauts ok");
        assert_eq!(req.llms, defaults);
    }

    #[test]
    fn speculative_validates_drafter_verifier() {
        let bad = SpecDto {
            user_text: "x".into(),
            task_class: None,
            drafter_llm: "  ".into(),
            verifier_llm: "v".into(),
        };
        assert!(bad.into_request().is_err(), "drafter vide → refus");

        let ok = SpecDto {
            user_text: "Capitale ?".into(),
            task_class: Some("FactualQa".into()),
            drafter_llm: "mock-gemini".into(),
            verifier_llm: "mock-claude".into(),
        };
        let req = ok.into_request().expect("ok");
        assert_eq!(req.drafter_llm, "mock-gemini");
        assert_eq!(req.verifier_llm, "mock-claude");
    }
}
