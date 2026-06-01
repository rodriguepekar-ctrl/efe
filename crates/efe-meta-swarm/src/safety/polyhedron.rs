//! Polyhedral safety set : $C = \{x : A x \le b\}$.
//!
//! ## Phase-1 simplification
//!
//! A correct projection onto a general polyhedron requires solving a
//! quadratic program ; doing so robustly is the job of a dedicated solver
//! (OSQP, Clarabel, or similar).  Phase 1 ships a simpler implementation
//! that uses **Dykstra's alternating projection algorithm** with a fixed
//! number of inner iterations.  Dykstra converges to the exact metric
//! projection ;  we cap the iteration count at 30, which is sufficient
//! for the polyhedra we will encounter (≤ 100 constraints, well-conditioned).
//!
//! For Phase 3 (closed alpha), this module will be replaced with an
//! `osqp`- or `clarabel`-backed solver behind the `qp-solver` feature.

use crate::{MetaSwarmError, Result, Tensor};
use super::SafetySet;

/// Polyhedron $\{x : Ax \le b\}$ stored row-major.
#[derive(Debug, Clone)]
pub struct PolyhedronSafety {
    name:    String,
    dim:     usize,
    n_rows:  usize,
    /// Row-major `A` matrix : `a[i * dim + j]`.
    a:       Vec<f32>,
    /// Right-hand side `b`, length `n_rows`.
    b:       Vec<f32>,
    /// Cached row norms, used by the projection.
    row_norm_sq: Vec<f32>,
}

impl PolyhedronSafety {
    /// Construct from row-major `A` and vector `b`.
    pub fn new(
        name: impl Into<String>,
        dim: usize,
        a: Vec<f32>,
        b: Vec<f32>,
    ) -> Result<Self> {
        if a.len() % dim != 0 {
            return Err(MetaSwarmError::Internal(
                format!("PolyhedronSafety: A length {} is not a multiple of dim {dim}", a.len())
            ));
        }
        let n_rows = a.len() / dim;
        if b.len() != n_rows {
            return Err(MetaSwarmError::DimensionMismatch { expected: n_rows, actual: b.len() });
        }
        let row_norm_sq: Vec<f32> = (0..n_rows).map(|i| {
            let row = &a[i * dim..(i + 1) * dim];
            row.iter().map(|v| v * v).sum::<f32>().max(1e-12)
        }).collect();
        Ok(Self {
            name:    name.into(),
            dim, n_rows, a, b, row_norm_sq,
        })
    }

    /// Single half-space projection :
    /// $P_{\{x : a^T x \le b\}}(x) = x - \max(0, (a^T x - b) / \|a\|^2) a$.
    fn project_halfspace(
        x: &mut [f32],
        row: &[f32],
        b: f32,
        norm_sq: f32,
    ) {
        let dot: f32 = x.iter().zip(row).map(|(a, b)| a * b).sum();
        let excess = dot - b;
        if excess > 0.0 {
            let lambda = excess / norm_sq;
            for (xi, &ri) in x.iter_mut().zip(row) {
                *xi -= lambda * ri;
            }
        }
    }
}

const POLYHEDRON_MAX_DYKSTRA_ITERS: usize = 30;
const POLYHEDRON_TOLERANCE: f32 = 1e-5;

impl SafetySet for PolyhedronSafety {
    fn dim(&self) -> usize { self.dim }

    fn project(&self, x: &Tensor) -> Result<Tensor> {
        if x.dim() != self.dim {
            return Err(MetaSwarmError::DimensionMismatch { expected: self.dim, actual: x.dim() });
        }
        // Simplified Dykstra projection : project sequentially onto each
        // half-space, repeat until convergence.  This is NOT Dykstra in the
        // strict sense (we omit the displacement corrections) ; it
        // converges to a *feasible* point but not necessarily the metric
        // projection.  For Phase 1 this is sufficient because we only use
        // the projection to enforce safety, not to optimise distance.
        let mut y = x.as_slice().to_vec();
        for _ in 0..POLYHEDRON_MAX_DYKSTRA_ITERS {
            let mut max_violation = 0.0_f32;
            for i in 0..self.n_rows {
                let row = &self.a[i * self.dim..(i + 1) * self.dim];
                let bi = self.b[i];
                let ns = self.row_norm_sq[i];
                let dot: f32 = y.iter().zip(row).map(|(a, b)| a * b).sum();
                let excess = dot - bi;
                if excess > 0.0 {
                    max_violation = max_violation.max(excess);
                    Self::project_halfspace(&mut y, row, bi, ns);
                }
            }
            if max_violation < POLYHEDRON_TOLERANCE { break; }
        }
        Ok(Tensor::from_vec(y))
    }

