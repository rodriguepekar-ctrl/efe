//! PR #26 § 5.2.5 — post-mortem schema validator integration tests.
//!
//! Exercises `scripts/postmortem-validator.sh` against the canonical
//! reference example (positive path) and against a comprehensive set
//! of deliberately-broken fixtures (negative path).
//!
//! Every test corresponds to a specific behaviour of the validator
//! described in the architecture document § 5.2.2 and locked under
//! STABILITY § 26.3 (the exit-code contract).
//!
//! Runtime : well under one second on standard CI hardware.
//!
//! References :
//!   * docs/architecture/EFE_A1_PHASE3_PR26_ARCHITECTURE.md § 5.2
//!   * docs/postmortems/_template.md          (canonical schema)
//!   * docs/postmortems/_VALIDATOR_REFERENCE.md (worked example)
//!   * docs/STABILITY_CONTRACT.md § 26.3        (frozen exit codes)
//!   * scripts/postmortem-validator.sh          (impl under test)

use std::fs;
use std::path::PathBuf;
use std::process::Command;

// ─────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root must canonicalize")
}

fn validator_path() -> PathBuf {
    workspace_root().join("scripts/postmortem-validator.sh")
}

fn reference_example_path() -> PathBuf {
    workspace_root().join("docs/postmortems/_VALIDATOR_REFERENCE.md")
}

/// Run the validator against `pm_path` and return its exit code.
fn run_validator(pm_path: &std::path::Path) -> i32 {
    let output = Command::new("bash")
        .arg(validator_path())
        .arg(pm_path)
        .env("NO_COLOR", "1")
        .output()
        .expect("validator must spawn");
    output.status.code().unwrap_or(-1)
}

/// Create a temp directory + write the given content into a file named
/// `INC-2026-099.md` (a non-underscore-prefixed name so the validator
/// actually runs against it rather than skipping it).
fn tempfile_with(content: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir must create");
    let path = dir.path().join("INC-2026-099.md");
    fs::write(&path, content).expect("write must succeed");
    (dir, path)
}

/// Load the reference example, mutate it via `f`, and write to a
/// tempfile.  Returns the temp directory (drops on test end) and the
/// path to the mutated file.
fn mutated_reference(f: impl FnOnce(String) -> String) -> (tempfile::TempDir, PathBuf) {
    let ref_content = fs::read_to_string(reference_example_path())
        .expect("reference example must be readable");
    tempfile_with(&f(ref_content))
}

// ─────────────────────────────────────────────────────────────────────
// Positive path
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_accepts_canonical_reference() {
    // Copy the underscore-prefixed reference into a non-underscore
    // tempfile so the validator's skip-list does not exempt it.
    let (_dir, path) = mutated_reference(|s| s);
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "the canonical reference example must validate cleanly (got exit {})",
        code
    );
}

#[test]
fn validator_skips_underscore_prefixed_files() {
    // The reference example IS underscore-prefixed in its real
    // location ; the validator's skip-list must exit 0 without
    // running any checks.
    let code = run_validator(&reference_example_path());
    assert_eq!(
        code, 0,
        "underscore-prefixed files must be skipped with exit 0 (got {})",
        code
    );
}

// ─────────────────────────────────────────────────────────────────────
// Check 1 — path validity (exit 2)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_rejects_nonexistent_path() {
    let bogus = PathBuf::from("/does/not/exist-INC-2026-099.md");
    let code = run_validator(&bogus);
    assert_eq!(code, 2, "non-existent path must exit 2 (got {})", code);
}

#[test]
fn validator_rejects_empty_file() {
    let (_dir, path) = tempfile_with("");
    let code = run_validator(&path);
    assert_eq!(code, 2, "empty file must exit 2 (got {})", code);
}

// ─────────────────────────────────────────────────────────────────────
// Check 2 — frontmatter (exit 3)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_rejects_missing_frontmatter() {
    let (_dir, path) = tempfile_with("# Just a heading\n\nNo frontmatter at all.\n");
    let code = run_validator(&path);
    assert_eq!(code, 3, "missing frontmatter must exit 3 (got {})", code);
}

