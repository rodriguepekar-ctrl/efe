//! Axis-aligned box safety set : $C = \{x : l \le x \le u\}$.
//!
//! The metric projection is component-wise clipping :
//!
//! $$ P_C(x)_i = \max(l_i, \min(u_i, x_i)). $$

use crate::{MetaSwarmError, Result, Tensor};
use super::SafetySet;

/// Box $\{x : l \le x \le u\}$ — the simplest non-trivial safety set.
#[derive(Debug, Clone)]
pub struct BoxSafety {
    name:  String,
    lo:    Vec<f32>,
    hi:    Vec<f32>,
}

impl BoxSafety {
    /// Build from explicit lower and upper bounds.  Each must have the
    /// same length, which becomes the ambient dimension.
    pub fn new(lo: Vec<f32>, hi: Vec<f32>) -> Result<Self> {
        if lo.len() != hi.len() {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: lo.len(),
                actual:   hi.len(),
            });
        }
        for (i, (l, h)) in lo.iter().zip(&hi).enumerate() {
            if !(l <= h) {
                return Err(MetaSwarmError::Internal(format!(
                    "BoxSafety: lo[{i}]={l} > hi[{i}]={h}"
                )));
            }
        }
        let name = format!("BoxSafety(dim={})", lo.len());
        Ok(Self { name, lo, hi })
    }

    /// Symmetric box `[-r, r]^d`.
    #[must_use]
    pub fn symmetric(dim: usize, r: f32) -> Self {
        Self {
            name: format!("BoxSafety(symmetric, dim={dim}, r={r})"),
            lo:   vec![-r; dim],
            hi:   vec![ r; dim],
        }
    }

    /// Unit box $[-1, 1]^d$ — the most common default.
    #[must_use]
    pub fn unit_ball(dim: usize) -> Self {
        Self::symmetric(dim, 1.0)
    }
}

impl SafetySet for BoxSafety {
    fn dim(&self) -> usize { self.lo.len() }

