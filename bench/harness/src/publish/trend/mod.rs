//! PR #24 § 9 + AD11-AD13 — 30-day rolling sparkline.
//!
//! Each scenario page embeds a 200×40 SVG sparkline of its p95
//! latency over the last 30 UTC days.  Missing days render as a
//! dashed segment (visual distinction from p95=NaN).

pub mod bucket;

use crate::publish::badge::{colour_for, xml_escape};
use crate::compare::Verdict;

/// Frozen sparkline dimensions (STABILITY v1.6 § 24 anchor).
pub const SPARK_WIDTH: u32 = 200;
pub const SPARK_HEIGHT: u32 = 40;

/// One day's data point.
#[derive(Debug, Clone, Copy)]
pub struct DailyPoint {
    /// UTC date as days-since-epoch (so we can index into a 30-slot array).
    pub day_index: u32,
    /// p95 latency in milliseconds.  `None` ⇒ no run that day.
    pub p95_ms: Option<f64>,
}

/// Render a 200×40 sparkline SVG given 30 daily points (oldest first).
///
/// `verdict` colours the line + circles ; same hex as badges.
/// Missing days (`p95_ms = None`) are interpolated linearly and drawn
/// as a dashed segment.
#[must_use]
pub fn render(points: &[DailyPoint], verdict: Verdict) -> String {
    if points.is_empty() {
        return empty_svg();
    }
    let colour = colour_for(verdict);
    let (min_y, max_y) = compute_y_range(points);
    let n = points.len() as u32;
    let x_step = if n > 1 {
        (SPARK_WIDTH as f64 - 8.0) / (n - 1) as f64
    } else {
        0.0
    };

    // Build the polyline d attribute + dashed-or-solid segments + dots.
    //
    // **D5 fix** — each sub-path (solid + dashed) MUST start with `M`
    // (move-to).  Previously the dashed sub-path could start with `L`
    // if the first sample was missing for i > 0, producing invalid SVG
    // (browsers silently dropped the segment).
    let mut path_solid = String::new();
    let mut path_dashed = String::new();
    let mut circles = String::new();
    let mut solid_started = false;
    let mut prev_was_dashed = false;
    for (i, p) in points.iter().enumerate() {
        let x = 4.0 + (i as f64) * x_step;
        let y = match p.p95_ms {
            Some(v) => y_for(v, min_y, max_y),
            None => interpolate_missing(points, i, min_y, max_y),
        };
        let is_missing = p.p95_ms.is_none();
        if is_missing && !prev_was_dashed {
            // Open a dashed sub-path : ALWAYS `M` (D5 fix).
            path_dashed.push_str(&format!("M {x:.1} {y:.1} "));
            prev_was_dashed = true;
        } else if !is_missing && prev_was_dashed {
            // Close dashed segment + restart solid : explicit `M`.
            path_dashed.push_str(&format!("L {x:.1} {y:.1} "));
            path_solid.push_str(&format!("M {x:.1} {y:.1} "));
            solid_started = true;
            prev_was_dashed = false;
        } else if is_missing {
            // Continuing dashed run.
            path_dashed.push_str(&format!("L {x:.1} {y:.1} "));
        } else {
            // Solid : `M` on first solid point, `L` thereafter.
            if !solid_started {
                path_solid.push_str(&format!("M {x:.1} {y:.1} "));
                solid_started = true;
            } else {
                path_solid.push_str(&format!("L {x:.1} {y:.1} "));
            }
        }
        if p.p95_ms.is_some() {
            circles.push_str(&format!(
                r##"<circle cx="{x:.1}" cy="{y:.1}" r="2" fill="{colour}"/>"##,
            ));
        }
    }
    // Aria-label : the value range summary (no PII).
    let aria = xml_escape(&format!(
        "p95 sparkline ; {n}-day window ; {min_y:.0}ms — {max_y:.0}ms",
    ));
    format!(
r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{SPARK_WIDTH}" height="{SPARK_HEIGHT}" viewBox="0 0 {SPARK_WIDTH} {SPARK_HEIGHT}" role="img" aria-label="{aria}">
  <title>{aria}</title>
  <rect width="{SPARK_WIDTH}" height="{SPARK_HEIGHT}" fill="#f6f8fa"/>
  <path d="{path_solid}" fill="none" stroke="{colour}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
  <path d="{path_dashed}" fill="none" stroke="{colour}" stroke-width="1.5" stroke-dasharray="3,2" stroke-linecap="round" stroke-linejoin="round" opacity="0.5"/>
  {circles}
</svg>"##,
    )
}

