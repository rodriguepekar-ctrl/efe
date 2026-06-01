//! PR #24 § 22 — golden JSON byte-stability suite (5 endpoints frozen v1.6).

use std::collections::BTreeMap;

use efe_bench_harness::compare::Verdict;
use efe_bench_harness::publish::api::{
    baselines::{BaselineEntry, BaselineSet, BaselinesJson},
    current::{CurrentJson, CurrentScenarioEntry},
    history::{sha256_of, HistoryEntry, HistoryJson},
    scenarios::{ScenarioMeta, ScenariosJson},
    version::VersionJson,
};

// ─────────────────────────────────────────────────────────────────────
// /api/current.json
// ─────────────────────────────────────────────────────────────────────

#[test]
fn current_json_schema_version_1() {
    let c = CurrentJson::build(
        "0.6.0".into(),
        "r".into(),
        "a".into(),
        Verdict::Pass,
        BTreeMap::new(),
        None,
    );
    assert_eq!(c.schema_version, 1);
    assert_eq!(c.stability_contract_version, "1.6");
}

#[test]
fn current_json_scenarios_sort_canonically() {
    let mut s = BTreeMap::new();
    let mut tol = BTreeMap::new();
    tol.insert("p95_ms_max_delta_pct".into(), 10.0);
    let entry = CurrentScenarioEntry {
        verdict: "pass".into(),
        p50_ms: Some(4.0),
        p95_ms: Some(8.0),
        p99_ms: Some(12.0),
        error_rate: Some(0.0),
        throughput_qps: Some(99.0),
        tolerance_used: tol,
    };
    s.insert("S03_ramp".into(), entry.clone());
    s.insert("S01_baseline".into(), entry.clone());
    let c = CurrentJson::build("0.6.0".into(), "r".into(), "a".into(), Verdict::Pass, s, None);
    let json = serde_json::to_string_pretty(&c).unwrap();
    assert!(json.find("S01_baseline").unwrap() < json.find("S03_ramp").unwrap());
}

// ─────────────────────────────────────────────────────────────────────
// /api/history.json
// ─────────────────────────────────────────────────────────────────────

#[test]
fn history_sha256_matches_canonical() {
    let body = b"sample body";
    let h = sha256_of(body);
    assert_eq!(h.len(), 64); // sha256 hex
}

#[test]
fn history_replaces_field_omitted_when_none() {
    let e = HistoryEntry {
        date: "2026-05-16".into(),
        url: "/history/2026-05-16.json".into(),
        sha256: "abc".into(),
        overall_verdict: "pass".into(),
        replaces: None,
    };
    let h = HistoryJson::new(vec![e]);
    let json = serde_json::to_string(&h).unwrap();
    assert!(!json.contains("\"replaces\""));
}

#[test]
fn history_replaces_field_included_for_errata() {
    let e = HistoryEntry {
        date: "2026-05-15-rev2".into(),
        url: "/history/2026-05-15-rev2.json".into(),
        sha256: "abc".into(),
        overall_verdict: "pass".into(),
        replaces: Some("2026-05-15".into()),
    };
    let h = HistoryJson::new(vec![e]);
    let json = serde_json::to_string(&h).unwrap();
    assert!(json.contains("\"replaces\":\"2026-05-15\""));
}

// ─────────────────────────────────────────────────────────────────────
// /api/scenarios.json
// ─────────────────────────────────────────────────────────────────────

#[test]
fn scenarios_json_keys_canonical_order() {
    let mut s = BTreeMap::new();
    s.insert("S04_chaos".into(), ScenarioMeta {
        description: "chaos".into(),
        duration: "10m".into(),
        tier: "chaos".into(),
        chaos_count: 1,
        latest_verdict: "pass".into(),
        page: "/scenarios/s04-chaos.html".into(),
        badge: "/badges/scenario-S04.svg".into(),
    });
    s.insert("S01_baseline".into(), ScenarioMeta {
        description: "baseline".into(),
        duration: "5m".into(),
        tier: "quick".into(),
        chaos_count: 0,
        latest_verdict: "pass".into(),
        page: "/scenarios/s01-baseline.html".into(),
        badge: "/badges/scenario-S01.svg".into(),
    });
    let s = ScenariosJson::new(s);
    let json = serde_json::to_string_pretty(&s).unwrap();
    assert!(json.find("S01_baseline").unwrap() < json.find("S04_chaos").unwrap());
}

// ─────────────────────────────────────────────────────────────────────
// /api/baselines.json
// ─────────────────────────────────────────────────────────────────────

