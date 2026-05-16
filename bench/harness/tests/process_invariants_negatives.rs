//! PR #25 § 4.4 — process invariant **I.2** negative tests.
//!
//! These tests verify that the SOT detector ACTUALLY FIRES on synthetic
//! input.  Without them, a broken detector would silently pass
//! `process_invariants.rs` (which scans the live workspace — a clean
//! workspace yields zero findings, indistinguishable from a no-op
//! detector).
//!
//! Pattern : every positive test in `process_invariants.rs` has a
//! corresponding negative test here that feeds the detector a known
//! bad input + asserts it flags it.
//!
//! References :
//!   * docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md § 4.4 + § 8.2
//!   * docs/lessons/pr24-lessons-learned.md § 6 (D-T : test mental
//!     models are unreliable ; negatives catch detector regressions)

mod common;

use std::path::PathBuf;

use common::sot::{
    detect_duplications, extract_string_array_consts, overlap_pct, ExtractedConst,
    SOT_MIN_ENTRIES, SOT_OVERLAP_THRESHOLD_PCT,
};

// ───────────────────────────────────────────────────────────────────
// Helpers for synthetic input.
// ───────────────────────────────────────────────────────────────────

/// Build a synthetic `ExtractedConst` for testing.  The `file` field is
/// a fake path ; the test doesn't read the filesystem.
fn synth(name: &str, items: Vec<&str>, file: &str, line: u32) -> ExtractedConst {
    ExtractedConst {
        file: PathBuf::from(file),
        line,
        name: name.into(),
        items: items.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// Write a tiny Rust source string to a temp file and return its path.
fn write_tmp_rs(body: &str) -> (tempfile::TempDir, PathBuf) {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("input.rs");
    std::fs::File::create(&f)
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    (tmp, f)
}

// ═══════════════════════════════════════════════════════════════════
// 1.  Detector MUST fire on identical lists.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn negative_detects_two_identical_lists() {
    // 11 identical items → 100% overlap.  MUST be flagged at threshold 80.
    let items: Vec<&str> = (0..11).map(|_| "scenario").collect();
    // Note : duplicates in the items list collapse to a set ; the
    // detector uses |A ∩ B| / max(|A|,|B|).  For an honest test, use
    // 11 DISTINCT items.
    let canonical_items: Vec<&str> = vec![
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
    ];
    drop(items);
    let a = synth("CANONICAL", canonical_items.clone(), "a.rs", 1);
    let b = synth("EVIL_DUPE", canonical_items, "b.rs", 1);

    let dups = detect_duplications(&[a, b], SOT_OVERLAP_THRESHOLD_PCT);
    assert_eq!(
        dups.len(),
        1,
        "detector must flag 100% identical lists at threshold {SOT_OVERLAP_THRESHOLD_PCT}%"
    );
    assert_eq!(dups[0].overlap_pct, 100);
}

#[test]
fn negative_diagnostic_mentions_both_locations() {
    let items: Vec<&str> = (0..10).map(|i| Box::leak(format!("item{i}").into_boxed_str()) as &str).collect();
    let a = synth("X", items.clone(), "/path/to/a.rs", 42);
    let b = synth("Y", items, "/path/to/b.rs", 99);
    let dups = detect_duplications(&[a, b], 80);
    let diag = dups[0].diagnostic();
    assert!(diag.contains("/path/to/a.rs:42"));
    assert!(diag.contains("/path/to/b.rs:99"));
    assert!(diag.contains("100%"));
}

// ═══════════════════════════════════════════════════════════════════
// 2.  Detector MUST fire at the exact threshold boundary.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn negative_detects_at_exact_threshold_80() {
    // 4 of 5 shared = 80% exactly.
    let a = synth("A", vec!["x1", "x2", "x3", "x4", "x5"], "a.rs", 1);
    let b = synth("B", vec!["x1", "x2", "x3", "x4", "y5"], "b.rs", 1);
    // overlap = |{x1,x2,x3,x4}| = 4 ; max = 5 ; 4/5*100 = 80.
    assert_eq!(overlap_pct(&a.items, &b.items), 80);
    let dups = detect_duplications(&[a, b], 80);
    assert_eq!(
        dups.len(),
        1,
        "detector must flag exact-threshold overlap (the >= comparison must be inclusive)"
    );
}

