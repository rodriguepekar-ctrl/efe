//! PR #25 § 4.4 — process invariant **I.2** : single source of truth.
//!
//! This integration test is the **hard gate** for I.2.  It walks the
//! workspace, extracts every `const NAME: &[&str] = &[...]` declaration
//! with ≥ 5 entries, and asserts that no two non-whitelisted consts
//! overlap by ≥ 80 %.
//!
//! The companion `sot-audit.sh` shell script (PR #25 Day 1) is the
//! human-readable scanner ; this test is what blocks PRs in CI.
//!
//! Frozen by STABILITY_CONTRACT v1.7 § 25.2 :
//!   * overlap threshold = 80 %
//!   * min entries       = 5
//!   * whitelist requires per-entry justification comment
//!
//! Companion negative tests are in `process_invariants_negatives.rs` :
//!   they verify the detector ACTUALLY fires on synthetic input, so a
//!   broken detector cannot silently pass this file.
//!
//! References :
//!   * docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 4
//!   * docs/process/sot-pattern.md
//!   * docs/STABILITY_CONTRACT.md § 25.2

mod common;

use common::sot::{
    collect_workspace_qualifying_consts, detect_duplications, extract_string_array_consts,
    is_whitelisted, overlap_pct, workspace_root, SOT_MIN_ENTRIES, SOT_OVERLAP_THRESHOLD_PCT,
};

// ───────────────────────────────────────────────────────────────────
// THE HARD GATE
// ───────────────────────────────────────────────────────────────────

/// **I.2 hard gate** — no const lists with ≥ 5 entries may overlap
/// by ≥ 80 % across the workspace, modulo the explicit whitelist.
///
/// If this test fails, the PR carries a parallel-list duplication.
/// Resolution paths (in order of preference) :
///   1. Consolidate to a single canonical `const`, `use` from the
///      other site.
///   2. If the duplication is intentional (test mirror, generated code),
///      add an entry to `ALLOWED_PARALLEL_DUPLICATIONS` in
///      `tests/common/sot.rs` with a comment explaining why.
///
/// See `docs/process/sot-pattern.md` § 8 (migration playbook).
#[test]
fn i_2_no_parallel_const_lists_in_workspace() {
    let consts = collect_workspace_qualifying_consts();

    println!(
        "[i.2] scanned workspace : {} qualifying const &[&str] (≥ {} entries) discovered",
        consts.len(),
        SOT_MIN_ENTRIES
    );

    let duplications = detect_duplications(&consts, SOT_OVERLAP_THRESHOLD_PCT);

    if !duplications.is_empty() {
        let mut msg = format!(
            "I.2 violation : {} parallel-list pair(s) found (threshold = {}%)\n\n",
            duplications.len(),
            SOT_OVERLAP_THRESHOLD_PCT
        );
        for d in &duplications {
            msg.push_str(&format!("  • {}\n", d.diagnostic()));
        }
        msg.push_str(
            "\nFix : consolidate the duplicated list to a single canonical\n\
             location ; reference from the other site by `use ::path`.\n\
             OR add a whitelist entry to ALLOWED_PARALLEL_DUPLICATIONS\n\
             in tests/common/sot.rs (with a justifying comment).\n\
             See docs/process/sot-pattern.md for the SOT decision tree.",
        );
        panic!("{msg}");
    }
}

// ───────────────────────────────────────────────────────────────────
// Per-canonical-const targeted tests.
//
// These exist for diagnostic precision : if the hard gate above fails,
// the per-canonical tests pinpoint which canonical const has been
// duplicated.  Each test asserts that the canonical const is present
// in the expected file (drift detector) AND that no other site has
// ≥ 80 % overlap (parallel detector).
// ───────────────────────────────────────────────────────────────────

/// The 11-element `BADGE_FILENAMES` const — STABILITY v1.6 § 24.1.
#[test]
fn i_2_badge_filenames_canonical_and_unique() {
    use efe_bench_harness::publish::badge::BADGE_FILENAMES;

    // (1) Canonical presence + count.
    assert_eq!(
        BADGE_FILENAMES.len(),
        11,
        "BADGE_FILENAMES must have 11 entries (v1.6 § 24.1)"
    );

    // (2) No parallel const list with ≥ 80 % overlap.
    let canonical_items: Vec<String> = BADGE_FILENAMES.iter().map(|s| (*s).to_string()).collect();
    let consts = collect_workspace_qualifying_consts();
    let canonical_loc = consts.iter().find(|c| c.name == "BADGE_FILENAMES");
    assert!(
        canonical_loc.is_some(),
        "BADGE_FILENAMES not found by the extractor — extractor regression?"
    );

    for other in &consts {
        if other.name == "BADGE_FILENAMES" {
            continue; // canonical itself
        }
        if is_whitelisted(other) {
            continue;
        }
        let pct = overlap_pct(&canonical_items, &other.items);
        assert!(
            pct < SOT_OVERLAP_THRESHOLD_PCT,
            "{} {} mirrors BADGE_FILENAMES at {}% (must be < {}%)",
            other.location(),
            other.name,
            pct,
            SOT_OVERLAP_THRESHOLD_PCT
        );
    }
}

