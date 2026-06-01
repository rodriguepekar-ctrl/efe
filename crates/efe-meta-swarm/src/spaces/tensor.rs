//! Dynamic-dimension tensor type with the linear-algebra primitives needed
//! by the coupled iteration.
//!
//! Internally a `Tensor` is a `Vec<f32>` paired with its declared
//! dimension.  Operations that change shape are explicit ; in-place
//! operations are provided to avoid allocator churn in hot loops.
//!
//! ## Numerical precision
//!
//! All arithmetic is `f32`.  In production EFE uses `bf16` ; the Phase 1
//! scaffold keeps `f32` because (a) `bf16` is a `efe-kernels` concern, and
//! (b) the convergence theorems of A5 carry over from `f32` to `bf16` with
//! a bounded perturbation argument (Remark 4.5 of A5).

use serde::{Deserialize, Serialize};

use crate::{MetaSwarmError, Result};

/// A real vector of fixed dimension living in a Hilbert space
/// $\mathcal{H} = \mathbb{R}^d$.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tensor {
    data: Vec<f32>,
    dim:  usize,
}

impl Tensor {
    // --------------------------------------------------------------------
    // Constructors
    // --------------------------------------------------------------------

    /// Build the zero tensor of dimension `dim`.
    #[must_use]
    pub fn zeros(dim: usize) -> Self {
        Self { data: vec![0.0; dim], dim }
    }

    /// Build a tensor filled with a constant value.
    #[must_use]
    pub fn fill(dim: usize, value: f32) -> Self {
        Self { data: vec![value; dim], dim }
    }

    /// Build from an owning `Vec`.  Length determines dimension.
    #[must_use]
    pub fn from_vec(data: Vec<f32>) -> Self {
        let dim = data.len();
        Self { data, dim }
    }

    /// Build from a slice (allocates).
    #[must_use]
    pub fn from_slice(data: &[f32]) -> Self {
        Self::from_vec(data.to_vec())
    }

    /// Build a tensor with values drawn from a deterministic Linear
    /// Congruential Generator seeded with `seed`.  Used for examples
    /// and tests to avoid an `rand` dependency at the crate level.
    /// Values are in `[-1.0, 1.0)`.
    #[must_use]
    pub fn pseudo_random(dim: usize, seed: u64) -> Self {
        let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let mut data = Vec::with_capacity(dim);
        for _ in 0..dim {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            // Map the high 24 bits into [-1.0, 1.0).
            let u = ((state >> 40) as f32) / (1u32 << 23) as f32 - 1.0;
            data.push(u);
        }
        Self { data, dim }
    }

    // --------------------------------------------------------------------
    // Accessors
    // --------------------------------------------------------------------

    /// Dimension of the tensor.
    #[must_use]
    pub fn dim(&self) -> usize { self.dim }

    /// Read-only view of the underlying data.
    #[must_use]
    pub fn as_slice(&self) -> &[f32] { &self.data }

    /// Mutable view of the underlying data.  Intended for hot-path
    /// in-place updates ;  do **not** resize the underlying `Vec`.
    pub fn as_mut_slice(&mut self) -> &mut [f32] { &mut self.data }

    /// Index a single component (panics on OOB — use [`Self::get`] for the
    /// fallible version).
    #[must_use]
    pub fn at(&self, i: usize) -> f32 { self.data[i] }

    /// Fallible component access.
    #[must_use]
    pub fn get(&self, i: usize) -> Option<f32> { self.data.get(i).copied() }

    // --------------------------------------------------------------------
    // Norms
    // --------------------------------------------------------------------

    /// Squared euclidean norm $\|x\|^2 = \sum_i x_i^2$.
    #[must_use]
    pub fn norm_squared(&self) -> f64 {
        self.data.iter().map(|&v| f64::from(v).powi(2)).sum()
    }

    /// Euclidean norm $\|x\| = \sqrt{\sum_i x_i^2}$.
    #[must_use]
    pub fn norm(&self) -> f64 { self.norm_squared().sqrt() }