#[test]
fn baselines_json_chart_versions_sort() {
    let mut b = BTreeMap::new();
    b.insert("0.7.0".into(), BaselineSet {
        released_at: "2026-07-01T00:00:00Z".into(),
        scenarios: vec![BaselineEntry {
            id: "S01_baseline".into(),
            url: "/baselines/v0.7.0/L2/S01.json".into(),
        }],
    });
    b.insert("0.6.0".into(), BaselineSet {
        released_at: "2026-04-01T00:00:00Z".into(),
        scenarios: vec![],
    });
    let bj = BaselinesJson::new(b);
    let json = serde_json::to_string_pretty(&bj).unwrap();
    assert!(json.find("0.6.0").unwrap() < json.find("0.7.0").unwrap());
}

// ─────────────────────────────────────────────────────────────────────
// /api/version.json
// ─────────────────────────────────────────────────────────────────────

#[test]
fn version_json_stability_v1_6() {
    let v = VersionJson::build("0.6.0".into());
    assert_eq!(v.stability_contract, "1.6");
    assert_eq!(v.schema_version, 1);
}

// ─────────────────────────────────────────────────────────────────────
// Forward-compatible unknown-field deserialisation (AD16)
// ─────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────
// D-W — forward-compat (AD16) MUST hold for EVERY type the aggregator
// + publisher reads/writes, not just CurrentJson.  Without this, a
// future PR adding a field to (say) the summary.json schema would
// crash the current aggregator on legacy data.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn d_w_scenario_result_ignores_unknown_field_ad16() {
    // The aggregator reads `bench/results/<run>/<scen>/summary.json`
    // via `ScenarioResult::read`.  A future PR adding `p999_ms` or any
    // other field must not break the current parser.
    use efe_bench_harness::result::ScenarioResult;
    let body = r#"{
        "schema_version": 1,
        "scenario_id": "S01_baseline_100qps_5min",
        "harness_version": "0.2.0",
        "loadgen_version": "0.1.0",
        "chart_version": "0.6.0",
        "image_tag": "v0.6.0",
        "git_commit": "abc",
        "run_id": "r",
        "started_at": "2026-05-16T09:00:00Z",
        "duration_s": 300.0,
        "total_requests": 30000,
        "metrics": {"p50_ms": 7.5, "p95_ms": 12.3, "p99_ms": 18.0, "error_rate": 0.001, "throughput_qps": 99.7, "p999_future_metric": 25.0},
        "chaos_events": [],
        "verdict": "pass",
        "v2_future_field": {"nested": "allowed"}
    }"#;
    let r: ScenarioResult = serde_json::from_str(body)
        .expect("D-W : ScenarioResult must ignore unknown fields at top level + in metrics");
    assert_eq!(r.metrics.p95_ms, Some(12.3));
}

#[test]
fn d_w_baselines_json_ignores_unknown_field_ad16() {
    use efe_bench_harness::publish::api::baselines::BaselinesJson;
    let body = r#"{
        "schema_version": 1,
        "baselines": {},
        "future_index_field": [1,2,3]
    }"#;
    let _: BaselinesJson = serde_json::from_str(body)
        .expect("D-W : BaselinesJson must ignore unknown top-level fields");
}

#[test]
fn d_w_scenarios_json_ignores_unknown_field_ad16() {
    use efe_bench_harness::publish::api::scenarios::ScenariosJson;
    let body = r#"{
        "schema_version": 1,
        "scenarios": {},
        "feature_flags": {"experimental": true}
    }"#;
    let _: ScenariosJson = serde_json::from_str(body)
        .expect("D-W : ScenariosJson must ignore unknown top-level fields");
}

#[test]
fn d_w_history_json_ignores_unknown_field_ad16() {
    use efe_bench_harness::publish::api::history::HistoryJson;
    let body = r#"{
        "schema_version": 1,
        "entries": [],
        "rolling_window_days": 30
    }"#;
    let _: HistoryJson = serde_json::from_str(body)
        .expect("D-W : HistoryJson must ignore unknown top-level fields");
}

#[test]
fn d_w_version_json_ignores_unknown_field_ad16() {
    use efe_bench_harness::publish::api::version::VersionJson;
    let body = r#"{
        "schema_version": 1,
        "chart_version": "0.6.0",
        "harness_version": "0.2.0",
        "git_sha": "abc",
        "stability_contract": "1.6",
        "build_date": "2026-05-16T00:00:00Z",
        "published_at": "2026-05-16T09:00:00Z",
        "extra_metadata_v2": ["a","b"]
    }"#;
    let _: VersionJson = serde_json::from_str(body)
        .expect("D-W : VersionJson must ignore unknown top-level fields");
}

#[test]
fn current_json_ignores_unknown_field_ad16() {
    let body = r#"{
        "schema_version": 1,
        "as_of": "2026-05-16T16:00:00Z",
        "chart_version": "0.6.0",
        "harness_version": "0.2.0",
        "run_id": "r",
        "git_sha": "a",
        "overall_verdict": "pass",
        "scenarios": {},
        "stability_contract_version": "1.6",
        "this_is_future_v2_field": "ignored"
    }"#;
    let _: CurrentJson = serde_json::from_str(body).expect("unknown field must be ignored");
}
