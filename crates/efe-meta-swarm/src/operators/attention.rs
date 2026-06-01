//! Attention-weighted meta-operator — gated behind the
//! `attention-weighted` feature for the reasons detailed in Section 6 of
//! the A5 paper (loss of global non-expansiveness under softmax).
//!
//! This module is a Phase-1 stub : the implementation is structurally
//! complete but **not** validated against the convergence theorem when
//! used in a meta-swarm.  Use with `domain-restricted` certification
//! only (cf. Theorem 6.15 of A5).

use crate::{MetaSwarmError, Result, Tensor};
use super::MetaOperator;

/// Attention-weighted aggregator
/// $\Psi(v_1, \ldots, v_M) = \sum_m \mathrm{softmax}(\alpha)_m \cdot v_m$
/// with `α_m = ⟨q, v_m⟩ / τ` for a fixed query vector `q` and temperature
/// `τ > 0`.
#[derive(Debug, Clone)]
pub struct AttentionMetaOp {
    name:        String,
    meta_dim:    usize,
    query:       Tensor,
    temperature: f32,
}

impl AttentionMetaOp {
    /// Build an attention aggregator.
    pub fn new(meta_dim: usize, query: Tensor, temperature: f32) -> Result<Self> {
        if query.dim() != meta_dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: meta_dim,
                actual:   query.dim(),
            });
        }
        if temperature <= 0.0 {
            return Err(MetaSwarmError::Internal(
                "AttentionMetaOp: temperature must be positive".to_string()
            ));
        }
        Ok(Self {
            name: format!("AttentionMetaOp(dim={meta_dim}, tau={temperature})"),
            meta_dim,
            query,
            temperature,
        })
    }

    fn softmax_weights(&self, votes: &[Tensor]) -> Result<Vec<f32>> {
        // Compute α_m = ⟨q, v_m⟩ / τ.
        let mut scores = Vec::with_capacity(votes.len());
        for v in votes {
            let s = self.query.dot(v)? as f32 / self.temperature;
            scores.push(s);
        }
        // Stabilised softmax : subtract max, exponentiate, normalise.
        let smax = scores.iter().fold(f32::NEG_INFINITY, |a, &b| a.max(b));
        let exp: Vec<f32> = scores.iter().map(|s| (s - smax).exp()).collect();
        let sum: f32 = exp.iter().sum();
        Ok(exp.iter().map(|e| e / sum).collect())
    }
}

impl MetaOperator for AttentionMetaOp {
    fn meta_dim(&self) -> usize { self.meta_dim }

    fn aggregate(&self, votes: &[Tensor]) -> Result<Tensor> {
        if votes.is_empty() {
            return Err(MetaSwarmError::EmptyMetaSwarm);
        }
        for v in votes {
            if v.dim() != self.meta_dim {
                return Err(MetaSwarmError::DimensionMismatch {
                    expected: self.meta_dim,
                    actual:   v.dim(),
                });
            }
        }
        let w = self.softmax_weights(votes)?;
        let mut acc = Tensor::zeros(self.meta_dim);
        for (v, &lambda) in votes.iter().zip(&w) {
            acc.add_scaled(lambda, v)?;
        }
        Ok(acc)
    }

    /// Worst-case bound under H1 + bounded query.  See Theorem 6.5 of A5.
    fn lipschitz_bound(&self) -> f64 {
        // K ≤ 1 + 2 B L_Λ / τ — for a query-based attention, L_Λ ≤ ‖q‖
        // and B is assumed bounded by 1 (normalised representations).
        let q_norm = self.query.norm();
        1.0 + 2.0 * q_norm / f64::from(self.temperature)
    }

