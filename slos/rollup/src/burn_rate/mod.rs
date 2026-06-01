//! PR #22 § 11 — Google-SRE 2-window burn-rate.

pub mod prom_rule;
pub mod verdict;

use serde::{Deserialize, Serialize};

use crate::thresholds::{BURN_RATE_FAST, BURN_RATE_SLOW};

/// One window's burn rate (how fast we're consuming the budget).
///
/// `burn = (1 - SLI) / (1 - target)`
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BurnRate {
    pub sli: f64,
    pub target: f64,
    pub multiplier: f64,
}

impl BurnRate {
    #[must_use]
    pub fn from_sli(sli: f64, target: f64) -> Self {
        let error_budget = 1.0 - target;
        let multiplier = if error_budget.abs() < f64::EPSILON {
            0.0
        } else {
            ((1.0 - sli) / error_budget).max(0.0)
        };
        Self { sli, target, multiplier }
    }

    /// Days to exhaust the monthly budget at this rate.
    #[must_use]
    pub fn exhaustion_days(self) -> f64 {
        if self.multiplier <= 0.0 {
            f64::INFINITY
        } else {
            30.0 / self.multiplier
        }
    }
}

pub use verdict::{BurnRateVerdict, evaluate_2window};

/// Both thresholds — public constants for testability + alert generation.
#[must_use]
pub const fn fast_threshold() -> f64 {
    BURN_RATE_FAST
}

#[must_use]
pub const fn slow_threshold() -> f64 {
    BURN_RATE_SLOW
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burn_at_target_is_unity() {
        // At SLI == target, burn = (1 - target) / (1 - target) = 1.0.
        // That's "consuming budget exactly at the sustainable rate".
        // It is NOT zero (perfect SLI 1.0 gives zero).
        let b = BurnRate::from_sli(0.995, 0.995);
        assert!((b.multiplier - 1.0).abs() < 1e-9);
        // 30d / 1× = 30 days to exhaustion.
        assert!((b.exhaustion_days() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn burn_zero_when_sli_is_perfect() {
        // At SLI 1.0, no budget consumed → burn = 0.
        let b = BurnRate::from_sli(1.0, 0.995);
        assert_eq!(b.multiplier, 0.0);
        assert!(b.exhaustion_days().is_infinite());
    }

    #[test]
    fn burn_14_4_exhausts_in_2_days() {
        // target 0.995 → error budget 0.005
        // burn = (1 - 0.928) / 0.005 = 14.4
        let b = BurnRate::from_sli(0.928, 0.995);
        assert!((b.multiplier - 14.4).abs() < 1e-9);
        // 30/14.4 ≈ 2.083 days
        assert!((b.exhaustion_days() - (30.0 / 14.4)).abs() < 1e-9);
    }

    #[test]
    fn burn_6_exhausts_in_5_days() {
        let b = BurnRate::from_sli(0.970, 0.995);
        assert!((b.multiplier - 6.0).abs() < 1e-9);
        assert!((b.exhaustion_days() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn burn_caps_below_zero() {
        // SLI above target → negative numerator → clamped to 0
        let b = BurnRate::from_sli(1.0, 0.995);
        assert_eq!(b.multiplier, 0.0);
    }

    #[test]
    fn thresholds_frozen() {
        assert!((fast_threshold() - 14.4).abs() < f64::EPSILON);
        assert!((slow_threshold() - 6.0).abs() < f64::EPSILON);
    }
}
