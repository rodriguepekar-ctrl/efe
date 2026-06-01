//! # PR #27 — G.11 closure — Day 4 — backfill script edge-case pinning tests
//!
//! Pin the **8 edge cases** plus the **golden file byte-identity**
//! of `scripts/backfill-tolerance-used.sh` as permanent Rust
//! integration tests.
//!
//! ## Why Day 4 needs these tests
//!
//! The mid-S2 chirurgical audit (D-AUDIT-1) surfaced an edge case
//! where the script panicked under `set -u` on an empty `.ndjson`
//! input file.  The fix was applied AND a test was run manually to
//! confirm.  But the manual verification is ephemeral : a future PR
//! that re-introduces the bug (e.g. by removing the `${#arr[@]} > 0`
//! guard) would not be caught by any test.
//!
//! Day 4 makes the 8 edge cases STRUCTURALLY non-regressable by
//! pinning them as permanent Rust tests.  Each test invokes the
//! bash script via `std::process::Command` and asserts on the exit
//! code + stdout/stderr content.  If a future PR breaks any edge
//! case, the corresponding test fails immediately in CI.
//!
//! ## Stable exit code contract (AD-D4-12, STABILITY § 24.3.6 future
//! amendment)
//!
//! - `0` — success (incl. --help, empty .ndjson file, single-file arg)
//! - `2` — bad arguments (missing arg, bad flag, --sign w/o --write)
//! - `3` — input unreadable (nonexistent path, empty dir)
//! - `4` — transformation failed (reserved, no edge case exercises it)
//! - `5` — cosign not installed (reserved, only with --sign --write)
//!
//! ## Mapping : edge case → test → exit code
//!
//! | Edge case                          | Test                                       | Exit |
//! |------------------------------------|--------------------------------------------|------|
//! | 1. missing arg                     | `missing_arg_exits_2_with_usage`           | 2    |
//! | 2. bad flag                        | `bad_flag_exits_2_with_error`              | 2    |
//! | 3. --sign without --write          | `sign_without_write_exits_2`               | 2    |
//! | 4. nonexistent path                | `nonexistent_path_exits_3`                 | 3    |
//! | 5. empty dir                       | `empty_dir_exits_3`                        | 3    |
//! | 6. empty .ndjson file              | `empty_ndjson_file_exits_0`                | 0    |
//! | 7. --help                          | `help_flag_exits_0_with_usage`             | 0    |
//! | 8. single file argument            | `single_file_argument_exits_0`             | 0    |
//! | 9. golden byte-identity            | `dry_run_byte_identical_to_golden_file`    | 0    |
//!
//! ## References
//!
//! - Architecture : `docs/architecture/EFE_A1_PHASE3_PR27_S2_DAY4_ARCHITECTURE.md`
//! - Roadmap : `docs/roadmap/EFE_A1_PHASE3_PR27_ROADMAP.md` § 3.2 day 4
//! - Script under test : `scripts/backfill-tolerance-used.sh`
//! - Golden file : `bench/fixtures/backfill_golden_diff.txt`

#![allow(clippy::needless_return)]

use std::path::PathBuf;
use std::process::{Command, Output};

use tempfile::TempDir;

// ═══════════════════════════════════════════════════════════════════
//  Workspace + script path helpers (AD-D3-2 : duplicated intentionally)
// ═══════════════════════════════════════════════════════════════════

fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let cargo = p.join("Cargo.toml");
        if cargo.exists() {
            let body = std::fs::read_to_string(&cargo).expect("Cargo.toml must read");
            if body.contains("[workspace]") {
                return p;
            }
        }
        assert!(p.pop(), "workspace root not found");
    }
}

fn script_path() -> PathBuf {
    workspace_root().join("scripts/backfill-tolerance-used.sh")
}

fn pre_g11_ndjson_dir() -> PathBuf {
    workspace_root().join("bench/fixtures/pre_g11_ndjson")
}

fn golden_file_path() -> PathBuf {
    workspace_root().join("bench/fixtures/backfill_golden_diff.txt")
}

/// Run the script with given args, capturing stdout/stderr/exit code.
fn run_script(args: &[&str]) -> Output {
    Command::new("bash")
        .arg(script_path())
        .args(args)
        .output()
        .expect("bash command must execute (is `bash` on PATH ?)")
}

/// Extract exit code from Output.  Asserts that the process exited
/// normally (not killed by signal).
fn exit_code(out: &Output) -> i32 {
    out.status
        .code()
        .expect("process must exit normally (not killed by signal)")
}

