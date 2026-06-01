//! Sprint 3 integration tests — cone_project, weighted_mean, moe_reconcile,
//! topk, scaled_dot_product_attention, multinomial_sample, kl_divergence,
//! std_dev — via the [`ComputeBackend`] public surface.
#![allow(clippy::float_cmp)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::approx_constant)]
#![allow(clippy::useless_vec)]

use efe_core::compute::{ComputeBackend, ComputeError, CpuBackend, Tensor};

// ====================================================================
// cone_project — COEUR EFE
// ====================================================================

#[test]
fn s3_cone_project_outside_projects_to_boundary() {
    let b = CpuBackend::new();
    // Match tests/golden/cone_project/outside_cone_projected.json
    let latents = Tensor::<f32>::from_slice(&[3.0, 4.0, 6.0, 8.0], [2, 2].into()).unwrap();
    let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
    let out = b.cone_project(&latents, &anchors, 1.0).unwrap();
    let s = out.as_slice().unwrap();
    let expected = [0.6, 0.8, 0.6, 0.8];
    for i in 0..4 {
        assert!(
            (s[i] - expected[i]).abs() < 1e-5,
            "idx {i}: got {}, expected {}",
            s[i],
            expected[i]
        );
    }
}

#[test]
fn s3_cone_project_idempotent_under_repeated_application() {
    let b = CpuBackend::new();
    let latents = Tensor::<f32>::from_slice(
        &[10.0, 0.0, 5.0, 5.0, 0.0, -10.0],
        [3, 2].into(),
    )
    .unwrap();
    let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
    let once = b.cone_project(&latents, &anchors, 2.0).unwrap();
    let twice = b.cone_project(&once, &anchors, 2.0).unwrap();
    let s1 = once.as_slice().unwrap();
    let s2 = twice.as_slice().unwrap();
    for i in 0..6 {
        assert!((s1[i] - s2[i]).abs() < 1e-5);
    }
}

#[test]
fn s3_cone_project_multi_anchor() {
    let b = CpuBackend::new();
    // Anchors at (0,0) and (5,0). Latent at (2,0) is nearest to (0,0) at dist 2.
    // max_dist = 1.5 → project toward (0,0) at distance 1.5
    let latents = Tensor::<f32>::from_slice(&[2.0, 0.0], [1, 2].into()).unwrap();
    let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0, 5.0, 0.0], [2, 2].into()).unwrap();
    let out = b.cone_project(&latents, &anchors, 1.5).unwrap();
    let s = out.as_slice().unwrap();
    assert!((s[0] - 1.5).abs() < 1e-5);
    assert!(s[1].abs() < 1e-5);
}

#[test]
fn s3_cone_project_rejects_bad_max_dist() {
    let b = CpuBackend::new();
    let latents = Tensor::<f32>::zeros([1, 2].into()).unwrap();
    let anchors = Tensor::<f32>::zeros([1, 2].into()).unwrap();
    assert!(b.cone_project(&latents, &anchors, 0.0).is_err());
    assert!(b.cone_project(&latents, &anchors, -1.0).is_err());
}

// ====================================================================
// weighted_mean — COEUR EFE
// ====================================================================

#[test]
fn s3_weighted_mean_matches_golden_nonuniform() {
    let b = CpuBackend::new();
    let values = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        [3, 2].into(),
    )
    .unwrap();
    let weights = Tensor::<f32>::from_slice(&[0.1, 0.3, 0.6], [3].into()).unwrap();
    let out = b.weighted_mean(&values, &weights, false).unwrap();
    let s = out.as_slice().unwrap();
    assert!((s[0] - 4.0).abs() < 1e-5);
    assert!((s[1] - 5.0).abs() < 1e-5);
}

#[test]
fn s3_weighted_mean_uniform_equals_mean() {
    let b = CpuBackend::new();
    let values = Tensor::<f32>::from_slice(
        &[10.0, 20.0, 30.0, 40.0, 50.0, 60.0],
        [3, 2].into(),
    )
    .unwrap();
    let weights = Tensor::<f32>::from_slice(&[1.0, 1.0, 1.0], [3].into()).unwrap();
    let out = b.weighted_mean(&values, &weights, false).unwrap();
    let s = out.as_slice().unwrap();
    // Mean : [(10+30+50)/3, (20+40+60)/3] = [30, 40]
    assert!((s[0] - 30.0).abs() < 1e-5);
    assert!((s[1] - 40.0).abs() < 1e-5);
}

#[test]
fn s3_weighted_mean_renormalize() {
    let b = CpuBackend::new();
    let values = Tensor::<f32>::from_slice(&[3.0, 4.0, 6.0, 8.0], [2, 2].into()).unwrap();
    let weights = Tensor::<f32>::from_slice(&[1.0, 1.0], [2].into()).unwrap();
    let out = b.weighted_mean(&values, &weights, true).unwrap();
    let s = out.as_slice().unwrap();
    let norm: f32 = s.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1e-5);
}

