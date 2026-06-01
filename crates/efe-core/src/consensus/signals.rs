//! Six consensus signals `s_i(history) ∈ [0, 1]`.
//!
//! Each signal captures one dimension of convergence. The
//! [`ConfidenceScorer`](crate::consensus::ConfidenceScorer) combines
//! them into a single scalar.
//!
//! # Signals at a glance
//!
//! | Signal | Captures | Numerator | Denominator |
//! |---|---|---|---|
//! | S1 variance_level      | global dispersion | `variance` | `variance_threshold` |
//! | S2 entropy_drop        | attention polarization | `mean_attention_entropy` | `ln(N) · η_max` |
//! | S3 centroid_stability  | immobility | `\|μ_r - μ_{r-1}\|` | `\|μ_r\| · δ_max` |
//! | S4 outlier_ratio       | cohesion | `outlier_ratio` | `ρ_max` |
//! | S5 variance_trend      | decay | Theil-Sen slope | `σ(-slope · κ)` |
//! | S6 no_anomaly          | integrity | 1 | 1 (binary) |
//!
//! All signals return `0.0` when their inputs are non-finite — this is
//! a deliberate "failsafe" : a broken signal contributes zero confidence
//! rather than silently poisoning the score.

use crate::consensus::config::ConsensusConfig;
use crate::consensus::history::ConvergenceHistory;
use crate::fusion::FusionStats;

/// Raw signal values, 6-tuple in canonical order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    /// S1 — variance level ∈ `[0, 1]`, 1 means at / below threshold.
    pub s1_variance_level: f32,
    /// S2 — entropy drop ∈ `[0, 1]`, 1 means fully polarized.
    pub s2_entropy_drop: f32,
    /// S3 — centroid stability ∈ `[0, 1]`, 1 means immobile.
    pub s3_centroid_stability: f32,
    /// S4 — outlier cohesion ∈ `[0, 1]`, 1 means no outliers.
    pub s4_outlier_ratio: f32,
    /// S5 — variance trend ∈ `[0, 1]`, 0.5 = flat, 1 = decreasing.
    pub s5_variance_trend: f32,
    /// S6 — no anomaly, binary ∈ {0, 1}.
    pub s6_no_anomaly: f32,
}

impl Signals {
    /// All signal values as a fixed-size array in canonical order.
    #[must_use]
    pub fn to_array(&self) -> [f32; 6] {
        [
            self.s1_variance_level,
            self.s2_entropy_drop,
            self.s3_centroid_stability,
            self.s4_outlier_ratio,
            self.s5_variance_trend,
            self.s6_no_anomaly,
        ]
    }

    /// True iff every signal is finite (no NaN).
    #[must_use]
    pub fn all_finite(&self) -> bool {
        self.to_array().iter().all(|s| s.is_finite())
    }

    /// True iff every signal is in `[0, 1]`.
    #[must_use]
    pub fn all_in_unit_range(&self) -> bool {
        self.to_array().iter().all(|&s| (0.0..=1.0).contains(&s))
    }

    /// Neutral signals: `(0, 0, 0, 0, 0.5, 0)` — used as a default /
    /// placeholder when computation fails or history is empty.
    ///
    /// `s5 = 0.5` rather than `0.0` because S5 is a two-sided signal
    /// (decreasing variance pushes toward 1, increasing toward 0, so
    /// the "no-information" center is 0.5).
    #[must_use]
    pub fn neutral() -> Self {
        Self {
            s1_variance_level: 0.0,
            s2_entropy_drop: 0.0,
            s3_centroid_stability: 0.0,
            s4_outlier_ratio: 0.0,
            s5_variance_trend: 0.5,
            s6_no_anomaly: 0.0,
        }
    }
}

impl Default for Signals {
    /// Calls [`Signals::neutral`].
    fn default() -> Self {
        Self::neutral()
    }
}

