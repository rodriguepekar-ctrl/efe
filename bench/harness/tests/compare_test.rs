//! PR #21 § 12 + § 26 — integration tests for the compare pipeline.
//!
//! The unit tests in `src/compare/mod.rs` cover pure in-memory paths.
//! This file exercises the **disk I/O + CLI dispatch** path :
//!   * write a baseline JSON,
//!   * write a result JSON,
//!   * call `engine::dispatch_compare`,
//!   * assert exit code + verdict + that the JSON output parses back.

use std::path::Path;

use efe_bench_harness::baseline::Baseline;
use efe_bench_harness::cli::{CompareArgs, CompareFormat};
use efe_bench_harness::compare::{compare, Verdict};
use efe_bench_harness::engine;
use efe_bench_harness::error::ExitCode;
use efe_bench_harness::result::{ResultMetrics, ScenarioResult};
use efe_bench_harness::scenario::Scenario;

fn make_scenario(id: &str) -> Scenario {
    Scenario {
        schema_version: 1,
        id: id.into(),
        description: "test".into(),
        duration: "1m".into(),
        cluster: Default::default(),
        loadgen: efe_bench_harness::scenario::ScenarioLoadgen {
            version: 1,
            name: "t".into(),
            target: serde_yaml::Value::Null,
            pattern: serde_yaml::Value::Null,
            workload: None,
            rate: None,
            termination: None,
            output: None,
            observability: None,
            run: None,
            extra: Default::default(),
        },
        chaos: vec![],
        results: Default::default(),
        tolerance: Default::default(),
    }
}

fn make_baseline(scenario_id: &str, metrics: ResultMetrics) -> Baseline {
    Baseline {
        schema_version: 1,
        scenario_id: scenario_id.into(),
        harness_version: "0.1.0".into(),
        loadgen_version: "0.1.0".into(),
        chart_version: "0.6.0".into(),
        image_tag: "t".into(),
        git_commit: "abcdef0".into(),
        run_id: "r-test".into(),
        started_at: "2026-05-16T00:00:00Z".into(),
        duration_s: 60.0,
        total_requests: 6000,
        metrics,
        chaos_events: vec![],
        verdict: Some("pass".into()),
    }
}

fn write_baseline(b: &Baseline, path: &Path) {
    b.write(path).expect("write baseline");
}

fn write_result(r: &ScenarioResult, path: &Path) {
    r.write(path).expect("write result");
}

// ─────────────────────────────────────────────────────────────────────
// 1.  JSON roundtrip — baseline disk → load → memory.
// ─────────────────────────────────────────────────────────────────────
#[test]
fn baseline_json_roundtrips_via_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let baseline_metrics = ResultMetrics {
        p50_ms: Some(8.0),
        p95_ms: Some(100.0),
        p99_ms: Some(500.0),
        error_rate: Some(0.001),
        throughput_qps: Some(100.0),
        recovery_time_s: None,
    };
    let baseline_path = tmp.path().join("S01_x.json");
    let b = make_baseline("S01_x", baseline_metrics.clone());
    write_baseline(&b, &baseline_path);
    let loaded = Baseline::load(&baseline_path).expect("load");
    assert_eq!(loaded.metrics, baseline_metrics);
    assert_eq!(loaded.scenario_id, "S01_x");
}

// ─────────────────────────────────────────────────────────────────────
// 2.  Verdict combine — pass + fail = fail (worst-of).
// ─────────────────────────────────────────────────────────────────────
#[test]
fn mixed_metrics_fail_overrides_pass() {
    let scen = make_scenario("S02_x");
    let baseline = make_baseline(
        "S02_x",
        ResultMetrics {
            p50_ms: Some(8.0),
            p95_ms: Some(100.0),
            p99_ms: Some(500.0),
            error_rate: Some(0.001),
            throughput_qps: Some(100.0),
            recovery_time_s: None,
        },
    );
    let result = ScenarioResult::with_metrics(
        "S02_x".into(),
        ResultMetrics {
            p50_ms: Some(8.1),    // +1.25% — well within
            p95_ms: Some(160.0),  // +60% — exceeds default 25% fail
            p99_ms: Some(510.0),  // +2% — within
            error_rate: Some(0.001),
            throughput_qps: Some(99.0),
            recovery_time_s: None,
        },
    );
    let report = compare(&scen, &result, Some(&baseline));
    assert!(matches!(report.overall, Verdict::Fail));
    // Confirm the failing metric is identified specifically.
    let p95_row = report
        .rows
        .get(&efe_bench_harness::scenario::ToleranceMetric::P95Ms)
        .expect("p95 row present");
    use efe_bench_harness::compare::MetricStatus;
    assert!(matches!(p95_row.status, MetricStatus::Fail));
}

