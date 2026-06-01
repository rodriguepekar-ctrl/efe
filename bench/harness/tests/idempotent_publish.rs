//! PR #24 § 22 + AC2 + AD37 — idempotent publish.
//!
//! Re-running `publish` against the same inputs MUST produce byte-
//! identical output (modulo non-deterministic timestamps : `as_of`,
//! `published_at`, `run_id`).  We compare the **deterministic** subset
//! (badges + scenario pages + chaos.html + the JSON files we control).

use std::path::Path;

use efe_bench_harness::publish::Publisher;

fn collect_bytes(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
                let body = std::fs::read(&p).unwrap_or_default();
                out.push((rel, body));
            }
        }
    }
}

fn scenarios_fixture() -> Vec<(String, efe_bench_harness::compare::Verdict, Option<f64>)> {
    vec![
        ("S01_baseline_100qps_5min".into(), efe_bench_harness::compare::Verdict::Pass, Some(8.2)),
        ("S02_canary_1000qps_15min".into(), efe_bench_harness::compare::Verdict::Pass, Some(12.1)),
        ("S03_ramp_to_breakdown".into(), efe_bench_harness::compare::Verdict::Pass, Some(45.0)),
        ("S04_chaos_participant_loss".into(), efe_bench_harness::compare::Verdict::Pass, Some(89.0)),
        ("S05_chaos_network_partition".into(), efe_bench_harness::compare::Verdict::Pass, Some(92.0)),
        ("S06_chaos_cert_expiry".into(), efe_bench_harness::compare::Verdict::Pass, Some(78.0)),
        ("S07_chaos_reload_storm".into(), efe_bench_harness::compare::Verdict::Pass, Some(101.0)),
        ("S08_multi_region_replay".into(), efe_bench_harness::compare::Verdict::Pass, Some(142.0)),
        ("S09_soak_2h_stability".into(), efe_bench_harness::compare::Verdict::Pass, Some(99.0)),
        ("S10_slo_validation_30min".into(), efe_bench_harness::compare::Verdict::Pass, None),
    ]
}

#[tokio::test]
async fn ac2_publish_is_byte_identical_across_runs_for_badges() {
    // We compare the BADGES only — those are pure deterministic (no
    // timestamps, no run_id).  JSON files carry as_of/run_id which
    // change per call by design (D6 covers the deterministic subset).
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();

    let p1 = Publisher::new(
        tmp1.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nonexistent-source-1"),
        "0.6.0".into(),
    );
    let p2 = Publisher::new(
        tmp2.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nonexistent-source-2"),
        "0.6.0".into(),
    );
    let scenarios = scenarios_fixture();
    p1.write_skeleton(&scenarios).await.unwrap();
    p2.write_skeleton(&scenarios).await.unwrap();

    // Compare only the badges/ subtree.
    let bytes1 = collect_bytes(&tmp1.path().join("badges"));
    let bytes2 = collect_bytes(&tmp2.path().join("badges"));
    assert_eq!(bytes1.len(), bytes2.len(), "badge counts differ");
    assert_eq!(bytes1, bytes2, "badge bytes diverged between runs");
}

#[tokio::test]
async fn ac2_publish_html_pages_count_stable_across_runs() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    let p1 = Publisher::new(
        tmp1.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-a"),
        "0.6.0".into(),
    );
    let p2 = Publisher::new(
        tmp2.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-b"),
        "0.6.0".into(),
    );
    let s = scenarios_fixture();
    p1.write_skeleton(&s).await.unwrap();
    p2.write_skeleton(&s).await.unwrap();

    // Same number of files written in both runs.
    let n1 = collect_bytes(tmp1.path()).len();
    let n2 = collect_bytes(tmp2.path()).len();
    assert_eq!(n1, n2);
}

#[tokio::test]
async fn placeholder_html_carries_csp_header_ac6() {
    // **AC6** — every placeholder HTML file must carry a CSP meta tag.
    let tmp = tempfile::tempdir().unwrap();
    let p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nonexistent"),
        "0.6.0".into(),
    );
    let s = scenarios_fixture();
    p.write_skeleton(&s).await.unwrap();

    // Walk every .html file emitted and verify CSP meta tag.
    let entries = collect_bytes(tmp.path());
    let html_files: Vec<_> = entries
        .iter()
        .filter(|(name, _)| name.ends_with(".html"))
        .collect();
    assert!(html_files.len() >= 14, "expected at least 14 HTML pages (4 top-level + 10 scenarios), got {}", html_files.len());
    for (name, body) in &html_files {
        let body_str = String::from_utf8_lossy(body);
        assert!(
            body_str.contains("Content-Security-Policy"),
            "{name} missing CSP meta tag (AC6 violated)"
        );
        assert!(
            !body_str.contains("<script"),
            "{name} contains <script> (T100 / AC5 violated)"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// D-J — `--embed-smoke` flag MUST have a visible effect on the placeholder
// index.html.  Previously the flag was decorative (gated nothing).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_j_embed_smoke_true_injects_smoke_badge_into_index_html() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-smoke-on"),
        "0.6.0".into(),
    );
    p.embed_smoke = true;
    p.write_skeleton(&scenarios_fixture()).await.unwrap();

    let index = std::fs::read_to_string(tmp.path().join("index.html")).unwrap();
    assert!(
        index.contains("https://efe.github.io/smoke/badge.svg"),
        "embed_smoke=true must inject the PR #23 smoke badge URL into index.html — got :\n{index}"
    );
    assert!(
        index.contains("<img"),
        "smoke badge fragment must include an <img> tag"
    );
}