/// Compute the 6 signals from the current history + latest stats.
///
/// The `history` is expected to already contain the latest stats as its
/// last record. `stats` is passed separately so the centroid can be
/// forwarded to S3 without needing it in the history itself.
#[must_use]
pub fn compute_signals(
    history: &ConvergenceHistory,
    cfg: &ConsensusConfig,
    stats: &FusionStats,
) -> Signals {
    Signals {
        s1_variance_level: s1_variance_level(stats, cfg),
        s2_entropy_drop: s2_entropy_drop(stats, cfg),
        s3_centroid_stability: s3_centroid_stability(history, cfg),
        s4_outlier_ratio: s4_outlier_ratio(stats, cfg),
        s5_variance_trend: s5_variance_trend(history, cfg),
        s6_no_anomaly: s6_no_anomaly(stats),
    }
}

/// S1 — `1 - min(1, variance / threshold)`.
///
/// Failsafe: returns `0.0` if `variance` is NaN/Inf or threshold is ≤ 0.
#[must_use]
pub fn s1_variance_level(stats: &FusionStats, cfg: &ConsensusConfig) -> f32 {
    if !stats.variance.is_finite() || cfg.variance_threshold <= 0.0 {
        return 0.0;
    }
    let ratio = stats.variance / cfg.variance_threshold;
    (1.0 - ratio).clamp(0.0, 1.0)
}

/// S2 — entropy drop, normalized to `ln(N_active)`.
///
/// `s2 = 1 - min(1, H / (ln(N) · η_max))`.
/// Failsafe: 0 on NaN entropy or `n_active == 0`.
#[must_use]
pub fn s2_entropy_drop(stats: &FusionStats, cfg: &ConsensusConfig) -> f32 {
    if stats.n_active == 0 || cfg.entropy_fraction_max <= 0.0 {
        return 0.0;
    }
    if !stats.mean_attention_entropy.is_finite() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let ln_n = (stats.n_active as f32).ln().max(1e-6);
    let denom = ln_n * cfg.entropy_fraction_max;
    if denom <= 0.0 {
        return 0.0;
    }
    let ratio = stats.mean_attention_entropy / denom;
    (1.0 - ratio).clamp(0.0, 1.0)
}

/// S3 — centroid stability between the last two rounds.
///
/// Returns `0.0` if there's no previous centroid yet (warming-up).
#[must_use]
pub fn s3_centroid_stability(history: &ConvergenceHistory, cfg: &ConsensusConfig) -> f32 {
    let (Some(curr), Some(prev)) = (history.last_centroid(), history.prev_centroid()) else {
        return 0.0;
    };
    if curr.len() != prev.len() || curr.is_empty() {
        return 0.0;
    }
    let mut drift_sq = 0.0f32;
    let mut norm_sq = 0.0f32;
    for (a, b) in curr.iter().zip(prev.iter()) {
        if !a.is_finite() || !b.is_finite() {
            return 0.0;
        }
        let d = a - b;
        drift_sq += d * d;
        norm_sq += a * a;
    }
    let norm = norm_sq.sqrt();
    if norm < f32::EPSILON {
        return 0.0;
    }
    let drift = drift_sq.sqrt();
    let ratio = drift / (norm * cfg.centroid_drift_max.max(1e-6));
    (1.0 - ratio).clamp(0.0, 1.0)
}

/// S4 — outlier ratio normalized to `ρ_max`.
#[must_use]
pub fn s4_outlier_ratio(stats: &FusionStats, cfg: &ConsensusConfig) -> f32 {
    if !stats.outlier_ratio.is_finite() || cfg.outlier_ratio_max <= 0.0 {
        return 0.0;
    }
    let ratio = stats.outlier_ratio / cfg.outlier_ratio_max;
    (1.0 - ratio).clamp(0.0, 1.0)
}

/// S5 — variance trend via Theil-Sen slope.
///
/// Returns `0.5` (neutral) when the history has fewer than 2 finite
/// variance points. Robust to spikes because it uses the **median** of
/// all pairwise slopes.
#[must_use]
pub fn s5_variance_trend(history: &ConvergenceHistory, cfg: &ConsensusConfig) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<(f32, f32)> = history
        .iter()
        .enumerate()
        .filter(|(_, r)| r.variance.is_finite())
        .map(|(i, r)| (i as f32, r.variance))
        .collect();
    if points.len() < 2 {
        return 0.5;
    }
    let mut slopes: Vec<f32> = Vec::with_capacity(points.len() * (points.len() - 1) / 2);
    for i in 0..points.len() {
        for j in (i + 1)..points.len() {
            let (xi, yi) = points[i];
            let (xj, yj) = points[j];
            let dx = xj - xi;
            if dx > 0.0 {
                slopes.push((yj - yi) / dx);
            }
        }
    }
    if slopes.is_empty() {
        return 0.5;
    }
    slopes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = slopes.len() / 2;
    let median_slope = slopes[mid];
    sigmoid(-median_slope * cfg.trend_gain)
}

