//! PR #20 § 9.5 — replay from a recorded session.
//!
//! The replay pattern doesn't strictly produce a QPS curve — it
//! delegates timing to the recorded inter-arrival schedule.  The
//! [`TrafficPattern`] implementation here returns a *nominal* QPS so
//! the engine can still display progress ; the actual replay timing
//! is driven by [`crate::replay`].

use std::path::PathBuf;

use super::TrafficPattern;
use crate::config::ReplayPlaybackMode;

/// Pattern adapter for replayed sessions.
#[derive(Debug, Clone)]
pub struct ReplayPattern {
    file: PathBuf,
    mode: ReplayPlaybackMode,
    speed: f64,
    /// Cached "nominal QPS" — computed at engine startup by inspecting
    /// the recorded file ; set via [`ReplayPattern::set_nominal_qps`].
    nominal_qps: f64,
}

impl ReplayPattern {
    #[must_use]
    pub fn new(file: PathBuf, mode: ReplayPlaybackMode, speed: f64) -> Self {
        Self { file, mode, speed: speed.max(0.01), nominal_qps: 0.0 }
    }

    /// Source file path.
    #[must_use]
    pub fn file(&self) -> &PathBuf {
        &self.file
    }

    /// Replay timing mode.
    #[must_use]
    pub const fn mode(&self) -> ReplayPlaybackMode {
        self.mode
    }

    /// Speed multiplier.
    #[must_use]
    pub const fn speed(&self) -> f64 {
        self.speed
    }

    /// Set the nominal QPS post-load (the engine calls this once it has
    /// inspected the recording).
    pub fn set_nominal_qps(&mut self, qps: f64) {
        self.nominal_qps = qps.max(0.0);
    }
}

impl TrafficPattern for ReplayPattern {
    fn target_qps(&self, _t: f64) -> f64 {
        // Replay mode bypasses the rate controller — but return a value
        // for stats/TUI displays.  Drain mode reports infinite ;
        // accelerate scales by speed.
        match self.mode {
            ReplayPlaybackMode::Drain => f64::INFINITY,
            ReplayPlaybackMode::Accelerate => self.nominal_qps * self.speed,
            ReplayPlaybackMode::PreserveSpacing => self.nominal_qps,
        }
    }
    fn expected_requests(&self, _total: f64) -> u64 {
        // Set when the engine loads the recording file.  Defaults to 0
        // — the progress bar will adjust mid-run.
        0
    }
    fn name(&self) -> &'static str {
        "replay"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_preserve_spacing_uses_nominal() {
        let mut p = ReplayPattern::new(
            "/tmp/recording.parquet".into(),
            ReplayPlaybackMode::PreserveSpacing,
            1.0,
        );
        p.set_nominal_qps(42.0);
        assert!((p.target_qps(0.0) - 42.0).abs() < 1e-9);
    }

    #[test]
    fn replay_accelerate_scales_by_speed() {
        let mut p = ReplayPattern::new(
            "/tmp/recording.parquet".into(),
            ReplayPlaybackMode::Accelerate,
            10.0,
        );
        p.set_nominal_qps(50.0);
        assert!((p.target_qps(0.0) - 500.0).abs() < 1e-9);
    }

    #[test]
    fn replay_drain_returns_infinity() {
        let p = ReplayPattern::new("/tmp/r.parquet".into(), ReplayPlaybackMode::Drain, 1.0);
        assert!(p.target_qps(0.0).is_infinite());
    }
}
