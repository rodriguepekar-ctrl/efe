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

// ═══════════════════════════════════════════════════════════════════
// PR #26 — § 26 post-mortem template CI enforcement freeze tests.
// ═══════════════════════════════════════════════════════════════════
//
// Six tests, one per § 26.1 — § 26.6.  Each test loads the
// corresponding artefact and asserts that its surface matches the
// frozen expectation.  A drift in any of the six surfaces fails CI
// on the PR that introduces the drift, before review.
//
// References :
//   * docs/STABILITY_CONTRACT.md § 26
//   * docs/architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md § 5.5

/// § 26.1 — the eleven canonical sections.  Present, in order, in the
/// canonical template at `docs/postmortems/_template.md`.
#[test]
fn v1_7_section_26_1_pm_canonical_sections() {
    let template = read_workspace_file("docs/postmortems/_template.md");
    let canonical = [
        "## 1. Summary",
        "## 2. Timeline",
        "## 3. Root cause",
        "## 4. Contributing factors",
        "## 5. What went well",
        "## 6. What went wrong",
        "## 7. Action items",
        "## 8. Prevention",
        "## 9. Customer impact",
        "## 10. Compliance ramifications",
        "## 11. Cross-references",
    ];
    let mut cursor: usize = 0;
    for section in canonical {
        let where_at = template[cursor..].find(section).unwrap_or_else(|| {
            panic!("STABILITY § 26.1 : canonical section heading not found in canonical order — '{}' (cursor was at byte {})", section, cursor)
        });
        cursor += where_at + section.len();
    }
}

/// § 26.2 — the YAML frontmatter schema.  Nine mandatory fields
/// (incident_id, severity, three timestamps, author, reviewers,
/// github_issue, status) present in the canonical template's
/// frontmatter example.
#[test]
fn v1_7_section_26_2_pm_frontmatter_schema() {
    let template = read_workspace_file("docs/postmortems/_template.md");
    let mandatory_fields = [
        "incident_id:",
        "severity:",
        "detected_at:",
        "mitigated_at:",
        "resolved_at:",
        "author:",
        "reviewers:",
        "github_issue:",
        "status:",
    ];
    for field in mandatory_fields {
        assert!(
            template.contains(field),
            "STABILITY § 26.2 : mandatory frontmatter field '{}' missing from canonical template",
            field
        );
    }
}

/// § 26.3 — the validator's stable exit-code contract.  Codes 0, 2,
/// 3, 4, 5, 6, 7 documented in the script's documentation block.
#[test]
fn v1_7_section_26_3_pm_validator_exit_codes() {
    let validator = read_workspace_file("scripts/postmortem-validator.sh");
    // Each exit code must be documented in the script's header block.
    // We assert the textual presence of each numeric code in the
    // documentation context.  This deliberately fragile check is the
    // freeze : changing an exit code requires editing the doc, which
    // is the moment a STABILITY-bump conversation should happen.
    for code in ["0  validation", "2  file argument", "3  frontmatter",
                  "4  one or more canonical", "5  Timeline", "6  Action items",
                  "7  Compliance"] {
        assert!(
            validator.contains(code),
            "STABILITY § 26.3 : exit-code documentation line containing '{}' not found in validator script",
            code
        );
    }
    // Also exit-code 1 must NOT be reachable (it's reserved).  We
    // assert the script's `exit` statements use only the stable codes.
    let allowed_exits: &[&str] = &["exit 0", "exit 2", "exit 3", "exit 4",
                                    "exit 5", "exit 6", "exit 7"];
    for line in validator.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("exit ") && !trimmed.starts_with("exit code") {
            // Cleanly skip any "exit ${VAR}" patterns ; only check literal numeric exits.
            if trimmed.chars().any(|c| c == '$') { continue; }
            assert!(
                allowed_exits.iter().any(|a| trimmed.starts_with(a)),
                "STABILITY § 26.3 : non-canonical exit statement in validator : '{}'",
                trimmed
            );
        }
    }
}

