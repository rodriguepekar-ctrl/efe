//! Pre-round and post-round invariant checks.
//!
//! Cheap sanity checks that run before and after kernel execution.
//! Deliberately conservative — the goal is to surface bugs or anomalies
//! early, not to re-validate every step of the math.

use crate::els::stats::ElsStats;
use crate::errors::{EfeError, Result};
use crate::fusion::config::FusionConfig;

/// Check the ELS is fit for fusion at round `round`.
///
/// Returns `Ok(())` iff:
///
/// 1. At least one agent is active.
/// 2. No anomaly (NaN/Inf) in the current state.
/// 3. Stats are finite.
///
/// # Errors
/// - [`EfeError::Internal`] if `n_active == 0`.
/// - [`EfeError::NumericalExplosion`] if stats carry NaN/Inf.
pub(crate) fn validate_pre_round(stats: &ElsStats, round: u32) -> Result<()> {
    if stats.n_active == 0 {
        return Err(EfeError::Internal("pre-round validator: no active agents"));
    }
    if stats.has_anomaly() || !stats.all_finite() {
        return Err(EfeError::NumericalExplosion { round });
    }
    Ok(())
}

/// Post-round check: ensure the round didn't destroy the ELS.
///
/// Deliberately accepts a rising variance — one-off spikes are normal.
/// The orchestrator tracks variance trend across rounds in M6.
///
/// # Errors
/// - [`EfeError::NumericalExplosion`] on post-round anomaly.
pub(crate) fn validate_post_round(stats: &ElsStats, round: u32, _cfg: &FusionConfig) -> Result<()> {
    if stats.has_anomaly() || !stats.all_finite() {
        return Err(EfeError::NumericalExplosion { round });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_stats(n_active: u32, has_nan: bool, variance: f32) -> ElsStats {
        ElsStats {
            centroid: vec![0.1; 4],
            variance,
            gradient_norm: variance.sqrt(),
            outlier_ratio: 0.0,
            has_nan,
            has_inf: false,
            n_active,
        }
    }

    #[test]
    fn pre_round_rejects_no_active() {
        let s = mk_stats(0, false, 0.1);
        let err = validate_pre_round(&s, 0).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn pre_round_rejects_nan() {
        let s = mk_stats(4, true, 0.1);
        let err = validate_pre_round(&s, 3).unwrap_err();
        assert!(matches!(err, EfeError::NumericalExplosion { round: 3 }));
    }

    #[test]
    fn pre_round_accepts_normal_state() {
        let s = mk_stats(4, false, 0.1);
        assert!(validate_pre_round(&s, 0).is_ok());
    }

    #[test]
    fn post_round_rejects_nan() {
        let s = mk_stats(4, true, 0.1);
        let err = validate_post_round(&s, 5, &FusionConfig::default()).unwrap_err();
        assert!(matches!(err, EfeError::NumericalExplosion { round: 5 }));
    }

    #[test]
    fn post_round_accepts_rising_variance() {
        // Post-round does NOT enforce monotonicity.
        let s = mk_stats(4, false, 1.0);
        assert!(validate_post_round(&s, 2, &FusionConfig::default()).is_ok());
    }
}
