//! PR #20 § 33.3 — CLI golden snapshot test.
//!
//! Compares `--help` output to a committed golden file.  CI updates the
//! golden via `cargo run -p efe-meta-swarm-loadgen -- --help > tools/loadgen/tests/data/golden-cli.txt`.

use std::process::Command;

/// Cargo wraps `--help` output ; we just verify a couple of stable
/// invariants (subcommands present, version flag works).  Full byte-
/// for-byte golden diffing happens in the CI workflow Job `cli-golden`.

#[test]
fn help_lists_run_subcommand() {
    let out = Command::new(env!("CARGO_BIN_EXE_efe-meta-swarm-loadgen"))
        .arg("--help")
        .output()
        .expect("loadgen binary present");
    assert!(out.status.success(), "--help non-zero exit");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("run"));
    assert!(s.contains("validate"));
    assert!(s.contains("replay"));
    assert!(s.contains("aggregate"));
    assert!(s.contains("compare"));
}

#[test]
fn version_subcommand_prints_version() {
    let out = Command::new(env!("CARGO_BIN_EXE_efe-meta-swarm-loadgen"))
        .arg("version")
        .output()
        .expect("loadgen binary present");
    assert!(out.status.success());
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("efe-meta-swarm-loadgen"));
    assert!(s.contains("git_sha"));
}

#[test]
fn run_without_target_exits_with_config_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_efe-meta-swarm-loadgen"))
        .args(["run", "--mode", "dev"])
        .output()
        .expect("loadgen binary present");
    // Exit code 3 = Config error (AD17).
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn production_mode_without_confirm_exits_3() {
    let out = Command::new(env!("CARGO_BIN_EXE_efe-meta-swarm-loadgen"))
        .args([
            "run",
            "--target",
            "grpc://localhost:50051",
            "--mode",
            "production",
        ])
        .output()
        .expect("loadgen binary present");
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn dry_run_exits_zero_with_valid_config() {
    let out = Command::new(env!("CARGO_BIN_EXE_efe-meta-swarm-loadgen"))
        .args([
            "run",
            "--target",
            "grpc://localhost:50051",
            "--dry-run",
            "--mode",
            "dev",
        ])
        .output()
        .expect("loadgen binary present");
    assert_eq!(out.status.code(), Some(0));
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("dry-run config OK"));
}

#[test]
fn qps_above_safe_cap_without_i_am_sure_exits_3() {
    let out = Command::new(env!("CARGO_BIN_EXE_efe-meta-swarm-loadgen"))
        .args([
            "run",
            "--target",
            "grpc://localhost:50051",
            "--qps",
            "5000",
            "--mode",
            "dev",
        ])
        .output()
        .expect("loadgen binary present");
    assert_eq!(out.status.code(), Some(3));
}
