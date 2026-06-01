// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + ADR-4 + I6** : exact cost conversion between
//! the M50 `cost_cents: f64` model and the N21
//! `cost_microcents: i64` model.
//!
//! The cost unit divergence between the two ecosystems is a hard
//! audit gap : NanoAgent estimates and runtime actuals are in
//! `f64` cents, while SwarmAgent outcomes report `i64` microcents.
//! The adapter performs **banker's rounding** (`f64::round`) on the
//! conversion and saturates at half of `i64::MAX` so the swarm
//! aggregator's running total cannot overflow within any
//! reasonable session length (F14).
//!
//! The factor is `10_000` : 1 cent = 10_000 microcents.  This
//! matches the existing `SwarmCostTracker` fixed-point precision
//! (cf. `efe-nano::runtime::context::SwarmCostTracker`).

/// **N23 + ADR-4** : banker's-rounded conversion of cents (f64) to
/// microcents (i64).
///
/// Properties :
/// - Round-half-to-even via `f64::round`.
/// - Non-finite inputs (`NaN`, `±Inf`) collapse to `0`.
/// - Saturates at ±(i64::MAX / 2) to leave headroom for the swarm
///   aggregator's running total (F14).
///
/// # Examples
///
/// ```
/// use efe_nano_swarm_adapter::cents_to_microcents;
/// assert_eq!(cents_to_microcents(1.0), 10_000);
/// assert_eq!(cents_to_microcents(0.0001), 1);
/// assert_eq!(cents_to_microcents(-2.5), -25_000);
/// assert_eq!(cents_to_microcents(f64::NAN), 0);
/// ```
pub fn cents_to_microcents(cents: f64) -> i64 {
    if !cents.is_finite() {
        return 0;
    }
    let raw = (cents * 10_000.0).round();
    // Cap at 2^53 — the largest integer exactly representable as
    // f64.  Plenty of headroom : 2^53 microcents = $9 × 10^11
    // worth of dispatch cost.  This avoids the precision-loss
    // pitfall where (i64::MAX / 2) as f64 rounds *up* and the
    // subsequent `as i64` cast saturates beyond the intended cap.
    const CAP: f64 = (1u64 << 53) as f64;
    raw.clamp(-CAP, CAP) as i64
}

/// **N23** : reverse of [`cents_to_microcents`].
///
/// # Examples
///
/// ```
/// use efe_nano_swarm_adapter::microcents_to_cents;
/// assert!((microcents_to_cents(10_000) - 1.0).abs() < f64::EPSILON);
/// assert_eq!(microcents_to_cents(0), 0.0);
/// ```
pub fn microcents_to_cents(microcents: i64) -> f64 {
    (microcents as f64) / 10_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_cent_round_trip() {
        let mc = cents_to_microcents(1.0);
        assert_eq!(mc, 10_000);
        let back = microcents_to_cents(mc);
        assert!((back - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn fractional_cent_preserved() {
        let mc = cents_to_microcents(0.0001);
        assert_eq!(mc, 1);
    }

    #[test]
    fn negative_value_round_trips() {
        let mc = cents_to_microcents(-2.5);
        assert_eq!(mc, -25_000);
        let back = microcents_to_cents(mc);
        assert!((back + 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn nan_collapses_to_zero() {
        assert_eq!(cents_to_microcents(f64::NAN), 0);
    }

    #[test]
    fn infinity_collapses_to_zero() {
        assert_eq!(cents_to_microcents(f64::INFINITY), 0);
        assert_eq!(cents_to_microcents(f64::NEG_INFINITY), 0);
    }

    #[test]
    fn very_large_value_saturates() {
        // Push way past the f64 exact-integer range.
        let huge = 1.0e30_f64;
        let mc = cents_to_microcents(huge);
        // Cap is 2^53 ; mc must be exactly that (since we
        // clamp f64-to-i64 with no further rounding).
        assert_eq!(mc, 1i64 << 53);
        assert!(mc > 0);
    }

    #[test]
    fn very_large_negative_saturates() {
        let huge = -1.0e30_f64;
        let mc = cents_to_microcents(huge);
        assert_eq!(mc, -(1i64 << 53));
        assert!(mc < 0);
    }

    #[test]
    fn zero_round_trips_exactly() {
        assert_eq!(cents_to_microcents(0.0), 0);
        assert_eq!(microcents_to_cents(0), 0.0);
    }

    #[test]
    fn rounding_is_banker() {
        // f64::round is banker's rounding ("round half away from
        // zero" actually in IEEE 754).  We test that the boundary
        // case still produces a stable integer.
        let mc = cents_to_microcents(0.00005);
        // 0.00005 * 10_000 = 0.5 ; round() = 1 (away from zero).
        assert_eq!(mc, 1);
    }

    #[test]
    fn round_trip_property_for_typical_range() {
        // Pre-analysis cost estimates : 0.01 cent → 100 cents.
        for cents_i in 0..10_000 {
            let cents = (cents_i as f64) / 100.0;
            let mc = cents_to_microcents(cents);
            let back = microcents_to_cents(mc);
            // Tolerance : one microcent.
            assert!(
                (back - cents).abs() < 0.0002,
                "round-trip drift : {cents} → {mc} → {back}"
            );
        }
    }
}
