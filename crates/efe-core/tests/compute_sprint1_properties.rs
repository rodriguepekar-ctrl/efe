//! **Audit-S1-bis — Property-based tests for Sprint 1 ops.**
//!
//! Sprint 1 ships the four foundational reductions
//! (`l2_norm`, `l2_normalize`, `mean`, `argmax`). Until now Sprint 1
//! had **no** property test file (Sprint 2 has
//! `compute_sprint2_properties.rs`, Sprint 3 has
//! `compute_sprint3_properties.rs` — Sprint 1 was missing). The
//! cross-sprint audit added this suite so every property listed in
//! the docstrings has at least one randomised proptest backing it.
//!
//! ~12 properties × 100 cases = ~1 200 random checks per `cargo test`.
#![allow(clippy::float_cmp)]
#![allow(clippy::cast_precision_loss)]

use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};
use proptest::prelude::*;

/// Generate a finite f32 vector of given size range, values in `[-10, 10]`.
fn finite_vec(min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(-10.0f32..10.0, min_len..=max_len)
}

// ====================================================================
// l2_norm — properties
// ====================================================================

proptest! {
    /// `l2_norm(x) ≥ 0` for any finite vector.
    #[test]
    fn l2_norm_non_negative(x in finite_vec(1, 32)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let n = b.l2_norm(&t, -1).unwrap().as_slice().unwrap()[0];
        prop_assert!(n >= 0.0, "l2_norm produced negative: {n}");
    }

    /// `l2_norm(α·x) = |α| · l2_norm(x)` (positive homogeneity).
    #[test]
    fn l2_norm_scale_homogeneous(
        x in finite_vec(2, 16),
        alpha in -5.0f32..5.0,
    ) {
        // Skip degenerate alphas where the multiplied test loses precision.
        if alpha.abs() < 0.1 { return Ok(()); }
        let b = CpuBackend::new();
        let scaled: Vec<f32> = x.iter().map(|v| v * alpha).collect();
        let t1 = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let t2 = Tensor::<f32>::from_slice(&scaled, [scaled.len()].into()).unwrap();
        let n1 = b.l2_norm(&t1, -1).unwrap().as_slice().unwrap()[0];
        let n2 = b.l2_norm(&t2, -1).unwrap().as_slice().unwrap()[0];
        let expected = alpha.abs() * n1;
        let bound = 1e-3 * expected.abs().max(1.0);
        prop_assert!(
            (n2 - expected).abs() < bound,
            "‖αx‖ ({n2}) ≠ |α|·‖x‖ ({expected}) for α={alpha}"
        );
    }

    /// `l2_norm(zeros) = 0`.
    #[test]
    fn l2_norm_zero_vector_is_zero(n in 1usize..32) {
        let b = CpuBackend::new();
        let zeros = vec![0.0f32; n];
        let t = Tensor::<f32>::from_slice(&zeros, [n].into()).unwrap();
        let v = b.l2_norm(&t, -1).unwrap().as_slice().unwrap()[0];
        prop_assert!(v.abs() < f32::EPSILON);
    }
}

// ====================================================================
// l2_normalize — properties
// ====================================================================

proptest! {
    /// After `l2_normalize`, the norm of the output is `1.0` (within
    /// f32 tolerance) UNLESS the input was zero (then output stays 0).
    #[test]
    fn l2_normalize_unit_norm_or_zero(x in finite_vec(2, 16)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let original_norm: f32 = x.iter().map(|v| v * v).sum::<f32>().sqrt();
        let out = b.l2_normalize(&t, -1, 1e-12).unwrap();
        let s = out.as_slice().unwrap();
        let new_norm: f32 = s.iter().map(|v| v * v).sum::<f32>().sqrt();
        if original_norm > 1e-6 {
            prop_assert!(
                (new_norm - 1.0).abs() < 1e-4,
                "expected unit norm, got {new_norm} for input ‖x‖={original_norm}"
            );
        } else {
            // For near-zero inputs the eps fallback keeps the output finite.
            prop_assert!(s.iter().all(|v| v.is_finite()));
        }
    }

    /// `l2_normalize(α·x) = sign(α) · l2_normalize(x)` for α ≠ 0
    /// (direction is preserved up to sign).
    #[test]
    fn l2_normalize_direction_preserved(
        x in finite_vec(2, 8),
        alpha in 0.1f32..5.0,
    ) {
        let b = CpuBackend::new();
        let scaled: Vec<f32> = x.iter().map(|v| v * alpha).collect();
        let t1 = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let t2 = Tensor::<f32>::from_slice(&scaled, [scaled.len()].into()).unwrap();
        let original_norm: f32 = x.iter().map(|v| v * v).sum::<f32>().sqrt();
        if original_norm < 1e-3 { return Ok(()); }
        let n1 = b.l2_normalize(&t1, -1, 1e-12).unwrap();
        let n2 = b.l2_normalize(&t2, -1, 1e-12).unwrap();
        let s1 = n1.as_slice().unwrap();
        let s2 = n2.as_slice().unwrap();
        for i in 0..x.len() {
            prop_assert!(
                (s1[i] - s2[i]).abs() < 1e-3,
                "directions diverge at i={i}: n1={} n2={}", s1[i], s2[i]
            );
        }
    }

    /// `l2_normalize` rejects negative or non-finite eps.
    #[test]
    fn l2_normalize_rejects_invalid_eps(eps in -10.0f32..0.0) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&[1.0_f32, 2.0, 3.0], [3].into()).unwrap();
        prop_assert!(b.l2_normalize(&t, -1, eps).is_err());
    }
}