/// § 26.4 — the deadline algorithm.  Severity windows (7/14/30
/// calendar days) documented in the STABILITY contract, AND the
/// reference implementation script must exist + match the
/// contract.  This is the freeze that catches drift between the
/// contract's textual claims and the workflow's actual behaviour.
#[test]
fn v1_7_section_26_4_pm_deadline_algorithm() {
    let contract = read_workspace_file("docs/STABILITY_CONTRACT.md");
    // The three severity → window mappings must each appear in the
    // contract.  The exact phrasing is part of the freeze.
    for required in ["severity == S1 ? 7",
                      "severity == S2 ? 14",
                      "severity == S3 ? 30"] {
        assert!(
            contract.contains(required),
            "STABILITY § 26.4 : deadline algorithm clause '{}' missing from contract",
            required
        );
    }
    // The weekend-shift rule is part of the freeze.
    assert!(
        contract.contains("next Monday at 23:59:59 UTC"),
        "STABILITY § 26.4 : weekend-shift rule missing from contract"
    );

    // The reference implementation script MUST exist and be
    // executable.  This is the property that closes the
    // contract-vs-implementation drift caught by round-2 audit D-J.
    let script = workspace_root().join("scripts/postmortem-deadline.sh");
    assert!(
        script.exists(),
        "STABILITY § 26.4 : reference impl 'scripts/postmortem-deadline.sh' missing"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&script)
            .expect("script metadata must read")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "STABILITY § 26.4 : 'scripts/postmortem-deadline.sh' not executable (mode = {:o})",
            mode
        );
    }
    // The script's documentation block MUST cite the same three
    // severity windows + the shift rules.  Drift between the
    // script's doc and STABILITY § 26.4 fails this assertion.
    let script_body = read_workspace_file("scripts/postmortem-deadline.sh");
    for required in ["S1 \u{2192} 7",
                      "S2 \u{2192} 14",
                      "S3 \u{2192} 30"] {
        assert!(
            script_body.contains(required),
            "STABILITY § 26.4 : deadline script's doc block missing '{}'",
            required
        );
    }
    // The workflow MUST invoke the script (not inline the algorithm).
    let workflow = read_workspace_file(".github/workflows/pr26-postmortem-checks.yml");
    assert!(
        workflow.contains("scripts/postmortem-deadline.sh"),
        "STABILITY § 26.4 : workflow must invoke scripts/postmortem-deadline.sh, not inline the algorithm"
    );
}

/// § 26.5 — the filename convention.  `docs/postmortems/INC-YYYY-NNN.md`
/// for single incidents, `INC-YYYY-NNN+MMM.md` for multi-incident.
#[test]
fn v1_7_section_26_5_pm_filename_convention() {
    let contract = read_workspace_file("docs/STABILITY_CONTRACT.md");
    assert!(
        contract.contains("docs/postmortems/<incident_id>.md"),
        "STABILITY § 26.5 : filename convention 'docs/postmortems/<incident_id>.md' missing from contract"
    );
    assert!(
        contract.contains("INC-YYYY-NNN+MMM.md"),
        "STABILITY § 26.5 : multi-incident filename convention missing from contract"
    );
    // The validator must accept the multi-incident shape ; the regex
    // pattern in the script is the locked artefact.
    let validator = read_workspace_file("scripts/postmortem-validator.sh");
    assert!(
        validator.contains("^INC-[0-9]{4}-[0-9A-Z]+(\\+[0-9A-Z]+)*$"),
        "STABILITY § 26.5 : incident_id regex pattern missing from validator"
    );
}

/// § 26.6 — the three bypass classes : MISLABEL, MERGED, DRILL.
#[test]
fn v1_7_section_26_6_pm_bypass_classes() {
    let contract = read_workspace_file("docs/STABILITY_CONTRACT.md");
    for class in ["Class 1 : MISLABEL", "Class 2 : MERGED", "Class 3 : DRILL"] {
        assert!(
            contract.contains(class),
            "STABILITY § 26.6 : bypass class '{}' missing from contract",
            class
        );
    }
    // The bypass-log file must exist (its content schema is part of
    // the freeze, even though the entries are append-only).
    let bypass_log_path = workspace_root().join("docs/process/postmortem-bypass-log.md");
    assert!(
        bypass_log_path.exists(),
        "STABILITY § 26.6 : bypass log file 'docs/process/postmortem-bypass-log.md' missing"
    );
    // The drill-label exemption must be implemented in the workflow.
    let workflow = read_workspace_file(".github/workflows/pr26-postmortem-checks.yml");
    assert!(
        workflow.contains("drill-class — exempt from deadline check")
            || workflow.contains("is_drill"),
        "STABILITY § 26.6 : drill-label exemption logic missing from workflow"
    );
}

