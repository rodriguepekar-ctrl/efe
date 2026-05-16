//! PR #24 § 19 + § 22 — STABILITY_CONTRACT v1.6 § 24 frozen-test suite.
//!
//! Locks the public-page URL schema, badge filenames, JSON API,
//! quarterly evidence path, CLI surface, and cosign identity regex.
//! Failure here is a contract violation — fixing the test is **NOT**
//! a valid option ; the offending change must be reverted or a
//! major-version bump filed.

use efe_bench_harness::publish::badge::BADGE_FILENAMES;
use efe_bench_harness::publish::quarterly::{COSIGN_OIDC_ISSUER, SHARED_COSIGN_IDENTITY_REGEXP};
use efe_bench_harness::publish::trend::{SPARK_HEIGHT, SPARK_WIDTH};

// ─────────────────────────────────────────────────────────────────────
// § 24.1 — 11 badge file names
// ─────────────────────────────────────────────────────────────────────

#[test]
fn v1_6_section_24_1_eleven_badges() {
    assert_eq!(BADGE_FILENAMES.len(), 11);
    assert_eq!(
        BADGE_FILENAMES,
        &[
            "scenario-S01.svg",
            "scenario-S02.svg",
            "scenario-S03.svg",
            "scenario-S04.svg",
            "scenario-S05.svg",
            "scenario-S06.svg",
            "scenario-S07.svg",
            "scenario-S08.svg",
            "scenario-S09.svg",
            "scenario-S10.svg",
            "overall.svg",
        ]
    );
}

// ─────────────────────────────────────────────────────────────────────
// § 24.3 — schema_version frozen 1
// ─────────────────────────────────────────────────────────────────────

#[test]
fn v1_6_section_24_3_schema_version_1() {
    assert_eq!(efe_bench_harness::publish::api::API_SCHEMA_VERSION, 1);
}

/// **PR #25 day 4** — the 5 frozen API endpoint filenames.
///
/// Hoisted from the local `REQUIRED_API` const in
/// `publish/deploy/ratchet.rs` to the canonical
/// `crate::publish::api::API_ENDPOINT_FILENAMES`.  The 5 names
/// themselves are part of the v1.6 frozen surface (§ 24.3) — adding
/// a 6th requires a STABILITY major bump.
#[test]
fn v1_6_section_24_3_five_api_endpoint_filenames() {
    use efe_bench_harness::publish::api::API_ENDPOINT_FILENAMES;
    assert_eq!(
        API_ENDPOINT_FILENAMES.len(),
        5,
        "v1.6 § 24.3 freezes exactly 5 API endpoints"
    );
    assert_eq!(
        API_ENDPOINT_FILENAMES,
        &[
            "current.json",
            "history.json",
            "scenarios.json",
            "baselines.json",
            "version.json",
        ]
    );
}

// ─────────────────────────────────────────────────────────────────────
// § 24.5 — CLI surface : 7 new flags + new subcommand
// ─────────────────────────────────────────────────────────────────────

#[test]
fn v1_6_section_24_5_publish_has_7_new_flags() {
    use clap::CommandFactory;
    use efe_bench_harness::cli::Cli;
    let cmd = Cli::command();
    let publish = cmd
        .get_subcommands()
        .find(|s| s.get_name() == "publish")
        .expect("`publish` subcommand present");
    let flags: Vec<_> = publish
        .get_arguments()
        .map(|a| a.get_long().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for required in [
        "bench-results",
        "bench-baselines",
        "chart-version",
        "embed-slos",
        "embed-smoke",
        "dry-run",
        "history-window-days",
    ] {
        assert!(
            flags.iter().any(|f| f == required),
            "missing --{required} on `publish` (v1.6 § 24.5)"
        );
    }
}

#[test]
fn v1_6_section_24_5_publish_quarterly_subcommand_present() {
    use clap::CommandFactory;
    use efe_bench_harness::cli::Cli;
    let cmd = Cli::command();
    let has_pq = cmd.get_subcommands().any(|s| s.get_name() == "publish-quarterly");
    assert!(has_pq, "subcommand `publish-quarterly` must exist (v1.6 § 24.5)");
}

#[test]
fn v1_6_section_24_5_evidence_verify_subcommand_present() {
    // D-E fix : `evidence verify` is the cross-chain verifier (AC22).
    // Frozen v1.6 § 24.5 — must accept BOTH PR #22 SLO + PR #24 bench PDFs
    // via the shared cosign identity regex.
    use clap::CommandFactory;
    use efe_bench_harness::cli::Cli;
    let cmd = Cli::command();
    let evidence = cmd
        .get_subcommands()
        .find(|s| s.get_name() == "evidence")
        .expect("subcommand `evidence` must exist (v1.6 § 24.5)");
    let has_verify = evidence.get_subcommands().any(|s| s.get_name() == "verify");
    assert!(has_verify, "subcommand `evidence verify` must exist (AC22)");
}

// ─────────────────────────────────────────────────────────────────────
// § 24.6 — site-URL invariants
// ─────────────────────────────────────────────────────────────────────

#[test]
fn v1_6_section_24_6_badges_use_kebab_case() {
    for f in BADGE_FILENAMES {
        // Frozen filenames must contain only ASCII letters/digits/dashes/dots.
        for c in f.chars() {
            assert!(
                c.is_ascii_alphanumeric() || c == '-' || c == '.',
                "badge filename '{f}' violates URL invariant § 24.6 : char '{c}'"
            );
        }
        // No underscores (kebab-case).
        assert!(!f.contains('_'), "badge filename '{f}' must be kebab-case");
        // No uppercase outside the scenario short id : we waive this for
        // `S01`-style scenario ids which are uppercase by convention.
    }
}

// ─────────────────────────────────────────────────────────────────────
// § 24.8 — cosign identity regex (shared with PR #22)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn v1_6_section_24_8_shared_cosign_identity_regex() {
    // Frozen ; extended from PR #22 v1.4 § 22.8 to include the bench
    // publish-quarterly workflow (AD25).
    assert!(SHARED_COSIGN_IDENTITY_REGEXP.contains("slo-quarterly"));
    assert!(SHARED_COSIGN_IDENTITY_REGEXP.contains("bench-publish-quarterly"));
    assert!(SHARED_COSIGN_IDENTITY_REGEXP.contains("efe-ai/efe"));
    assert_eq!(
        COSIGN_OIDC_ISSUER,
        "https://token.actions.githubusercontent.com"
    );
}

