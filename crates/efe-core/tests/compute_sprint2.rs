//! Sprint 2 integration tests — softmax, log_softmax, gelu, dot, matmul,
//! matmul_fused_gelu via the [`ComputeBackend`] public surface.
//!
//! Golden values come from PyTorch reference runs and are therefore
//! intentionally printed with more precision than f32 can hold (clippy
//! warns but the implicit truncation matches what PyTorch itself does).
#![allow(clippy::float_cmp)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::redundant_closure_for_method_calls)]
#![allow(clippy::cast_precision_loss)]

use efe_core::compute::{ComputeBackend, ComputeError, CpuBackend, Tensor};

// ====================================================================
// softmax
// ====================================================================

#[test]
fn softmax_standard_row_sums_to_one() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
    let y = b.softmax(&x, -1).unwrap();
    let sum: f32 = y.as_slice().unwrap().iter().sum();
    assert!((sum - 1.0).abs() < 1e-6);
}

#[test]
fn softmax_matches_golden_basic_1d() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
    let y = b.softmax(&x, -1).unwrap();
    // Golden values from tests/golden/softmax/basic_1d.json
    let expected = [0.032_058_6, 0.087_144_32, 0.236_882_82, 0.643_914_26];
    let got = y.as_slice().unwrap();
    for i in 0..4 {
        assert!(
            (got[i] - expected[i]).abs() < 1e-5,
            "mismatch at {}: got {}, expected {}",
            i,
            got[i],
            expected[i]
        );
    }
}

#[test]
fn softmax_handles_extreme_values() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1000.0, 1001.0, 1002.0], [3].into()).unwrap();
    let y = b.softmax(&x, -1).unwrap();
    let s = y.as_slice().unwrap();
    // All values must be finite and sum to 1.
    assert!(s.iter().all(|v| v.is_finite()));
    let sum: f32 = s.iter().sum();
    assert!((sum - 1.0).abs() < 1e-5);
    // Matches golden : [0.09003057, 0.24472847, 0.66524096]
    assert!((s[0] - 0.090_030_57).abs() < 1e-5);
    assert!((s[1] - 0.244_728_47).abs() < 1e-5);
    assert!((s[2] - 0.665_240_96).abs() < 1e-5);
}

#[test]
fn softmax_batched_2d_uniform() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::zeros([2, 4].into()).unwrap();
    let y = b.softmax(&x, -1).unwrap();
    for &v in y.as_slice().unwrap() {
        assert!((v - 0.25).abs() < 1e-6);
    }
}

#[test]
fn softmax_rejects_bad_axis() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::zeros([3].into()).unwrap();
    let err = b.softmax(&x, 5).unwrap_err();
    assert!(matches!(err, ComputeError::InvalidAxis { .. }));
}

// ====================================================================
// log_softmax
// ====================================================================

#[test]
fn log_softmax_matches_golden_basic_1d() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
    let y = b.log_softmax(&x, -1).unwrap();
    let expected = [-3.440_189_84, -2.440_189_84, -1.440_189_84, -0.440_189_84];
    let got = y.as_slice().unwrap();
    for i in 0..4 {
        assert!((got[i] - expected[i]).abs() < 1e-5);
    }
}

#[test]
fn log_softmax_is_log_of_softmax() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[0.5, 1.5, -0.2, 2.1, 0.0], [5].into()).unwrap();
    let ls = b.log_softmax(&x, -1).unwrap();
    let sm = b.softmax(&x, -1).unwrap();
    let ls_s = ls.as_slice().unwrap();
    let sm_s = sm.as_slice().unwrap();
    for i in 0..5 {
        assert!((ls_s[i] - sm_s[i].ln()).abs() < 1e-5);
    }
}

// ====================================================================
// gelu
// ====================================================================

#[test]
fn gelu_matches_golden_reference_points() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[-3.0, -1.0, -0.5, 0.0, 0.5, 1.0, 3.0],
        [7].into(),
    )
    .unwrap();
    let y = b.gelu(&x).unwrap();
    let expected = [
        -0.004_049_52,
        -0.158_655_25,
        -0.154_268_77,
        0.0,
        0.345_731_23,
        0.841_344_75,
        2.995_950_48,
    ];
    let got = y.as_slice().unwrap();
    for i in 0..7 {
        assert!(
            (got[i] - expected[i]).abs() < 1e-5,
            "GELU mismatch at x={}: got {}, expected {}",
            [-3.0, -1.0, -0.5, 0.0, 0.5, 1.0, 3.0][i],
            got[i],
            expected[i]
        );
    }
}