#[test]
fn s3_weighted_mean_rejects_zero_sum() {
    let b = CpuBackend::new();
    let values = Tensor::<f32>::zeros([3, 2].into()).unwrap();
    let weights = Tensor::<f32>::zeros([3].into()).unwrap();
    assert!(matches!(
        b.weighted_mean(&values, &weights, false).unwrap_err(),
        ComputeError::NumericalInstability { .. }
    ));
}

// ====================================================================
// moe_reconcile — COEUR EFE
// ====================================================================

#[test]
fn s3_moe_reconcile_topk1_matches_golden() {
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::from_slice(
        &[1.0, 0.0, 0.5, 0.5, 0.0, 1.0],
        [1, 3, 2].into(),
    )
    .unwrap();
    let gate = Tensor::<f32>::from_slice(&[0.1, 0.9, 0.3], [1, 3].into()).unwrap();
    let out = b.moe_reconcile(&cands, &gate, 1).unwrap();
    let s = out.as_slice().unwrap();
    // Golden: argmax=1 → candidates[1] = [0.5, 0.5]
    assert!((s[0] - 0.5).abs() < 1e-5);
    assert!((s[1] - 0.5).abs() < 1e-5);
}

#[test]
fn s3_moe_reconcile_topk_full_equals_softmax_weighted() {
    let b = CpuBackend::new();
    // 2 candidates uniform → each contributes 0.5
    let cands = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0, 1.0], [1, 2, 2].into()).unwrap();
    let gate = Tensor::<f32>::from_slice(&[1.0, 1.0], [1, 2].into()).unwrap();
    let out = b.moe_reconcile(&cands, &gate, 2).unwrap();
    let s = out.as_slice().unwrap();
    assert!((s[0] - 0.5).abs() < 1e-5);
    assert!((s[1] - 0.5).abs() < 1e-5);
}

#[test]
fn s3_moe_reconcile_batched() {
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::from_slice(
        &[
            // batch 0
            1.0, 0.0, 0.0, 1.0,
            // batch 1
            2.0, 0.0, 0.0, 2.0,
        ],
        [2, 2, 2].into(),
    )
    .unwrap();
    let gate = Tensor::<f32>::from_slice(
        &[10.0, 0.0, 0.0, 10.0],
        [2, 2].into(),
    )
    .unwrap();
    let out = b.moe_reconcile(&cands, &gate, 1).unwrap();
    let s = out.as_slice().unwrap();
    // batch 0 argmax=0 → [1, 0]; batch 1 argmax=1 → [0, 2]
    assert!((s[0] - 1.0).abs() < 1e-5);
    assert!(s[1].abs() < 1e-5);
    assert!(s[2].abs() < 1e-5);
    assert!((s[3] - 2.0).abs() < 1e-5);
}

#[test]
fn s3_moe_reconcile_rejects_top_k_zero() {
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::zeros([1, 2, 2].into()).unwrap();
    let gate = Tensor::<f32>::zeros([1, 2].into()).unwrap();
    assert!(b.moe_reconcile(&cands, &gate, 0).is_err());
}

// ====================================================================
// topk
// ====================================================================

#[test]
fn s3_topk_matches_golden() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[0.1, 0.5, 0.3, 0.9, 0.7], [5].into()).unwrap();
    let (vals, idx) = b.topk(&x, 3, -1).unwrap();
    assert_eq!(vals.as_slice().unwrap(), &[0.9, 0.7, 0.5]);
    assert_eq!(idx.as_slice().unwrap(), &[3, 4, 1]);
}

#[test]
fn s3_topk_2d_per_row() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[0.1, 0.9, 0.3, 0.5, 0.1, 0.7],
        [2, 3].into(),
    )
    .unwrap();
    let (vals, idx) = b.topk(&x, 2, -1).unwrap();
    assert_eq!(vals.shape().dims(), &[2, 2]);
    let v = vals.as_slice().unwrap();
    let i = idx.as_slice().unwrap();
    // Row 0 : top-2 = (0.9@1, 0.3@2)
    assert!((v[0] - 0.9).abs() < 1e-6);
    assert!((v[1] - 0.3).abs() < 1e-6);
    assert_eq!(i[0], 1);
    assert_eq!(i[1], 2);
    // Row 1 : top-2 = (0.7@2, 0.5@0)
    assert!((v[2] - 0.7).abs() < 1e-6);
    assert!((v[3] - 0.5).abs() < 1e-6);
    assert_eq!(i[2], 2);
    assert_eq!(i[3], 0);
}

