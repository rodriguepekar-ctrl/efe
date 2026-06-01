//! Shared helpers used by multiple strategies and the engine.

use crate::collapse::config::CollapseConfig;
use crate::els::stats::ElsStats;

/// Heuristic estimate of the probability that subsequent fusion will
/// converge from the current state.
///
/// Returns `0.0` for non-finite stats; otherwise a linear combination
/// of :
/// - `variance_factor = 1 - min(1, variance / post_variance_max)`
/// - `outlier_factor = 1 - min(1, 5 * outlier_ratio)`
/// - `active_factor = min(1, n_active / 16)` (avoids over-shrunk swarms)
///
/// Weights `0.5 / 0.3 / 0.2` respectively. Sum of weights = 1 so the
/// result lives in `[0, 1]`. Sprint 7+ replaces this with a learned
/// regressor.
#[must_use]
pub(crate) fn estimate_recovery_confidence(stats: &ElsStats, cfg: &CollapseConfig) -> f32 {
    // Guard: `all_finite()` checks the scalars and centroid; we also
    // explicitly reject the `has_nan/has_inf` bookkeeping flags, which
    // may be set even when the cached scalars look fine (e.g. right
    // after a propagating M4 compute_stats).
    if !stats.all_finite() || stats.has_anomaly() {
        return 0.0;
    }
    let variance_factor = 1.0 - (stats.variance / cfg.post_variance_max).clamp(0.0, 1.0);
    let outlier_factor = 1.0 - (stats.outlier_ratio * 5.0).clamp(0.0, 1.0);
    #[allow(clippy::cast_precision_loss)]
    let active_factor = if stats.n_active >= 16 {
        1.0
    } else {
        (stats.n_active as f32) / 16.0
    };
    (variance_factor * 0.5 + outlier_factor * 0.3 + active_factor * 0.2).clamp(0.0, 1.0)
}

/// Deterministic seed derived from a string via FNV-1a 64-bit.
///
/// Used when `CollapseConfig::deterministic_seed = true` so CI runs
/// always exercise the same branches of the PRNG.
#[must_use]
pub(crate) fn fnv1a64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = FNV_OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_stats(variance: f32, n_active: u32, outlier_ratio: f32) -> ElsStats {
        ElsStats {
            centroid: vec![0.1; 4],
            variance,
            gradient_norm: variance.sqrt(),
            outlier_ratio,
            has_nan: false,
            has_inf: false,
            n_active,
        }
    }

    #[test]
    fn confidence_zero_on_nan() {
        let mut s = mk_stats(0.01, 32, 0.0);
        s.has_nan = true;
        let c = estimate_recovery_confidence(&s, &CollapseConfig::default());
        assert!(c.abs() < 1e-6);
    }

    #[test]
    fn confidence_high_on_clean_stats() {
        let s = mk_stats(0.001, 32, 0.0);
        let c = estimate_recovery_confidence(&s, &CollapseConfig::default());
        assert!(c > 0.9, "expected high confidence, got {c}");
    }

    #[test]
    fn confidence_drops_with_variance() {
        let cfg = CollapseConfig::default();
        let low = estimate_recovery_confidence(&mk_stats(0.0, 32, 0.0), &cfg);
        let high = estimate_recovery_confidence(&mk_stats(cfg.post_variance_max, 32, 0.0), &cfg);
        assert!(low > high);
    }

    #[test]
    fn confidence_drops_with_outliers() {
        let cfg = CollapseConfig::default();
        let none = estimate_recovery_confidence(&mk_stats(0.001, 32, 0.0), &cfg);
        let many = estimate_recovery_confidence(&mk_stats(0.001, 32, 0.5), &cfg);
        assert!(none > many);
    }

    #[test]
    fn confidence_penalizes_small_populations() {
        let cfg = CollapseConfig::default();
        let full = estimate_recovery_confidence(&mk_stats(0.001, 64, 0.0), &cfg);
        let small = estimate_recovery_confidence(&mk_stats(0.001, 4, 0.0), &cfg);
        assert!(full > small);
    }

    #[test]
    fn confidence_bounded_in_unit_range() {
        let cfg = CollapseConfig::default();
        for v in [0.0f32, 0.01, 1.0, 100.0, 1e9] {
            for n in [0u32, 1, 16, 64, 1024] {
                for o in [0.0f32, 0.1, 0.5, 1.0] {
                    let c = estimate_recovery_confidence(&mk_stats(v, n, o), &cfg);
                    assert!(
                        (0.0..=1.0).contains(&c),
                        "confidence out of range: {c} for v={v}, n={n}, o={o}"
                    );
                }
            }
        }
    }

    #[test]
    fn fnv1a_deterministic() {
        assert_eq!(fnv1a64(b"hello"), fnv1a64(b"hello"));
    }

    #[test]
    fn fnv1a_differs_on_different_inputs() {
        assert_ne!(fnv1a64(b"abc"), fnv1a64(b"abd"));
    }
}
