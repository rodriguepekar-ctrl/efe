//! PR #24 — Passage 10 — exhaustive edge cases for AC22, forward-compat,
//! perf bounds, concurrency, and pathological metric values.

use efe_bench_harness::compare::Verdict;
use efe_bench_harness::publish::aggregator::AggregatedPublish;
use efe_bench_harness::publish::Publisher;
use efe_bench_harness::publish::quarterly::pdf::extract_embedded_json_sha256;

// ─────────────────────────────────────────────────────────────────────
// D-BH — Compare must roundtrip a summary.json with an UNKNOWN
// metric field (e.g. PR #25 adds `p999_ms` to the metrics map).
// Existing fields keep working ; the new one is silently dropped.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_bh_compare_summary_with_unknown_metric_roundtrips() {
    use efe_bench_harness::result::ScenarioResult;
    let body = r#"{
        "schema_version": 1,
        "scenario_id": "S01_baseline_100qps_5min",
        "harness_version": "0.2.0",
        "loadgen_version": "0.1.0",
        "chart_version": "0.6.0",
        "image_tag": "v",
        "git_commit": "abc",
        "run_id": "r",
        "started_at": "2026-05-16T09:00:00Z",
        "duration_s": 300.0,
        "total_requests": 30000,
        "metrics": {
            "p50_ms": 8.0,
            "p95_ms": 12.0,
            "p99_ms": 18.0,
            "error_rate": 0.001,
            "throughput_qps": 100.0,
            "p999_ms_future": 25.0,
            "p99999_ms_future": 50.0
        },
        "chaos_events": [],
        "verdict": "pass"
    }"#;
    let r: ScenarioResult = serde_json::from_str(body).expect("D-BH : must parse");
    assert_eq!(r.metrics.p95_ms, Some(12.0));
}

// ─────────────────────────────────────────────────────────────────────
// D-BI — extract_embedded_json_sha256 on a 10 MB PDF must complete
// in well under 1 second.  Detects an accidental quadratic scan.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_bi_extract_sha_on_10mb_pdf_under_one_second() {
    let mut pdf = vec![b'A'; 10 * 1024 * 1024];
    // Plant the marker near the end (worst case for early-exit).
    let marker_pos = pdf.len() - 200;
    let marker = b"/EFE_BENCH_EVIDENCE_JSON_SHA256 (abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789)";
    pdf[marker_pos..marker_pos + marker.len()].copy_from_slice(marker);
    let start = std::time::Instant::now();
    let r = extract_embedded_json_sha256(&pdf);
    let elapsed = start.elapsed();
    assert!(r.is_some(), "D-BI : marker at end of 10 MB PDF must be found");
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "D-BI : 10 MB scan took too long : {elapsed:?}"
    );
}

#[test]
fn d_bi_extract_sha_on_10mb_pdf_no_marker_returns_none_fast() {
    let pdf = vec![b'A'; 10 * 1024 * 1024];
    let start = std::time::Instant::now();
    let r = extract_embedded_json_sha256(&pdf);
    let elapsed = start.elapsed();
    assert!(r.is_none());
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "D-BI : no-marker 10 MB scan took too long : {elapsed:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// D-BJ — concurrent publish-quarterly invocations on the SAME output
// dir must not corrupt each other (UUID temp suffix in write_canonical
// protects JSON, but quarterly writes use plain fs::write — assert
// concurrent runs at least don't crash + leave a valid PDF + JSON pair).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_bj_concurrent_quarterly_runs_dont_corrupt() {
    use efe_bench_harness::cli::PublishQuarterlyArgs;
    use efe_bench_harness::engine::dispatch_publish_quarterly;

    let tmp = std::sync::Arc::new(tempfile::tempdir().unwrap());
    // Two concurrent writers to the SAME output dir + SAME period.
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let tmp = tmp.clone();
            tokio::spawn(async move {
                let args = PublishQuarterlyArgs {
                    period: "Q1-2026".to_string(),
                    chart_version: Some("0.6.0".into()),
                    output: tmp.path().to_path_buf(),
                    cosign_identity: None,
                    cosign_disabled: true,
                    dry_run: false,
                };
                dispatch_publish_quarterly(args).await
            })
        })
        .collect();
    let mut all_ok = true;
    for h in handles {
        let r = h.await.unwrap();
        if r.is_err() {
            all_ok = false;
        }
    }
    assert!(all_ok, "D-BJ : concurrent quarterly runs must both succeed");
    // Final state : PDF + JSON pair must both exist + be parseable.
    let pdf = tmp.path().join("v0.6.0/quarterly/Q1-2026.pdf");
    let json = tmp.path().join("v0.6.0/quarterly/Q1-2026.json");
    assert!(pdf.exists(), "D-BJ : PDF missing after concurrent");
    assert!(json.exists(), "D-BJ : JSON missing after concurrent");
    let _: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json).unwrap()).expect("D-BJ : JSON corrupted");
}

