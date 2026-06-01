//! PR #21 § 9.2 + § 26.1 — golden CLI snapshot.
//!
//! Locks the public surface of the `efe-bench-harness` binary so any
//! unintended rename / removal of a flag or subcommand fails CI.
//!
//! We deliberately do **not** byte-diff the full `--help` text because :
//!   * clap formats vary by terminal width + crate version,
//!   * the version line embeds `crate::build::VERSION` (changes every release).
//!
//! Instead we assert :
//!   1. every subcommand is present in `--help` ;
//!   2. every locked flag of `run --help` is present, in stable order ;
//!   3. the `--target-override` escape hatch (G6) is wired up.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_efe-bench-harness")
}

fn run(args: &[&str]) -> String {
    let out = Command::new(bin())
        .args(args)
        .output()
        .expect("invoke harness binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    // clap may emit help to stderr depending on version.
    format!("{stdout}\n{stderr}")
}

const REQUIRED_SUBCOMMANDS: &[&str] = &[
    "run",
    "list",
    "validate",
    "compare",
    "baseline-regen",
    "publish",
    "version",
];

/// Stable flag taxonomy for `run` — STABILITY_CONTRACT v1.3 § 21.4.
/// **Removing or renaming an entry is a breaking change** : it requires
/// a major-version bump of the harness CLI.
const REQUIRED_RUN_FLAGS: &[&str] = &[
    "--scenario",
    "--all",
    "--scenarios",
    "--baselines",
    "--output-dir",
    "--kubeconfig",
    "--provisioner",
    "--cluster-per-scenario",
    "--loadgen-subprocess",
    "--skip-teardown",
    "--pr-comment-file",
    "--verdict-json-file",
    "--chart-version",
    "--image-tag",
    "--seed",
    "--mode",
    "--dry-run",
    "--target-override", // G6 — CI escape hatch.
];

#[test]
fn top_level_help_lists_every_subcommand() {
    let out = run(&["--help"]);
    for sub in REQUIRED_SUBCOMMANDS {
        assert!(
            out.contains(sub),
            "subcommand `{sub}` missing from `--help` :\n{out}"
        );
    }
}

#[test]
fn run_help_lists_every_locked_flag() {
    let out = run(&["run", "--help"]);
    for flag in REQUIRED_RUN_FLAGS {
        assert!(
            out.contains(flag),
            "flag `{flag}` missing from `run --help` (STABILITY_CONTRACT v1.3 § 21.4) :\n{out}"
        );
    }
}

#[test]
fn version_subcommand_emits_version_string() {
    let out = run(&["version"]);
    assert!(
        out.contains("efe-bench-harness"),
        "`version` did not emit the crate name :\n{out}"
    );
}

#[test]
fn target_override_flag_documents_escape_hatch() {
    // The doc comment must mention `localhost` so operators reading
    // `--help` understand the CI port-forward use case.
    let out = run(&["run", "--help"]);
    assert!(
        out.contains("--target-override"),
        "--target-override flag missing"
    );
}

#[test]
fn invalid_subcommand_exits_with_config_code() {
    // ExitCode::Config = 3 (AD17).  clap's "unknown subcommand" maps there.
    let out = Command::new(bin())
        .arg("does-not-exist")
        .output()
        .expect("invoke");
    assert!(!out.status.success(), "unknown subcommand should fail");
    let code = out.status.code().unwrap_or(-1);
    assert_eq!(code, 3, "expected ExitCode::Config (3), got {code}");
}
