//! # PR #27 — G.11 closure — Day 5 — CI workflow YAML structural pinning
//!
//! Pin the **12 structural invariants** of
//! `.github/workflows/pr27-tolerance-used.yml` as permanent Rust
//! integration tests.
//!
//! ## Why Day 5 needs these tests
//!
//! The workflow YAML is the bridge between the code (Day 1-3) and the
//! deployment (post-merge S4).  If its structure drifts silently — a
//! job loses its timeout, the gate loses a dependency, the name field
//! loses its quotes (D-AUDIT-2), the path filters miss a new fixture —
//! the protection chain is compromised but no test catches it locally.
//!
//! Day 5 makes the workflow structure STRUCTURALLY non-regressable by
//! pinning 12 invariants as permanent Rust tests.  Each test parses
//! the workflow YAML via `serde_yaml` and asserts on a specific
//! structural property.  If a future PR breaks the structure, the
//! corresponding test fails immediately in `cargo test` — before the
//! workflow is even invoked by GitHub Actions.
//!
//! ## Mapping : architecture decision → test
//!
//! | Architecture decision (Day 5)        | Test                                            |
//! |--------------------------------------|-------------------------------------------------|
//! | AD-D5-1 : 5 jobs                     | `workflow_has_5_jobs`                           |
//! | AD-D5-2 : concurrency + cancel       | `workflow_concurrency_cancels_in_progress`      |
//! | AD-D5-3 : path filters cover surface | `workflow_path_filters_cover_pr27_surface`      |
//! | AD-D5-4 : name quoted                | `workflow_name_field_is_quoted_not_truncated`   |
//! | AD-D5-5 : cargo cache                | `workflow_uses_cargo_cache_keyed_on_lockfiles`  |
//! | AD-D5-6 : backfill dry-run + diff    | `workflow_backfill_dry_run_diffs_against_golden`|
//! | AD-D5-7 : audit-reader deferral      | `workflow_documents_audit_reader_deferral`      |
//! | AD-D5-8 : pr27-gate aggregator       | `workflow_pr27_gate_depends_on_4_jobs`          |
//! | AD-D5-9 : env vars                   | `workflow_env_vars_set`                         |
//! | AD-D5-10 : timeout per job           | `workflow_each_job_has_timeout_minutes`         |
//! | AD-D5-11 : triggers PR + push main   | `workflow_triggers_on_pr_and_push_main`         |
//! | AD-D5-12 : workflow is contractual   | `workflow_yaml_parses_cleanly` (base)           |
//!
//! ## References
//!
//! - Architecture : Day 5 section of `EFE_A1_PHASE3_PR27_S2_ARCHITECTURE.md`
//! - Roadmap : `docs/roadmap/EFE_A1_PHASE3_PR27_ROADMAP.md` § 3.2 day 5
//! - Workflow under test : `.github/workflows/pr27-tolerance-used.yml`

#![allow(clippy::needless_return)]

use std::path::PathBuf;

use serde_yaml::Value;

// ═══════════════════════════════════════════════════════════════════
//  Workspace + workflow path helpers
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

fn workflow_path() -> PathBuf {
    workspace_root().join(".github/workflows/pr27-tolerance-used.yml")
}

fn load_workflow() -> Value {
    let body = std::fs::read_to_string(workflow_path())
        .expect("workflow YAML must read");
    serde_yaml::from_str(&body).expect("workflow YAML must parse")
}

fn load_workflow_raw() -> String {
    std::fs::read_to_string(workflow_path()).expect("workflow YAML must read")
}

fn jobs(workflow: &Value) -> &serde_yaml::Mapping {
    workflow["jobs"]
        .as_mapping()
        .expect("workflow.jobs must be a mapping")
}

// ═══════════════════════════════════════════════════════════════════
//  Base test : workflow parses cleanly + file exists
// ═══════════════════════════════════════════════════════════════════

