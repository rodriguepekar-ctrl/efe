//! Property-based tests for Sprint 2 operations using `proptest`.
//!
//! Each `proptest!` block generates ~100 random inputs satisfying the
//! declared preconditions and checks that the given algebraic invariant
//! holds. Together these cover ~1 200 cases across 12 properties.
#![allow(clippy::float_cmp)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::needless_range_loop)]

use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};
use proptest::prelude::*;

// ====================================================================
// Helpers — input generation
// ====================================================================

/// Generate a rank-1 tensor of length in [2..=64] with finite f32 values
/// in a mild range to avoid f32 saturation corner cases.
fn finite_vec(min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(-10.0f32..10.0, min_len..=max_len)
}

/// Generate a positive f32 in [0.01, 10.0].
fn positive_scalar() -> impl Strategy<Value = f32> {
    0.01f32..10.0
}

// ====================================================================
// 1. softmax properties
// ====================================================================

proptest! {
    /// Softmax always sums to 1 on finite inputs.
    #[test]
    fn softmax_sums_to_one(data in finite_vec(2, 64)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let y = b.softmax(&t, -1).unwrap();
        let sum: f32 = y.as_slice().unwrap().iter().sum();
        prop_assert!((sum - 1.0).abs() < 1e-5,
            "softmax sum drifted: {}", sum);
    }

    /// Softmax produces values in [0, 1].
    #[test]
    fn softmax_values_in_unit_interval(data in finite_vec(2, 32)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let y = b.softmax(&t, -1).unwrap();
        for &v in y.as_slice().unwrap() {
            prop_assert!((0.0..=1.0 + 1e-6).contains(&v),
                "softmax value out of range: {}", v);
        }
    }

    /// Softmax is shift-invariant: softmax(x + c) = softmax(x).
    #[test]
    fn softmax_shift_invariant(
        data in finite_vec(2, 32),
        shift in -50.0f32..50.0,
    ) {
        let b = CpuBackend::new();
        let t1 = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let shifted: Vec<f32> = data.iter().map(|v| v + shift).collect();
        let t2 = Tensor::<f32>::from_slice(&shifted, [shifted.len()].into()).unwrap();
        let y1 = b.softmax(&t1, -1).unwrap();
        let y2 = b.softmax(&t2, -1).unwrap();
        let s1 = y1.as_slice().unwrap();
        let s2 = y2.as_slice().unwrap();
        for i in 0..s1.len() {
            prop_assert!((s1[i] - s2[i]).abs() < 1e-4);
        }
    }

    /// Softmax preserves argmax: argmax(softmax(x)) == argmax(x).
    #[test]
    fn softmax_preserves_argmax(data in finite_vec(2, 32)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let y = b.softmax(&t, -1).unwrap();
        // Compute argmaxes by hand (tie-break : smallest index).
        let argmax_in = data.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i).unwrap();
        let out = y.as_slice().unwrap();
        let argmax_out = out.iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i).unwrap();
        prop_assert_eq!(argmax_in, argmax_out);
    }
}

// ====================================================================
// 2. log_softmax properties
// ====================================================================

proptest! {
    /// log_softmax values are all ≤ 0.
    #[test]
    fn log_softmax_nonpositive(data in finite_vec(2, 32)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let y = b.log_softmax(&t, -1).unwrap();
        for &v in y.as_slice().unwrap() {
            prop_assert!(v <= 1e-5, "log_softmax value positive: {}", v);
        }
    }

    /// log_softmax = log(softmax) up to tolerance.
    #[test]
    fn log_softmax_equals_log_of_softmax(data in finite_vec(2, 32)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let ls = b.log_softmax(&t, -1).unwrap();
        let sm = b.softmax(&t, -1).unwrap();
        let ls_s = ls.as_slice().unwrap();
        let sm_s = sm.as_slice().unwrap();
        for i in 0..ls_s.len() {
            if sm_s[i] > 1e-20 {
                prop_assert!((ls_s[i] - sm_s[i].ln()).abs() < 1e-4);
            }
        }
    }
}

// ====================================================================
// 3. gelu properties
// ====================================================================

proptest! {
    /// GELU preserves tensor shape.
    #[test]
    fn gelu_preserves_shape(data in finite_vec(1, 32)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let y = b.gelu(&t).unwrap();
        prop_assert_eq!(y.shape().dims(), t.shape().dims());
    }

    /// GELU(0) = 0 exactly.
    #[test]
    fn gelu_zero_invariant(len in 1usize..16) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::zeros([len].into()).unwrap();
        let y = b.gelu(&t).unwrap();
        for &v in y.as_slice().unwrap() {
            prop_assert!(v.abs() < 1e-7);
        }
    }

    /// GELU is bounded by x for x > 0 (i.e. GELU(x) ≤ x).
    #[test]
    fn gelu_bounded_above_by_identity_on_positives(
        data in prop::collection::vec(0.01f32..10.0, 1..32)
    ) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let y = b.gelu(&t).unwrap();
        for (i, &v) in y.as_slice().unwrap().iter().enumerate() {
            prop_assert!(v <= data[i] + 1e-5,
                "GELU({}) = {} > {}", data[i], v, data[i]);
        }
    }
}

