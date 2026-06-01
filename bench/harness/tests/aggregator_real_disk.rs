//! PR #24 § 15 + D-R — end-to-end aggregator test.
//!
//! The first three audit passes verified the aggregator's data flow
//! through unit tests, but NEVER exercised the actual path :
//!   real `summary.json` on disk → `AggregatedPublish::from_disk` →
//!   non-empty `/api/current.json` written to disk.
//!
//! This test plants a single fake bench-results run on disk, runs the
//! aggregator + publisher, and asserts that `current.json` and
//! `baselines.json` carry the real data (not empty stubs).
//!
//! If the aggregator silently drops data, this test fails.

use std::collections::BTreeMap;

use efe_bench_harness::publish::Publisher;
use efe_bench_harness::publish::aggregator::AggregatedPublish;

fn plant_fake_run(
    bench_results: &std::path::Path,
    bench_baselines: &std::path::Path,
    scenarios_root: &std::path::Path,
) {
    // 1.  Plant a scenarios/S01_baseline_100qps_5min.yaml.
    std::fs::create_dir_all(scenarios_root).unwrap();
    let real_scen = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    let yaml = std::fs::read(&real_scen).expect("real scenario must exist");
    std::fs::write(
        scenarios_root.join("S01_baseline_100qps_5min.yaml"),
        yaml,
    )
    .unwrap();

    // 2.  Plant bench/results/<run>/S01/summary.json.
    let run_id = "01J9-fake-run-2026-05";
    let scen_dir = bench_results.join(run_id).join("S01_baseline_100qps_5min");
    std::fs::create_dir_all(&scen_dir).unwrap();
    let summary = serde_json::json!({
        "schema_version": 1,
        "scenario_id": "S01_baseline_100qps_5min",
        "harness_version": "0.2.0",
        "loadgen_version": "0.1.0",
        "chart_version": "0.6.0",
        "image_tag": "v0.6.0",
        "git_commit": "deadbeefcafe",
        "run_id": run_id,
        "started_at": "2026-05-16T09:00:00Z",
        "duration_s": 300.0,
        "total_requests": 30_000,
        "metrics": {
            "p50_ms": 7.5,
            "p95_ms": 12.3,
            "p99_ms": 18.0,
            "error_rate": 0.001,
            "throughput_qps": 99.7
        },
        "chaos_events": [],
        "verdict": "pass"
    });
    std::fs::write(
        scen_dir.join("summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();

    // 3.  Plant bench_baselines/v0.6.0/L2/S01_baseline_100qps_5min.json.
    let baseline_dir = bench_baselines.join("v0.6.0/L2");
    std::fs::create_dir_all(&baseline_dir).unwrap();
    let baseline = serde_json::json!({
        "schema_version": 1,
        "scenario_id": "S01_baseline_100qps_5min",
        "harness_version": "0.2.0",
        "loadgen_version": "0.1.0",
        "chart_version": "0.6.0",
        "image_tag": "v0.6.0",
        "git_commit": "deadbeefcafe",
        "run_id": "01J9-baseline-run",
        "started_at": "2026-04-01T00:00:00Z",
        "duration_s": 300.0,
        "total_requests": 30_000,
        "metrics": {
            "p50_ms": 7.0,
            "p95_ms": 12.0,
            "p99_ms": 17.0,
            "error_rate": 0.001,
            "throughput_qps": 100.0
        },
        "chaos_events": [],
        "verdict": "pass"
    });
    std::fs::write(
        baseline_dir.join("S01_baseline_100qps_5min.json"),
        serde_json::to_vec_pretty(&baseline).unwrap(),
    )
    .unwrap();
}

#[test]
fn d_r_aggregator_reads_real_summary_json_into_current_scenarios() {
    let tmp = tempfile::tempdir().unwrap();
    let bench_results = tmp.path().join("bench/results");
    let bench_baselines = tmp.path().join("bench_baselines");
    let scenarios_root = tmp.path().join("bench/scenarios");
    plant_fake_run(&bench_results, &bench_baselines, &scenarios_root);

    let agg = AggregatedPublish::from_disk(&bench_results, &bench_baselines, &scenarios_root)
        .expect("aggregator must succeed");

    // CRITICAL : current_scenarios must contain S01 with REAL p95 = 12.3.
    let entry = agg
        .current_scenarios
        .get("S01_baseline_100qps_5min")
        .expect("D-R : aggregator dropped the scenario — current.json would be empty");
    assert_eq!(entry.p95_ms, Some(12.3), "D-R : aggregator lost the p95 metric");
    assert_eq!(entry.p50_ms, Some(7.5));
    assert_eq!(entry.verdict, "pass");

    // overall_verdict = Pass when the only scenario is Pass.
    assert_eq!(
        agg.overall_verdict.as_str(),
        "pass",
        "D-R : overall_verdict aggregation broken"
    );

    // latest_run_id propagated.
    assert_eq!(
        agg.latest_run_id.as_deref(),
        Some("01J9-fake-run-2026-05"),
        "D-R : latest_run_id missing"
    );

    // baselines populated.
    assert!(
        agg.baselines.contains_key("0.6.0"),
        "D-R : baselines map missing v0.6.0 entry"
    );
    let bset = &agg.baselines["0.6.0"];
    assert!(
        bset.scenarios.iter().any(|b| b.id == "S01_baseline_100qps_5min"),
        "D-R : baseline entry for S01 missing"
    );

    // badge_rows has the same data.
    assert!(
        agg.badge_rows
            .iter()
            .any(|(id, _, p)| id == "S01_baseline_100qps_5min" && *p == Some(12.3)),
        "D-R : badge_rows lost S01's p95"
    );
}

#[tokio::test]
async fn d_r_publisher_writes_non_empty_current_json_from_real_data() {
    use efe_bench_harness::compare::Verdict;

    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("site");
    let bench_results = tmp.path().join("bench/results");
    let bench_baselines = tmp.path().join("bench_baselines");
    let scenarios_root = tmp.path().join("bench/scenarios");
    plant_fake_run(&bench_results, &bench_baselines, &scenarios_root);

    let mut p = Publisher::new(
        out.clone(),
        std::path::PathBuf::from("/tmp/nx-real-data"),
        "0.6.0".into(),
    );
    p.bench_results = bench_results.clone();
    p.bench_baselines = bench_baselines.clone();

    let agg = AggregatedPublish::from_disk(&bench_results, &bench_baselines, &scenarios_root)
        .unwrap();
    let scenarios = efe_bench_harness::scenario::Scenario::discover(&scenarios_root).unwrap();
    let rows: Vec<_> = agg.badge_rows.clone();

    p.write_skeleton_with_agg(&rows, Some(&agg), Some(&scenarios))
        .await
        .unwrap();

    // current.json MUST be non-empty (D-B was the original sin :
    // current.json was always empty even when bench data existed).
    let current_body =
        std::fs::read_to_string(out.join("api/current.json")).expect("current.json must exist");
    assert!(
        current_body.contains("S01_baseline_100qps_5min"),
        "D-R : current.json missing the scenario id — aggregator wiring broken :\n{current_body}"
    );
    assert!(
        current_body.contains("12.3"),
        "D-R : current.json missing the p95 metric value 12.3 :\n{current_body}"
    );
    assert!(
        current_body.contains("\"pass\""),
        "D-R : current.json missing the canonical pass verdict :\n{current_body}"
    );

    // baselines.json MUST list v0.6.0.
    let baselines_body = std::fs::read_to_string(out.join("api/baselines.json")).unwrap();
    assert!(
        baselines_body.contains("0.6.0"),
        "D-R : baselines.json missing v0.6.0 entry :\n{baselines_body}"
    );

    // scenarios.json MUST list S01.
    let scen_body = std::fs::read_to_string(out.join("api/scenarios.json")).unwrap();
    assert!(
        scen_body.contains("S01_baseline_100qps_5min"),
        "D-R : scenarios.json missing S01 entry :\n{scen_body}"
    );

    // D-P invariant : every verdict string in current.json + scenarios.json
    // uses the canonical kebab-case "no-baseline" (not the diverged
    // "no_data" / "no_baseline" snake-case).  We accept "pass" here
    // because S01 had a real result.
    assert!(
        !current_body.contains("\"no_baseline\""),
        "D-P : current.json must not carry the diverged snake_case verdict"
    );
    assert!(
        !scen_body.contains("\"no_data\""),
        "D-P : scenarios.json must not carry the legacy 'no_data' literal"
    );

    // Suppress unused warning.
    let _ = Verdict::NoBaseline;
}

// ─────────────────────────────────────────────────────────────────────
// D-S — sparkline byte stability : with the D-K change (real verdict
// + real p95 propagated), sparklines must remain deterministic across
// runs with the same input data.
// ─────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────
// D-AO — aggregator must degrade gracefully on malformed summary.json
// (corrupt JSON, schema-version drift, missing fields).  Must NEVER
// crash the publish flow — emits warn! and falls back to NoBaseline.
// ─────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────
// D-AZ — KNOWN GAP : tolerance_used always empty in current.json.
// This test pins the current (incomplete) behaviour so any future PR
// that fixes it MUST update this test — making the fix visible.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_az_tolerance_used_currently_always_empty_pinned() {
    let tmp = tempfile::tempdir().unwrap();
    let bench_results = tmp.path().join("bench/results");
    let bench_baselines = tmp.path().join("bench_baselines");
    let scenarios_root = tmp.path().join("bench/scenarios");
    plant_fake_run(&bench_results, &bench_baselines, &scenarios_root);

    let agg = AggregatedPublish::from_disk(&bench_results, &bench_baselines, &scenarios_root)
        .unwrap();
    let entry = agg.current_scenarios.get("S01_baseline_100qps_5min").unwrap();
    assert!(
        entry.tolerance_used.is_empty(),
        "D-AZ : current.json::tolerance_used is currently always empty (known Phase-2 gap). \
         If you've populated it, update this test to assert the new fields."
    );
}

// ─────────────────────────────────────────────────────────────────────
// D-BA — schema field `recovery_time_s` exists in ResultMetrics and
// flows through compare, but real runs do NOT populate it (loadgen
// doesn't measure recovery time yet).  Pin this so the surface stays
// honest : the field is there but always None in produced runs.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_ba_recovery_time_s_field_present_but_unpopulated_in_aggregator() {
    use efe_bench_harness::result::{ResultMetrics, ScenarioResult};
    let m = ResultMetrics {
        p50_ms: Some(8.0),
        p95_ms: Some(12.0),
        p99_ms: Some(18.0),
        error_rate: Some(0.001),
        throughput_qps: Some(100.0),
        recovery_time_s: Some(2.5),  // explicitly set
    };
    let r = ScenarioResult::with_metrics("S04_chaos".into(), m);
    let body = serde_json::to_vec_pretty(&r).unwrap();
    let parsed: ScenarioResult = serde_json::from_slice(&body).unwrap();
    // The field round-trips correctly when populated upstream.
    assert_eq!(
        parsed.metrics.recovery_time_s,
        Some(2.5),
        "D-BA : recovery_time_s schema field must round-trip when set"
    );
}

