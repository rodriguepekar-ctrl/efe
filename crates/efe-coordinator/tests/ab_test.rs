// SPDX-License-Identifier: UNLICENSED
//! **R3.S8 — Tests d'intégration A/B test de tables de scoring.**
//!
//! Valident :
//! - le routage par bras (la table du bras est réellement utilisée) + l'arm
//!   tracé dans `chewing_metrics.ab_arm` ;
//! - la précédence (`request > A/B`) : une table épinglée skip l'A/B ;
//! - la non-régression (`ab_test: None` ⇒ pas d'arm) ;
//! - la ségrégation `SavingsReport` control vs experiment.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{ABArm, ABTestConfig, ChewingMetrics, Coordinator, MissionARequest, MockLlmProvider, ProviderRegistry, RunFacts, RunMode, RunRecord, SavingsReport, run_mission_a};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata, ScoringWeights, TaskClassScoringTable};

/// `LlmMetadata` paramétrable : capability FactualQa + coût µ¢/M.
fn meta(cost_microcents_per_m: u64, capability: f32) -> LlmMetadata {
    let mut capability_scores = HashMap::new();
    capability_scores.insert("FACTUAL_QA".to_string(), capability);
    LlmMetadata {
        cost_input_per_million: 0,
        cost_output_per_million: 0,
        cost_input_per_million_microcents: cost_microcents_per_m,
        cost_output_per_million_microcents: cost_microcents_per_m,
        p50_latency_ms: 100,
        p99_latency_ms: 500,
        success_rate: 1.0,
        capability_scores,
        circuit_open: false,
    }
}

/// Table minimale dont seuls les poids FactualQa importent (le router lit la
/// capability/coût depuis `llm_metadata`, et les poids depuis la table).
fn table_weighted(version: &str, q: f32, l: f32, c: f32) -> TaskClassScoringTable {
    let mut weights_per_task = HashMap::new();
    weights_per_task.insert(TaskClass::FactualQa, ScoringWeights::new(q, l, c));
    TaskClassScoringTable {
        capability: HashMap::new(),
        static_metadata: HashMap::new(),
        weights_per_task,
        version: version.to_string(),
    }
}

fn budget() -> Budget {
    Budget {
        max_cost_cents: 0,
        max_cost_microcents: 1_000_000, // large
        max_latency_ms: 10_000,
        min_quality_score: 0.0,
    }
}

/// Requête FactualQa sur [premium, cheap], sans table épinglée (l'A/B la
/// remplit).
fn req(pinned: Option<TaskClassScoringTable>) -> MissionARequest {
    let mut metas = HashMap::new();
    // premium : capability haute, coût élevé. cheap : capability basse, coût bas.
    metas.insert("premium".to_string(), meta(10_000_000, 1.0));
    metas.insert("cheap".to_string(), meta(20_000, 0.3));
    MissionARequest {
        user_text: "Quelle est la capitale de la France ?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["premium".into(), "cheap".into()],
        llm_metadata: metas,
        budget: budget(),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: pinned.map(Arc::new),
        tenant_id: None,
    }
}

fn coord_with_ab(ratio: f32) -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("premium").with_response("from-premium")));
    reg.register(Arc::new(MockLlmProvider::new("cheap").with_response("from-cheap")));
    let cfg = ABTestConfig {
        // control : quality-weighted → préfère premium (capability 1.0).
        control_table: table_weighted("ctrl-v1", 5.0, 0.0, 0.0),
        // experiment : cost-weighted → préfère cheap (coût bas).
        experiment_table: table_weighted("exp-v2", 1.0, 0.0, 5.0),
        experiment_traffic_ratio: ratio,
        tenant_filter: None,
        fixture_filter: None,
    };
    Coordinator::builder().registry(reg).ab_test(cfg).build().unwrap()
}

// =====================================================================
// #8a — ratio 1.0 → Experiment (cost-weighted) → cheap, arm tracé
// =====================================================================

#[tokio::test]
async fn coordinator_ab_experiment_routes_to_cheap() {
    let coord = coord_with_ab(1.0); // toujours Experiment
    let resp = run_mission_a(&coord, req(None)).await.unwrap();
    let cm = resp.chewing_metrics.as_ref().unwrap();
    assert_eq!(cm.ab_arm, Some(ABArm::Experiment), "ratio 1.0 → Experiment");
    assert_eq!(
        cm.ab_table_version.as_deref(),
        Some("exp-v2"),
        "version de table du bras tracée (chemin version_of en prod)"
    );
    assert_eq!(resp.llm_used, "cheap", "experiment cost-weighted → cheap");
    assert_eq!(resp.final_text, "from-cheap");
}

// =====================================================================
// #8b — ratio 0.0 → Control (quality-weighted) → premium, arm tracé
// =====================================================================

#[tokio::test]
async fn coordinator_ab_control_routes_to_premium() {
    let coord = coord_with_ab(0.0); // toujours Control
    let resp = run_mission_a(&coord, req(None)).await.unwrap();
    let cm = resp.chewing_metrics.as_ref().unwrap();
    assert_eq!(cm.ab_arm, Some(ABArm::Control), "ratio 0.0 → Control");
    assert_eq!(cm.ab_table_version.as_deref(), Some("ctrl-v1"));
    assert_eq!(resp.llm_used, "premium", "control quality-weighted → premium");
    assert_eq!(resp.final_text, "from-premium");
}

// =====================================================================
// #9 — table épinglée par la requête → A/B skippé (précédence)
// =====================================================================