// ═══════════════════════════════════════════════════════════════════
// § 27 — CB-suite catalogue freeze tests.
// ═══════════════════════════════════════════════════════════════════
//
// The 35 CB benchmark identifiers + their existence on disk + their
// `requires_meta_swarm: false` invariant.  Drift in any of these
// triggers a contract violation.  Widened from 15 to 35 in the
// deep-coverage expansion ; see § 27.5 audit-trail row 2026-05-17
// (deep-coverage row).
//
// References :
//   * docs/STABILITY_CONTRACT.md § 27
//   * docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md

/// § 27.1 — the 35 CB benchmark identifiers, frozen + closed.
/// Widened from 15 to 35 in the deep-coverage expansion : the audit
/// surfaced ten previously-uncovered crates, eight top-level
/// subsystems and two packaging artefacts.  See STABILITY § 27.5
/// audit-trail row dated 2026-05-17 (deep-coverage row).
#[test]
fn v1_7_section_27_1_cb_catalogue_identifiers() {
    let canonical: [&str; 35] = [
        "CB-01_events_append_throughput",
        "CB-02_telemetry_ingestion_rate",
        "CB-03_vector_query_latency",
        "CB-04_kg_query_latency",
        "CB-05_cache_hit_rate",
        "CB-06_server_api_latency",
        "CB-07_region_routing_latency",
        "CB-08_saga_step_latency",
        "CB-09_llm_router_decision_latency",
        "CB-10_prompt_rendering_throughput",
        "CB-11_pii_redaction_throughput",
        "CB-12_billing_rollup_computation",
        "CB-13_retrieval_hybrid",
        "CB-14_multimodal_fusion",
        "CB-15_curation_pipeline",
        "CB-16_active_learning_sampling",
        "CB-17_backup_dr_orchestration",
        "CB-18_admin_tenant_onboarding",
        "CB-19_agent_runner_firecracker_cold_start",
        "CB-20_kernels_cuda_throughput",
        "CB-21_core_orchestration_primitives",
        "CB-22_metering_ingestion_standalone",
        "CB-23_ffi_pyo3_call_overhead",
        "CB-24_mcp_server_tool_invocation",
        "CB-25_slos_rollup_throughput",
        "CB-26_publish_quarterly_pdf",
        "CB-27_eval_harness_throughput",
        "CB-28_agent_core_lifecycle",
        "CB-29_soak_sustained_load",
        "CB-30_dr_drill_simulator",
        "CB-A1_harness_cli_cold_start",
        "CB-A2_workspace_test_duration",
        "CB-A3_cluster_provisioner_cold_start",
        "CB-A4_helm_chart_install_time",
        "CB-A5_docker_image_build_time",
    ];
    let scenarios_dir = workspace_root().join("bench/scenarios");
    // Every canonical identifier must have a corresponding YAML file.
    for id in &canonical {
        let p = scenarios_dir.join(format!("{}.yaml", id));
        assert!(
            p.exists(),
            "STABILITY § 27.1 : scenario file for '{}' missing",
            id
        );
    }
    // Conversely, every CB-prefixed YAML in the directory must be in
    // the canonical set.  No silent additions.
    let mut found: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&scenarios_dir)
        .expect("scenarios dir must read")
    {
        let entry = entry.expect("dir entry must read");
        let name = entry.file_name().into_string().unwrap_or_default();
        if let Some(stem) = name.strip_suffix(".yaml") {
            if stem.starts_with("CB-") {
                found.push(stem.to_string());
            }
        }
    }
    found.sort();
    let mut expected: Vec<String> = canonical.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        found, expected,
        "STABILITY § 27.1 : CB-prefixed scenarios on disk do not match the frozen catalogue"
    );
}

/// § 27.1 — every CB scenario MUST declare `requires_meta_swarm: false`.
#[test]
fn v1_7_section_27_1_cb_no_meta_swarm_dependency() {
    let scenarios_dir = workspace_root().join("bench/scenarios");
    for entry in std::fs::read_dir(&scenarios_dir)
        .expect("scenarios dir must read")
    {
        let entry = entry.expect("dir entry must read");
        let name = entry.file_name().into_string().unwrap_or_default();
        if !name.starts_with("CB-") || !name.ends_with(".yaml") { continue; }
        let body = std::fs::read_to_string(entry.path())
            .expect("scenario file must read");
        assert!(
            body.contains("requires_meta_swarm: false"),
            "STABILITY § 27.1 : {} must declare 'requires_meta_swarm: false'",
            name
        );
        assert!(
            !body.contains("requires_meta_swarm: true"),
            "STABILITY § 27.1 : {} declares 'requires_meta_swarm: true' (forbidden in CB suite)",
            name
        );
    }
}

