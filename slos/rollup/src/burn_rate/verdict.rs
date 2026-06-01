//! PR #22 § 11 — 2-window verdict (AD38 + AD39).

use serde::{Deserialize, Serialize};

use crate::burn_rate::BurnRate;

/// Verdict of the 2-window burn-rate evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BurnRateVerdict {
    Healthy,
    SlowBurn,
    FastBurn,
}

impl BurnRateVerdict {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::SlowBurn => "slow_burn",
            Self::FastBurn => "fast_burn",
        }
    }
}

/// Evaluate the 2-window AND rule (AD39) :
///   * **FastBurn** if both (1h burn >= 14.4) AND (5m burn >= 14.4)
///   * **SlowBurn** if both (6h burn >= 6) AND (30m burn >= 6)
///   * **Healthy** otherwise
///
/// Fast takes precedence over slow (a fast-burn condition also satisfies slow).
/// Epsilon used for burn-rate threshold comparisons (AD9 — float
/// arithmetic at the SLI boundary can swing ±1e-12 due to (1-SLI)
/// subtraction, which would otherwise produce non-deterministic
/// verdicts at the 14.4× / 6× exact thresholds).
const EPS: f64 = 1e-9;

#[must_use]
pub fn evaluate_2window(
    burn_5m: BurnRate,
    burn_30m: BurnRate,
    burn_1h: BurnRate,
    burn_6h: BurnRate,
) -> BurnRateVerdict {
    let fast = burn_1h.multiplier + EPS >= crate::thresholds::BURN_RATE_FAST
        && burn_5m.multiplier + EPS >= crate::thresholds::BURN_RATE_FAST;
    if fast {
        return BurnRateVerdict::FastBurn;
    }
    let slow = burn_6h.multiplier + EPS >= crate::thresholds::BURN_RATE_SLOW
        && burn_30m.multiplier + EPS >= crate::thresholds::BURN_RATE_SLOW;
    if slow {
        return BurnRateVerdict::SlowBurn;
    }
    BurnRateVerdict::Healthy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn br(sli: f64) -> BurnRate {
        BurnRate::from_sli(sli, 0.995)
    }

    #[test]
    fn healthy_when_all_within_budget() {
        let v = evaluate_2window(br(1.0), br(1.0), br(1.0), br(1.0));
        assert_eq!(v, BurnRateVerdict::Healthy);
    }

    #[test]
    fn fast_burn_requires_both_5m_and_1h() {
        // 5m and 1h both at 14.4x → fast
        let v = evaluate_2window(br(0.928), br(0.97), br(0.928), br(0.97));
        assert_eq!(v, BurnRateVerdict::FastBurn);
    }

    #[test]
    fn fast_burn_does_not_fire_with_5m_only() {
        // 5m at 14.4x, but 1h at healthy → no fast (anti-flap AD39)
        let v = evaluate_2window(br(0.928), br(1.0), br(1.0), br(1.0));
        assert_eq!(v, BurnRateVerdict::Healthy);
    }

    #[test]
    fn fast_burn_does_not_fire_with_1h_only() {
        // 1h at 14.4x, but 5m healthy → no fast
        let v = evaluate_2window(br(1.0), br(1.0), br(0.928), br(1.0));
        assert_eq!(v, BurnRateVerdict::Healthy);
    }

    #[test]
    fn slow_burn_requires_both_30m_and_6h() {
        let v = evaluate_2window(br(1.0), br(0.97), br(1.0), br(0.97));
        assert_eq!(v, BurnRateVerdict::SlowBurn);
    }

    #[test]
    fn fast_takes_precedence_over_slow() {
        // All 4 windows hot → must report fast
        let v = evaluate_2window(br(0.928), br(0.928), br(0.928), br(0.928));
        assert_eq!(v, BurnRateVerdict::FastBurn);
    }

    #[test]
    fn verdict_strings_frozen() {
        assert_eq!(BurnRateVerdict::Healthy.as_str(), "healthy");
        assert_eq!(BurnRateVerdict::SlowBurn.as_str(), "slow_burn");
        assert_eq!(BurnRateVerdict::FastBurn.as_str(), "fast_burn");
    }
}
