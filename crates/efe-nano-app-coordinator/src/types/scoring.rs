// SPDX-License-Identifier: UNLICENSED
//! **Sprint S6 R2** — Table de scoring statique par `TaskClass`.
//!
//! Ce module fournit une **abstraction calibrée** au-dessus du
//! [`RouterClient`](crate::clients::router::RouterClient) pour qu'un caller
//! n'ait pas à remplir lui-même un `HashMap<LlmRef, LlmMetadata>` à chaque
//! requête.
//!
//! ## Vue d'ensemble
//!
//! - [`ScoringWeights`] : pondérations Pareto `(quality, latency, cost)`.
//!   Default `(1.0, 1.0, 1.0)`.
//! - [`LlmStaticMetadata`] : `cost_input/output_per_million`, `p50/p99`,
//!   `success_rate`. **Pas de circuit_open ni de capability_scores** : ces
//!   champs viennent respectivement de l'observabilité runtime (S8) et de la
//!   table per-task (ce module).
//! - [`TaskClassScoringTable`] : la table complète. Pour chaque LLM, un
//!   `HashMap<TaskClass, f32>` de capabilités + une [`LlmStaticMetadata`].
//!   Pour chaque `TaskClass`, un [`ScoringWeights`].
//!
//! ## Usage typique (caller)
//!
//! ```ignore
//! use efe_nano_app_coordinator::types::{TaskClass, TaskClassScoringTable};
//! use efe_nano_app_coordinator::clients::router::RouterClient;
//!
//! let table = TaskClassScoringTable::default_3llm_table();
//! let router = RouterClient::for_task_class(&table, TaskClass::Code);
//! let llms = vec![
//!     "claude_claude-sonnet-4-6".to_string(),
//!     "gemini_gemini-2.5-flash".to_string(),
//!     "gpt_gpt-4o".to_string(),
//! ];
//! let metadata_map = table.build_metadata_map(&llms, TaskClass::Code);
//! // → metadata_map est prêt à être passé à `RouterInput.llm_metadata`.
//! ```
//!
//! ## Pourquoi statique et pas dynamique
//!
//! Le metering runtime (mise à jour `success_rate` réelle) est livré en
//! Sprint S11. S6 fournit **les valeurs initiales** + le format de la table,
//! servant aussi de fallback quand le metering est froid (bootstrap).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::types::router::{LlmMetadata, TaskClass};

// ─────────────────────────────────────────────────────────────────────────
// ScoringWeights
// ─────────────────────────────────────────────────────────────────────────

/// Pondérations Pareto utilisées par le router pour un type de tâche.
///
/// Le score d'un LLM candidat est calculé par le router comme :
///
/// ```text
/// score = quality_weight * predicted_quality
///       - latency_weight * normalized_latency
///       - cost_weight    * normalized_cost
/// ```
///
/// Les valeurs absolues n'ont pas de sens (le router normalise par max sur
/// l'ensemble des candidats), seules les **proportions relatives** comptent.
///
/// Exemple : `(2.0, 1.0, 0.5)` privilégie la qualité, tolère la latence,
/// ignore largement le coût (utile pour [`TaskClass::Reasoning`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ScoringWeights {
    /// Coefficient pondérant la qualité prédite (`capability × success_rate`).
    pub quality_weight: f32,
    /// Coefficient pondérant la latence p50 (pénalité).
    pub latency_weight: f32,
    /// Coefficient pondérant le coût estimé (pénalité).
    pub cost_weight: f32,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            quality_weight: 1.0,
            latency_weight: 1.0,
            cost_weight: 1.0,
        }
    }
}