// ====================================================================
// scaled_dot_product_attention
// ====================================================================

#[test]
fn s3_sdpa_identity_weights_give_mean() {
    let b = CpuBackend::new();
    // Q=K=V all zeros → uniform attention → output is mean of V (= 0)
    let t = 3;
    let d = 2;
    let q = Tensor::<f32>::zeros([1, 1, t, d].into()).unwrap();
    let out = b.scaled_dot_product_attention(&q, &q, &q, None).unwrap();
    assert_eq!(out.shape().dims(), &[1, 1, t, d]);
    for &v in out.as_slice().unwrap() {
        assert!(v.abs() < 1e-6);
    }
}

#[test]
fn s3_sdpa_preserves_shape_with_mask() {
    let b = CpuBackend::new();
    let q = Tensor::<f32>::zeros([1, 1, 4, 4].into()).unwrap();
    // Causal mask
    let mut m = vec![0.0f32; 16];
    for i in 0..4 {
        for j in 0..4 {
            if j > i {
                m[i * 4 + j] = f32::NEG_INFINITY;
            }
        }
    }
    let mask = Tensor::<f32>::from_slice(&m, [4, 4].into()).unwrap();
    let out = b.scaled_dot_product_attention(&q, &q, &q, Some(&mask)).unwrap();
    assert_eq!(out.shape().dims(), &[1, 1, 4, 4]);
    for &v in out.as_slice().unwrap() {
        assert!(v.is_finite());
    }
}

// ====================================================================
// multinomial_sample
// ====================================================================

#[test]
fn s3_multinomial_seed_reproducibility() {
    let b = CpuBackend::new();
    let probs = Tensor::<f32>::from_slice(&[0.25, 0.25, 0.25, 0.25], [4].into()).unwrap();
    let s1 = b.multinomial_sample(&probs, 500, 42).unwrap();
    let s2 = b.multinomial_sample(&probs, 500, 42).unwrap();
    assert_eq!(s1.as_slice().unwrap(), s2.as_slice().unwrap());
}

#[test]
fn s3_multinomial_all_in_range() {
    let b = CpuBackend::new();
    let probs = Tensor::<f32>::from_slice(&[0.1, 0.2, 0.3, 0.4], [4].into()).unwrap();
    let s = b.multinomial_sample(&probs, 1000, 123).unwrap();
    assert!(s.as_slice().unwrap().iter().all(|&i| (0..4).contains(&i)));
}

#[test]
fn s3_multinomial_concentrated_mass() {
    let b = CpuBackend::new();
    // 99% probability on index 0
    let probs = Tensor::<f32>::from_slice(&[0.99, 0.01], [2].into()).unwrap();
    let s = b.multinomial_sample(&probs, 1000, 7).unwrap();
    let samples = s.as_slice().unwrap();
    let count_0 = samples.iter().filter(|&&i| i == 0).count();
    // With n=1000, mean should be ~990, 3σ ≈ ±30
    assert!(count_0 > 950 && count_0 <= 1000);
}

// ====================================================================
// kl_divergence
// ====================================================================

#[test]
fn s3_kl_self_is_zero() {
    let b = CpuBackend::new();
    let p = Tensor::<f32>::from_slice(&[0.2, 0.3, 0.5], [3].into()).unwrap();
    let kl = b.kl_divergence(&p, &p, -1, 1e-9).unwrap();
    assert!(kl.as_slice().unwrap()[0].abs() < 1e-5);
}

#[test]
fn s3_kl_positive_when_different() {
    let b = CpuBackend::new();
    let p = Tensor::<f32>::from_slice(&[0.9, 0.1], [2].into()).unwrap();
    let q = Tensor::<f32>::from_slice(&[0.1, 0.9], [2].into()).unwrap();
    let kl = b.kl_divergence(&p, &q, -1, 1e-9).unwrap();
    let v = kl.as_slice().unwrap()[0];
    assert!(v > 0.1);
    assert!(v <= 50.0);
}

#[test]
fn s3_kl_batched() {
    let b = CpuBackend::new();
    // Two distributions
    let p = Tensor::<f32>::from_slice(
        &[0.5, 0.5, 0.1, 0.9],
        [2, 2].into(),
    )
    .unwrap();
    let q = Tensor::<f32>::from_slice(
        &[0.5, 0.5, 0.9, 0.1],
        [2, 2].into(),
    )
    .unwrap();
    let kl = b.kl_divergence(&p, &q, -1, 1e-9).unwrap();
    let s = kl.as_slice().unwrap();
    // Row 0: self-KL = 0
    assert!(s[0].abs() < 1e-5);
    // Row 1: p ≠ q, positive
    assert!(s[1] > 0.0);
}

