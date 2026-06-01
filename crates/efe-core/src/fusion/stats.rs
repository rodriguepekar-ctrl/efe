//! `FusionStats` — scalar statistics emitted by a fusion round.
//!
//! This struct is the M5 → M6 → M11 data interchange: it carries every
//! diagnostic signal the consensus detector and telemetry pipeline need.
//! It is `Clone + Debug + Serialize + Deserialize` (via serde) and free
//! of pointers or lifetimes, so it can be shipped across async/thread
//! boundaries without fuss.

use serde::{Deserialize, Serialize};

/// Scalar statistics emitted by a fusion round.
///
/// # Backward compatibility
///
/// Earlier sprints only filled `round`, `variance`, `gradient_norm`,
/// `centroid`, `outlier_ratio`, `has_nan`, `has_inf`, `confidence`. M5
/// introduces extra attention-entropy fields. Synthetic constructors
/// default these to neutral values (0.0) so existing callers keep
/// compiling.
///
/// # Field semantics
///
/// - `round`                       — 0-based round index.
/// - `variance`                    — `(1/N_active) Σ ||h_i - μ||²`.
/// - `gradient_norm`               — `sqrt(variance)` in MVP.
/// - `centroid`                    — mean over active agents, length = D.
/// - `outlier_ratio`               — fraction > 2σ from centroid. `NaN`
///                                    on numerical anomaly (see M4).
/// - `confidence`                  — filled by M6 on convergence.
/// - `has_nan` / `has_inf`         — anomaly flags.
/// - `mean_attention_entropy`      — avg row-entropy of softmax weights.
/// - `min_attention_entropy`       — minimum row-entropy (most focused).
/// - `max_attention_entropy`       — maximum row-entropy (most diffuse).
/// - `beta_applied`                — β used this round (problem anchor).
/// - `alpha_applied`               — α used this round (alignment).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FusionStats {
    /// Round index (0-based).
    pub round: u32,
    /// Variance across active nano-agent states.
    pub variance: f32,
    /// Gradient-norm proxy (`sqrt(variance)` in MVP).
    pub gradient_norm: f32,
    /// Current centroid on host, length = latent dim.
    pub centroid: Vec<f32>,
    /// Ratio of agents behaving as outliers (> 2σ).
    pub outlier_ratio: f32,
    /// True if any NaN was observed.
    pub has_nan: bool,
    /// True if any Inf was observed.
    pub has_inf: bool,
    /// Confidence produced at consensus — filled by M6.
    pub confidence: f32,
    /// Mean row-entropy of the attention weights (M5-only).
    pub mean_attention_entropy: f32,
    /// Minimum row-entropy (most focused row) (M5-only).
    pub min_attention_entropy: f32,
    /// Maximum row-entropy (most diffuse row) (M5-only).
    pub max_attention_entropy: f32,
    /// β coefficient applied to the problem anchor this round.
    pub beta_applied: f32,
    /// α coefficient applied to the alignment cone this round.
    pub alpha_applied: f32,
    /// Number of active agents at the end of the round.
    ///
    /// M6 uses this for the `ln(N)` bound on the entropy signal. Source
    /// of truth is [`crate::els::ElsStats::n_active`].
    pub n_active: u32,
}

impl FusionStats {
    /// True if the round produced non-finite scalars (NaN / Inf).
    ///
    /// The controller aborts (or rolls back via M7) when this is true.
    #[must_use]
    pub fn has_numerical_issues(&self) -> bool {
        self.has_nan || self.has_inf || !self.variance.is_finite()
    }

    /// Every diagnostic scalar is finite and within sane ranges.
    #[must_use]
    pub fn all_finite(&self) -> bool {
        self.variance.is_finite()
            && self.gradient_norm.is_finite()
            && self.outlier_ratio.is_finite()
            && self.confidence.is_finite()
            && self.mean_attention_entropy.is_finite()
            && self.min_attention_entropy.is_finite()
            && self.max_attention_entropy.is_finite()
            && self.beta_applied.is_finite()
            && self.alpha_applied.is_finite()
            && self.centroid.iter().all(|x| x.is_finite())
    }

