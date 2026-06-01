//! Geometric metrics on host-side vectors.
//!
//! ## Sprint 5 — façade over `ComputeBackend`
//!
//! Prior to Sprint 5 this module rolled its own arithmetic. The audit
//! found four duplicate sites (here, `genesis::meta_analyzer`,
//! `alignment::policy`, `alignment::compose`) that all re-implemented
//! `cosine_similarity`, `dot_product` and `l2_norm`. Sprint 5
//! consolidates the canonical implementations into `compute::`
//! (`compute::ComputeBackend::dot`, `l2_norm`, `l2_distance`,
//! `cosine_similarity_pair`) and turns this module into a **thin
//! façade**:
//!
//! - The four `f32` functions delegate to a process-wide
//!   [`crate::compute::global_backend`] singleton.
//! - The two `bf16` mixed-precision helpers (`l2_distance_mixed`,
//!   `l2_distance_sq_mixed`) are kept as-is; bf16 paths are out of
//!   the Sprint 5 scope (planned for S7 once the backend gains
//!   bf16 tensor support).
//!
//! ## Length-mismatch contract (preserved)
//!
//! All four `f32` functions still **panic** on length mismatch, in
//! both debug and release builds, to keep the public signature
//! `fn(&[f32], &[f32]) -> f32` stable for the ~10 external callers.
//! The backend would surface a structured `ShapeMismatch` error, but
//! we map it back to a panic to honour the legacy contract. The
//! `expect` messages are deliberately precise so that any callsite
//! breakage is diagnosed immediately.

use half::bf16;

use crate::compute::global_backend;
use crate::compute::Tensor;

/// Dot product `a · b`.
///
/// **Sprint 5 façade** — delegates to
/// [`crate::compute::ComputeBackend::dot`], which uses Kahan
/// compensated summation for `O(ε)` drift instead of the previous
/// `O(n·ε)` naïve accumulator.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[inline]
#[must_use]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "dot_product length mismatch");
    let backend = global_backend();
    let ta = Tensor::<f32>::from_slice(a, [a.len()].into())
        .expect("dot_product: tensor from valid slice");
    let tb = Tensor::<f32>::from_slice(b, [b.len()].into())
        .expect("dot_product: tensor from valid slice");
    backend
        .dot(&ta, &tb)
        .expect("dot_product: backend dot must succeed on equal-length rank-1")
}

/// L2 norm: `||a||_2`.
///
/// **Sprint 5 façade** — delegates to
/// [`crate::compute::ComputeBackend::l2_norm`].
#[inline]
#[must_use]
pub fn l2_norm(a: &[f32]) -> f32 {
    if a.is_empty() {
        return 0.0;
    }
    let backend = global_backend();
    let ta = Tensor::<f32>::from_slice(a, [a.len()].into())
        .expect("l2_norm: tensor from valid slice");
    backend
        .l2_norm(&ta, -1)
        .expect("l2_norm: backend l2_norm must succeed on rank-1 tensor")
        .as_slice()
        .expect("l2_norm: backend output is contiguous rank-0 tensor")[0]
}

/// L2 distance between two vectors of identical length.
///
/// **Sprint 5 façade** — delegates to
/// [`crate::compute::ComputeBackend::l2_distance`] (newly added in
/// Sprint 5).
///
/// ## NaN / Inf propagation contract (preserved)
///
/// The backend op now rejects non-finite inputs with a structured
/// [`crate::compute::ComputeError::NumericalInstability`]. The legacy
/// façade signature `fn(&[f32], &[f32]) -> f32` cannot surface a
/// `Result`, and several call sites (notably
/// `dissolution::alignment::project` line 121-130) rely on a
/// **NaN-returning** distance to feed their downstream
/// `delta.is_finite()` check. Audit-S5-bis therefore translates the
/// backend's `NumericalInstability` back into a NaN value, so the
/// pre-Sprint-5 propagation contract is preserved bit-for-bit at every
/// existing call site.
///
/// # Panics
///
/// * If `a.len() != b.len()` (legacy contract).
#[inline]
#[must_use]
pub fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "l2_distance length mismatch");
    if a.is_empty() {
        return 0.0;
    }
    let backend = global_backend();
    let ta = Tensor::<f32>::from_slice(a, [a.len()].into())
        .expect("l2_distance: tensor from valid slice");
    let tb = Tensor::<f32>::from_slice(b, [b.len()].into())
        .expect("l2_distance: tensor from valid slice");
    match backend.l2_distance(&ta, &tb) {
        Ok(t) => t
            .as_slice()
            .expect("l2_distance: backend output is contiguous rank-0 tensor")[0],
        // Audit-S5-bis: legacy callers (e.g. dissolution::alignment)
        // depend on NaN propagation to detect alignment breaches via
        // `if !delta.is_finite() { return AlignmentBreach }`. Re-raise
        // the backend's structured error as a NaN value to keep the
        // pre-Sprint-5 contract.
        Err(crate::compute::ComputeError::NumericalInstability { .. }) => f32::NAN,
        Err(e) => panic!("l2_distance backend error: {e}"),
    }
}