/// § 27.3 — the CB scenario YAML schema : every scenario MUST carry
/// the mandatory top-level fields.
#[test]
fn v1_7_section_27_3_cb_scenario_schema() {
    let scenarios_dir = workspace_root().join("bench/scenarios");
    let mandatory_fields = [
        "schema_version: 1",
        "suite: CB",
        "id:",
        "family:",
        "subsystem:",
        "description:",
        "duration:",
        "requires_meta_swarm:",
        "loadgen:",
        "results:",
        "tolerance:",
    ];
    for entry in std::fs::read_dir(&scenarios_dir)
        .expect("scenarios dir must read")
    {
        let entry = entry.expect("dir entry must read");
        let name = entry.file_name().into_string().unwrap_or_default();
        if !name.starts_with("CB-") || !name.ends_with(".yaml") { continue; }
        let body = std::fs::read_to_string(entry.path())
            .expect("scenario file must read");
        for field in &mandatory_fields {
            assert!(
                body.contains(field),
                "STABILITY § 27.3 : {} missing mandatory field '{}'",
                name, field
            );
        }
    }
}

/// § 27.3 — the validator + the orchestrator scripts exist + are
/// executable + invoke each other.
#[test]
fn v1_7_section_27_3_cb_scripts_present_and_executable() {
    for rel in &["scripts/cb-scenario-validator.sh",
                  "scripts/run-core-benchmarks.sh"] {
        let p = workspace_root().join(rel);
        assert!(p.exists(), "STABILITY § 27.3 : '{}' missing", rel);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p)
                .expect("script must read")
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "STABILITY § 27.3 : '{}' not executable (mode = {:o})",
                rel, mode
            );
        }
    }
    // The orchestrator must reference the catalogue's 35 identifiers.
    // Sample one from each end of each numeric block (originals,
    // deep-coverage CB-13..CB-30, packaging artefacts CB-A4/A5) to
    // catch a regression that drops a whole block silently.
    let orch = read_workspace_file("scripts/run-core-benchmarks.sh");
    for id in &["CB-01_events_append_throughput",
                 "CB-12_billing_rollup_computation",
                 "CB-13_retrieval_hybrid",
                 "CB-22_metering_ingestion_standalone",
                 "CB-23_ffi_pyo3_call_overhead",
                 "CB-30_dr_drill_simulator",
                 "CB-A3_cluster_provisioner_cold_start",
                 "CB-A5_docker_image_build_time"] {
        assert!(
            orch.contains(id),
            "STABILITY § 27.3 : orchestrator missing identifier '{}'",
            id
        );
    }
}

/// § 27.4 — verdict taxonomy : identical to S-suite (§ 24.3).
/// Mostly a documentation check : the catalogue references the
/// PASS / WARN / FAIL / INCONCLUSIVE four-state taxonomy.
#[test]
fn v1_7_section_27_4_cb_verdict_taxonomy() {
    let catalogue = read_workspace_file(
        "docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md"
    );
    for state in &["PASS", "WARN", "FAIL", "INCONCLUSIVE"] {
        assert!(
            catalogue.contains(state),
            "STABILITY § 27.4 : verdict state '{}' not documented in catalogue",
            state
        );
    }
}

/// § 27.5 — audit-trail addendum row present in the contract.
#[test]
fn v1_7_section_27_5_cb_audit_trail_row_present() {
    let contract = read_workspace_file("docs/STABILITY_CONTRACT.md");
    assert!(
        contract.contains("CB catalogue introduced"),
        "STABILITY § 27.5 : audit-trail row for CB catalogue missing"
    );
}

// ═══════════════════════════════════════════════════════════════════
// PR #27 — G.11 closure : `tolerance_used` populated by verdict
// aggregator.
//
// PHASE 1 STATUS — PREFLIGHT
//   All three tests below are committed as STUBS that fail with
//   `unimplemented!()`.  They become real freeze enforcement in
//   Phase 2 (S2 day 5), once the resolver / emit / NDJSON v3 schema
//   are implemented.  Their structural presence in S1 is the
//   gap-tracking discipline : the freeze tests for § 24.3.6 +
//   § 27.2.5/6/7 exist *before* the contract amendments land,
//   pinning the intent.
//
//   Companion fixtures :
//     bench/fixtures/scenarios/G11_synthetic.yaml
//     bench/fixtures/post_g11_ndjson/*.ndjson
//
//   Companion tests :
//     bench/harness/tests/g11_round_trip.rs                    (pinning)
//     bench/harness/tests/g11_tolerance_used_populated.rs      (integration)
//
//   References :
//     docs/architecture/EFE_A1_PHASE3_PR27_ARCHITECTURE.md § 7.5, § 8
//     docs/roadmap/EFE_A1_PHASE3_PR27_ROADMAP.md            § 3.1 (preflight D4)
// ═══════════════════════════════════════════════════════════════════

