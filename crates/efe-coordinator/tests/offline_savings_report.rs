// SPDX-License-Identifier: UNLICENSED
//! **Sprint S9 R2** — Tests offline du générateur de rapport savings.
//!
//! Ces 4 tests valident la **structure et la cohérence du report** sans
//! dépendre des APIs LLM live (= CI safe, coût $ zéro). Couvrent :
//!
//! 1. Génération end-to-end avec MockLlmProviders simulant les 5 modes
//! 2. Cohérence sémantique : si baseline = full → savings = 0
//! 3. Schéma JSON stable (clés requises présentes après roundtrip)
//! 4. Agrégation correcte sur 5 fixtures simulées

#![cfg(feature = "all-providers")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, Coordinator, FailureMode, MissionARequest, MockLlmProvider,
    ProviderRegistry, RunMode, RunRecord, SavingsReport,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

fn stub_metadata() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

fn build_request_for_llm(llm_id: &str, query: &str) -> MissionARequest {
    let mut metadata = HashMap::new();
    metadata.insert(llm_id.to_string(), stub_metadata());
    MissionARequest {
        user_text: query.to_string(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![llm_id.to_string()],
        llm_metadata: metadata,
        budget: Budget {
            max_cost_cents: 100,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
    }
}

/// Helper : run une mission avec un MockLlmProvider configuré, capture
/// le record. Coût + latence simulés.
async fn simulate_run(
    fixture_name: &str,
    mode: RunMode,
    cost_cents: u32,
    response_text: &str,
    expected_keyword: &str,
) -> RunRecord {
    let llm_id = match mode {
        RunMode::Baseline | RunMode::S4Chewing | RunMode::Full => "claude_stub",
        RunMode::S6Routing => "gemini_stub",
        RunMode::Speculative => "gemini_stub",
    };
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new(llm_id)
            .with_response(response_text)
            .with_cost_cents(cost_cents),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let req = build_request_for_llm(llm_id, "test query");
    let resp = run_mission_a(&coord, req).await.unwrap();

    let passed = resp
        .final_text
        .to_lowercase()
        .contains(&expected_keyword.to_lowercase());

    RunRecord::new(
        fixture_name.to_string(),
        mode,
        resp.cost_cents,
        resp.latency_ms,
        resp.chewing_metrics
            .as_ref()
            .map(|m| m.llm_tokens_generated)
            .unwrap_or(0),
        &resp.final_text,
        resp.llm_used,
        "FactualQa".to_string(),
        resp.chewing_metrics,
        passed,
        resp.failure_mode,
    )
}

/// **T1** : Génération end-to-end avec mocks → markdown valide structuré.
#[tokio::test]
async fn offline_report_with_mock_providers_generates_valid_markdown() {
    let mut report = SavingsReport::with_version("s9-r2-test");
    // Simule 5 fixtures × 4 modes avec coûts décroissants (baseline > S4 > S6 > full).
    let fixtures = [
        "01_factual_simple",
        "03_factual_compressible",
        "05_code_gen_rust",
        "06_creative_writing",
        "07_translation_fr_en",
    ];
    for fx in &fixtures {
        report.add_run(simulate_run(fx, RunMode::Baseline, 5, "Paris is the answer", "paris").await);
        report.add_run(simulate_run(fx, RunMode::S4Chewing, 4, "Paris compressed", "paris").await);
        report.add_run(simulate_run(fx, RunMode::S6Routing, 2, "Paris routed", "paris").await);
        report.add_run(simulate_run(fx, RunMode::Full, 1, "Paris full", "paris").await);
    }
    report.aggregate();
    let md = report.to_markdown();

    // Structure attendue.
    assert!(md.contains("# EFE Phase 3 R2 — Chewing Savings Report"));
    assert!(md.contains("## Résumé exécutif"));
    assert!(md.contains("## Détail par mode"));
    assert!(md.contains("## Verdict"));
    assert!(md.contains("## Reproductibilité"));
    // Une section par mode (descriptions).
    assert!(md.contains("Baseline (Claude direct"));
    assert!(md.contains("S4 only"));
    assert!(md.contains("S6 only"));
    assert!(md.contains("Full (S4 + S6"));
    // Tableau des fixtures dans Baseline.
    assert!(md.contains("01_factual_simple"));
    assert!(md.contains("07_translation_fr_en"));
    // Commande de relance.
    assert!(md.contains("EFE_LIVE_LLM=1"));
    // Calcul savings cohérent : baseline 25¢, full 5¢ → 80 % savings.
    assert_eq!(report.baseline_total_cost_cents, 25);
    assert_eq!(report.full_total_cost_cents, 5);
    assert_eq!(report.savings_cents, 20);
}

/// **T2** : Cohérence sémantique — si baseline = full, savings = 0.
#[tokio::test]
async fn offline_report_savings_ratio_zero_when_baseline_eq_full() {
    let mut report = SavingsReport::with_version("s9-r2-test");
    for fx in &["a", "b", "c"] {
        // baseline et full identiques : 5¢ chacun.
        report.add_run(simulate_run(fx, RunMode::Baseline, 5, "ok", "ok").await);
        report.add_run(simulate_run(fx, RunMode::Full, 5, "ok", "ok").await);
    }
    report.aggregate();
    assert_eq!(report.baseline_total_cost_cents, 15);
    assert_eq!(report.full_total_cost_cents, 15);
    assert_eq!(report.savings_cents, 0);
    assert_eq!(report.savings_ratio, 0.0);
}

/// **T3** : Schéma JSON stable — roundtrip + clés requises présentes.
#[tokio::test]
async fn offline_report_json_schema_stable() {
    let mut report = SavingsReport::with_version("s9-r2-test");
    report.add_run(simulate_run("01", RunMode::Baseline, 5, "Paris", "paris").await);
    report.add_run(simulate_run("01", RunMode::Full, 1, "Paris", "paris").await);
    report.aggregate();
    let json = report.to_json().expect("to_json must serialize");

    // Roundtrip exact.
    let back: SavingsReport = serde_json::from_str(&json).expect("roundtrip");
    assert_eq!(back.runs.len(), 2);
    assert_eq!(back.baseline_total_cost_cents, 5);
    assert_eq!(back.full_total_cost_cents, 1);

    // Clés requises pour les consumers (CI, dashboards futurs).
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    for key in [
        "runs",
        "summaries",
        "baseline_total_cost_cents",
        "full_total_cost_cents",
        "savings_cents",
        "savings_ratio",
        "generated_at",
        "version",
    ] {
        assert!(
            value.get(key).is_some(),
            "JSON schema doit contenir la clé '{key}'"
        );
    }
}

/// **T4** : Agrégation correcte sur 5 fixtures × 5 modes.
/// Vérifie les compteurs `summaries[mode].n_runs == 5`.
#[tokio::test]
async fn offline_report_aggregates_5_fixtures_correctly() {
    let mut report = SavingsReport::with_version("s9-r2-test");
    let fixtures = [
        "01_factual_simple",
        "03_factual_compressible",
        "05_code_gen_rust",
        "06_creative_writing",
        "07_translation_fr_en",
    ];
    for fx in &fixtures {
        for mode in &[
            RunMode::Baseline,
            RunMode::S4Chewing,
            RunMode::S6Routing,
            RunMode::Full,
        ] {
            report.add_run(simulate_run(fx, *mode, 2, "Paris", "paris").await);
        }
    }
    // Speculative uniquement sur 2 fixtures FactualQa.
    for fx in &["01_factual_simple", "03_factual_compressible"] {
        report.add_run(simulate_run(fx, RunMode::Speculative, 1, "Paris", "paris").await);
    }
    report.aggregate();

    assert_eq!(report.runs.len(), 22, "5×4 + 2 speculative = 22");
    assert_eq!(report.summaries.get(&RunMode::Baseline).unwrap().n_runs, 5);
    assert_eq!(report.summaries.get(&RunMode::S4Chewing).unwrap().n_runs, 5);
    assert_eq!(report.summaries.get(&RunMode::S6Routing).unwrap().n_runs, 5);
    assert_eq!(report.summaries.get(&RunMode::Full).unwrap().n_runs, 5);
    assert_eq!(
        report.summaries.get(&RunMode::Speculative).unwrap().n_runs,
        2,
        "Speculative limité aux 2 FactualQa fixtures"
    );

    // Quality pass rate = 100 % (toutes les réponses contiennent "paris").
    for summary in report.summaries.values() {
        if summary.n_runs > 0 {
            assert_eq!(summary.quality_pass_rate, 1.0);
        }
    }
}

/// Suppress dead_code warning sur FailureMode (utilisé via RunRecord).
#[allow(dead_code)]
fn _failure_mode_witness() -> Option<FailureMode> {
    None
}
