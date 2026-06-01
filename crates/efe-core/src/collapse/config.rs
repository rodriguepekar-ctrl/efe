//! `CollapseConfig` — tunable policy of the M7 engine.

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};

/// Tunable config of the collapse engine.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct CollapseConfig {
    /// Maximum number of collapses per swarm before forced `FullAbort`.
    pub max_collapses_per_swarm: u32,
    /// Wallclock budget for one `handle()` call (ms).
    pub deadline_ms: u32,
    /// σ used by `NoiseInjectionStrategy`.
    pub noise_sigma: f32,
    /// Clip range applied after any perturbation (matches M5's `clip_max`).
    pub clip_max: f32,
    /// Jitter σ used by `CentroidProjectionStrategy`.
    pub centroid_project_jitter: f32,
    /// Z-score threshold used by `PartialRespawnStrategy` to identify outliers.
    pub outlier_z_threshold: f32,
    /// Minimum fraction of agents that must remain active after a respawn.
    pub min_active_fraction: f32,
    /// Variance above which the collapse is considered to have failed.
    pub post_variance_max: f32,
    /// Ring-buffer capacity for `CollapseEvent` history.
    pub events_capacity: u32,
    /// When `true`, seed all PRNGs from a deterministic hash of
    /// `(swarm_id, round)` instead of wall-clock time. Used in CI.
    pub deterministic_seed: bool,
}

impl Default for CollapseConfig {
    fn default() -> Self {
        Self {
            max_collapses_per_swarm: 5,
            deadline_ms: 50,
            noise_sigma: 0.05,
            clip_max: 32.0,
            centroid_project_jitter: 0.01,
            outlier_z_threshold: 2.0,
            min_active_fraction: 0.5,
            post_variance_max: 10.0,
            events_capacity: 64,
            deterministic_seed: false,
        }
    }
}

impl CollapseConfig {
    /// Validate configuration invariants.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] if any field is out of its
    /// admissible range.
    pub fn validate(&self) -> Result<()> {
        if self.max_collapses_per_swarm == 0 {
            return Err(EfeError::InvalidConfig(
                "CollapseConfig: max_collapses_per_swarm must be ≥ 1".into(),
            ));
        }
        if self.deadline_ms == 0 {
            return Err(EfeError::InvalidConfig(
                "CollapseConfig: deadline_ms must be ≥ 1".into(),
            ));
        }
        if !self.noise_sigma.is_finite() || self.noise_sigma <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseConfig: noise_sigma {} must be finite and > 0",
                self.noise_sigma
            )));
        }
        if !self.clip_max.is_finite() || self.clip_max <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseConfig: clip_max {} must be finite and > 0",
                self.clip_max
            )));
        }
        if !self.centroid_project_jitter.is_finite() || self.centroid_project_jitter < 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseConfig: centroid_project_jitter {} must be finite and ≥ 0",
                self.centroid_project_jitter
            )));
        }
        if !self.outlier_z_threshold.is_finite() || self.outlier_z_threshold <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseConfig: outlier_z_threshold {} must be finite and > 0",
                self.outlier_z_threshold
            )));
        }
        if !(0.0..=1.0).contains(&self.min_active_fraction) || self.min_active_fraction <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseConfig: min_active_fraction {} must be in (0, 1]",
                self.min_active_fraction
            )));
        }
        if !self.post_variance_max.is_finite() || self.post_variance_max <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "CollapseConfig: post_variance_max {} must be finite and > 0",
                self.post_variance_max
            )));
        }
        if self.events_capacity == 0 {
            return Err(EfeError::InvalidConfig(
                "CollapseConfig: events_capacity must be ≥ 1".into(),
            ));
        }
        Ok(())
    }

    /// Fluent setter — override `max_collapses_per_swarm`.
    #[must_use]
    pub fn with_max_collapses(mut self, n: u32) -> Self {
        self.max_collapses_per_swarm = n;
        self
    }

    /// Fluent setter — override `deadline_ms`.
    #[must_use]
    pub fn with_deadline_ms(mut self, ms: u32) -> Self {
        self.deadline_ms = ms;
        self
    }

    /// Fluent setter — override `deterministic_seed`.
    #[must_use]
    pub fn with_deterministic_seed(mut self, d: bool) -> Self {
        self.deterministic_seed = d;
        self
    }

    /// Fluent setter — override `noise_sigma`.
    #[must_use]
    pub fn with_noise_sigma(mut self, s: f32) -> Self {
        self.noise_sigma = s;
        self
    }

    /// Fluent setter — override `post_variance_max`.
    #[must_use]
    pub fn with_post_variance_max(mut self, v: f32) -> Self {
        self.post_variance_max = v;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        CollapseConfig::default().validate().unwrap();
    }

    #[test]
    fn rejects_zero_max_collapses() {
        let c = CollapseConfig::default().with_max_collapses(0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_zero_deadline() {
        let c = CollapseConfig::default().with_deadline_ms(0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_nan_noise_sigma() {
        let c = CollapseConfig::default().with_noise_sigma(f32::NAN);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_zero_noise_sigma() {
        let c = CollapseConfig::default().with_noise_sigma(0.0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_non_positive_clip_max() {
        let c = CollapseConfig {
            clip_max: 0.0,
            ..CollapseConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_negative_jitter() {
        let c = CollapseConfig {
            centroid_project_jitter: -0.1,
            ..CollapseConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn accepts_zero_jitter() {
        let c = CollapseConfig {
            centroid_project_jitter: 0.0,
            ..CollapseConfig::default()
        };
        c.validate().unwrap();
    }

    #[test]
    fn rejects_out_of_range_min_active_fraction() {
        let c = CollapseConfig {
            min_active_fraction: 1.5,
            ..CollapseConfig::default()
        };
        assert!(c.validate().is_err());
        let c = CollapseConfig {
            min_active_fraction: 0.0,
            ..CollapseConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_non_positive_post_variance_max() {
        let c = CollapseConfig {
            post_variance_max: 0.0,
            ..CollapseConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_zero_events_capacity() {
        let c = CollapseConfig {
            events_capacity: 0,
            ..CollapseConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn serde_round_trip() {
        let c = CollapseConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: CollapseConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }
}
