//! PR #20 § 9.2 — linear / exponential ramp.

use std::time::Duration;

use super::TrafficPattern;
use crate::config::RampShape;

/// QPS ramp from `initial_qps` to `final_qps` over `duration`.  After
/// `duration`, the QPS stays at `final_qps` (clamped).
#[derive(Debug, Clone, Copy)]
pub struct RampPattern {
    initial_qps: f64,
    final_qps: f64,
    shape: RampShape,
    duration_s: f64,
}

impl RampPattern {
    #[must_use]
    pub fn new(initial_qps: f64, final_qps: f64, shape: RampShape, duration: Duration) -> Self {
        Self {
            initial_qps: initial_qps.max(0.0),
            final_qps: final_qps.max(0.0),
            shape,
            duration_s: duration.as_secs_f64().max(0.0),
        }
    }
}

impl TrafficPattern for RampPattern {
    fn target_qps(&self, t: f64) -> f64 {
        if self.duration_s <= 0.0 {
            return self.final_qps;
        }
        let progress = (t / self.duration_s).clamp(0.0, 1.0);
        match self.shape {
            RampShape::Linear => self.initial_qps + (self.final_qps - self.initial_qps) * progress,
            RampShape::Exponential => {
                // Logarithmic interpolation : QPS(t) = initial * (final/initial)^progress
                if self.initial_qps <= 0.0 {
                    // Degenerate ; fall back to linear.
                    self.final_qps * progress
                } else {
                    let ratio = (self.final_qps / self.initial_qps).max(0.0);
                    self.initial_qps * ratio.powf(progress)
                }
            }
        }
    }
    fn expected_requests(&self, total: f64) -> u64 {
        // Trapezoid for linear ; integration of geometric for exponential.
        let ramp_window = self.duration_s.min(total);
        let avg_during_ramp = match self.shape {
            RampShape::Linear => 0.5 * (self.initial_qps + self.final_qps),
            RampShape::Exponential => {
                if self.initial_qps <= 0.0 || self.final_qps <= 0.0 {
                    0.5 * (self.initial_qps + self.final_qps)
                } else {
                    // Average of an exponential : (final - initial) / ln(final/initial)
                    let diff = self.final_qps - self.initial_qps;
                    let ratio = self.final_qps / self.initial_qps;
                    if (ratio - 1.0).abs() < 1e-9 {
                        self.final_qps
                    } else {
                        diff / ratio.ln()
                    }
                }
            }
        };
        let after_ramp = (total - ramp_window).max(0.0);
        ((avg_during_ramp * ramp_window) + (self.final_qps * after_ramp)) as u64
    }
    fn name(&self) -> &'static str {
        "ramp"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_endpoints_match() {
        let p = RampPattern::new(0.0, 100.0, RampShape::Linear, Duration::from_secs(10));
        assert!((p.target_qps(0.0) - 0.0).abs() < 1e-9);
        assert!((p.target_qps(5.0) - 50.0).abs() < 1e-9);
        assert!((p.target_qps(10.0) - 100.0).abs() < 1e-9);
        // Past the ramp, holds at final.
        assert!((p.target_qps(60.0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn exponential_grows_geometrically() {
        let p = RampPattern::new(1.0, 100.0, RampShape::Exponential, Duration::from_secs(10));
        let mid = p.target_qps(5.0);
        // Midpoint must be ~ sqrt(1 * 100) = 10
        assert!((mid - 10.0).abs() < 0.1, "mid = {mid}");
    }

    #[test]
    fn expected_requests_linear() {
        let p = RampPattern::new(0.0, 100.0, RampShape::Linear, Duration::from_secs(10));
        // Trapezoid 0..100 over 10s = 500 ; then 100 * 50s = 5000 ; total 5500
        let expected = p.expected_requests(60.0);
        assert!((5400..=5600).contains(&expected), "got {expected}");
    }

    #[test]
    fn ramp_zero_duration_returns_final_immediately() {
        let p = RampPattern::new(1.0, 100.0, RampShape::Linear, Duration::ZERO);
        assert!((p.target_qps(0.0) - 100.0).abs() < 1e-9);
    }
}
