//! PR #23 § 17 — STABILITY_CONTRACT v1.5 § 23 frozen-test suite.
//!
//! These tests LOCK the 9 check IDs, 9 budgets, 14 metric families, 5 CLI
//! subcommands, result JSON schema, ClusterProvisioner trait, and 6
//! ExitCodes as committed in v1.5.  Any test failure here is a contract
//! violation — fixing the test is **not** a valid option ; either the
//! offending change must be reverted, or a major-version bump of
//! STABILITY_CONTRACT must be filed.

use efe_smoke::checks::SmokeCheckId;
use efe_smoke::{
    budgets, SMOKE_CHECK_IDS, SMOKE_RESULT_SCHEMA_VERSION, STABLE_METRIC_FAMILIES,
};

// ─────────────────────────────────────────────────────────────────────
// § 23.1 — 9 check IDs (closed taxonomy)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn nine_check_ids_in_stable_order() {
    assert_eq!(
        SMOKE_CHECK_IDS,
        &[
            "S1_boot",
            "S2_l1_swarm",
            "S3_l2_meta_swarm",
            "S4_mtls_rotation",
            "S5_reload_sighup",
            "S6_metrics",
            "S7_otel",
            "S8_restart",
            "S9_shutdown",
        ]
    );
}

#[test]
fn smoke_check_id_enum_matches_string_form() {
    let ids = SmokeCheckId::all();
    for (i, expected) in SMOKE_CHECK_IDS.iter().enumerate() {
        assert_eq!(ids[i].as_str(), *expected);
    }
}

// ─────────────────────────────────────────────────────────────────────
// § 23.2 — 9 budgets (sum = 180s)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn per_check_budgets_frozen() {
    assert_eq!(budgets::S1_BOOT.as_secs(), 30);
    assert_eq!(budgets::S2_L1_SWARM.as_secs(), 15);
    assert_eq!(budgets::S3_L2_META_SWARM.as_secs(), 20);
    assert_eq!(budgets::S4_MTLS_ROTATION.as_secs(), 25);
    assert_eq!(budgets::S5_RELOAD_SIGHUP.as_secs(), 20);
    assert_eq!(budgets::S6_METRICS.as_secs(), 10);
    assert_eq!(budgets::S7_OTEL.as_secs(), 15);
    assert_eq!(budgets::S8_RESTART.as_secs(), 30);
    assert_eq!(budgets::S9_SHUTDOWN.as_secs(), 15);
}

#[test]
fn budgets_sum_to_180_seconds() {
    assert_eq!(budgets::TOTAL.as_secs(), 180);
}

#[test]
fn global_hard_cap_is_240_seconds() {
    assert_eq!(budgets::GLOBAL_HARD_CAP.as_secs(), 240);
}

// ─────────────────────────────────────────────────────────────────────
// § 23.3 — 14 STABLE metric families
// ─────────────────────────────────────────────────────────────────────

#[test]
fn fourteen_stable_metric_families_frozen() {
    assert_eq!(STABLE_METRIC_FAMILIES.len(), 14);
    // Order matters — operators index into this for diagnostics.
    assert_eq!(STABLE_METRIC_FAMILIES[0], "efe_l2_request_total");
    assert_eq!(STABLE_METRIC_FAMILIES[1], "efe_l2_request_duration_seconds_bucket");
    assert_eq!(STABLE_METRIC_FAMILIES[13], "efe_swarm_size_current");
}

#[test]
fn pr17_anchor_families_present() {
    // 4 critical families from PR #17 STABILITY v1.0 § 4.
    for f in [
        "efe_l2_request_total",
        "efe_l2_request_duration_seconds_bucket",
        "efe_replication_lag_bucket",
        "efe_l2_tls_handshakes_total",
    ] {
        assert!(
            STABLE_METRIC_FAMILIES.contains(&f),
            "PR #17 STABLE family '{f}' missing from PR #23 list"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// § 23.4 — 5 CLI subcommands
// ─────────────────────────────────────────────────────────────────────

#[test]
fn five_cli_subcommands_frozen() {
    use clap::CommandFactory;
    use efe_smoke::cli::Cli;
    let cmd = Cli::command();
    let subs: Vec<_> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    assert_eq!(subs.len(), 5, "exactly 5 subcommands (v1.5 § 23.4)");
    for required in ["run", "list", "validate", "gate", "version"] {
        assert!(
            subs.iter().any(|s| s == required),
            "missing subcommand `{required}` (v1.5 § 23.4)"
        );
    }
}

#[test]
fn run_help_locks_critical_flags() {
    use clap::CommandFactory;
    use efe_smoke::cli::Cli;
    let cmd = Cli::command();
    let run = cmd
        .get_subcommands()
        .find(|s| s.get_name() == "run")
        .expect("run subcommand present");
    let flags: Vec<_> = run
        .get_arguments()
        .map(|a| a.get_long().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for required in [
        "chart-version",
        "provisioner",
        "kubeconfig",
        "check",
        "keep-cluster",
        "keep-cluster-on-failure",
        "per-check-timeout",
        "global-timeout",
        "output",
        "pr-comment-file",
        "badge-file",
        "redact-logs",
        "mock",
        "namespace-prefix",
    ] {
        assert!(
            flags.iter().any(|f| f == required),
            "missing flag `--{required}` (v1.5 § 23.4)"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// § 23.5 — result JSON schema v1
// ─────────────────────────────────────────────────────────────────────

#[test]
fn smoke_result_schema_version_is_1() {
    assert_eq!(SMOKE_RESULT_SCHEMA_VERSION, 1);
}

// ─────────────────────────────────────────────────────────────────────
// § 23.6 — ClusterProvisioner trait surface
// ─────────────────────────────────────────────────────────────────────

#[test]
fn cluster_provisioner_kind_taxonomy_3() {
    use efe_cluster_provisioner::ProvisionerKind;
    assert_eq!(ProvisionerKind::Kind.as_str(), "kind");
    assert_eq!(ProvisionerKind::K3s.as_str(), "k3s");
    assert_eq!(ProvisionerKind::External.as_str(), "external");
}

// ─────────────────────────────────────────────────────────────────────
// § 23.7 — 6 ExitCodes
// ─────────────────────────────────────────────────────────────────────

#[test]
fn six_exit_codes_frozen() {
    use efe_smoke::error::ExitCode;
    assert_eq!(ExitCode::Success.as_i32(), 0);
    assert_eq!(ExitCode::SmokeFail.as_i32(), 1);
    assert_eq!(ExitCode::Cluster.as_i32(), 2);
    assert_eq!(ExitCode::Config.as_i32(), 3);
    assert_eq!(ExitCode::Interrupted.as_i32(), 4);
    assert_eq!(ExitCode::Internal.as_i32(), 5);
}
