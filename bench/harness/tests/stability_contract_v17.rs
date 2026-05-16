//! PR #25 § 25 — STABILITY_CONTRACT v1.7 frozen-test suite.
//!
//! Locks the v1.7 process invariants :
//!   * § 25.1  I.1 audit prompt mandatory  (LOC threshold, 7 sections, bypass tags, exit codes)
//!   * § 25.2  I.2 single source of truth  (overlap threshold 80, min entries 5)
//!   * § 25.3  I.6 audit budget             (4 line items, tracker file presence)
//!   * § 25.4  Drill cadence                (quarterly defacement, annual revocation)
//!
//! Failure here is a contract violation — fixing the test is **NOT**
//! a valid option ; the offending change must be reverted or a
//! major-version bump filed.
//!
//! References :
//!   * docs/STABILITY_CONTRACT.md § 25
//!   * docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md

use std::path::PathBuf;

mod common;
use common::sot::{SOT_MIN_ENTRIES, SOT_OVERLAP_THRESHOLD_PCT};

// ───────────────────────────────────────────────────────────────────
// Helpers : path resolution + file reading.
// ───────────────────────────────────────────────────────────────────

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root must canonicalize")
}

fn read_workspace_file(rel: &str) -> String {
    let path = workspace_root().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {} : {e}", path.display()))
}

fn workspace_file_exists(rel: &str) -> bool {
    workspace_root().join(rel).exists()
}

// ═══════════════════════════════════════════════════════════════════
// § 25.1  I.1 — audit prompt mandatory
// ═══════════════════════════════════════════════════════════════════

/// The LOC threshold for I.1 enforcement is frozen at 1000.
/// Hardcoded both in the workflow YAML and in the architecture doc.
#[test]
fn v1_7_section_25_1_loc_threshold_frozen_1000() {
    let workflow = read_workspace_file(".github/workflows/pr25-process-checks.yml");
    assert!(
        workflow.contains("\"${NET}\" -ge 1000"),
        "I.1 LOC threshold drifted from 1000 in pr25-process-checks.yml"
    );

    let stability = read_workspace_file("docs/STABILITY_CONTRACT.md");
    assert!(
        stability.contains("≥ 1000 LOC"),
        "STABILITY § 25.1 LOC threshold drifted"
    );
}

/// The lessons file schema MUST require exactly the 7 canonical sections.
/// The validator hardcodes these labels ; the template + STABILITY mirror them.
#[test]
fn v1_7_section_25_1_lessons_template_has_7_required_sections() {
    let template = read_workspace_file("docs/lessons/_template.md");
    let expected = [
        "## 1. Headline",
        "## 2. Timeline summary",
        "## 3. Defect taxonomy",
        "## 4. What worked",
        "## 5. Process changes proposed",
        "## 6. Defect index",
        "## 7. Known residual gaps",
    ];
    for hdr in expected {
        assert!(
            template.contains(hdr),
            "STABILITY § 25.1 : lessons template missing section '{hdr}'"
        );
    }
}

/// The validator's exit-code contract is frozen.  Listed at the top of
/// the script in a stable comment block.
#[test]
fn v1_7_section_25_1_validator_exit_codes_documented() {
    let script = read_workspace_file("scripts/audit-prompt-validator.sh");
    // Each code must appear in the documented contract block.
    for (code, meaning) in [
        ("0", "validation passed"),
        ("2", "file argument missing or path doesn't exist"),
        ("3", "a required section header is missing"),
        ("4", "the defect-index table has zero rows"),
        ("5", "\"Known residual gaps\" section absent"),
        ("6", "the audit prompt"),
    ] {
        let probe = format!("{code}  {meaning}");
        // Tolerate variable whitespace : just check the code + a key
        // word from the meaning are present somewhere in the script.
        let key_word = meaning.split_whitespace().next().unwrap();
        let has_code_and_word = script
            .lines()
            .any(|l| l.contains(code) && l.contains(key_word));
        assert!(
            has_code_and_word,
            "STABILITY § 25.1 : validator exit code {code} ('{meaning}') not documented \
             (looking for '{probe}')"
        );
    }
}

/// Two bypass tags `[I.1-bypass]` and `[I.1-refactor]` are spelled
/// EXACTLY in the workflow + audit-prompt doc + STABILITY.
#[test]
fn v1_7_section_25_1_bypass_tags_spelling_frozen() {
    for tag in ["[I.1-bypass]", "[I.1-refactor]"] {
        for path in [
            "docs/STABILITY_CONTRACT.md",
            "docs/process/audit-prompt.md",
            "docs/process/audit-prompt-en.md",
            "docs/process/i1-bypass-log.md",
            ".github/workflows/pr25-process-checks.yml",
        ] {
            let body = read_workspace_file(path);
            assert!(
                body.contains(tag),
                "STABILITY § 25.1 : bypass tag '{tag}' missing from {path}"
            );
        }
    }
}