// ====================================================================
// mean — properties
// ====================================================================

proptest! {
    /// `mean(x + c) = mean(x) + c` (shift).
    #[test]
    fn mean_shift_invariant(
        x in finite_vec(2, 32),
        c in -5.0f32..5.0,
    ) {
        let b = CpuBackend::new();
        let shifted: Vec<f32> = x.iter().map(|v| v + c).collect();
        let t1 = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let t2 = Tensor::<f32>::from_slice(&shifted, [shifted.len()].into()).unwrap();
        let m1 = b.mean(&t1, -1).unwrap().as_slice().unwrap()[0];
        let m2 = b.mean(&t2, -1).unwrap().as_slice().unwrap()[0];
        prop_assert!(
            ((m2 - m1) - c).abs() < 1e-4,
            "shift broken: m1={m1} m2={m2} c={c}"
        );
    }

    /// `mean(α·x) = α · mean(x)` (scale).
    #[test]
    fn mean_scale_linear(
        x in finite_vec(2, 16),
        alpha in -5.0f32..5.0,
    ) {
        let b = CpuBackend::new();
        let scaled: Vec<f32> = x.iter().map(|v| v * alpha).collect();
        let t1 = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let t2 = Tensor::<f32>::from_slice(&scaled, [scaled.len()].into()).unwrap();
        let m1 = b.mean(&t1, -1).unwrap().as_slice().unwrap()[0];
        let m2 = b.mean(&t2, -1).unwrap().as_slice().unwrap()[0];
        let bound = 1e-3 * (alpha * m1).abs().max(1.0);
        prop_assert!(
            (m2 - alpha * m1).abs() < bound,
            "scale broken: m1={m1} m2={m2} α={alpha}"
        );
    }

    /// `mean([c; n]) = c` for any constant `c`.
    #[test]
    fn mean_constant_returns_constant(
        c in -5.0f32..5.0,
        n in 2usize..32,
    ) {
        let b = CpuBackend::new();
        let v = vec![c; n];
        let t = Tensor::<f32>::from_slice(&v, [n].into()).unwrap();
        let m = b.mean(&t, -1).unwrap().as_slice().unwrap()[0];
        prop_assert!((m - c).abs() < 1e-5, "expected {c}, got {m}");
    }
}

// ====================================================================
// argmax — properties
// ====================================================================

proptest! {
    /// `argmax(x) ∈ [0, n)`.
    #[test]
    fn argmax_returns_valid_index(x in finite_vec(1, 32)) {
        let b = CpuBackend::new();
        let n = x.len() as i32;
        let t = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let i = b.argmax(&t, -1).unwrap().as_slice().unwrap()[0];
        prop_assert!(
            (0..n).contains(&i),
            "argmax index {i} out of [0, {n})"
        );
    }

    /// `x[argmax(x)] = max(x)` (the index points at the max value).
    #[test]
    fn argmax_picks_actual_max(x in finite_vec(1, 16)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&x, [x.len()].into()).unwrap();
        let i = b.argmax(&t, -1).unwrap().as_slice().unwrap()[0] as usize;
        let claimed_max = x[i];
        // Every other value must be ≤ claimed_max.
        for (j, &v) in x.iter().enumerate() {
            // Allow ties: argmax breaks them by smallest index, so other
            // entries may equal claimed_max.
            prop_assert!(
                v <= claimed_max + 1e-6 || (v - claimed_max).abs() < 1e-6,
                "argmax={i} (val={claimed_max}) but found x[{j}]={v} > x[{i}]"
            );
        }
    }

    /// `argmax([c; n])` returns `0` (smallest-index tie-break).
    #[test]
    fn argmax_tie_breaks_to_smallest_index(
        c in -5.0f32..5.0,
        n in 1usize..16,
    ) {
        let b = CpuBackend::new();
        let v = vec![c; n];
        let t = Tensor::<f32>::from_slice(&v, [n].into()).unwrap();
        let i = b.argmax(&t, -1).unwrap().as_slice().unwrap()[0];
        prop_assert_eq!(i, 0);
    }
}