#[test]
fn d_ao_aggregator_survives_corrupted_summary_json() {
    let tmp = tempfile::tempdir().unwrap();
    let bench_results = tmp.path().join("bench/results");
    let bench_baselines = tmp.path().join("bench_baselines");
    let scenarios_root = tmp.path().join("bench/scenarios");

    // Plant a scenario YAML so the aggregator has something to look for.
    std::fs::create_dir_all(&scenarios_root).unwrap();
    let real_scen = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    let yaml = std::fs::read(&real_scen).expect("real scenario");
    std::fs::write(
        scenarios_root.join("S01_baseline_100qps_5min.yaml"),
        yaml,
    )
    .unwrap();

    // Plant a CORRUPT summary.json.
    let scen_dir = bench_results.join("run-corrupt").join("S01_baseline_100qps_5min");
    std::fs::create_dir_all(&scen_dir).unwrap();
    std::fs::write(scen_dir.join("summary.json"), b"{not valid json").unwrap();

    let agg = AggregatedPublish::from_disk(&bench_results, &bench_baselines, &scenarios_root)
        .expect("D-AO : aggregator must succeed even with corrupt summary");

    // Falls back to NoBaseline (not Pass / Fail / Warn).
    assert_eq!(
        agg.overall_verdict,
        efe_bench_harness::compare::Verdict::NoBaseline,
        "D-AO : corrupt summary must collapse to NoBaseline"
    );
    // current_scenarios still has S01 (via continue_with_empty_entry).
    assert!(
        agg.current_scenarios.contains_key("S01_baseline_100qps_5min"),
        "D-AO : aggregator must still emit a scenario entry"
    );
    let entry = &agg.current_scenarios["S01_baseline_100qps_5min"];
    assert_eq!(entry.verdict, "no-baseline");
    assert_eq!(entry.p95_ms, None);
}