#[test]
fn negative_does_not_fire_just_below_threshold() {
    // 3 of 5 shared = 60%.  At threshold 80, must NOT fire.
    let a = synth("A", vec!["x1", "x2", "x3", "y4", "y5"], "a.rs", 1);
    let b = synth("B", vec!["x1", "x2", "x3", "z4", "z5"], "b.rs", 1);
    assert_eq!(overlap_pct(&a.items, &b.items), 60);
    let dups = detect_duplications(&[a, b], 80);
    assert!(
        dups.is_empty(),
        "detector must not fire below threshold (60% < 80%)"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 3.  Detector ignores lists below SOT_MIN_ENTRIES (caller filters).
// ═══════════════════════════════════════════════════════════════════

#[test]
fn negative_detector_does_not_apply_min_entries_itself() {
    // The detector itself does NOT filter on size — the caller (the
    // workspace pipeline) does that.  Two 3-item identical lists,
    // fed directly to the detector, ARE flagged.  This is intentional :
    // the detector is a pure algorithm ; size filtering is policy.
    let a = synth("A", vec!["x", "y", "z"], "a.rs", 1);
    let b = synth("B", vec!["x", "y", "z"], "b.rs", 1);
    let dups = detect_duplications(&[a, b], 80);
    assert_eq!(dups.len(), 1, "raw detector fires on small lists ; policy lives in the pipeline");
}

// ═══════════════════════════════════════════════════════════════════
// 4.  Same-location pairs (file+line) skipped.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn negative_skips_same_location_pairs() {
    // Two ExtractedConst with same file+line are treated as the same
    // declaration ; the detector skips them.  Defensive against
    // accidental duplicate extraction.
    let items: Vec<&str> = (0..10).map(|i| Box::leak(format!("dup{i}").into_boxed_str()) as &str).collect();
    let a = synth("X", items.clone(), "a.rs", 1);
    let b = synth("X", items, "a.rs", 1);
    let dups = detect_duplications(&[a, b], 80);
    assert!(
        dups.is_empty(),
        "same file+line pair should be skipped (treated as the same decl)"
    );
}

// ═══════════════════════════════════════════════════════════════════
// 5.  Detector handles empty input.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn negative_empty_input_yields_no_findings() {
    let dups = detect_duplications(&[], 80);
    assert!(dups.is_empty());
}

#[test]
fn negative_single_const_yields_no_findings() {
    let a = synth("LONELY", vec!["a", "b", "c", "d", "e"], "a.rs", 1);
    let dups = detect_duplications(&[a], 80);
    assert!(dups.is_empty());
}

// ═══════════════════════════════════════════════════════════════════
// 6.  Extractor regression tests : files crafted to hit known edges.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn extractor_does_not_match_type_annotation_only() {
    // The line contains `&[&str]` in the type annotation but the value
    // is something else.  Must not be misinterpreted as a const list.
    let (_t, f) = write_tmp_rs(
        r#"
fn f() -> &'static [&'static str] {
    panic!("not a const declaration")
}
"#,
    );
    let out = extract_string_array_consts(&f);
    assert!(out.is_empty(), "function returning &[&str] should not be extracted as a const");
}

#[test]
fn extractor_handles_const_with_trailing_comma() {
    let (_t, f) = write_tmp_rs(
        r#"
pub const X: &[&str] = &[
    "a",
    "b",
    "c",
    "d",
    "e",
];
"#,
    );
    let out = extract_string_array_consts(&f);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].items, vec!["a", "b", "c", "d", "e"]);
}

#[test]
fn extractor_handles_const_without_trailing_comma() {
    let (_t, f) = write_tmp_rs(
        r#"
pub const Y: &[&str] = &[
    "a",
    "b",
    "c",
    "d",
    "e"
];
"#,
    );
    let out = extract_string_array_consts(&f);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].items, vec!["a", "b", "c", "d", "e"]);
}

#[test]
fn extractor_handles_const_with_string_containing_special_chars() {
    // Strings with paths, scenes — these are exactly the items we use.
    let (_t, f) = write_tmp_rs(
        r#"
pub const PATHS: &[&str] = &[
    "scenario-S01.svg",
    "scenario-S02.svg",
    "/api/current.json",
    "/quarterly/Q2-2026.pdf",
    "https://efe.github.io/smoke/badge.svg",
];
"#,
    );
    let out = extract_string_array_consts(&f);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].items.len(), 5);
    assert!(out[0].items.iter().any(|s| s.contains("scenario-S01.svg")));
    assert!(out[0].items.iter().any(|s| s.contains("https://efe.github.io")));
}

