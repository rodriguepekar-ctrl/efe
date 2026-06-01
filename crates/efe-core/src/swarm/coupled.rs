//! Sprint **A1 — Phase 1** — primitives for the *coupled iteration* of
//! a hierarchical (meta-)swarm.
//!
//! This module exposes a single pure function, [`mix_coupled`], that
//! implements the Krasnosel'skii-Mann affine mix used by the meta-swarm :
//!
//! ```text
//!   x_{t+1} = (1 - α - γ) · x_t  +  α · Φ(x_t)  +  γ · pull
//! ```
//!
//! It is paired with [`SwarmController::step_with_pull`] (added in
//! `controller.rs`) which feeds the controller's own aggregator as
//! `Φ(x_t)`.
//!
//! ## Why this lives in `efe-core`
//!
//! The meta-swarm crate (`efe-meta-swarm`) sits *above* `efe-core` and
//! orchestrates several controllers together.  It needs a primitive
//! that asks each controller "compute one coupled step toward a pull
//! vector", and that primitive must reuse the controller's existing
//! aggregator without duplicating its internals.  Exposing
//! `mix_coupled` + `SwarmController::step_with_pull` is the minimal
//! additive surface required.
//!
//! ## Backward compatibility
//!
//! Both additions are strictly **additive** : no existing public type
//! or method is modified.  Callers that do not use the meta-swarm
//! observe no behavioural change.

use crate::errors::{EfeError, Result};