impl ScoringWeights {
    /// Constructeur explicite.
    #[must_use]
    pub const fn new(quality: f32, latency: f32, cost: f32) -> Self {
        Self {
            quality_weight: quality,
            latency_weight: latency,
            cost_weight: cost,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// LlmStaticMetadata
// ─────────────────────────────────────────────────────────────────────────

/// Métadonnées statiques (price-list + benchmarks) d'un LLM.
///
/// Distinct de [`LlmMetadata`] car :
/// - Pas de `capability_scores` (vient de [`TaskClassScoringTable::capability`])
/// - Pas de `circuit_open` (vient de l'observabilité runtime, S8)
///
/// Ces valeurs sont versionnées et **rafraîchies à la main** sur cycle
/// trimestriel (ou lors d'un changement de pricing chez un provider). S11
/// ajoutera une couche dynamique qui maintient `p50_latency_ms` et
/// `success_rate` à jour via métriques observées.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LlmStaticMetadata {
    /// Coût input pour 1M tokens en cents (ex: 1500 = 15 $/M).
    ///
    /// **LEGACY** : utiliser `cost_input_per_million_microcents` (R3.S1) pour
    /// précision sub-cent (Gemini Flash : 7.5 cents/M exacte, sinon arrondi 8).
    pub cost_input_per_million: u32,
    /// Coût output pour 1M tokens en cents.
    pub cost_output_per_million: u32,
    /// **R3.S1** : coût input pour 1M tokens en **microcents** (1 µ¢ = 1/1000 c).
    ///
    /// `#[serde(default)]` pour rétrocompat ; quand vaut 0, fallback runtime
    /// via `cost_input_microcents_resolved()` (= `cost_input_per_million × 1000`).
    #[serde(default)]
    pub cost_input_per_million_microcents: u64,
    /// **R3.S1** : coût output pour 1M tokens en **microcents**.
    #[serde(default)]
    pub cost_output_per_million_microcents: u64,
    /// Latence p50 historique (ms).
    pub p50_latency_ms: u32,
    /// Latence p99 historique (ms).
    pub p99_latency_ms: u32,
    /// Taux de succès historique ∈ [0,1].
    pub success_rate: f32,
}

impl LlmStaticMetadata {
    /// **R3.S1** : tarif input en µ¢/M résolu (fallback legacy si µ¢ == 0).
    #[must_use]
    pub fn cost_input_microcents_resolved(&self) -> u64 {
        if self.cost_input_per_million_microcents > 0 {
            self.cost_input_per_million_microcents
        } else {
            (self.cost_input_per_million as u64).saturating_mul(1000)
        }
    }

    /// **R3.S1** : tarif output en µ¢/M résolu (fallback legacy si µ¢ == 0).
    #[must_use]
    pub fn cost_output_microcents_resolved(&self) -> u64 {
        if self.cost_output_per_million_microcents > 0 {
            self.cost_output_per_million_microcents
        } else {
            (self.cost_output_per_million as u64).saturating_mul(1000)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// TaskClassScoringTable
// ─────────────────────────────────────────────────────────────────────────

/// Table complète **(LLM × TaskClass)** de scoring statique.
///
/// 3 dimensions de données :
///
/// 1. **`capability[llm][task]`** : score `[0,1]` indiquant l'adéquation du
///    LLM pour cette `TaskClass`. Proxy issu des benchmarks publics
///    (MMLU, HumanEval, GPQA, MT-Bench…).
/// 2. **`static_metadata[llm]`** : cost + latency + success_rate.
/// 3. **`weights_per_task[task]`** : pondérations Pareto à utiliser pour ce
///    type de tâche (différentes selon que coût ou qualité prime).
///
/// Plus un **versioning** : `version: String` permet à l'audit chain de
/// tracer "quelle table a routé quoi" (utile pour S12 A/B test).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskClassScoringTable {
    /// Pour chaque LLM (clé = `LlmRef`), un mapping `TaskClass → capability`.
    /// Si une entrée manque pour `(llm, task)`, le router fallback à `0.5`.
    pub capability: HashMap<String, HashMap<TaskClass, f32>>,

    /// Pour chaque LLM, ses coûts / latences statiques.
    pub static_metadata: HashMap<String, LlmStaticMetadata>,

    /// Pour chaque `TaskClass`, les pondérations Pareto à appliquer.
    /// Si une entrée manque, fallback [`ScoringWeights::default`].
    pub weights_per_task: HashMap<TaskClass, ScoringWeights>,

    /// Version de la table (audit + A/B test).  Format conseillé :
    /// `"s<sprint>-<release>-<year>-<month>"`, e.g. `"s6-r2-2025-11"`.
    pub version: String,
}

impl TaskClassScoringTable {
    /// La table par défaut S6 — calibrée pour Claude Sonnet 4.6 + Gemini 2.5
    /// Flash + GPT-4o.
    ///
    /// Données nov-2025, proxys benchmarks publics + jugement d'expert
    /// (cf. `docs/architecture/EFE_A1_PHASE3_R2_S6_*` §2.2).
    ///
    /// **Provider IDs attendus** (cf. `ClaudeProvider::id()`, etc.) :
    /// - `"claude_claude-sonnet-4-6"`
    /// - `"gemini_gemini-2.5-flash"`
    /// - `"gpt_gpt-4o"`
    #[must_use]
    pub fn default_3llm_table() -> Self {
        let claude = "claude_claude-sonnet-4-6";
        let gemini = "gemini_gemini-2.5-flash";
        let gpt = "gpt_gpt-4o";

        // ── Capability matrix (6 task × 3 LLMs) ────────────────────────
        let mut capability: HashMap<String, HashMap<TaskClass, f32>> = HashMap::new();

        let mut claude_caps = HashMap::new();
        claude_caps.insert(TaskClass::FactualQa, 0.85);
        claude_caps.insert(TaskClass::Analysis, 0.95);
        claude_caps.insert(TaskClass::Code, 0.95);
        claude_caps.insert(TaskClass::Creative, 0.88);
        claude_caps.insert(TaskClass::Reasoning, 0.95);
        claude_caps.insert(TaskClass::Translation, 0.85);
        capability.insert(claude.to_string(), claude_caps);

        let mut gemini_caps = HashMap::new();
        gemini_caps.insert(TaskClass::FactualQa, 0.92);
        gemini_caps.insert(TaskClass::Analysis, 0.78);
        gemini_caps.insert(TaskClass::Code, 0.72);
        gemini_caps.insert(TaskClass::Creative, 0.80);
        gemini_caps.insert(TaskClass::Reasoning, 0.82);
        gemini_caps.insert(TaskClass::Translation, 0.92);
        capability.insert(gemini.to_string(), gemini_caps);

        let mut gpt_caps = HashMap::new();
        gpt_caps.insert(TaskClass::FactualQa, 0.86);
        gpt_caps.insert(TaskClass::Analysis, 0.85);
        gpt_caps.insert(TaskClass::Code, 0.85);
        gpt_caps.insert(TaskClass::Creative, 0.93);
        gpt_caps.insert(TaskClass::Reasoning, 0.86);
        gpt_caps.insert(TaskClass::Translation, 0.85);
        capability.insert(gpt.to_string(), gpt_caps);

        // ── Static metadata (cost in cents / 1M, latency, success) ─────
        let mut static_metadata: HashMap<String, LlmStaticMetadata> = HashMap::new();
        static_metadata.insert(
            claude.to_string(),
            LlmStaticMetadata {
                cost_input_per_million: 300,    // 3 $ / 1M input (legacy cents)
                cost_output_per_million: 1500,  // 15 $ / 1M output (legacy cents)
                // R3.S1 : 3 $ = 300 cents = 300_000 µ¢ EXACT.
                cost_input_per_million_microcents: 300_000,
                cost_output_per_million_microcents: 1_500_000,
                p50_latency_ms: 1200,
                p99_latency_ms: 3000,
                success_rate: 0.99,
            },
        );
        static_metadata.insert(
            gemini.to_string(),
            LlmStaticMetadata {
                // Gemini 2.5 Flash : ridiculement bon marché.
                // Pricing officiel : $0.075/M in, $0.30/M out.
                // R3.S1 (D7) : on stocke EXACT en µ¢/M (7_500/30_000).
                // Le legacy `cost_input_per_million: 8` reste pour compat mais
                // introduit +6.7% d'erreur — utiliser `_microcents_resolved()`.
                cost_input_per_million: 8,      // legacy, arrondi sup.
                cost_output_per_million: 30,    // legacy.
                // R3.S1 EXACT :
                cost_input_per_million_microcents: 7_500,   // = $0.075/M EXACT
                cost_output_per_million_microcents: 30_000, // = $0.30/M EXACT
                p50_latency_ms: 800,
                p99_latency_ms: 2500,
                success_rate: 0.99,
            },
        );
        static_metadata.insert(
            gpt.to_string(),
            LlmStaticMetadata {
                cost_input_per_million: 250,    // 2.50 $ / 1M (legacy)
                cost_output_per_million: 1000,  // 10 $ / 1M (legacy)
                // R3.S1 EXACT : 2.50 $/M = 250_000 µ¢/M, 10 $/M = 1_000_000 µ¢/M.
                cost_input_per_million_microcents: 250_000,
                cost_output_per_million_microcents: 1_000_000,
                p50_latency_ms: 900,
                p99_latency_ms: 3000,
                success_rate: 0.99,
            },
        );

        // ── Weights per task class (calibrés via tests offline §5.3) ──
        //
        // Note : pour `Code` & `Reasoning`, on descend cost_weight à 0.2
        // (très en-dessous de quality_weight) parce que Gemini est 30×
        // moins cher que Claude, et le scoring brut le ferait gagner
        // malgré l'écart de capability. La calibration ci-dessous est
        // celle qui produit les winners attendus (cf. tests).
        let mut weights_per_task: HashMap<TaskClass, ScoringWeights> = HashMap::new();
        weights_per_task.insert(
            TaskClass::FactualQa,
            ScoringWeights::new(1.0, 1.5, 2.0), // cost-driven
        );
        weights_per_task.insert(
            TaskClass::Analysis,
            // R3.S1 re-calibration (D7) : on bump q_weight de 2.0 → 3.0
            // et descend cost_weight de 1.0 → 0.2. En cents, le cost_weight
            // ne discriminait pas (tous arrondis). En µ¢, Gemini est 50×
            // moins cher → cost_weight=1.0 le ferait gagner malgré
            // capability 0.78 vs Claude 0.95. On préserve l'intention R2
            // (Claude pour Analysis — meilleure deep reasoning).
            ScoringWeights::new(3.0, 0.5, 0.2), // quality-dominant, cost soft
        );
        weights_per_task.insert(
            TaskClass::Code,
            ScoringWeights::new(2.5, 0.5, 0.2), // quality-driven, cost soft
        );
        weights_per_task.insert(
            TaskClass::Creative,
            // R3.S1 re-calibration (D7) : avant µ¢, la norm_cost s'écrasait
            // à 1.0 partout (cents arrondis identiques) → cost_weight=1.0
            // n'avait pas d'effet discriminant et le score était quality-only.
            // Post-R3.S1 : µ¢ exposent que Gemini est ~50× moins cher que
            // Claude → un cost_weight=1.0 ferait gagner Gemini malgré sa
            // capability 0.80 (vs GPT 0.93). On bump quality_weight à 3.0
            // et descend cost_weight à 0.1 pour préserver l'intention R2
            // (GPT pour Creative — meilleur sur style/variété 0.93 vs
            // 0.88/0.80) tout en gardant Pareto cost-aware.
            ScoringWeights::new(3.0, 1.0, 0.1), // quality-dominant fort, cost très soft
        );
        weights_per_task.insert(
            TaskClass::Reasoning,
            // R3.S1 re-calibration (D7) : descend cost_weight 0.3 → 0.1.
            // En cents, le score Claude vs Gemini sur Reasoning (0.95 vs 0.82
            // capability × q=3.0 = 2.85 vs 2.46) gardait Claude vainqueur.
            // En µ¢, Gemini 50× moins cher → cost_norm Gemini=0.02 vs
            // Claude 1.0 → différence 0.3 × 0.98 ≈ 0.29 → Claude/Gemini
            // quasi-tied à 2.25 chacun. On baisse cost_weight pour
            // préserver l'intention R2 (Claude pour extended thinking).
            ScoringWeights::new(3.0, 0.3, 0.1), // quality strongly dominant, cost very soft
        );
        weights_per_task.insert(
            TaskClass::Translation,
            ScoringWeights::new(1.0, 1.0, 2.0), // cost-driven
        );

        Self {
            capability,
            static_metadata,
            weights_per_task,
            version: "s6-r2-2025-11".to_string(),
        }
    }

    /// Construit un [`LlmMetadata`] complet pour un couple `(LLM, TaskClass)`.
    ///
    /// Fusionne :
    /// - `static_metadata[llm]` (cost, latency, success_rate)
    /// - `capability[llm][task]` (1 entrée du HashMap → 1 entrée
    ///   `capability_scores: HashMap<String, f32>` en SCREAMING_SNAKE_CASE)
    /// - `circuit_open: false` (runtime concern, S8)
    ///
    /// **Fallback** : si le LLM est absent de la table, retourne un
    /// [`LlmMetadata`] avec valeurs neutres (cost médian, p50=1000ms,
    /// success=0.95, capability=0.5). C'est l'équivalent d'un "unknown LLM"
    /// — le router peut quand même calculer un score.
    #[must_use]
    pub fn build_llm_metadata(&self, llm: &str, task: TaskClass) -> LlmMetadata {
        let static_meta = self.static_metadata.get(llm).copied();
        let cap = self
            .capability
            .get(llm)
            .and_then(|m| m.get(&task).copied())
            .unwrap_or(0.5);

        // Conversion TaskClass → String SCREAMING_SNAKE_CASE (cohérent
        // avec le format consommé par le router cf. router.rs:101-104).
        let class_key = task_class_key(task);
        let mut capability_scores = HashMap::new();
        capability_scores.insert(class_key, cap);

        match static_meta {
            Some(sm) => LlmMetadata {
                cost_input_per_million: sm.cost_input_per_million,
                cost_output_per_million: sm.cost_output_per_million,
                // R3.S1 : propagation des µ¢ depuis le scoring table.
                cost_input_per_million_microcents: sm.cost_input_microcents_resolved(),
                cost_output_per_million_microcents: sm.cost_output_microcents_resolved(),
                p50_latency_ms: sm.p50_latency_ms,
                p99_latency_ms: sm.p99_latency_ms,
                success_rate: sm.success_rate,
                capability_scores,
                circuit_open: false,
            },
            None => LlmMetadata {
                cost_input_per_million: 500,
                cost_output_per_million: 2000,
                // R3.S1 : fallback unknown = 500 cents × 1000 = 500_000 µ¢/M.
                cost_input_per_million_microcents: 500_000,
                cost_output_per_million_microcents: 2_000_000,
                p50_latency_ms: 1000,
                p99_latency_ms: 3000,
                success_rate: 0.95,
                capability_scores,
                circuit_open: false,
            },
        }
    }

    /// Construit le `HashMap<LlmRef, LlmMetadata>` complet pour une liste
    /// de LLMs et une `TaskClass`.
    ///
    /// Pratique pour remplir
    /// [`MissionARequest.llm_metadata`](crate::types::router::RouterInput)
    /// d'un seul appel.
    #[must_use]
    pub fn build_metadata_map(
        &self,
        llms: &[String],
        task: TaskClass,
    ) -> HashMap<String, LlmMetadata> {
        llms.iter()
            .map(|llm| (llm.clone(), self.build_llm_metadata(llm, task)))
            .collect()
    }

    /// Retourne les pondérations Pareto pour une `TaskClass` (avec fallback
    /// [`ScoringWeights::default`] si absente).
    #[must_use]
    pub fn weights_for(&self, task: TaskClass) -> ScoringWeights {
        self.weights_per_task
            .get(&task)
            .copied()
            .unwrap_or_default()
    }
}

/// Convertit `TaskClass` en `String` SCREAMING_SNAKE_CASE en passant par
/// serde — garantit la cohérence avec
/// [`RouterClient::invoke`](crate::clients::router::RouterClient::invoke).
fn task_class_key(task: TaskClass) -> String {
    serde_json::to_value(task)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{task:?}").to_uppercase())
}

// ─────────────────────────────────────────────────────────────────────────
// Tests offline — calibration des winners attendus par TaskClass
// ─────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::router::RouterClient;
    use crate::context::CallContext;
    use crate::types::router::{Budget, RouterInput, TaskDescription};
    use crate::NanoRobotClient;

    /// Helper : instancie le router tuné pour une TaskClass, build l'input
    /// avec les 3 LLMs S6 + budget non-contraignant, invoke → renvoie
    /// `(selected, reason)`.
    async fn run_router_decision(task: TaskClass) -> (String, String) {
        let table = TaskClassScoringTable::default_3llm_table();
        let router = RouterClient::for_task_class(&table, task);
        let llms = vec![
            "claude_claude-sonnet-4-6".to_string(),
            "gemini_gemini-2.5-flash".to_string(),
            "gpt_gpt-4o".to_string(),
        ];
        let input = RouterInput {
            task: TaskDescription {
                prompt_summary: "test query for routing".into(),
                task_class: task,
                estimated_input_tokens: 100,
                expected_output_tokens: 500,
            },
            available_llms: llms.clone(),
            llm_metadata: table.build_metadata_map(&llms, task),
            user_budget: Budget {
                max_cost_cents: 50,
                // R3.S1 : 0 → _resolved() fallback à cents × 1000.
                max_cost_microcents: 0,
                max_latency_ms: 30_000,
                min_quality_score: 0.0,
            },
        };
        let ctx = CallContext::mock();
        let out = router.invoke(input, &ctx).await.expect("router should pick a winner");
        (out.selected, out.reason)
    }

    #[tokio::test]
    async fn factual_qa_picks_gemini() {
        let (selected, _reason) = run_router_decision(TaskClass::FactualQa).await;
        assert_eq!(
            selected, "gemini_gemini-2.5-flash",
            "FactualQa cost-driven → Gemini gagne (8/30 ¢/M vs Claude 300/1500)"
        );
    }

    #[tokio::test]
    async fn analysis_picks_claude() {
        let (selected, _reason) = run_router_decision(TaskClass::Analysis).await;
        assert_eq!(
            selected, "claude_claude-sonnet-4-6",
            "Analysis quality-driven (q=2.0) → Claude gagne (cap 0.95 vs 0.78/0.85)"
        );
    }

    #[tokio::test]
    async fn code_picks_claude() {
        let (selected, _reason) = run_router_decision(TaskClass::Code).await;
        assert_eq!(
            selected, "claude_claude-sonnet-4-6",
            "Code quality-driven (q=2.5, c=0.2) → Claude gagne (HumanEval ~89 %)"
        );
    }

    #[tokio::test]
    async fn creative_picks_gpt() {
        let (selected, _reason) = run_router_decision(TaskClass::Creative).await;
        assert_eq!(
            selected, "gpt_gpt-4o",
            "Creative balanced → GPT-4o gagne (cap 0.93 vs 0.88/0.80)"
        );
    }

    #[tokio::test]
    async fn reasoning_picks_claude() {
        let (selected, _reason) = run_router_decision(TaskClass::Reasoning).await;
        assert_eq!(
            selected, "claude_claude-sonnet-4-6",
            "Reasoning quality strongly dominant (q=3.0) → Claude (extended thinking)"
        );
    }

    #[tokio::test]
    async fn translation_picks_gemini() {
        let (selected, _reason) = run_router_decision(TaskClass::Translation).await;
        assert_eq!(
            selected, "gemini_gemini-2.5-flash",
            "Translation cost-driven (c=2.0) → Gemini (multilingue + cheap)"
        );
    }

    // ── Tests de cohérence structurelle ─────────────────────────────────

    #[test]
    fn default_table_has_3_llms() {
        let table = TaskClassScoringTable::default_3llm_table();
        assert_eq!(table.capability.len(), 3);
        assert_eq!(table.static_metadata.len(), 3);
    }

    #[test]
    fn default_table_has_6_task_classes_per_llm() {
        let table = TaskClassScoringTable::default_3llm_table();
        for (llm, caps) in &table.capability {
            assert_eq!(
                caps.len(),
                6,
                "LLM {llm} should have capability for all 6 TaskClass"
            );
            // Tous les scores ∈ [0, 1].
            for (task, &score) in caps {
                assert!(
                    (0.0..=1.0).contains(&score),
                    "{llm} {task:?} score {score} out of [0,1]"
                );
            }
        }
    }

    #[test]
    fn default_table_has_weights_for_all_task_classes() {
        let table = TaskClassScoringTable::default_3llm_table();
        for task in [
            TaskClass::FactualQa,
            TaskClass::Analysis,
            TaskClass::Code,
            TaskClass::Creative,
            TaskClass::Reasoning,
            TaskClass::Translation,
        ] {
            assert!(
                table.weights_per_task.contains_key(&task),
                "weights for {task:?} must be defined"
            );
        }
    }

    #[test]
    fn version_format_is_sprint_release_yyyy_mm() {
        let table = TaskClassScoringTable::default_3llm_table();
        // Format attendu : "s<num>-r<num>-<yyyy>-<mm>"
        assert!(table.version.starts_with("s6-r2-"));
        assert!(
            table.version.len() >= "s6-r2-2025-11".len(),
            "version {} too short",
            table.version
        );
    }

    #[test]
    fn build_llm_metadata_includes_capability_in_screaming_snake_case() {
        let table = TaskClassScoringTable::default_3llm_table();
        let meta = table.build_llm_metadata("claude_claude-sonnet-4-6", TaskClass::Code);
        // Le router lit `capability_scores["CODE"]` (cf. router.rs:101).
        assert!(
            meta.capability_scores.contains_key("CODE"),
            "key 'CODE' attendu, got {:?}",
            meta.capability_scores.keys().collect::<Vec<_>>()
        );
        assert_eq!(meta.capability_scores.get("CODE"), Some(&0.95));
    }

    #[test]
    fn build_llm_metadata_unknown_llm_returns_neutral_fallback() {
        let table = TaskClassScoringTable::default_3llm_table();
        let meta = table.build_llm_metadata("unknown_llm_xyz", TaskClass::Code);
        assert_eq!(meta.cost_input_per_million, 500);
        assert_eq!(meta.success_rate, 0.95);
        // Capability fallback 0.5.
        assert_eq!(meta.capability_scores.get("CODE"), Some(&0.5));
    }

    #[test]
    fn build_metadata_map_fills_all_llms() {
        let table = TaskClassScoringTable::default_3llm_table();
        let llms = vec![
            "claude_claude-sonnet-4-6".to_string(),
            "gemini_gemini-2.5-flash".to_string(),
            "gpt_gpt-4o".to_string(),
        ];
        let map = table.build_metadata_map(&llms, TaskClass::Reasoning);
        assert_eq!(map.len(), 3);
        // Reasoning : Claude cap 0.95, Gemini cap 0.82, GPT cap 0.86.
        assert_eq!(
            map["claude_claude-sonnet-4-6"]
                .capability_scores
                .get("REASONING"),
            Some(&0.95)
        );
    }

    #[test]
    fn weights_for_falls_back_to_default_for_unknown_task() {
        // Construit une table custom SANS weights pour Reasoning.
        let mut table = TaskClassScoringTable::default_3llm_table();
        table.weights_per_task.remove(&TaskClass::Reasoning);
        let w = table.weights_for(TaskClass::Reasoning);
        assert_eq!(w, ScoringWeights::default());
    }

    #[test]
    fn scoring_weights_serializes_roundtrip() {
        let w = ScoringWeights::new(2.5, 0.5, 0.2);
        let s = serde_json::to_string(&w).unwrap();
        let back: ScoringWeights = serde_json::from_str(&s).unwrap();
        assert_eq!(w, back);
    }

    #[test]
    fn task_class_scoring_table_serializes_roundtrip() {
        let table = TaskClassScoringTable::default_3llm_table();
        let s = serde_json::to_string(&table).unwrap();
        let back: TaskClassScoringTable = serde_json::from_str(&s).unwrap();
        assert_eq!(back.version, table.version);
        assert_eq!(back.capability.len(), table.capability.len());
        assert_eq!(back.static_metadata.len(), table.static_metadata.len());
        assert_eq!(back.weights_per_task.len(), table.weights_per_task.len());
    }

    #[test]
    fn task_class_key_matches_router_format() {
        // Le router consomme les clés sérialisées via serde
        // (cf. router.rs:101-104). Cohérence garantie.
        assert_eq!(task_class_key(TaskClass::FactualQa), "FACTUAL_QA");
        assert_eq!(task_class_key(TaskClass::Code), "CODE");
        assert_eq!(task_class_key(TaskClass::Reasoning), "REASONING");
    }
}