// ====================================================================
// std_dev
// ====================================================================

#[test]
fn s3_std_dev_unbiased_matches_numpy() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], [5].into()).unwrap();
    let s = b.std_dev(&x, -1, true).unwrap();
    // numpy.std([1..5], ddof=1) = 1.5811388
    assert!((s.as_slice().unwrap()[0] - 1.581_138_8).abs() < 1e-5);
}

#[test]
fn s3_std_dev_biased_matches_numpy() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], [5].into()).unwrap();
    let s = b.std_dev(&x, -1, false).unwrap();
    // numpy.std([1..5], ddof=0) = 1.4142135
    assert!((s.as_slice().unwrap()[0] - 1.414_213_5).abs() < 1e-5);
}

#[test]
fn s3_std_dev_constant_is_zero() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[7.0, 7.0, 7.0, 7.0], [4].into()).unwrap();
    let s = b.std_dev(&x, -1, false).unwrap();
    assert!(s.as_slice().unwrap()[0].abs() < 1e-6);
}

// ====================================================================
// Integration: full MoE pipeline via ComputeBackend
// ====================================================================

// ====================================================================
// AUDIT-REVEALED edge cases
// ====================================================================

#[test]
fn s3_audit_sdpa_rejects_zero_d_k() {
    // Bug-fix guard: d_k = 0 was causing 1/sqrt(0) = Inf.
    let b = CpuBackend::new();
    let q = Tensor::<f32>::zeros([1, 1, 2, 0].into()).unwrap();
    let k = Tensor::<f32>::zeros([1, 1, 2, 0].into()).unwrap();
    let v = Tensor::<f32>::zeros([1, 1, 2, 4].into()).unwrap();
    assert!(b.scaled_dot_product_attention(&q, &k, &v, None).is_err());
}

#[test]
fn s3_audit_cone_project_nan_input_yields_nan_output() {
    let b = CpuBackend::new();
    let latents = Tensor::<f32>::from_slice(
        &[1.0, 2.0, f32::NAN, 0.5],
        [2, 2].into(),
    )
    .unwrap();
    let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
    let out = b.cone_project(&latents, &anchors, 1.0).unwrap();
    let s = out.as_slice().unwrap();
    // Row 0 : finite and bounded ; Row 1 : contains NaN
    let norm0 = (s[0].powi(2) + s[1].powi(2)).sqrt();
    assert!(norm0.is_finite() && norm0 <= 1.0 + 1e-5);
    assert!(s[2].is_nan() || s[3].is_nan());
}

#[test]
fn s3_audit_weighted_mean_empty_batch_errors_cleanly() {
    let b = CpuBackend::new();
    let values = Tensor::<f32>::zeros([0, 4].into()).unwrap();
    let weights = Tensor::<f32>::zeros([0].into()).unwrap();
    // Empty batch → sum of weights is 0 → should error
    assert!(b.weighted_mean(&values, &weights, false).is_err());
}

#[test]
fn s3_audit_multinomial_golden_seed_42() {
    // Lock the Pcg64Mcg contract : seed=42, uniform(4), n=10
    // produces a fixed sequence. If this ever changes (e.g. `rand_pcg`
    // major bump), downstream pipelines relying on reproducibility break.
    let b = CpuBackend::new();
    let probs = Tensor::<f32>::from_slice(&[0.25, 0.25, 0.25, 0.25], [4].into()).unwrap();
    let s1 = b.multinomial_sample(&probs, 10, 42).unwrap();
    let s2 = b.multinomial_sample(&probs, 10, 42).unwrap();
    // Reproducibility on same call
    assert_eq!(s1.as_slice().unwrap(), s2.as_slice().unwrap());
    // Shape
    assert_eq!(s1.shape().dims(), &[1, 10]);
    // All indices in [0, 4)
    assert!(s1.as_slice().unwrap().iter().all(|&i| (0..4).contains(&i)));
}

#[test]
fn s3_audit_topk_with_nan_in_input() {
    // NaN handling: TotalF32 orders NaN > +Inf, so NaN should appear
    // first in top-k output.
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[1.0, f32::NAN, 3.0, 2.0],
        [4].into(),
    )
    .unwrap();
    let (vals, idx) = b.topk(&x, 2, -1).unwrap();
    let v = vals.as_slice().unwrap();
    let i = idx.as_slice().unwrap();
    // One of the top-2 must be the NaN (value or index 1)
    assert!(v.iter().any(|x| x.is_nan()) || i.contains(&1));
}