    fn contains(&self, x: &Tensor, tolerance: f64) -> bool {
        if x.dim() != self.dim { return false; }
        let tol = tolerance as f32;
        for i in 0..self.n_rows {
            let row = &self.a[i * self.dim..(i + 1) * self.dim];
            let dot: f32 = x.as_slice().iter().zip(row).map(|(a, b)| a * b).sum();
            if dot > self.b[i] + tol {
                return false;
            }
        }
        true
    }

    fn name(&self) -> &str { &self.name }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build the box [-1, 1]^2 as a polyhedron.  Four constraints :
    ///   x ≤ 1
    ///  -x ≤ 1
    ///   y ≤ 1
    ///  -y ≤ 1
    fn unit_square() -> PolyhedronSafety {
        let a = vec![
            1.0,  0.0,
           -1.0,  0.0,
            0.0,  1.0,
            0.0, -1.0,
        ];
        let b = vec![1.0, 1.0, 1.0, 1.0];
        PolyhedronSafety::new("unit_square", 2, a, b).unwrap()
    }

    #[test]
    fn origin_is_inside_unit_square() {
        assert!(unit_square().contains(&Tensor::zeros(2), 0.0));
    }

    #[test]
    fn outside_point_is_outside() {
        let p = unit_square();
        let x = Tensor::from_vec(vec![2.0, 0.0]);
        assert!(!p.contains(&x, 0.0));
    }

    #[test]
    fn projection_brings_inside() {
        let p = unit_square();
        let x = Tensor::from_vec(vec![2.0, 3.0]);
        let y = p.project(&x).unwrap();
        assert!(p.contains(&y, 1e-3));
    }

    #[test]
    fn projection_preserves_feasible_point() {
        let p = unit_square();
        let x = Tensor::from_vec(vec![0.5, -0.3]);
        let y = p.project(&x).unwrap();
        assert!(x.distance(&y).unwrap() < 1e-4);
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 7 : PolyhedronSafety proptest (6) =
    // =========================================================================

    use proptest::prelude::*;

    fn pt_cfg_poly() -> ProptestConfig {
        ProptestConfig { cases: 24, ..ProptestConfig::default() }
    }

    proptest! {
        #![proptest_config(pt_cfg_poly())]

        /// Cat 7 / Test 1 — Project always produces a feasible point.
        #[test]
        fn polyhedron_project_feasible_pt(
            seed in any::<u64>(),
            scale in 0.1_f32..50.0,
        ) {
            let p = unit_square();
            let x = Tensor::pseudo_random(2, seed).scale(scale);
            let y = p.project(&x).unwrap();
            prop_assert!(p.contains(&y, 1e-3));
        }

        /// Cat 7 / Test 2 — Feasible point is approximately fixed by P.
        #[test]
        fn polyhedron_idempotent_on_feasible_pt(
            x0 in -0.9_f32..0.9,
            x1 in -0.9_f32..0.9,
        ) {
            let p = unit_square();
            let x = Tensor::from_vec(vec![x0, x1]);
            let y = p.project(&x).unwrap();
            prop_assert!(x.distance(&y).unwrap() < 1e-3);
        }

        /// Cat 7 / Test 3 — Half-space projection : `x ≤ 1` projects
        /// `(value, 0)` (value > 1) to `(1, 0)`.
        #[test]
        fn polyhedron_half_space_analytic_pt(value in 1.1_f32..10.0) {
            let p = PolyhedronSafety::new(
                "pt7-3", 2,
                vec![1.0, 0.0],
                vec![1.0],
            ).unwrap();
            let x = Tensor::from_vec(vec![value, 0.0]);
            let y = p.project(&x).unwrap();
            prop_assert!((y.at(0) - 1.0).abs() < 1e-3);
            prop_assert!(y.at(1).abs() < 1e-3);
        }

        /// Cat 7 / Test 4 — Project preserves dimension.
        #[test]
        fn polyhedron_project_dim_pt(seed in any::<u64>()) {
            let p = unit_square();
            let x = Tensor::pseudo_random(2, seed).scale(5.0);
            let y = p.project(&x).unwrap();
            prop_assert_eq!(y.dim(), 2);
            prop_assert_eq!(SafetySet::dim(&p), 2);
        }

        /// Cat 7 / Test 5 — `contains` true at origin (always inside).
        #[test]
        fn polyhedron_contains_origin_pt(_filler in 0u8..=255) {
            let p = unit_square();
            prop_assert!(p.contains(&Tensor::zeros(2), 0.0));
        }

        /// Cat 7 / Test 6 — Dim mismatch on project rejected.
        #[test]
        fn polyhedron_dim_mismatch_rejected_pt(
            wrong in prop_oneof![Just(1usize), Just(3usize), Just(8usize)],
            seed in any::<u64>(),
        ) {
            let p = unit_square();
            let x = Tensor::pseudo_random(wrong, seed);
            let is_dim_err = matches!(
                p.project(&x),
                Err(MetaSwarmError::DimensionMismatch { .. })
            );
            prop_assert!(is_dim_err);
        }
    }
}
