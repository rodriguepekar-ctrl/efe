//! `ConsensusConfig` + `SignalWeights` — tunable policy of M6.
//!
//! Every threshold, weight, and window size is exposed here; `default()`
//! is a sane starting point tuned on synthetic benchmarks, but operators
//! can override via TOML/JSON without recompiling.

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};

/// Weights applied to the 6 consensus signals.
///
/// Must sum to `1.0` (± 1e-4 tolerance); enforced by
/// [`SignalWeights::new`]. Used by [`crate::consensus::ConfidenceScorer`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignalWeights {
    /// S1 — variance level (converged when variance ≤ threshold).
    pub variance_level: f32,
    /// S2 — attention entropy drop (agents polarize).
    pub entropy_drop: f32,
    /// S3 — centroid stability (no drift).
    pub centroid_stability: f32,
    /// S4 — outlier ratio (cohesion).
    pub outlier_ratio: f32,
    /// S5 — variance trend (Theil-Sen slope).
    pub variance_trend: f32,
    /// S6 — absence of numerical anomaly.
    pub no_anomaly: f32,
}

impl SignalWeights {
    /// Construct custom weights. Sum must equal `1.0 ± 1e-4`.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if any weight is non-finite,
    ///   negative, or the sum is not close to `1.0`.
    pub fn new(
        variance_level: f32,
        entropy_drop: f32,
        centroid_stability: f32,
        outlier_ratio: f32,
        variance_trend: f32,
        no_anomaly: f32,
    ) -> Result<Self> {
        let s = Self {
            variance_level,
            entropy_drop,
            centroid_stability,
            outlier_ratio,
            variance_trend,
            no_anomaly,
        };
        s.validate()?;
        Ok(s)
    }

    /// MVP defaults — sum = 1.0 by construction.
    #[must_use]
    pub fn default_mvp() -> Self {
        Self {
            variance_level: 0.25,
            entropy_drop: 0.20,
            centroid_stability: 0.15,
            outlier_ratio: 0.10,
            variance_trend: 0.15,
            no_anomaly: 0.15,
        }
    }

    /// Sum of all weights.
    #[must_use]
    pub fn sum(&self) -> f32 {
        self.variance_level
            + self.entropy_drop
            + self.centroid_stability
            + self.outlier_ratio
            + self.variance_trend
            + self.no_anomaly
    }

    /// Return the weights as a fixed-size array in canonical order.
    #[must_use]
    pub fn to_array(&self) -> [f32; 6] {
        [
            self.variance_level,
            self.entropy_drop,
            self.centroid_stability,
            self.outlier_ratio,
            self.variance_trend,
            self.no_anomaly,
        ]
    }

    /// Validate invariants.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] on NaN / negative / non-unit-sum.
    pub fn validate(&self) -> Result<()> {
        for (name, w) in [
            ("variance_level", self.variance_level),
            ("entropy_drop", self.entropy_drop),
            ("centroid_stability", self.centroid_stability),
            ("outlier_ratio", self.outlier_ratio),
            ("variance_trend", self.variance_trend),
            ("no_anomaly", self.no_anomaly),
        ] {
            if !w.is_finite() || w < 0.0 {
                return Err(EfeError::InvalidConfig(format!(
                    "SignalWeights.{name} = {w} must be finite and ≥ 0"
                )));
            }
        }
        let s = self.sum();
        if !s.is_finite() || (s - 1.0).abs() > 1e-4 {
            return Err(EfeError::InvalidConfig(format!(
                "SignalWeights: sum {s} must be ≈ 1.0 (got |Δ| = {:.6})",
                (s - 1.0).abs()
            )));
        }
        Ok(())
    }
}

impl Default for SignalWeights {
    fn default() -> Self {
        Self::default_mvp()
    }
}