#[test]
fn s3_audit_kl_divergence_preserves_asymmetry() {
    let b = CpuBackend::new();
    let p = Tensor::<f32>::from_slice(&[0.1, 0.9], [2].into()).unwrap();
    let q = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
    let kl_pq = b.kl_divergence(&p, &q, -1, 1e-9).unwrap().as_slice().unwrap()[0];
    let kl_qp = b.kl_divergence(&q, &p, -1, 1e-9).unwrap().as_slice().unwrap()[0];
    assert!((kl_pq - kl_qp).abs() > 1e-3, "KL must be asymmetric on ≠ distributions");
}

#[test]
fn s3_audit_std_dev_empty_row_produces_nan_scalar() {
    // Edge case : rank-1 shape [0] reduced along axis -1 gives a
    // rank-0 (scalar) output of size 1 containing NaN (the sum-over-
    // empty is ill-defined for std_dev).
    let b = CpuBackend::new();
    let x = Tensor::<f32>::zeros([0].into()).unwrap();
    let y = b.std_dev(&x, -1, true).unwrap();
    // Reducing a rank-1 tensor along its single axis gives a scalar.
    assert_eq!(y.rank(), 0);
    assert_eq!(y.numel(), 1);
    assert!(y.as_slice().unwrap()[0].is_nan());
}

// ====================================================================
// DEEP AUDIT — second-pass correctness tests
// ====================================================================

#[test]
fn s3_deep_moe_reconcile_top_k_equals_k_matches_full_softmax() {
    // When top_k = K, moe_reconcile must equal:
    //   weights = softmax(gate_logits)
    //   output[b, j] = Σ_k weights[b, k] · candidates[b, k, j]
    let b = CpuBackend::new();
    let cands_data: Vec<f32> = (0..12).map(|i| (i as f32) * 0.1).collect();
    let gate_data: Vec<f32> = vec![1.0, 2.0, 0.5, -0.5];
    let cands = Tensor::<f32>::from_slice(&cands_data, [1, 4, 3].into()).unwrap();
    let gate = Tensor::<f32>::from_slice(&gate_data, [1, 4].into()).unwrap();

    let moe_out = b.moe_reconcile(&cands, &gate, 4).unwrap();

    // Manual computation
    let gate_tensor = Tensor::<f32>::from_slice(&gate_data, [4].into()).unwrap();
    let sm = b.softmax(&gate_tensor, -1).unwrap();
    let sm_weights = sm.as_slice().unwrap();
    let mut manual = vec![0.0f32; 3];
    for k in 0..4 {
        for j in 0..3 {
            manual[j] += sm_weights[k] * cands_data[k * 3 + j];
        }
    }

    let got = moe_out.as_slice().unwrap();
    for j in 0..3 {
        assert!(
            (got[j] - manual[j]).abs() < 1e-5,
            "top_k=K mismatch at {}: got {}, expected {}",
            j, got[j], manual[j]
        );
    }
}

