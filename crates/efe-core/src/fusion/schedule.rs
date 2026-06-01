//! α / β schedule functions for the fusion campaign.
//!
//! - **β_r = β₀ · exp(-λ_β · r)** — exponential decay of the
//!   problem-anchor mixing weight. The swarm emancipates from the
//!   initial problem embedding as rounds progress.
//! - **α_r = max(floor, α₀ · (1 - r / R_max))** — linear decay of the
//!   alignment-cone projection strength, floored to a minimum.
//!
//! Both schedules are deterministic pure functions of `FusionConfig`
//! and the round index. They return values in `[0, 1]` or the
//! config-specified ranges, and never produce NaN.

use crate::fusion::config::FusionConfig;

/// Problem-anchor weight β for round `r`.
///
/// Returns `β₀ · e^(-λ · r)`, clamped to `[0, β₀]` so numerical
/// underflow in exp doesn't flip the sign nor produce NaN.
#[must_use]
pub fn beta_for_round(cfg: &FusionConfig, round: u32) -> f32 {
    if cfg.problem_anchor_init == 0.0 {
        return 0.0;
    }
    let r = round as f32;
    let decayed = cfg.problem_anchor_init * (-cfg.problem_anchor_decay * r).exp();
    decayed.clamp(0.0, cfg.problem_anchor_init)
}

/// Alignment-cone projection strength α for round `r`.
///
/// Linearly decays `α₀` toward `floor` as `r / max_rounds → 1`. Never
/// drops below `floor`.
#[must_use]
pub fn alpha_for_round(cfg: &FusionConfig, round: u32) -> f32 {
    if cfg.max_rounds == 0 {
        return cfg.alignment_strength_floor;
    }
    let r = round as f32;
    let rmax = cfg.max_rounds as f32;
    let decayed = cfg.alignment_strength_init * (1.0 - r / rmax);
    decayed.max(cfg.alignment_strength_floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> FusionConfig {
        FusionConfig::default()
    }

    #[test]
    fn beta_round_zero_equals_init() {
        let c = cfg();
        let b = beta_for_round(&c, 0);
        assert!((b - c.problem_anchor_init).abs() < 1e-6);
    }

    #[test]
    fn beta_decays_monotonically() {
        let c = cfg();
        let b0 = beta_for_round(&c, 0);
        let b5 = beta_for_round(&c, 5);
        let b10 = beta_for_round(&c, 10);
        assert!(b0 >= b5);
        assert!(b5 >= b10);
    }

    #[test]
    fn beta_in_unit_range() {
        let c = cfg();
        for r in 0..100u32 {
            let b = beta_for_round(&c, r);
            assert!((0.0..=1.0).contains(&b), "b={b} @ r={r}");
        }
    }

    #[test]
    fn beta_zero_if_init_zero() {
        let mut c = cfg();
        c.problem_anchor_init = 0.0;
        assert!(beta_for_round(&c, 0).abs() < f32::EPSILON);
        assert!(beta_for_round(&c, 5).abs() < f32::EPSILON);
    }

    #[test]
    fn alpha_round_zero_equals_init() {
        let c = cfg();
        let a = alpha_for_round(&c, 0);
        assert!((a - c.alignment_strength_init).abs() < 1e-6);
    }

    #[test]
    fn alpha_decays_linearly() {
        let c = cfg();
        let a0 = alpha_for_round(&c, 0);
        let a_half = alpha_for_round(&c, c.max_rounds / 2);
        let a_end = alpha_for_round(&c, c.max_rounds);
        assert!(a0 >= a_half);
        assert!(a_half >= a_end);
    }

    #[test]
    fn alpha_floors_at_configured_min() {
        let c = cfg();
        let a_past_end = alpha_for_round(&c, c.max_rounds * 10);
        assert!((a_past_end - c.alignment_strength_floor).abs() < 1e-6);
    }

    #[test]
    fn alpha_handles_zero_max_rounds() {
        let mut c = cfg();
        c.max_rounds = 0;
        let a = alpha_for_round(&c, 0);
        assert!((a - c.alignment_strength_floor).abs() < 1e-6);
    }

    #[test]
    fn both_schedules_finite_for_large_rounds() {
        let c = cfg();
        for r in [0u32, 10, 100, 1000, u32::MAX / 2] {
            assert!(beta_for_round(&c, r).is_finite());
            assert!(alpha_for_round(&c, r).is_finite());
        }
    }
}
