//! PR #23 § 25 — golden CLI snapshot.
//!
//! Locks the public binary surface for the 5 v1.5 subcommands.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_efe-smoke")
}

fn run(args: &[&str]) -> (i32, String) {
    let out = Command::new(bin())
        .args(args)
        .env("EFE_GIT_SHA", "test-sha")
        .env("EFE_BUILD_DATE", "2026-05-16")
        .output()
        .expect("invoke binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (out.status.code().unwrap_or(-1), format!("{stdout}\n{stderr}"))
}

const REQUIRED_SUBCOMMANDS: &[&str] = &["run", "list", "validate", "gate", "version"];

#[test]
fn top_level_help_lists_5_subcommands() {
    let (_, body) = run(&["--help"]);
    for s in REQUIRED_SUBCOMMANDS {
        assert!(body.contains(s), "subcommand `{s}` missing from --help");
    }
}

#[test]
fn version_subcommand_emits_stability_tier() {
    let (rc, body) = run(&["version"]);
    assert_eq!(rc, 0);
    assert!(body.contains("efe-smoke"));
    assert!(body.contains("v1.5"));
}

#[test]
fn list_text_format_lists_9_checks() {
    let (rc, body) = run(&["list"]);
    assert_eq!(rc, 0);
    for id in [
        "S1_boot",
        "S2_l1_swarm",
        "S3_l2_meta_swarm",
        "S4_mtls_rotation",
        "S5_reload_sighup",
        "S6_metrics",
        "S7_otel",
        "S8_restart",
        "S9_shutdown",
    ] {
        assert!(body.contains(id), "missing check `{id}` in list output");
    }
}

#[test]
fn list_json_format_parses_as_array_of_9() {
    let (rc, body) = run(&["list", "--format", "json"]);
    assert_eq!(rc, 0);
    // Strip stderr (after the first `]`) by extracting the JSON array.
    let start = body.find('[').expect("must have `[`");
    let end = body.find(']').expect("must have `]`");
    let json = &body[start..=end];
    let v: serde_json::Value = serde_json::from_str(json).expect("must parse JSON");
    assert_eq!(v.as_array().unwrap().len(), 9);
}

#[test]
fn run_with_invalid_subcommand_exits_config() {
    let (rc, _) = run(&["nonexistent-subcommand"]);
    // clap exits 3 (ExitCode::Config) on unknown command.
    assert_eq!(rc, 3);
}

#[test]
fn run_without_chart_version_exits_config() {
    let (rc, _) = run(&["run", "--mock"]);
    // Missing required --chart-version.
    assert_eq!(rc, 3);
}

#[test]
fn run_with_mock_and_chart_version_succeeds() {
    let (rc, _) = run(&["run", "--chart-version", "0.6.0", "--mock"]);
    assert_eq!(rc, 0);
}

#[test]
fn run_with_unknown_check_exits_config() {
    let (rc, _) = run(&[
        "run",
        "--chart-version",
        "0.6.0",
        "--mock",
        "--check",
        "S10",
    ]);
    assert_eq!(rc, 3);
}

#[test]
fn gate_pass_report_exits_success() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("p.json");
    // Build a pass report by running mock.
    let out = Command::new(bin())
        .args([
            "run",
            "--chart-version",
            "0.6.0",
            "--mock",
            "--output",
            path.to_str().unwrap(),
        ])
        .output()
        .expect("run");
    assert!(out.status.success());
    let (rc, _) = run(&["gate", "--input", path.to_str().unwrap()]);
    assert_eq!(rc, 0);
}
