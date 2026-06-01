//! `UniformMetaOp` — the simplest non-trivial meta-operator.
//!
//! $$ \Psi(v_1, \ldots, v_M) = \frac{1}{M} \sum_{m=1}^{M} v_m. $$
//!
//! By Lemma 3.1 of A5, this aggregator is non-expansive in the joint
//! $\ell^2$ norm.

use crate::{MetaSwarmError, Result, Tensor};
use super::MetaOperator;

/// Uniform-weighted aggregator.
///
/// Always produces $\frac{1}{M}\sum_m v_m$ where $M = \text{votes.len()}$.
#[derive(Debug, Clone)]
pub struct UniformMetaOp {
    name:     String,
    meta_dim: usize,
}

impl UniformMetaOp {
    /// Build a uniform meta-operator targeting a `meta_dim`-dimensional
    /// meta-space.
    #[must_use]
    pub fn new(meta_dim: usize) -> Self {
        Self { name: format!("UniformMetaOp(dim={meta_dim})"), meta_dim }
    }

    /// Build with an explicit name.
    pub fn with_name(name: impl Into<String>, meta_dim: usize) -> Self {
        Self { name: name.into(), meta_dim }
    }
}

impl MetaOperator for UniformMetaOp {
    fn meta_dim(&self) -> usize { self.meta_dim }

    fn aggregate(&self, votes: &[Tensor]) -> Result<Tensor> {
        if votes.is_empty() {
            return Err(MetaSwarmError::EmptyMetaSwarm);
        }
        // All votes must share meta_dim.
        for v in votes {
            if v.dim() != self.meta_dim {
                return Err(MetaSwarmError::DimensionMismatch {
                    expected: self.meta_dim,
                    actual:   v.dim(),
                });
            }
        }
        // Sum then scale.
        let mut acc = Tensor::zeros(self.meta_dim);
        for v in votes {
            acc.add_assign(v)?;
        }
        let inv = 1.0_f32 / (votes.len() as f32);
        acc.scale_in_place(inv);
        Ok(acc)
    }

