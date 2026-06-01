//! Sprint 4 Light — cross-backend equivalence tests.
//!
//! Two flavours :
//!
//! * **Always-on tests** (no feature required) : guarantee the Sprint 2
//!   naïve path stays bit-identical when only the *public* `matmul`
//!   entry point is used. These catch regressions in the dispatcher.
//!
//! * **`accelerate` / `openblas`-gated tests** : compare the naïve
//!   `ikj` loop (computed locally in the test) against the BLAS path
//!   (dispatched through `ComputeBackend::matmul` when size ≥ threshold).
//!   The tolerance is the one documented in `docs/compute/tolerances.md`:
//!     rtol = 1e-4, atol = 1e-5
#![allow(clippy::float_cmp)]
#![allow(clippy::excessive_precision)]
#![allow(clippy::needless_range_loop)]

use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};

// ---------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------

/// Deterministic xorshift-based f32 RNG for reproducible random inputs.
fn xorshift_vec(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed.max(1);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32) * 2.0 - 1.0
        })
        .collect()
}

fn random_tensor(shape: &[usize], seed: u64) -> Tensor<f32> {
    let n: usize = shape.iter().product();
    Tensor::<f32>::from_slice(&xorshift_vec(n, seed), shape.to_vec().into()).unwrap()
}

/// Manual `ikj` matmul reference — single source of truth for
/// cross-backend assertions.
fn manual_ikj_matmul(a: &Tensor<f32>, b: &Tensor<f32>) -> Tensor<f32> {
    let a_dims = a.shape().dims();
    let b_dims = b.shape().dims();
    let (m, k) = (a_dims[0], a_dims[1]);
    let n = b_dims[1];
    let sa = a.as_slice().unwrap();
    let sb = b.as_slice().unwrap();
    let mut out = vec![0.0f32; m * n];
    for i in 0..m {
        for kk in 0..k {
            let aik = sa[i * k + kk];
            for j in 0..n {
                out[i * n + j] += aik * sb[kk * n + j];
            }
        }
    }
    Tensor::<f32>::from_slice(&out, [m, n].into()).unwrap()
}

/// Tolerance check : `|a - b| ≤ atol + rtol · |b|` element-wise.
fn assert_tensors_close(
    actual: &Tensor<f32>,
    expected: &Tensor<f32>,
    rtol: f32,
    atol: f32,
) {
    let sa = actual.as_slice().unwrap();
    let sb = expected.as_slice().unwrap();
    assert_eq!(sa.len(), sb.len(), "shape mismatch");
    for i in 0..sa.len() {
        let diff = (sa[i] - sb[i]).abs();
        let tol = atol + rtol * sb[i].abs();
        assert!(
            diff <= tol,
            "index {i}: |{} - {}| = {} > tol = {}",
            sa[i],
            sb[i],
            diff,
            tol
        );
    }
}

// =====================================================================
// ALWAYS-ON : no feature flag required
// =====================================================================

#[test]
fn s4_matmul_public_api_small_matches_manual_naive() {
    // Below BLAS threshold → naïve path on all configurations.
    let b = CpuBackend::new();
    let a = random_tensor(&[8, 12], 11);
    let bb = random_tensor(&[12, 6], 22);
    let c_api = b.matmul(&a, &bb).unwrap();
    let c_ref = manual_ikj_matmul(&a, &bb);
    assert_tensors_close(&c_api, &c_ref, 0.0, 0.0);
}

#[test]
fn s4_matmul_public_api_at_threshold_matches_manual_naive() {
    // 32 × 32 × 32 = 32 768 → exactly at threshold. Depending on the
    // exact comparison (≥ vs >), the dispatcher may route either way.
    // Either path must match the manual reference to the BLAS tolerance.
    let b = CpuBackend::new();
    let a = random_tensor(&[32, 32], 33);
    let bb = random_tensor(&[32, 32], 44);
    let c_api = b.matmul(&a, &bb).unwrap();
    let c_ref = manual_ikj_matmul(&a, &bb);
    assert_tensors_close(&c_api, &c_ref, 1e-4, 1e-5);
}

#[test]
fn s4_matmul_fused_gelu_fresh_storage_enables_mut_slice() {
    // Regression guard : the fused epilog relies on Arc::get_mut succeeding
    // on the tensor returned by `matmul`. If a future refactor clones C
    // between matmul and the epilog, this test catches it.
    let b = CpuBackend::new();
    let a = random_tensor(&[8, 8], 7);
    let bb = random_tensor(&[8, 8], 8);
    let bias = random_tensor(&[8], 9);
    // Just verify the op runs without panicking or returning Err(Aliased).
    let out = b.matmul_fused_gelu(&a, &bb, Some(&bias)).unwrap();
    assert_eq!(out.shape().dims(), &[8, 8]);
    for &v in out.as_slice().unwrap() {
        assert!(v.is_finite());
    }
}

