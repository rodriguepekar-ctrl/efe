// SPDX-License-Identifier: UNLICENSED
//! Client du robot **Routeur** (bge-m3 + heuristiques Pareto).
//!
//! Sélectionne le LLM cible parmi N candidats selon coût/latence/qualité.

use std::time::Instant;

use async_trait::async_trait;

use super::llama_server::RobotHealth;
use super::{NanoRobotClient, RunMode};
use crate::context::{blake3_canonical_json, AuditEntry, CallContext, SuccessRecord};
use crate::error::{RobotError, RobotResult};
use crate::types::router::{LlmMetadata, LlmRef, RouterInput, RouterOutput};

/// Client pour le robot Routeur.
#[derive(Debug, Default, Clone, Copy)]
pub struct RouterClient {
    /// Coefficient pondérant la qualité dans le scoring (défaut 1.0).
    pub quality_weight: f32,
    /// Coefficient pondérant la latence (défaut 1.0).
    pub latency_weight: f32,
    /// Coefficient pondérant le coût (défaut 1.0).
    pub cost_weight: f32,
}

impl RouterClient {
    /// Construit avec poids par défaut (équilibre triangulaire).
    #[must_use]
    pub fn new() -> Self {
        Self {
            quality_weight: 1.0,
            latency_weight: 1.0,
            cost_weight: 1.0,
        }
    }

    /// Construit avec poids custom (utile pour tenants où le coût prime
    /// largement, par ex.).
    #[must_use]
    pub fn with_weights(quality: f32, latency: f32, cost: f32) -> Self {
        Self {
            quality_weight: quality,
            latency_weight: latency,
            cost_weight: cost,
        }
    }

    /// **Sprint S6 R2** — Construit avec les pondérations pré-calibrées pour
    /// une [`TaskClass`](crate::types::router::TaskClass) via la table
    /// statique [`TaskClassScoringTable`](crate::types::scoring::TaskClassScoringTable).
    ///
    /// Pratique pour les workflows qui connaissent juste la `task_class` :
    ///
    /// ```ignore
    /// let table = TaskClassScoringTable::default_3llm_table();
    /// let router = RouterClient::for_task_class(&table, TaskClass::Code);
    /// // → quality=2.5, latency=0.5, cost=0.2 (Code privilégie la qualité)
    /// ```
    #[must_use]
    pub fn for_task_class(
        table: &crate::types::scoring::TaskClassScoringTable,
        task: crate::types::router::TaskClass,
    ) -> Self {
        let w = table.weights_for(task);
        Self {
            quality_weight: w.quality_weight,
            latency_weight: w.latency_weight,
            cost_weight: w.cost_weight,
        }
    }
}

#[async_trait]
impl NanoRobotClient for RouterClient {
    type Input = RouterInput;
    type Output = RouterOutput;