    fn lipschitz_bound(&self) -> f64 { 1.0 }
    fn name(&self) -> &str { &self.name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_two_equal_votes() {
        let op = UniformMetaOp::new(3);
        let a = Tensor::from_vec(vec![1.0, 2.0, 3.0]);
        let b = Tensor::from_vec(vec![1.0, 2.0, 3.0]);
        let r = op.aggregate(&[a, b]).unwrap();
        assert_eq!(r.as_slice(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn aggregates_three_different_votes() {
        let op = UniformMetaOp::new(2);
        let a = Tensor::from_vec(vec![0.0, 0.0]);
        let b = Tensor::from_vec(vec![3.0, 3.0]);
        let c = Tensor::from_vec(vec![6.0, 6.0]);
        let r = op.aggregate(&[a, b, c]).unwrap();
        assert!((r.at(0) - 3.0).abs() < 1e-6);
        assert!((r.at(1) - 3.0).abs() < 1e-6);
    }

    #[test]
    fn empty_votes_is_an_error() {
        let op = UniformMetaOp::new(4);
        match op.aggregate(&[]) {
            Err(MetaSwarmError::EmptyMetaSwarm) => (),
            other => panic!("expected EmptyMetaSwarm, got {other:?}"),
        }
    }

    #[test]
    fn dim_mismatch_in_vote() {
        let op = UniformMetaOp::new(4);
        let v = vec![Tensor::zeros(3)];
        match op.aggregate(&v) {
            Err(MetaSwarmError::DimensionMismatch { expected: 4, actual: 3 }) => (),
            other => panic!("expected DimensionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn lipschitz_bound_is_one() {
        let op = UniformMetaOp::new(16);
        assert!((op.lipschitz_bound() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn aggregator_is_non_expansive_empirically() {
        // For two pairs of votes (a1, a2) and (b1, b2), the aggregator output
        // distance must be ≤ joint distance between the inputs.
        let op = UniformMetaOp::new(4);
        let a1 = Tensor::pseudo_random(4, 1);
        let a2 = Tensor::pseudo_random(4, 2);
        let b1 = Tensor::pseudo_random(4, 3);
        let b2 = Tensor::pseudo_random(4, 4);

        let psi_a = op.aggregate(&[a1.clone(), a2.clone()]).unwrap();
        let psi_b = op.aggregate(&[b1.clone(), b2.clone()]).unwrap();

        let out_dist = psi_a.distance(&psi_b).unwrap();
        let joint_dist = (a1.distance_squared(&b1).unwrap() + a2.distance_squared(&b2).unwrap()).sqrt();

        // ‖Ψ(a) - Ψ(b)‖ ≤ (1/√M) · ‖a - b‖ ≤ ‖a - b‖
        assert!(out_dist <= joint_dist + 1e-6);
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 4 : UniformMetaOp proptest (8) ===
    // =========================================================================

    use proptest::prelude::*;

    const TOL_UNI: f64 = 1e-4;

    fn pt_cfg_uni() -> ProptestConfig {
        ProptestConfig { cases: 32, ..ProptestConfig::default() }
    }

    proptest! {
        #![proptest_config(pt_cfg_uni())]

        /// Cat 4 / Test 1 — Aggregation equals component-wise mean.
        #[test]
        fn uniform_mean_pt(
            n in 2usize..=5,
            dim in 1usize..=16,
            seed in any::<u64>(),
        ) {
            let op = UniformMetaOp::new(dim);
            let votes: Vec<Tensor> = (0..n)
                .map(|i| Tensor::pseudo_random(dim, seed.wrapping_add(i as u64)))
                .collect();
            let agg = op.aggregate(&votes).unwrap();
            // Manual mean
            let mut sum = Tensor::zeros(dim);
            for v in &votes {
                sum.add_assign(v).unwrap();
            }
            sum.scale_in_place(1.0 / n as f32);
            prop_assert!(agg.distance(&sum).unwrap() < TOL_UNI);
        }

        /// Cat 4 / Test 2 — Permutation invariance.
        #[test]
        fn uniform_permutation_invariant_pt(
            dim in 1usize..=16,
            seed_a in any::<u64>(),
            seed_b in any::<u64>(),
            seed_c in any::<u64>(),
        ) {
            let op = UniformMetaOp::new(dim);
            let a = Tensor::pseudo_random(dim, seed_a);
            let b = Tensor::pseudo_random(dim, seed_b);
            let c = Tensor::pseudo_random(dim, seed_c);
            let r1 = op.aggregate(&[a.clone(), b.clone(), c.clone()]).unwrap();
            let r2 = op.aggregate(&[c, a, b]).unwrap();
            prop_assert!(r1.distance(&r2).unwrap() < TOL_UNI);
        }

        /// Cat 4 / Test 3 — Convex hull invariant : per-component result
        /// is between the per-component min and max of the votes.
        #[test]
        fn uniform_in_convex_hull_pt(
            dim in 1usize..=16,
            seed_a in any::<u64>(),
            seed_b in any::<u64>(),
            seed_c in any::<u64>(),
        ) {
            let op = UniformMetaOp::new(dim);
            let votes = vec![
                Tensor::pseudo_random(dim, seed_a),
                Tensor::pseudo_random(dim, seed_b),
                Tensor::pseudo_random(dim, seed_c),
            ];
            let r = op.aggregate(&votes).unwrap();
            for i in 0..dim {
                let mn = votes.iter().fold(f32::INFINITY, |a, v| a.min(v.at(i)));
                let mx = votes.iter().fold(f32::NEG_INFINITY, |a, v| a.max(v.at(i)));
                prop_assert!(r.at(i) >= mn - 1e-5);
                prop_assert!(r.at(i) <= mx + 1e-5);
            }
        }

        /// Cat 4 / Test 4 — Empty votes is rejected.
        #[test]
        fn uniform_empty_rejected_pt(dim in 1usize..=64) {
            let op = UniformMetaOp::new(dim);
            let is_empty_err = matches!(
                op.aggregate(&[]),
                Err(MetaSwarmError::EmptyMetaSwarm)
            );
            prop_assert!(is_empty_err);
        }

        /// Cat 4 / Test 5 — Dim mismatch rejected.
        #[test]
        fn uniform_dim_mismatch_rejected_pt(
            dim in 2usize..=16,
            wrong in 1usize..=64,
            seed in any::<u64>(),
        ) {
            prop_assume!(wrong != dim);
            let op = UniformMetaOp::new(dim);
            let v = vec![Tensor::pseudo_random(wrong, seed)];
            let is_dim_err = matches!(
                op.aggregate(&v),
                Err(MetaSwarmError::DimensionMismatch { .. })
            );
            prop_assert!(is_dim_err);
        }

        /// Cat 4 / Test 6 — Output dim matches meta_dim.
        #[test]
        fn uniform_output_dim_pt(
            dim in 1usize..=64,
            n in 1usize..=8,
            seed in any::<u64>(),
        ) {
            let op = UniformMetaOp::new(dim);
            let votes: Vec<Tensor> = (0..n)
                .map(|i| Tensor::pseudo_random(dim, seed.wrapping_add(i as u64)))
                .collect();
            let r = op.aggregate(&votes).unwrap();
            prop_assert_eq!(r.dim(), dim);
            prop_assert_eq!(op.meta_dim(), dim);
        }

        /// Cat 4 / Test 7 — Single-vote aggregation returns the vote.
        #[test]
        fn uniform_single_vote_identity_pt(
            dim in 1usize..=32,
            seed in any::<u64>(),
        ) {
            let op = UniformMetaOp::new(dim);
            let v = Tensor::pseudo_random(dim, seed);
            let r = op.aggregate(&[v.clone()]).unwrap();
            prop_assert!(r.distance(&v).unwrap() < TOL_UNI);
        }

        /// Cat 4 / Test 8 — Lipschitz bound is exactly 1.
        #[test]
        fn uniform_lipschitz_one_pt(dim in 1usize..=128) {
            let op = UniformMetaOp::new(dim);
            prop_assert!((op.lipschitz_bound() - 1.0).abs() < 1e-12);
        }
    }
}
