//! Shared test helpers for the cross-backend test suite.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 16.

#![cfg(feature = "cuda")]
#![allow(dead_code)]

use efe_kernels::CudaBackend;

/// Open the first available GPU. Returns `None` when no CUDA
/// device is available — callers print a `[gpu_skip]` line and
/// `return` early. Avoids panic in CI matrices that run on
/// machines without a GPU.
pub fn require_gpu() -> Option<CudaBackend> {
    match CudaBackend::new(0) {
        Ok(b) => Some(b),
        Err(e) => {
            eprintln!("[gpu_skip] CUDA unavailable, skipping test: {e}");
            None
        }
    }
}

/// Assert two slices are close within `(rtol, atol)`. Mirrors
/// `numpy.allclose` semantics : `|a - b| ≤ atol + rtol · |b|`.
pub fn assert_close(actual: &[f32], expected: &[f32], rtol: f32, atol: f32) {
    assert_eq!(
        actual.len(),
        expected.len(),
        "length mismatch: actual={}, expected={}",
        actual.len(),
        expected.len()
    );
    let mut max_abs = 0.0f32;
    let mut max_rel = 0.0f32;
    for (i, (&a, &b)) in actual.iter().zip(expected.iter()).enumerate() {
        let abs_diff = (a - b).abs();
        let tol = atol + rtol * b.abs();
        if abs_diff > max_abs { max_abs = abs_diff; }
        let rel = if b.abs() > 1e-12 { abs_diff / b.abs() } else { 0.0 };
        if rel > max_rel { max_rel = rel; }
        assert!(
            abs_diff <= tol,
            "mismatch at index {i}: actual={a} expected={b} \
             abs_diff={abs_diff} tol={tol} (rtol={rtol}, atol={atol})"
        );
    }
    eprintln!(
        "[cross_backend_OK] max_abs={max_abs:.3e} max_rel={max_rel:.3e} \
         (rtol={rtol}, atol={atol})"
    );
}
