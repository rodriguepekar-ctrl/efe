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
    ///
    /// **LEGACY** : utiliser `cost_input_per_million_microcents` (R3.S1)
    /// pour la précision sub-cent (Gemini Flash : 7.5 cents/M).
    pub cost_input_per_million: u32,

    /// Coût output pour 1M tokens en cents.
    pub cost_output_per_million: u32,

    /// **R3.S1** : coût input pour 1M tokens en **microcents** (1 µ¢ = 1/1000 cent).
    ///
    /// `#[serde(default)]` pour rétrocompat avec les fichiers JSON pré-R3.
    /// Quand vaut `0`, fallback sur `cost_input_per_million × 1000` au runtime
    /// (cf. helper `cost_input_microcents_resolved()`).
    #[serde(default)]
    pub cost_input_per_million_microcents: u64,

    /// **R3.S1** : coût output pour 1M tokens en **microcents**.  Cf. ci-dessus.
    #[serde(default)]
    pub cost_output_per_million_microcents: u64,

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

impl LlmMetadata {
    /// **R3.S1** : résout le tarif input en µ¢/M en gérant les anciens JSON
    /// (où `cost_input_per_million_microcents` vaut 0).  Si > 0, on l'utilise
    /// tel quel ; sinon, on dérive du legacy `cost_input_per_million × 1000`.
    #[must_use]
    pub fn cost_input_microcents_resolved(&self) -> u64 {
        if self.cost_input_per_million_microcents > 0 {
            self.cost_input_per_million_microcents
        } else {
            (self.cost_input_per_million as u64).saturating_mul(1000)
        }
    }

    /// **R3.S1** : résout le tarif output en µ¢/M (cf. ci-dessus).
    #[must_use]
    pub fn cost_output_microcents_resolved(&self) -> u64 {
        if self.cost_output_per_million_microcents > 0 {
            self.cost_output_per_million_microcents
        } else {
            (self.cost_output_per_million as u64).saturating_mul(1000)
        }
    }
}

/// Contraintes budgétaires pour cette requête.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct Budget {
    /// Cost max en cents (ex: 100 = 1 $).
    ///
    /// **LEGACY** : utiliser `max_cost_microcents` (R3.S1) pour la précision
    /// sub-cent (filtrage Pareto plus fin sur fixtures cheap).
    pub max_cost_cents: u32,

    /// **R3.S1** : Cost max en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// Permet au filtrage budgétaire du router de discriminer des LLMs
    /// dont le coût estimé est < 1 cent (e.g. Gemini Flash sur 50/20 tokens
    /// ≈ 1 µ¢ vs Claude 45 µ¢ — différenciation invisible en cents).
    ///
    /// `#[serde(default)]` pour rétrocompat : si 0 et `max_cost_cents > 0`,
    /// le helper `max_cost_microcents_resolved()` dérive `cents × 1000`.
    #[serde(default)]
    pub max_cost_microcents: u64,

    /// Latence max en ms (souvent issue du SLA tenant).
    pub max_latency_ms: u32,

    /// Qualité min acceptable ∈ [0,1] (proxy historique).
    #[validate(range(min = 0.0, max = 1.0))]
    pub min_quality_score: f32,
}

impl Budget {
    /// **R3.S1** — Résout le budget µ¢ avec fallback `cents × 1000`.
    ///
    /// Si `max_cost_microcents > 0` → utilise directement (précision R3+).
    /// Sinon → dérive depuis `max_cost_cents` (rétrocompat pré-R3).
    /// Si les deux sont à 0 → 0 (budget zéro, no LLM disponible).
    #[must_use]
    pub fn max_cost_microcents_resolved(&self) -> u64 {
        if self.max_cost_microcents > 0 {
            self.max_cost_microcents
        } else {
            (self.max_cost_cents as u64).saturating_mul(1000)
        }
    }
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
    ///
    /// **LEGACY** : utiliser `estimated_cost_microcents` (R3.S1) pour la
    /// précision sub-cent.
    pub estimated_cost_cents: u32,

