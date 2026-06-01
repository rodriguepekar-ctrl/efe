//! PR #20 § 9 — traffic pattern engines.
//!
//! 5 patterns implement [`TrafficPattern`].  The engine picks one per
//! run based on the merged config and queries `target_qps(t)` on every
//! rate-controller tick.

use std::time::Duration;

use crate::config::PatternSpec;

pub mod burst;
pub mod constant;
pub mod diurnal;
pub mod ramp;
pub mod replay;

/// Common interface for traffic patterns.
pub trait TrafficPattern: Send + Sync {
    /// Return the target QPS at time `t` (seconds since start).
    fn target_qps(&self, t: f64) -> f64;

    /// Expected request count over `total_duration` seconds.  Used by
    /// the engine for pre-allocation + progress reporting.
    fn expected_requests(&self, total_duration: f64) -> u64;

    /// Friendly name (matches the YAML `pattern.type` enum value).
    fn name(&self) -> &'static str;
}

/// Factory : build a boxed [`TrafficPattern`] from a [`PatternSpec`].
#[must_use]
pub fn from_spec(spec: &PatternSpec) -> Box<dyn TrafficPattern> {
    match spec {
        PatternSpec::Constant { qps } => Box::new(constant::ConstantPattern::new(*qps)),
        PatternSpec::Ramp {
            initial_qps,
            final_qps,
            shape,
            duration,
        } => Box::new(ramp::RampPattern::new(*initial_qps, *final_qps, *shape, *duration)),
        PatternSpec::Burst {
            baseline_qps,
            burst_qps,
            burst_duration,
            burst_interval,
        } => Box::new(burst::BurstPattern::new(
            *baseline_qps,
            *burst_qps,
            *burst_duration,
            *burst_interval,
        )),
        PatternSpec::Diurnal {
            daily_min_qps,
            daily_max_qps,
            period,
            phase,
        } => Box::new(diurnal::DiurnalPattern::new(
            *daily_min_qps,
            *daily_max_qps,
            *period,
            *phase,
        )),
        PatternSpec::Replay { file, mode, speed } => {
            Box::new(replay::ReplayPattern::new(file.clone(), *mode, *speed))
        }
    }
}

/// Apply a ramp-up envelope to any base pattern.  Scales the target
/// QPS linearly from 0 → 1 over the `ramp_up` window.
#[must_use]
pub fn with_ramp_up(
    base: Box<dyn TrafficPattern>,
    ramp_up: Duration,
) -> Box<dyn TrafficPattern> {
    Box::new(RampedPattern { base, ramp_up_s: ramp_up.as_secs_f64() })
}

struct RampedPattern {
    base: Box<dyn TrafficPattern>,
    ramp_up_s: f64,
}

impl TrafficPattern for RampedPattern {
    fn target_qps(&self, t: f64) -> f64 {
        let base = self.base.target_qps(t);
        if self.ramp_up_s <= 0.0 || t >= self.ramp_up_s {
            base
        } else {
            base * (t / self.ramp_up_s).clamp(0.0, 1.0)
        }
    }
    fn expected_requests(&self, total: f64) -> u64 {
        // Approximate : pretend ramp half-saturates the early window.
        let base = self.base.expected_requests(total);
        let ramped_loss = (self.ramp_up_s / 2.0).min(total) * self.base.target_qps(total / 2.0) / 2.0;
        base.saturating_sub(ramped_loss as u64)
    }
    fn name(&self) -> &'static str {
        self.base.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_up_scales_linearly() {
        let base = Box::new(constant::ConstantPattern::new(100.0));
        let p = with_ramp_up(base, Duration::from_secs(10));
        assert!((p.target_qps(0.0) - 0.0).abs() < 1e-9);
        assert!((p.target_qps(5.0) - 50.0).abs() < 1e-9);
        assert!((p.target_qps(10.0) - 100.0).abs() < 1e-9);
        assert!((p.target_qps(20.0) - 100.0).abs() < 1e-9);
    }

    #[test]
    fn factory_dispatches_correct_pattern() {
        let p = from_spec(&PatternSpec::Constant { qps: 5.0 });
        assert_eq!(p.name(), "constant");
        let p = from_spec(&PatternSpec::Ramp {
            initial_qps: 1.0,
            final_qps: 10.0,
            shape: crate::config::RampShape::Linear,
            duration: Duration::from_secs(10),
        });
        assert_eq!(p.name(), "ramp");
    }
}