// ─────────────────────────────────────────────────────────────────────
// D-BL — aggregator handles NaN/Inf metrics in summary.json without
// panic.  These are PHYSICAL outliers a hostile producer could craft.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_bl_aggregator_handles_nan_inf_metrics() {
    let tmp = tempfile::tempdir().unwrap();
    let bench_results = tmp.path().join("bench/results");
    let bench_baselines = tmp.path().join("bench_baselines");
    let scenarios_root = tmp.path().join("bench/scenarios");
    std::fs::create_dir_all(&scenarios_root).unwrap();
    let real_scen = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    std::fs::write(
        scenarios_root.join("S01_baseline_100qps_5min.yaml"),
        std::fs::read(&real_scen).unwrap(),
    )
    .unwrap();

    let scen_dir = bench_results
        .join("run-nan")
        .join("S01_baseline_100qps_5min");
    std::fs::create_dir_all(&scen_dir).unwrap();
    // JSON doesn't support NaN/Inf natively, so serde_json rejects them
    // on serialize.  But what if a producer emits the LITERAL strings
    // "NaN" / "Infinity"?  The aggregator should NOT crash — it should
    // treat the file as malformed and fall back.
    let evil = br#"{"schema_version": 1, "scenario_id": "S01_baseline_100qps_5min",
        "harness_version": "0.2.0", "loadgen_version": "0.1.0", "chart_version": "0.6.0",
        "image_tag": "v", "git_commit": "abc", "run_id": "r",
        "started_at": "2026-05-16T09:00:00Z", "duration_s": 300.0, "total_requests": 30000,
        "metrics": {"p50_ms": NaN, "p95_ms": Infinity, "p99_ms": -Infinity, "error_rate": 0.0, "throughput_qps": 0.0},
        "chaos_events": [], "verdict": "pass"}"#;
    std::fs::write(scen_dir.join("summary.json"), evil).unwrap();

    let agg = AggregatedPublish::from_disk(&bench_results, &bench_baselines, &scenarios_root)
        .expect("D-BL : aggregator must not crash on NaN/Inf JSON");
    let entry = agg
        .current_scenarios
        .get("S01_baseline_100qps_5min")
        .unwrap();
    // serde_json rejects NaN/Inf → falls back to NoBaseline.
    assert_eq!(
        entry.verdict, "no-baseline",
        "D-BL : NaN/Inf summary must collapse to no-baseline"
    );
}

// ─────────────────────────────────────────────────────────────────────
// D-BR — sparkline trend files are written using `short_id(&s.id)`.
// If short_id ever returns a value with `/`, file would escape the
// badges/ directory.  Pin the safe-name invariant.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_br_sparkline_filename_cannot_escape_badges_dir() {
    use efe_bench_harness::scenario::Scenario;
    let tmp = tempfile::tempdir().unwrap();
    let p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-br"),
        "0.6.0".into(),
    );
    let scen_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    let scen = Scenario::load(&scen_path).unwrap();
    let agg = AggregatedPublish {
        current_scenarios: std::collections::BTreeMap::new(),
        scenarios_meta: std::collections::BTreeMap::new(),
        baselines: std::collections::BTreeMap::new(),
        overall_verdict: Verdict::Pass,
        latest_run_id: None,
        latest_git_sha: None,
        badge_rows: vec![(scen.id.clone(), Verdict::Pass, Some(8.0))],
    };
    p.write_sparklines(std::slice::from_ref(&scen), Some(&agg))
        .await
        .unwrap();
    // Sanity : only files INSIDE badges/ exist ; nothing escapes.
    let bad = tmp.path().join("trend-S01.svg"); // would-be escape destination
    assert!(
        !bad.exists(),
        "D-BR : sparkline must NOT write outside badges/ ({bad:?})"
    );
    let good = tmp.path().join("badges/trend-S01.svg");
    assert!(
        good.exists(),
        "D-BR : sparkline must write to badges/trend-{{short}}.svg"
    );
}

// ─────────────────────────────────────────────────────────────────────
// D-BM — build module constants must be non-empty + non-default.
// ─────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────
// D-BQ — dry_run does not touch the supplied bench-results path
// (read-only invariant for hermetic dev iteration).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_bq_dry_run_does_not_touch_bench_results_dir() {
    use efe_bench_harness::cli::PublishArgs;
    use efe_bench_harness::engine::dispatch_publish;

    let tmp = tempfile::tempdir().unwrap();
    let bench_results = tmp.path().join("bench-results");
    std::fs::create_dir(&bench_results).unwrap();
    let sentinel = bench_results.join("sentinel.txt");
    std::fs::write(&sentinel, "do not touch").unwrap();
    let original = std::fs::read(&sentinel).unwrap();

    let args = PublishArgs {
        output: tmp.path().join("site"),
        source: std::path::PathBuf::from("/tmp/nx-source"),
        bench_results: Some(bench_results.clone()),
        bench_baselines: None,
        chart_version: Some("0.6.0".into()),
        embed_slos: false,
        embed_smoke: false,
        dry_run: true,
        history_window_days: 30,
    };
    let _ = dispatch_publish(args).await;

    // Sentinel byte-identical.
    let after = std::fs::read(&sentinel).unwrap();
    assert_eq!(
        original, after,
        "D-BQ : dry-run wrote into bench-results dir"
    );
}

#[test]
fn d_bm_build_constants_are_sane() {
    use efe_bench_harness::build;
    assert!(!build::VERSION.is_empty(), "D-BM : VERSION empty");
    assert!(
        build::VERSION.split('.').count() >= 2,
        "D-BM : VERSION not semver-like : '{}'",
        build::VERSION
    );
    assert!(!build::GIT_SHA.is_empty(), "D-BM : GIT_SHA empty");
    // GIT_SHA can be "unknown" in dev builds, but length should match
    // when set (40 chars for full SHA-1, 7 for short).
    assert!(
        build::GIT_SHA == "unknown"
            || build::GIT_SHA.len() == 7
            || build::GIT_SHA.len() == 40,
        "D-BM : GIT_SHA '{}' has unexpected shape",
        build::GIT_SHA
    );
    assert!(!build::BUILD_DATE.is_empty(), "D-BM : BUILD_DATE empty");
}