// ─────────────────────────────────────────────────────────────────────
// 3.  Throughput is higher-is-better : a DROP triggers fail.
// ─────────────────────────────────────────────────────────────────────
#[test]
fn throughput_drop_below_min_delta_triggers_fail() {
    let scen = make_scenario("S03_x");
    let baseline = make_baseline(
        "S03_x",
        ResultMetrics {
            p50_ms: Some(8.0),
            p95_ms: Some(100.0),
            p99_ms: Some(500.0),
            error_rate: Some(0.001),
            throughput_qps: Some(1000.0),
            recovery_time_s: None,
        },
    );
    // Throughput drops to 700 QPS = -30% ; default min_delta_pct = -15%.
    let result = ScenarioResult::with_metrics(
        "S03_x".into(),
        ResultMetrics {
            p50_ms: Some(8.0),
            p95_ms: Some(100.0),
            p99_ms: Some(500.0),
            error_rate: Some(0.001),
            throughput_qps: Some(700.0),
            recovery_time_s: None,
        },
    );
    let report = compare(&scen, &result, Some(&baseline));
    assert!(
        matches!(report.overall, Verdict::Fail),
        "expected Fail, got {:?}",
        report.overall
    );
}

// ─────────────────────────────────────────────────────────────────────
// 4.  Full dispatch_compare CLI path — exit codes + JSON output.
// ─────────────────────────────────────────────────────────────────────
// ─────────────────────────────────────────────────────────────────────
// D-AN — dispatch_compare against the SHIPPED baseline fixtures in
// `bench_baselines/v0.6.0/L2/`.  Previously every test used synthetic
// inline fixtures ; this proves the path also works against real
// on-disk baseline JSON files committed to the repo.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_an_dispatch_compare_against_shipped_baseline_v0_6_0() {
    let baseline_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../bench_baselines/v0.6.0/L2/S01_baseline_100qps_5min.json");
    if !baseline_path.exists() {
        // If a future PR moves the baseline files, fail loudly here
        // rather than silently skip.
        panic!(
            "D-AN : shipped baseline missing at {} — rename drift",
            baseline_path.display()
        );
    }
    // Build a result that mirrors the baseline metrics exactly → must
    // verdict Pass.
    let baseline_body = std::fs::read_to_string(&baseline_path).unwrap();
    let baseline: serde_json::Value = serde_json::from_str(&baseline_body).unwrap();
    let metrics = ResultMetrics {
        p50_ms: baseline["metrics"]["p50_ms"].as_f64(),
        p95_ms: baseline["metrics"]["p95_ms"].as_f64(),
        p99_ms: baseline["metrics"]["p99_ms"].as_f64(),
        error_rate: baseline["metrics"]["error_rate"].as_f64(),
        throughput_qps: baseline["metrics"]["throughput_qps"].as_f64(),
        recovery_time_s: baseline["metrics"]["recovery_time_s"].as_f64(),
    };
    let tmp = tempfile::tempdir().unwrap();
    let result_path = tmp.path().join("result.json");
    write_result(
        &ScenarioResult::with_metrics("S01_baseline_100qps_5min".into(), metrics),
        &result_path,
    );

    let args = CompareArgs {
        result: result_path,
        baseline: baseline_path,
        format: CompareFormat::Json,
    };
    let rc = engine::dispatch_compare(args).await.expect("dispatch");
    assert_eq!(
        rc,
        ExitCode::Success,
        "D-AN : shipped baseline must compare clean against identical metrics"
    );
}

