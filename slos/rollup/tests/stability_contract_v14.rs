//! PR #22 § 21.2 — STABILITY_CONTRACT v1.4 frozen-test suite.
//!
//! These tests LOCK the SLO IDs, thresholds, windows, CLI surface, and
//! file schemas as committed in STABILITY_CONTRACT v1.4 § 22.
//!
//! Any test failure here is a contract violation.  Fixing the test
//! is NOT a valid option : the offending change must be reverted, or
//! a major-version bump of STABILITY_CONTRACT must be filed.

use efe_slo_rollup::{thresholds, window::WindowSpec, SloId};

#[test]
fn slo_ids_match_v1_4_taxonomy() {
    assert_eq!(SloId::all().map(|s| s.as_str()), [
        "L2_AVAILABILITY",
        "L2_LATENCY_P95",
        "SWARM_RECOVERY_TIME",
        "REPLICATION_LAG_P99",
    ]);
}

#[test]
fn thresholds_locked_to_v1_4() {
    // § 22.2.
    assert!((thresholds::L2_AVAILABILITY_TARGET - 0.995).abs() < f64::EPSILON);
    assert!((thresholds::L2_LATENCY_P95_TARGET_S - 0.050).abs() < f64::EPSILON);
    assert!((thresholds::L2_LATENCY_P95_COMPLIANCE - 0.99).abs() < f64::EPSILON);
    assert!((thresholds::SWARM_RECOVERY_TIME_TARGET_S - 60.0).abs() < f64::EPSILON);
    assert!((thresholds::SWARM_RECOVERY_TIME_COMPLIANCE - 0.99).abs() < f64::EPSILON);
    assert_eq!(thresholds::REPLICATION_LAG_TARGET, 100);
    assert!((thresholds::REPLICATION_LAG_COMPLIANCE - 0.99).abs() < f64::EPSILON);
    assert!((thresholds::BURN_RATE_FAST - 14.4).abs() < f64::EPSILON);
    assert!((thresholds::BURN_RATE_SLOW - 6.0).abs() < f64::EPSILON);
}

#[test]
fn window_taxonomy_8_tiers() {
    // § 22.3 — exactly 8 tiers, in stable order.
    let ws = WindowSpec::all();
    assert_eq!(ws.len(), 8);
    assert_eq!(ws[0].as_str(), "1m");
    assert_eq!(ws[1].as_str(), "5m");
    assert_eq!(ws[2].as_str(), "30m");
    assert_eq!(ws[3].as_str(), "1h");
    assert_eq!(ws[4].as_str(), "6h");
    assert_eq!(ws[5].as_str(), "24h");
    assert_eq!(ws[6].as_str(), "7d");
    assert_eq!(ws[7].as_str(), "30d");
}

#[test]
fn schema_versions_v1() {
    assert_eq!(efe_slo_rollup::DAILY_PARQUET_SCHEMA_VERSION, 1);
    assert_eq!(efe_slo_rollup::MONTHLY_JSON_SCHEMA_VERSION, 1);
    assert_eq!(efe_slo_rollup::QUARTERLY_EVIDENCE_SCHEMA_VERSION, 1);
}

#[test]
fn cli_subcommand_count_is_7() {
    // § 22.5.
    use clap::CommandFactory;
    use efe_slo_rollup::cli::Cli;
    let cmd = Cli::command();
    let subs: Vec<_> = cmd.get_subcommands().map(|s| s.get_name().to_string()).collect();
    assert_eq!(subs.len(), 7, "CLI must expose exactly 7 subcommands (v1.4 § 22.5)");
    for required in ["compute", "rollup", "publish", "evidence", "list", "validate", "version"] {
        assert!(
            subs.iter().any(|s| s == required),
            "missing subcommand : {required}"
        );
    }
}

#[test]
fn cli_run_flags_locked() {
    use clap::CommandFactory;
    use efe_slo_rollup::cli::Cli;
    let cmd = Cli::command();
    let compute = cmd
        .get_subcommands()
        .find(|s| s.get_name() == "compute")
        .expect("compute subcommand present");
    let flags: Vec<_> = compute
        .get_arguments()
        .map(|a| a.get_long().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .collect();
    // Required flags (subset — locked by § 22.5 + § 22.6).
    for required in [
        "slo", "window", "offset", "chart-version", "region", "prom-url",
        "storage-root", "format", "dry-run", "target-override",
    ] {
        assert!(flags.iter().any(|f| f == required), "missing --{required}");
    }
}

#[test]
fn burn_rate_alert_yaml_carries_v1_4_label() {
    let yaml = efe_slo_rollup::burn_rate::prom_rule::emit_yaml();
    assert!(yaml.contains("stability_contract: v1.4"));
    // Exactly 8 alerts.
    assert_eq!(yaml.matches("- alert:").count(), 8);
}