/// S6 — binary absence of numerical anomaly.
#[must_use]
pub fn s6_no_anomaly(stats: &FusionStats) -> f32 {
    if stats.has_numerical_issues() {
        0.0
    } else {
        1.0
    }
}

/// Numerically stable sigmoid: `1 / (1 + exp(-x))`.
///
/// **Sprint 5 façade** — delegates to
/// [`crate::compute::ComputeBackend::sigmoid`]. The body retains the
/// scalar `f32 -> f32` signature (preserving every call site in this
/// module's scoring functions) but routes through the backend so that
/// future CUDA / SIMD optimisation propagates automatically. The
/// backend rejects non-finite inputs; this façade preserves the
/// historical "always returns a finite value" contract by short-circuiting
/// on NaN/Inf inputs to the appropriate limit.
#[inline]
#[must_use]
fn sigmoid(x: f32) -> f32 {
    // Defensive limits for NaN/Inf so the scalar contract is preserved.
    if x.is_nan() {
        return 0.5;
    }
    if x == f32::INFINITY {
        return 1.0;
    }
    if x == f32::NEG_INFINITY {
        return 0.0;
    }
    let backend = crate::compute::global_backend();
    let t = crate::compute::Tensor::<f32>::from_slice(&[x], [1].into())
        .expect("sigmoid: scalar tensor");
    backend
        .sigmoid(&t)
        .expect("sigmoid: backend must succeed on finite scalar")
        .as_slice()
        .expect("sigmoid: contiguous output slice")[0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::history::StatsRecord;
    use crate::fusion::FusionStats;

    fn mk_config() -> ConsensusConfig {
        ConsensusConfig::default()
    }

    fn mk_stats(
        variance: f32,
        entropy: f32,
        n_active: u32,
        outlier_ratio: f32,
        has_nan: bool,
    ) -> FusionStats {
        let mut s = FusionStats::synthetic_with(0, 4, variance, variance.sqrt().abs());
        s.mean_attention_entropy = entropy;
        s.outlier_ratio = outlier_ratio;
        s.n_active = n_active;
        s.has_nan = has_nan;
        s
    }

    // -------------------------------------------------------------
    // S1 tests
    // -------------------------------------------------------------

    #[test]
    fn s1_saturates_at_variance_zero() {
        let c = mk_config();
        let s = s1_variance_level(&mk_stats(0.0, 0.0, 1, 0.0, false), &c);
        assert!((s - 1.0).abs() < 1e-6);
    }

    #[test]
    fn s1_zero_at_threshold() {
        let c = mk_config();
        let s = s1_variance_level(&mk_stats(c.variance_threshold, 0.0, 1, 0.0, false), &c);
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s1_zero_above_threshold() {
        let c = mk_config();
        let s = s1_variance_level(
            &mk_stats(c.variance_threshold * 10.0, 0.0, 1, 0.0, false),
            &c,
        );
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s1_handles_nan_safely() {
        let c = mk_config();
        let s = s1_variance_level(&mk_stats(f32::NAN, 0.0, 1, 0.0, false), &c);
        assert!(s.abs() < 1e-6);
    }

    // -------------------------------------------------------------
    // S2 tests
    // -------------------------------------------------------------

    #[test]
    fn s2_zero_at_uniform_entropy() {
        let c = mk_config();
        // Uniform over N=8 → H = ln(8) ≈ 2.08 ; denom = ln(8) * 0.7 ≈ 1.45.
        // ratio > 1 → s2 = 0.
        let s = s2_entropy_drop(&mk_stats(0.0, (8f32).ln(), 8, 0.0, false), &c);
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s2_one_at_zero_entropy() {
        let c = mk_config();
        let s = s2_entropy_drop(&mk_stats(0.0, 0.0, 8, 0.0, false), &c);
        assert!((s - 1.0).abs() < 1e-6);
    }

    #[test]
    fn s2_zero_when_n_active_zero() {
        let c = mk_config();
        let s = s2_entropy_drop(&mk_stats(0.0, 0.0, 0, 0.0, false), &c);
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s2_nan_entropy_yields_zero() {
        let c = mk_config();
        let s = s2_entropy_drop(&mk_stats(0.0, f32::NAN, 4, 0.0, false), &c);
        assert!(s.abs() < 1e-6);
    }

    // -------------------------------------------------------------
    // S3 tests
    // -------------------------------------------------------------

    #[test]
    fn s3_zero_when_no_prev_centroid() {
        let h = ConvergenceHistory::new(5);
        let c = mk_config();
        let s = s3_centroid_stability(&h, &c);
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s3_one_on_identical_centroids() {
        let mut h = ConvergenceHistory::new(5);
        let centroid = vec![1.0f32, 2.0, 3.0];
        let stats = mk_stats(0.0, 0.0, 4, 0.0, false);
        h.push(StatsRecord::from_stats(&stats), &centroid);
        h.push(StatsRecord::from_stats(&stats), &centroid);
        let s = s3_centroid_stability(&h, &mk_config());
        assert!((s - 1.0).abs() < 1e-3);
    }

    #[test]
    fn s3_zero_on_large_drift() {
        let mut h = ConvergenceHistory::new(5);
        let c1 = vec![1.0f32, 0.0];
        let c2 = vec![5.0f32, 0.0];
        let stats = mk_stats(0.0, 0.0, 4, 0.0, false);
        h.push(StatsRecord::from_stats(&stats), &c1);
        h.push(StatsRecord::from_stats(&stats), &c2);
        let s = s3_centroid_stability(&h, &mk_config());
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s3_handles_zero_norm_safely() {
        let mut h = ConvergenceHistory::new(5);
        let c = vec![0.0f32, 0.0];
        let stats = mk_stats(0.0, 0.0, 4, 0.0, false);
        h.push(StatsRecord::from_stats(&stats), &c);
        h.push(StatsRecord::from_stats(&stats), &c);
        let s = s3_centroid_stability(&h, &mk_config());
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s3_zero_on_nan_centroid() {
        let mut h = ConvergenceHistory::new(5);
        let c1 = vec![1.0f32];
        let c2 = vec![f32::NAN];
        let stats = mk_stats(0.0, 0.0, 4, 0.0, false);
        h.push(StatsRecord::from_stats(&stats), &c1);
        h.push(StatsRecord::from_stats(&stats), &c2);
        let s = s3_centroid_stability(&h, &mk_config());
        assert!(s.abs() < 1e-6);
    }

    // -------------------------------------------------------------
    // S4 tests
    // -------------------------------------------------------------

    #[test]
    fn s4_one_at_zero_outliers() {
        let s = s4_outlier_ratio(&mk_stats(0.0, 0.0, 4, 0.0, false), &mk_config());
        assert!((s - 1.0).abs() < 1e-6);
    }

    #[test]
    fn s4_zero_at_threshold() {
        let c = mk_config();
        let s = s4_outlier_ratio(&mk_stats(0.0, 0.0, 4, c.outlier_ratio_max, false), &c);
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn s4_nan_ratio_yields_zero() {
        let s = s4_outlier_ratio(&mk_stats(0.0, 0.0, 4, f32::NAN, false), &mk_config());
        assert!(s.abs() < 1e-6);
    }

    // -------------------------------------------------------------
    // S5 tests (Theil-Sen)
    // -------------------------------------------------------------

    #[test]
    fn s5_neutral_on_empty_history() {
        let h = ConvergenceHistory::new(5);
        let s = s5_variance_trend(&h, &mk_config());
        assert!((s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn s5_neutral_on_single_point() {
        let mut h = ConvergenceHistory::new(5);
        h.push(
            StatsRecord::from_stats(&mk_stats(1.0, 0.0, 4, 0.0, false)),
            &[0.0f32],
        );
        let s = s5_variance_trend(&h, &mk_config());
        assert!((s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn s5_high_when_variance_decreasing() {
        let mut h = ConvergenceHistory::new(6);
        for (r, v) in [0.5, 0.3, 0.2, 0.1, 0.05].iter().enumerate() {
            let mut s = mk_stats(*v, 0.0, 4, 0.0, false);
            s.round = r as u32;
            h.push(StatsRecord::from_stats(&s), &[0.0f32]);
        }
        let s5 = s5_variance_trend(&h, &mk_config());
        assert!(
            s5 > 0.7,
            "expected s5 >> 0.5 for decreasing variance, got {s5}"
        );
    }

    #[test]
    fn s5_low_when_variance_increasing() {
        let mut h = ConvergenceHistory::new(6);
        for (r, v) in [0.05f32, 0.1, 0.2, 0.3, 0.5].iter().enumerate() {
            let mut s = mk_stats(*v, 0.0, 4, 0.0, false);
            s.round = r as u32;
            h.push(StatsRecord::from_stats(&s), &[0.0f32]);
        }
        let s5 = s5_variance_trend(&h, &mk_config());
        assert!(
            s5 < 0.3,
            "expected s5 << 0.5 for increasing variance, got {s5}"
        );
    }

    #[test]
    fn s5_robust_to_single_spike() {
        // 5 decreasing rounds with one spike in the middle — Theil-Sen
        // (median slope) should still report "decreasing".
        let mut h = ConvergenceHistory::new(6);
        for (r, v) in [0.5f32, 0.3, 5.0, 0.1, 0.05].iter().enumerate() {
            let mut s = mk_stats(*v, 0.0, 4, 0.0, false);
            s.round = r as u32;
            h.push(StatsRecord::from_stats(&s), &[0.0f32]);
        }
        let s5 = s5_variance_trend(&h, &mk_config());
        assert!(s5 > 0.5, "spike-resistant: got {s5}");
    }

    // -------------------------------------------------------------
    // S6 tests
    // -------------------------------------------------------------

    #[test]
    fn s6_one_on_clean_stats() {
        let s = s6_no_anomaly(&mk_stats(0.1, 0.5, 4, 0.01, false));
        assert!((s - 1.0).abs() < 1e-6);
    }

    #[test]
    fn s6_zero_on_nan() {
        let s = s6_no_anomaly(&mk_stats(0.1, 0.5, 4, 0.01, true));
        assert!(s.abs() < 1e-6);
    }

    // -------------------------------------------------------------
    // Signals struct tests
    // -------------------------------------------------------------

    #[test]
    fn signals_all_finite_true_by_default() {
        let s = Signals::neutral();
        assert!(s.all_finite());
    }

    #[test]
    fn signals_all_in_range_true_by_default() {
        let s = Signals::neutral();
        assert!(s.all_in_unit_range());
    }

    #[test]
    fn signals_to_array_canonical_order() {
        let s = Signals {
            s1_variance_level: 0.1,
            s2_entropy_drop: 0.2,
            s3_centroid_stability: 0.3,
            s4_outlier_ratio: 0.4,
            s5_variance_trend: 0.5,
            s6_no_anomaly: 0.6,
        };
        let a = s.to_array();
        assert!((a[0] - 0.1).abs() < 1e-6);
        assert!((a[5] - 0.6).abs() < 1e-6);
    }

    #[test]
    fn compute_signals_dispatches_correctly() {
        let mut h = ConvergenceHistory::new(5);
        let centroid = vec![0.5f32; 4];
        let stats = mk_stats(0.0, 0.0, 4, 0.0, false);
        h.push(StatsRecord::from_stats(&stats), &centroid);
        h.push(StatsRecord::from_stats(&stats), &centroid);
        let signals = compute_signals(&h, &mk_config(), &stats);
        assert!(signals.all_in_unit_range());
        assert!(signals.all_finite());
        // Clean stats with zero variance → s1=1, s6=1, s2=1, s4=1.
        assert!(signals.s1_variance_level > 0.99);
        assert!(signals.s6_no_anomaly > 0.99);
    }

    #[test]
    fn sigmoid_monotone() {
        assert!(sigmoid(-10.0) < sigmoid(0.0));
        assert!(sigmoid(0.0) < sigmoid(10.0));
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn sigmoid_bounded_in_unit_range() {
        for x in [-100.0f32, -10.0, 0.0, 10.0, 100.0] {
            let y = sigmoid(x);
            assert!((0.0..=1.0).contains(&y), "sigmoid({x}) = {y}");
        }
    }
}
