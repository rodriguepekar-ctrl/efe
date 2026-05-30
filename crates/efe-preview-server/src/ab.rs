// SPDX-License-Identifier: UNLICENSED
//! **P4.S5** — Endpoint A/B Lab : `POST /v1/ab/run`.
//!
//! Lance `n` Missions A à travers un coordinator configuré avec un A/B test
//! (table de contrôle vs table d'expérimentation), agrège les runs en un
//! `SavingsReport`, et renvoie la **comparaison ségrégée** control vs experiment
//! (`ABComparison`).
//!
//! Pour qu'un split de routing soit **visible** en mock, on injecte des
//! métadonnées explicites avec un écart coût/capability garanti (cf. R3.S8 §D6 /
//! `r3s8_live_ab_arm_routing_consistency`) : `mock-claude` cher+capable,
//! `mock-gemini` cheap. Control (quality-weighted) → claude ; experiment
//! (cost-weighted) → gemini.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde::{Deserialize, Serialize};

use efe_coordinator::savings_report::ABComparison;
use efe_coordinator::{
    run_mission_a, ABTestConfig, Coordinator, MetricsRegistry, MissionARequest, MockLlmProvider,
    ProviderRegistry, RunMode, RunRecord, SavingsReport,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata, ScoringWeights, TaskClassScoringTable};

/// Pondérations Pareto (quality / latency / cost) d'une table.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Weights {
    pub quality: f32,
    pub latency: f32,
    pub cost: f32,
}

/// Corps de `POST /v1/ab/run`.
#[derive(Debug, Clone, Deserialize)]
pub struct AbRunDto {
    #[serde(default)]
    pub user_text: Option<String>,
    #[serde(default)]
    pub n: Option<usize>,
    /// Table de contrôle (défaut : quality-weighted `{5,0,0}`).
    #[serde(default)]
    pub control_weights: Option<Weights>,
    /// Table d'expérimentation (défaut : cost-weighted `{1,0,5}`).
    #[serde(default)]
    pub experiment_weights: Option<Weights>,
}

/// Résultat de `POST /v1/ab/run`.
#[derive(Debug, Serialize)]
pub struct AbRunResult {
    /// Nombre de missions effectivement lancées.
    pub n: usize,
    /// Comparaison ségrégée control vs experiment.
    pub comparison: ABComparison,
}

/// Construit une `TaskClassScoringTable` avec des poids FactualQa custom.
fn table_weighted(version: &str, w: Weights) -> TaskClassScoringTable {
    let mut t = TaskClassScoringTable::default_3llm_table();
    t.version = version.to_string();
    t.weights_per_task
        .insert(TaskClass::FactualQa, ScoringWeights::new(w.quality, w.latency, w.cost));
    t
}

/// Métadonnées LLM avec coût/capability explicites (écart garanti pour le split).
fn meta_for(cost_microcents_per_m: u64, capability: f32) -> LlmMetadata {
    let mut caps = HashMap::new();
    caps.insert("FACTUAL_QA".to_string(), capability);
    LlmMetadata {
        cost_input_per_million: 0,
        cost_output_per_million: 0,
        cost_input_per_million_microcents: cost_microcents_per_m,
        cost_output_per_million_microcents: cost_microcents_per_m,
        p50_latency_ms: 100,
        p99_latency_ms: 500,
        success_rate: 1.0,
        capability_scores: caps,
        circuit_open: false,
    }
}

/// Requête A/B : table NON épinglée (`None`) → l'A/B la remplit (précédence
/// request > A/B). Métadonnées explicites : claude cher+capable, gemini cheap.
fn ab_mission_request(user_text: &str) -> MissionARequest {
    let mut metas = HashMap::new();
    metas.insert("mock-claude".to_string(), meta_for(10_000_000, 1.0));
    metas.insert("mock-gemini".to_string(), meta_for(20_000, 0.5));
    metas.insert("mock-gpt".to_string(), meta_for(1_000_000, 0.7));
    MissionARequest {
        user_text: user_text.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![
            "mock-claude".to_string(),
            "mock-gemini".to_string(),
            "mock-gpt".to_string(),
        ],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 0,
            max_cost_microcents: 1_000_000_000,
            max_latency_ms: 30_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(10),
        task_class_scoring_table: None,
        tenant_id: None,
    }
}

/// Coordinator A/B : 3 mock providers (coûts observés distincts) + ab_test + télémétrie.
fn ab_coordinator(cfg: ABTestConfig) -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("mock-claude")
            .with_response("Paris est la capitale de la France.")
            .with_cost_microcents(3_000),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("mock-gemini")
            .with_response("Paris.")
            .with_cost_microcents(37),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("mock-gpt")
            .with_response("La capitale est Paris.")
            .with_cost_microcents(1_500),
    ));
    Coordinator::builder()
        .registry(reg)
        .ab_test(cfg)
        .metrics(Arc::new(MetricsRegistry::new()))
        .build()
        .expect("ab coordinator buildable")
}

/// `POST /v1/ab/run` — lance `n` missions A/B et renvoie la comparaison.
pub async fn ab_run(Json(dto): Json<AbRunDto>) -> Response {
    let n = dto.n.unwrap_or(12).clamp(1, 64);
    let user_text = dto
        .user_text
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Quelle est la capitale de la France ?".to_string());
    let cw = dto.control_weights.unwrap_or(Weights { quality: 5.0, latency: 0.0, cost: 0.0 });
    let ew = dto.experiment_weights.unwrap_or(Weights { quality: 1.0, latency: 0.0, cost: 5.0 });

    let cfg = ABTestConfig {
        control_table: table_weighted("control-quality", cw),
        experiment_table: table_weighted("experiment-cost", ew),
        experiment_traffic_ratio: 0.5,
        tenant_filter: None,
        fixture_filter: None,
    };
    let coord = ab_coordinator(cfg);

    let mut report = SavingsReport::new();
    for _ in 0..n {
        let req = ab_mission_request(&user_text);
        if let Ok(resp) = run_mission_a(&coord, req).await {
            let passed = resp.final_text.to_lowercase().contains("paris");
            report.add_run(RunRecord::new_with_microcents(
                "ab".to_string(),
                RunMode::Full,
                resp.cost_microcents,
                resp.latency_ms,
                resp.chewing_metrics.as_ref().map_or(0, |m| m.llm_tokens_generated),
                &resp.final_text,
                resp.llm_used.clone(),
                "FactualQa".to_string(),
                resp.chewing_metrics.clone(),
                passed,
                resp.failure_mode.clone(),
            ));
        }
    }
    report.aggregate();

    match report.ab_comparison() {
        Some(comparison) => Json(AbRunResult { n, comparison }).into_response(),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "aucun run A/B produit (providers indisponibles ?)",
        )
            .into_response(),
    }
}
