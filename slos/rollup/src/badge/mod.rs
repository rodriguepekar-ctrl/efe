//! PR #22 § 12 + AD43 — 4 SVG badge generators.
//!
//! Each generator emits a **deterministic byte-stable** SVG for one
//! SLO.  No randomness, no timestamps in the body.
//!
//! AD42 + AD43 — file names frozen :
//!   * `slo-1-availability.svg`
//!   * `slo-2-latency-p95.svg`
//!   * `slo-3-recovery-time.svg`
//!   * `slo-4-replication-lag.svg`

use crate::sli::SliVerdict;
use crate::SloId;

/// Map a verdict to a hex colour (HTML-safe).
#[must_use]
pub const fn colour_for(verdict: SliVerdict) -> &'static str {
    match verdict {
        SliVerdict::Pass => "#3fb950",        // green
        SliVerdict::Fail => "#da3633",        // red
        SliVerdict::Indeterminate => "#8b949e", // grey
    }
}

/// Generate a badge for a single SLO.
#[must_use]
pub fn render(slo: SloId, value: f64, verdict: SliVerdict) -> String {
    let label = label_for(slo);
    let val_str = value_str_for(slo, value, verdict);
    let colour = colour_for(verdict);
    let label_width = 6 * label.len() as u32 + 20;
    let value_width = 6 * val_str.len() as u32 + 20;
    let total = label_width + value_width;
    format!(
r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{total}" height="20" role="img" aria-label="{label}: {val_str}">
  <title>{label}: {val_str}</title>
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
    <text x="{vx}" y="14">{val_str}</text>
  </g>
</svg>"##,
        lx = label_width / 2,
        vx = label_width + value_width / 2,
    )
}

/// Frozen file-name for the badge of `slo` (AD43).
#[must_use]
pub const fn filename(slo: SloId) -> &'static str {
    match slo {
        SloId::L2Availability => "slo-1-availability.svg",
        SloId::L2LatencyP95 => "slo-2-latency-p95.svg",
        SloId::SwarmRecoveryTime => "slo-3-recovery-time.svg",
        SloId::ReplicationLagP99 => "slo-4-replication-lag.svg",
    }
}

/// Frozen label shown on the LEFT side of the badge.
#[must_use]
pub const fn label_for(slo: SloId) -> &'static str {
    match slo {
        SloId::L2Availability => "L2 availability",
        SloId::L2LatencyP95 => "L2 latency p95",
        SloId::SwarmRecoveryTime => "swarm recovery",
        SloId::ReplicationLagP99 => "replication lag",
    }
}

/// Frozen value-rendering rules per SLO.
#[must_use]
pub fn value_str_for(slo: SloId, value: f64, verdict: SliVerdict) -> String {
    if matches!(verdict, SliVerdict::Indeterminate) || value.is_nan() {
        return "no data".into();
    }
    match slo {
        SloId::L2Availability => format!("{:.3}%", value * 100.0),
        SloId::L2LatencyP95 => format!("{:.1}% comp", value * 100.0),
        SloId::SwarmRecoveryTime => format!("{:.1}% comp", value * 100.0),
        SloId::ReplicationLagP99 => format!("{:.1}% comp", value * 100.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_pass_is_green() {
        assert_eq!(colour_for(SliVerdict::Pass), "#3fb950");
    }

    #[test]
    fn colour_fail_is_red() {
        assert_eq!(colour_for(SliVerdict::Fail), "#da3633");
    }

    #[test]
    fn colour_indeterminate_is_grey() {
        assert_eq!(colour_for(SliVerdict::Indeterminate), "#8b949e");
    }

    #[test]
    fn filenames_match_v1_4_frozen() {
        assert_eq!(filename(SloId::L2Availability), "slo-1-availability.svg");
        assert_eq!(filename(SloId::L2LatencyP95), "slo-2-latency-p95.svg");
        assert_eq!(filename(SloId::SwarmRecoveryTime), "slo-3-recovery-time.svg");
        assert_eq!(filename(SloId::ReplicationLagP99), "slo-4-replication-lag.svg");
    }

    #[test]
    fn badge_is_byte_stable() {
        let a = render(SloId::L2Availability, 0.999, SliVerdict::Pass);
        let b = render(SloId::L2Availability, 0.999, SliVerdict::Pass);
        assert_eq!(a, b);
    }

    #[test]
    fn badge_starts_with_xml_declaration() {
        let s = render(SloId::L2Availability, 0.999, SliVerdict::Pass);
        assert!(s.starts_with("<?xml"));
        assert!(s.contains("</svg>"));
    }

    #[test]
    fn indeterminate_shows_no_data() {
        let s = render(SloId::L2Availability, f64::NAN, SliVerdict::Indeterminate);
        assert!(s.contains("no data"));
    }

    #[test]
    fn fail_badge_is_red() {
        let s = render(SloId::L2Availability, 0.99, SliVerdict::Fail);
        assert!(s.contains("#da3633"));
    }

    #[test]
    fn no_javascript_in_badge_t100() {
        // T100 mitigation : badges are pure SVG, no scripting.
        for id in SloId::all() {
            let s = render(id, 0.999, SliVerdict::Pass);
            assert!(!s.contains("<script"));
            assert!(!s.contains("onclick"));
            assert!(!s.contains("javascript:"));
        }
    }
}