    fn name(&self) -> &str { &self.name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_temperature_is_almost_uniform() {
        let q = Tensor::from_vec(vec![1.0, 0.0]);
        let op = AttentionMetaOp::new(2, q, 1000.0).unwrap();
        let v1 = Tensor::from_vec(vec![1.0, 0.0]);
        let v2 = Tensor::from_vec(vec![0.0, 1.0]);
        let r = op.aggregate(&[v1, v2]).unwrap();
        // ≈ (0.5, 0.5)
        assert!((r.at(0) - 0.5).abs() < 0.01);
        assert!((r.at(1) - 0.5).abs() < 0.01);
    }

    #[test]
    fn low_temperature_is_winner_take_all() {
        let q = Tensor::from_vec(vec![1.0, 0.0]);
        let op = AttentionMetaOp::new(2, q, 0.01).unwrap();
        let v1 = Tensor::from_vec(vec![10.0, 0.0]);   // matches q strongly
        let v2 = Tensor::from_vec(vec![-10.0, 1.0]);
        let r = op.aggregate(&[v1.clone(), v2]).unwrap();
        // Result should be ≈ v1.
        assert!((r.at(0) - 10.0).abs() < 0.1);
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 5 : AttentionMetaOp proptest (8) =
    // =========================================================================
    //
    // These tests probe the attention-softmax aggregator on the algebraic
    // properties one can prove without invoking the full Section 6 of A5 :
    //
    //   1. Output dim contract.
    //   2. High-temp limit  → uniform aggregator.
    //   3. Empty / dim-mismatch error paths.
    //   4. Identical votes  → identity (collapse to a single point).
    //   5. Lipschitz formula honours the closed-form bound.
    //   6. Lipschitz bound is monotone non-increasing in τ.
    //   7. Large vote magnitudes still produce finite outputs (no NaN/Inf,
    //      thanks to the stabilised softmax).
    //
    // We do *not* test global non-expansiveness here — Theorem 6.5 explicitly
    // shows the attention operator is **not** non-expansive in general ;
    // it is only domain-restricted-non-expansive, which is the topic of the
    // hierarchical safety tests (Cat 11).

    use proptest::prelude::*;

    fn pt_cfg_att() -> ProptestConfig {
        ProptestConfig { cases: 32, ..ProptestConfig::default() }
    }

    proptest! {
        #![proptest_config(pt_cfg_att())]

        /// Cat 5 / Test 1 — Output dim matches meta_dim.
        #[test]
        fn attention_output_dim_pt(
            dim in 1usize..=16,
            n in 1usize..=5,
            tau in 0.5_f32..50.0,
            seed_q in any::<u64>(),
            seed_v in any::<u64>(),
        ) {
            let q = Tensor::pseudo_random(dim, seed_q);
            let op = AttentionMetaOp::new(dim, q, tau).unwrap();
            let votes: Vec<Tensor> = (0..n)
                .map(|i| Tensor::pseudo_random(dim, seed_v.wrapping_add(i as u64)))
                .collect();
            let r = op.aggregate(&votes).unwrap();
            prop_assert_eq!(r.dim(), dim);
            prop_assert_eq!(op.meta_dim(), dim);
        }

        /// Cat 5 / Test 2 — High temperature → uniform aggregator (mean).
        ///
        /// As τ → ∞, softmax_τ(α) → 1/M, so Ψ_att(v) → (1/M)Σv = Ψ_uni(v).
        /// We pick τ very large relative to the dot-product magnitudes.
        #[test]
        fn attention_high_temp_near_uniform_pt(
            dim in 1usize..=8,
            n in 2usize..=4,
            seed_q in any::<u64>(),
            seed_v in any::<u64>(),
        ) {
            // Bounded query and votes ; with τ = 1e6 the softmax weights are
            // numerically uniform to 1e-5.
            let q = Tensor::pseudo_random(dim, seed_q);
            let op = AttentionMetaOp::new(dim, q, 1.0e6).unwrap();
            let votes: Vec<Tensor> = (0..n)
                .map(|i| Tensor::pseudo_random(dim, seed_v.wrapping_add(i as u64)))
                .collect();
            let r = op.aggregate(&votes).unwrap();
            // Manual uniform mean.
            let mut mean = Tensor::zeros(dim);
            for v in &votes { mean.add_assign(v).unwrap(); }
            mean.scale_in_place(1.0 / n as f32);
            prop_assert!(r.distance(&mean).unwrap() < 1e-3);
        }

        /// Cat 5 / Test 3 — Empty vote slice is rejected.
        #[test]
        fn attention_empty_rejected_pt(
            dim in 1usize..=16,
            tau in 0.1_f32..10.0,
            seed_q in any::<u64>(),
        ) {
            let q = Tensor::pseudo_random(dim, seed_q);
            let op = AttentionMetaOp::new(dim, q, tau).unwrap();
            let is_empty_err = matches!(
                op.aggregate(&[]),
                Err(MetaSwarmError::EmptyMetaSwarm)
            );
            prop_assert!(is_empty_err);
        }

        /// Cat 5 / Test 4 — Dim mismatch on a vote is rejected.
        #[test]
        fn attention_dim_mismatch_rejected_pt(
            dim in 2usize..=16,
            wrong in 1usize..=32,
            tau in 0.1_f32..10.0,
            seed_q in any::<u64>(),
            seed_v in any::<u64>(),
        ) {
            prop_assume!(wrong != dim);
            let q = Tensor::pseudo_random(dim, seed_q);
            let op = AttentionMetaOp::new(dim, q, tau).unwrap();
            let v = vec![Tensor::pseudo_random(wrong, seed_v)];
            let is_dim_err = matches!(
                op.aggregate(&v),
                Err(MetaSwarmError::DimensionMismatch { .. })
            );
            prop_assert!(is_dim_err);
        }

        /// Cat 5 / Test 5 — Identical votes : Ψ(v, v, …, v) = v.
        #[test]
        fn attention_identical_votes_pt(
            dim in 1usize..=16,
            n in 1usize..=5,
            tau in 0.1_f32..50.0,
            seed_q in any::<u64>(),
            seed_v in any::<u64>(),
        ) {
            let q = Tensor::pseudo_random(dim, seed_q);
            let v = Tensor::pseudo_random(dim, seed_v);
            let op = AttentionMetaOp::new(dim, q, tau).unwrap();
            let votes: Vec<Tensor> = (0..n).map(|_| v.clone()).collect();
            let r = op.aggregate(&votes).unwrap();
            prop_assert!(r.distance(&v).unwrap() < 1e-4);
        }

        /// Cat 5 / Test 6 — Lipschitz bound honours the closed-form formula
        /// `K = 1 + 2‖q‖ / τ`.
        #[test]
        fn attention_lipschitz_formula_pt(
            dim in 1usize..=16,
            tau in 0.1_f32..10.0,
            seed_q in any::<u64>(),
        ) {
            let q = Tensor::pseudo_random(dim, seed_q);
            let q_norm = q.norm();
            let op = AttentionMetaOp::new(dim, q, tau).unwrap();
            let expected = 1.0 + 2.0 * q_norm / f64::from(tau);
            prop_assert!((op.lipschitz_bound() - expected).abs() < 1e-9);
        }

        /// Cat 5 / Test 7 — Lipschitz bound is monotone non-increasing in τ
        /// (larger temperature ⇒ smaller bound, asymptotically → 1).
        #[test]
        fn attention_lipschitz_monotonic_in_tau_pt(
            dim in 1usize..=16,
            tau1 in 0.1_f32..1.0,
            tau2 in 1.0_f32..50.0,
            seed_q in any::<u64>(),
        ) {
            let q = Tensor::pseudo_random(dim, seed_q);
            let op1 = AttentionMetaOp::new(dim, q.clone(), tau1).unwrap();
            let op2 = AttentionMetaOp::new(dim, q, tau2).unwrap();
            prop_assert!(op1.lipschitz_bound() >= op2.lipschitz_bound() - 1e-12);
        }

        /// Cat 5 / Test 8 — Large vote magnitudes still produce finite
        /// outputs (stabilised softmax must not overflow).
        #[test]
        fn attention_large_votes_finite_pt(
            dim in 1usize..=8,
            scale in 10.0_f32..1.0e6,
            seed_q in any::<u64>(),
            seed_v in any::<u64>(),
        ) {
            let q = Tensor::pseudo_random(dim, seed_q);
            let op = AttentionMetaOp::new(dim, q, 1.0).unwrap();
            let votes: Vec<Tensor> = (0..3)
                .map(|i| Tensor::pseudo_random(dim, seed_v.wrapping_add(i as u64)).scale(scale))
                .collect();
            let r = op.aggregate(&votes).unwrap();
            for i in 0..dim {
                prop_assert!(r.at(i).is_finite());
            }
        }
    }
}
