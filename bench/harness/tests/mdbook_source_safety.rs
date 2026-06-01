//! PR #24 § 11 + D-AA — security audit of the mdbook SOURCE markdowns.
//!
//! mdbook produces HTML from `bench/publish/src/*.md` ; the CSP meta
//! tag blocks `<script>` etc. at runtime, BUT we want to fail HARD at
//! `cargo test` if someone ever lands a markdown file containing
//! inline JS or styles : runtime CSP is the LAST line of defense, the
//! source files are the FIRST.
//!
//! Catches : drive-by `<script>`, `onclick=`, `onerror=`,
//! `javascript:`-prefixed anchors, inline `<style>` blocks, `style="`
//! attributes, `<iframe>`, `<object>`.  If any of these legitimately
//! need to be in a markdown, add a per-pattern exception with a
//! comment explaining why.

use std::path::{Path, PathBuf};

fn list_markdown_files() -> Vec<PathBuf> {
    let src_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../publish/src");
    let mut out = Vec::new();
    walk(&src_root, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().and_then(|s| s.to_str()) == Some("md") {
                out.push(p);
            }
        }
    }
}

const FORBIDDEN: &[&str] = &[
    "<script",
    "</script>",
    "onclick=",
    "onerror=",
    "onload=",
    "onmouseover=",
    "javascript:",
    "<style",
    "</style>",
    " style=\"",
    "<iframe",
    "<object",
    "<embed",
];

#[test]
fn d_aa_mdbook_source_files_carry_no_inline_js_or_styles() {
    let files = list_markdown_files();
    assert!(!files.is_empty(), "no markdown files discovered — wrong path?");

    let mut violations: Vec<String> = Vec::new();
    for f in &files {
        let body = std::fs::read_to_string(f)
            .unwrap_or_else(|e| panic!("cannot read {f:?} : {e}"));
        let lower = body.to_lowercase();
        for pat in FORBIDDEN {
            if lower.contains(pat) {
                violations.push(format!("{}: contains '{pat}'", f.display()));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "D-AA : forbidden patterns in mdbook sources :\n  - {}",
        violations.join("\n  - ")
    );
}

#[test]
fn d_aa_smoke_badge_sentinel_present_in_overview() {
    // The smoke sentinel MUST live in overview.md so the post-process
    // hook (D-N) can substitute it.  If someone removes it, the
    // `--embed-smoke` flag becomes a no-op in the mdbook path.
    let overview = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../publish/src/overview.md");
    let body = std::fs::read_to_string(&overview).expect("overview.md must exist");
    assert!(
        body.contains("<!-- EFE_SMOKE_BADGE_SLOT -->"),
        "D-AA / D-N : overview.md must keep the smoke sentinel — \
         removing it silently breaks --embed-smoke in the mdbook path"
    );
}

#[test]
fn d_aa_summary_lists_exactly_10_scenarios() {
    // STABILITY § 24.5 : 10 scenarios frozen.  If SUMMARY.md drifts,
    // the published site loses navigation entries.
    let summary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../publish/src/SUMMARY.md");
    let body = std::fs::read_to_string(&summary).expect("SUMMARY.md must exist");
    let count = body
        .lines()
        .filter(|l| l.trim_start().starts_with("- [S") && l.contains("scenarios/s"))
        .count();
    assert_eq!(count, 10, "D-AA : SUMMARY.md must list exactly 10 scenario entries — got {count}");
}