    fn id(&self) -> &'static str {
        crate::robot_ids::ROUTER
    }
    fn schema_version(&self) -> &'static str {
        "1.0.0"
    }
    fn run_mode(&self) -> RunMode {
        RunMode::A
    }

    async fn invoke(
        &self,
        input: Self::Input,
        ctx: &CallContext,
    ) -> RobotResult<Self::Output> {
        use validator::Validate as _;
        let started = Instant::now();
        input.validate().map_err(|e| RobotError::SemanticViolation {
            robot: self.id(),
            issues: format!("input invalid: {e}"),
        })?;
        let input_hash = blake3_canonical_json(&input);

        // 1. Filtre les LLM disqualifiés (circuit ouvert, hors budget).
        let mut candidates: Vec<(LlmRef, LlmMetadata, f32)> = Vec::new();
        for llm in &input.available_llms {
            let meta = input.llm_metadata.get(llm).ok_or_else(|| {
                RobotError::SemanticViolation {
                    robot: self.id(),
                    issues: format!("missing metadata for {llm}"),
                }
            })?;
            if meta.circuit_open {
                continue;
            }
            // R3.S1 : filtrage budgétaire en microcents (précision sub-cent).
            // Le helper `_resolved()` reconstruit µ¢ depuis cents si l'appelant
            // n'a pas peuplé `max_cost_microcents` (rétrocompat).
            let cost_mc = estimate_cost_microcents(meta, &input.task);
            let max_cost_mc = input.user_budget.max_cost_microcents_resolved();
            if cost_mc > max_cost_mc {
                continue;
            }
            if meta.p50_latency_ms > input.user_budget.max_latency_ms {
                continue;
            }
            // Score qualité prédite = capability_score * success_rate.
            // Utilise serde pour avoir la repr SCREAMING_SNAKE_CASE
            // cohérente avec celle utilisée côté caller.
            let class_key = serde_json::to_value(input.task.task_class)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let cap_score = meta
                .capability_scores
                .get(&class_key)
                .copied()
                .unwrap_or(0.5);
            let predicted_quality = cap_score * meta.success_rate;
            if predicted_quality < input.user_budget.min_quality_score {
                continue;
            }
            candidates.push((llm.clone(), meta.clone(), predicted_quality));
        }

        if candidates.is_empty() {
            return Err(RobotError::SemanticViolation {
                robot: self.id(),
                issues: "no LLM satisfies budget + capability constraints".into(),
            });
        }

        // 2. Tri Pareto-pondéré : score = quality_weight*q - latency_weight*norm(lat) - cost_weight*norm(cost).
        //
        // R3.S1 (D7) : la normalisation `cost` se fait désormais en microcents.
        // Sur fixtures courtes en cents, tous les coûts étaient arrondis à 1 ¢
        // → `norm(cost) = 1.0` partout → la composante cost ne discriminait
        // PLUS dans le score Pareto. En µ¢, Claude (45 µ¢) vs Gemini (1 µ¢)
        // sont séparés ×45 → le `cost_weight` retrouve son pouvoir
        // discriminant Pareto réel.
        let max_cost_mc = candidates
            .iter()
            .map(|(_, m, _)| estimate_cost_microcents(m, &input.task))
            .max()
            .unwrap_or(1)
            .max(1);
        let max_lat = candidates
            .iter()
            .map(|(_, m, _)| m.p50_latency_ms)
            .max()
            .unwrap_or(1)
            .max(1);

        let scored: Vec<(LlmRef, LlmMetadata, f32)> = candidates
            .into_iter()
            .map(|(llm, meta, quality)| {
                let cost = estimate_cost_microcents(&meta, &input.task) as f32
                    / max_cost_mc as f32;
                let lat = meta.p50_latency_ms as f32 / max_lat as f32;
                let score = self.quality_weight * quality
                    - self.latency_weight * lat
                    - self.cost_weight * cost;
                (llm, meta, score)
            })
            .collect();

        // 3. Top-N tri descendant par score.
        let mut sorted = scored;
        sorted.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

        let (selected, sel_meta, _) = sorted.first().cloned().expect("non-empty checked above");
        let fallback_chain: Vec<LlmRef> = sorted.iter().skip(1).map(|(l, _, _)| l.clone()).collect();
        let est_cost = estimate_cost_cents(&sel_meta, &input.task);
        // R3.S1 : également exposer l'estimation en µ¢ (précis).
        let est_cost_mc = estimate_cost_microcents(&sel_meta, &input.task);
        let est_lat = sel_meta.p50_latency_ms;
        let reason = format!(
            "selected {selected} (p50={est_lat}ms, est_cost={est_cost}c [{est_cost_mc}µ¢], fallback={fallback_chain:?})"
        );

        let output = RouterOutput {
            selected,
            fallback_chain,
            estimated_cost_cents: est_cost,
            estimated_cost_microcents: est_cost_mc,
            estimated_latency_ms: est_lat,
            reason,
        };
        let output_hash = blake3_canonical_json(&output);
        let latency_ms = started.elapsed().as_millis() as u64;
        let entry = AuditEntry::success(
            ctx,
            SuccessRecord {
                robot_id: self.id(),
                schema_version: self.schema_version(),
                input_hash,
                output_hash,
                tokens_prompt: 0,
                tokens_completion: 0,
                latency_ms,
            },
        );
        ctx.audit
            .record(entry)
            .await
            .map_err(|e| RobotError::AuditFailure {
                robot: self.id(),
                reason: e.to_string(),
            })?;

        Ok(output)
    }

    async fn health(&self) -> RobotResult<RobotHealth> {
        Ok(RobotHealth::Up)
    }
}