/// Tunable policy of the consensus detector.
///
/// Every field is exposed for external configuration (serde). See the
/// type-level docs of [`crate::consensus::ConsensusDetector`] for the
/// conceptual role of each parameter.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct ConsensusConfig {
    /// Window size for trend analysis. Must be ≥ 2.
    pub history_window: u32,
    /// Convergence threshold on the confidence score.
    pub confidence_threshold: f32,
    /// Consecutive rounds above threshold required for `Converged`.
    pub hysteresis_rounds: u32,
    /// σ²_conv — variance ≤ this is considered converged (signal S1).
    pub variance_threshold: f32,
    /// η_max ∈ (0, 1] — fraction of ln(N) below which entropy is "low" (S2).
    pub entropy_fraction_max: f32,
    /// δ_max — max relative centroid drift for S3=1.
    pub centroid_drift_max: f32,
    /// ρ_max — max outlier ratio for S4=1.
    pub outlier_ratio_max: f32,
    /// κ — trend gain for S5 sigmoid.
    pub trend_gain: f32,
    /// Absolute variance above which `Diverged` is declared.
    pub variance_explosion_threshold: f32,
    /// Number of rounds of flat low-confidence to trigger `Stalled`.
    pub stall_window: u32,
    /// Confidence range (max-min) below which "flat" for stall detection.
    pub stall_confidence_range: f32,
    /// Weights for the 6 signals.
    pub weights: SignalWeights,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            history_window: 6,
            confidence_threshold: 0.80,
            hysteresis_rounds: 2,
            variance_threshold: 0.01,
            entropy_fraction_max: 0.7,
            centroid_drift_max: 0.02,
            outlier_ratio_max: 0.05,
            trend_gain: 50.0,
            variance_explosion_threshold: 100.0,
            stall_window: 6,
            stall_confidence_range: 0.05,
            weights: SignalWeights::default_mvp(),
        }
    }
}

