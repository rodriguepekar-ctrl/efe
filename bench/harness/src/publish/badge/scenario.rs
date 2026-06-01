//! PR #24 § 8 + AD9 — per-scenario badge.
//!
//! Content : `{scenario_short_id}: {verdict_glyph} {p95_ms}`.
//! Byte-stable given identical input.

use crate::compare::Verdict;
use crate::publish::badge::{colour_for, xml_escape};

#[derive(Debug, Clone)]
pub struct ScenarioBadge {
    /// Short ID e.g. "S01", "S04".
    pub scenario_short_id: String,
    pub verdict: Verdict,
    /// p95 latency in milliseconds.  When the scenario doesn't measure
    /// p95 (e.g. S10 compliance), set to `None` and the value side
    /// shows the verdict text.
    pub p95_ms: Option<f64>,
}

impl ScenarioBadge {
    #[must_use]
    pub fn new(scenario_short_id: impl Into<String>, verdict: Verdict, p95_ms: Option<f64>) -> Self {
        Self {
            scenario_short_id: scenario_short_id.into(),
            verdict,
            p95_ms,
        }
    }

    /// PR #24 § 8 — frozen value rendering.
    #[must_use]
    pub fn value_text(&self) -> String {
        match (self.verdict, self.p95_ms) {
            (Verdict::Fail, _) => "FAIL".into(),
            (Verdict::NoBaseline, _) => "no data".into(),
            (Verdict::Pass | Verdict::Warn, Some(v)) => {
                // p95 < 1ms ⇒ 0.x ; else integer ms.
                if v < 1.0 {
                    format!("{v:.2}ms")
                } else if v < 10.0 {
                    format!("{v:.1}ms")
                } else {
                    format!("{v:.0}ms")
                }
            }
            (Verdict::Pass | Verdict::Warn, None) => "pass".into(),
        }
    }

    /// Render the SVG.  Deterministic — same input ⇒ identical bytes.
    #[must_use]
    pub fn render(&self) -> String {
        let label_text = xml_escape(&self.scenario_short_id);
        let value_text = xml_escape(&self.value_text());
        let colour = colour_for(self.verdict);
        let label_width = 6 * label_text.chars().count() as u32 + 16;
        let value_width = 6 * value_text.chars().count() as u32 + 16;
        let total = label_width + value_width;
        let lx = label_width / 2;
        let vx = label_width + value_width / 2;
        format!(
r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{total}" height="20" role="img" aria-label="{label_text}: {value_text}">
  <title>{label_text}: {value_text}</title>
  <linearGradient id="s" x2="0" y2="100%">
    <stop offset="0" stop-color="#fff" stop-opacity=".7"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <clipPath id="r"><rect width="{total}" height="20" rx="3"/></clipPath>
  <g clip-path="url(#r)">
    <rect width="{label_width}" height="20" fill="#555"/>
    <rect x="{label_width}" width="{value_width}" height="20" fill="{colour}"/>
    <rect width="{total}" height="20" fill="url(#s)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">
    <text x="{lx}" y="14">{label_text}</text>
    <text x="{vx}" y="14">{value_text}</text>
  </g>
</svg>"##,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_with_p95_renders_with_ms() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(8.2));
        let svg = b.render();
        assert!(svg.contains("S01"));
        assert!(svg.contains("8.2ms"));
        assert!(svg.contains("#3fb950"));
    }

    #[test]
    fn fail_renders_fail_label_in_red() {
        let b = ScenarioBadge::new("S04", Verdict::Fail, Some(300.0));
        let svg = b.render();
        assert!(svg.contains("FAIL"));
        assert!(svg.contains("#da3633"));
        // p95 NOT included on fail.
        assert!(!svg.contains("300ms"));
    }

    #[test]
    fn no_baseline_shows_no_data_grey() {
        let b = ScenarioBadge::new("S02", Verdict::NoBaseline, None);
        let svg = b.render();
        assert!(svg.contains("no data"));
        assert!(svg.contains("#8b949e"));
    }

    #[test]
    fn warn_renders_amber() {
        let b = ScenarioBadge::new("S03", Verdict::Warn, Some(145.0));
        let svg = b.render();
        assert!(svg.contains("#d29922"));
        assert!(svg.contains("145ms"));
    }

    #[test]
    fn p95_below_1ms_uses_two_decimals() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(0.34));
        assert_eq!(b.value_text(), "0.34ms");
    }

    #[test]
    fn p95_between_1_and_10_uses_one_decimal() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(4.5));
        assert_eq!(b.value_text(), "4.5ms");
    }

    #[test]
    fn p95_above_10_uses_no_decimals() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(42.7));
        assert_eq!(b.value_text(), "43ms");  // rounded
    }

    #[test]
    fn render_byte_stable_across_runs() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(8.2));
        let a = b.render();
        let c = b.render();
        assert_eq!(a, c);
    }

    #[test]
    fn render_xml_escapes_label_input_t118() {
        // Defensive : even though scenario_short_id is constrained,
        // a future bug shouldn't allow injection.
        let b = ScenarioBadge::new("S<X>", Verdict::Pass, Some(1.0));
        let svg = b.render();
        assert!(!svg.contains("S<X>"));
        assert!(svg.contains("S&lt;X&gt;"));
    }

    /// **D-AT** — empty short_id : badge still renders valid SVG
    /// (no panic, no zero-width crash).
    #[test]
    fn d_at_empty_short_id_still_renders_valid_svg() {
        let b = ScenarioBadge::new("", Verdict::Pass, Some(8.0));
        let svg = b.render();
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("</svg>"));
    }

    /// **D-AT** — NaN p95 : value_text renders `NaNms` (acceptable
    /// for SVG, no panic).
    #[test]
    fn d_at_nan_p95_does_not_panic() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(f64::NAN));
        let svg = b.render();
        assert!(svg.contains("NaN"));
        // Still produces parseable SVG.
        assert!(svg.starts_with("<?xml"));
    }

    /// **D-AT** — `+inf` p95 : same — must not panic.
    #[test]
    fn d_at_infinity_p95_does_not_panic() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(f64::INFINITY));
        let _svg = b.render();
        // No panic — that's the contract.
    }

    /// **D-AT** — very large p95 (1e30) : badge grows in width but
    /// must not overflow u32 width math.
    #[test]
    fn d_at_huge_p95_renders_without_overflow() {
        let b = ScenarioBadge::new("S01", Verdict::Pass, Some(1e30));
        let svg = b.render();
        assert!(svg.contains("1000000000"));  // first digits visible
        assert!(svg.starts_with("<?xml"));
    }

    #[test]
    fn no_javascript_in_any_state_t100() {
        for verdict in [Verdict::Pass, Verdict::Warn, Verdict::Fail, Verdict::NoBaseline] {
            for p95 in [None, Some(8.0), Some(0.5)] {
                let svg = ScenarioBadge::new("S01", verdict, p95).render().to_lowercase();
                assert!(!svg.contains("<script"));
                assert!(!svg.contains("onclick"));
                assert!(!svg.contains("javascript:"));
            }
        }
    }
}