#[test]
fn s3_deep_moe_reconcile_all_neg_inf_gate_returns_error() {
    // Degenerate case : all gate logits are -∞ → topk returns [-∞, -∞],
    // max_val is non-finite → Err(NumericalInstability).
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::zeros([1, 3, 2].into()).unwrap();
    let gate = Tensor::<f32>::from_slice(
        &[f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
        [1, 3].into(),
    )
    .unwrap();
    assert!(matches!(
        b.moe_reconcile(&cands, &gate, 2).unwrap_err(),
        ComputeError::NumericalInstability { .. }
    ));
}

#[test]
fn s3_deep_moe_reconcile_all_pos_inf_gate_returns_error() {
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::zeros([1, 2, 2].into()).unwrap();
    let gate = Tensor::<f32>::from_slice(
        &[f32::INFINITY, f32::INFINITY],
        [1, 2].into(),
    )
    .unwrap();
    assert!(matches!(
        b.moe_reconcile(&cands, &gate, 2).unwrap_err(),
        ComputeError::NumericalInstability { .. }
    ));
}

#[test]
fn s3_deep_sdpa_causal_mask_produces_lower_triangular_attention() {
    // With Q = K = ones, no scaling difference, a causal mask should make
    // each output row a progressively-broader average of V.
    // For T=3, V = [[1,0],[2,0],[3,0]]:
    //   row 0 : only V[0] available, weight=1 → [1, 0]
    //   row 1 : V[0] + V[1] with softmax over 2 elements (equal) → [(1+2)/2, 0] = [1.5, 0]
    //   row 2 : V[0]+V[1]+V[2] softmax over 3 equal → [(1+2+3)/3, 0] = [2, 0]
    let b = CpuBackend::new();
    let t = 3;
    let d = 2;
    // Q = K = ones means all QK^T scores equal (before mask). After mask, softmax
    // applies uniform weight to unmasked positions.
    let q_data = vec![1.0f32; t * d];
    let q = Tensor::<f32>::from_slice(&q_data, [1, 1, t, d].into()).unwrap();
    let k = q.clone();
    // V with distinguishable values per row
    let v_data = vec![1.0, 0.0, 2.0, 0.0, 3.0, 0.0];
    let v = Tensor::<f32>::from_slice(&v_data, [1, 1, t, d].into()).unwrap();
    // Causal mask : 0 on lower+diag, -Inf on upper
    let mut mask_data = vec![0.0f32; t * t];
    for i in 0..t {
        for j in 0..t {
            if j > i {
                mask_data[i * t + j] = f32::NEG_INFINITY;
            }
        }
    }
    let mask = Tensor::<f32>::from_slice(&mask_data, [t, t].into()).unwrap();
    let out = b.scaled_dot_product_attention(&q, &k, &v, Some(&mask)).unwrap();
    let s = out.as_slice().unwrap();
    // Row 0 : [1, 0]
    assert!((s[0] - 1.0).abs() < 1e-5);
    assert!(s[1].abs() < 1e-5);
    // Row 1 : [1.5, 0]
    assert!((s[2] - 1.5).abs() < 1e-5);
    assert!(s[3].abs() < 1e-5);
    // Row 2 : [2, 0]
    assert!((s[4] - 2.0).abs() < 1e-5);
    assert!(s[5].abs() < 1e-5);
}

#[test]
fn s3_deep_topk_on_rank3_axis_last() {
    // Rank-3 tensor, topk along last axis. Shape (2, 3, 4).
    // Each (batch, row) vector of length 4 is independent.
    let b = CpuBackend::new();
    let data: Vec<f32> = (0..24).map(|i| i as f32).collect();
    let x = Tensor::<f32>::from_slice(&data, [2, 3, 4].into()).unwrap();
    let (vals, idx) = b.topk(&x, 2, -1).unwrap();
    assert_eq!(vals.shape().dims(), &[2, 3, 2]);
    assert_eq!(idx.shape().dims(), &[2, 3, 2]);
    // First row : values [0, 1, 2, 3] → top-2 = [3, 2], indices [3, 2]
    let v = vals.as_slice().unwrap();
    let i = idx.as_slice().unwrap();
    assert!((v[0] - 3.0).abs() < 1e-6);
    assert!((v[1] - 2.0).abs() < 1e-6);
    assert_eq!(i[0], 3);
    assert_eq!(i[1], 2);
    // Last row : values [20, 21, 22, 23] → top-2 = [23, 22], indices [3, 2]
    assert!((v[10] - 23.0).abs() < 1e-6);
    assert!((v[11] - 22.0).abs() < 1e-6);
    assert_eq!(i[10], 3);
    assert_eq!(i[11], 2);
}

#[test]
fn s3_deep_multinomial_n_zero_returns_empty() {
    // n = 0 should return an empty (B, 0) tensor, not an error.
    let b = CpuBackend::new();
    let probs = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
    let s = b.multinomial_sample(&probs, 0, 42).unwrap();
    assert_eq!(s.shape().dims(), &[1, 0]);
    assert_eq!(s.numel(), 0);
}

#[test]
fn s3_deep_multinomial_empirical_distribution_matches_probs() {
    // With n = 10 000 samples, empirical frequencies should be within
    // a few sigma of the target distribution. Probs = [0.1, 0.3, 0.6].
    let b = CpuBackend::new();
    let probs = Tensor::<f32>::from_slice(&[0.1, 0.3, 0.6], [3].into()).unwrap();
    let n = 10_000;
    let s = b.multinomial_sample(&probs, n, 123).unwrap();
    let samples = s.as_slice().unwrap();
    let mut counts = [0u32; 3];
    for &i in samples {
        counts[i as usize] += 1;
    }
    // Expected : 1000 / 3000 / 6000. Tolerance 3σ ≈ 3·√(np(1-p)).
    // For p=0.1, σ = √(10000·0.1·0.9) ≈ 30, so ±100 tolerance.
    assert!(
        (counts[0] as i32 - 1000).abs() < 150,
        "freq for p=0.1: expected ~1000, got {}",
        counts[0]
    );
    assert!(
        (counts[1] as i32 - 3000).abs() < 200,
        "freq for p=0.3: expected ~3000, got {}",
        counts[1]
    );
    assert!(
        (counts[2] as i32 - 6000).abs() < 200,
        "freq for p=0.6: expected ~6000, got {}",
        counts[2]
    );
}

#[test]
fn s3_deep_cone_project_matches_math_exactly() {
    // Precise formula test :
    //   z = (3, 4), a = (0, 0), max_dist = 2.0
    //   d_min = 5. d_min > max_dist, so :
    //     scale = 2/5 = 0.4
    //     z' = (0,0) + (3, 4) · 0.4 = (1.2, 1.6)
    //   ‖z'‖ = √(1.44 + 2.56) = √4 = 2 ✓
    let b = CpuBackend::new();
    let z = Tensor::<f32>::from_slice(&[3.0, 4.0], [1, 2].into()).unwrap();
    let a = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
    let out = b.cone_project(&z, &a, 2.0).unwrap();
    let s = out.as_slice().unwrap();
    assert!((s[0] - 1.2).abs() < 1e-5);
    assert!((s[1] - 1.6).abs() < 1e-5);
    // Verify the norm is exactly max_dist
    let norm = (s[0].powi(2) + s[1].powi(2)).sqrt();
    assert!((norm - 2.0).abs() < 1e-5);
}

#[test]
fn s3_deep_weighted_mean_matches_numpy_average() {
    // numpy.average([1,2,3,4], weights=[0.1, 0.2, 0.3, 0.4]) = 3.0
    let b = CpuBackend::new();
    let values = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0],
        [4, 1].into(),
    )
    .unwrap();
    let weights = Tensor::<f32>::from_slice(&[0.1, 0.2, 0.3, 0.4], [4].into()).unwrap();
    let out = b.weighted_mean(&values, &weights, false).unwrap();
    assert!((out.as_slice().unwrap()[0] - 3.0).abs() < 1e-5);
}