/// § 24.3.6 — every NDJSON row in `bench/fixtures/post_g11_ndjson/`
/// MUST contain a `tolerance_used` field conforming to the v3 shape
/// (`value`, `unit`, `direction`, `source`, `applied_at`, optional
/// `override_reason`).  A `value: null` is permitted *only* when
/// `source = "no_tolerance_defined"`.  The double-implication holds.
#[test]
fn v1_7_section_24_3_6_tolerance_used_shape() {
    use efe_bench_harness::compare::MetricVerdictV3;
    let fixtures_dir = workspace_root().join("bench/fixtures/post_g11_ndjson");
    assert!(
        fixtures_dir.is_dir(),
        "STABILITY § 24.3.6 : post-G.11 NDJSON fixtures directory missing : {}",
        fixtures_dir.display()
    );

    let mut row_count = 0;
    for entry in std::fs::read_dir(&fixtures_dir).expect("read fixtures dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ndjson") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("read fixture");
        for (line_idx, line) in body.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let row: MetricVerdictV3 = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!(
                    "STABILITY § 24.3.6 : {} line {} fails v3 parse : {e}",
                    path.display(),
                    line_idx + 1
                )
            });
            // The shape contract is enforced by the validate method.
            row.tolerance_used
                .validate_internal_consistency()
                .unwrap_or_else(|e| {
                    panic!(
                        "STABILITY § 24.3.6 : {} line {} invariant : {e}",
                        path.display(),
                        line_idx + 1
                    )
                });
            row_count += 1;
        }
    }
    assert!(
        row_count >= 8,
        "STABILITY § 24.3.6 : expected ≥ 8 NDJSON rows across fixtures, got {row_count}"
    );
}

/// § 27.2.5 / § 27.2.6 / § 27.2.7 — closed taxonomies for `unit`,
/// `direction`, `source` MUST be respected by every NDJSON row in
/// `bench/fixtures/post_g11_ndjson/`.  Any value outside the closed
/// taxonomy is a freeze violation requiring a STABILITY-bump PR.
#[test]
fn v1_7_section_27_2_taxonomy_closed() {
    use efe_bench_harness::compare::MetricVerdictV3;
    let fixtures_dir = workspace_root().join("bench/fixtures/post_g11_ndjson");
    // The closed allowed-value sets, hard-coded here to detect drift
    // between the contract wording (§ 27.2.5/6/7) and the Rust enums.
    let allowed_units: &[&str] = &[
        "ms",
        "sec",
        "ns",
        "pct",
        "bytes",
        "events_per_sec",
        "count",
        "ratio",
    ];
    let allowed_directions: &[&str] =
        &["no_more_than", "no_less_than", "within_range", "exactly"];
    let allowed_sources: &[&str] = &[
        "yaml",
        "runtime_override",
        "gpu_runner_override",
        "defaulted",
        "no_tolerance_defined",
        "pre_g11_unbackfilled",
        "backfilled_yaml",
        "backfilled_runtime_override",
    ];

    for entry in std::fs::read_dir(&fixtures_dir).expect("read fixtures dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ndjson") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("read fixture");
        for (line_idx, line) in body.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            // Parse as raw JSON to inspect the wire-level strings, which
            // is what the contract actually freezes.
            let raw: serde_json::Value =
                serde_json::from_str(line).expect("parse JSON");
            let tu = &raw["tolerance_used"];

            let unit_str = tu["unit"].as_str().unwrap_or("");
            assert!(
                allowed_units.contains(&unit_str),
                "STABILITY § 27.2.5 : {} line {} : unit '{unit_str}' not in closed taxonomy",
                path.display(),
                line_idx + 1
            );

            let dir_str = tu["direction"].as_str().unwrap_or("");
            assert!(
                allowed_directions.contains(&dir_str),
                "STABILITY § 27.2.6 : {} line {} : direction '{dir_str}' not in closed taxonomy",
                path.display(),
                line_idx + 1
            );

            let src_str = tu["source"].as_str().unwrap_or("");
            assert!(
                allowed_sources.contains(&src_str),
                "STABILITY § 27.2.7 : {} line {} : source '{src_str}' not in closed taxonomy",
                path.display(),
                line_idx + 1
            );

            // Round-trip through MetricVerdictV3 to ensure the Rust
            // enums match the wire strings.
            let _row: MetricVerdictV3 = serde_json::from_str(line)
                .expect("v3 parse must succeed when taxonomies are respected");
        }
    }
}

