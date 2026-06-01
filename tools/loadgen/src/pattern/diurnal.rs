//! PR #20 § 9.4 — sinusoidal day/night cycle.

use std::f64::consts::TAU;
use std::time::Duration;

use super::TrafficPattern;

/// QPS(t) = (max+min)/2 + (max-min)/2 * sin(2π(t + phase) / period)
/// Centred so the average over the period is `(min+max)/2`.
#[derive(Debug, Clone, Copy)]
pub struct DiurnalPattern {
    min_qps: f64,
    max_qps: f64,
    period_s: f64,
    phase_s: f64,
}

impl DiurnalPattern {
    #[must_use]
    pub fn new(min_qps: f64, max_qps: f64, period: Duration, phase: Duration) -> Self {
        Self {
            min_qps: min_qps.max(0.0),
            max_qps: max_qps.max(0.0),
            period_s: period.as_secs_f64().max(1.0),
            phase_s: phase.as_secs_f64(),
        }
    }
}

impl TrafficPattern for DiurnalPattern {
    fn target_qps(&self, t: f64) -> f64 {
        let mean = 0.5 * (self.min_qps + self.max_qps);
        let amp = 0.5 * (self.max_qps - self.min_qps);
        let theta = TAU * (t + self.phase_s) / self.period_s;
        mean + amp * theta.sin()
    }
    fn expected_requests(&self, total: f64) -> u64 {
        // Average QPS over an integer number of periods equals the mean.
        // For partial periods we ignore the small sinusoidal residue.
        let mean = 0.5 * (self.min_qps + self.max_qps);
        (mean * total).max(0.0) as u64
    }
    fn name(&self) -> &'static str {
        "diurnal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diurnal_mean_qps_matches_average() {
        let d = DiurnalPattern::new(10.0, 100.0, Duration::from_secs(86400), Duration::ZERO);
        // mean = 55
        let mut acc = 0.0;
        let n = 1000usize;
        for i in 0..n {
            acc += d.target_qps((i as f64) * 86400.0 / (n as f64));
        }
        let avg = acc / n as f64;
        assert!((avg - 55.0).abs() < 2.0, "avg = {avg}");
    }

    #[test]
    fn diurnal_peak_at_period_quarter() {
        let d = DiurnalPattern::new(0.0, 100.0, Duration::from_secs(4), Duration::ZERO);
        // Peak at t = period/4 = 1s
        let peak = d.target_qps(1.0);
        assert!((peak - 100.0).abs() < 1e-9, "peak = {peak}");
    }

    #[test]
    fn diurnal_trough_at_three_quarters() {
        let d = DiurnalPattern::new(0.0, 100.0, Duration::from_secs(4), Duration::ZERO);
        // Trough at t = 3 * period/4 = 3s
        let trough = d.target_qps(3.0);
        assert!((trough - 0.0).abs() < 1e-9, "trough = {trough}");
    }
}