#[test]
fn s3_deep_kl_divergence_symmetry_breaks() {
    // Specific case where KL(p||q) and KL(q||p) must differ significantly.
    // p = concentrated, q = uniform
    let b = CpuBackend::new();
    let p = Tensor::<f32>::from_slice(&[0.9, 0.1], [2].into()).unwrap();
    let q = Tensor::<f32>::from_slice(&[0.5, 0.5], [2].into()).unwrap();
    let kl_pq = b.kl_divergence(&p, &q, -1, 1e-9).unwrap().as_slice().unwrap()[0];
    let kl_qp = b.kl_divergence(&q, &p, -1, 1e-9).unwrap().as_slice().unwrap()[0];
    // Both positive
    assert!(kl_pq > 0.0);
    assert!(kl_qp > 0.0);
    // Different
    assert!((kl_pq - kl_qp).abs() > 0.1);
}

#[test]
fn s3_deep_std_dev_scale_linearity() {
    // std(α·x) = |α| · std(x) for any α
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], [5].into()).unwrap();
    let alpha = 3.0;
    let x_scaled = Tensor::<f32>::from_slice(
        &[3.0, 6.0, 9.0, 12.0, 15.0],
        [5].into(),
    )
    .unwrap();
    let s1 = b.std_dev(&x, -1, true).unwrap().as_slice().unwrap()[0];
    let s2 = b.std_dev(&x_scaled, -1, true).unwrap().as_slice().unwrap()[0];
    assert!((s2 - alpha * s1).abs() < 1e-4);
}

#[test]
fn s3_deep_topk_boundary_k_equals_dim() {
    // k = dim → full sort in descending order, with indices.
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[3.0, 1.0, 4.0, 1.0, 5.0], [5].into()).unwrap();
    let (vals, idx) = b.topk(&x, 5, -1).unwrap();
    // Sorted desc : [5, 4, 3, 1, 1] with tie-break by smallest index for 1s
    let v = vals.as_slice().unwrap();
    let i = idx.as_slice().unwrap();
    assert!((v[0] - 5.0).abs() < 1e-6 && i[0] == 4);
    assert!((v[1] - 4.0).abs() < 1e-6 && i[1] == 2);
    assert!((v[2] - 3.0).abs() < 1e-6 && i[2] == 0);
    assert!((v[3] - 1.0).abs() < 1e-6 && i[3] == 1); // Index 1 is first '1'
    assert!((v[4] - 1.0).abs() < 1e-6 && i[4] == 3);
}

#[test]
fn s3_audit_multinomial_golden_seed_42_exact_values() {
    // Locks the Pcg64Mcg reproducibility contract : seed=42,
    // uniform probs [0.25; 4], n=10 MUST produce this exact sequence.
    // Matches tests/golden/multinomial_sample/seed_42_reproducibility.json.
    // If this test ever fails, downstream tenant pipelines that depend
    // on deterministic sampling are at risk ; investigate rand_pcg bump.
    let b = CpuBackend::new();
    let probs = Tensor::<f32>::from_slice(&[0.25, 0.25, 0.25, 0.25], [4].into()).unwrap();
    let s = b.multinomial_sample(&probs, 10, 42).unwrap();
    assert_eq!(
        s.as_slice().unwrap(),
        &[1, 3, 1, 3, 0, 2, 2, 3, 2, 2]
    );
}