#[tokio::test]
async fn dispatch_compare_pass_exits_success() {
    let tmp = tempfile::tempdir().unwrap();
    let metrics = ResultMetrics {
        p50_ms: Some(8.0),
        p95_ms: Some(100.0),
        p99_ms: Some(500.0),
        error_rate: Some(0.001),
        throughput_qps: Some(100.0),
        recovery_time_s: None,
    };
    let baseline_path = tmp.path().join("baseline.json");
    let result_path = tmp.path().join("result.json");
    write_baseline(&make_baseline("S04_x", metrics.clone()), &baseline_path);
    write_result(
        &ScenarioResult::with_metrics("S04_x".into(), metrics),
        &result_path,
    );

    let args = CompareArgs {
        result: result_path,
        baseline: baseline_path,
        format: CompareFormat::Json,
    };
    let rc = engine::dispatch_compare(args).await.expect("dispatch");
    assert_eq!(rc, ExitCode::Success);
}

#[tokio::test]
async fn dispatch_compare_fail_returns_bench_verdict_error() {
    let tmp = tempfile::tempdir().unwrap();
    let baseline_metrics = ResultMetrics {
        p50_ms: Some(8.0),
        p95_ms: Some(100.0),
        p99_ms: Some(500.0),
        error_rate: Some(0.001),
        throughput_qps: Some(100.0),
        recovery_time_s: None,
    };
    let bad_metrics = ResultMetrics {
        p99_ms: Some(2_000.0), // +300% vs 500ms baseline ⇒ blow past 35% fail
        ..baseline_metrics.clone()
    };
    let baseline_path = tmp.path().join("baseline.json");
    let result_path = tmp.path().join("result.json");
    write_baseline(&make_baseline("S05_x", baseline_metrics), &baseline_path);
    write_result(
        &ScenarioResult::with_metrics("S05_x".into(), bad_metrics),
        &result_path,
    );

    let args = CompareArgs {
        result: result_path,
        baseline: baseline_path,
        format: CompareFormat::Text,
    };
    let err = engine::dispatch_compare(args).await.expect_err("must fail");
    assert_eq!(err.exit_code(), ExitCode::BenchFail);
}

// ─────────────────────────────────────────────────────────────────────
// 5.  No baseline ⇒ no-baseline verdict, dispatch still succeeds.
// ─────────────────────────────────────────────────────────────────────
#[test]
fn missing_baseline_yields_no_baseline_verdict() {
    use efe_bench_harness::compare::MetricStatus;
    let scen = make_scenario("S06_x");
    let metrics = ResultMetrics {
        p50_ms: Some(8.0),
        p95_ms: Some(100.0),
        p99_ms: Some(500.0),
        error_rate: Some(0.001),
        throughput_qps: Some(100.0),
        recovery_time_s: None,
    };
    let result = ScenarioResult::with_metrics("S06_x".into(), metrics);
    let report = compare(&scen, &result, None);
    assert!(matches!(report.overall, Verdict::NoBaseline));
    // 5 metrics present (recovery_time_s is None) ⇒ 5 NoBaseline rows.
    assert_eq!(report.rows.len(), 5);
    for row in report.rows.values() {
        assert!(
            matches!(row.status, MetricStatus::NoBaseline),
            "every row must be NoBaseline ; got {:?}", row.status
        );
        assert!(row.baseline.is_none());
        assert!(row.actual.is_some());
    }
}

// ─────────────────────────────────────────────────────────────────────
// 6.  Scenario-level tolerance override beats the default.
// ─────────────────────────────────────────────────────────────────────
#[test]
fn scenario_tolerance_override_loosens_threshold() {
    // S01 overrides p95 fail threshold to 80% — a +50% p95 regression
    // that would normally fail (default 25%) must now PASS.
    let mut scen = make_scenario("S07_x");
    scen.tolerance = efe_bench_harness::scenario::ToleranceSpec {
        p95_ms_max_delta_pct: Some(80.0),
        ..Default::default()
    };
    let baseline_metrics = ResultMetrics {
        p50_ms: Some(8.0),
        p95_ms: Some(100.0),
        p99_ms: Some(500.0),
        error_rate: Some(0.001),
        throughput_qps: Some(100.0),
        recovery_time_s: None,
    };
    let baseline = make_baseline("S07_x", baseline_metrics.clone());
    let result = ScenarioResult::with_metrics(
        "S07_x".into(),
        ResultMetrics {
            p95_ms: Some(150.0), // +50%
            ..baseline_metrics
        },
    );
    let report = compare(&scen, &result, Some(&baseline));
    assert!(
        !matches!(report.overall, Verdict::Fail),
        "scenario override should suppress fail ; got {:?}",
        report.overall
    );
}
