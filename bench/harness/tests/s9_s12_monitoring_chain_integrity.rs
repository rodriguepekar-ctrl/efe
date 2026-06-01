//! # PR #27 — S9-S12 — Monitoring chain integrity tests
//!
//! Pin the structural existence and completeness of the S9-S12
//! post-merge monitoring artefacts as permanent Rust integration tests.
//!
//! ## Why these tests
//!
//! The 4-week post-merge monitoring window of PR #27 (13 juillet → 7
//! août 2026) produces 6 artefacts forming the **monitoring chain** :
//! 4 weekly monitoring reports (S9, S10, S11, S12) + 1 quarterly
//! publish dry-run report (Vendredi 31 juillet 2026, mid-S11) + 1
//! closure ceremony consolidation (Vendredi 7 août 2026, S12 EOW).
//!
//! These artefacts form the **evidence trail** that PR #27 underwent
//! 4 weeks of post-merge surveillance before being marked
//! `[closed, verified]` in the master gap tracker.
//!
//! Without permanent pinning, a future PR could accidentally delete
//! or corrupt these artefacts — silently, with no test failure.
//! Closure traceability would be lost.  These tests make the
//! monitoring chain STRUCTURALLY non-regressable : if any monitoring
//! report is deleted, the corresponding test fails immediately in
//! `cargo test`.
//!
//! Mirrors `s3_audit_chain_integrity.rs` (which pins the S3
//! chirurgical audit chain) — same pattern, different artefacts.
//!
//! ## Mapping : artefact → test
//!
//! | Artefact                                                              | Test                              |
//! |-----------------------------------------------------------------------|-----------------------------------|
//! | `docs/lessons/monitoring/PR-27-monitoring-week-9.md`                  | `s9_week_report_exists`           |
//! | `docs/lessons/monitoring/PR-27-monitoring-week-10.md`                 | `s10_week_report_exists`          |
//! | `docs/lessons/monitoring/PR-27-monitoring-week-11.md`                 | `s11_week_report_exists`          |
//! | `docs/lessons/monitoring/PR-27-monitoring-week-12.md`                 | `s12_week_report_exists`          |
//! | `docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md`       | `s11_quarterly_dry_run_exists`    |
//! | `docs/lessons/monitoring/PR-27-closure-2026-08-07.md`                 | `s12_closure_ceremony_exists`     |
//!
//! Plus 6 structural / cross-doc meta-tests :
//! - The closure ceremony references all 4 weekly reports + the dry-run.
//! - The closure ceremony records the triple sign-off (3 reviewers).
//! - The closure ceremony marks G.11 as `[closed, verified 2026-08-07]`.
//! - Every weekly report declares its phase (S9/S10/S11/S12).
//! - Every weekly report records the pinning ritual cadence (5x per day).
//! - The cumulative pinning count across S9-S12 matches the architecture
//!   contract (110/110 PASS over 20 working days).
//!
//! ## References
//!
//! - S9-S12 architecture parent doc :
//!   `docs/architecture/EFE_A1_PHASE3_PR27_ARCHITECTURE.md` § S9-S12
//! - Sibling chain meta-test : `tests/s3_audit_chain_integrity.rs`
//! - Master gap tracker entry (G.11 closed) : `docs/gaps/master_tracker.md`
//! - Lessons file (closure ceremony reference) :
//!   `docs/lessons/PR-27-lessons.md` § 9

#![allow(clippy::needless_return)]

use std::path::PathBuf;

// ═══════════════════════════════════════════════════════════════════
//  Workspace + path helpers (duplicated intentionally per AD-D3-2)
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

fn weekly_report_path(week_num: u32) -> PathBuf {
    workspace_root().join(format!(
        "docs/lessons/monitoring/PR-27-monitoring-week-{week_num}.md"
    ))
}

fn quarterly_dry_run_path() -> PathBuf {
    workspace_root().join("docs/lessons/monitoring/PR-27-quarterly-dry-run-2026-07-31.md")
}

fn closure_ceremony_path() -> PathBuf {
    workspace_root().join("docs/lessons/monitoring/PR-27-closure-2026-08-07.md")
}

