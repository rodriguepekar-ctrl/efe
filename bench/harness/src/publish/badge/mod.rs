//! PR #24 § 8 + AD2 + AD6-AD10 — 11 byte-stable SVG badges
//!
//! Closed taxonomy (STABILITY_CONTRACT v1.6 § 24.1) :
//!
//!   `/badges/scenario-S01.svg` … `/badges/scenario-S10.svg`
//!   `/badges/overall.svg`
//!
//! Adding a 12th badge requires a major-version bump.

pub mod overall;
pub mod scenario;

use crate::compare::Verdict;

/// PR #24 § 8.1 — frozen 3-colour palette + amber for warn.
/// **Mirrors PR #22 + PR #23 for visual consistency** (visitors who
/// see both `efe.github.io/slos/` and `efe.github.io/bench/` should
/// experience identical colour semantics).
pub const COLOUR_PASS: &str = "#3fb950";
pub const COLOUR_WARN: &str = "#d29922";
pub const COLOUR_FAIL: &str = "#da3633";
pub const COLOUR_NO_BASELINE: &str = "#8b949e";

/// Map a verdict to the frozen hex colour.
#[must_use]
pub const fn colour_for(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => COLOUR_PASS,
        Verdict::Warn => COLOUR_WARN,
        Verdict::Fail => COLOUR_FAIL,
        Verdict::NoBaseline => COLOUR_NO_BASELINE,
    }
}

/// Frozen file names (STABILITY v1.6 § 24.1) for the 11 badges.
///
/// Order matters : it's the order they appear in `/api/scenarios.json`
/// and the overview page badge grid.
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

/// Frozen file name for the per-scenario badge of `scenario_short_id`
/// (e.g. "S01" → "scenario-S01.svg").
#[must_use]
pub fn filename_for_scenario(scenario_short_id: &str) -> Option<&'static str> {
    BADGE_FILENAMES
        .iter()
        .find(|f| f.contains(&format!("-{scenario_short_id}.svg")))
        .copied()
}

/// HTML-/XML-escape numeric values + verdict labels before SVG
/// interpolation (AD43, T118 mitigation).  Conservative — escapes the
/// classic XML metacharacters even though our inputs are numeric.
#[must_use]
pub fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

pub use overall::OverallBadge;
pub use scenario::ScenarioBadge;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eleven_badge_filenames_frozen() {
        // STABILITY_CONTRACT v1.6 § 24.1.
        assert_eq!(BADGE_FILENAMES.len(), 11);
        assert_eq!(BADGE_FILENAMES[0], "scenario-S01.svg");
        assert_eq!(BADGE_FILENAMES[9], "scenario-S10.svg");
        assert_eq!(BADGE_FILENAMES[10], "overall.svg");
    }

    #[test]
    fn colours_match_pr22_pr23() {
        // Cross-PR visual consistency (PR #22 § 12 + PR #23 § 12).
        assert_eq!(COLOUR_PASS, "#3fb950");
        assert_eq!(COLOUR_FAIL, "#da3633");
        assert_eq!(COLOUR_NO_BASELINE, "#8b949e");
    }

    #[test]
    fn colour_dispatch_complete() {
        assert_eq!(colour_for(Verdict::Pass), COLOUR_PASS);
        assert_eq!(colour_for(Verdict::Warn), COLOUR_WARN);
        assert_eq!(colour_for(Verdict::Fail), COLOUR_FAIL);
        assert_eq!(colour_for(Verdict::NoBaseline), COLOUR_NO_BASELINE);
    }

    #[test]
    fn filename_for_scenario_returns_frozen() {
        assert_eq!(filename_for_scenario("S01"), Some("scenario-S01.svg"));
        assert_eq!(filename_for_scenario("S10"), Some("scenario-S10.svg"));
        assert_eq!(filename_for_scenario("S99"), None);
    }

    #[test]
    fn xml_escape_neutralises_lt_gt_amp() {
        assert_eq!(xml_escape("a<b>c&d"), "a&lt;b&gt;c&amp;d");
        assert_eq!(xml_escape("plain"), "plain");
        assert_eq!(xml_escape("12.3"), "12.3");
    }

    #[test]
    fn xml_escape_handles_quotes_t118() {
        assert_eq!(xml_escape("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(xml_escape("o'brien"), "o&apos;brien");
    }

    // ───────────────────────────────────────────────────────────────
    // D-AK — defense-in-depth : xml_escape behaviour on hostile or
    // unexpected inputs.  Production inputs are validated upstream
    // (^S[0-9]{2}$ short IDs, numeric p95s, kebab-case verdicts), so
    // these tests *document* what happens when something slips
    // through.  If a future caller passes user-controlled strings,
    // these tests show the surface limits.
    // ───────────────────────────────────────────────────────────────

    #[test]
    fn d_ak_xml_escape_handles_combined_metacharacters() {
        let input = r##"<svg onload="alert(\"xss&'\")"/>"##;
        let out = xml_escape(input);
        assert!(!out.contains('<'), "lt escaped");
        assert!(!out.contains('>'), "gt escaped");
        assert!(!out.contains('&') || out.contains("&amp;") || out.contains("&lt;"),
            "raw & only as the start of an entity");
        assert!(!out.contains('"'), "quote escaped");
        assert!(!out.contains('\''), "apos escaped");
    }

    #[test]
    fn d_ak_xml_escape_preserves_non_metacharacters() {
        // BMP letters / digits / common punctuation pass through.
        let allowed = "S01-baseline 12.3 ms / 99.7 qps";
        assert_eq!(xml_escape(allowed), allowed);
    }

    #[test]
    fn d_ak_xml_escape_preserves_non_bmp_unicode() {
        // `chars()` iterates by Unicode codepoint, so non-BMP chars
        // (4-byte UTF-8) round-trip cleanly.  Important : a future
        // emoji in a verdict glyph should not corrupt the SVG.
        let emoji = "✅ 🔥 🦀";
        assert_eq!(xml_escape(emoji), emoji);
    }

    #[test]
    fn d_ak_xml_escape_known_gap_control_chars_passthrough() {
        // **Honest disclosure** : xml_escape does NOT strip C0
        // control characters (NUL, BEL, BS, …).  Production callers
        // never feed these in — the input validation is upstream.
        // If a future caller passes user-controlled strings, they
        // MUST sanitise control chars BEFORE calling xml_escape.
        // This test pins the current behaviour so a change is loud.
        assert_eq!(xml_escape("\x00\x07\x08"), "\x00\x07\x08");
    }

    #[test]
    fn d_ak_xml_escape_idempotent_when_no_metacharacters() {
        // Applying xml_escape twice to a string without any meta
        // characters must return the same string.
        let s = "abc 123 -. xyz";
        assert_eq!(xml_escape(&xml_escape(s)), xml_escape(s));
    }

    #[test]
    fn d_ak_xml_escape_already_escaped_entities_double_escape() {
        // Documented behaviour : xml_escape is character-based, not
        // entity-aware.  So `&amp;` becomes `&amp;amp;` — that's the
        // SAFE choice (no entity smuggling), but callers must not
        // pre-escape.
        assert_eq!(xml_escape("&amp;"), "&amp;amp;");
    }
}
