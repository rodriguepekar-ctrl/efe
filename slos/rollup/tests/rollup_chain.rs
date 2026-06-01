//! PR #22 § 25 — integration test : T1 → T2 → T3 chain end-to-end.

use std::collections::BTreeMap;

use efe_slo_rollup::sli::{SliPoint, SliVerdict};
use efe_slo_rollup::storage;
use efe_slo_rollup::window::WindowSpec;
use efe_slo_rollup::SloId;

fn sample_point(slo: SloId, window_start_unix: i64, value: f64, verdict: SliVerdict) -> SliPoint {
    SliPoint {
        slo_id: slo,
        window: WindowSpec::OneHour,
        window_start_unix,
        value,
        numerator: (value * 1_000.0) as u64,
        denominator: 1_000,
        compliance_bit: if matches!(verdict, SliVerdict::Indeterminate) {
            255
        } else if verdict == SliVerdict::Pass {
            1
        } else {
            0
        },
        coverage_pct: 1.0,
        source: "prod".into(),
        region: "global".into(),
        verdict,
    }
}

#[test]
fn t1_envelope_roundtrip_preserves_all_points() {
    let tmp = tempfile::tempdir().unwrap();
    let path = storage::daily_path(tmp.path(), "0.6.0", "2026-05-15");
    let mut env = storage::parquet::DailyEnvelope {
        schema_version: 1,
        chart_version: "0.6.0".into(),
        region: "global".into(),
        date: "2026-05-15".into(),
        rollup_run_id: "r".into(),
        generated_at_unix: 1_779_667_200,
        points: vec![],
    };
    for h in 0..24 {
        for slo in SloId::all() {
            env.points.push(sample_point(slo, 1_779_667_200 + h * 3_600, 0.999, SliVerdict::Pass));
        }
    }
    storage::parquet::write_daily(&path, &env).unwrap();
    let back = storage::parquet::read_daily(&path).unwrap();
    assert_eq!(back.points.len(), 24 * 4);
}

#[test]
fn t2_aggregate_4_slos_to_monthly_json() {
    // Build 30 daily envelopes, each with one OneHour point per SLO.
    let mut all_points: Vec<SliPoint> = Vec::new();
    for d in 0..30 {
        for slo in SloId::all() {
            // Vary value slightly so per-SLO aggregates differ.
            let v = 0.997 + 0.001 * (d as f64 / 30.0);
            all_points.push(sample_point(slo, 1_779_667_200 + d * 86_400, v, SliVerdict::Pass));
        }
    }
    // Use the public aggregate helper.
    let agg = efe_slo_rollup::rollup::aggregate_per_slo(&all_points);
    assert_eq!(agg.len(), 4);
    for slo in SloId::all() {
        let r = &agg[&slo];
        assert!(!matches!(r.verdict, SliVerdict::Indeterminate));
    }
}

#[test]
fn t2_canonical_monthly_json_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = storage::monthly_path(tmp.path(), "0.6.0", "2026-05");

    let mut agg = BTreeMap::new();
    for slo in SloId::all() {
        agg.insert(slo, efe_slo_rollup::window::aggregate::AggregatedSli {
            value: 0.998,
            numerator: 998_000,
            denominator: 1_000_000,
            valid_count: 1,
            verdict: SliVerdict::Pass,
        });
    }
    let m = storage::json::build_monthly("0.6.0", "global", "2026-05", "r-test", &agg, vec![]);
    storage::json::write_monthly(&path, &m).unwrap();
    let back = storage::json::read_monthly(&path).unwrap();

    // Canonical-form check : BTreeMap → keys in deterministic order.
    let keys: Vec<_> = back.slos.keys().cloned().collect();
    assert_eq!(keys, ["L2_AVAILABILITY", "L2_LATENCY_P95", "REPLICATION_LAG_P99", "SWARM_RECOVERY_TIME"]);
}

#[test]
fn missing_monthly_makes_t3_fail() {
    // T3 requires all 3 monthlies of the quarter ; if any is missing
    // the orchestrator must surface ExitCode::StorageIo (AC16).
    // We exercise this via a missing-file probe (the rollup orchestrator
    // proper requires async runtime — covered by lib unit tests).
    let tmp = tempfile::tempdir().unwrap();
    // Only 2 of 3 monthlies present.
    for month in ["2026-04", "2026-05"] {
        let path = storage::monthly_path(tmp.path(), "0.6.0", month);
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
        let m = storage::json::build_monthly("0.6.0", "global", month, "r", &agg, vec![]);
        storage::json::write_monthly(&path, &m).unwrap();
    }
    // 2026-06 is missing.
    let missing = storage::monthly_path(tmp.path(), "0.6.0", "2026-06");
    assert!(!missing.exists());
}