/// Affine mix used by the meta-swarm coupled iteration.
///
/// Computes, component-wise,
///
/// ```text
/// out_i = (1 - α - γ) · current_i  +  α · phi_i  +  γ · pull_i
/// ```
///
/// where all three input slices must share the same length.
///
/// # Mathematical context
///
/// This is the inner step of the projected Krasnosel'skii-Mann
/// iteration (Theorem 4.1 of the EFE A5 convergence paper, draft v0.1).
/// The three terms correspond to :
///
/// * **inertia** `(1 - α - γ) x` — keeps the iterate close to its
///   current position,
/// * **aggregation** `α Φ(x)` — pulls toward the swarm's own fixed
///   point,
/// * **coupling** `γ pull` — pulls toward the meta-consensus.
///
/// The convexity-preserving requirement `α + γ < 1` is enforced.
///
/// # Errors
///
/// Returns [`EfeError::InvalidConfig`] if the slices have mismatched
/// lengths or if `α + γ ≥ 1`, `α < 0`, or `γ < 0`.
///
/// # Examples
///
/// ```
/// use efe_core::swarm::coupled::mix_coupled;
/// let current = vec![1.0, 0.0];
/// let phi_x   = vec![0.0, 1.0];
/// let pull    = vec![1.0, 1.0];
/// let out = mix_coupled(&current, &phi_x, &pull, 0.4, 0.2).unwrap();
/// // (1 - 0.4 - 0.2) · current + 0.4 · phi_x + 0.2 · pull
/// // = 0.4 · [1, 0] + 0.4 · [0, 1] + 0.2 · [1, 1]
/// // = [0.4 + 0.0 + 0.2, 0.0 + 0.4 + 0.2] = [0.6, 0.6]
/// assert!((out[0] - 0.6).abs() < 1e-6);
/// assert!((out[1] - 0.6).abs() < 1e-6);
/// ```
pub fn mix_coupled(
    current: &[f32],
    phi_x: &[f32],
    pull: &[f32],
    alpha: f64,
    gamma: f64,
) -> Result<Vec<f32>> {
    if !alpha.is_finite() || !gamma.is_finite() {
        return Err(EfeError::InvalidConfig(format!(
            "mix_coupled: non-finite coefficients (alpha={alpha}, gamma={gamma})"
        )));
    }
    if alpha < 0.0 || gamma < 0.0 {
        return Err(EfeError::InvalidConfig(format!(
            "mix_coupled: coefficients must be non-negative (alpha={alpha}, gamma={gamma})"
        )));
    }
    if alpha + gamma >= 1.0 {
        return Err(EfeError::InvalidConfig(format!(
            "mix_coupled: alpha + gamma must be < 1 (got {alpha} + {gamma} = {})",
            alpha + gamma
        )));
    }
    let dim = current.len();
    if phi_x.len() != dim {
        return Err(EfeError::InvalidConfig(format!(
            "mix_coupled: phi_x length {} != current length {dim}",
            phi_x.len()
        )));
    }
    if pull.len() != dim {
        return Err(EfeError::InvalidConfig(format!(
            "mix_coupled: pull length {} != current length {dim}",
            pull.len()
        )));
    }
    #[allow(clippy::cast_possible_truncation)]
    let l1 = (1.0 - alpha - gamma) as f32;
    #[allow(clippy::cast_possible_truncation)]
    let l2 = alpha as f32;
    #[allow(clippy::cast_possible_truncation)]
    let l3 = gamma as f32;
    let mut out = Vec::with_capacity(dim);
    for i in 0..dim {
        out.push(l1 * current[i] + l2 * phi_x[i] + l3 * pull[i]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_when_alpha_and_gamma_are_zero() {
        let cur = vec![1.0, 2.0, 3.0];
        let phi = vec![10.0, 20.0, 30.0];
        let pul = vec![100.0, 200.0, 300.0];
        let out = mix_coupled(&cur, &phi, &pul, 0.0, 0.0).unwrap();
        assert!((out[0] - 1.0).abs() < 1e-6);
        assert!((out[1] - 2.0).abs() < 1e-6);
        assert!((out[2] - 3.0).abs() < 1e-6);
    }

    #[test]
    fn pure_aggregation_when_alpha_one_minus_epsilon() {
        let cur = vec![0.0];
        let phi = vec![10.0];
        let pul = vec![100.0];
        let out = mix_coupled(&cur, &phi, &pul, 0.9, 0.0).unwrap();
        // ~0.1 · 0 + 0.9 · 10 + 0 · 100 = 9.0
        assert!((out[0] - 9.0).abs() < 1e-5);
    }

    #[test]
    fn pure_pull_when_gamma_one_minus_epsilon() {
        let cur = vec![0.0];
        let phi = vec![10.0];
        let pul = vec![100.0];
        let out = mix_coupled(&cur, &phi, &pul, 0.0, 0.9).unwrap();
        // ~0.1 · 0 + 0 · 10 + 0.9 · 100 = 90.0
        assert!((out[0] - 90.0).abs() < 1e-4);
    }

    #[test]
    fn dim_mismatch_is_reported() {
        let cur = vec![1.0, 2.0];
        let phi = vec![1.0];
        let pul = vec![1.0, 2.0];
        match mix_coupled(&cur, &phi, &pul, 0.5, 0.2) {
            Err(EfeError::InvalidConfig(m)) => assert!(m.contains("phi_x length")),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn alpha_plus_gamma_geq_one_is_rejected() {
        let cur = vec![1.0];
        let phi = vec![1.0];
        let pul = vec![1.0];
        match mix_coupled(&cur, &phi, &pul, 0.6, 0.5) {
            Err(EfeError::InvalidConfig(_)) => (),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn negative_alpha_is_rejected() {
        let cur = vec![1.0];
        let phi = vec![1.0];
        let pul = vec![1.0];
        match mix_coupled(&cur, &phi, &pul, -0.1, 0.5) {
            Err(EfeError::InvalidConfig(_)) => (),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn non_finite_coefficient_is_rejected() {
        let cur = vec![1.0];
        let phi = vec![1.0];
        let pul = vec![1.0];
        match mix_coupled(&cur, &phi, &pul, f64::NAN, 0.0) {
            Err(EfeError::InvalidConfig(_)) => (),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    #[test]
    fn empty_slices_succeed_trivially() {
        let out = mix_coupled(&[], &[], &[], 0.5, 0.2).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn mixing_is_linear() {
        // ((1-α-γ) cur + α phi + γ pull) summed across i should equal
        // the linear combination of summed components — sanity check.
        let cur: Vec<f32> = (0..32).map(|i| i as f32).collect();
        let phi: Vec<f32> = (0..32).map(|i| -(i as f32)).collect();
        let pul: Vec<f32> = (0..32).map(|i| (i * 2) as f32).collect();
        let alpha = 0.3;
        let gamma = 0.2;
        let out = mix_coupled(&cur, &phi, &pul, alpha, gamma).unwrap();
        let sum_out: f32 = out.iter().sum();
        let sum_cur: f32 = cur.iter().sum();
        let sum_phi: f32 = phi.iter().sum();
        let sum_pul: f32 = pul.iter().sum();
        #[allow(clippy::cast_possible_truncation)]
        let expected =
            (1.0 - alpha - gamma) as f32 * sum_cur + alpha as f32 * sum_phi + gamma as f32 * sum_pul;
        assert!((sum_out - expected).abs() < 1e-3);
    }
}