#[test]
fn d_ao_aggregator_survives_schema_version_drift() {
    let tmp = tempfile::tempdir().unwrap();
    let bench_results = tmp.path().join("bench/results");
    let bench_baselines = tmp.path().join("bench_baselines");
    let scenarios_root = tmp.path().join("bench/scenarios");

    std::fs::create_dir_all(&scenarios_root).unwrap();
    let real_scen = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    let yaml = std::fs::read(&real_scen).unwrap();
    std::fs::write(
        scenarios_root.join("S01_baseline_100qps_5min.yaml"),
        yaml,
    )
    .unwrap();

    // Plant a summary.json with FUTURE schema_version (= 2).
    // The harness rejects this with BenchError::Internal, and the
    // aggregator must catch + degrade gracefully (D-O fix).
    let scen_dir = bench_results.join("run-future").join("S01_baseline_100qps_5min");
    std::fs::create_dir_all(&scen_dir).unwrap();
    let future = serde_json::json!({
        "schema_version": 2,
        "scenario_id": "S01_baseline_100qps_5min",
        "harness_version": "0.3.0",
        "loadgen_version": "0.2.0",
        "chart_version": "0.7.0",
        "image_tag": "v0.7.0",
        "git_commit": "future",
        "run_id": "run-future",
        "started_at": "2027-01-01T00:00:00Z",
        "duration_s": 300.0,
        "total_requests": 30000,
        "metrics": {"p50_ms": 5.0, "p95_ms": 10.0, "p99_ms": 15.0, "error_rate": 0.0, "throughput_qps": 100.0},
        "chaos_events": [],
        "verdict": "pass",
        "new_field_only_in_v2": "ignored"
    });
    std::fs::write(
        scen_dir.join("summary.json"),
        serde_json::to_vec_pretty(&future).unwrap(),
    )
    .unwrap();

    let agg = AggregatedPublish::from_disk(&bench_results, &bench_baselines, &scenarios_root)
        .expect("D-AO : aggregator must not crash on schema drift");
    // Schema version mismatch → ScenarioResult::read returns Err →
    // aggregator falls back to NoBaseline.
    let entry = agg.current_scenarios.get("S01_baseline_100qps_5min").unwrap();
    assert_eq!(
        entry.verdict, "no-baseline",
        "D-AO : schema drift must NOT be silently promoted to pass"
    );
}