    /// **R3.S1** : coût estimé en microcents (1 µ¢ = 1/1000 cent).
    ///
    /// `#[serde(default)]` pour compat ; quand vaut 0, fallback sur
    /// `estimated_cost_cents × 1000`.
    #[serde(default)]
    pub estimated_cost_microcents: u64,

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
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 1.5, // > 1.0
        };
        assert!(b.validate().is_err());
    }

    // ============================================================
    // R3.S1 — Tests Budget microcents (Gap A)
    // ============================================================

    #[test]
    fn r3s1_budget_max_cost_microcents_resolved_uses_microcents_when_set() {
        let b = Budget {
            max_cost_cents: 100,
            max_cost_microcents: 75_500,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        };
        // R3+ caller — utilise µ¢ directement (75.5 ¢).
        assert_eq!(b.max_cost_microcents_resolved(), 75_500);
    }

    #[test]
    fn r3s1_budget_max_cost_microcents_resolved_falls_back_to_cents() {
        // Pré-R3 caller — uniquement cents peuplé.
        let b = Budget {
            max_cost_cents: 50,
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        };
        // 50 cents × 1000 = 50_000 µ¢.
        assert_eq!(b.max_cost_microcents_resolved(), 50_000);
    }

    #[test]
    fn r3s1_budget_both_zero_returns_zero() {
        let b = Budget {
            max_cost_cents: 0,
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        };
        assert_eq!(b.max_cost_microcents_resolved(), 0);
    }

    #[test]
    fn r3s1_budget_backward_compat_pre_r3_json() {
        // JSON pré-R3 sans max_cost_microcents.
        let json = r#"{
            "max_cost_cents": 25,
            "max_latency_ms": 3000,
            "min_quality_score": 0.7
        }"#;
        let b: Budget = serde_json::from_str(json).unwrap();
        assert_eq!(b.max_cost_cents, 25);
        // serde(default) → 0
        assert_eq!(b.max_cost_microcents, 0);
        // Resolved derive depuis cents : 25 × 1000 = 25_000 µ¢.
        assert_eq!(b.max_cost_microcents_resolved(), 25_000);
    }

    #[test]
    fn llm_metadata_serializes() {
        let meta = LlmMetadata {
            cost_input_per_million: 1500,
            cost_output_per_million: 7500,
            cost_input_per_million_microcents: 1_500_000,
            cost_output_per_million_microcents: 7_500_000,
            p50_latency_ms: 900,
            p99_latency_ms: 3000,
            success_rate: 0.995,
            capability_scores: HashMap::new(),
            circuit_open: false,
        };
        let s = serde_json::to_string(&meta).unwrap();
        let back: LlmMetadata = serde_json::from_str(&s).unwrap();
        assert_eq!(meta.cost_input_per_million, back.cost_input_per_million);
        assert_eq!(
            meta.cost_input_per_million_microcents,
            back.cost_input_per_million_microcents
        );
    }

    // ============================================================
    // R3.S1 — Tests microcents
    // ============================================================

    #[test]
    fn r3s1_llm_metadata_backward_compat_pre_r3_json() {
        // JSON sans champs microcents (= pré-R3 sérialisé).
        let json = r#"{
            "cost_input_per_million": 300,
            "cost_output_per_million": 1500,
            "p50_latency_ms": 800,
            "p99_latency_ms": 2500,
            "success_rate": 0.99,
            "capability_scores": {},
            "circuit_open": false
        }"#;
        let m: LlmMetadata = serde_json::from_str(json).unwrap();
        // Les champs µ¢ doivent être 0 par défaut.
        assert_eq!(m.cost_input_per_million_microcents, 0);
        assert_eq!(m.cost_output_per_million_microcents, 0);
        // Mais le helper `_resolved()` doit reconstruire depuis cents.
        assert_eq!(m.cost_input_microcents_resolved(), 300_000);
        assert_eq!(m.cost_output_microcents_resolved(), 1_500_000);
    }

    #[test]
    fn r3s1_llm_metadata_resolved_uses_microcents_when_set() {
        let m = LlmMetadata {
            cost_input_per_million: 8, // legacy (arrondi sup pour Flash)
            cost_output_per_million: 30,
            cost_input_per_million_microcents: 7_500, // précis (D7 fix)
            cost_output_per_million_microcents: 30_000,
            p50_latency_ms: 400,
            p99_latency_ms: 1200,
            success_rate: 0.99,
            capability_scores: HashMap::new(),
            circuit_open: false,
        };
        // _resolved() doit privilégier les µ¢ (précis) → pas le legacy arrondi.
        assert_eq!(m.cost_input_microcents_resolved(), 7_500);
        assert_eq!(m.cost_output_microcents_resolved(), 30_000);
    }

    #[test]
    fn r3s1_router_output_backward_compat_pre_r3_json() {
        // LlmRef est un alias String → la valeur est une string nue.
        let json = r#"{
            "selected": "claude-sonnet-4-6",
            "fallback_chain": [],
            "estimated_cost_cents": 5,
            "estimated_latency_ms": 1500,
            "reason": "test"
        }"#;
        let o: RouterOutput = serde_json::from_str(json).unwrap();
        // µ¢ par défaut = 0 (pas re-dérivé ici, on garde la valeur stockée).
        assert_eq!(o.estimated_cost_microcents, 0);
        assert_eq!(o.estimated_cost_cents, 5);
    }
}
