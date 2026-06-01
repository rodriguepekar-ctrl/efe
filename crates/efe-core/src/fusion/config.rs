//! `FusionConfig` — the knobs of a fusion campaign.
//!
//! Carefully validated at construction time to avoid surprises deep in a
//! fusion round. The defaults are tuned for the MVP swarm (N=128,
//! D=2048) and should converge in under 16 rounds on synthetic data.

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};

/// Configuration for one fusion campaign (N rounds of swarm attention).
///
/// Constructed via the builder or directly (with `#[non_exhaustive]`
/// tolerance). Always call `validate()` before use — the `FusionStrategy`
/// implementations call it internally.
///
/// The struct intentionally packs 4 boolean flags. Each flag is
/// orthogonal — splitting them into a state machine would inflate the
/// call-site code without any real gain. The `#[allow]` is surgical.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
#[allow(clippy::struct_excessive_bools)]
pub struct FusionConfig {
    /// Maximum number of rounds. Guards against non-convergence.
    pub max_rounds: u32,
    /// Hard wallclock budget per round (ms). Kernel honors deadline.
    pub round_deadline_ms: u32,
    /// Temperature scale for softmax. Higher = softer distribution.
    pub temperature: f32,
    /// When `true`, mask self-attention (α_ii = 0).
    pub mask_self_attention: bool,
    /// Initial alignment force α₀.
    pub alignment_strength_init: f32,
    /// Minimum alignment force (asymptote).
    pub alignment_strength_floor: f32,
    /// Initial β₀ weight on problem embedding.
    pub problem_anchor_init: f32,
    /// Exponential decay rate of β across rounds (λ_β).
    pub problem_anchor_decay: f32,
    /// Clip threshold (coordinate-wise).
    pub clip_max: f32,
    /// When `true`, run post-round validator. Defaults to debug-only.
    pub validate_post: bool,
    /// Enable replay buffer (required for M7 rollback).
    pub enable_replay: bool,
    /// Replay buffer capacity (# of snapshots retained).
    pub replay_capacity: u32,
    /// When `true`, return `FusionWarning::DeadlineExceeded` on overrun;
    /// when `false`, round still completes, warning is emitted.
    pub strict_deadline: bool,
    /// Multiplicative ratio above which a post-round variance increase
    /// triggers `FusionWarning::HighVarianceDelta`.
    ///
    /// Default: `1.5` — variance more than 1.5× the pre-round value
    /// (plus a tiny `1e-6` constant for absolute stability) is flagged.
    /// Set to `f32::INFINITY` to disable the warning.
    pub variance_warn_ratio: f32,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            max_rounds: 16,
            round_deadline_ms: 30,
            temperature: 1.0,
            mask_self_attention: false,
            alignment_strength_init: 0.5,
            alignment_strength_floor: 0.05,
            problem_anchor_init: 0.2,
            problem_anchor_decay: 0.15,
            clip_max: 32.0,
            validate_post: cfg!(debug_assertions),
            enable_replay: true,
            replay_capacity: 4,
            strict_deadline: false,
            variance_warn_ratio: 1.5,
        }
    }
}