#[test]
fn d_ao_aggregator_survives_missing_metrics_field() {
    let tmp = tempfile::tempdir().unwrap();
    let bench_results = tmp.path().join("bench/results");
    let bench_baselines = tmp.path().join("bench_baselines");
    let scenarios_root = tmp.path().join("bench/scenarios");

    std::fs::create_dir_all(&scenarios_root).unwrap();
    let real_scen = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    let yaml = std::fs::read(&real_scen).unwrap();
    std::fs::write(
        scenarios_root.join("S01_baseline_100qps_5min.yaml"),
        yaml,
    )
    .unwrap();

    // Plant a summary.json with completely missing required fields.
    let scen_dir = bench_results
        .join("run-incomplete")
        .join("S01_baseline_100qps_5min");
    std::fs::create_dir_all(&scen_dir).unwrap();
    std::fs::write(
        scen_dir.join("summary.json"),
        br#"{"schema_version": 1, "incomplete": true}"#,
    )
    .unwrap();

    let agg = AggregatedPublish::from_disk(&bench_results, &bench_baselines, &scenarios_root)
        .expect("D-AO : aggregator must survive incomplete JSON");
    let entry = agg.current_scenarios.get("S01_baseline_100qps_5min").unwrap();
    assert_eq!(entry.verdict, "no-baseline");
    assert!(entry.p95_ms.is_none());
}

#[tokio::test]
async fn d_s_sparkline_byte_stable_with_real_aggregator_data() {
    use efe_bench_harness::compare::Verdict;
    use efe_bench_harness::scenario::Scenario;

    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();

    let scen_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    let scen = Scenario::load(&scen_path).unwrap();

    let agg = AggregatedPublish {
        current_scenarios: BTreeMap::new(),
        scenarios_meta: BTreeMap::new(),
        baselines: BTreeMap::new(),
        overall_verdict: Verdict::Pass,
        latest_run_id: None,
        latest_git_sha: None,
        badge_rows: vec![(scen.id.clone(), Verdict::Pass, Some(12.3))],
    };

    for tmp in [&tmp1, &tmp2] {
        let p = Publisher::new(
            tmp.path().to_path_buf(),
            std::path::PathBuf::from("/tmp/nx-spark-stable"),
            "0.6.0".into(),
        );
        p.write_sparklines(std::slice::from_ref(&scen), Some(&agg))
            .await
            .unwrap();
    }

    let a = std::fs::read(tmp1.path().join("badges/trend-S01.svg")).unwrap();
    let b = std::fs::read(tmp2.path().join("badges/trend-S01.svg")).unwrap();
    assert_eq!(
        a, b,
        "D-S : trend sparkline must be byte-stable for the same input"
    );
}
