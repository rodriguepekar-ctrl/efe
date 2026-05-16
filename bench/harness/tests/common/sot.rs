//! PR #25 § 4.4 + STABILITY v1.7 § 25.2 — SOT enforcement shared helpers.
//!
//! This module is the Rust counterpart to `scripts/sot-audit.sh`.  The shell
//! script is the human-readable companion ; this module is the **hard gate**.
//!
//! It is consumed by two integration tests :
//!   * `tests/process_invariants.rs` — scans the live workspace
//!   * `tests/process_invariants_negatives.rs` — verifies the detector
//!     itself fires on synthetic input (without this, a broken detector
//!     would silently pass `process_invariants.rs`)
//!
//! Frozen constants (`SOT_OVERLAP_THRESHOLD_PCT`, `SOT_MIN_ENTRIES`)
//! are locked by `tests/stability_contract_v17.rs` § 25.2.
//!
//! References :
//!   * `docs/architecture/EFE_A1_PHASE3_PR25_ARCHITECTURE.md` § 4
//!   * `docs/process/sot-pattern.md`
//!   * `docs/STABILITY_CONTRACT.md` § 25.2

#![allow(dead_code)]

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

// ───────────────────────────────────────────────────────────────────
// Frozen constants — STABILITY v1.7 § 25.2.
// ───────────────────────────────────────────────────────────────────

/// Overlap percentage above which two const lists are considered
/// duplicated for SOT enforcement.  Frozen at 80 (STABILITY § 25.2).
pub const SOT_OVERLAP_THRESHOLD_PCT: u32 = 80;

/// Minimum number of items for a const list to be considered for
/// SOT enforcement.  Frozen at 5 (STABILITY § 25.2).
///
/// Rationale : lists with < 5 entries are unlikely to drift in
/// meaningful ways ; enforcing SOT on tuples / pairs / triples would
/// flood the CI signal with noise.
pub const SOT_MIN_ENTRIES: usize = 5;

// ───────────────────────────────────────────────────────────────────
// Whitelist for legitimate parallel cases.
// ───────────────────────────────────────────────────────────────────

/// (file_path_suffix, const_name) pairs that are ALLOWED to mirror
/// a canonical const.  Each entry MUST carry a justifying comment.
///
/// The whitelist is reviewed quarterly by release-eng (see
/// `docs/process/i1-bypass-log.md` quarterly review for the
/// adjacent cadence).  If this list grows beyond ~10 entries, that's
/// a signal that the canonical SOT pattern needs reconsideration.
pub const ALLOWED_PARALLEL_DUPLICATIONS: &[(&str, &str)] = &[
    // (no current legitimate parallel const &[&str] in the workspace —
    //  the stability tests use inline `&[...]` literals inside `assert_eq!`,
    //  NOT `const` declarations, so they do not trigger the extractor.)
];

// ───────────────────────────────────────────────────────────────────
// Data model.
// ───────────────────────────────────────────────────────────────────

/// One extracted `const NAME: &[&str] = &[...];` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedConst {
    /// Absolute path of the file that contains the declaration.
    pub file: PathBuf,
    /// 1-based line number where the declaration starts.
    pub line: u32,
    /// SCREAMING_SNAKE_CASE name of the const.
    pub name: String,
    /// Items in the array, in declared order, with quotes + whitespace
    /// stripped.
    pub items: Vec<String>,
}

impl ExtractedConst {
    /// Human-readable location for diagnostics : `path:line`.
    pub fn location(&self) -> String {
        format!("{}:{}", self.file.display(), self.line)
    }
}

/// A flagged pair where two consts overlap above the threshold.
#[derive(Debug, Clone)]
pub struct Duplication {
    pub a: ExtractedConst,
    pub b: ExtractedConst,
    pub overlap_pct: u32,
}

impl Duplication {
    pub fn diagnostic(&self) -> String {
        format!(
            "{} {} (n={}) and {} {} (n={}) overlap {}%",
            self.a.location(),
            self.a.name,
            self.a.items.len(),
            self.b.location(),
            self.b.name,
            self.b.items.len(),
            self.overlap_pct,
        )
    }
}

// ───────────────────────────────────────────────────────────────────
// Workspace walker.
// ───────────────────────────────────────────────────────────────────

