//! PR #20 § 9.3 — bursty traffic (baseline + periodic spikes).

use std::time::Duration;

use super::TrafficPattern;

/// Baseline QPS punctuated by short bursts at `burst_qps`.
#[derive(Debug, Clone, Copy)]
pub struct BurstPattern {
    baseline_qps: f64,
    burst_qps: f64,
    burst_duration_s: f64,
    burst_interval_s: f64,
}

impl BurstPattern {
    #[must_use]
    pub fn new(
        baseline_qps: f64,
        burst_qps: f64,
        burst_duration: Duration,
        burst_interval: Duration,
    ) -> Self {
        Self {
            baseline_qps: baseline_qps.max(0.0),
            burst_qps: burst_qps.max(0.0),
            burst_duration_s: burst_duration.as_secs_f64().max(0.001),
            burst_interval_s: burst_interval.as_secs_f64().max(0.001),
        }
    }
}

impl TrafficPattern for BurstPattern {
    fn target_qps(&self, t: f64) -> f64 {
        // Period = interval.  Within each period, the first
        // `burst_duration_s` seconds are at `burst_qps`, the rest at
        // `baseline_qps`.
        let phase = t.rem_euclid(self.burst_interval_s);
        if phase < self.burst_duration_s {
            self.burst_qps
        } else {
            self.baseline_qps
        }
    }
    fn expected_requests(&self, total: f64) -> u64 {
        let n_full_periods = (total / self.burst_interval_s).floor() as u64;
        let leftover = total - (n_full_periods as f64) * self.burst_interval_s;
        let per_period = self.baseline_qps * (self.burst_interval_s - self.burst_duration_s)
            + self.burst_qps * self.burst_duration_s;
        let leftover_count = if leftover < self.burst_duration_s {
            self.burst_qps * leftover
        } else {
            self.burst_qps * self.burst_duration_s
                + self.baseline_qps * (leftover - self.burst_duration_s)
        };
        (per_period * n_full_periods as f64 + leftover_count) as u64
    }
    fn name(&self) -> &'static str {
        "burst"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> BurstPattern {
        BurstPattern::new(10.0, 500.0, Duration::from_millis(500), Duration::from_secs(60))
    }

    #[test]
    fn burst_within_window() {
        // At t=0..0.5s the burst is active.
        let bp = p();
        assert!((bp.target_qps(0.0) - 500.0).abs() < 1e-9);
        assert!((bp.target_qps(0.4) - 500.0).abs() < 1e-9);
    }

    #[test]
    fn baseline_outside_window() {
        let bp = p();
        assert!((bp.target_qps(0.6) - 10.0).abs() < 1e-9);
        assert!((bp.target_qps(30.0) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn burst_repeats_per_interval() {
        let bp = p();
        // t=60.2 is in the burst of the 2nd period
        assert!((bp.target_qps(60.2) - 500.0).abs() < 1e-9);
    }
}