    /// $\ell^\infty$ norm $\|x\|_\infty = \max_i |x_i|$.
    #[must_use]
    pub fn norm_infinity(&self) -> f64 {
        self.data.iter().fold(0.0, |acc, &v| acc.max(f64::from(v).abs()))
    }

    /// Inner product $\langle x, y \rangle$.  Requires matching dimensions.
    pub fn dot(&self, other: &Tensor) -> Result<f64> {
        self.check_same_dim(other)?;
        Ok(self.data.iter().zip(&other.data).map(|(&a, &b)| f64::from(a) * f64::from(b)).sum())
    }

    // --------------------------------------------------------------------
    // Algebraic ops (allocating)
    // --------------------------------------------------------------------

    /// Pure addition $x + y$.
    pub fn add(&self, other: &Tensor) -> Result<Tensor> {
        self.check_same_dim(other)?;
        let data = self.data.iter().zip(&other.data).map(|(a, b)| a + b).collect();
        Ok(Tensor { data, dim: self.dim })
    }

    /// Pure subtraction $x - y$.
    pub fn sub(&self, other: &Tensor) -> Result<Tensor> {
        self.check_same_dim(other)?;
        let data = self.data.iter().zip(&other.data).map(|(a, b)| a - b).collect();
        Ok(Tensor { data, dim: self.dim })
    }

    /// Scalar multiplication $\lambda \cdot x$.
    #[must_use]
    pub fn scale(&self, lambda: f32) -> Tensor {
        let data = self.data.iter().map(|v| lambda * v).collect();
        Tensor { data, dim: self.dim }
    }

    /// Fused affine combination $\lambda_1 x_1 + \lambda_2 x_2$.
    /// Requires the two operands to share dimensions.
    pub fn affine2(lambda1: f32, x1: &Tensor, lambda2: f32, x2: &Tensor) -> Result<Tensor> {
        x1.check_same_dim(x2)?;
        let data = x1.data.iter().zip(&x2.data)
            .map(|(a, b)| lambda1 * a + lambda2 * b)
            .collect();
        Ok(Tensor { data, dim: x1.dim })
    }

    /// Fused affine combination of three tensors :
    /// $\lambda_1 x_1 + \lambda_2 x_2 + \lambda_3 x_3$.
    ///
    /// This signature is hot-loop critical : it implements the inner kernel
    /// of the coupled iteration
    ///     `(1 - α - γ) x + α Φ(x) + γ pull`
    /// without intermediate allocations.
    pub fn affine3(
        lambda1: f32, x1: &Tensor,
        lambda2: f32, x2: &Tensor,
        lambda3: f32, x3: &Tensor,
    ) -> Result<Tensor> {
        x1.check_same_dim(x2)?;
        x1.check_same_dim(x3)?;
        let data = (0..x1.dim).map(|i|
            lambda1 * x1.data[i] + lambda2 * x2.data[i] + lambda3 * x3.data[i]
        ).collect();
        Ok(Tensor { data, dim: x1.dim })
    }

    /// In-place add : `self += other`.
    pub fn add_assign(&mut self, other: &Tensor) -> Result<()> {
        self.check_same_dim(other)?;
        for (a, b) in self.data.iter_mut().zip(&other.data) {
            *a += b;
        }
        Ok(())
    }

    /// In-place scaled add : `self += lambda * other`.
    pub fn add_scaled(&mut self, lambda: f32, other: &Tensor) -> Result<()> {
        self.check_same_dim(other)?;
        for (a, b) in self.data.iter_mut().zip(&other.data) {
            *a += lambda * b;
        }
        Ok(())
    }

    /// In-place scaling : `self *= lambda`.
    pub fn scale_in_place(&mut self, lambda: f32) {
        for a in &mut self.data {
            *a *= lambda;
        }
    }

    // --------------------------------------------------------------------
    // Distances
    // --------------------------------------------------------------------