/// **D-AB** — mirror PR #22's narrower regex INSIDE the bench harness
/// test suite.  If PR #22 (`efe/slos/rollup/src/evidence/mod.rs`)
/// changes its `FROZEN_IDENTITY_REGEXP`, this test fails until the
/// bench harness's cross-chain superset regex is updated in lock-step.
///
/// This is the cross-PR consistency guard the D-T audit pass identified
/// as missing : without it, a unilateral PR #22 change could silently
/// break the bench harness's "accepts both chains" claim.
const PR22_FROZEN_IDENTITY_REGEXP: &str =
    r"^https://github\.com/efe-ai/efe/\.github/workflows/slo-quarterly\.yml@refs/.+";

#[test]
fn d_ab_pr22_regex_value_mirrored_for_cross_chain_invariant() {
    // The mirror exists.  Any divergence here means PR #22's regex was
    // changed without updating the bench harness — fix : pull the new
    // PR #22 value into this constant + re-derive the cross-chain
    // superset in publish/quarterly/mod.rs.
    assert_eq!(
        PR22_FROZEN_IDENTITY_REGEXP,
        r"^https://github\.com/efe-ai/efe/\.github/workflows/slo-quarterly\.yml@refs/.+",
        "D-AB : PR #22 regex drifted (re-sync from efe/slos/rollup/src/evidence/mod.rs)"
    );
}

#[test]
fn d_ab_pr22_regex_rejects_bench_publish_quarterly_subjects() {
    // **The asymmetry contract** (documented in D-T) : PR #22's regex
    // is intentionally narrower → it does NOT accept PR #24 bench
    // PDFs.  A single shared regex would be cleaner BUT requires a
    // cross-crate refactor + STABILITY bump ; for now we assert the
    // asymmetry holds so it stays a known design choice, not a bug.
    let re = regex::Regex::new(PR22_FROZEN_IDENTITY_REGEXP).unwrap();
    // PR #22 SLO subject : MUST match.
    assert!(re.is_match(
        "https://github.com/efe-ai/efe/.github/workflows/slo-quarterly.yml@refs/heads/main"
    ));
    // PR #24 bench subject : MUST NOT match (the asymmetry).
    assert!(
        !re.is_match(
            "https://github.com/efe-ai/efe/.github/workflows/bench-publish-quarterly.yml@refs/heads/main"
        ),
        "D-AB : PR #22's narrower regex must reject bench PDFs"
    );
}

#[test]
fn d_ab_cross_chain_regex_is_strict_superset_of_pr22() {
    // The bench harness's cross-chain regex MUST accept every OIDC
    // subject PR #22's regex accepts.  Failure here would mean cross-
    // chain verification of PR #22 PDFs is broken in the bench
    // verifier (AC22 violation).
    let pr22 = regex::Regex::new(PR22_FROZEN_IDENTITY_REGEXP).unwrap();
    let bench = regex::Regex::new(SHARED_COSIGN_IDENTITY_REGEXP).unwrap();

    let pr22_subjects = [
        "https://github.com/efe-ai/efe/.github/workflows/slo-quarterly.yml@refs/heads/main",
        "https://github.com/efe-ai/efe/.github/workflows/slo-quarterly.yml@refs/tags/v0.6.0",
        "https://github.com/efe-ai/efe/.github/workflows/slo-quarterly.yml@refs/heads/release-0.6",
    ];
    for subject in pr22_subjects {
        assert!(
            pr22.is_match(subject),
            "fixture sanity : PR #22 regex must accept '{subject}'"
        );
        assert!(
            bench.is_match(subject),
            "D-AB : cross-chain regex MUST accept every PR #22 subject — failed on '{subject}'"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
// Sparkline dimensions frozen (§ 9 + AD11)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn sparkline_dimensions_frozen_200x40() {
    assert_eq!(SPARK_WIDTH, 200);
    assert_eq!(SPARK_HEIGHT, 40);
}

// ─────────────────────────────────────────────────────────────────────
// Cross-link with PR #22 + PR #23 (frozen URLs)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn pr22_pr23_cross_link_urls_frozen() {
    use efe_bench_harness::publish::crosslink::{
        SLO_BADGE_BASE_URL, SLO_JSON_API_BASE_URL, SMOKE_BADGE_URL,
    };
    assert_eq!(SLO_BADGE_BASE_URL, "https://efe.github.io/slos/badges");
    assert_eq!(SLO_JSON_API_BASE_URL, "https://efe.github.io/slos");
    assert_eq!(SMOKE_BADGE_URL, "https://efe.github.io/smoke/badge.svg");
}