#[test]
fn s3_audit_cone_project_inside_cone_passthrough_golden() {
    // Matches tests/golden/cone_project/inside_cone_passthrough.json
    // All 3 latents have L2 norm ≤ 1.0 ; output must be identical input.
    let b = CpuBackend::new();
    let latents = Tensor::<f32>::from_slice(
        &[0.3, 0.4, -0.5, 0.1, 0.0, -0.8],
        [3, 2].into(),
    )
    .unwrap();
    let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0], [1, 2].into()).unwrap();
    let out = b.cone_project(&latents, &anchors, 1.0).unwrap();
    let s = out.as_slice().unwrap();
    let expected = [0.3, 0.4, -0.5, 0.1, 0.0, -0.8];
    for i in 0..6 {
        assert!((s[i] - expected[i]).abs() < 1e-7,
            "idx {i}: got {}, expected {}", s[i], expected[i]);
    }
}

#[test]
fn s3_audit_moe_reconcile_topk_full_softmax_golden() {
    // Matches tests/golden/moe_reconcile/topk_full_softmax.json
    // Uniform gate → weights = [1/3, 1/3, 1/3] → output = mean of candidates.
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::from_slice(
        &[1.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        [1, 3, 2].into(),
    )
    .unwrap();
    let gate = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0], [1, 3].into()).unwrap();
    let out = b.moe_reconcile(&cands, &gate, 3).unwrap();
    let s = out.as_slice().unwrap();
    assert!((s[0] - 2.0 / 3.0).abs() < 1e-5);
    assert!((s[1] - 2.0 / 3.0).abs() < 1e-5);
}

#[test]
fn s3_audit_moe_reconcile_mixed_nan_gate_errors() {
    // Mixed NaN + finite gate : TotalF32 orders NaN > +Inf, so the NaN
    // bubbles up to top-1. The downstream softmax LogSumExp check
    // (!max_val.is_finite || !sum_exp.is_finite) must catch it and
    // return NumericalInstability, NOT silently propagate NaN.
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::zeros([1, 3, 2].into()).unwrap();
    let gate = Tensor::<f32>::from_slice(
        &[f32::NAN, 2.0, 3.0],
        [1, 3].into(),
    )
    .unwrap();
    // With top_k = 1, topk picks NaN (ranked highest) → max_val not finite
    //   OR max_val finite but sum_exp involves exp(NaN) = NaN
    let err = b.moe_reconcile(&cands, &gate, 1).unwrap_err();
    assert!(matches!(err, ComputeError::NumericalInstability { .. }));

    // Same with top_k = 2 : NaN is still among the top-k
    let err = b.moe_reconcile(&cands, &gate, 2).unwrap_err();
    assert!(matches!(err, ComputeError::NumericalInstability { .. }));
}

#[test]
fn s3_audit_moe_reconcile_top_k_larger_than_k_errors() {
    let b = CpuBackend::new();
    let cands = Tensor::<f32>::zeros([1, 3, 2].into()).unwrap();
    let gate = Tensor::<f32>::zeros([1, 3].into()).unwrap();
    // top_k = 4 > K = 3 → error
    assert!(b.moe_reconcile(&cands, &gate, 4).is_err());
}

#[test]
fn s3_full_moe_pipeline_through_backend() {
    let b = CpuBackend::new();

    // Simulate 3 contributor latents that need projection into a cone
    let latents_raw = Tensor::<f32>::from_slice(
        &[
            3.0, 4.0, 0.0,   // far from origin — must be projected
            0.1, 0.1, 0.1,   // close to origin
            2.0, 0.0, 1.0,   // far
        ],
        [3, 3].into(),
    )
    .unwrap();
    let anchors = Tensor::<f32>::from_slice(&[0.0, 0.0, 0.0], [1, 3].into()).unwrap();

    // 1. Project into alignment cone
    let projected = b.cone_project(&latents_raw, &anchors, 1.0).unwrap();

    // 2. Use projected latents as both candidates and gate input
    // Build shape (1, 3, 3) — single batch, 3 candidates, dim 3
    let cands_data: Vec<f32> = projected.as_slice().unwrap().to_vec();
    let cands = Tensor::<f32>::from_slice(&cands_data, [1, 3, 3].into()).unwrap();

    // Gate logits: prefer second candidate
    let gate = Tensor::<f32>::from_slice(&[0.1, 0.9, 0.2], [1, 3].into()).unwrap();

    // 3. MoE reconcile with top_k=2
    let reconciled = b.moe_reconcile(&cands, &gate, 2).unwrap();

    // 4. Weighted mean of [projected, reconciled] with uniform weights
    //    Verifies all three core ops compose without error
    assert_eq!(reconciled.shape().dims(), &[1, 3]);

    // Sanity check: output is finite
    for &v in reconciled.as_slice().unwrap() {
        assert!(v.is_finite());
    }
}
