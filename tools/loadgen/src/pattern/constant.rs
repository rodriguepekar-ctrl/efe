//! PR #20 § 9.1 — constant QPS.

use super::TrafficPattern;

/// Fixed QPS throughout the run.  The simplest, most operationally
/// useful pattern — default for `--pattern=constant`.
#[derive(Debug, Clone, Copy)]
pub struct ConstantPattern {
    qps: f64,
}

impl ConstantPattern {
    #[must_use]
    pub const fn new(qps: f64) -> Self {
        Self { qps: if qps < 0.0 { 0.0 } else { qps } }
    }
}

impl TrafficPattern for ConstantPattern {
    fn target_qps(&self, _t: f64) -> f64 {
        self.qps
    }
    fn expected_requests(&self, total_duration: f64) -> u64 {
        (self.qps * total_duration).max(0.0) as u64
    }
    fn name(&self) -> &'static str {
        "constant"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_returns_same_qps_at_any_time() {
        let p = ConstantPattern::new(42.0);
        assert!((p.target_qps(0.0) - 42.0).abs() < 1e-9);
        assert!((p.target_qps(100.0) - 42.0).abs() < 1e-9);
        assert!((p.target_qps(1e9) - 42.0).abs() < 1e-9);
    }

    #[test]
    fn expected_requests_is_qps_times_duration() {
        let p = ConstantPattern::new(50.0);
        assert_eq!(p.expected_requests(60.0), 3000);
    }

    #[test]
    fn negative_qps_clamped_to_zero() {
        let p = ConstantPattern::new(-1.0);
        assert_eq!(p.target_qps(0.0), 0.0);
    }
}