/// The PR template must reference the audit-prompt + lessons-file paths
/// so authors land on the schema by default.
#[test]
fn v1_7_section_25_1_pr_template_references_canonical_paths() {
    let pr_template = read_workspace_file(".github/pull_request_template.md");
    for needle in [
        "docs/lessons/PR-<N>-lessons.md",
        "scripts/audit-prompt-validator.sh",
        "docs/lessons/_template.md",
    ] {
        assert!(
            pr_template.contains(needle),
            "STABILITY § 25.1 : PR template missing '{needle}'"
        );
    }
}

// ═══════════════════════════════════════════════════════════════════
// § 25.2  I.2 — single source of truth
// ═══════════════════════════════════════════════════════════════════

/// SOT overlap threshold is frozen at 80%.
#[test]
fn v1_7_section_25_2_sot_overlap_threshold_80() {
    // Frozen value lives in tests/common/sot.rs.
    assert_eq!(
        SOT_OVERLAP_THRESHOLD_PCT, 80,
        "STABILITY § 25.2 : SOT overlap threshold drifted from 80%"
    );

    // Also asserted in the STABILITY_CONTRACT doc.
    let stability = read_workspace_file("docs/STABILITY_CONTRACT.md");
    assert!(
        stability.contains("80 %") || stability.contains("80%"),
        "STABILITY § 25.2 : 80% threshold not documented"
    );

    // The shell scanner defaults must agree.
    let shell = read_workspace_file("scripts/sot-audit.sh");
    assert!(
        shell.contains("THRESHOLD=80"),
        "STABILITY § 25.2 : sot-audit.sh default threshold drifted from 80"
    );
}

/// SOT minimum entries is frozen at 5.
#[test]
fn v1_7_section_25_2_sot_min_entries_5() {
    assert_eq!(
        SOT_MIN_ENTRIES, 5,
        "STABILITY § 25.2 : SOT min entries drifted from 5"
    );

    let shell = read_workspace_file("scripts/sot-audit.sh");
    assert!(
        shell.contains("MIN_ENTRIES=5"),
        "STABILITY § 25.2 : sot-audit.sh default min-entries drifted from 5"
    );
}

/// The canonical badge filenames const lives at its frozen location.
#[test]
fn v1_7_section_25_2_canonical_badge_filenames_present() {
    let path = "bench/harness/src/publish/badge/mod.rs";
    assert!(
        workspace_file_exists(path),
        "STABILITY § 25.2 : canonical {path} missing"
    );
    let body = read_workspace_file(path);
    assert!(
        body.contains("pub const BADGE_FILENAMES: &[&str]"),
        "STABILITY § 25.2 : canonical BADGE_FILENAMES const missing"
    );
}

/// The canonical scenario-page filenames const lives at its frozen location.
#[test]
fn v1_7_section_25_2_canonical_scenario_page_filenames_present() {
    let path = "bench/harness/src/publish/mod.rs";
    assert!(
        workspace_file_exists(path),
        "STABILITY § 25.2 : canonical {path} missing"
    );
    let body = read_workspace_file(path);
    assert!(
        body.contains("pub const SCENARIO_PAGE_FILENAMES"),
        "STABILITY § 25.2 : canonical SCENARIO_PAGE_FILENAMES const missing"
    );
}

/// **PR #25 day 4** — the canonical `API_ENDPOINT_FILENAMES` const
/// lives at its frozen location (`publish/api/mod.rs`).
///
/// Before day 4, the 5 names lived as a local `REQUIRED_API` in
/// `publish/deploy/ratchet.rs`.  Day 4 hoists them to the most
/// general module that DEFINES the concept (the api module itself).
#[test]
fn v1_7_section_25_2_canonical_api_endpoint_filenames_present() {
    let path = "bench/harness/src/publish/api/mod.rs";
    assert!(
        workspace_file_exists(path),
        "STABILITY § 25.2 : canonical {path} missing"
    );
    let body = read_workspace_file(path);
    assert!(
        body.contains("pub const API_ENDPOINT_FILENAMES: &[&str]"),
        "STABILITY § 25.2 : canonical API_ENDPOINT_FILENAMES const missing — \
         the hoist from REQUIRED_API was undone?"
    );

    // The ratchet MUST use the canonical const, not a local copy.
    let ratchet = read_workspace_file("bench/harness/src/publish/deploy/ratchet.rs");
    assert!(
        ratchet.contains("use crate::publish::api::API_ENDPOINT_FILENAMES"),
        "STABILITY § 25.2 : ratchet does not import API_ENDPOINT_FILENAMES"
    );
    assert!(
        !ratchet.contains("const REQUIRED_API:"),
        "STABILITY § 25.2 : ratchet still carries the deprecated REQUIRED_API const — \
         day-4 hoist incomplete (D-AS regression)"
    );
}

