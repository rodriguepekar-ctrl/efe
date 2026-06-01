//! PR #22 § 25 — golden CLI snapshot (subcommand surface + flag taxonomy).

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_efe-slo-rollup")
}

fn run(args: &[&str]) -> String {
    let out = Command::new(bin()).args(args).output().expect("invoke");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    format!("{stdout}\n{stderr}")
}

const REQUIRED_SUBCOMMANDS: &[&str] = &[
    "compute",
    "rollup",
    "publish",
    "evidence",
    "list",
    "validate",
    "version",
];

const REQUIRED_COMPUTE_FLAGS: &[&str] = &[
    "--slo",
    "--window",
    "--offset",
    "--backfill",
    "--release-backfill-confirmed",
    "--prom-url",
    "--target-override",
    "--storage-root",
    "--chart-version",
    "--region",
    "--format",
    "--dry-run",
];

#[test]
fn top_level_help_lists_every_subcommand() {
    let out = run(&["--help"]);
    for s in REQUIRED_SUBCOMMANDS {
        assert!(out.contains(s), "missing subcommand `{s}` :\n{out}");
    }
}

#[test]
fn compute_help_locks_flags_v1_4() {
    let out = run(&["compute", "--help"]);
    for f in REQUIRED_COMPUTE_FLAGS {
        assert!(out.contains(f), "missing flag `{f}` (v1.4 § 22.5) :\n{out}");
    }
}

#[test]
fn version_subcommand_emits_crate_name() {
    let out = run(&["version"]);
    assert!(out.contains("efe-slo-rollup"));
    assert!(out.contains("v1.4"));
}

#[test]
fn unknown_subcommand_exits_config() {
    let out = Command::new(bin())
        .arg("does-not-exist")
        .output()
        .expect("invoke");
    assert!(!out.status.success());
    assert_eq!(out.status.code().unwrap_or(-1), 3);
}

#[test]
fn rollup_t3_requires_cosign_or_disabled() {
    let out = Command::new(bin())
        .args(["rollup", "--stage", "t3", "--period", "Q2-2026", "--chart-version", "0.6.0"])
        .output()
        .expect("invoke");
    assert!(!out.status.success(), "t3 without cosign must fail");
}
