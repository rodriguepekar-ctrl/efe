//! PR #24 D-AP — structural invariants for the CI workflow YAML.
//!
//! Catches silent regressions in `.github/workflows/bench-publish-quarterly.yml` :
//! cron schedule drift, OIDC permission missing, `evidence verify`
//! step removed.  We parse the YAML and assert key invariants — this
//! is cheaper than launching `act` and runs in milliseconds.

use std::path::PathBuf;

fn workflow_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/bench-publish-quarterly.yml")
}

fn workflow_body() -> String {
    let p = workflow_path();
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("workflow at {p:?} : {e}"))
}

#[test]
fn d_ap_workflow_yaml_parses_cleanly() {
    let body = workflow_body();
    let _: serde_yaml::Value = serde_yaml::from_str(&body)
        .expect("D-AP : workflow YAML must parse — syntax error blocks CI");
}

#[test]
fn d_ap_cron_schedule_matches_quarterly_frequency() {
    let yaml: serde_yaml::Value = serde_yaml::from_str(&workflow_body()).unwrap();
    // Top-level `on.schedule` is a sequence of `{cron: "..."}` items.
    // YAML's bareword `on` is special — serde-yaml deserialises it as
    // a Bool!  We look up by the string key `"on"` AND fall back to
    // `Bool(true)` because `on: schedule:` is interpreted as
    // `true: schedule:` by some YAML parsers (the so-called "Norway
    // problem" applied to `on`).  Defensive.
    let on_key = yaml
        .as_mapping()
        .and_then(|m| {
            m.get(serde_yaml::Value::String("on".into()))
                .or_else(|| m.get(serde_yaml::Value::Bool(true)))
        })
        .expect("D-AP : 'on:' section missing");
    let schedule = on_key
        .as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("schedule".into())))
        .expect("D-AP : 'schedule:' missing");
    let crons: Vec<_> = schedule
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|item| {
            item.as_mapping()?
                .get(serde_yaml::Value::String("cron".into()))?
                .as_str()
        })
        .collect();
    assert!(!crons.is_empty(), "D-AP : at least one cron entry required");
    // Must fire on the 1st of Jan/Apr/Jul/Oct (quarterly boundary + 1d).
    assert!(
        crons.iter().any(|c| c.contains("1,4,7,10")),
        "D-AP : cron must run on the 1st of Jan/Apr/Jul/Oct — got {crons:?}"
    );
}

#[test]
fn d_ap_workflow_has_id_token_write_for_oidc() {
    let body = workflow_body();
    assert!(
        body.contains("id-token: write"),
        "D-AP : id-token: write permission required for cosign keyless OIDC"
    );
}

#[test]
fn d_ap_workflow_invokes_evidence_verify_subcommand() {
    let body = workflow_body();
    assert!(
        body.contains("evidence verify"),
        "D-AP : workflow must invoke `evidence verify` (AC22 cross-chain verifier)"
    );
}

#[test]
fn d_ap_workflow_invokes_publish_quarterly_subcommand() {
    let body = workflow_body();
    assert!(
        body.contains("publish-quarterly"),
        "D-AP : workflow must invoke `publish-quarterly` subcommand"
    );
}

#[test]
fn d_ap_workflow_pins_cosign_installer_version() {
    let body = workflow_body();
    // Critical : pinning sigstore/cosign-installer protects against
    // supply-chain compromise of an unpinned floating tag.
    assert!(
        body.contains("sigstore/cosign-installer@v"),
        "D-AP : cosign-installer must be pinned to a version (vX.Y.Z)"
    );
}