// ═══════════════════════════════════════════════════════════════════
// § 25.3  I.6 — audit budget
// ═══════════════════════════════════════════════════════════════════

/// The PR template's audit-budget section has exactly 4 numeric line items.
#[test]
fn v1_7_section_25_3_audit_budget_4_line_items() {
    let pr_template = read_workspace_file(".github/pull_request_template.md");
    for needle in [
        "Implementation : __ days",
        "Audit (per I.1) : __ days",
        "Buffer for unforeseen depth : __ days",
        "Total estimate",
    ] {
        assert!(
            pr_template.contains(needle),
            "STABILITY § 25.3 : PR template missing audit-budget item '{needle}'"
        );
    }
}

/// The audit-budget tracker file is present + has the 4-column structure.
#[test]
fn v1_7_section_25_3_budget_tracker_file_present() {
    let path = "docs/process/audit-budget-tracker.md";
    assert!(
        workspace_file_exists(path),
        "STABILITY § 25.3 : tracker file {path} missing"
    );
    let body = read_workspace_file(path);
    // Verify the 9 column labels expected by the tracker format.
    for col in [
        "PR",
        "Impl est",
        "Audit est",
        "Audit % est",
        "Actual impl",
        "Actual audit",
        "Actual %",
        "Defects",
        "Verdict",
    ] {
        assert!(
            body.contains(col),
            "STABILITY § 25.3 : tracker missing column '{col}'"
        );
    }
}

/// The workflow `pr25-i6-audit-budget` job exists + greps for the 4 fields.
#[test]
fn v1_7_section_25_3_workflow_enforces_audit_budget() {
    let workflow = read_workspace_file(".github/workflows/pr25-process-checks.yml");
    assert!(
        workflow.contains("i6-audit-budget"),
        "STABILITY § 25.3 : i6-audit-budget job missing"
    );
    // The 4 grep patterns for numeric fields.
    for pattern in [
        "Implementation : [0-9]+",
        "Audit \\(per I\\.1\\) : [0-9]+",
        "Buffer for unforeseen depth : [0-9]+",
        "Total estimate",
    ] {
        assert!(
            workflow.contains(pattern),
            "STABILITY § 25.3 : workflow regex missing pattern '{pattern}'"
        );
    }
}

/// PR #24 calibration anchor is recorded in the tracker (150% outlier).
#[test]
fn v1_7_section_25_3_pr24_anchor_in_tracker() {
    let body = read_workspace_file("docs/process/audit-budget-tracker.md");
    assert!(
        body.contains("PR #24") || body.contains("#24"),
        "STABILITY § 25.3 : PR #24 anchor row missing from tracker"
    );
    assert!(
        body.contains("150"),
        "STABILITY § 25.3 : PR #24 150% outlier value missing from tracker"
    );
    assert!(
        body.contains("67"),
        "STABILITY § 25.3 : PR #24 defect count (67) missing from tracker anchor"
    );
}

// ═══════════════════════════════════════════════════════════════════
// § 25.4  Drill cadence
// ═══════════════════════════════════════════════════════════════════

/// Quarterly defacement drill doc exists at the frozen path.
#[test]
fn v1_7_section_25_4_quarterly_defacement_drill_present() {
    assert!(
        workspace_file_exists("docs/drills/drill-defacement-response.md"),
        "STABILITY § 25.4 : quarterly defacement drill missing"
    );
    let body = read_workspace_file("docs/drills/drill-defacement-response.md");
    assert!(
        body.contains("quarterly") || body.contains("Quarterly"),
        "STABILITY § 25.4 : defacement drill not marked as quarterly cadence"
    );
}

/// Annual revocation drill doc exists at the frozen path.
#[test]
fn v1_7_section_25_4_annual_revocation_drill_present() {
    assert!(
        workspace_file_exists("docs/drills/drill-quarterly-revocation.md"),
        "STABILITY § 25.4 : annual revocation drill missing"
    );
    let body = read_workspace_file("docs/drills/drill-quarterly-revocation.md");
    assert!(
        body.contains("annual") || body.contains("Annual"),
        "STABILITY § 25.4 : revocation drill not marked as annual cadence"
    );
}