    /// Squared distance between two tensors of equal dimension.
    pub fn distance_squared(&self, other: &Tensor) -> Result<f64> {
        self.check_same_dim(other)?;
        let mut acc = 0.0_f64;
        for (a, b) in self.data.iter().zip(&other.data) {
            let d = f64::from(*a) - f64::from(*b);
            acc += d * d;
        }
        Ok(acc)
    }

    /// Euclidean distance $\|x - y\|$.
    pub fn distance(&self, other: &Tensor) -> Result<f64> {
        Ok(self.distance_squared(other)?.sqrt())
    }

    // --------------------------------------------------------------------
    // Internal helpers
    // --------------------------------------------------------------------

    fn check_same_dim(&self, other: &Tensor) -> Result<()> {
        if self.dim != other.dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: self.dim,
                actual:   other.dim,
            });
        }
        Ok(())
    }
}

impl AsRef<[f32]> for Tensor {
    fn as_ref(&self) -> &[f32] { &self.data }
}

impl std::ops::Index<usize> for Tensor {
    type Output = f32;
    fn index(&self, i: usize) -> &f32 { &self.data[i] }
}

impl std::ops::IndexMut<usize> for Tensor {
    fn index_mut(&mut self, i: usize) -> &mut f32 { &mut self.data[i] }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeros_has_correct_dim_and_values() {
        let t = Tensor::zeros(5);
        assert_eq!(t.dim(), 5);
        assert!(t.as_slice().iter().all(|&v| v == 0.0));
    }