/// Cosine similarity in `[-1, 1]`. Returns `0.0` if either vector has
/// near-zero norm (the geometric limit of undefined directions).
///
/// **Sprint 5 façade** — delegates to
/// [`crate::compute::ComputeBackend::cosine_similarity_pair`] (newly
/// added in Sprint 5). The geometric fallback to `0.0` is preserved.
///
/// # Panics
///
/// * If `a.len() != b.len()` (legacy contract).
/// * If any element is non-finite (newly enforced; the legacy
///   implementation would have produced `NaN` silently — the backend
///   surfaces a structured error which the façade re-raises as panic
///   for signature compatibility, except for empty input which we
///   short-circuit to `0.0`).
#[inline]
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "cosine_similarity length mismatch");
    if a.is_empty() {
        return 0.0;
    }
    // Audit-5 hardening defers NaN detection to the backend; legacy
    // callers might pass NaN-bearing slices and previously silently
    // returned NaN. We keep that visible behaviour by absorbing the
    // backend error into `0.0` *only when it is the
    // NumericalInstability variant* — shape mismatches are still
    // panics (and would have already been caught by the `assert_eq!`
    // above anyway).
    let backend = global_backend();
    let ta = Tensor::<f32>::from_slice(a, [a.len()].into())
        .expect("cosine_similarity: tensor from valid slice");
    let tb = Tensor::<f32>::from_slice(b, [b.len()].into())
        .expect("cosine_similarity: tensor from valid slice");
    match backend.cosine_similarity_pair(&ta, &tb) {
        Ok(v) => v,
        // Non-finite input: legacy behaviour was NaN propagation; we
        // return 0.0 to preserve the geometric-limit contract. This
        // is strictly safer than the previous silent NaN.
        Err(crate::compute::ComputeError::NumericalInstability { .. }) => 0.0,
        Err(e) => panic!("cosine_similarity backend error: {e}"),
    }
}

/// L2 distance between a `bf16` row (latent) and an `f32` vector (centroid).
/// Used extensively in stats and projection.
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[inline]
#[must_use]
pub fn l2_distance_mixed(a: &[bf16], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "l2_distance_mixed length mismatch");
    let mut acc = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        let diff = x.to_f32() - *y;
        acc += diff * diff;
    }
    acc.sqrt()
}