/// Pre-flight assertion : the script and golden file exist on disk.
/// If they don't, none of the edge case tests can run.
fn assert_artifacts_present() {
    let script = script_path();
    assert!(
        script.is_file(),
        "Day 4 invariant : script file must exist at {}",
        script.display()
    );
    let golden = golden_file_path();
    assert!(
        golden.is_file(),
        "Day 4 invariant : golden file must exist at {}",
        golden.display()
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 1 — Missing positional argument
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 1** : `bash backfill-tolerance-used.sh` (no positional
/// argument).
///
/// Expected : exit 2 (bad arguments) + USAGE printed to stderr.
#[test]
fn missing_arg_exits_2_with_usage() {
    assert_artifacts_present();
    let out = run_script(&[]);
    assert_eq!(exit_code(&out), 2, "missing arg must exit 2 (stable contract)");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("USAGE"),
        "missing arg : USAGE block expected on stderr, got : {stderr}"
    );
    assert!(
        stderr.contains("EXIT CODES"),
        "missing arg : EXIT CODES section expected in usage : {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 2 — Unknown flag
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 2** : `bash backfill-tolerance-used.sh --foo`.
///
/// Expected : exit 2 + error message mentioning the unknown flag.
#[test]
fn bad_flag_exits_2_with_error() {
    assert_artifacts_present();
    let out = run_script(&["--foo"]);
    assert_eq!(exit_code(&out), 2, "bad flag must exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error") && stderr.contains("--foo"),
        "bad flag : error message must mention '--foo' : {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 3 — --sign without --write
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 3** : `bash backfill-tolerance-used.sh --sign some-path`.
///
/// `--sign` requires `--write` (cosign signature needs an actual file
/// to sign, not a dry-run).  Without `--write`, exit 2.
#[test]
fn sign_without_write_exits_2() {
    assert_artifacts_present();
    let out = run_script(&["--sign", "/tmp"]);
    assert_eq!(exit_code(&out), 2, "--sign w/o --write must exit 2");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--sign requires --write"),
        "explicit error message expected, got : {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 4 — Nonexistent input path
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 4** : input path does not exist on disk.
/// Expected : exit 3 (input unreadable) + descriptive error.
#[test]
fn nonexistent_path_exits_3() {
    assert_artifacts_present();
    let out = run_script(&["/nonexistent/path/that/should/not/exist"]);
    assert_eq!(exit_code(&out), 3, "nonexistent path must exit 3");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("does not exist"),
        "nonexistent path error expected, got : {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 5 — Empty directory (no .ndjson files)
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 5** : directory exists but contains no `.ndjson` files.
/// Expected : exit 3 + "no NDJSON files to process" error.
#[test]
fn empty_dir_exits_3() {
    assert_artifacts_present();
    let tmp = TempDir::new().expect("create tempdir");
    let out = run_script(&[tmp.path().to_str().expect("UTF-8 tempdir path")]);
    assert_eq!(exit_code(&out), 3, "empty dir must exit 3");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no NDJSON files"),
        "empty dir : 'no NDJSON files' error expected, got : {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 6 — Empty .ndjson file in dir
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 6** : a directory containing an empty `.ndjson` file
/// (zero bytes).  This is the edge case that surfaced D-AUDIT-1
/// during the mid-S2 audit : `set -u` panicked on `${arr[@]}` when
/// the array was empty.  Pin : exit 0 (no panic).
#[test]
fn empty_ndjson_file_exits_0() {
    assert_artifacts_present();
    let tmp = TempDir::new().expect("create tempdir");
    let empty_file = tmp.path().join("empty.ndjson");
    std::fs::write(&empty_file, "").expect("create empty file");

    let out = run_script(&[tmp.path().to_str().expect("UTF-8 tempdir path")]);
    assert_eq!(
        exit_code(&out),
        0,
        "empty .ndjson : must exit 0 (no panic from set -u on empty array) ; \
         this pins D-AUDIT-1 closure"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("unbound variable"),
        "empty .ndjson : must not trip set -u : {stderr}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 7 — --help flag
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 7** : `bash backfill-tolerance-used.sh --help`.
/// Expected : exit 0 + full USAGE on stderr.
#[test]
fn help_flag_exits_0_with_usage() {
    assert_artifacts_present();
    let out = run_script(&["--help"]);
    assert_eq!(exit_code(&out), 0, "--help must exit 0");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("USAGE"),
        "--help : USAGE block expected, got : {stderr}"
    );
    assert!(
        stderr.contains("--dry-run"),
        "--help : --dry-run mode must be documented : {stderr}"
    );
    assert!(
        stderr.contains("--write"),
        "--help : --write mode must be documented : {stderr}"
    );
    assert!(
        stderr.contains("--sign"),
        "--help : --sign mode must be documented : {stderr}"
    );
}

/// **Edge case 7b** : `-h` (short form) must work too.
#[test]
fn help_short_flag_exits_0() {
    assert_artifacts_present();
    let out = run_script(&["-h"]);
    assert_eq!(exit_code(&out), 0, "-h must exit 0");
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 8 — Single file argument
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 8** : positional argument is a single `.ndjson` file
/// (not a directory).  The script must accept this and process the
/// single file.
#[test]
fn single_file_argument_exits_0() {
    assert_artifacts_present();
    let file = pre_g11_ndjson_dir().join("case_a_warn_null_tolerance.ndjson");
    assert!(file.is_file(), "fixture file must exist for this test");
    let out = run_script(&[file.to_str().expect("UTF-8 path")]);
    assert_eq!(exit_code(&out), 0, "single file arg must exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("=== "),
        "single file : diff structure must appear on stdout : {stdout}"
    );
    assert!(
        stdout.contains("case_a_warn_null_tolerance.ndjson"),
        "single file : the file path must appear in the diff header : {stdout}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Edge case 9 — Dry-run byte-identical to golden file
// ═══════════════════════════════════════════════════════════════════

/// **Edge case 9 — the contract test** : running the script in
/// `--dry-run` mode on the canonical fixtures must produce a
/// byte-identical output to `bench/fixtures/backfill_golden_diff.txt`.
///
/// This is the byte-stability contract that the CI workflow's
/// `backfill-dry-run` job enforces.  Pinning it as a Rust test means
/// it runs locally for every `cargo test`, not just in CI.
#[test]
fn dry_run_byte_identical_to_golden_file() {
    assert_artifacts_present();
    let out = run_script(&[
        "--dry-run",
        pre_g11_ndjson_dir()
            .to_str()
            .expect("UTF-8 pre_g11_ndjson dir path"),
    ]);
    assert_eq!(exit_code(&out), 0, "dry-run on canonical fixtures must exit 0");

    let actual = String::from_utf8(out.stdout)
        .expect("dry-run stdout must be valid UTF-8");
    let expected = std::fs::read_to_string(golden_file_path())
        .expect("golden file must read");

    if actual != expected {
        // Produce a concise diff hint to make CI failures actionable.
        let actual_lines: Vec<&str> = actual.lines().collect();
        let expected_lines: Vec<&str> = expected.lines().collect();
        let max_len = std::cmp::max(actual_lines.len(), expected_lines.len());
        let mut first_diff_line = None;
        for i in 0..max_len {
            let a = actual_lines.get(i).copied().unwrap_or("<EOF>");
            let e = expected_lines.get(i).copied().unwrap_or("<EOF>");
            if a != e {
                first_diff_line = Some((i + 1, a.to_string(), e.to_string()));
                break;
            }
        }
        panic!(
            "Day 4 contract violation : dry-run output diverges from golden.\n\n\
             actual lines  : {}\n\
             expected lines: {}\n\
             first diff at line {} :\n\
               actual   : {:?}\n\
               expected : {:?}\n\n\
             To regenerate the golden : \n\
               bash scripts/backfill-tolerance-used.sh --dry-run \\\n\
                   bench/fixtures/pre_g11_ndjson/ > bench/fixtures/backfill_golden_diff.txt\n\
             then commit the delta WITH a PR description explaining why.",
            actual_lines.len(),
            expected_lines.len(),
            first_diff_line
                .as_ref()
                .map_or_else(|| "?".to_string(), |(n, _, _)| n.to_string()),
            first_diff_line
                .as_ref()
                .map_or("", |(_, a, _)| a.as_str()),
            first_diff_line
                .as_ref()
                .map_or("", |(_, _, e)| e.as_str()),
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Meta-tests : the script itself satisfies basic invariants
// ═══════════════════════════════════════════════════════════════════

/// Meta-test : the script file is executable (`-rwxr-xr-x` or similar).
/// Required by I-S2-D4-1.
#[test]
#[cfg(unix)]
fn script_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let script = script_path();
    let mode = std::fs::metadata(&script)
        .expect("script must read")
        .permissions()
        .mode();
    assert!(
        mode & 0o111 != 0,
        "I-S2-D4-1 : script must be executable, found mode {:o}",
        mode
    );
}

/// Meta-test : `bash -n` (syntax check) on the script returns 0.
/// Required by I-S2-D4-2.
#[test]
fn script_syntax_checks_clean() {
    let out = Command::new("bash")
        .args(["-n", script_path().to_str().unwrap()])
        .output()
        .expect("bash -n must execute");
    assert_eq!(
        out.status.code(),
        Some(0),
        "I-S2-D4-2 : `bash -n` on script must exit 0 (stderr: {})",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Meta-test : the script's USAGE block declares all 5 stable exit
/// codes (0, 2, 3, 4, 5) per AD-D4-12.
#[test]
fn script_documents_all_stable_exit_codes() {
    let body = std::fs::read_to_string(script_path()).expect("script must read");
    for code in &["0", "2", "3", "4", "5"] {
        assert!(
            body.contains(&format!("    {code}")) || body.contains(&format!("exit {code}")),
            "AD-D4-12 : exit code {code} must be documented or used in the script"
        );
    }
}

/// Meta-test : the script's docstring references the architecture
/// document for Day 4.
#[test]
fn script_documents_day4_architecture_reference() {
    let body = std::fs::read_to_string(script_path()).expect("script must read");
    assert!(
        body.contains("PR #27"),
        "script header must reference PR #27"
    );
    assert!(
        body.contains("G.11"),
        "script header must reference the G.11 closure"
    );
    assert!(
        body.contains("backfill_golden_diff.txt"),
        "script header must reference the golden file"
    );
}