impl FusionConfig {
    /// Validate the configuration eagerly.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] on any unsound field.
    pub fn validate(&self) -> Result<()> {
        if self.max_rounds == 0 {
            return Err(EfeError::InvalidConfig(
                "FusionConfig: max_rounds must be > 0".into(),
            ));
        }
        if self.round_deadline_ms == 0 {
            return Err(EfeError::InvalidConfig(
                "FusionConfig: round_deadline_ms must be > 0".into(),
            ));
        }
        if !self.temperature.is_finite() || self.temperature <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: temperature {} must be finite and > 0",
                self.temperature
            )));
        }
        if !(0.0..=1.0).contains(&self.alignment_strength_init) {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: alignment_strength_init {} must be in [0, 1]",
                self.alignment_strength_init
            )));
        }
        if !(0.0..=1.0).contains(&self.alignment_strength_floor) {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: alignment_strength_floor {} must be in [0, 1]",
                self.alignment_strength_floor
            )));
        }
        if self.alignment_strength_floor > self.alignment_strength_init {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: floor {} > init {}",
                self.alignment_strength_floor, self.alignment_strength_init
            )));
        }
        if !(0.0..=1.0).contains(&self.problem_anchor_init) {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: problem_anchor_init {} must be in [0, 1]",
                self.problem_anchor_init
            )));
        }
        if !self.problem_anchor_decay.is_finite() || self.problem_anchor_decay < 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: problem_anchor_decay {} must be finite and ≥ 0",
                self.problem_anchor_decay
            )));
        }
        if !self.clip_max.is_finite() || self.clip_max <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: clip_max {} must be finite and > 0",
                self.clip_max
            )));
        }
        if self.enable_replay && self.replay_capacity == 0 {
            return Err(EfeError::InvalidConfig(
                "FusionConfig: replay_capacity must be > 0 when replay enabled".into(),
            ));
        }
        if self.variance_warn_ratio.is_nan() || self.variance_warn_ratio < 1.0 {
            return Err(EfeError::InvalidConfig(format!(
                "FusionConfig: variance_warn_ratio {} must be finite and ≥ 1.0",
                self.variance_warn_ratio
            )));
        }
        Ok(())
    }

    /// Fluent builder-style setter.
    #[must_use]
    pub fn with_max_rounds(mut self, r: u32) -> Self {
        self.max_rounds = r;
        self
    }

    /// Fluent setter for `temperature`.
    #[must_use]
    pub fn with_temperature(mut self, t: f32) -> Self {
        self.temperature = t;
        self
    }

    /// Fluent setter for `mask_self_attention`.
    #[must_use]
    pub fn with_mask_self_attention(mut self, b: bool) -> Self {
        self.mask_self_attention = b;
        self
    }

    /// Fluent setter for `alignment_strength_init`.
    #[must_use]
    pub fn with_alignment_init(mut self, a: f32) -> Self {
        self.alignment_strength_init = a;
        self
    }

    /// Fluent setter for `problem_anchor_init`.
    #[must_use]
    pub fn with_problem_anchor_init(mut self, b: f32) -> Self {
        self.problem_anchor_init = b;
        self
    }

    /// Fluent setter for `clip_max`.
    #[must_use]
    pub fn with_clip_max(mut self, c: f32) -> Self {
        self.clip_max = c;
        self
    }

    /// Fluent setter for `enable_replay`.
    #[must_use]
    pub fn with_replay(mut self, enable: bool) -> Self {
        self.enable_replay = enable;
        self
    }

    /// Fluent setter for `round_deadline_ms`.
    #[must_use]
    pub fn with_round_deadline_ms(mut self, d: u32) -> Self {
        self.round_deadline_ms = d;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        FusionConfig::default().validate().unwrap();
    }

    #[test]
    fn rejects_zero_max_rounds() {
        let cfg = FusionConfig::default().with_max_rounds(0);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_nonfinite_temperature() {
        let cfg = FusionConfig::default().with_temperature(f32::NAN);
        assert!(cfg.validate().is_err());
        let cfg = FusionConfig::default().with_temperature(0.0);
        assert!(cfg.validate().is_err());
        let cfg = FusionConfig::default().with_temperature(-1.0);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_alignment_out_of_range() {
        let cfg = FusionConfig::default().with_alignment_init(1.5);
        assert!(cfg.validate().is_err());
        let cfg = FusionConfig::default().with_alignment_init(-0.1);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_floor_above_init() {
        let cfg = FusionConfig {
            alignment_strength_init: 0.1,
            alignment_strength_floor: 0.5,
            ..FusionConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_zero_clip_max() {
        let cfg = FusionConfig::default().with_clip_max(0.0);
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn accepts_explicit_large_but_finite_clip() {
        let cfg = FusionConfig::default().with_clip_max(1e20);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn replay_requires_nonzero_capacity() {
        let cfg = FusionConfig {
            enable_replay: true,
            replay_capacity: 0,
            ..FusionConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn serde_round_trip() {
        let cfg = FusionConfig::default();
        let s = serde_json::to_string(&cfg).unwrap();
        let back: FusionConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn rejects_variance_ratio_below_one() {
        let cfg = FusionConfig {
            variance_warn_ratio: 0.5,
            ..FusionConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_variance_ratio_nan() {
        let cfg = FusionConfig {
            variance_warn_ratio: f32::NAN,
            ..FusionConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn accepts_variance_ratio_infinity() {
        let cfg = FusionConfig {
            variance_warn_ratio: f32::INFINITY,
            ..FusionConfig::default()
        };
        assert!(cfg.validate().is_ok());
    }
}