/// Resolve the workspace root from the test's `CARGO_MANIFEST_DIR`.
///
/// The harness's manifest dir is `bench/harness/` ; the workspace root
/// is two levels up.  Falls back to the manifest dir itself if the
/// upper levels don't exist.
pub fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest.join("../..");
    candidate.canonicalize().unwrap_or(manifest)
}

/// Walk the workspace looking for production `*.rs` files.  Skips :
///   * `target/`        (build artefacts)
///   * `.git/`          (version control)
///   * `node_modules/`  (frontend deps if any)
///   * `generated/`     (generated code, any depth)
///   * `tests/`         (integration tests — never declare canonical consts)
///   * `examples/`      (example code — same rationale)
///   * `benches/`       (criterion benches — same)
///
/// **Policy** : canonical SOT-eligible consts live ONLY in `*/src/`
/// trees.  Test / example / bench files MAY contain string literals
/// that look like const declarations (e.g. synthetic Rust source
/// inside `r#"..."#` strings, used by the detector's own negative
/// tests).  Scanning those would produce false-positives that
/// pollute the hard-gate signal.  See `process_invariants_negatives.rs`
/// for the synthetic-fixture pattern.
pub fn collect_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        let name = match p.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        // Skip well-known noise dirs + non-production trees.
        if matches!(
            name,
            "target"
                | ".git"
                | "node_modules"
                | "generated"
                | "tests"
                | "examples"
                | "benches"
        ) {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

// ───────────────────────────────────────────────────────────────────
// Const extractor.
// ───────────────────────────────────────────────────────────────────

/// Compile the start-of-declaration regex once.
///
/// Matches `const NAME: &[&str] = ...` and `pub const NAME: &[&str] = ...`.
/// Other element types (`&[u32]`, `&[(&str, &str)]`, ...) are NOT
/// matched ; SOT enforcement for those is deferred.
fn start_re() -> Regex {
    Regex::new(
        r"^\s*(?:pub\s+)?const\s+([A-Z_][A-Z_0-9]*)\s*:\s*&\[\s*&str\s*\]\s*=",
    )
    .expect("start_re must compile")
}

/// Extract every `const NAME: &[&str] = &[...];` declaration from `path`.
///
/// Handles :
///   * single-line declarations (`const X: &[&str] = &["a", "b"];`)
///   * multi-line declarations (the common pattern with each item
///     on its own line)
///   * `pub const` and `const` (private) variants
///
/// Does NOT handle :
///   * `lazy_static! { ... }` blocks
///   * `phf_map!` or other macro-generated lists
///   * Inline `&[...]` literals (not `const` form)
///   * Element types other than `&str`
///
/// Returns an empty Vec if the file is unreadable.
pub fn extract_string_array_consts(path: &Path) -> Vec<ExtractedConst> {
    let body = match fs::read_to_string(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let re = start_re();
    let lines: Vec<&str> = body.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(caps) = re.captures(line) {
            let name = caps.get(1).unwrap().as_str().to_string();
            let start_line = (i + 1) as u32;
            // Find `&[` AFTER the `=`.  The line contains two `&[`
            // tokens : one in the type annotation, one in the value.
            // We want the second.  (Same bug fix as shell sot-audit.sh
            // pass at Day 1.)
            let after_eq = match line.find('=') {
                Some(eq_pos) => &line[eq_pos..],
                None => {
                    i += 1;
                    continue;
                }
            };
            let body_in_after = match after_eq.find("&[") {
                Some(p) => p + 2,
                None => {
                    i += 1;
                    continue;
                }
            };
            let mut accumulated = after_eq[body_in_after..].to_string();
            // If `]` is on the same line, finalise here.
            let consumed = if let Some(close) = accumulated.find(']') {
                accumulated.truncate(close);
                0
            } else {
                // Multi-line : consume subsequent lines until we find `]`.
                let mut extra = 0;
                let mut j = i + 1;
                while j < lines.len() {
                    extra += 1;
                    let nl = lines[j];
                    if let Some(close) = nl.find(']') {
                        accumulated.push(' ');
                        accumulated.push_str(&nl[..close]);
                        break;
                    }
                    accumulated.push(' ');
                    accumulated.push_str(nl);
                    j += 1;
                }
                extra
            };

            let items: Vec<String> = accumulated
                .split(',')
                .map(|s| {
                    // Strip whitespace, then strip surrounding quotes.
                    let trimmed = s.trim();
                    let unquoted = trimmed.trim_start_matches('"').trim_end_matches('"');
                    unquoted.trim().to_string()
                })
                .filter(|s| !s.is_empty())
                .collect();

            // Only record non-trivial extractions.
            if !items.is_empty() {
                out.push(ExtractedConst {
                    file: path.to_path_buf(),
                    line: start_line,
                    name,
                    items,
                });
            }
            i += 1 + consumed;
        } else {
            i += 1;
        }
    }
    out
}

// ───────────────────────────────────────────────────────────────────
// Overlap detection.
// ───────────────────────────────────────────────────────────────────

/// Compute pairwise overlap percentage between two item lists.
///
/// Formula : `|A ∩ B| / max(|A|, |B|) * 100`, rounded down.
///
/// Returns 0 if either list is empty.
pub fn overlap_pct(a: &[String], b: &[String]) -> u32 {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let set_a: HashSet<&String> = a.iter().collect();
    let set_b: HashSet<&String> = b.iter().collect();
    let intersection = set_a.intersection(&set_b).count();
    let max_size = a.len().max(b.len());
    ((intersection * 100) / max_size) as u32
}

/// Return true if `c` is in the whitelist.
pub fn is_whitelisted(c: &ExtractedConst) -> bool {
    let path_str = c.file.to_string_lossy();
    ALLOWED_PARALLEL_DUPLICATIONS
        .iter()
        .any(|(suffix, name)| path_str.ends_with(suffix) && c.name == *name)
}

/// Find all (i, j) pairs with overlap ≥ threshold, excluding any pair
/// where one side is whitelisted.
///
/// Returns a Vec of [`Duplication`].  Empty Vec means SOT-compliant.
pub fn detect_duplications(
    consts: &[ExtractedConst],
    threshold_pct: u32,
) -> Vec<Duplication> {
    let mut out = Vec::new();
    for i in 0..consts.len() {
        for j in (i + 1)..consts.len() {
            let a = &consts[i];
            let b = &consts[j];
            // Skip same-location pairs.
            if a.file == b.file && a.line == b.line {
                continue;
            }
            // Skip if either side is whitelisted.
            if is_whitelisted(a) || is_whitelisted(b) {
                continue;
            }
            let pct = overlap_pct(&a.items, &b.items);
            if pct >= threshold_pct {
                out.push(Duplication {
                    a: a.clone(),
                    b: b.clone(),
                    overlap_pct: pct,
                });
            }
        }
    }
    out
}

// ───────────────────────────────────────────────────────────────────
// Convenience : workspace-wide pipeline.
// ───────────────────────────────────────────────────────────────────

/// Walk the workspace, extract all eligible const lists, filter to
/// `>= SOT_MIN_ENTRIES`, return them.
///
/// Used by `process_invariants.rs` as the input to `detect_duplications`.
pub fn collect_workspace_qualifying_consts() -> Vec<ExtractedConst> {
    let root = workspace_root();
    let files = collect_rs_files(&root);
    let mut all = Vec::new();
    for f in files {
        all.extend(extract_string_array_consts(&f));
    }
    all.retain(|c| c.items.len() >= SOT_MIN_ENTRIES);
    all
}

// ───────────────────────────────────────────────────────────────────
// Self-tests for the helpers themselves.
//
// These run as part of any binary that includes this module ; they
// validate the extractor + overlap algorithm against tiny inline
// fixtures so a bug in the helpers can't silently invalidate the
// hard-gate tests.
// ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod self_tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("input.rs");
        let mut h = fs::File::create(&f).unwrap();
        h.write_all(contents.as_bytes()).unwrap();
        (tmp, f)
    }

    #[test]
    fn extractor_single_line() {
        let (_t, f) = write_tmp(
            r#"
pub const SINGLE: &[&str] = &["a", "b", "c", "d", "e"];
"#,
        );
        let out = extract_string_array_consts(&f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "SINGLE");
        assert_eq!(
            out[0].items,
            ["a", "b", "c", "d", "e"]
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn extractor_multi_line() {
        let (_t, f) = write_tmp(
            r#"
pub const MULTI: &[&str] = &[
    "x1",
    "x2",
    "x3",
    "x4",
    "x5",
    "x6",
];
"#,
        );
        let out = extract_string_array_consts(&f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "MULTI");
        assert_eq!(out[0].items.len(), 6);
        assert_eq!(out[0].items[0], "x1");
        assert_eq!(out[0].items[5], "x6");
    }

    #[test]
    fn extractor_private_const() {
        let (_t, f) = write_tmp(
            r#"
const PRIVATE: &[&str] = &["only", "private", "but", "extracted"];
"#,
        );
        let out = extract_string_array_consts(&f);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "PRIVATE");
    }

    #[test]
    fn extractor_ignores_non_string_arrays() {
        // &[u32] or &[(&str, &str)] should not match.
        let (_t, f) = write_tmp(
            r#"
pub const NUMBERS: &[u32] = &[1, 2, 3];
pub const PAIRS: &[(&str, &str)] = &[("a", "b")];
"#,
        );
        let out = extract_string_array_consts(&f);
        assert!(out.is_empty(), "non-&str arrays must not match");
    }

    #[test]
    fn extractor_handles_two_consts_same_file() {
        let (_t, f) = write_tmp(
            r#"
pub const A: &[&str] = &["a1", "a2", "a3", "a4", "a5"];
pub const B: &[&str] = &["b1", "b2", "b3", "b4", "b5"];
"#,
        );
        let out = extract_string_array_consts(&f);
        assert_eq!(out.len(), 2);
        let names: Vec<_> = out.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"A"));
        assert!(names.contains(&"B"));
    }

    #[test]
    fn overlap_identical() {
        let a = vec!["x".into(), "y".into(), "z".into()];
        assert_eq!(overlap_pct(&a, &a), 100);
    }

    #[test]
    fn overlap_disjoint() {
        let a = vec!["x".into(), "y".into()];
        let b = vec!["a".into(), "b".into()];
        assert_eq!(overlap_pct(&a, &b), 0);
    }

    #[test]
    fn overlap_partial() {
        let a = vec!["x".into(), "y".into(), "z".into(), "w".into()];
        let b = vec!["x".into(), "y".into(), "a".into(), "b".into()];
        // intersection = 2, max = 4 → 50%.
        assert_eq!(overlap_pct(&a, &b), 50);
    }

    #[test]
    fn overlap_with_empty() {
        let a: Vec<String> = vec![];
        let b = vec!["x".into()];
        assert_eq!(overlap_pct(&a, &b), 0);
        assert_eq!(overlap_pct(&b, &a), 0);
    }

    #[test]
    fn overlap_different_sizes_uses_max() {
        // a = {x, y} ; b = {x, y, z, w, v} ; intersection = 2 ; max = 5 → 40%.
        let a = vec!["x".into(), "y".into()];
        let b = vec!["x".into(), "y".into(), "z".into(), "w".into(), "v".into()];
        assert_eq!(overlap_pct(&a, &b), 40);
    }

    #[test]
    fn whitelist_match_by_suffix_and_name() {
        let c = ExtractedConst {
            file: PathBuf::from("/some/path/stability_contract_v16.rs"),
            line: 42,
            name: "FOO".into(),
            items: vec![],
        };
        // No entry in whitelist for FOO — should not match.
        assert!(!is_whitelisted(&c));
    }

    #[test]
    fn detect_duplications_finds_obvious() {
        let a = ExtractedConst {
            file: PathBuf::from("a.rs"),
            line: 1,
            name: "X".into(),
            items: (0..10).map(|i| format!("item{i}")).collect(),
        };
        let b = ExtractedConst {
            file: PathBuf::from("b.rs"),
            line: 1,
            name: "Y".into(),
            items: (0..10).map(|i| format!("item{i}")).collect(),
        };
        let dups = detect_duplications(&[a, b], 80);
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].overlap_pct, 100);
    }

    #[test]
    fn detect_duplications_respects_threshold() {
        let a = ExtractedConst {
            file: PathBuf::from("a.rs"),
            line: 1,
            name: "X".into(),
            items: (0..10).map(|i| format!("item{i}")).collect(),
        };
        let b = ExtractedConst {
            file: PathBuf::from("b.rs"),
            line: 1,
            name: "Y".into(),
            items: (0..5).map(|i| format!("item{i}")).collect(),  // 50% overlap
        };
        // At threshold 80, this pair should NOT be flagged.
        let dups = detect_duplications(&[a.clone(), b.clone()], 80);
        assert!(dups.is_empty());
        // At threshold 40, it SHOULD be flagged.
        let dups = detect_duplications(&[a, b], 40);
        assert_eq!(dups.len(), 1);
    }
}