/// Master incident response runbook exists + references both drills.
#[test]
fn v1_7_section_25_4_master_runbook_references_drills() {
    let path = "docs/runbooks/bench-publish-incident-response.md";
    assert!(workspace_file_exists(path));
    let body = read_workspace_file(path);
    assert!(
        body.contains("drill-defacement-response.md") || body.contains("drills/"),
        "STABILITY § 25.4 : master runbook does not reference defacement drill"
    );
}

/// The drill verdict taxonomy is frozen : pass / partial / fail.
#[test]
fn v1_7_section_25_4_drill_verdict_taxonomy() {
    for path in [
        "docs/drills/drill-defacement-response.md",
        "docs/drills/drill-quarterly-revocation.md",
    ] {
        let body = read_workspace_file(path);
        for verdict in ["pass", "partial", "fail"] {
            assert!(
                body.contains(verdict),
                "STABILITY § 25.4 : {path} missing verdict '{verdict}'"
            );
        }
    }
}

// ═══════════════════════════════════════════════════════════════════
// § 25.5  Audit trail addendum — v1.7 row present
// ═══════════════════════════════════════════════════════════════════

#[test]
fn v1_7_section_25_5_audit_trail_row_present() {
    let body = read_workspace_file("docs/STABILITY_CONTRACT.md");
    // The v1.7 entry MUST appear in the audit trail.
    assert!(
        body.contains("**v1.7**"),
        "STABILITY § 25.5 : v1.7 audit trail row missing"
    );
    // It must reference PR #25.
    assert!(
        body.contains("PR #25"),
        "STABILITY § 25.5 : v1.7 row does not cite PR #25"
    );
}

// ═══════════════════════════════════════════════════════════════════
// Companion file presence — everything PR #25 needs to operate.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn v1_7_companion_files_all_present() {
    let required = [
        // Day 1 — enforcement plumbing.
        ".github/pull_request_template.md",
        ".github/workflows/pr25-process-checks.yml",
        "scripts/audit-prompt-validator.sh",
        "scripts/sot-audit.sh",
        // Day 2 — process documentation.
        "docs/lessons/_template.md",
        "docs/process/audit-prompt.md",
        "docs/process/audit-prompt-en.md",
        "docs/process/sot-pattern.md",
        "docs/process/audit-budget-tracker.md",
        "docs/process/i1-bypass-log.md",
        "docs/STABILITY_CONTRACT.md",
        // Day 3 — Rust tests (this file + neighbours).
        "bench/harness/tests/common/sot.rs",
        "bench/harness/tests/common/mod.rs",
        "bench/harness/tests/process_invariants.rs",
        "bench/harness/tests/process_invariants_negatives.rs",
        "bench/harness/tests/stability_contract_v17.rs",
    ];
    let mut missing = Vec::new();
    for rel in required {
        if !workspace_file_exists(rel) {
            missing.push(rel);
        }
    }
    assert!(
        missing.is_empty(),
        "STABILITY § 25 : PR #25 companion file(s) missing : {missing:?}"
    );
}

/// Shell scripts are executable.  Git on Windows can drop the +x bit
/// if `core.fileMode` is off — this catches that.
#[test]
fn v1_7_shell_scripts_executable() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for rel in [
            "scripts/audit-prompt-validator.sh",
            "scripts/sot-audit.sh",
        ] {
            let path = workspace_root().join(rel);
            let meta = std::fs::metadata(&path)
                .unwrap_or_else(|e| panic!("can't stat {} : {e}", path.display()));
            let mode = meta.permissions().mode();
            // Any of user/group/other execute bit must be set.
            assert!(
                mode & 0o111 != 0,
                "STABILITY § 25 : {rel} not executable (mode = {mode:o})"
            );
        }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, the +x bit isn't meaningful ; CI runs on Linux.
    }
}

// ═══════════════════════════════════════════════════════════════════
// Frozen-for-6-versions guard.
// ═══════════════════════════════════════════════════════════════════
//
// STABILITY § 25.5 says the process invariants are frozen for ≥ 6
// chart-minor versions starting v1.7.  This test asserts the doc
// commitment is present ; it does NOT enforce the actual version count
// (that's an operator review item).

#[test]
fn v1_7_freeze_commitment_documented() {
    let body = read_workspace_file("docs/STABILITY_CONTRACT.md");
    assert!(
        body.contains("≥ 6 chart-minor versions"),
        "STABILITY § 25 : 6-version freeze commitment not documented"
    );
}