#[test]
fn gelu_preserves_tensor_shape() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::ones([3, 4, 5].into()).unwrap();
    let y = b.gelu(&x).unwrap();
    assert_eq!(y.shape().dims(), &[3, 4, 5]);
}

// ====================================================================
// dot
// ====================================================================

#[test]
fn dot_basic() {
    let b = CpuBackend::new();
    let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
    let bb = Tensor::<f32>::from_slice(&[5.0, 6.0, 7.0, 8.0], [4].into()).unwrap();
    // 1*5+2*6+3*7+4*8 = 5+12+21+32 = 70
    assert!((b.dot(&a, &bb).unwrap() - 70.0).abs() < 1e-6);
}

#[test]
fn dot_is_kahan_reproducible_on_long_vectors() {
    let b = CpuBackend::new();
    // 10 000 elements, each 1e-6. True sum = 1e-2.
    let n = 10_000;
    let a = Tensor::<f32>::from_slice(&vec![1.0f32; n], [n].into()).unwrap();
    let bb = Tensor::<f32>::from_slice(&vec![1e-6f32; n], [n].into()).unwrap();
    let got = b.dot(&a, &bb).unwrap();
    assert!(
        (got - 1e-2).abs() < 1e-6,
        "Kahan sum drifted: got {got}, expected 1e-2"
    );
}

#[test]
fn dot_rejects_different_shapes() {
    let b = CpuBackend::new();
    let a = Tensor::<f32>::zeros([3].into()).unwrap();
    let bb = Tensor::<f32>::zeros([4].into()).unwrap();
    assert!(matches!(
        b.dot(&a, &bb).unwrap_err(),
        ComputeError::ShapeMismatch { .. }
    ));
}

// ====================================================================
// matmul
// ====================================================================

#[test]
fn matmul_2x2_golden() {
    let b = CpuBackend::new();
    // [[1,2],[3,4]] @ [[5,6],[7,8]] = [[19,22],[43,50]]
    let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
    let bb = Tensor::<f32>::from_slice(&[5.0, 6.0, 7.0, 8.0], [2, 2].into()).unwrap();
    let c = b.matmul(&a, &bb).unwrap();
    let s = c.as_slice().unwrap();
    assert!((s[0] - 19.0).abs() < 1e-5);
    assert!((s[1] - 22.0).abs() < 1e-5);
    assert!((s[2] - 43.0).abs() < 1e-5);
    assert!((s[3] - 50.0).abs() < 1e-5);
}

#[test]
fn matmul_identity_preserves() {
    let b = CpuBackend::new();
    let id = Tensor::<f32>::from_slice(
        &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        [3, 3].into(),
    )
    .unwrap();
    let x_data: Vec<f32> = (0..15).map(|i| i as f32 * 0.1).collect();
    let x = Tensor::<f32>::from_slice(&x_data, [3, 5].into()).unwrap();
    let y = b.matmul(&id, &x).unwrap();
    let s = y.as_slice().unwrap();
    for i in 0..15 {
        assert!((s[i] - x_data[i]).abs() < 1e-5);
    }
}

#[test]
fn matmul_3x4_times_4x2() {
    let b = CpuBackend::new();
    let a_data: Vec<f32> = (0..12).map(|i| i as f32).collect();
    let b_data: Vec<f32> = (0..8).map(|i| i as f32).collect();
    let a = Tensor::<f32>::from_slice(&a_data, [3, 4].into()).unwrap();
    let bb = Tensor::<f32>::from_slice(&b_data, [4, 2].into()).unwrap();
    let c = b.matmul(&a, &bb).unwrap();
    // Expected from golden: [28, 34, 76, 98, 124, 162]
    let s = c.as_slice().unwrap();
    let expected = [28.0, 34.0, 76.0, 98.0, 124.0, 162.0];
    for i in 0..6 {
        assert!((s[i] - expected[i]).abs() < 1e-4);
    }
}

#[test]
fn matmul_rejects_incompatible_shapes() {
    let b = CpuBackend::new();
    let a = Tensor::<f32>::zeros([2, 3].into()).unwrap();
    let bb = Tensor::<f32>::zeros([4, 5].into()).unwrap();
    assert!(matches!(
        b.matmul(&a, &bb).unwrap_err(),
        ComputeError::ShapeMismatch { .. }
    ));
}

