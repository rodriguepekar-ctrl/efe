// SPDX-License-Identifier: UNLICENSED
//! Types I/O du robot **Routeur** (bge-m3 + Pareto sur coût/latence/qualité).
//!
//! Robot ONNX : pour une tâche entrante, sélectionne le gros LLM cible
//! optimal selon contraintes utilisateur (budget, latence max).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use validator::Validate;

/// Identifiant d'un LLM gros (ex: `claude_opus_4_7`, `gpt4_turbo`, `gemini_2_5`).
pub type LlmRef = String;

/// Description d'une tâche à router.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct TaskDescription {
    /// Prompt complet ou résumé court.
    #[validate(length(min = 1, max = 50_000))]
    pub prompt_summary: String,

    /// Catégorie sémantique de la tâche.
    pub task_class: TaskClass,

    /// Tokens estimés du prompt (compute côté gateway via tokenizer).
    #[validate(range(min = 1, max = 200_000))]
    pub estimated_input_tokens: u32,

    /// Tokens attendus en sortie (cible).
    #[validate(range(min = 1, max = 10_000))]
    pub expected_output_tokens: u32,
}

/// Catégorie sémantique d'une tâche (enum fermé).
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaskClass {
    /// QA simple.
    FactualQa,
    /// Analyse / synthèse.
    Analysis,
    /// Code / programmation.
    Code,
    /// Génération créative.
    Creative,
    /// Multi-hop reasoning.
    Reasoning,
    /// Multilingue.
    Translation,
}

/// Métadonnées sur un LLM disponible (mises à jour en continu par metering).
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct LlmMetadata {
    /// Coût input pour 1M tokens en cents (ex: 1500 = 15 $).
    pub cost_input_per_million: u32,

    /// Coût output pour 1M tokens en cents.
    pub cost_output_per_million: u32,

    /// Latence p50 récente (ms).
    pub p50_latency_ms: u32,

    /// Latence p99 récente (ms).
    pub p99_latency_ms: u32,

    /// Taux de succès récent ∈ [0,1].
    #[validate(range(min = 0.0, max = 1.0))]
    pub success_rate: f32,

    /// Capacités déclarées (mapping TaskClass → score 0..1).
    pub capability_scores: HashMap<String, f32>,

    /// Circuit breaker actuellement ouvert ?
    pub circuit_open: bool,
}

/// Contraintes budgétaires pour cette requête.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Budget {
    /// Cost max en cents (ex: 100 = 1 $).
    pub max_cost_cents: u32,

    /// Latence max en ms (souvent issue du SLA tenant).
    pub max_latency_ms: u32,

    /// Qualité min acceptable ∈ [0,1] (proxy historique).
    #[validate(range(min = 0.0, max = 1.0))]
    pub min_quality_score: f32,
}

/// Entrée du robot Routeur.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct RouterInput {
    /// La tâche à router.
    pub task: TaskDescription,

    /// LLM candidats.
    #[validate(length(min = 1, max = 10))]
    pub available_llms: Vec<LlmRef>,

    /// Métadonnées par LLM.
    pub llm_metadata: HashMap<LlmRef, LlmMetadata>,

    /// Contraintes du user.
    pub user_budget: Budget,
}

/// Sortie du Routeur.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterOutput {
    /// LLM choisi primaire.
    pub selected: LlmRef,

    /// Chaîne de fallback en cas d'erreur (ordre).
    pub fallback_chain: Vec<LlmRef>,

    /// Coût estimé en cents.
    pub estimated_cost_cents: u32,

    /// Latence estimée en ms.
    pub estimated_latency_ms: u32,

    /// Raison du choix (debug + audit).
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_class_serializes_screaming() {
        let s = serde_json::to_string(&TaskClass::FactualQa).unwrap();
        assert_eq!(s, "\"FACTUAL_QA\"");
    }

    #[test]
    fn task_description_validates() {
        let task = TaskDescription {
            prompt_summary: "What is the Fed rate?".into(),
            task_class: TaskClass::FactualQa,
            estimated_input_tokens: 100,
            expected_output_tokens: 50,
        };
        assert!(task.validate().is_ok());
    }

    #[test]
    fn budget_with_invalid_quality_fails() {
        let b = Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 1.5, // > 1.0
        };
        assert!(b.validate().is_err());
    }

    #[test]
    fn llm_metadata_serializes() {
        let meta = LlmMetadata {
            cost_input_per_million: 1500,
            cost_output_per_million: 7500,
            p50_latency_ms: 900,
            p99_latency_ms: 3000,
            success_rate: 0.995,
            capability_scores: HashMap::new(),
            circuit_open: false,
        };
        let s = serde_json::to_string(&meta).unwrap();
        let back: LlmMetadata = serde_json::from_str(&s).unwrap();
        assert_eq!(meta.cost_input_per_million, back.cost_input_per_million);
    }
}