/// Coût estimé d'une tâche en cents (input × $/M + output × $/M).
///
/// **LEGACY** : utiliser `estimate_cost_microcents()` (R3.S1) pour la précision
/// sub-cent.
#[must_use]
pub fn estimate_cost_cents(
    meta: &LlmMetadata,
    task: &crate::types::router::TaskDescription,
) -> u32 {
    let input_cost = (task.estimated_input_tokens as u64 * meta.cost_input_per_million as u64)
        / 1_000_000;
    let output_cost = (task.expected_output_tokens as u64 * meta.cost_output_per_million as u64)
        / 1_000_000;
    (input_cost + output_cost) as u32
}

/// **R3.S1** — Coût estimé d'une tâche en **microcents** (1 µ¢ = 1/1000 cent).
///
/// Utilise `LlmMetadata::cost_{input,output}_microcents_resolved()` pour gérer
/// le cas legacy (fields µ¢ à 0 → fallback `cents × 1000`).
///
/// Saturating math pour éviter tout overflow sur tokens gargantuesques.
#[must_use]
pub fn estimate_cost_microcents(
    meta: &LlmMetadata,
    task: &crate::types::router::TaskDescription,
) -> u64 {
    let in_rate = meta.cost_input_microcents_resolved();
    let out_rate = meta.cost_output_microcents_resolved();
    let in_mc = (task.estimated_input_tokens as u64).saturating_mul(in_rate) / 1_000_000;
    let out_mc = (task.expected_output_tokens as u64).saturating_mul(out_rate) / 1_000_000;
    in_mc.saturating_add(out_mc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::router::{Budget, TaskClass, TaskDescription};
    use std::collections::HashMap;

    fn make_metadata(
        cost_in: u32,
        cost_out: u32,
        p50: u32,
        success: f32,
        cap: f32,
        circuit_open: bool,
    ) -> LlmMetadata {
        let mut caps = HashMap::new();
        caps.insert("FACTUAL_QA".into(), cap);
        LlmMetadata {
            cost_input_per_million: cost_in,
            cost_output_per_million: cost_out,
            // R3.S1 : dérivé cents × 1000.
            cost_input_per_million_microcents: (cost_in as u64) * 1000,
            cost_output_per_million_microcents: (cost_out as u64) * 1000,
            p50_latency_ms: p50,
            p99_latency_ms: p50 * 3,
            success_rate: success,
            capability_scores: caps,
            circuit_open,
        }
    }

    #[tokio::test]
    async fn router_picks_cheapest_if_quality_equal() {
        let client = RouterClient::with_weights(1.0, 0.5, 2.0); // privilégie le coût
        let ctx = CallContext::mock();
        let mut meta = HashMap::new();
        meta.insert(
            "claude".into(),
            make_metadata(15_00, 75_00, 900, 0.99, 0.9, false),
        );
        meta.insert(
            "gpt4".into(),
            make_metadata(10_00, 30_00, 1200, 0.99, 0.9, false),
        );
        let input = RouterInput {
            task: TaskDescription {
                prompt_summary: "factual question".into(),
                task_class: TaskClass::FactualQa,
                estimated_input_tokens: 1000,
                expected_output_tokens: 500,
            },
            available_llms: vec!["claude".into(), "gpt4".into()],
            llm_metadata: meta,
            user_budget: Budget {
                max_cost_cents: 10_000,
                // R3.S1 : 0 → _resolved() fallback à cents × 1000.
                max_cost_microcents: 0,
                max_latency_ms: 5000,
                min_quality_score: 0.5,
            },
        };
        let out = client.invoke(input, &ctx).await.unwrap();
        // Avec poids coût élevé, gpt4 doit gagner (moins cher).
        assert_eq!(out.selected, "gpt4");
    }

    #[tokio::test]
    async fn router_rejects_circuit_open() {
        let client = RouterClient::new();
        let ctx = CallContext::mock();
        let mut meta = HashMap::new();
        meta.insert(
            "claude".into(),
            make_metadata(15_00, 75_00, 900, 0.99, 0.9, true), // circuit OUVERT
        );
        let input = RouterInput {
            task: TaskDescription {
                prompt_summary: "x".into(),
                task_class: TaskClass::FactualQa,
                estimated_input_tokens: 100,
                expected_output_tokens: 50,
            },
            available_llms: vec!["claude".into()],
            llm_metadata: meta,
            user_budget: Budget {
                max_cost_cents: 10_000,
                // R3.S1 : 0 → _resolved() fallback à cents × 1000.
                max_cost_microcents: 0,
                max_latency_ms: 5000,
                min_quality_score: 0.5,
            },
        };
        let result = client.invoke(input, &ctx).await;
        assert!(matches!(result, Err(RobotError::SemanticViolation { .. })));
    }

    #[test]
    fn estimate_cost_works() {
        let meta = make_metadata(15_00, 75_00, 900, 1.0, 1.0, false);
        let task = TaskDescription {
            prompt_summary: "x".into(),
            task_class: TaskClass::FactualQa,
            estimated_input_tokens: 1_000_000,
            expected_output_tokens: 1_000_000,
        };
        let c = estimate_cost_cents(&meta, &task);
        assert_eq!(c, 9_000); // 1500 + 7500 = 9000 cents (= $90)
    }

    // ============================================================
    // R3.S1 — Tests Router Pareto microcents (Gap B)
    // ============================================================

    #[test]
    fn r3s1_estimate_cost_microcents_exact_sub_cent() {
        // Gemini Flash : 7_500 µ¢/M input, 30_000 µ¢/M output.
        let mut meta = make_metadata(8, 30, 800, 0.99, 0.9, false);
        // Override µ¢ pour valeurs EXACTES (vs legacy arrondi cents).
        meta.cost_input_per_million_microcents = 7_500;
        meta.cost_output_per_million_microcents = 30_000;
        let task = TaskDescription {
            prompt_summary: "x".into(),
            task_class: TaskClass::FactualQa,
            estimated_input_tokens: 1_000,
            expected_output_tokens: 1_000,
        };
        // (1000 × 7_500) / 1M + (1000 × 30_000) / 1M = 7 + 30 = 37 µ¢ EXACT.
        let mc = estimate_cost_microcents(&meta, &task);
        assert_eq!(mc, 37);
    }

    #[test]
    fn r3s1_estimate_cost_microcents_legacy_meta_falls_back_to_cents() {
        // Métadonnées pré-R3 (µ¢ = 0) → fallback cents × 1000.
        let meta = make_metadata(8, 30, 800, 0.99, 0.9, false);
        // make_metadata met µ¢ = cents × 1000 (cf. patch R3.S1 test helper).
        let task = TaskDescription {
            prompt_summary: "x".into(),
            task_class: TaskClass::FactualQa,
            estimated_input_tokens: 1_000,
            expected_output_tokens: 1_000,
        };
        let mc = estimate_cost_microcents(&meta, &task);
        // make_metadata avec cost_in=8, cost_out=30 → µ¢ peuplés à 8_000/30_000.
        // (1000 × 8_000) / 1M + (1000 × 30_000) / 1M = 8 + 30 = 38 µ¢.
        // (Légèrement plus haut que la valeur EXACT 37 µ¢ — c'est le coût du
        // legacy arrondi en cents propagé en µ¢ via cents × 1000.)
        assert_eq!(mc, 38);
    }

    #[tokio::test]
    async fn r3s1_router_pareto_discriminates_in_microcents_when_cents_uniform() {
        // Scénario : 2 LLMs avec coûts µ¢ très différents (×45) mais
        // qui s'arrondissent à 1 ¢ tous deux en cents.
        // → en R2, le score Pareto sur cents donnait le même cost normalisé.
        // → en R3.S1, le score µ¢ discrimine ×45.

        let client = RouterClient::with_weights(0.5, 0.0, 5.0); // cost-driven extrême
        let ctx = CallContext::mock();
        let mut meta = HashMap::new();

        // LLM "claude" : 45 µ¢/M input, 45 µ¢/M output (= 0.045 cent total
        // sur 1M+1M tokens) — arrondi à 1 ¢ en legacy.
        let mut claude_meta = make_metadata(0, 0, 800, 0.99, 0.9, false);
        claude_meta.cost_input_per_million_microcents = 45_000;
        claude_meta.cost_output_per_million_microcents = 45_000;
        meta.insert("claude_cheap".into(), claude_meta);

        // LLM "gemini" : 1 µ¢/M input, 1 µ¢/M output (45× moins cher).
        let mut gemini_meta = make_metadata(0, 0, 800, 0.99, 0.9, false);
        gemini_meta.cost_input_per_million_microcents = 1_000;
        gemini_meta.cost_output_per_million_microcents = 1_000;
        meta.insert("gemini_cheaper".into(), gemini_meta);

        let input = RouterInput {
            task: TaskDescription {
                prompt_summary: "x".into(),
                task_class: TaskClass::FactualQa,
                estimated_input_tokens: 1_000_000,
                expected_output_tokens: 1_000_000,
            },
            available_llms: vec!["claude_cheap".into(), "gemini_cheaper".into()],
            llm_metadata: meta,
            user_budget: Budget {
                max_cost_cents: 100,
                max_cost_microcents: 0, // fallback cents × 1000
                max_latency_ms: 5000,
                min_quality_score: 0.5,
            },
        };
        let out = client.invoke(input, &ctx).await.unwrap();
        // R3.S1 : Gemini gagne grâce au scoring µ¢ (×45 moins cher).
        assert_eq!(
            out.selected, "gemini_cheaper",
            "Le router R3.S1 doit choisir le LLM µ¢ le moins cher (×45) ; \
             en R2 cents, la composante cost s'écrasait à norm=1.0 partout."
        );
        // Vérification estimated_cost_microcents propagé.
        assert!(
            out.estimated_cost_microcents > 0,
            "estimated_cost_microcents doit être > 0"
        );
    }

    #[tokio::test]
    async fn r3s1_router_budget_filter_uses_microcents_resolved() {
        // Budget tight en µ¢ : 500 µ¢ max. Un LLM à 1000 µ¢ doit être filtré.
        let client = RouterClient::with_weights(1.0, 1.0, 1.0);
        let ctx = CallContext::mock();
        let mut meta = HashMap::new();
        let mut expensive = make_metadata(0, 0, 800, 0.99, 0.9, false);
        // Sur 1M+1M tokens : 1_000_000 × 1_000 / 1M = 1_000 µ¢ d'input,
        // 1_000_000 × 1_000 / 1M = 1_000 µ¢ output → 2_000 µ¢ total > 500.
        expensive.cost_input_per_million_microcents = 1_000;
        expensive.cost_output_per_million_microcents = 1_000;
        meta.insert("expensive".into(), expensive);

        let input = RouterInput {
            task: TaskDescription {
                prompt_summary: "x".into(),
                task_class: TaskClass::FactualQa,
                estimated_input_tokens: 1_000_000,
                expected_output_tokens: 1_000_000,
            },
            available_llms: vec!["expensive".into()],
            llm_metadata: meta,
            user_budget: Budget {
                max_cost_cents: 0,         // pas de budget cents
                max_cost_microcents: 500,  // budget tight 500 µ¢ (0.5 ¢)
                max_latency_ms: 5000,
                min_quality_score: 0.5,
            },
        };
        // Le LLM coûte 2_000 µ¢ > 500 µ¢ budget → filtré → no candidate.
        let result = client.invoke(input, &ctx).await;
        assert!(
            matches!(result, Err(RobotError::SemanticViolation { .. })),
            "Le filtrage budgétaire µ¢ doit rejeter le LLM trop cher"
        );
    }
}