// ====================================================================
// matmul_fused_gelu
// ====================================================================

#[test]
fn matmul_fused_gelu_no_bias_equivalent_to_gelu_of_matmul() {
    let b = CpuBackend::new();
    let a = Tensor::<f32>::from_slice(&[1.0, 0.5, -0.5, 0.0], [2, 2].into()).unwrap();
    let bb = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0, 1.0], [2, 2].into()).unwrap();

    let fused = b.matmul_fused_gelu(&a, &bb, None).unwrap();

    let manual = {
        let c = b.matmul(&a, &bb).unwrap();
        b.gelu(&c).unwrap()
    };

    let sf = fused.as_slice().unwrap();
    let sm = manual.as_slice().unwrap();
    for i in 0..4 {
        assert!((sf[i] - sm[i]).abs() < 1e-5);
    }
}

#[test]
fn matmul_fused_gelu_with_bias_golden() {
    let b = CpuBackend::new();
    let a = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0, 1.0], [2, 2].into()).unwrap();
    let bb = Tensor::<f32>::from_slice(&[2.0, 3.0, 4.0, 5.0], [2, 2].into()).unwrap();
    let bias = Tensor::<f32>::from_slice(&[10.0, 20.0], [2].into()).unwrap();

    let out = b.matmul_fused_gelu(&a, &bb, Some(&bias)).unwrap();
    let s = out.as_slice().unwrap();

    // Large positive inputs → GELU(x) ≈ x (to 3 decimals)
    assert!((s[0] - 12.0).abs() < 1e-3);
    assert!((s[1] - 23.0).abs() < 1e-3);
    assert!((s[2] - 14.0).abs() < 1e-3);
    assert!((s[3] - 25.0).abs() < 1e-3);
}

#[test]
fn matmul_fused_gelu_rejects_bias_shape_mismatch() {
    let b = CpuBackend::new();
    let a = Tensor::<f32>::zeros([2, 3].into()).unwrap();
    let bb = Tensor::<f32>::zeros([3, 4].into()).unwrap();
    let bias = Tensor::<f32>::zeros([5].into()).unwrap(); // must be [4]
    assert!(matches!(
        b.matmul_fused_gelu(&a, &bb, Some(&bias)).unwrap_err(),
        ComputeError::ShapeMismatch { .. }
    ));
}

// ====================================================================
// AUDIT — Sprint 2 edge cases
// ====================================================================

#[test]
fn audit_softmax_column_wise_on_2d() {
    // Verifies column softmax (axis=0) works correctly.
    // x = [[1,2,3],[4,5,6]]
    // Columns : [1,4], [2,5], [3,6]
    // Each column softmax should sum to 1.
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        [2, 3].into(),
    )
    .unwrap();
    let y = b.softmax(&x, 0).unwrap();
    let s = y.as_slice().unwrap();
    // Column 0 sum : s[0] + s[3]
    assert!((s[0] + s[3] - 1.0).abs() < 1e-5);
    // Column 1 sum : s[1] + s[4]
    assert!((s[1] + s[4] - 1.0).abs() < 1e-5);
    // Column 2 sum : s[2] + s[5]
    assert!((s[2] + s[5] - 1.0).abs() < 1e-5);
}

#[test]
fn audit_softmax_3d_middle_axis() {
    // 3D tensor (2, 3, 4), softmax along axis=1 (middle).
    // Each "fiber" (b, *, c) is a 3-element vector that should softmax to sum=1.
    let b = CpuBackend::new();
    let data: Vec<f32> = (0..24).map(|i| i as f32).collect();
    let x = Tensor::<f32>::from_slice(&data, [2, 3, 4].into()).unwrap();
    let y = b.softmax(&x, 1).unwrap();
    let s = y.as_slice().unwrap();
    // Strides for (2,3,4) : [12, 4, 1]
    // Fiber (batch=0, channel=0) = indices (0,0,0), (0,1,0), (0,2,0) = 0, 4, 8
    let sum_b0_c0 = s[0] + s[4] + s[8];
    assert!((sum_b0_c0 - 1.0).abs() < 1e-5);
    // Fiber (batch=1, channel=3) = (1,0,3), (1,1,3), (1,2,3) = 15, 19, 23
    let sum_b1_c3 = s[15] + s[19] + s[23];
    assert!((sum_b1_c3 - 1.0).abs() < 1e-5);
}