    fn project(&self, x: &Tensor) -> Result<Tensor> {
        if x.dim() != self.lo.len() {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: self.lo.len(),
                actual:   x.dim(),
            });
        }
        let data = x.as_slice().iter().zip(&self.lo).zip(&self.hi)
            .map(|((v, l), h)| v.clamp(*l, *h))
            .collect();
        Ok(Tensor::from_vec(data))
    }

    fn contains(&self, x: &Tensor, tolerance: f64) -> bool {
        if x.dim() != self.lo.len() { return false; }
        let tol = tolerance as f32;
        x.as_slice().iter().zip(&self.lo).zip(&self.hi)
            .all(|((v, l), h)| *v >= *l - tol && *v <= *h + tol)
    }

    fn name(&self) -> &str { &self.name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_box_contains_origin() {
        let c = BoxSafety::unit_ball(8);
        assert!(c.contains(&Tensor::zeros(8), 0.0));
    }

    #[test]
    fn unit_box_does_not_contain_two() {
        let c = BoxSafety::unit_ball(2);
        let x = Tensor::from_vec(vec![2.0, 0.0]);
        assert!(!c.contains(&x, 0.0));
    }

    #[test]
    fn projection_clips() {
        let c = BoxSafety::unit_ball(3);
        let x = Tensor::from_vec(vec![-3.0, 0.5, 2.0]);
        let y = c.project(&x).unwrap();
        assert_eq!(y.as_slice(), &[-1.0, 0.5, 1.0]);
    }

    #[test]
    fn projection_is_idempotent_when_inside() {
        let c = BoxSafety::unit_ball(4);
        let x = Tensor::from_vec(vec![0.0, 0.5, -0.3, 0.7]);
        let y = c.project(&x).unwrap();
        assert!(x.distance(&y).unwrap() < 1e-9);
    }

    #[test]
    fn projection_lands_inside_set() {
        let c = BoxSafety::unit_ball(16);
        let x = Tensor::pseudo_random(16, 99).scale(10.0); // far outside
        let y = c.project(&x).unwrap();
        assert!(c.contains(&y, 1e-6));
    }

    #[test]
    fn projection_is_non_expansive() {
        // P_C is firmly non-expansive — empirically test ‖P(x)-P(y)‖ ≤ ‖x-y‖.
        let c = BoxSafety::unit_ball(8);
        let a = Tensor::pseudo_random(8, 1).scale(3.0);
        let b = Tensor::pseudo_random(8, 2).scale(3.0);
        let pa = c.project(&a).unwrap();
        let pb = c.project(&b).unwrap();
        assert!(pa.distance(&pb).unwrap() <= a.distance(&b).unwrap() + 1e-6);
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 6 : BoxSafety proptest (10 tests) =
    // =========================================================================

    use proptest::prelude::*;

    const TOL_BOX: f64 = 1e-5;

    fn pt_cfg_box() -> ProptestConfig {
        ProptestConfig { cases: 64, ..ProptestConfig::default() }
    }

    proptest! {
        #![proptest_config(pt_cfg_box())]

        /// Cat 6 / Test 1 — Project always lands inside C.
        #[test]
        fn box_project_inside_c_pt(
            dim in 1usize..=32,
            r in 0.1_f32..10.0,
            seed in any::<u64>(),
            scale in 0.1_f32..50.0,
        ) {
            let c = BoxSafety::symmetric(dim, r);
            let x = Tensor::pseudo_random(dim, seed).scale(scale);
            let y = c.project(&x).unwrap();
            prop_assert!(c.contains(&y, TOL_BOX));
        }

        /// Cat 6 / Test 2 — Projection is idempotent : P(P(x)) = P(x).
        #[test]
        fn box_project_idempotent_pt(
            dim in 1usize..=32,
            r in 0.1_f32..10.0,
            seed in any::<u64>(),
            scale in 0.1_f32..50.0,
        ) {
            let c = BoxSafety::symmetric(dim, r);
            let x = Tensor::pseudo_random(dim, seed).scale(scale);
            let p1 = c.project(&x).unwrap();
            let p2 = c.project(&p1).unwrap();
            prop_assert!(p1.distance(&p2).unwrap() < TOL_BOX);
        }

        /// Cat 6 / Test 3 — Points inside C are fixed by P.
        #[test]
        fn box_project_identity_when_inside_pt(
            dim in 1usize..=32,
            r in 0.5_f32..10.0,
            seed in any::<u64>(),
        ) {
            let c = BoxSafety::symmetric(dim, r);
            let x = Tensor::pseudo_random(dim, seed).scale(r * 0.5);
            let y = c.project(&x).unwrap();
            prop_assert!(x.distance(&y).unwrap() < TOL_BOX);
        }

        /// Cat 6 / Test 4 — Firmness empirical : projection is firmly
        /// non-expansive (Bauschke–Combettes Prop 4.16).
        #[test]
        fn box_project_firmly_non_expansive_pt(
            dim in 1usize..=16,
            r in 0.5_f32..5.0,
            seed_a in any::<u64>(),
            seed_b in any::<u64>(),
        ) {
            let c = BoxSafety::symmetric(dim, r);
            let a = Tensor::pseudo_random(dim, seed_a).scale(3.0);
            let b = Tensor::pseudo_random(dim, seed_b).scale(3.0);
            let pa = c.project(&a).unwrap();
            let pb = c.project(&b).unwrap();
            let res_a = a.sub(&pa).unwrap();
            let res_b = b.sub(&pb).unwrap();
            let lhs = pa.distance_squared(&pb).unwrap()
                    + res_a.distance_squared(&res_b).unwrap();
            let rhs = a.distance_squared(&b).unwrap();
            prop_assert!(lhs <= rhs + TOL_BOX,
                "firmness violation: {lhs} > {rhs}");
        }

        /// Cat 6 / Test 5 — Component-wise clipping is the projection.
        #[test]
        fn box_project_componentwise_clip_pt(
            dim in 1usize..=32,
            r in 0.1_f32..5.0,
            seed in any::<u64>(),
            scale in 0.1_f32..50.0,
        ) {
            let c = BoxSafety::symmetric(dim, r);
            let x = Tensor::pseudo_random(dim, seed).scale(scale);
            let y = c.project(&x).unwrap();
            for i in 0..dim {
                let expected = x.at(i).clamp(-r, r);
                prop_assert!((y.at(i) - expected).abs() < TOL_BOX as f32);
            }
        }

        /// Cat 6 / Test 6 — Degenerate box (lo = hi) collapses to a point.
        #[test]
        fn box_degenerate_collapses_pt(
            dim in 1usize..=16,
            value in -10.0_f32..10.0,
            seed in any::<u64>(),
        ) {
            let lo = vec![value; dim];
            let hi = vec![value; dim];
            let c = BoxSafety::new(lo, hi).unwrap();
            let x = Tensor::pseudo_random(dim, seed).scale(20.0);
            let y = c.project(&x).unwrap();
            for i in 0..dim {
                prop_assert!((y.at(i) - value).abs() < TOL_BOX as f32);
            }
        }

        /// Cat 6 / Test 7 — Asymmetric box is honoured.
        #[test]
        fn box_asymmetric_project_pt(
            seed in any::<u64>(),
            scale in 0.1_f32..50.0,
        ) {
            let lo = vec![-2.0_f32, 0.0, -1.0];
            let hi = vec![ 5.0_f32, 3.0,  1.0];
            let c = BoxSafety::new(lo, hi).unwrap();
            let x = Tensor::pseudo_random(3, seed).scale(scale);
            let y = c.project(&x).unwrap();
            prop_assert!(y.at(0) >= -2.0 - 1e-6 && y.at(0) <=  5.0 + 1e-6);
            prop_assert!(y.at(1) >=  0.0 - 1e-6 && y.at(1) <=  3.0 + 1e-6);
            prop_assert!(y.at(2) >= -1.0 - 1e-6 && y.at(2) <=  1.0 + 1e-6);
        }

        /// Cat 6 / Test 8 — `contains` is sound w.r.t. project.
        #[test]
        fn box_contains_after_project_pt(
            dim in 1usize..=32,
            r in 0.1_f32..10.0,
            seed in any::<u64>(),
            scale in 0.1_f32..50.0,
        ) {
            let c = BoxSafety::symmetric(dim, r);
            let x = Tensor::pseudo_random(dim, seed).scale(scale);
            let y = c.project(&x).unwrap();
            prop_assert!(c.contains(&y, TOL_BOX));
        }

        /// Cat 6 / Test 9 — Project of zero (when 0 ∈ C) is zero.
        #[test]
        fn box_project_zero_pt(dim in 1usize..=32, r in 0.1_f32..10.0) {
            let c = BoxSafety::symmetric(dim, r);
            let z = Tensor::zeros(dim);
            let pz = c.project(&z).unwrap();
            prop_assert!(z.distance(&pz).unwrap() < TOL_BOX);
        }

        /// Cat 6 / Test 10 — Project preserves dimension.
        #[test]
        fn box_project_preserves_dim_pt(
            dim in 1usize..=32,
            r in 0.1_f32..10.0,
            seed in any::<u64>(),
        ) {
            let c = BoxSafety::symmetric(dim, r);
            let x = Tensor::pseudo_random(dim, seed);
            let y = c.project(&x).unwrap();
            prop_assert_eq!(y.dim(), dim);
            prop_assert_eq!(SafetySet::dim(&c), dim);
        }
    }
}