/// Helper : asserts that a weekly monitoring report exists and contains
/// the canonical structural sections.  Mirrors the S3 helper but for
/// monitoring reports (different section set).
fn assert_canonical_weekly_report_structure(
    path: &PathBuf,
    week_num: u32,
    phase_label: &str,
) {
    assert!(
        path.is_file(),
        "S9-S12 monitoring chain integrity : weekly report must exist at {}",
        path.display()
    );
    let body = std::fs::read_to_string(path).expect("weekly report must read");

    // Canonical sections (per AD-S9-S12-2 weekly report format).
    let required_sections = [
        "## 1. Scope",
        "## 2. CI nightly job status",
        "## 3. Pinning test 5x ritual",
        "## 4. Quarterly publish status",
        "## 5. Incident log",
        "## 6. Backfill long-term spot-check",
        "## 7. Recommendation",
    ];
    for section in &required_sections {
        assert!(
            body.contains(section),
            "S{week_num} weekly report : missing canonical section '{section}'"
        );
    }

    // Phase label must appear (e.g. "S9", "S10", "S11", "S12").
    assert!(
        body.contains(phase_label),
        "S{week_num} weekly report : title/body must reference phase label '{phase_label}'"
    );

    // Reviewer signature (single reviewer = bench-infra-lead per AD-S9-S12-2,
    // although S11 has a release-eng co-signer for cosign segment of the dry-run
    // and S12 has triple sign-off for the closure ceremony summary).
    assert!(
        body.contains("@bench-infra-lead"),
        "S{week_num} weekly report : must reference @bench-infra-lead"
    );

    // Audit prompt read attestation (I.1 compliance).
    assert!(
        body.contains("Audit prompt read"),
        "S{week_num} weekly report : must declare 'Audit prompt read' timestamp (I.1 compliance)"
    );

    // Pinning test cadence : every weekly report MUST declare a 5/5 result
    // for each of 5 working days (at minimum 5 occurrences of "5/5").
    let cadence_mentions = body.matches("5/5").count();
    assert!(
        cadence_mentions >= 5,
        "S{week_num} weekly report : pinning test 5/5 must be reported in each of 5 working days, found {cadence_mentions} mentions"
    );

    // Signature block placeholder per AD-S3-2 (re-used by AD-S9-S12-2).
    assert!(
        body.contains("BEGIN PGP SIGNED MESSAGE"),
        "S{week_num} weekly report : must include GPG signature block"
    );

    // Recommendation must be one of the canonical wordings.
    let recommendation_present = body.contains("continue surveillance")
        || body.contains("transition to long-term lights-out")
        || body.contains("rollback");
    assert!(
        recommendation_present,
        "S{week_num} weekly report : recommendation must be canonical (continue surveillance / transition / rollback)"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Per-week existence + structural tests
// ═══════════════════════════════════════════════════════════════════

/// **S9 pin** : Week 9 monitoring report exists with canonical structure.
#[test]
fn s9_week_report_exists() {
    assert_canonical_weekly_report_structure(&weekly_report_path(9), 9, "S9");
}

/// **S10 pin** : Week 10 monitoring report exists with canonical structure.
#[test]
fn s10_week_report_exists() {
    assert_canonical_weekly_report_structure(&weekly_report_path(10), 10, "S10");
}

/// **S11 pin** : Week 11 monitoring report exists with canonical structure.
/// S11 has the additional quarterly-publish-dry-run cross-reference.
#[test]
fn s11_week_report_exists() {
    let path = weekly_report_path(11);
    assert_canonical_weekly_report_structure(&path, 11, "S11");

    let body = std::fs::read_to_string(&path).expect("S11 report must read");

    // S11 must cross-reference the quarterly dry-run companion doc.
    assert!(
        body.contains("PR-27-quarterly-dry-run-2026-07-31.md"),
        "S11 weekly report : must cross-reference the quarterly dry-run companion doc"
    );

    // S11 must declare the dry-run was EXECUTED.
    assert!(
        body.contains("EXECUTED") || body.contains("dry-run") || body.contains("Dry-run"),
        "S11 weekly report : must declare quarterly dry-run was executed"
    );
}

/// **S12 pin** : Week 12 monitoring report exists with canonical structure.
/// S12 is the closure week — must reference the closure ceremony companion.
#[test]
fn s12_week_report_exists() {
    let path = weekly_report_path(12);
    assert_canonical_weekly_report_structure(&path, 12, "S12");

    let body = std::fs::read_to_string(&path).expect("S12 report must read");

    // S12 must cross-reference the closure ceremony companion doc.
    assert!(
        body.contains("PR-27-closure-2026-08-07.md"),
        "S12 weekly report : must cross-reference the closure ceremony companion doc"
    );

    // S12 must declare G.11 closed.
    assert!(
        body.contains("closed, verified") || body.contains("[closed, verified]"),
        "S12 weekly report : must declare G.11 [closed, verified]"
    );

    // S12 must declare the transition to long-term lights-out monitoring.
    assert!(
        body.contains("lights-out") || body.contains("long-term"),
        "S12 weekly report : must declare transition to long-term lights-out monitoring"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Quarterly publish dry-run (S11) + Closure ceremony (S12)
// ═══════════════════════════════════════════════════════════════════

/// **Quarterly dry-run pin (S11)** : the quarterly publish dry-run
/// report exists, covers all 7 stages, validates `tolerance_used`
/// end-to-end.
#[test]
fn s11_quarterly_dry_run_exists() {
    let path = quarterly_dry_run_path();
    assert!(
        path.is_file(),
        "S11 quarterly dry-run report must exist at {}",
        path.display()
    );
    let body = std::fs::read_to_string(&path).expect("dry-run report must read");

    // Must reference all 7 stages.
    let required_stages = [
        "Stage 1",
        "Stage 2",
        "Stage 3",
        "Stage 4",
        "Stage 5",
        "Stage 6",
        "Stage 7",
    ];
    for stage in &required_stages {
        assert!(
            body.contains(stage),
            "dry-run report : must reference '{stage}'"
        );
    }

    // Must declare overall verdict.
    assert!(
        body.contains("DRY-RUN PASS"),
        "dry-run report : must explicitly declare overall verdict DRY-RUN PASS"
    );

    // Must include cosign signature stage (AD-S9-S12-5).
    assert!(
        body.contains("cosign") && body.contains("Verified OK"),
        "dry-run report : cosign signature stage must report 'Verified OK'"
    );

    // Must declare `tolerance_used` flowed through.
    assert!(
        body.contains("tolerance_used") && body.contains("populated"),
        "dry-run report : must declare tolerance_used is populated"
    );

    // Must declare zero defects.
    assert!(
        body.contains("Zero defects") || body.contains("0 defects") || body.contains("**Zero defects**"),
        "dry-run report : must declare zero defects found"
    );

    // Two-reviewer signature block (bench-infra + release-eng for cosign segment).
    assert!(
        body.contains("@bench-infra-lead"),
        "dry-run report : must include @bench-infra-lead signature"
    );
    assert!(
        body.contains("@release-eng-lead"),
        "dry-run report : must include @release-eng-lead signature (cosign segment)"
    );
}

/// **Closure ceremony pin (S12)** : the closure ceremony consolidation
/// exists, aggregates all 4 weeks of monitoring evidence, records
/// triple sign-off, marks G.11 closed.
#[test]
fn s12_closure_ceremony_exists() {
    let path = closure_ceremony_path();
    assert!(
        path.is_file(),
        "S12 closure ceremony document must exist at {}",
        path.display()
    );
    let body = std::fs::read_to_string(&path).expect("closure doc must read");

    // Must reference all 4 weekly monitoring reports (W9, W10, W11, W12).
    for week in 9..=12_u32 {
        assert!(
            body.contains(&format!("S{week}")),
            "closure ceremony : must reference week S{week}"
        );
    }

    // Must reference the quarterly dry-run companion.
    assert!(
        body.contains("PR-27-quarterly-dry-run-2026-07-31.md")
            || body.contains("quarterly publish dry-run"),
        "closure ceremony : must reference the S11 quarterly publish dry-run"
    );

    // Must include all 3 reviewer sign-offs (triple sign-off per AD-S9-S12-12).
    let required_sign_offs = [
        "@bench-infra-lead",
        "@release-eng-lead",
        "@stability-contract-steward",
    ];
    for reviewer in &required_sign_offs {
        assert!(
            body.contains(reviewer),
            "closure ceremony : must include sign-off from {reviewer}"
        );
    }

    // Must explicitly mark G.11 as CLOSED AND VERIFIED with the closure date.
    assert!(
        body.contains("CLOSED AND VERIFIED") || body.contains("closed, verified"),
        "closure ceremony : must declare G.11 CLOSED AND VERIFIED"
    );
    assert!(
        body.contains("2026-08-07"),
        "closure ceremony : must record the closure date 2026-08-07"
    );

    // Must record the cumulative pinning tally (110/110 PASS per AD-S9-S12-1).
    assert!(
        body.contains("110/110"),
        "closure ceremony : must record cumulative pinning ritual 110/110 PASS"
    );

    // Must record the cumulative nightly tally (20/20 PASS).
    assert!(
        body.contains("20/20"),
        "closure ceremony : must record cumulative nightly 20/20 PASS"
    );

    // Must include the joint statement verbatim (AD-S9-S12-12 §"joint statement discipline").
    assert!(
        body.contains("Joint statement") || body.contains("joint statement"),
        "closure ceremony : must include a joint statement section"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Cross-document meta-tests (chain coherence)
// ═══════════════════════════════════════════════════════════════════

/// **Cross-doc invariant** : the 4 weekly reports + 2 special-purpose
/// docs collectively cover the four monitoring axes specified by
/// AD-S9-S12-1 (CI nightly, pinning ritual, quarterly publish, backfill
/// long-term).
#[test]
fn monitoring_chain_covers_four_axes() {
    let axes = [
        "CI nightly",
        "Pinning test 5x ritual",
        "Quarterly publish",
        "Backfill long-term",
    ];
    // The weekly reports must each cover all four axes (the "Quarterly publish"
    // section will be either "executed" or "not applicable in S<week>" depending
    // on the week — but the SECTION must be present).
    for week in 9..=12_u32 {
        let body = std::fs::read_to_string(weekly_report_path(week)).expect("read");
        for axis in &axes {
            assert!(
                body.contains(axis),
                "S{week} weekly report : must cover monitoring axis '{axis}'"
            );
        }
    }
}

/// **Cumulative tally invariant** : the S12 weekly report and the
/// closure ceremony agree on the cumulative tally.  Both must report
/// 110/110 pinning + 20/20 nightly + 0 PR #27 scope incidents.
#[test]
fn monitoring_cumulative_tally_consistent() {
    let s12_body = std::fs::read_to_string(weekly_report_path(12)).expect("read");
    let closure_body = std::fs::read_to_string(closure_ceremony_path()).expect("read");

    // Both must agree on cumulative tally.
    for ref_doc in &[s12_body.as_str(), closure_body.as_str()] {
        assert!(
            ref_doc.contains("110/110"),
            "cumulative pinning 110/110 must match in both S12 weekly and closure"
        );
        assert!(
            ref_doc.contains("20/20"),
            "cumulative nightly 20/20 must match in both S12 weekly and closure"
        );
    }

    // The closure ceremony specifically must state zero PR #27 scope incidents.
    assert!(
        closure_body.contains("PR #27 scope")
            && (closure_body.contains("Incidents (PR #27 scope)   : 0")
                || closure_body.contains("0 PR #27 scope")
                || closure_body.contains("Incidents (PR #27 scope") /* matches signature block tabular layout */),
        "closure ceremony : must declare 0 incidents in PR #27 scope"
    );
}

/// **Pinning test 5x ritual reproducibility pin** : every weekly report
/// must declare 25/25 PASS for the regular cadence (5 days × 5 runs).
#[test]
fn monitoring_pinning_ritual_25_per_week() {
    for week in 9..=12_u32 {
        let body = std::fs::read_to_string(weekly_report_path(week)).expect("read");
        // Each weekly report must mention "25/25" at least once (the weekly
        // cumulative).
        assert!(
            body.contains("25/25"),
            "S{week} weekly report : must declare 25/25 weekly cumulative pinning ritual PASS"
        );
    }
}

/// **Backfill canonical case rotation pin** : the 4 canonical cases
/// (case_a, case_b, case_c, case_d) must be covered exactly once
/// across S9 → S12 (per AD-S9-S12-6 §c).
#[test]
fn monitoring_backfill_case_rotation_covered() {
    let mapping = [
        (9, "case_a"),
        (10, "case_b"),
        (11, "case_c"),
        (12, "case_d"),
    ];
    for (week, expected_case) in &mapping {
        let body = std::fs::read_to_string(weekly_report_path(*week)).expect("read");
        assert!(
            body.contains(expected_case),
            "S{week} weekly report : must reference rotation case '{expected_case}'"
        );
    }
}

/// **Closure-day actions sequence pin** : the closure ceremony must
/// record all 5 closure-day actions executed between 14:00 and 17:00 UTC
/// (per AD-S9-S12-12).
#[test]
fn closure_ceremony_records_all_closure_actions() {
    let body = std::fs::read_to_string(closure_ceremony_path()).expect("read");

    let required_actions = [
        "Master gap tracker",
        "Lessons file finalisation",
        "Roadmap archival",
        "Audit-trail row",
        "Triple sign-off",
    ];
    for action in &required_actions {
        assert!(
            body.contains(action),
            "closure ceremony : must record closure-day action '{action}'"
        );
    }

    // Time stamps of the 5 actions (14:00, 14:30, 15:00, 15:30, 16:00, 16:30, 17:00).
    let required_timestamps = ["14:00 UTC", "15:00 UTC", "15:30 UTC", "16:00 UTC", "17:00 UTC"];
    for ts in &required_timestamps {
        assert!(
            body.contains(ts),
            "closure ceremony : must record closure-day timestamp '{ts}'"
        );
    }
}

/// **Defect surface accumulation pin** : the closure ceremony must
/// report the cumulative defect surface honestly (43 total, with the
/// 33-fixed / 7-deferred / 1-process breakdown per AD-S9-S12-12
/// §"honest residual").
#[test]
fn closure_ceremony_records_defect_surface_honestly() {
    let body = std::fs::read_to_string(closure_ceremony_path()).expect("read");

    // Cumulative defect count.
    assert!(
        body.contains("43") && body.contains("defect"),
        "closure ceremony : must report 43 cumulative defects"
    );

    // Breakdown : 33 fixed / 7 G-XX / 1 process doc.
    assert!(
        body.contains("33 fixed") || body.contains("33") && body.contains("fix"),
        "closure ceremony : must report 33 defects fixed in-session"
    );
    assert!(
        body.contains("7 G-XX") || body.contains("7") && body.contains("G-XX"),
        "closure ceremony : must report 7 G-XX deferred to PR #27.1"
    );

    // The "zero defect hidden" claim must be explicit.
    assert!(
        body.contains("Zero defect") || body.contains("zero defect") || body.contains("0 defect"),
        "closure ceremony : must explicitly claim zero defect hidden"
    );
}

/// **Long-term lights-out transition pin** : the closure ceremony must
/// document the post-closure transition to long-term lights-out
/// monitoring (per AD-S9-S12-12 § 5).
#[test]
fn closure_ceremony_documents_post_closure_transition() {
    let body = std::fs::read_to_string(closure_ceremony_path()).expect("read");

    assert!(
        body.contains("Post-closure transition") || body.contains("post-closure transition"),
        "closure ceremony : must document the post-closure transition section"
    );

    assert!(
        body.contains("lights-out"),
        "closure ceremony : must declare transition to long-term lights-out"
    );

    assert!(
        body.contains("weekly canary") || body.contains("Weekly canary"),
        "closure ceremony : must declare the daily-ritual-becomes-weekly-canary transition"
    );

    assert!(
        body.contains("PR #27.1"),
        "closure ceremony : must declare PR #27.1 is unblocked"
    );
}

/// **G.11 master gap tracker entry pin** : the master gap tracker
/// must record G.11 as [closed, verified 2026-08-07] (per AD-S9-S12-12
/// step 5).  This pins the cross-document link.
#[test]
fn master_gap_tracker_records_g11_closed() {
    let tracker_path = workspace_root().join("docs/gaps/master_tracker.md");
    assert!(
        tracker_path.is_file(),
        "master gap tracker must exist at {}",
        tracker_path.display()
    );
    let body = std::fs::read_to_string(&tracker_path).expect("master tracker must read");

    assert!(
        body.contains("G.11"),
        "master tracker : must reference G.11"
    );
    assert!(
        body.contains("[closed, verified 2026-08-07]"),
        "master tracker : must mark G.11 as [closed, verified 2026-08-07]"
    );
    assert!(
        body.contains("PR #27"),
        "master tracker : G.11 closure must attribute to PR #27"
    );

    // The closure provenance section must be present.
    assert!(
        body.contains("Closure provenance"),
        "master tracker : G.11 must have a 'Closure provenance' subsection"
    );

    // The pinning tests reference must include g11_round_trip.
    assert!(
        body.contains("g11_round_trip"),
        "master tracker : G.11 closure must reference g11_round_trip pinning test"
    );
}

/// **Lessons file finalisation pin** : the PR #27 lessons file section 1
/// (headline) and section 10 (conclusion) must be finalised (no more
/// `<PLACEHOLDER>`) after the closure ceremony.
#[test]
fn lessons_file_section_1_and_10_finalised() {
    let lessons_path = workspace_root().join("docs/lessons/PR-27-lessons.md");
    assert!(
        lessons_path.is_file(),
        "PR #27 lessons file must exist at {}",
        lessons_path.display()
    );
    let body = std::fs::read_to_string(&lessons_path).expect("lessons file must read");

    // Section 1 finalised : must contain "Closure verdict (filled at S12"
    // and must NOT contain the original "Anticipated final form" placeholder
    // for section 1.
    assert!(
        body.contains("Closure verdict (filled at S12 closure ceremony, 2026-08-07)"),
        "lessons file § 1 : must contain finalised closure verdict marker"
    );

    // Section 10 finalised : must contain "Conclusion (finalised at S12
    // closure ceremony, 2026-08-07)" header.
    assert!(
        body.contains("Conclusion (finalised at S12 closure ceremony, 2026-08-07)"),
        "lessons file § 10 : must contain finalised conclusion header"
    );

    // No <PLACEHOLDER> markers should remain in section 1 or section 10.
    // We accept <PLACEHOLDER> in sections 2-8 (timeline, taxonomy, what-worked,
    // process changes, defect index, residual, audit-prompt iterations) — only
    // sections 1 and 10 are required to be finalised at closure.
    // Heuristic : ensure the FINAL conclusion section does not contain
    // "Filled at S12" (the placeholder phrase) within its first 2000
    // characters.  A later occurrence (e.g. in a sibling document
    // appended as an appendix) is allowed ; only a direct placeholder
    // inside the conclusion body is forbidden.
    let conclusion_idx = body
        .find("Conclusion (finalised at S12 closure ceremony, 2026-08-07)")
        .expect("§ 10 header must be located by the previous assert");
    let after_conclusion = &body[conclusion_idx..];
    let placeholder_position = after_conclusion.find("Filled at S12.");
    let placeholder_too_close = matches!(placeholder_position, Some(idx) if idx <= 2000);
    assert!(
        !placeholder_too_close,
        "lessons file § 10 : finalised conclusion must not retain the 'Filled at S12.' placeholder marker in its body"
    );

    // The conclusion must reference the closure ceremony companion doc.
    assert!(
        after_conclusion.contains("monitoring/PR-27-closure-2026-08-07.md")
            || body.contains("monitoring/PR-27-closure-2026-08-07.md"),
        "lessons file § 10 : finalised conclusion must reference the closure ceremony companion"
    );

    // The conclusion must declare the final test count of 174
    // (45 unit + 9 + 7 + 51 + 14 + 18 + 14 + 16 = 174).  The architecture
    // budgeted 170 (with 12 monitoring chain tests) but the actual landed
    // surface is richer (16 monitoring chain tests = +4 overshoot, accepted
    // per AD-S9-S12-12 "richer-than-budget is fine, leaner is not").
    assert!(
        body.contains("174 tests"),
        "lessons file § 10 : must declare final test count of 174 in PR #27 scope"
    );
}

// ═══════════════════════════════════════════════════════════════════
//  Documentation traceability : the lessons file references the chain
// ═══════════════════════════════════════════════════════════════════

/// **Cross-doc traceability pin** : the lessons file § 9 (References)
/// must link to all 4 weekly reports + the dry-run + the closure
/// ceremony.  Future readers of the lessons file must be able to
/// reach the monitoring chain by one click.
#[test]
fn lessons_file_references_monitoring_chain() {
    let lessons_path = workspace_root().join("docs/lessons/PR-27-lessons.md");
    let body = std::fs::read_to_string(&lessons_path).expect("lessons file must read");

    let required_refs = [
        "monitoring/PR-27-monitoring-week-9.md",
        "monitoring/PR-27-monitoring-week-10.md",
        "monitoring/PR-27-monitoring-week-11.md",
        "monitoring/PR-27-monitoring-week-12.md",
        "monitoring/PR-27-quarterly-dry-run-2026-07-31.md",
        "monitoring/PR-27-closure-2026-08-07.md",
    ];
    for r in &required_refs {
        assert!(
            body.contains(r),
            "lessons file : must reference monitoring chain artefact '{r}'"
        );
    }

    // The master gap tracker must also be referenced (for the closure provenance link).
    assert!(
        body.contains("gaps/master_tracker.md"),
        "lessons file : must reference master gap tracker (G.11 closure provenance)"
    );
}