/// The 5-element `API_ENDPOINT_FILENAMES` const — STABILITY v1.6 § 24.3.
///
/// **PR #25 day 4** — hoisted from a local `REQUIRED_API` const in
/// `publish/deploy/ratchet.rs` to the canonical
/// `crate::publish::api::API_ENDPOINT_FILENAMES`.  This test asserts :
///   1. The canonical const exists with 5 entries.
///   2. No parallel-list duplication elsewhere in the workspace.
#[test]
fn i_2_api_endpoint_filenames_canonical_and_unique() {
    use efe_bench_harness::publish::api::API_ENDPOINT_FILENAMES;

    // (1) Canonical presence + count.
    assert_eq!(
        API_ENDPOINT_FILENAMES.len(),
        5,
        "API_ENDPOINT_FILENAMES must have 5 entries (v1.6 § 24.3)"
    );

    // (2) No parallel const list with >= 80 % overlap.
    let canonical_items: Vec<String> =
        API_ENDPOINT_FILENAMES.iter().map(|s| (*s).to_string()).collect();
    let consts = collect_workspace_qualifying_consts();
    let canonical_loc = consts.iter().find(|c| c.name == "API_ENDPOINT_FILENAMES");
    assert!(
        canonical_loc.is_some(),
        "API_ENDPOINT_FILENAMES not found by the extractor — extractor regression \
         OR the day-4 hoist was undone"
    );

    for other in &consts {
        if other.name == "API_ENDPOINT_FILENAMES" {
            continue; // canonical itself
        }
        if is_whitelisted(other) {
            continue;
        }
        let pct = overlap_pct(&canonical_items, &other.items);
        assert!(
            pct < SOT_OVERLAP_THRESHOLD_PCT,
            "{} {} mirrors API_ENDPOINT_FILENAMES at {}% (must be < {}%) — \
             a parallel-list duplication of the 5 JSON endpoints has been \
             reintroduced ; the day-4 SOT cleanup must hold",
            other.location(),
            other.name,
            pct,
            SOT_OVERLAP_THRESHOLD_PCT
        );
    }
}

/// The 10-element `SCENARIO_PAGE_FILENAMES` const — STABILITY v1.6 § 24.2.
///
/// Note : this const is `&[(&str, &str)]`, not `&[&str]`, so the SOT
/// extractor does NOT see it.  But we still assert its count + canonical
/// presence here, as the v1.6 § 24.2 stability anchor.
#[test]
fn i_2_scenario_page_filenames_canonical_and_unique() {
    use efe_bench_harness::publish::SCENARIO_PAGE_FILENAMES;

    assert_eq!(
        SCENARIO_PAGE_FILENAMES.len(),
        10,
        "SCENARIO_PAGE_FILENAMES must have 10 entries (v1.6 § 24.2)"
    );

    // Extract just the filename column for the overlap check.
    let canonical_items: Vec<String> = SCENARIO_PAGE_FILENAMES
        .iter()
        .map(|(_, name)| (*name).to_string())
        .collect();

    let consts = collect_workspace_qualifying_consts();
    for other in &consts {
        if is_whitelisted(other) {
            continue;
        }
        // Skip the BADGE_FILENAMES const (we test it separately).
        if other.name == "BADGE_FILENAMES" {
            continue;
        }
        let pct = overlap_pct(&canonical_items, &other.items);
        assert!(
            pct < SOT_OVERLAP_THRESHOLD_PCT,
            "{} {} mirrors SCENARIO_PAGE_FILENAMES (filenames column) at {}% (must be < {}%)",
            other.location(),
            other.name,
            pct,
            SOT_OVERLAP_THRESHOLD_PCT
        );
    }
}

// ───────────────────────────────────────────────────────────────────
// Extractor invariants : prove the extractor sees the canonical
// consts (regression guard for the regex itself).
// ───────────────────────────────────────────────────────────────────

/// The extractor MUST find `BADGE_FILENAMES` at its canonical location.
/// If this fails, the regex / multi-line parser has regressed and the
/// hard-gate above could silently pass.
#[test]
fn i_2_extractor_finds_canonical_badge_filenames() {
    let badge_mod = workspace_root().join("bench/harness/src/publish/badge/mod.rs");
    assert!(
        badge_mod.exists(),
        "canonical badge module missing : {}",
        badge_mod.display()
    );
    let consts = extract_string_array_consts(&badge_mod);
    let found = consts.iter().find(|c| c.name == "BADGE_FILENAMES");
    assert!(
        found.is_some(),
        "extractor did not find BADGE_FILENAMES in {} — regex broken?",
        badge_mod.display()
    );
    let f = found.unwrap();
    assert_eq!(
        f.items.len(),
        11,
        "extractor returned {} items for BADGE_FILENAMES, expected 11",
        f.items.len()
    );
    assert!(
        f.items.iter().any(|s| s == "overall.svg"),
        "extractor missed 'overall.svg' in BADGE_FILENAMES"
    );
}

// ───────────────────────────────────────────────────────────────────
// I.2 statistical sanity : the workspace must have ≥ 1 const list
// at all (otherwise the hard gate is meaningless).
// ───────────────────────────────────────────────────────────────────

#[test]
fn i_2_workspace_has_at_least_one_qualifying_const() {
    let consts = collect_workspace_qualifying_consts();
    assert!(
        !consts.is_empty(),
        "no qualifying const &[&str] found in workspace — the I.2 gate is vacuous"
    );
}

// ───────────────────────────────────────────────────────────────────
// I.2 frozen-threshold guard : the constants used by this test must
// match the STABILITY § 25.2 frozen values.  Catches accidental tweaks
// of the threshold.
// ───────────────────────────────────────────────────────────────────

#[test]
fn i_2_thresholds_match_stability_v1_7_section_25_2() {
    assert_eq!(
        SOT_OVERLAP_THRESHOLD_PCT, 80,
        "SOT overlap threshold drifted from STABILITY v1.7 § 25.2 (frozen at 80%)"
    );
    assert_eq!(
        SOT_MIN_ENTRIES, 5,
        "SOT min entries drifted from STABILITY v1.7 § 25.2 (frozen at 5)"
    );
}