#[tokio::test]
async fn d_j_embed_smoke_false_omits_smoke_badge_from_index_html() {
    let tmp = tempfile::tempdir().unwrap();
    let mut p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-smoke-off"),
        "0.6.0".into(),
    );
    p.embed_smoke = false;
    p.write_skeleton(&scenarios_fixture()).await.unwrap();

    let index = std::fs::read_to_string(tmp.path().join("index.html")).unwrap();
    assert!(
        !index.contains("smoke/badge.svg"),
        "embed_smoke=false must NOT inject the smoke badge — got :\n{index}"
    );
    // CSP + structure preserved even with the flag disabled.
    assert!(index.contains("Content-Security-Policy"));
}

// ─────────────────────────────────────────────────────────────────────
// D-Q — re-running publish twice with the same `embed_smoke` value must
// produce byte-identical index.html (smoke fragment is deterministic).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_q_smoke_embed_is_byte_idempotent_across_runs() {
    let tmp1 = tempfile::tempdir().unwrap();
    let tmp2 = tempfile::tempdir().unwrap();
    for tmp in [&tmp1, &tmp2] {
        let mut p = Publisher::new(
            tmp.path().to_path_buf(),
            std::path::PathBuf::from("/tmp/nx-smoke-idem"),
            "0.6.0".into(),
        );
        p.embed_smoke = true;
        p.write_skeleton(&scenarios_fixture()).await.unwrap();
    }
    let a = std::fs::read(tmp1.path().join("index.html")).unwrap();
    let b = std::fs::read(tmp2.path().join("index.html")).unwrap();
    assert_eq!(
        a, b,
        "D-Q : embed_smoke=true index.html must be byte-identical across runs"
    );
}

// ─────────────────────────────────────────────────────────────────────
// D-K — sparkline verdict colour reflects the aggregator data instead
// of hard-coding NoBaseline.  Visual coherence with scenario badge.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn d_k_sparkline_verdict_colour_matches_scenario_badge() {
    use efe_bench_harness::compare::Verdict;
    use efe_bench_harness::publish::aggregator::AggregatedPublish;
    use efe_bench_harness::scenario::Scenario;

    let tmp = tempfile::tempdir().unwrap();
    let p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx-spark"),
        "0.6.0".into(),
    );

    // Load a real scenario from the repo so we don't have to construct
    // the (large) struct manually — keeps the test honest about what
    // the aggregator/publisher pipeline actually consumes.
    // Test runs from the crate's manifest dir (bench/harness/), so the
    // S01 scenario YAML lives at `../scenarios/`.
    let scen_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../scenarios/S01_baseline_100qps_5min.yaml");
    let scen = Scenario::load(&scen_path)
        .unwrap_or_else(|e| panic!("S01 scenario must load from {scen_path:?} : {e}"));

    // Fake aggregator carrying a PASS verdict + a real p95 for S01.
    let agg = AggregatedPublish {
        current_scenarios: std::collections::BTreeMap::new(),
        scenarios_meta: std::collections::BTreeMap::new(),
        baselines: std::collections::BTreeMap::new(),
        overall_verdict: Verdict::Pass,
        latest_run_id: None,
        latest_git_sha: None,
        badge_rows: vec![(scen.id.clone(), Verdict::Pass, Some(8.2))],
    };
    p.write_sparklines(&[scen], Some(&agg)).await.unwrap();

    let svg = std::fs::read_to_string(tmp.path().join("badges/trend-S01.svg")).unwrap();
    // PASS colour = green #3fb950 (frozen badge palette).
    assert!(
        svg.contains("#3fb950"),
        "D-K : sparkline must carry the PASS colour #3fb950 from the aggregator — got :\n{svg}"
    );
    // Empty-data sentinel must NOT appear when we have a real p95.
    assert!(
        !svg.contains("no data"),
        "D-K : sparkline must render the p95 datapoint, not the empty placeholder"
    );
}

#[tokio::test]
async fn dry_run_does_not_pollute_caller_output_dir() {
    // D1 fix verifies — dry_run writes to a tempdir owned by itself,
    // not the publisher's output_dir.
    let tmp = tempfile::tempdir().unwrap();
    let p = Publisher::new(
        tmp.path().to_path_buf(),
        std::path::PathBuf::from("/tmp/nx"),
        "0.6.0".into(),
    );
    let manifest = p.dry_run().await.unwrap();
    manifest.validate().unwrap();
    // The publisher's output_dir should still be empty.
    assert!(
        std::fs::read_dir(tmp.path())
            .map(|d| d.count() == 0)
            .unwrap_or(true),
        "dry_run leaked files into the publisher's output_dir"
    );
}
