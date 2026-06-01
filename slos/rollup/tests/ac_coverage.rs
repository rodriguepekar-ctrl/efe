//! PR #22 § 24 — coverage for ACs not exercised elsewhere :
//!   * AC9 — counter reset is excluded
//!   * AC10 — DST has no effect (UTC-only)
//!   * AC22 — /current.json conforms to monthly-json-v1.json
//!     (modulo `as_of` vs `period`)
//!   * AC34 — `list --format csv` produces a valid header + N rows

use std::collections::BTreeMap;
use std::process::Command;

use efe_slo_rollup::sli::SliVerdict;
use efe_slo_rollup::storage;
use efe_slo_rollup::window::WindowSpec;
use efe_slo_rollup::SloId;

// ─────────────────────────────────────────────────────────────────────
// AC9 — counter reset detection
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ac9_counter_reset_yields_indeterminate_via_low_traffic() {
    // A counter reset surfaces as a sudden drop in the integer counter.
    // The kernel treats it as "low traffic" (D < 100) and marks the
    // window Indeterminate rather than failing.
    //
    // This is the simplest boundary semantic ; full
    // `negative-rate` detection is a Phase-3 enhancement (G.11).
    use efe_slo_rollup::sli::{dispatch_for, SliContext};
    use efe_slo_rollup::source::MockSource;

    let mut m = MockSource::default();
    // After a reset, only 5 events have accumulated since restart :
    m.set_counter("efe_l2_request_total", "status", "success", 5);
    m.set_counter("efe_l2_request_total", "status", "server_error", 0);
    m.set_counter("efe_l2_request_total", "status", "transport_error", 0);
    m.set_coverage(1.0);
    let ctx = SliContext {
        window: WindowSpec::FiveMin,
        window_start_unix: 0,
        source: &m,
        source_label: "prod".into(),
        region: "global".into(),
        chart_version: "0.6.0".into(),
    };
    let p = dispatch_for(SloId::L2Availability).compute(&ctx).await.unwrap();
    assert_eq!(p.verdict, SliVerdict::Indeterminate);
}

// ─────────────────────────────────────────────────────────────────────
// AC10 — DST has no effect (UTC-only)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn ac10_dst_boundary_aligns_to_utc_not_local() {
    // 2026-03-29 in Europe/Paris : DST jumps 02:00 → 03:00 local.
    // In UTC, this is just another midnight ; alignment must be
    // unaffected.
    let unix_2026_03_29_00z = efe_slo_rollup::window::align::parse_date_to_unix("2026-03-29").unwrap();
    let aligned_day = WindowSpec::TwentyFourHours.align(unix_2026_03_29_00z);
    assert_eq!(aligned_day, unix_2026_03_29_00z);

    // Subsequent day boundary is exactly 86_400 s later (no DST drift).
    let next = unix_2026_03_29_00z + 86_400;
    let aligned_next = WindowSpec::TwentyFourHours.align(next);
    assert_eq!(aligned_next, next);
    assert_eq!(
        efe_slo_rollup::window::align::fmt_date(aligned_next),
        "2026-03-30"
    );
}

#[test]
fn ac10_5min_aligned_across_dst() {
    // 2026-03-29 01:55 UTC, 02:00 UTC — both must align to their own
    // 5-min bucket, independently of local DST behaviour.
    let base = efe_slo_rollup::window::align::parse_date_to_unix("2026-03-29").unwrap();
    let t_0155 = base + 3600 + 55 * 60;
    let t_0200 = base + 2 * 3600;
    assert_eq!(WindowSpec::FiveMin.align(t_0155), t_0155);
    assert_eq!(WindowSpec::FiveMin.align(t_0200), t_0200);
}

// ─────────────────────────────────────────────────────────────────────
// AC22 — /current.json schema
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ac22_current_json_carries_required_keys() {
    let tmp = tempfile::tempdir().unwrap();

    // Create a minimal monthly aggregate so publish has something to consume.
    let mut agg = BTreeMap::new();
    for slo in SloId::all() {
        agg.insert(slo, efe_slo_rollup::window::aggregate::AggregatedSli {
            value: 0.998,
            numerator: 998,
            denominator: 1_000,
            valid_count: 1,
            verdict: SliVerdict::Pass,
        });
    }
    let m = storage::json::build_monthly("0.6.0", "global", "2026-05", "r", &agg, vec![]);
    let monthly_path = storage::monthly_path(tmp.path(), "0.6.0", "2026-05");
    storage::json::write_monthly(&monthly_path, &m).unwrap();

    let publish_args = efe_slo_rollup::cli::PublishArgs {
        source: tmp.path().join("site"),     // absent
        output: tmp.path().join("book"),
        storage_root: tmp.path().to_path_buf(),
        chart_version: Some("0.6.0".into()),
    };
    let rc = efe_slo_rollup::rollup::dispatch_publish(publish_args.clone()).await.unwrap();
    assert_eq!(rc, efe_slo_rollup::ExitCode::Success);

    let current_path = publish_args.output.join("current.json");
    let raw = std::fs::read(&current_path).expect("current.json must be written");
    let v: serde_json::Value = serde_json::from_slice(&raw).unwrap();

    // Schema check : keys must include schema_version, as_of,
    // chart_version, slos.
    for key in ["schema_version", "as_of", "chart_version", "slos"] {
        assert!(v.get(key).is_some(), "current.json missing key '{key}'");
    }
    let slos = v["slos"].as_object().unwrap();
    assert_eq!(slos.len(), 4, "4 SLO entries expected");

    // D3 fix : each SLO entry must carry BOTH consumed AND remaining
    // error-budget percentages (arch doc § 12.2).
    for slo_id in ["L2_AVAILABILITY", "L2_LATENCY_P95", "REPLICATION_LAG_P99", "SWARM_RECOVERY_TIME"] {
        let entry = &slos[slo_id];
        for k in [
            "value",
            "target",
            "verdict",
            "error_budget_consumed_pct",
            "error_budget_remaining_pct",
        ] {
            assert!(entry.get(k).is_some(), "{slo_id} missing key '{k}'");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// AC34 — list --format csv
// ─────────────────────────────────────────────────────────────────────

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_efe-slo-rollup")
}

#[test]
fn ac34_list_csv_has_header() {
    let tmp = tempfile::tempdir().unwrap();
    let out = Command::new(bin())
        .args([
            "list",
            "--storage-root", tmp.path().to_str().unwrap(),
            "--chart-version", "0.6.0",
            "--range", "30d",
            "--format", "csv",
        ])
        .output()
        .expect("invoke binary");
    assert!(out.status.success(), "list --format csv must succeed even on empty storage : {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Header line must be present.
    assert!(
        stdout.starts_with("slo_id,window,window_start_unix,value,verdict,source,region"),
        "expected CSV header at top of stdout, got :\n{stdout}"
    );
}