#[tokio::test]
async fn request_pinned_table_skips_ab() {
    let coord = coord_with_ab(1.0); // A/B actif (ratio 1.0)
    // La requête épingle une table quality-weighted → doit primer sur l'A/B.
    let pinned = table_weighted("pinned", 5.0, 0.0, 0.0);
    let resp = run_mission_a(&coord, req(Some(pinned))).await.unwrap();
    let cm = resp.chewing_metrics.as_ref().unwrap();
    assert_eq!(cm.ab_arm, None, "table épinglée → pas d'A/B (request > A/B)");
    assert_eq!(cm.ab_table_version, None, "pas d'A/B → pas de version tracée");
    assert_eq!(resp.llm_used, "premium", "la table épinglée (quality) route premium");
}

// =====================================================================
// Non-régression — pas d'A/B configuré → pas d'arm
// =====================================================================

#[tokio::test]
async fn coordinator_without_ab_test_no_arm() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("premium").with_response("x")));
    reg.register(Arc::new(MockLlmProvider::new("cheap").with_response("y")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let resp = run_mission_a(&coord, req(None)).await.unwrap();
    assert_eq!(
        resp.chewing_metrics.as_ref().unwrap().ab_arm,
        None,
        "pas d'A/B configuré → ab_arm None (backward-compat)"
    );
}

// =====================================================================
// #7 — SavingsReport ségrège control vs experiment
// =====================================================================

#[test]
fn ab_test_savings_report_segregates_control_vs_experiment() {
    let arm_record = |arm: ABArm, cost_mc: u64, passed: bool, latency: u64| {
        let version = match arm {
            ABArm::Control => "ctrl-v1",
            ABArm::Experiment => "exp-v2",
        };
        let cm = ChewingMetrics {
            ab_arm: Some(arm),
            ab_table_version: Some(version.to_string()),
            ..ChewingMetrics::default()
        };
        RunRecord::new_with_microcents(
            cost_mc,
            RunFacts {
                fixture_name: "fixture".into(),
                mode: RunMode::Full,
                latency_ms: latency,
                tokens_output: 10,
                final_text: "Paris",
                llm_used: "llm".into(),
                task_class: "FactualQa".into(),
                chewing_metrics: Some(cm),
                passed_quality: passed,
                failure_mode: efe_coordinator::FailureMode::None,
            },
        )
    };

    let mut report = SavingsReport::new();
    // Control : 2 runs, coût 1000 µ¢, 1/2 qualité.
    report.add_run(arm_record(ABArm::Control, 1_000, true, 100));
    report.add_run(arm_record(ABArm::Control, 1_000, false, 200));
    // Experiment : 2 runs, coût 500 µ¢ (moins cher), 2/2 qualité.
    report.add_run(arm_record(ABArm::Experiment, 500, true, 80));
    report.add_run(arm_record(ABArm::Experiment, 500, true, 120));
    // 1 run hors A/B (ab_arm None) → ignoré par la ségrégation.
    report.add_run({
        let cm = ChewingMetrics::default(); // ab_arm None
        RunRecord::new_with_microcents(
            9_999,
            RunFacts {
                fixture_name: "f".into(),
                mode: RunMode::Baseline,
                latency_ms: 999,
                tokens_output: 10,
                final_text: "x",
                llm_used: "llm".into(),
                task_class: "FactualQa".into(),
                chewing_metrics: Some(cm),
                passed_quality: true,
                failure_mode: efe_coordinator::FailureMode::None,
            },
        )
    });

    let cmp = report.ab_comparison().expect("runs A/B présents");
    assert_eq!(cmp.control.n_runs, 2);
    assert_eq!(cmp.experiment.n_runs, 2);
    assert!((cmp.control.avg_cost_microcents - 1_000.0).abs() < 1e-6);
    assert!((cmp.experiment.avg_cost_microcents - 500.0).abs() < 1e-6);
    assert!((cmp.control.quality_pass_rate - 0.5).abs() < 1e-6);
    assert!((cmp.experiment.quality_pass_rate - 1.0).abs() < 1e-6);
    // Δ experiment − control : coût -500 µ¢ (moins cher), qualité +0.5.
    assert!((cmp.cost_delta_microcents + 500.0).abs() < 1e-6);
    assert!((cmp.quality_delta - 0.5).abs() < 1e-6);
    // R3.S8 : version de table tracée par bras (traçabilité calibration).
    assert_eq!(cmp.control.table_version, "ctrl-v1");
    assert_eq!(cmp.experiment.table_version, "exp-v2");

    // Markdown : contient les deux bras + leurs versions + le delta.
    let md = report.ab_section_markdown();
    assert!(md.contains("control"));
    assert!(md.contains("experiment"));
    assert!(md.contains("A/B test"));
    assert!(md.contains("ctrl-v1"), "la version control doit apparaître");
    assert!(md.contains("exp-v2"), "la version experiment doit apparaître");
}

#[test]
fn ab_comparison_none_when_no_ab_runs() {
    let mut report = SavingsReport::new();
    report.add_run(RunRecord::new_with_microcents(
        100,
        RunFacts {
            fixture_name: "f".into(),
            mode: RunMode::Baseline,
            latency_ms: 50,
            tokens_output: 10,
            final_text: "x",
            llm_used: "llm".into(),
            task_class: "FactualQa".into(),
            chewing_metrics: None,
            passed_quality: true,
            failure_mode: efe_coordinator::FailureMode::None,
        },
    ));
    assert!(report.ab_comparison().is_none(), "aucun run A/B → None");
    assert!(report.ab_section_markdown().is_empty());
}