#[test]
fn s4_matmul_fused_gelu_no_bias_equals_gelu_of_matmul() {
    // Cross-check : fused(no bias) == gelu(matmul(a, b))
    let b = CpuBackend::new();
    let a = random_tensor(&[16, 16], 11);
    let bb = random_tensor(&[16, 16], 22);
    let fused = b.matmul_fused_gelu(&a, &bb, None).unwrap();
    let c = b.matmul(&a, &bb).unwrap();
    let manual = b.gelu(&c).unwrap();
    assert_tensors_close(&fused, &manual, 1e-5, 1e-6);
}

#[test]
fn s4_matmul_fused_gelu_with_bias_equals_explicit_composition() {
    let b = CpuBackend::new();
    let a = random_tensor(&[12, 20], 3);
    let bb = random_tensor(&[20, 12], 5);
    let bias = random_tensor(&[12], 7);
    let fused = b
        .matmul_fused_gelu(&a, &bb, Some(&bias))
        .unwrap();
    // Manual composition
    let c = b.matmul(&a, &bb).unwrap();
    let bs = bias.as_slice().unwrap();
    let cs = c.as_slice().unwrap().to_vec();
    let mut c_with_bias = vec![0.0f32; 12 * 12];
    for i in 0..12 {
        for j in 0..12 {
            c_with_bias[i * 12 + j] = cs[i * 12 + j] + bs[j];
        }
    }
    let cb = Tensor::<f32>::from_slice(&c_with_bias, [12, 12].into()).unwrap();
    let manual = b.gelu(&cb).unwrap();
    assert_tensors_close(&fused, &manual, 1e-5, 1e-6);
}

// =====================================================================
// GATED on `accelerate` — BLAS equivalence tests
// =====================================================================

#[cfg(feature = "accelerate")]
mod blas_tests {
    use super::*;

    #[test]
    fn s4_blas_matmul_128_matches_naive() {
        // 128·128·128 = 2 097 152 ≥ threshold → BLAS path active.
        let b = CpuBackend::new();
        let a = random_tensor(&[128, 128], 11);
        let bb = random_tensor(&[128, 128], 22);
        let c_blas = b.matmul(&a, &bb).unwrap(); // dispatches to sgemm
        let c_ref = manual_ikj_matmul(&a, &bb);
        assert_tensors_close(&c_blas, &c_ref, 1e-4, 1e-5);
    }

    #[test]
    fn s4_blas_matmul_256_matches_naive() {
        let b = CpuBackend::new();
        let a = random_tensor(&[256, 256], 7);
        let bb = random_tensor(&[256, 256], 13);
        let c_blas = b.matmul(&a, &bb).unwrap();
        let c_ref = manual_ikj_matmul(&a, &bb);
        assert_tensors_close(&c_blas, &c_ref, 1e-4, 1e-5);
    }

    #[test]
    fn s4_blas_matmul_rectangular_128x64_64x96() {
        // 128 · 64 · 96 = 786 432 ≥ threshold
        let b = CpuBackend::new();
        let a = random_tensor(&[128, 64], 3);
        let bb = random_tensor(&[64, 96], 5);
        let c_blas = b.matmul(&a, &bb).unwrap();
        let c_ref = manual_ikj_matmul(&a, &bb);
        assert_eq!(c_blas.shape().dims(), &[128, 96]);
        assert_tensors_close(&c_blas, &c_ref, 1e-4, 1e-5);
    }

    #[test]
    fn s4_blas_identity_matmul_preserves_input() {
        // 64 · 64 · 64 = 262 144 ≥ threshold → BLAS path active
        let b = CpuBackend::new();
        let mut id_data = vec![0.0f32; 64 * 64];
        for i in 0..64 {
            id_data[i * 64 + i] = 1.0;
        }
        let id = Tensor::<f32>::from_slice(&id_data, [64, 64].into()).unwrap();
        let x = random_tensor(&[64, 64], 99);
        let y = b.matmul(&id, &x).unwrap();
        // Identity times X = X, should be exact or within BLAS tolerance
        assert_tensors_close(&y, &x, 1e-4, 1e-5);
    }