#[test]
fn d_ap_workflow_dispatch_period_input_optional() {
    let body = workflow_body();
    // Manual dispatch with empty `period` must work (the workflow
    // computes the previous quarter from current date).  Assert the
    // input exists and is NOT required.
    let yaml: serde_yaml::Value = serde_yaml::from_str(&body).unwrap();
    let on = yaml
        .as_mapping()
        .and_then(|m| {
            m.get(serde_yaml::Value::String("on".into()))
                .or_else(|| m.get(serde_yaml::Value::Bool(true)))
        })
        .unwrap();
    let dispatch = on
        .as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("workflow_dispatch".into())))
        .expect("D-AP : workflow_dispatch missing — manual trigger broken");
    let inputs = dispatch
        .as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("inputs".into())))
        .expect("inputs missing");
    let period = inputs
        .as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("period".into())))
        .expect("period input missing");
    let required = period
        .as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("required".into())))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(
        !required,
        "D-AP : period input MUST be optional (workflow auto-derives the previous quarter)"
    );
}

// ─────────────────────────────────────────────────────────────────────
// D-BK — structural invariants for the MAIN publish workflow
// (.github/workflows/bench-publish.yml).  Audited separately from
// the quarterly workflow.
// ─────────────────────────────────────────────────────────────────────

fn main_workflow_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/bench-publish.yml")
}

fn main_workflow_body() -> String {
    std::fs::read_to_string(main_workflow_path()).expect("main workflow")
}

#[test]
fn d_bk_main_workflow_concurrency_is_sequential_t122() {
    // **AD31 / T122** : `cancel-in-progress: false` prevents race
    // conditions on the gh-pages branch where parallel publishes would
    // each rebase on top of each other and drop commits.
    let body = main_workflow_body();
    assert!(
        body.contains("cancel-in-progress: false"),
        "D-BK : main publish workflow MUST set cancel-in-progress: false (AD31 / T122)"
    );
    assert!(
        body.contains("group:"),
        "D-BK : main publish workflow MUST declare a concurrency group"
    );
}

#[test]
fn d_bk_main_workflow_keeps_files_immutable() {
    // **AD30** : `keep_files: true` means past publishes are NEVER
    // deleted from gh-pages.  Critical for SOC 2 evidence trails.
    let body = main_workflow_body();
    assert!(
        body.contains("keep_files: true"),
        "D-BK : main publish workflow MUST set keep_files: true (AD30 — immutable)"
    );
    assert!(
        !body.contains("force_orphan: true"),
        "D-BK : force_orphan: true would wipe history — forbidden"
    );
}

#[test]
fn d_bk_main_workflow_pins_mdbook_minor_version() {
    let body = main_workflow_body();
    assert!(
        body.contains(r#"mdbook --version "^0.4""#) || body.contains(r#"mdbook --version '^0.4'"#),
        "D-BK : mdbook minor version must be pinned (supply-chain safety)"
    );
}

#[test]
fn d_bk_main_workflow_runs_ratchet_check() {
    // Even though the harness internally runs the ratchet, the
    // workflow ALSO does an inline shell check for the 11 badges /
    // index.html / current.json — defense in depth.
    let body = main_workflow_body();
    assert!(
        body.contains("Ratchet sanity"),
        "D-BK : workflow must run the AD32 ratchet sanity check"
    );
    assert!(
        body.contains("ne 11"),
        "D-BK : ratchet must assert exactly 11 badges"
    );
}

#[test]
fn d_bk_main_workflow_does_not_skip_signing_or_hooks() {
    let body = main_workflow_body();
    assert!(!body.contains("--no-verify"));
    assert!(!body.contains("--cosign-disabled"));
}

#[test]
fn d_ap_workflow_does_not_skip_hooks_or_disable_signing() {
    let body = workflow_body();
    // Sanity : the workflow MUST NOT pass --no-verify, --no-gpg-sign,
    // or disable any signing step (subtle supply-chain risk).
    assert!(
        !body.contains("--no-verify"),
        "D-AP : --no-verify forbidden in CI workflows"
    );
    assert!(
        !body.contains("--cosign-disabled"),
        "D-AP : --cosign-disabled MUST NOT appear in CI workflows (local dev only)"
    );
}