#[test]
fn audit_l2_normalize_column_wise_on_2d() {
    // Verifies that the refactored l2_normalize works along axis=0.
    // [[3, 0], [4, 0]] columns: [3,4] (norm 5), [0,0] (norm eps → 0)
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[3.0, 0.0, 4.0, 0.0], [2, 2].into()).unwrap();
    let y = b.l2_normalize(&x, 0, 1e-9).unwrap();
    let s = y.as_slice().unwrap();
    // Column 0 : [3/5, 4/5] = [0.6, 0.8]
    assert!((s[0] - 0.6).abs() < 1e-5);
    assert!((s[2] - 0.8).abs() < 1e-5);
    // Column 1 : [0, 0]
    assert!(s[1].abs() < 1e-3);
    assert!(s[3].abs() < 1e-3);
}

#[test]
fn audit_matmul_with_zero_inner_dim() {
    // A(3, 0) × B(0, 4) = C(3, 4) of zeros (sum over empty = 0).
    let b = CpuBackend::new();
    let a = Tensor::<f32>::zeros([3, 0].into()).unwrap();
    let bb = Tensor::<f32>::zeros([0, 4].into()).unwrap();
    let c = b.matmul(&a, &bb).unwrap();
    assert_eq!(c.shape().dims(), &[3, 4]);
    for &v in c.as_slice().unwrap() {
        assert!(v.abs() < 1e-7);
    }
}

#[test]
fn audit_matmul_with_zero_output_rows() {
    // A(0, 3) × B(3, 4) = C(0, 4) empty output.
    let b = CpuBackend::new();
    let a = Tensor::<f32>::zeros([0, 3].into()).unwrap();
    let bb = Tensor::<f32>::zeros([3, 4].into()).unwrap();
    let c = b.matmul(&a, &bb).unwrap();
    assert_eq!(c.shape().dims(), &[0, 4]);
    assert_eq!(c.numel(), 0);
}

#[test]
fn audit_empty_1d_softmax_returns_empty() {
    // Edge case : shape [0] → numel 0
    let b = CpuBackend::new();
    let x = Tensor::<f32>::zeros([0].into()).unwrap();
    let y = b.softmax(&x, -1).unwrap();
    assert_eq!(y.shape().dims(), &[0]);
    assert_eq!(y.numel(), 0);
}

#[test]
fn audit_dot_on_empty_1d() {
    // dot over empty vectors is 0 (empty sum).
    let b = CpuBackend::new();
    let a = Tensor::<f32>::zeros([0].into()).unwrap();
    let bb = Tensor::<f32>::zeros([0].into()).unwrap();
    assert!((b.dot(&a, &bb).unwrap()).abs() < 1e-9);
}

#[test]
fn audit_mean_on_single_element() {
    // Single-element mean is itself.
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[42.0], [1].into()).unwrap();
    let y = b.mean(&x, -1).unwrap();
    assert_eq!(y.rank(), 0);
    assert!((y.as_slice().unwrap()[0] - 42.0).abs() < 1e-7);
}

#[test]
fn audit_gelu_dtype_preserved() {
    // Verify dtype name is preserved through op.
    let b = CpuBackend::new();
    let x = Tensor::<f32>::zeros([3].into()).unwrap();
    let y = b.gelu(&x).unwrap();
    assert_eq!(y.dtype_name(), "f32");
}

#[test]
fn audit_matmul_fused_gelu_none_bias_equals_plain_gelu_of_matmul_precisely() {
    // Regression guard : fused with None bias must be bit-equivalent to
    // the explicit composition matmul → gelu.
    let b = CpuBackend::new();
    let a = Tensor::<f32>::from_slice(
        &(0..12).map(|i| (i as f32) * 0.1).collect::<Vec<_>>(),
        [3, 4].into(),
    )
    .unwrap();
    let bb = Tensor::<f32>::from_slice(
        &(0..8).map(|i| (i as f32) * 0.2 - 0.5).collect::<Vec<_>>(),
        [4, 2].into(),
    )
    .unwrap();
    let fused = b.matmul_fused_gelu(&a, &bb, None).unwrap();
    let c = b.matmul(&a, &bb).unwrap();
    let manual = b.gelu(&c).unwrap();
    let sf = fused.as_slice().unwrap();
    let sm = manual.as_slice().unwrap();
    assert_eq!(sf.len(), sm.len());
    for i in 0..sf.len() {
        assert!(
            (sf[i] - sm[i]).abs() < 1e-6,
            "fused != composition at {}: {} vs {}",
            i,
            sf[i],
            sm[i]
        );
    }
}