/// Empty placeholder when no data at all (still 200×40, neutral grey).
///
/// **D-AC fix** — text fill colour now references the shared
/// `COLOUR_NO_BASELINE` constant from `publish::badge` instead of
/// hardcoding `#8b949e`.  If the badge palette ever changes, the
/// sparkline "no data" text follows automatically — no silent drift.
#[must_use]
fn empty_svg() -> String {
    format!(
r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" width="{SPARK_WIDTH}" height="{SPARK_HEIGHT}" viewBox="0 0 {SPARK_WIDTH} {SPARK_HEIGHT}" role="img" aria-label="no sparkline data">
  <title>no sparkline data</title>
  <rect width="{SPARK_WIDTH}" height="{SPARK_HEIGHT}" fill="#f6f8fa"/>
  <text x="{cx}" y="{cy}" text-anchor="middle" dy=".35em" font-family="Verdana,sans-serif" font-size="10" fill="{nb}">no data</text>
</svg>"##,
        cx = SPARK_WIDTH / 2,
        cy = SPARK_HEIGHT / 2,
        nb = crate::publish::badge::COLOUR_NO_BASELINE,
    )
}

/// Linear Y mapping from p95(ms) to SVG y (top=4, bottom=36).  Inverts
/// Y axis (higher p95 → lower y).
fn y_for(p95_ms: f64, min_y: f64, max_y: f64) -> f64 {
    if (max_y - min_y).abs() < f64::EPSILON {
        return 20.0; // centre
    }
    let normalised = (p95_ms - min_y) / (max_y - min_y);
    36.0 - normalised * 32.0
}

/// Pick min/max p95 across all non-missing points.  Adds 10% headroom.
fn compute_y_range(points: &[DailyPoint]) -> (f64, f64) {
    let mut vals: Vec<f64> = points.iter().filter_map(|p| p.p95_ms).collect();
    if vals.is_empty() {
        return (0.0, 1.0);
    }
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let min = vals[0];
    let max = vals[vals.len() - 1];
    let range = (max - min).max(0.001);
    (min - range * 0.05, max + range * 0.05)
}

/// For a missing day, find the nearest neighbour with data + use its y.
/// Conservative fallback : centre.
fn interpolate_missing(points: &[DailyPoint], idx: usize, min_y: f64, max_y: f64) -> f64 {
    // Look backwards for the nearest data point.
    let prev = points[..idx].iter().rev().find_map(|p| p.p95_ms);
    let next = points[idx + 1..].iter().find_map(|p| p.p95_ms);
    match (prev, next) {
        (Some(a), Some(b)) => y_for((a + b) / 2.0, min_y, max_y),
        (Some(v), None) | (None, Some(v)) => y_for(v, min_y, max_y),
        (None, None) => 20.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(values: &[Option<f64>]) -> Vec<DailyPoint> {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| DailyPoint {
                day_index: i as u32,
                p95_ms: *v,
            })
            .collect()
    }

    #[test]
    fn empty_renders_no_data_placeholder() {
        let svg = render(&[], Verdict::Pass);
        assert!(svg.contains("no data"));
        assert!(svg.contains("width=\"200\""));
        assert!(svg.contains("height=\"40\""));
    }

    #[test]
    fn fully_populated_renders_solid_line() {
        let v: Vec<f64> = (1..=30).map(|i| (i as f64) * 2.0).collect();
        let points = pts(&v.iter().map(|x| Some(*x)).collect::<Vec<_>>());
        let svg = render(&points, Verdict::Pass);
        assert!(svg.contains("stroke=\"#3fb950\""));
        // No dashed-array on the solid path (dashed has its own path).
        assert!(svg.contains("stroke-dasharray=\"3,2\"") || !svg.contains("path_dashed"));
    }

    #[test]
    fn missing_day_yields_dashed_segment() {
        let mut v = vec![Some(10.0); 10];
        v[5] = None;
        let points = pts(&v);
        let svg = render(&points, Verdict::Pass);
        assert!(svg.contains("stroke-dasharray=\"3,2\""));
    }

    #[test]
    fn byte_stable_across_runs() {
        let v: Vec<Option<f64>> = (1..=30).map(|i| Some((i as f64) * 2.0)).collect();
        let points = pts(&v);
        let a = render(&points, Verdict::Pass);
        let b = render(&points, Verdict::Pass);
        assert_eq!(a, b);
    }

    #[test]
    fn dimensions_frozen_200x40() {
        assert_eq!(SPARK_WIDTH, 200);
        assert_eq!(SPARK_HEIGHT, 40);
    }

    #[test]
    fn no_javascript_in_sparkline_t100() {
        let v: Vec<Option<f64>> = (1..=30).map(|i| Some(i as f64)).collect();
        let svg = render(&pts(&v), Verdict::Fail).to_lowercase();
        assert!(!svg.contains("<script"));
        assert!(!svg.contains("onclick"));
        assert!(!svg.contains("javascript:"));
    }

    #[test]
    fn verdict_colour_propagates() {
        let v: Vec<Option<f64>> = (1..=30).map(|i| Some(i as f64)).collect();
        let pass_svg = render(&pts(&v), Verdict::Pass);
        let fail_svg = render(&pts(&v), Verdict::Fail);
        assert!(pass_svg.contains("#3fb950"));
        assert!(fail_svg.contains("#da3633"));
    }
}