#[test]
fn validator_rejects_missing_incident_id() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("incident_id: INC-2026-000\n", "")
    });
    let code = run_validator(&path);
    assert_eq!(code, 3, "missing incident_id must exit 3 (got {})", code);
}

#[test]
fn validator_rejects_malformed_incident_id() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("incident_id: INC-2026-000", "incident_id: not-an-id")
    });
    let code = run_validator(&path);
    assert_eq!(code, 3, "malformed incident_id must exit 3 (got {})", code);
}

#[test]
fn validator_accepts_multi_incident_id() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("incident_id: INC-2026-000", "incident_id: INC-2026-099+100")
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "multi-incident id (INC-2026-099+100) must be accepted (got {})",
        code
    );
}

#[test]
fn validator_rejects_invalid_severity() {
    let (_dir, path) = mutated_reference(|s| s.replace("severity: S2", "severity: S5"));
    let code = run_validator(&path);
    assert_eq!(code, 3, "invalid severity must exit 3 (got {})", code);
}

#[test]
fn validator_rejects_bad_timestamp() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("detected_at: 2026-03-14T09:07:00Z", "detected_at: yesterday-afternoon")
    });
    let code = run_validator(&path);
    assert_eq!(code, 3, "non-ISO8601 timestamp must exit 3 (got {})", code);
}

#[test]
fn validator_rejects_missing_resolved_at() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("resolved_at: 2026-03-14T10:18:00Z\n", "")
    });
    let code = run_validator(&path);
    assert_eq!(code, 3, "missing resolved_at must exit 3 (got {})", code);
}

#[test]
fn validator_rejects_single_reviewer() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace(
            "reviewers:\n  - sre-lead\n  - bench-infra-lead\n",
            "reviewers:\n  - sre-lead\n",
        )
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 3,
        "single reviewer (minimum is two) must exit 3 (got {})",
        code
    );
}

#[test]
fn validator_rejects_non_github_issue_url() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace(
            "github_issue: https://github.com/efe-ai/efe/issues/0",
            "github_issue: https://example.com/tickets/0",
        )
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 3,
        "non-github.com issue URL must exit 3 (got {})",
        code
    );
}

#[test]
fn validator_rejects_unknown_status() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("status: PUBLISHED", "status: MAYBE_DRAFT_PROBABLY")
    });
    let code = run_validator(&path);
    assert_eq!(code, 3, "unknown status must exit 3 (got {})", code);
}

#[test]
fn validator_accepts_published_retrospective_status() {
    // Backfilled post-mortems use PUBLISHED_RETROSPECTIVE per
    // architecture § 8.2.  The validator must accept it.
    let (_dir, path) = mutated_reference(|s| {
        s.replace("status: PUBLISHED", "status: PUBLISHED_RETROSPECTIVE")
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "PUBLISHED_RETROSPECTIVE must be accepted for backfills (got {})",
        code
    );
}

// ─────────────────────────────────────────────────────────────────────
// Check 3 — canonical sections (exit 4)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_rejects_missing_summary_section() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("## 1. Summary (= 3-4 sentences)\n", "## NotSummary\n")
    });
    let code = run_validator(&path);
    assert_eq!(code, 4, "missing § 1 must exit 4 (got {})", code);
}

#[test]
fn validator_rejects_missing_compliance_section() {
    let (_dir, path) = mutated_reference(|s| {
        s.replace("## 10. Compliance ramifications\n", "## TenSomethingElse\n")
    });
    let code = run_validator(&path);
    assert_eq!(code, 4, "missing § 10 must exit 4 (got {})", code);
}