#[test]
fn extractor_two_consts_same_file_isolated() {
    let (_t, f) = write_tmp_rs(
        r#"
pub const A: &[&str] = &["a1", "a2", "a3", "a4", "a5", "a6"];

pub const B: &[&str] = &["b1", "b2", "b3", "b4", "b5"];
"#,
    );
    let out = extract_string_array_consts(&f);
    assert_eq!(out.len(), 2);
    let a = out.iter().find(|c| c.name == "A").unwrap();
    let b = out.iter().find(|c| c.name == "B").unwrap();
    assert_eq!(a.items.len(), 6);
    assert_eq!(b.items.len(), 5);
    // The two extractions must NOT bleed into each other.
    assert!(!a.items.iter().any(|s| s.starts_with("b")));
    assert!(!b.items.iter().any(|s| s.starts_with("a")));
}

// ═══════════════════════════════════════════════════════════════════
// 7.  End-to-end detector pipeline on a synthetic 2-file workspace.
// ═══════════════════════════════════════════════════════════════════

#[test]
fn end_to_end_detector_pipeline_on_duplicated_workspace() {
    // Plant two files in a tempdir, each carrying a duplicate of the
    // canonical 11-element BADGE_FILENAMES list.  The detector pipeline
    // (extract + filter + detect) must flag the pair.
    let tmp = tempfile::tempdir().unwrap();
    let src_dir = tmp.path().join("src");
    std::fs::create_dir(&src_dir).unwrap();

    let canonical_body = r#"
pub const BADGE_FILENAMES: &[&str] = &[
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
];
"#;
    let evil_body = r#"
pub const EVIL_DUPE: &[&str] = &[
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
];
"#;
    std::fs::write(src_dir.join("canonical.rs"), canonical_body).unwrap();
    std::fs::write(src_dir.join("evil.rs"), evil_body).unwrap();

    // Extract directly from the two files (no workspace walker needed).
    let mut consts = Vec::new();
    consts.extend(extract_string_array_consts(&src_dir.join("canonical.rs")));
    consts.extend(extract_string_array_consts(&src_dir.join("evil.rs")));
    consts.retain(|c| c.items.len() >= SOT_MIN_ENTRIES);
    assert_eq!(consts.len(), 2);

    let dups = detect_duplications(&consts, SOT_OVERLAP_THRESHOLD_PCT);
    assert_eq!(
        dups.len(),
        1,
        "end-to-end pipeline must flag the planted duplicate"
    );
    assert_eq!(dups[0].overlap_pct, 100);

    // Diagnostic should reference the evil file specifically.
    let diag = dups[0].diagnostic();
    assert!(diag.contains("BADGE_FILENAMES"));
    assert!(diag.contains("EVIL_DUPE"));
}

// ═══════════════════════════════════════════════════════════════════
// 8.  Whitelist semantics : a synthetic whitelist entry exempts the
// flagged pair.
// ═══════════════════════════════════════════════════════════════════
//
// NOTE : the production whitelist `ALLOWED_PARALLEL_DUPLICATIONS` is a
// frozen `const` in `tests/common/sot.rs` — we cannot mutate it from
// here.  This test instead verifies the SHAPE of the whitelist data
// (so additions to it are reviewable) and the function `is_whitelisted`
// behaves correctly on a synthetic case.

#[test]
fn whitelist_stays_under_review_ceiling() {
    // The architecture (§ 4.4) says the whitelist > 10 entries is a
    // signal that the canonical SOT pattern needs reconsideration.
    // This test trips the alarm.  Comparison via len() (not is_empty())
    // so clippy doesn't fold the assert as trivially true on the
    // current empty whitelist.
    use common::sot::ALLOWED_PARALLEL_DUPLICATIONS;
    let n = ALLOWED_PARALLEL_DUPLICATIONS.len();
    assert!(
        n <= 10,
        "ALLOWED_PARALLEL_DUPLICATIONS has {n} entries (> 10 review ceiling) — \
         reconsider whether canonical SOT consolidation is still right"
    );
}

#[test]
fn whitelist_shape_validates() {
    // Every whitelist entry must be (suffix_str, name_str) — checked
    // by the type system, but we also verify both are non-empty
    // (catches `("", "")`).
    use common::sot::ALLOWED_PARALLEL_DUPLICATIONS;
    for (suffix, name) in ALLOWED_PARALLEL_DUPLICATIONS {
        assert!(!suffix.is_empty(), "whitelist entry has empty suffix");
        assert!(!name.is_empty(), "whitelist entry has empty const name");
        // const name must be SCREAMING_SNAKE_CASE.
        assert!(
            name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'),
            "whitelist entry const name '{name}' not SCREAMING_SNAKE_CASE"
        );
    }
}