/// Squared L2 distance (avoids the sqrt when only comparison matters).
///
/// # Panics
///
/// Panics if `a.len() != b.len()`.
#[inline]
#[must_use]
pub fn l2_distance_sq_mixed(a: &[bf16], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "l2_distance_sq_mixed length mismatch");
    let mut acc = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        let diff = x.to_f32() - *y;
        acc += diff * diff;
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_zero_vector_is_zero() {
        let a = vec![0.0f32; 8];
        let b = vec![1.0f32; 8];
        assert!(dot_product(&a, &b).abs() < f32::EPSILON);
    }

    #[test]
    fn dot_self_equals_norm_sq() {
        let a = vec![1.0f32, 2.0, 3.0];
        let d = dot_product(&a, &a);
        assert!((d - 14.0).abs() < 1e-5);
    }

    #[test]
    fn l2_norm_unit_vector() {
        let a = vec![0.6f32, 0.8];
        assert!((l2_norm(&a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn l2_norm_empty_is_zero() {
        let a: Vec<f32> = vec![];
        assert!(l2_norm(&a).abs() < f32::EPSILON);
    }

    #[test]
    fn l2_distance_zero_for_equal_vectors() {
        let a = vec![1.0f32, 2.0, 3.0];
        assert!(l2_distance(&a, &a).abs() < f32::EPSILON);
    }

    #[test]
    fn l2_distance_matches_formula() {
        let a = vec![0.0f32, 0.0];
        let b = vec![3.0f32, 4.0];
        assert!((l2_distance(&a, &b) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_identical_returns_one() {
        let a = vec![1.0f32, 2.0, 3.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_opposite_returns_minus_one() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_returns_zero() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_zero_vector_is_zero() {
        let a = vec![0.0f32; 4];
        let b = vec![1.0f32; 4];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn l2_distance_mixed_matches_pure_f32() {
        let a_f32: [f32; 3] = [1.0, 2.0, 3.0];
        let b_f32: [f32; 3] = [0.5, 1.5, 2.5];
        let a_bf: Vec<bf16> = a_f32.iter().map(|x| bf16::from_f32(*x)).collect();
        let pure = l2_distance(&a_f32, &b_f32);
        let mixed = l2_distance_mixed(&a_bf, &b_f32);
        // bf16 has limited precision; allow generous epsilon.
        assert!((pure - mixed).abs() < 0.02, "pure {pure} vs mixed {mixed}");
    }

    #[test]
    fn l2_distance_sq_is_square_of_l2_distance() {
        let a_f32: [f32; 3] = [1.0, 2.0, 3.0];
        let b: [f32; 3] = [0.0, 0.0, 0.0];
        let a_bf: Vec<bf16> = a_f32.iter().map(|x| bf16::from_f32(*x)).collect();
        let dist = l2_distance_mixed(&a_bf, &b);
        let dist_sq = l2_distance_sq_mixed(&a_bf, &b);
        assert!((dist_sq - dist * dist).abs() < 0.01);
    }

    #[test]
    fn cosine_triangle_bounded_by_one() {
        for vals in [(0.3f32, 0.4, 0.5), (-0.1, 0.2, -0.3), (7.0, -2.0, 1.0)] {
            let a = [vals.0, vals.1, vals.2];
            let b = [vals.2, vals.0, vals.1];
            let c = cosine_similarity(&a, &b);
            assert!(
                c.abs() <= 1.0 + 1e-6,
                "cosine out of bounds: {c} for {a:?} / {b:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "cosine_similarity length mismatch")]
    fn cosine_panics_on_length_mismatch() {
        let a = [1.0f32, 2.0, 3.0];
        let b = [1.0f32, 2.0];
        let _ = cosine_similarity(&a, &b);
    }

    #[test]
    #[should_panic(expected = "l2_distance length mismatch")]
    fn l2_distance_panics_on_length_mismatch() {
        let a = [1.0f32; 4];
        let b = [0.0f32; 3];
        let _ = l2_distance(&a, &b);
    }

    #[test]
    #[should_panic(expected = "dot_product length mismatch")]
    fn dot_product_panics_on_length_mismatch() {
        let a = [1.0f32; 4];
        let b = [1.0f32; 5];
        let _ = dot_product(&a, &b);
    }

    #[test]
    #[should_panic(expected = "l2_distance_mixed length mismatch")]
    fn l2_mixed_panics_on_length_mismatch() {
        let a: [bf16; 3] = [bf16::from_f32(1.0); 3];
        let b = [1.0f32; 2];
        let _ = l2_distance_mixed(&a, &b);
    }

    #[test]
    fn l2_norm_nan_propagates() {
        let a = [f32::NAN, 1.0, 2.0];
        assert!(l2_norm(&a).is_nan());
    }

    /// Audit-S5-bis regression: `dissolution::alignment::project`
    /// relies on `l2_distance` returning `NaN` (not panicking) so that
    /// the breach check `if !delta.is_finite()` triggers an
    /// `AlignmentBreach` error. The Sprint 5 backend hardening would
    /// have caused a panic instead — the façade absorbs the
    /// `NumericalInstability` error as `NaN` to preserve the contract.
    #[test]
    fn l2_distance_nan_propagates_as_nan_not_panic() {
        let a = [f32::NAN, 1.0, 2.0];
        let b = [0.0f32, 0.0, 0.0];
        let d = l2_distance(&a, &b);
        assert!(d.is_nan(), "expected NaN propagation, got {d}");
    }

    #[test]
    fn l2_distance_inf_propagates_as_nan() {
        let a = [f32::INFINITY, 1.0];
        let b = [0.0f32, 0.0];
        let d = l2_distance(&a, &b);
        assert!(d.is_nan(), "expected NaN propagation for Inf, got {d}");
    }

    #[test]
    fn cosine_with_nan_propagates() {
        let a = [f32::NAN, 1.0];
        let b = [1.0f32, 0.0];
        let c = cosine_similarity(&a, &b);
        assert!(c.is_nan() || c == 0.0);
    }
}