#[test]
fn validator_rejects_wrong_section_order() {
    // Swap headings of § 4 and § 5.  The regex matches by section
    // number ("4." vs "5.") AND title, so renaming both should
    // surface as missing-sections.
    let (_dir, path) = mutated_reference(|s| {
        s.replace("## 4. Contributing factors", "TEMP_PLACEHOLDER_4")
            .replace("## 5. What went well", "## 4. Contributing factors")
            .replace("TEMP_PLACEHOLDER_4", "## 5. What went well")
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 4,
        "out-of-order sections must exit 4 (got {})",
        code
    );
}

// ─────────────────────────────────────────────────────────────────────
// Check 4 — Timeline (exit 5)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_rejects_empty_timeline() {
    // Replace the timeline table body with only the header + separator.
    let (_dir, path) = mutated_reference(|s| {
        let mut out = String::new();
        let mut in_timeline = false;
        let mut wrote_header_separator = false;
        for line in s.lines() {
            if line.starts_with("## 2. Timeline") {
                in_timeline = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_timeline && line.starts_with("## 3.") {
                in_timeline = false;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_timeline {
                if line.starts_with('|') {
                    if !wrote_header_separator {
                        out.push_str("| Time | Event |\n|------|-------|\n");
                        wrote_header_separator = true;
                    }
                    // Skip all data rows.
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    });
    let code = run_validator(&path);
    assert_eq!(code, 5, "empty timeline must exit 5 (got {})", code);
}

#[test]
fn validator_accepts_minimal_timeline() {
    // Exactly two rows is the documented minimum.  Build a fresh
    // timeline of exactly two rows and assert exit 0.
    let (_dir, path) = mutated_reference(|s| {
        let mut out = String::new();
        let mut in_timeline = false;
        let mut wrote_minimal = false;
        for line in s.lines() {
            if line.starts_with("## 2. Timeline") {
                in_timeline = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_timeline && line.starts_with("## 3.") {
                in_timeline = false;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_timeline {
                if line.starts_with('|') {
                    if !wrote_minimal {
                        out.push_str("| Time | Event |\n");
                        out.push_str("|------|-------|\n");
                        out.push_str("| `09:07` | First signal |\n");
                        out.push_str("| `10:18` | All-clear |\n");
                        wrote_minimal = true;
                    }
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "exactly 2 timeline rows must be accepted as the minimum (got {})",
        code
    );
}

// ─────────────────────────────────────────────────────────────────────
// Check 5 — Action items (exit 6)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_rejects_empty_action_items() {
    let (_dir, path) = mutated_reference(|s| {
        let mut out = String::new();
        let mut in_actions = false;
        let mut wrote_header_separator = false;
        for line in s.lines() {
            if line.starts_with("## 7. Action items") {
                in_actions = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_actions && line.starts_with("## 8.") {
                in_actions = false;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            if in_actions {
                if line.starts_with('|') {
                    if !wrote_header_separator {
                        out.push_str("| Action | Owner | Linear | Due | Status |\n");
                        out.push_str("|--------|-------|--------|-----|--------|\n");
                        wrote_header_separator = true;
                    }
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    });
    let code = run_validator(&path);
    assert_eq!(code, 6, "empty action items must exit 6 (got {})", code);
}

// ─────────────────────────────────────────────────────────────────────
// Check 6 — Compliance ramifications (exit 7)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_rejects_unaddressed_compliance_checkbox() {
    // Strip the "N/A" annotation from one checkbox line.
    let (_dir, path) = mutated_reference(|s| {
        s.replace(
            "- [x] DPO notified (≤ 24 h) — N/A : no personal data involved.",
            "- [ ] DPO notified (≤ 24 h)",
        )
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 7,
        "un-addressed compliance checkbox must exit 7 (got {})",
        code
    );
}

#[test]
fn validator_accepts_compliance_with_na_annotations() {
    // The reference example uses `[x]` + "N/A" everywhere.  Verify
    // explicitly that the validator counts the N/A annotation as
    // address-equivalent regardless of the leading checkbox state.
    let (_dir, path) = mutated_reference(|s| {
        // Convert all "[x] ... N/A" lines to "[ ] ... N/A" — i.e. an
        // unticked checkbox but with the N/A annotation present.
        s.lines()
            .map(|line| {
                if line.contains("- [x]") && line.contains("N/A") {
                    line.replacen("- [x]", "- [ ]", 1)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "N/A annotation alone (without tick) must be accepted (got {})",
        code
    );
}

// ─────────────────────────────────────────────────────────────────────
// Coverage : heading depth, em-dash caveats, multi-incident, unknown
// fields.  These tests close the post-audit gaps D-F, D-G, D-H, D-I.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn validator_accepts_h3_heading_depth() {
    // The validator's regex is `^##+` so any heading level ≥ 2 is
    // accepted.  An author who nests their post-mortem under a `#`
    // top-level heading and uses `###` for sections must validate.
    let (_dir, path) = mutated_reference(|s| {
        s.lines()
            .map(|line| {
                if line.starts_with("## ") {
                    format!("#{}", line) // promotes "## " to "### "
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "heading depth h3 (###) must be accepted (got {})",
        code
    );
}

#[test]
fn validator_accepts_em_dash_caveat_after_heading() {
    // Authors should be able to add a trailing em-dash-introduced
    // caveat after the canonical section title.  The regex uses
    // word-boundary `\b` so this should work.  The canonical
    // template already does this for several sections (e.g. "## 1.
    // Summary (= 3-4 sentences)"), so this test enforces that the
    // tolerance survives.
    let (_dir, path) = mutated_reference(|s| {
        // Add an em-dash caveat to § 3 (Root cause), which in the
        // canonical reference has no parenthetical.
        s.replace(
            "## 3. Root cause\n",
            "## 3. Root cause — one-paragraph statement\n",
        )
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "em-dash caveat after section heading must be accepted (got {})",
        code
    );
}

#[test]
fn validator_accepts_multi_incident_with_three_identifiers() {
    // The architecture doc § 26.5.2 allows multi-incident post-mortems
    // covering N ≥ 2 incidents.  Verify the validator accepts a
    // three-incident concatenation.  The reference example's frontmatter
    // is mutated to demonstrate.
    let (_dir, path) = mutated_reference(|s| {
        s.replace(
            "incident_id: INC-2026-000",
            "incident_id: INC-2026-007+008+009",
        )
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "multi-incident id with 3 identifiers must be accepted (got {})",
        code
    );
}

#[test]
fn validator_tolerates_unknown_frontmatter_fields() {
    // Forward-compatibility analogue : authors may add additional
    // frontmatter fields beyond the nine mandatory ones (e.g. a
    // future field like `pagerduty_incident_id`) and the validator
    // must continue to accept the file.  This test guards against a
    // regression that would tighten the parser to reject unknown
    // fields.
    let (_dir, path) = mutated_reference(|s| {
        s.replace(
            "status: PUBLISHED\n",
            "status: PUBLISHED\npagerduty_incident_id: PD-2026-XYZ\nadditional_metadata: experimental\n",
        )
    });
    let code = run_validator(&path);
    assert_eq!(
        code, 0,
        "unknown frontmatter fields must be tolerated for forward-compat (got {})",
        code
    );
}

// ─────────────────────────────────────────────────────────────────────
// Deadline algorithm — STABILITY § 26.4.
//
// These tests exercise `scripts/postmortem-deadline.sh` directly with
// canonical input / expected-output pairs.  The script is the
// reference implementation of the deadline-shift rule, and these
// tests are what protects the rule from drift between the
// STABILITY contract and the workflow that consumes it.
//
// The script reads the holiday list from
// `docs/process/efe-business-calendar.md` ; the cases below assume
// the 2026 Swedish holidays as documented in that file (Good Friday
// 2026-04-03, Easter Monday 2026-04-06, Labour Day 2026-05-01, etc.).
// If those dates change in the calendar file, these tests must be
// updated in lock-step.
// ─────────────────────────────────────────────────────────────────────

fn deadline_script_path() -> PathBuf {
    workspace_root().join("scripts/postmortem-deadline.sh")
}

fn run_deadline(severity: &str, closed_at: &str) -> (i32, String) {
    let output = Command::new("bash")
        .arg(deadline_script_path())
        .arg(severity)
        .arg(closed_at)
        .current_dir(workspace_root())
        .env("NO_COLOR", "1")
        .output()
        .expect("deadline script must spawn");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (code, stdout)
}

#[test]
fn deadline_s2_no_shift_needed() {
    // Mon 2026-03-02 + 14 days = Mon 2026-03-16.  No shift.
    let (code, out) = run_deadline("S2", "2026-03-02T15:00:00Z");
    assert_eq!(code, 0);
    assert_eq!(out, "2026-03-16",
        "S2 closed Mon 2026-03-02 should land on Mon 2026-03-16 unchanged");
}

#[test]
fn deadline_s2_no_shift_friday_to_friday() {
    // Fri 2026-03-06 + 14 days = Fri 2026-03-20.  No shift.
    let (code, out) = run_deadline("S2", "2026-03-06T15:00:00Z");
    assert_eq!(code, 0);
    assert_eq!(out, "2026-03-20",
        "S2 closed Fri 2026-03-06 should land on Fri 2026-03-20 unchanged");
}

#[test]
fn deadline_s2_saturday_shifts_to_monday() {
    // Sat 2026-03-07 + 14 days = Sat 2026-03-21 → shift +2 = Mon 2026-03-23.
    let (code, out) = run_deadline("S2", "2026-03-07T10:00:00Z");
    assert_eq!(code, 0);
    assert_eq!(out, "2026-03-23",
        "Saturday deadline must shift to Monday (+2 days)");
}

#[test]
fn deadline_s1_through_good_friday_cluster() {
    // Fri 2026-03-27 + 7 days = Fri 2026-04-03 (Good Friday, holiday)
    //   → +1 day = Sat 04-04 → +2 = Mon 04-06 (Easter Monday, holiday)
    //   → +1 day = Tue 04-07 (business day).
    let (code, out) = run_deadline("S1", "2026-03-27T15:00:00Z");
    assert_eq!(code, 0);
    assert_eq!(out, "2026-04-07",
        "S1 closed Fri 2026-03-27 must shift past Good Friday + weekend + Easter Monday to Tue 2026-04-07");
}

#[test]
fn deadline_s1_sunday_shifts_to_monday() {
    // Sun 2026-04-26 + 7 days = Sun 2026-05-03 → +1 day = Mon 2026-05-04.
    let (code, out) = run_deadline("S1", "2026-04-26T10:00:00Z");
    assert_eq!(code, 0);
    assert_eq!(out, "2026-05-04",
        "Sunday deadline must shift to Monday (+1 day)");
}

#[test]
fn deadline_s3_no_shift_long_window() {
    // Mon 2026-05-04 + 30 days = Wed 2026-06-03.  No shift.
    let (code, out) = run_deadline("S3", "2026-05-04T10:00:00Z");
    assert_eq!(code, 0);
    assert_eq!(out, "2026-06-03",
        "S3 30-day window from Mon 2026-05-04 lands on Wed 2026-06-03");
}

#[test]
fn deadline_rejects_invalid_severity() {
    let (code, _) = run_deadline("S5", "2026-03-02T10:00:00Z");
    assert_eq!(code, 2, "invalid severity must exit 2");
}

#[test]
fn deadline_rejects_malformed_date() {
    let (code, _) = run_deadline("S2", "definitely-not-a-date");
    assert_eq!(code, 2, "malformed closed_at must exit 2");
}

#[test]
fn deadline_accepts_plain_date_input() {
    // The script should also accept YYYY-MM-DD (no time component).
    let (code, out) = run_deadline("S2", "2026-03-02");
    assert_eq!(code, 0);
    assert_eq!(out, "2026-03-16",
        "plain YYYY-MM-DD input must produce the same deadline as the ISO-8601 form");
}