impl ConsensusConfig {
    /// Validate structural invariants.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] on any out-of-range parameter.
    pub fn validate(&self) -> Result<()> {
        if self.history_window < 2 {
            return Err(EfeError::InvalidConfig(
                "ConsensusConfig: history_window must be ≥ 2".into(),
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence_threshold) {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: confidence_threshold {} must be in [0, 1]",
                self.confidence_threshold
            )));
        }
        if self.hysteresis_rounds == 0 {
            return Err(EfeError::InvalidConfig(
                "ConsensusConfig: hysteresis_rounds must be ≥ 1".into(),
            ));
        }
        if !self.variance_threshold.is_finite() || self.variance_threshold <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: variance_threshold {} must be finite and > 0",
                self.variance_threshold
            )));
        }
        if !self.entropy_fraction_max.is_finite()
            || !(0.0..=1.0).contains(&self.entropy_fraction_max)
        {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: entropy_fraction_max {} must be finite and in (0, 1]",
                self.entropy_fraction_max
            )));
        }
        if !self.centroid_drift_max.is_finite() || self.centroid_drift_max <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: centroid_drift_max {} must be finite and > 0",
                self.centroid_drift_max
            )));
        }
        if !self.outlier_ratio_max.is_finite() || !(0.0..=1.0).contains(&self.outlier_ratio_max) {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: outlier_ratio_max {} must be finite and in (0, 1]",
                self.outlier_ratio_max
            )));
        }
        if !self.trend_gain.is_finite() || self.trend_gain <= 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: trend_gain {} must be finite and > 0",
                self.trend_gain
            )));
        }
        if !self.variance_explosion_threshold.is_finite()
            || self.variance_explosion_threshold <= 0.0
        {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: variance_explosion_threshold {} must be finite and > 0",
                self.variance_explosion_threshold
            )));
        }
        if self.stall_window < 2 {
            return Err(EfeError::InvalidConfig(
                "ConsensusConfig: stall_window must be ≥ 2".into(),
            ));
        }
        // `detect_stall` requires the history to contain at least
        // `stall_window` records. The history capacity is capped at
        // `history_window`, so the stall detector would *never fire*
        // if this invariant is violated. Better to reject than to
        // silently disable the stall detector.
        if self.stall_window > self.history_window {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: stall_window ({}) must be ≤ history_window ({}); \
                 otherwise the stall detector can never fire",
                self.stall_window, self.history_window
            )));
        }
        if !(0.0..=1.0).contains(&self.stall_confidence_range) {
            return Err(EfeError::InvalidConfig(format!(
                "ConsensusConfig: stall_confidence_range {} must be in [0, 1]",
                self.stall_confidence_range
            )));
        }
        self.weights.validate()?;
        Ok(())
    }

    /// Fluent setter for weights.
    #[must_use]
    pub fn with_weights(mut self, w: SignalWeights) -> Self {
        self.weights = w;
        self
    }

    /// Fluent setter for confidence threshold.
    #[must_use]
    pub fn with_confidence_threshold(mut self, t: f32) -> Self {
        self.confidence_threshold = t;
        self
    }

    /// Fluent setter for hysteresis rounds.
    #[must_use]
    pub fn with_hysteresis(mut self, k: u32) -> Self {
        self.hysteresis_rounds = k;
        self
    }

    /// Fluent setter for history window.
    #[must_use]
    pub fn with_window(mut self, w: u32) -> Self {
        self.history_window = w;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_weights_sum_to_one() {
        let w = SignalWeights::default_mvp();
        assert!((w.sum() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn weights_construction_validates_sum() {
        let err = SignalWeights::new(0.5, 0.5, 0.5, 0.0, 0.0, 0.0).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn weights_reject_negative() {
        let err = SignalWeights::new(1.0, -0.2, 0.0, 0.0, 0.0, 0.2).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn weights_reject_nan() {
        let err = SignalWeights::new(f32::NAN, 0.2, 0.2, 0.2, 0.2, 0.2).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn weights_to_array_matches_canonical_order() {
        let w = SignalWeights::default_mvp();
        let a = w.to_array();
        assert!((a[0] - w.variance_level).abs() < 1e-6);
        assert!((a[5] - w.no_anomaly).abs() < 1e-6);
    }

    #[test]
    fn config_default_validates() {
        ConsensusConfig::default().validate().unwrap();
    }

    #[test]
    fn config_rejects_zero_window() {
        let c = ConsensusConfig {
            history_window: 0,
            ..ConsensusConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_threshold_out_of_range() {
        let c = ConsensusConfig::default().with_confidence_threshold(1.5);
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_zero_hysteresis() {
        let c = ConsensusConfig::default().with_hysteresis(0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_nan_threshold() {
        let c = ConsensusConfig::default().with_confidence_threshold(f32::NAN);
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_non_positive_variance_threshold() {
        let c = ConsensusConfig {
            variance_threshold: 0.0,
            ..ConsensusConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_entropy_fraction_over_one() {
        let c = ConsensusConfig {
            entropy_fraction_max: 1.5,
            ..ConsensusConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_zero_trend_gain() {
        let c = ConsensusConfig {
            trend_gain: 0.0,
            ..ConsensusConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_rejects_short_stall_window() {
        let c = ConsensusConfig {
            stall_window: 1,
            ..ConsensusConfig::default()
        };
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_serde_round_trip() {
        let c = ConsensusConfig::default();
        let s = serde_json::to_string(&c).unwrap();
        let back: ConsensusConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn config_fluent_setters() {
        let c = ConsensusConfig::default()
            .with_confidence_threshold(0.9)
            .with_hysteresis(3)
            .with_window(10);
        assert!((c.confidence_threshold - 0.9).abs() < 1e-6);
        assert_eq!(c.hysteresis_rounds, 3);
        assert_eq!(c.history_window, 10);
        c.validate().unwrap();
    }

    #[test]
    fn config_rejects_stall_window_exceeding_history_window() {
        let c = ConsensusConfig {
            history_window: 4,
            stall_window: 10,
            ..ConsensusConfig::default()
        };
        let err = c.validate().unwrap_err();
        match err {
            EfeError::InvalidConfig(msg) => {
                assert!(
                    msg.contains("stall_window") && msg.contains("history_window"),
                    "message should mention both, got: {msg}"
                );
            }
            other => panic!("expected InvalidConfig, got: {other:?}"),
        }
    }

    #[test]
    fn config_accepts_stall_window_equal_to_history_window() {
        let c = ConsensusConfig {
            history_window: 6,
            stall_window: 6,
            ..ConsensusConfig::default()
        };
        c.validate().unwrap();
    }
}