    /// Clone this struct with a new confidence value (used on convergence).
    #[must_use]
    pub fn with_confidence(mut self, c: f32) -> Self {
        self.confidence = c;
        self
    }

    /// Fill the attention-entropy triplet (mean, min, max).
    pub fn set_attention_entropy(&mut self, mean: f32, min: f32, max: f32) {
        self.mean_attention_entropy = mean;
        self.min_attention_entropy = min;
        self.max_attention_entropy = max;
    }

    /// Stamp the α / β schedule values used by the round.
    pub fn set_schedule(&mut self, alpha: f32, beta: f32) {
        self.alpha_applied = alpha;
        self.beta_applied = beta;
    }

    /// Construct a synthetic "converging" stats for tests.
    #[must_use]
    #[doc(hidden)]
    pub fn synthetic_converging(round: u32, dim: usize) -> Self {
        Self {
            round,
            variance: 1e-4 / (round.max(1) as f32),
            gradient_norm: 1e-5,
            centroid: vec![0.1; dim],
            outlier_ratio: 0.0,
            has_nan: false,
            has_inf: false,
            confidence: 0.0,
            mean_attention_entropy: 0.0,
            min_attention_entropy: 0.0,
            max_attention_entropy: 0.0,
            beta_applied: 0.0,
            alpha_applied: 0.0,
            n_active: 0,
        }
    }

    /// Construct synthetic stats with explicit variance and gradient norm.
    ///
    /// Other fields default to sane, finite values. Hidden — tests only.
    #[must_use]
    #[doc(hidden)]
    pub fn synthetic_with(round: u32, dim: usize, variance: f32, gradient_norm: f32) -> Self {
        Self {
            round,
            variance,
            gradient_norm,
            centroid: vec![0.0; dim],
            outlier_ratio: 0.0,
            has_nan: false,
            has_inf: false,
            confidence: 0.0,
            mean_attention_entropy: 0.0,
            min_attention_entropy: 0.0,
            max_attention_entropy: 0.0,
            beta_applied: 0.0,
            alpha_applied: 0.0,
            n_active: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_numerical_issues_detects_nan() {
        let mut s = FusionStats::synthetic_converging(1, 4);
        assert!(!s.has_numerical_issues());
        s.has_nan = true;
        assert!(s.has_numerical_issues());
    }

    #[test]
    fn with_confidence_sets_value() {
        let s = FusionStats::synthetic_converging(1, 4).with_confidence(0.95);
        assert!((s.confidence - 0.95).abs() < 1e-6);
    }

    #[test]
    fn all_finite_true_on_clean_stats() {
        let s = FusionStats::synthetic_converging(1, 4);
        assert!(s.all_finite());
    }

    #[test]
    fn all_finite_false_on_nan_variance() {
        let mut s = FusionStats::synthetic_converging(1, 4);
        s.variance = f32::NAN;
        assert!(!s.all_finite());
    }

    #[test]
    fn set_attention_entropy_sets_all_three() {
        let mut s = FusionStats::synthetic_converging(1, 4);
        s.set_attention_entropy(0.5, 0.1, 0.9);
        assert!((s.mean_attention_entropy - 0.5).abs() < 1e-6);
        assert!((s.min_attention_entropy - 0.1).abs() < 1e-6);
        assert!((s.max_attention_entropy - 0.9).abs() < 1e-6);
    }

    #[test]
    fn set_schedule_sets_alpha_beta() {
        let mut s = FusionStats::synthetic_converging(1, 4);
        s.set_schedule(0.3, 0.2);
        assert!((s.alpha_applied - 0.3).abs() < 1e-6);
        assert!((s.beta_applied - 0.2).abs() < 1e-6);
    }

    #[test]
    fn json_round_trip() {
        let s = FusionStats::synthetic_with(3, 4, 0.1, 0.3);
        let json = serde_json::to_string(&s).unwrap();
        let back: FusionStats = serde_json::from_str(&json).unwrap();
        assert_eq!(s.round, back.round);
        assert!((s.variance - back.variance).abs() < f32::EPSILON);
    }
}