    #[test]
    fn s4_blas_matmul_fused_gelu_with_bias_matches_composition() {
        // BLAS path for matmul, then fused bias+GELU epilog
        let b = CpuBackend::new();
        let a = random_tensor(&[128, 64], 11);
        let bb = random_tensor(&[64, 96], 22);
        let bias = random_tensor(&[96], 33);

        let fused = b.matmul_fused_gelu(&a, &bb, Some(&bias)).unwrap();

        // Manual : BLAS matmul + manual bias + manual GELU
        let c = b.matmul(&a, &bb).unwrap();
        let bs = bias.as_slice().unwrap();
        let cs = c.as_slice().unwrap().to_vec();
        let mut cb_data = vec![0.0f32; 128 * 96];
        for i in 0..128 {
            for j in 0..96 {
                cb_data[i * 96 + j] = cs[i * 96 + j] + bs[j];
            }
        }
        let cb = Tensor::<f32>::from_slice(&cb_data, [128, 96].into()).unwrap();
        let manual = b.gelu(&cb).unwrap();

        assert_tensors_close(&fused, &manual, 1e-4, 1e-5);
    }

    /// SDPA composes two internal matmuls. Under `--features accelerate`
    /// those dispatch to BLAS for matrices ≥ threshold, so the attention
    /// output inherits the BLAS tolerance `rtol = 1e-4`. This test
    /// validates that SDPA stays correct when matmul is replaced by
    /// sgemm under the hood, by comparing its output against a locally-
    /// computed attention that uses the naïve reference.
    #[test]
    fn s4_blas_sdpa_64head_matches_manual_naive_attention() {
        use efe_core::compute::Tensor;
        let backend = CpuBackend::new();
        let t = 32_usize;
        let d = 64_usize; // 32·32·64 = 65_536 > threshold : BLAS active
        // Build randomized Q, K, V of shape (1, 1, T, d)
        let q = random_tensor(&[1, 1, t, d], 101);
        let k = random_tensor(&[1, 1, t, d], 102);
        let v = random_tensor(&[1, 1, t, d], 103);

        // Reference : compute SDPA manually using ONLY manual_ikj_matmul
        // and the existing softmax op (which BLAS does not touch).
        let q_src = q.as_slice().unwrap();
        let k_src = k.as_slice().unwrap();
        let v_src = v.as_slice().unwrap();
        let scale = 1.0f32 / (d as f32).sqrt();

        // Extract 2D tiles
        let q_2d = Tensor::<f32>::from_slice(q_src, [t, d].into()).unwrap();
        let k_2d = Tensor::<f32>::from_slice(k_src, [t, d].into()).unwrap();
        let v_2d = Tensor::<f32>::from_slice(v_src, [t, d].into()).unwrap();
        // Manual K^T
        let ks = k_2d.as_slice().unwrap();
        let mut kt = vec![0.0f32; d * t];
        for i in 0..t {
            for j in 0..d {
                kt[j * t + i] = ks[i * d + j];
            }
        }
        let k_t = Tensor::<f32>::from_slice(&kt, [d, t].into()).unwrap();
        // Manual naïve scores = Q · K^T · scale  (never touches BLAS)
        let scores = manual_ikj_matmul(&q_2d, &k_t);
        let scaled_scores: Vec<f32> =
            scores.as_slice().unwrap().iter().map(|v| v * scale).collect();
        let scores_t = Tensor::<f32>::from_slice(&scaled_scores, [t, t].into()).unwrap();
        let weights = backend.softmax(&scores_t, -1).unwrap();
        let expected_out = manual_ikj_matmul(&weights, &v_2d);

        // Actual : SDPA (which goes through BLAS matmul under accelerate)
        let sdpa_out = backend
            .scaled_dot_product_attention(&q, &k, &v, None)
            .unwrap();
        let sdpa_out_2d = Tensor::<f32>::from_slice(
            sdpa_out.as_slice().unwrap(),
            [t, d].into(),
        )
        .unwrap();

        // Tolerance : SDPA ≈ matmul · softmax · matmul — each matmul
        // introduces BLAS-level drift up to 1e-4. Composition accumulates
        // to ~2e-4 in worst case. We use 2e-4 rtol.
        assert_tensors_close(&sdpa_out_2d, &expected_out, 2e-4, 1e-5);
    }

    #[test]
    fn s4_blas_below_threshold_stays_naive() {
        // 16·16·16 = 4 096 < threshold : dispatcher must pick naïve.
        // We cannot directly observe the dispatch, but we can verify
        // bit-exact equality with the manual naïve reference.
        let b = CpuBackend::new();
        let a = random_tensor(&[16, 16], 11);
        let bb = random_tensor(&[16, 16], 22);
        let c_api = b.matmul(&a, &bb).unwrap();
        let c_ref = manual_ikj_matmul(&a, &bb);
        // Bit-exact : same reduction order → same floats.
        assert_tensors_close(&c_api, &c_ref, 0.0, 0.0);
    }
}