// ====================================================================
// 4. dot properties
// ====================================================================

proptest! {
    /// dot is commutative.
    #[test]
    fn dot_commutative(
        a in finite_vec(1, 32),
        b in finite_vec(1, 32),
    ) {
        // Align lengths
        let n = a.len().min(b.len());
        let av = &a[..n];
        let bv = &b[..n];
        let backend = CpuBackend::new();
        let ta = Tensor::<f32>::from_slice(av, [n].into()).unwrap();
        let tb = Tensor::<f32>::from_slice(bv, [n].into()).unwrap();
        let ab = backend.dot(&ta, &tb).unwrap();
        let ba = backend.dot(&tb, &ta).unwrap();
        prop_assert!((ab - ba).abs() < 1e-5);
    }

    /// dot(a, a) = ||a||² (non-negative).
    #[test]
    fn dot_self_is_nonneg_and_equals_sum_of_squares(data in finite_vec(1, 32)) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let d = b.dot(&t, &t).unwrap();
        let expected: f32 = data.iter().map(|v| v * v).sum();
        prop_assert!(d >= -1e-6);
        prop_assert!((d - expected).abs() < 1e-4 * expected.max(1.0));
    }

    /// dot(α·a, b) = α · dot(a, b).
    #[test]
    fn dot_scales_linearly(
        a in finite_vec(1, 16),
        alpha in positive_scalar(),
    ) {
        let n = a.len();
        let b_data: Vec<f32> = (0..n).map(|i| (i as f32) * 0.3 - 1.0).collect();
        let backend = CpuBackend::new();
        let ta = Tensor::<f32>::from_slice(&a, [n].into()).unwrap();
        let tb = Tensor::<f32>::from_slice(&b_data, [n].into()).unwrap();
        let scaled: Vec<f32> = a.iter().map(|v| v * alpha).collect();
        let tas = Tensor::<f32>::from_slice(&scaled, [n].into()).unwrap();
        let lhs = backend.dot(&tas, &tb).unwrap();
        let rhs = alpha * backend.dot(&ta, &tb).unwrap();
        prop_assert!((lhs - rhs).abs() < 1e-3 * lhs.abs().max(1.0));
    }
}

// ====================================================================
// 5. matmul properties
// ====================================================================

proptest! {
    /// matmul output shape = (M, N) given A(M,K) × B(K,N).
    #[test]
    fn matmul_output_shape(
        m in 1usize..8,
        k in 1usize..8,
        n in 1usize..8,
    ) {
        let b = CpuBackend::new();
        let a = Tensor::<f32>::zeros([m, k].into()).unwrap();
        let bb = Tensor::<f32>::zeros([k, n].into()).unwrap();
        let c = b.matmul(&a, &bb).unwrap();
        prop_assert_eq!(c.shape().dims(), &[m, n]);
    }

    /// matmul with zero matrix yields zero output.
    #[test]
    fn matmul_zero_right(
        m in 1usize..6, k in 1usize..6, n in 1usize..6,
        a_data in prop::collection::vec(-5.0f32..5.0, 1..=36),
    ) {
        let b = CpuBackend::new();
        let needed = m * k;
        if a_data.len() < needed { return Ok(()); }
        let a = Tensor::<f32>::from_slice(&a_data[..needed], [m, k].into()).unwrap();
        let zeros = Tensor::<f32>::zeros([k, n].into()).unwrap();
        let c = b.matmul(&a, &zeros).unwrap();
        for &v in c.as_slice().unwrap() {
            prop_assert!(v.abs() < 1e-5);
        }
    }

    /// matmul(I, A) = A for square identity.
    #[test]
    fn matmul_identity_left(
        k in 1usize..6, n in 1usize..6,
        values in prop::collection::vec(-5.0f32..5.0, 1..=36),
    ) {
        let needed = k * n;
        if values.len() < needed { return Ok(()); }
        let b = CpuBackend::new();
        // Build identity k×k
        let mut id = vec![0.0f32; k * k];
        for i in 0..k { id[i * k + i] = 1.0; }
        let t_id = Tensor::<f32>::from_slice(&id, [k, k].into()).unwrap();
        let x = Tensor::<f32>::from_slice(&values[..needed], [k, n].into()).unwrap();
        let y = b.matmul(&t_id, &x).unwrap();
        for i in 0..needed {
            prop_assert!((y.as_slice().unwrap()[i] - values[i]).abs() < 1e-4);
        }
    }
}