/// § 24.3.6 — the double-implication invariant holds at the wire
/// level for every NDJSON row in `bench/fixtures/post_g11_ndjson/`.
///
/// The invariant : `tolerance_used.value == null  ⇔
///   tolerance_used.source ∈ {"no_tolerance_defined", "pre_g11_unbackfilled"}`.
///
/// Day 3 addition : this freeze test checks the invariant at the
/// JSON wire-level (parsing as raw `serde_json::Value`) in addition to
/// the typed `MetricVerdictV3` invariant check in
/// `v1_7_section_24_3_6_tolerance_used_shape`.  Two independent paths
/// of the same property prevent drift between the Rust enum encoding
/// and the wire-level string encoding.
#[test]
fn v1_7_section_24_3_6_double_implication_wire_level() {
    let fixtures_dir = workspace_root().join("bench/fixtures/post_g11_ndjson");
    let null_sources = ["no_tolerance_defined", "pre_g11_unbackfilled"];

    let mut rows_checked = 0;
    for entry in std::fs::read_dir(&fixtures_dir).expect("read fixtures dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ndjson") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("read fixture");
        for (line_idx, line) in body.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            rows_checked += 1;
            let raw: serde_json::Value = serde_json::from_str(line).expect("parse JSON");
            let tu = &raw["tolerance_used"];

            let value_is_null = tu["value"].is_null();
            let source_str = tu["source"].as_str().unwrap_or("");
            let source_allows_null = null_sources.contains(&source_str);

            assert_eq!(
                value_is_null, source_allows_null,
                "STABILITY § 24.3.6 : {} line {} violates double-implication \
                 (value_null={}, source='{source_str}', source_allows_null={})",
                path.display(),
                line_idx + 1,
                value_is_null,
                source_allows_null
            );
        }
    }
    assert!(
        rows_checked >= 8,
        "STABILITY § 24.3.6 : expected ≥ 8 NDJSON rows for wire-level \
         double-implication check, got {rows_checked}"
    );
}

/// § 24.3.6 — the G.11 pinning test file MUST physically exist
/// under `bench/harness/tests/`.  Meta-test : asserts that
/// `g11_round_trip.rs` is committed and references the canonical
/// fixture at `bench/fixtures/scenarios/G11_synthetic.yaml`.  This
/// is the structural guarantee that G.11 cannot regress silently.
#[test]
fn v1_7_section_24_3_6_g11_pinning_test_exists() {
    let test_path = workspace_root().join("bench/harness/tests/g11_round_trip.rs");
    assert!(
        test_path.is_file(),
        "STABILITY § 24.3.6 : the G.11 pinning test file must physically exist at {}",
        test_path.display()
    );
    let body = std::fs::read_to_string(&test_path).expect("read pinning test");
    assert!(
        body.contains("scenarios/G11_synthetic.yaml"),
        "STABILITY § 24.3.6 : pinning test must reference the canonical fixture path"
    );
    assert!(
        body.contains("ToleranceSource::Yaml"),
        "STABILITY § 24.3.6 : pinning test must exercise the Yaml source branch"
    );
    assert!(
        body.contains("ToleranceSource::RuntimeOverride"),
        "STABILITY § 24.3.6 : pinning test must exercise the RuntimeOverride source branch"
    );
    assert!(
        body.contains("ToleranceSource::GpuRunnerOverride"),
        "STABILITY § 24.3.6 : pinning test must exercise the GpuRunnerOverride source branch"
    );
    assert!(
        body.contains("ToleranceSource::NoToleranceDefined"),
        "STABILITY § 24.3.6 : pinning test must exercise the NoToleranceDefined source branch"
    );

    // Companion fixture must exist too.
    let fixture_path = workspace_root().join("bench/fixtures/scenarios/G11_synthetic.yaml");
    assert!(
        fixture_path.is_file(),
        "STABILITY § 24.3.6 : the canonical fixture must physically exist at {}",
        fixture_path.display()
    );
}
