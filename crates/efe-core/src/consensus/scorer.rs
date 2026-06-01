//! `ConfidenceScorer` — weighted aggregation of the 6 signals.
//!
//! Kept as a separate type so it can be unit-tested in isolation and
//! swapped (e.g. by a neural scorer in Sprint 7+) without touching the
//! detector orchestration.

use crate::consensus::config::SignalWeights;
use crate::consensus::signals::Signals;

/// Weighted sum of the 6 signals. Output clamped to `[0, 1]`.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfidenceScorer {
    weights: SignalWeights,
}

impl ConfidenceScorer {
    /// Construct with explicit weights. The caller guarantees
    /// `weights.validate()` has succeeded.
    #[must_use]
    pub fn new(weights: SignalWeights) -> Self {
        Self { weights }
    }

    /// MVP defaults.
    #[must_use]
    pub fn default_mvp() -> Self {
        Self {
            weights: SignalWeights::default_mvp(),
        }
    }

    /// Access the weights.
    #[must_use]
    pub fn weights(&self) -> &SignalWeights {
        &self.weights
    }

    /// Compute the weighted confidence.
    ///
    /// Returns a value in `[0, 1]`. If any signal is non-finite, that
    /// term contributes `0` (the signal's failsafe semantics — see
    /// `signals.rs`).
    #[must_use]
    pub fn score(&self, s: &Signals) -> f32 {
        let arr = s.to_array();
        let w = self.weights.to_array();
        let mut total = 0.0f32;
        for (value, weight) in arr.iter().zip(w.iter()) {
            if value.is_finite() {
                total += value * weight;
            }
        }
        total.clamp(0.0, 1.0)
    }
}

impl Default for ConfidenceScorer {
    fn default() -> Self {
        Self::default_mvp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_signals(v: f32) -> Signals {
        Signals {
            s1_variance_level: v,
            s2_entropy_drop: v,
            s3_centroid_stability: v,
            s4_outlier_ratio: v,
            s5_variance_trend: v,
            s6_no_anomaly: v,
        }
    }

    #[test]
    fn score_all_zeros_is_zero() {
        let s = mk_signals(0.0);
        let score = ConfidenceScorer::default().score(&s);
        assert!(score.abs() < 1e-6);
    }

    #[test]
    fn score_all_ones_is_one() {
        let s = mk_signals(1.0);
        let score = ConfidenceScorer::default().score(&s);
        assert!((score - 1.0).abs() < 1e-4);
    }

    #[test]
    fn score_all_half_is_half() {
        let s = mk_signals(0.5);
        let score = ConfidenceScorer::default().score(&s);
        assert!((score - 0.5).abs() < 1e-4);
    }

    #[test]
    fn score_clamps_to_unit_range() {
        // Build a scorer with custom weights that could theoretically
        // overshoot (they won't, but we verify the clamp anyway).
        let weights = SignalWeights::default_mvp();
        let scorer = ConfidenceScorer::new(weights);
        // Any combination produces a value in [0, 1] since weights sum to 1
        // and values are assumed in [0, 1] — but nothing in the scorer
        // prevents pathological inputs. Test the clamp contract:
        let bad = Signals {
            s1_variance_level: 2.0, // out-of-range
            s2_entropy_drop: 2.0,
            s3_centroid_stability: 2.0,
            s4_outlier_ratio: 2.0,
            s5_variance_trend: 2.0,
            s6_no_anomaly: 2.0,
        };
        let score = scorer.score(&bad);
        assert!((0.0..=1.0).contains(&score));
    }

    #[test]
    fn score_ignores_nan_signals() {
        let mut s = mk_signals(1.0);
        s.s3_centroid_stability = f32::NAN;
        let score = ConfidenceScorer::default().score(&s);
        // Everyone is 1 except s3 which contributes 0 → total = 1 - w3 = 0.85.
        let expected = 1.0 - SignalWeights::default_mvp().centroid_stability;
        assert!(
            (score - expected).abs() < 1e-4,
            "got {score}, expected {expected}"
        );
    }

    #[test]
    fn score_is_deterministic() {
        let s = mk_signals(0.75);
        let scorer = ConfidenceScorer::default();
        let a = scorer.score(&s);
        let b = scorer.score(&s);
        assert!((a - b).abs() < f32::EPSILON);
    }

    #[test]
    fn score_respects_weights() {
        // Custom weights concentrate all mass on s1.
        let weights = SignalWeights::new(1.0, 0.0, 0.0, 0.0, 0.0, 0.0).unwrap();
        let scorer = ConfidenceScorer::new(weights);
        let s = Signals {
            s1_variance_level: 0.5,
            s2_entropy_drop: 0.0,
            s3_centroid_stability: 0.0,
            s4_outlier_ratio: 0.0,
            s5_variance_trend: 0.0,
            s6_no_anomaly: 0.0,
        };
        let score = scorer.score(&s);
        assert!((score - 0.5).abs() < 1e-5);
    }

    #[test]
    fn weights_accessor_returns_reference() {
        let scorer = ConfidenceScorer::default();
        let w = scorer.weights();
        assert!((w.sum() - 1.0).abs() < 1e-6);
    }
}
