//! PR #24 § 8 + AD10 — aggregate "overall" badge.
//!
//! Verdict aggregation : **worst-of** across the 10 scenarios.
//!   Fail > Warn > Pass > NoBaseline

use crate::compare::Verdict;
use crate::publish::badge::{colour_for, xml_escape};

#[derive(Debug, Clone)]
pub struct OverallBadge {
    pub verdict: Verdict,
    pub failing_count: usize,
}

impl OverallBadge {
    /// Compute the aggregate verdict from per-scenario verdicts.
    /// Empty input ⇒ NoBaseline (no data).
    #[must_use]
    pub fn from_scenarios(verdicts: &[Verdict]) -> Self {
        if verdicts.is_empty() {
            return Self {
                verdict: Verdict::NoBaseline,
                failing_count: 0,
            };
        }
        let failing_count = verdicts.iter().filter(|v| matches!(**v, Verdict::Fail)).count();
        let agg = verdicts.iter().copied().fold(Verdict::Pass, |a, b| a.combine(b));
        // Empty-input case handled above ; if all `NoBaseline`, fold gives Pass — fix it :
        let all_no_baseline = verdicts.iter().all(|v| matches!(*v, Verdict::NoBaseline));
        let verdict = if all_no_baseline { Verdict::NoBaseline } else { agg };
        Self {
            verdict,
            failing_count,
        }
    }

    /// Frozen value rendering.
    #[must_use]
    pub fn value_text(&self) -> String {
        match self.verdict {
            Verdict::Pass => "pass".into(),
            Verdict::Warn => "warn".into(),
            Verdict::Fail => {
                if self.failing_count > 0 {
                    format!("fail ({})", self.failing_count)
                } else {
                    "fail".into()
                }
            }
            Verdict::NoBaseline => "no data".into(),
        }
    }

    /// Render the SVG.  Byte-stable.
    #[must_use]
    pub fn render(&self) -> String {
        let label_text = xml_escape("bench");
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
    fn aggregate_all_pass_is_pass() {
        let v = vec![Verdict::Pass; 10];
        let o = OverallBadge::from_scenarios(&v);
        assert_eq!(o.verdict, Verdict::Pass);
        assert_eq!(o.failing_count, 0);
    }

    #[test]
    fn aggregate_one_fail_is_fail() {
        let mut v = vec![Verdict::Pass; 9];
        v.push(Verdict::Fail);
        let o = OverallBadge::from_scenarios(&v);
        assert_eq!(o.verdict, Verdict::Fail);
        assert_eq!(o.failing_count, 1);
    }

    #[test]
    fn aggregate_three_fails_count() {
        let v = vec![
            Verdict::Pass, Verdict::Pass, Verdict::Fail,
            Verdict::Pass, Verdict::Fail, Verdict::Pass,
            Verdict::Pass, Verdict::Fail, Verdict::Pass, Verdict::Pass,
        ];
        let o = OverallBadge::from_scenarios(&v);
        assert_eq!(o.verdict, Verdict::Fail);
        assert_eq!(o.failing_count, 3);
    }

    #[test]
    fn aggregate_warn_not_fail() {
        let v = vec![Verdict::Pass, Verdict::Warn, Verdict::Pass];
        let o = OverallBadge::from_scenarios(&v);
        assert_eq!(o.verdict, Verdict::Warn);
    }

    #[test]
    fn aggregate_all_nobaseline_is_nobaseline() {
        let v = vec![Verdict::NoBaseline; 10];
        let o = OverallBadge::from_scenarios(&v);
        assert_eq!(o.verdict, Verdict::NoBaseline);
    }

    #[test]
    fn aggregate_mixed_pass_nobaseline_is_pass() {
        let v = vec![Verdict::Pass, Verdict::NoBaseline, Verdict::Pass];
        let o = OverallBadge::from_scenarios(&v);
        assert_eq!(o.verdict, Verdict::Pass);
    }

    #[test]
    fn aggregate_empty_is_nobaseline() {
        let o = OverallBadge::from_scenarios(&[]);
        assert_eq!(o.verdict, Verdict::NoBaseline);
    }

    /// **D-AT** — empty input MUST produce a valid SVG with verdict
    /// NoBaseline (no crash, no division-by-zero in width math).
    #[test]
    fn d_at_empty_input_produces_no_baseline_svg() {
        let o = OverallBadge::from_scenarios(&[]);
        assert_eq!(o.verdict, Verdict::NoBaseline);
        assert_eq!(o.failing_count, 0);
        let svg = o.render();
        assert!(svg.contains("no data"));
        assert!(svg.contains("#8b949e"));
        // Sanity : produces well-formed SVG.
        assert!(svg.starts_with("<?xml"));
        assert!(svg.contains("</svg>"));
    }

    /// **D-AT** — 10K fail scenarios : `failing_count` must not overflow
    /// the value cell rendering.  Acceptable upper bound : `format!`
    /// can produce arbitrarily long integers ; the SVG width math
    /// must not panic on u32 overflow.
    #[test]
    fn d_at_huge_failing_count_renders_without_panic() {
        let v = vec![Verdict::Fail; 10_000];
        let o = OverallBadge::from_scenarios(&v);
        assert_eq!(o.failing_count, 10_000);
        let svg = o.render();
        assert!(svg.contains("fail (10000)"));
    }

    /// **D-AT** — homogeneous all-NoBaseline aggregation : the function
    /// has special-case logic for this (the `combine` fold yields Pass
    /// for all-NoBaseline due to the absorbing-element design).
    #[test]
    fn d_at_all_no_baseline_does_not_collapse_to_pass() {
        let o = OverallBadge::from_scenarios(&[Verdict::NoBaseline; 5]);
        assert_eq!(o.verdict, Verdict::NoBaseline);
        let svg = o.render();
        assert!(svg.contains("no data"));
    }

    #[test]
    fn render_pass_is_green() {
        let o = OverallBadge::from_scenarios(&[Verdict::Pass; 10]);
        let svg = o.render();
        assert!(svg.contains("#3fb950"));
        assert!(svg.contains("bench"));
        assert!(svg.contains("pass"));
    }

    #[test]
    fn render_fail_includes_count() {
        let mut v = vec![Verdict::Pass; 8];
        v.push(Verdict::Fail);
        v.push(Verdict::Fail);
        let o = OverallBadge::from_scenarios(&v);
        let svg = o.render();
        assert!(svg.contains("fail (2)"));
        assert!(svg.contains("#da3633"));
    }

    #[test]
    fn render_byte_stable() {
        let o = OverallBadge::from_scenarios(&[Verdict::Pass; 10]);
        assert_eq!(o.render(), o.render());
    }

    #[test]
    fn no_javascript_t100() {
        for v in [Verdict::Pass, Verdict::Warn, Verdict::Fail, Verdict::NoBaseline] {
            let o = OverallBadge::from_scenarios(&[v]);
            let svg = o.render().to_lowercase();
            assert!(!svg.contains("<script"));
            assert!(!svg.contains("onclick"));
            assert!(!svg.contains("javascript:"));
        }
    }
}