    #[test]
    fn from_vec_preserves_data() {
        let t = Tensor::from_vec(vec![1.0, 2.0, 3.0]);
        assert_eq!(t.dim(), 3);
        assert_eq!(t.as_slice(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn norm_zero_is_zero() {
        assert!((Tensor::zeros(10).norm() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn norm_unit_vector() {
        let t = Tensor::from_vec(vec![3.0, 4.0]);
        assert!((t.norm() - 5.0).abs() < 1e-6);
        assert!((t.norm_squared() - 25.0).abs() < 1e-6);
    }

    #[test]
    fn dot_product_orthogonal() {
        let a = Tensor::from_vec(vec![1.0, 0.0]);
        let b = Tensor::from_vec(vec![0.0, 1.0]);
        assert!((a.dot(&b).unwrap()).abs() < 1e-9);
    }

    #[test]
    fn dot_product_parallel() {
        let a = Tensor::from_vec(vec![1.0, 2.0, 3.0]);
        let dot = a.dot(&a).unwrap();
        assert!((dot - 14.0).abs() < 1e-6);
    }

    #[test]
    fn add_works() {
        let a = Tensor::from_vec(vec![1.0, 2.0]);
        let b = Tensor::from_vec(vec![3.0, 4.0]);
        let c = a.add(&b).unwrap();
        assert_eq!(c.as_slice(), &[4.0, 6.0]);
    }

    #[test]
    fn sub_works() {
        let a = Tensor::from_vec(vec![5.0, 7.0]);
        let b = Tensor::from_vec(vec![1.0, 2.0]);
        let c = a.sub(&b).unwrap();
        assert_eq!(c.as_slice(), &[4.0, 5.0]);
    }

    #[test]
    fn scale_works() {
        let a = Tensor::from_vec(vec![1.0, 2.0, 3.0]);
        let b = a.scale(2.0);
        assert_eq!(b.as_slice(), &[2.0, 4.0, 6.0]);
    }

    #[test]
    fn affine3_matches_manual_computation() {
        let a = Tensor::from_vec(vec![1.0, 1.0]);
        let b = Tensor::from_vec(vec![2.0, 2.0]);
        let c = Tensor::from_vec(vec![4.0, 4.0]);
        // 0.3 * 1 + 0.5 * 2 + 0.2 * 4 = 0.3 + 1.0 + 0.8 = 2.1
        let r = Tensor::affine3(0.3, &a, 0.5, &b, 0.2, &c).unwrap();
        assert!((r.at(0) - 2.1).abs() < 1e-6);
        assert!((r.at(1) - 2.1).abs() < 1e-6);
    }

    #[test]
    fn add_assign_in_place() {
        let mut a = Tensor::from_vec(vec![1.0, 2.0]);
        let b = Tensor::from_vec(vec![3.0, 4.0]);
        a.add_assign(&b).unwrap();
        assert_eq!(a.as_slice(), &[4.0, 6.0]);
    }

    #[test]
    fn distance_satisfies_triangle_inequality() {
        let a = Tensor::from_vec(vec![0.0, 0.0]);
        let b = Tensor::from_vec(vec![3.0, 0.0]);
        let c = Tensor::from_vec(vec![3.0, 4.0]);
        let ab = a.distance(&b).unwrap();
        let bc = b.distance(&c).unwrap();
        let ac = a.distance(&c).unwrap();
        assert!(ac <= ab + bc + 1e-6);
    }

    #[test]
    fn distance_to_self_is_zero() {
        let a = Tensor::pseudo_random(32, 42);
        assert!(a.distance(&a).unwrap() < 1e-9);
    }

    #[test]
    fn dim_mismatch_reports_correctly() {
        let a = Tensor::zeros(5);
        let b = Tensor::zeros(7);
        match a.add(&b) {
            Err(MetaSwarmError::DimensionMismatch { expected: 5, actual: 7 }) => (),
            other => panic!("expected DimensionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn pseudo_random_is_deterministic() {
        let a = Tensor::pseudo_random(16, 12345);
        let b = Tensor::pseudo_random(16, 12345);
        assert_eq!(a.as_slice(), b.as_slice());
    }

    // =========================================================================
    // ===  Sprint A1 P1 — PR #3 — Category 1 : Tensor proptest (15 tests)  ===
    // =========================================================================
    //
    // Property-based tests using `proptest`.  Each test runs `cases` random
    // inputs (default 64 per the local config) and verifies that the
    // mathematical invariant holds modulo a tolerance dominated by f32
    // rounding error.
    //
    // Convention :  the suffix `_pt` distinguishes proptest cases from the
    // concrete-value tests above.  Helpers are factored into shared
    // `prop_compose!` generators.

    use proptest::prelude::*;

    /// Tolerance for f32 arithmetic comparisons.  Sized for sums over
    /// dimensions up to 64 with values in [-1, 1] :  worst-case
    /// accumulator error ≈ √n · 2^-23 ≈ 1e-6 ;  we keep a generous
    /// 1e-4 to absorb compounded operations.
    const TOL: f64 = 1e-4;

    /// Local proptest configuration : 64 cases per test keeps the runtime
    /// budget under control (~1.5 s for the 15 tensor proptests on a
    /// modern dev box).  Persists adverse seeds in `proptest-regressions/`.
    fn pt_cfg() -> ProptestConfig {
        ProptestConfig {
            cases: 64,
            ..ProptestConfig::default()
        }
    }

    prop_compose! {
        /// One random tensor of arbitrary dim in [1, max_dim].
        fn bounded_tensor(max_dim: usize)(
            dim in 1usize..=max_dim,
            seed in any::<u64>(),
        ) -> Tensor {
            Tensor::pseudo_random(dim, seed)
        }
    }

    prop_compose! {
        /// Two tensors sharing the same random dimension.
        fn tensor_pair(max_dim: usize)(
            dim in 1usize..=max_dim,
            seed_a in any::<u64>(),
            seed_b in any::<u64>(),
        ) -> (Tensor, Tensor) {
            (Tensor::pseudo_random(dim, seed_a), Tensor::pseudo_random(dim, seed_b))
        }
    }

    prop_compose! {
        /// Three tensors sharing the same random dimension.
        fn tensor_triple(max_dim: usize)(
            dim in 1usize..=max_dim,
            seed_a in any::<u64>(),
            seed_b in any::<u64>(),
            seed_c in any::<u64>(),
        ) -> (Tensor, Tensor, Tensor) {
            (
                Tensor::pseudo_random(dim, seed_a),
                Tensor::pseudo_random(dim, seed_b),
                Tensor::pseudo_random(dim, seed_c),
            )
        }
    }

    proptest! {
        #![proptest_config(pt_cfg())]

        /// Cat 1 / Test 1 — A tensor filled with zeros has zero norm.
        #[test]
        fn norm_of_zero_tensor_is_zero_pt(dim in 1usize..=32) {
            let z = Tensor::zeros(dim);
            prop_assert!(z.norm() < TOL);
            let v = Tensor::from_vec(vec![0.0_f32; dim]);
            prop_assert!(v.norm() < TOL);
        }

        /// Cat 1 / Test 2 — Triangle inequality :  ‖x + y‖ ≤ ‖x‖ + ‖y‖.
        #[test]
        fn triangle_inequality_pt((x, y) in tensor_pair(32)) {
            let sum = x.add(&y).unwrap();
            let n_sum = sum.norm();
            let n_x = x.norm();
            let n_y = y.norm();
            prop_assert!(
                n_sum <= n_x + n_y + TOL,
                "‖x + y‖ ({n_sum}) > ‖x‖ + ‖y‖ ({} + {} = {})",
                n_x, n_y, n_x + n_y
            );
        }

        /// Cat 1 / Test 3 — Cauchy-Schwarz :  |⟨x, y⟩| ≤ ‖x‖ · ‖y‖.
        #[test]
        fn cauchy_schwarz_pt((x, y) in tensor_pair(32)) {
            let dot = x.dot(&y).unwrap().abs();
            let bound = x.norm() * y.norm();
            // Use relative + absolute tolerance to accommodate large norms.
            prop_assert!(
                dot <= bound + TOL.max(bound * 1e-3),
                "|⟨x, y⟩| ({dot}) > ‖x‖·‖y‖ ({bound})"
            );
        }

        /// Cat 1 / Test 4 — Norm scale-equivariance :  ‖λx‖ = |λ| · ‖x‖.
        #[test]
        fn norm_scale_equivariance_pt(
            x in bounded_tensor(32),
            lambda in -10.0_f32..10.0,
        ) {
            let scaled = x.scale(lambda);
            let lhs = scaled.norm();
            let rhs = f64::from(lambda.abs()) * x.norm();
            // Relative tolerance for large lambda · ‖x‖.
            prop_assert!(
                (lhs - rhs).abs() <= TOL.max(rhs * 1e-3),
                "‖λx‖ ({lhs}) != |λ|·‖x‖ ({rhs}), λ={lambda}"
            );
        }

        /// Cat 1 / Test 5 — Distance is symmetric :  d(x, y) = d(y, x).
        #[test]
        fn distance_symmetric_pt((x, y) in tensor_pair(32)) {
            let dxy = x.distance(&y).unwrap();
            let dyx = y.distance(&x).unwrap();
            prop_assert!((dxy - dyx).abs() < TOL);
        }

        /// Cat 1 / Test 6 — Distance is non-negative.
        #[test]
        fn distance_non_negative_pt((x, y) in tensor_pair(32)) {
            prop_assert!(x.distance(&y).unwrap() >= 0.0);
        }

        /// Cat 1 / Test 7 — Self-distance is zero.
        #[test]
        fn self_distance_is_zero_pt(x in bounded_tensor(32)) {
            prop_assert!(x.distance(&x).unwrap().abs() < TOL);
        }

        /// Cat 1 / Test 8 — Inner product bilinear in the first argument :
        /// ⟨αx + βy, z⟩ = α⟨x, z⟩ + β⟨y, z⟩.
        #[test]
        fn inner_product_bilinear_left_pt(
            (x, y, z) in tensor_triple(16),
            alpha in -2.0_f32..2.0,
            beta  in -2.0_f32..2.0,
        ) {
            let combined = Tensor::affine2(alpha, &x, beta, &y).unwrap();
            let lhs = combined.dot(&z).unwrap();
            let rhs = f64::from(alpha) * x.dot(&z).unwrap()
                    + f64::from(beta)  * y.dot(&z).unwrap();
            let mag = (lhs.abs() + rhs.abs()).max(1.0);
            prop_assert!((lhs - rhs).abs() <= TOL * mag, "lhs={lhs}, rhs={rhs}");
        }

        /// Cat 1 / Test 9 — Inner product bilinear in the second argument.
        #[test]
        fn inner_product_bilinear_right_pt(
            (x, y, z) in tensor_triple(16),
            alpha in -2.0_f32..2.0,
            beta  in -2.0_f32..2.0,
        ) {
            let combined = Tensor::affine2(alpha, &y, beta, &z).unwrap();
            let lhs = x.dot(&combined).unwrap();
            let rhs = f64::from(alpha) * x.dot(&y).unwrap()
                    + f64::from(beta)  * x.dot(&z).unwrap();
            let mag = (lhs.abs() + rhs.abs()).max(1.0);
            prop_assert!((lhs - rhs).abs() <= TOL * mag, "lhs={lhs}, rhs={rhs}");
        }

        /// Cat 1 / Test 10 — Inner product is symmetric :  ⟨x, y⟩ = ⟨y, x⟩.
        #[test]
        fn inner_product_symmetric_pt((x, y) in tensor_pair(32)) {
            let xy = x.dot(&y).unwrap();
            let yx = y.dot(&x).unwrap();
            prop_assert!((xy - yx).abs() < TOL);
        }

        /// Cat 1 / Test 11 — `affine2` matches the manual `scale + add` form.
        #[test]
        fn affine2_matches_scale_add_pt(
            (x, y) in tensor_pair(32),
            alpha in -2.0_f32..2.0,
            beta  in -2.0_f32..2.0,
        ) {
            let direct = Tensor::affine2(alpha, &x, beta, &y).unwrap();
            let manual = x.scale(alpha).add(&y.scale(beta)).unwrap();
            let d = direct.distance(&manual).unwrap();
            let scale = direct.norm().max(1.0);
            prop_assert!(d <= TOL * scale);
        }

        /// Cat 1 / Test 12 — `affine3` matches the manual three-term form.
        #[test]
        fn affine3_matches_manual_pt(
            (x, y, z) in tensor_triple(32),
            alpha in -2.0_f32..2.0,
            beta  in -2.0_f32..2.0,
            gamma in -2.0_f32..2.0,
        ) {
            let direct = Tensor::affine3(alpha, &x, beta, &y, gamma, &z).unwrap();
            let two_term = Tensor::affine2(alpha, &x, beta, &y).unwrap();
            let manual = two_term.add(&z.scale(gamma)).unwrap();
            let d = direct.distance(&manual).unwrap();
            let scale = direct.norm().max(1.0);
            prop_assert!(d <= TOL * scale);
        }

        /// Cat 1 / Test 13 — `add_assign` is equivalent to pure `add`.
        #[test]
        fn add_assign_matches_pure_add_pt((x, y) in tensor_pair(32)) {
            let mut a = x.clone();
            a.add_assign(&y).unwrap();
            let b = x.add(&y).unwrap();
            prop_assert!(a.distance(&b).unwrap() < TOL);
        }

        /// Cat 1 / Test 14 — ℓ∞ norm ≤ ℓ² norm.
        #[test]
        fn linfty_at_most_l2_pt(x in bounded_tensor(32)) {
            let linf = x.norm_infinity();
            let l2   = x.norm();
            prop_assert!(linf <= l2 + TOL, "‖x‖∞ ({linf}) > ‖x‖₂ ({l2})");
        }

        /// Cat 1 / Test 15 — `pseudo_random` outputs are bounded by 1 in ℓ∞.
        ///
        /// The internal LCG of `Tensor::pseudo_random` maps the high 24
        /// bits into `[-1, 1)`, so every component is < 1 in absolute
        /// value.  This test pins that contract.
        #[test]
        fn pseudo_random_bounded_infty_pt(
            dim in 1usize..=64,
            seed in any::<u64>(),
        ) {
            let t = Tensor::pseudo_random(dim, seed);
            prop_assert!(
                t.norm_infinity() <= 1.0 + TOL,
                "‖pseudo_random({dim}, {seed})‖∞ = {} > 1", t.norm_infinity()
            );
        }
    }
}