/// **Base invariant (AD-D5-12)** : the workflow YAML file exists and
/// parses as valid YAML.  If this fails, all other tests fail by
/// cascade — diagnose this one first.
#[test]
fn workflow_yaml_parses_cleanly() {
    let path = workflow_path();
    assert!(
        path.is_file(),
        "AD-D5-12 : workflow file must exist at {}",
        path.display()
    );
    let _: Value = serde_yaml::from_str(
        &std::fs::read_to_string(&path).expect("read"),
    )
    .expect("workflow YAML must parse as valid YAML");
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-4 — name field quoted (pin D-AUDIT-2 fix)
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-4 pin (also D-AUDIT-2 fix permanent pin)** : the `name:`
/// field must be quoted so YAML doesn't truncate at the `#` character.
///
/// Without quoting, `name: PR #27 — tolerance_used validation` parses
/// as `name: "PR"` (everything after `#` is treated as a comment).
/// The GitHub Actions UI would then show just "PR" — useless.
#[test]
fn workflow_name_field_is_quoted_not_truncated() {
    let w = load_workflow();
    let name = w["name"]
        .as_str()
        .expect("workflow.name must be a string");
    assert_eq!(
        name,
        "PR #27 — tolerance_used validation",
        "AD-D5-4 / D-AUDIT-2 : name field must be the full quoted string. \
         If this fails, the name field has lost its quotes and YAML \
         truncates at the '#' character."
    );
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-1 — 5 jobs declared
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-1 pin** : the workflow declares exactly 5 jobs in expected
/// names.
#[test]
fn workflow_has_5_jobs() {
    let w = load_workflow();
    let jobs = jobs(&w);
    let mut names: Vec<&str> = jobs
        .keys()
        .filter_map(serde_yaml::Value::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "audit-reader-round-trip",
            "backfill-dry-run",
            "freeze-tests",
            "pr27-gate",
            "unit-tests",
        ],
        "AD-D5-1 : workflow must declare exactly 5 jobs with expected names. \
         Adding a job means updating both the workflow AND this test."
    );
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-2 — concurrency cancels in progress
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-2 pin** : concurrency block with cancel-in-progress.
#[test]
fn workflow_concurrency_cancels_in_progress() {
    let w = load_workflow();
    let concurrency = &w["concurrency"];
    assert!(
        !concurrency.is_null(),
        "AD-D5-2 : concurrency block must be present"
    );
    let group = concurrency["group"]
        .as_str()
        .expect("concurrency.group must be a string");
    assert!(
        group.contains("pr27-tolerance-used"),
        "AD-D5-2 : concurrency group must namespace pr27-tolerance-used, got '{group}'"
    );
    assert!(
        group.contains("github.ref"),
        "AD-D5-2 : concurrency group must include github.ref for per-branch isolation"
    );
    let cancel = concurrency["cancel-in-progress"]
        .as_bool()
        .expect("concurrency.cancel-in-progress must be a bool");
    assert!(cancel, "AD-D5-2 : cancel-in-progress must be true");
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-3 — path filters cover the PR #27 surface
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-3 pin** : the path filters of `on.pull_request.paths`
/// cover the full PR #27 surface (Rust sources, fixtures, scripts,
/// contracts, the workflow itself).
#[test]
fn workflow_path_filters_cover_pr27_surface() {
    let raw = load_workflow_raw();
    // Use raw text scan rather than YAML parsing to avoid the `on:` /
    // `true:` ambiguity (YAML parses `on` as a boolean true in some
    // versions, which confuses serde_yaml).
    let required_paths = [
        "bench/harness/src/compare/effective_tolerance.rs",
        "bench/harness/src/compare/mod.rs",
        "bench/harness/tests/g11_round_trip.rs",
        "bench/harness/tests/g11_tolerance_used_populated.rs",
        "bench/harness/tests/stability_contract_v17.rs",
        "bench/fixtures/post_g11_ndjson",
        "bench/fixtures/pre_g11_ndjson",
        "bench/fixtures/scenarios/G11_synthetic.yaml",
        "bench/fixtures/backfill_golden_diff.txt",
        "scripts/backfill-tolerance-used.sh",
        "docs/STABILITY_CONTRACT.md",
        ".github/workflows/pr27-tolerance-used.yml",
    ];
    for path in &required_paths {
        assert!(
            raw.contains(path),
            "AD-D5-3 : path filter must include '{path}' so the workflow \
             triggers when this file changes. Currently missing."
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-5 — cargo cache keyed on lockfiles
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-5 pin** : at least one job uses `actions/cache` with a
/// key based on `Cargo.lock` and `Cargo.toml`.
#[test]
fn workflow_uses_cargo_cache_keyed_on_lockfiles() {
    let raw = load_workflow_raw();
    assert!(
        raw.contains("actions/cache@v4") || raw.contains("actions/cache@v3"),
        "AD-D5-5 : workflow must use actions/cache for cargo registry/git/target"
    );
    assert!(
        raw.contains("Cargo.lock") && raw.contains("Cargo.toml"),
        "AD-D5-5 : cache key must reference Cargo.lock and Cargo.toml"
    );
    assert!(
        raw.contains("hashFiles"),
        "AD-D5-5 : cache key must use hashFiles() for invalidation"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-6 — backfill-dry-run diffs against golden
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-6 pin** : the `backfill-dry-run` job runs the script in
/// dry-run mode and diffs against the canonical golden file.
#[test]
fn workflow_backfill_dry_run_diffs_against_golden() {
    let w = load_workflow();
    let jobs = jobs(&w);
    let job = &jobs[&Value::String("backfill-dry-run".to_string())];
    assert!(!job.is_null(), "AD-D5-6 : backfill-dry-run job must exist");
    // Convert the job back to YAML text for content checks (since
    // step bodies are nested arbitrarily deep).
    let job_yaml = serde_yaml::to_string(job)
        .expect("re-serialise backfill-dry-run job");
    assert!(
        job_yaml.contains("scripts/backfill-tolerance-used.sh"),
        "AD-D5-6 : backfill job must invoke the script"
    );
    assert!(
        job_yaml.contains("--dry-run"),
        "AD-D5-6 : backfill job must use --dry-run mode"
    );
    assert!(
        job_yaml.contains("bench/fixtures/backfill_golden_diff.txt"),
        "AD-D5-6 : backfill job must compare against golden file"
    );
    assert!(
        job_yaml.contains("diff"),
        "AD-D5-6 : backfill job must use diff for comparison"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-7 — audit-reader CLI deferral documented
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-7 pin** : the `audit-reader-round-trip` job documents the
/// deferral of the audit-reader CLI binary to PR #27.1.
#[test]
fn workflow_documents_audit_reader_deferral() {
    let raw = load_workflow_raw();
    // The job must mention PR #27.1 (the future PR) AND mention that
    // the audit-reader CLI is deferred.
    assert!(
        raw.contains("PR #27.1") || raw.contains("PR 27.1"),
        "AD-D5-7 : workflow must reference PR #27.1 as the future home of the audit-reader CLI"
    );
    assert!(
        raw.contains("audit-reader")
            || raw.contains("audit_reader_golden_explain.txt"),
        "AD-D5-7 : workflow must reference the audit-reader contract"
    );
    assert!(
        raw.contains("deferred")
            || raw.contains("TODO")
            || raw.contains("PR #27.1)"),
        "AD-D5-7 : workflow must document the deferral explicitly"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-8 — pr27-gate depends on the 4 verification jobs
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-8 pin** : the `pr27-gate` job declares `needs:` covering
/// the 4 verification jobs, so it functions as the single required
/// check for branch protection.
#[test]
fn workflow_pr27_gate_depends_on_4_jobs() {
    let w = load_workflow();
    let jobs = jobs(&w);
    let gate = &jobs[&Value::String("pr27-gate".to_string())];
    assert!(!gate.is_null(), "AD-D5-8 : pr27-gate job must exist");

    let needs = gate["needs"]
        .as_sequence()
        .expect("AD-D5-8 : pr27-gate.needs must be a sequence");
    let needs_set: std::collections::HashSet<&str> = needs
        .iter()
        .filter_map(serde_yaml::Value::as_str)
        .collect();

    for required in &[
        "unit-tests",
        "freeze-tests",
        "backfill-dry-run",
        "audit-reader-round-trip",
    ] {
        assert!(
            needs_set.contains(required),
            "AD-D5-8 : pr27-gate.needs must include '{required}'. \
             Currently : {needs_set:?}"
        );
    }
    assert_eq!(
        needs_set.len(),
        4,
        "AD-D5-8 : pr27-gate.needs must be exactly the 4 verification jobs (no more, no less)"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-9 — env vars set
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-9 pin** : top-level env block sets CARGO_TERM_COLOR and
/// RUST_BACKTRACE for debugging visibility.
#[test]
fn workflow_env_vars_set() {
    let w = load_workflow();
    let env = &w["env"];
    assert!(!env.is_null(), "AD-D5-9 : top-level env block must be present");
    assert_eq!(
        env["CARGO_TERM_COLOR"].as_str(),
        Some("always"),
        "AD-D5-9 : CARGO_TERM_COLOR must be 'always'"
    );
    let backtrace = &env["RUST_BACKTRACE"];
    assert!(
        backtrace.as_i64() == Some(1) || backtrace.as_str() == Some("1"),
        "AD-D5-9 : RUST_BACKTRACE must be 1, got {backtrace:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-10 — each verification job has timeout-minutes
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-10 pin** : each of the 4 verification jobs declares
/// `timeout-minutes` explicitly to prevent runaway runner consumption.
#[test]
fn workflow_each_job_has_timeout_minutes() {
    let w = load_workflow();
    let jobs = jobs(&w);
    for job_name in &[
        "unit-tests",
        "freeze-tests",
        "backfill-dry-run",
        "audit-reader-round-trip",
    ] {
        let job = &jobs[&Value::String((*job_name).to_string())];
        let timeout = job["timeout-minutes"].as_u64().unwrap_or_else(|| {
            panic!(
                "AD-D5-10 : job '{job_name}' must declare timeout-minutes \
                 (currently missing or not a number)"
            )
        });
        assert!(
            (1..=60).contains(&timeout),
            "AD-D5-10 : job '{job_name}' timeout-minutes must be in [1, 60], got {timeout}"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
//  AD-D5-11 — triggers pull_request + push to main
// ═══════════════════════════════════════════════════════════════════

/// **AD-D5-11 pin** : the workflow triggers on both `pull_request`
/// (with path filters) and `push` to `main`.
///
/// Note : serde_yaml has a quirk where bare `on:` parses as the bool
/// key `true:`.  We work around by raw-text scanning the `on:` block.
#[test]
fn workflow_triggers_on_pr_and_push_main() {
    let raw = load_workflow_raw();
    assert!(
        raw.contains("pull_request:"),
        "AD-D5-11 : workflow must trigger on pull_request"
    );
    assert!(
        raw.contains("paths:"),
        "AD-D5-11 : pull_request trigger must have a paths: filter"
    );
    assert!(
        raw.contains("push:"),
        "AD-D5-11 : workflow must also trigger on push"
    );
    assert!(
        raw.contains("branches: [main]") || raw.contains("branches:\n      - main"),
        "AD-D5-11 : push must target the main branch"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Meta-test : runner is ubuntu-latest for all verification jobs
// ═══════════════════════════════════════════════════════════════════

/// Meta-test : the 4 verification jobs run on `ubuntu-latest`, the
/// canonical EFE CI runner.  Pin runner OS for reproducibility.
#[test]
fn workflow_verification_jobs_run_on_ubuntu_latest() {
    let w = load_workflow();
    let jobs = jobs(&w);
    for job_name in &[
        "unit-tests",
        "freeze-tests",
        "backfill-dry-run",
        "audit-reader-round-trip",
        "pr27-gate",
    ] {
        let job = &jobs[&Value::String((*job_name).to_string())];
        let runs_on = job["runs-on"].as_str().unwrap_or_else(|| {
            panic!("job '{job_name}' must declare runs-on as a string")
        });
        assert_eq!(
            runs_on, "ubuntu-latest",
            "all PR #27 jobs must run on ubuntu-latest for reproducibility, \
             '{job_name}' uses '{runs_on}'"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
//  Meta-test : checkout action is pinned to v4
// ═══════════════════════════════════════════════════════════════════

/// Meta-test : the workflow uses `actions/checkout@v4` (current
/// stable) and not a floating tag like `@main` or `@master`.
#[test]
fn workflow_checkout_action_pinned() {
    let raw = load_workflow_raw();
    assert!(
        raw.contains("actions/checkout@v4"),
        "workflow must use actions/checkout@v4 (pinned major version)"
    );
    assert!(
        !raw.contains("actions/checkout@main") && !raw.contains("actions/checkout@master"),
        "workflow must NOT use floating refs like @main or @master"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Day 5 audit additions : security + structural completeness pins
//  (surfaced by Day 5 chirurgical audit Pass 7)
// ═══════════════════════════════════════════════════════════════════

/// **Day 5 audit pin (security)** : the workflow's `permissions:` block
/// is **restrictive**.  Specifically :
/// - `contents` must be `read` (not `write` — `write` would allow the
///   workflow to push commits to the branch, a serious security risk
///   if a malicious dependency action is invoked).
/// - `pull-requests` may be `write` (needed for the comment-on-PR
///   feature).
/// - No other `permissions:` key is set (principle of least privilege).
///
/// If a future PR widens the permissions (e.g. to `contents: write`
/// or `actions: write`), this test fails LOUDLY before the change
/// hits CI.
#[test]
fn workflow_permissions_block_is_restrictive() {
    let w = load_workflow();
    let permissions = &w["permissions"];
    assert!(
        !permissions.is_null(),
        "Day 5 audit pin : workflow MUST declare a permissions: block. \
         Without one, GitHub grants default permissions which include \
         `contents: write` — dangerous for any workflow that runs \
         third-party actions."
    );

    let perms_map = permissions
        .as_mapping()
        .expect("permissions: must be a mapping");

    let contents = perms_map
        .get(serde_yaml::Value::String("contents".to_string()))
        .and_then(|v| v.as_str())
        .expect("permissions.contents must be set as string");
    assert_eq!(
        contents, "read",
        "Day 5 audit pin : permissions.contents MUST be 'read' (principle of \
         least privilege).  'write' is forbidden by EFE security policy. \
         Currently : '{contents}'"
    );

    // pull-requests may be either 'read' or 'write' (depending on whether
    // the workflow posts comments).  Both are acceptable.
    let pr_perm = perms_map
        .get(serde_yaml::Value::String("pull-requests".to_string()))
        .and_then(|v| v.as_str());
    if let Some(pr_val) = pr_perm {
        assert!(
            pr_val == "read" || pr_val == "write",
            "permissions.pull-requests must be 'read' or 'write', got '{pr_val}'"
        );
    }

    // Forbid 'write' on actions: or id-token: — these are sensitive.
    for forbidden_key in &["actions", "id-token", "packages", "deployments"] {
        if let Some(v) = perms_map
            .get(serde_yaml::Value::String((*forbidden_key).to_string()))
            .and_then(|v| v.as_str())
        {
            assert_ne!(
                v, "write",
                "Day 5 audit pin : permissions.{forbidden_key} must NOT be 'write' \
                 (security policy : least privilege)"
            );
        }
    }
}

/// **Day 5 audit pin (structural completeness)** : every job in the
/// workflow has at least one step.  An empty job is a configuration
/// error that would silently consume a runner minute without doing
/// any work.
#[test]
fn workflow_every_job_has_at_least_one_step() {
    let w = load_workflow();
    let jobs = jobs(&w);

    for (job_key, job_value) in jobs {
        let job_name = job_key.as_str().expect("job key must be a string");
        let steps = job_value
            .get("steps")
            .and_then(|s| s.as_sequence())
            .unwrap_or_else(|| {
                panic!(
                    "Day 5 audit pin : job '{job_name}' must have a steps: array. \
                     Empty jobs are configuration errors."
                )
            });
        assert!(
            !steps.is_empty(),
            "Day 5 audit pin : job '{job_name}' must have at least 1 step"
        );
    }
}

/// **Day 5 audit pin (cost discipline)** : no job uses `runs-on:
/// macos-latest` or `runs-on: windows-latest` accidentally.
///
/// Rationale : macOS runners cost 10× more than ubuntu-latest, Windows
/// 2× more.  PR #27's scope is entirely Linux-friendly (Rust + bash +
/// python3 + cosign), so accidentally specifying macOS would burn
/// runner minutes without justification.
#[test]
fn workflow_no_expensive_runner_os() {
    let w = load_workflow();
    let jobs = jobs(&w);
    for (job_key, job_value) in jobs {
        let job_name = job_key.as_str().expect("job key must be a string");
        if let Some(runs_on) = job_value.get("runs-on").and_then(|v| v.as_str()) {
            assert!(
                !runs_on.starts_with("macos"),
                "Day 5 audit pin : job '{job_name}' uses macOS runner '{runs_on}' — \
                 10× cost vs ubuntu-latest. macOS is forbidden by EFE cost policy \
                 unless explicitly justified."
            );
            assert!(
                !runs_on.starts_with("windows"),
                "Day 5 audit pin : job '{job_name}' uses Windows runner '{runs_on}' — \
                 2× cost vs ubuntu-latest. Windows is forbidden unless justified."
            );
        }
    }
}

/// **Day 5 audit pin (no leaked secrets)** : no job references a
/// secret that isn't necessary for PR #27.
///
/// PR #27 doesn't sign anything via the workflow (signing happens at
/// quarterly publish time, separately).  The only "secret-like"
/// reference allowed is the cosign keyless OIDC token via
/// `id-token: write`, which we've already verified is NOT in
/// permissions (per `workflow_permissions_block_is_restrictive`).
///
/// Search for `${{ secrets.` patterns ; any match indicates a
/// non-trivial secret usage that should be reviewed.
#[test]
fn workflow_no_leaked_secrets_references() {
    let raw = load_workflow_raw();
    let suspicious_patterns = [
        "${{ secrets.AWS_",
        "${{ secrets.GCP_",
        "${{ secrets.AZURE_",
        "${{ secrets.NPM_TOKEN",
        "${{ secrets.DOCKER_",
        "${{ secrets.CRATES_IO_",
    ];
    for pattern in &suspicious_patterns {
        assert!(
            !raw.contains(pattern),
            "Day 5 audit pin : workflow references suspicious secret '{pattern}'. \
             PR #27 is verification-only and should not need cloud or \
             registry credentials."
        );
    }
}
