//! PR #23 § 12 + Appendix E — gh-pages SVG badge.  Byte-stable.
//! T100 mitigation : no `<script>` / `onclick` / `javascript:`.

use crate::checks::CheckVerdict;

/// Render the public smoke badge SVG.  Frozen v1.5 § 23.7.
#[must_use]
pub fn render(verdict: CheckVerdict, failing_count: usize) -> String {
    let (colour, value) = match (verdict, failing_count) {
        (CheckVerdict::Pass, _) => ("#3fb950", "pass"),
        (CheckVerdict::Fail, 0) => ("#8b949e", "no data"),
        (CheckVerdict::Fail, n) => ("#da3633", &*format!("fail ({n})")),
    };
    let value_owned = value.to_string();
    let label = "smoke";
    let label_width = 6 * label.len() as u32 + 20;
    let value_width = 6 * value_owned.len() as u32 + 20;
    let total = label_width + value_width;
    format!(
r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{total}" height="20" role="img" aria-label="{label}: {value_owned}">
  <title>{label}: {value_owned}</title>
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
    <text x="{lx}" y="14">{label}</text>
    <text x="{vx}" y="14">{value_owned}</text>
  </g>
</svg>"##,
        lx = label_width / 2,
        vx = label_width + value_width / 2,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_is_green() {
        let s = render(CheckVerdict::Pass, 0);
        assert!(s.contains("#3fb950"));
        assert!(s.contains(">pass</text>"));
    }

    #[test]
    fn fail_with_count_is_red() {
        let s = render(CheckVerdict::Fail, 3);
        assert!(s.contains("#da3633"));
        assert!(s.contains("fail (3)"));
    }

    #[test]
    fn byte_stable() {
        let a = render(CheckVerdict::Pass, 0);
        let b = render(CheckVerdict::Pass, 0);
        assert_eq!(a, b);
    }

    #[test]
    fn no_javascript_t100() {
        for v in [(CheckVerdict::Pass, 0), (CheckVerdict::Fail, 3)] {
            let s = render(v.0, v.1).to_lowercase();
            assert!(!s.contains("<script"));
            assert!(!s.contains("onclick"));
            assert!(!s.contains("javascript:"));
        }
    }

    #[test]
    fn fail_with_zero_count_is_grey() {
        // Edge case : verdict Fail but failing_count = 0 (defensive).
        let s = render(CheckVerdict::Fail, 0);
        assert!(s.contains("#8b949e"));
    }
}
