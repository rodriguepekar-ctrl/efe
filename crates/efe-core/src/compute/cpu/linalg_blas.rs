//! Sprint 4 Light — BLAS-backed matrix multiplication.
//!
//! This module is **feature-gated** : it is only compiled when one of
//! the BLAS-provider features is enabled :
//!
//! * `accelerate` — Apple Accelerate framework (macOS)
//! * `openblas`  — OpenBLAS (Linux)
//!
//! When the feature is active, `matmul_blas` dispatches to the highly
//! optimised `sgemm` kernel exposed by `ndarray` through its `blas`
//! feature. On typical server CPUs the speedup over the naïve `ikj`
//! triple loop is ~40× at size 256×256×256.
//!
//! # Numerical contract
//!
//! BLAS uses tile-based blocking for cache locality, which reorders
//! the inner reductions relative to a plain `ikj` loop. The result
//! differs by at most :
//!
//! * `rtol = 1e-4` (relative tolerance)
//! * `atol = 1e-5` (absolute tolerance)
//!
//! These bounds are documented in `docs/compute/tolerances.md` and
//! enforced by `tests/compute_sprint4_cross_backend.rs`.
//!
//! # Dispatch threshold
//!
//! For very small matmuls the BLAS call overhead (~20-50 µs) dwarfs
//! the arithmetic savings. The `matmul` public entry-point guards the
//! dispatch with `M·K·N ≥ 32 768`, which is the empirical break-even
//! point on Apple M1 Pro.

#![cfg(any(feature = "accelerate", feature = "openblas"))]

// Force the linker to pull `blas-src`'s build-script link directives
// (e.g. `cargo:rustc-link-lib=framework=Accelerate` on macOS). Without
// an explicit symbol reference Rust drops the crate's link flags,
// leaving `cblas_sgemm` unresolved at link time.
#[allow(unused_imports)]
use blas_src as _;

use ndarray::Array2;

use crate::compute::error::{ComputeError, Result};
use crate::compute::tensor::Tensor;

/// Dispatch threshold (`M · K · N` product) above which BLAS pays.
///
/// Below this value the naïve `ikj` loop is faster due to sgemm's
/// fixed call overhead. The constant is public to `super` so the
/// dispatching code in `linalg.rs` can read the same value.
pub(super) const BLAS_DISPATCH_THRESHOLD: usize = 32_768;

/// BLAS-accelerated matrix multiplication.
///
/// Callers must have already validated the shapes via
/// `shared::validate_matmul_shapes` (the `(m, k, n)` triple is passed
/// in to avoid re-validating).
///
/// # Contract
///
/// * `a.shape() == (m, k)`, contiguous C-order
/// * `b.shape() == (k, n)`, contiguous C-order
/// * `m * k * n ≥ BLAS_DISPATCH_THRESHOLD` (not enforced here; the
///   dispatcher in `linalg.rs` is responsible)
///
/// # Returns
///
/// A fresh tensor of shape `(m, n)` with fresh storage (ref-count = 1).
pub(super) fn matmul_blas(
    a: &Tensor<f32>,
    b: &Tensor<f32>,
    m: usize,
    k: usize,
    n: usize,
) -> Result<Tensor<f32>> {
    let a_slice = a.as_slice()?;
    let b_slice = b.as_slice()?;

    // Build ndarray views over the existing buffers. `from_shape_vec`
    // takes ownership, so we must `to_vec` — an O(m·k) + O(k·n) copy
    // that is dwarfed by the sgemm savings at our threshold (~20-40×).
    //
    // A future optimisation would use `ArrayView::from_shape` to avoid
    // the copy, but it creates lifetime gymnastics with the `.dot()`
    // result. Sprint 4 Light stays with this simpler approach.
    let a_arr = Array2::from_shape_vec((m, k), a_slice.to_vec()).map_err(|e| {
        ComputeError::Internal {
            detail: format!("matmul_blas: a Array2 from_shape_vec failed : {e}"),
        }
    })?;
    let b_arr = Array2::from_shape_vec((k, n), b_slice.to_vec()).map_err(|e| {
        ComputeError::Internal {
            detail: format!("matmul_blas: b Array2 from_shape_vec failed : {e}"),
        }
    })?;

    // `Array2::dot` with `ndarray/blas` feature active dispatches to
    // `cblas_sgemm` via `cblas-sys`. Without the feature, this would
    // fall back to ndarray's own matmul — but we only compile this
    // module when `accelerate` or `openblas` is on.
    let c_arr = a_arr.dot(&b_arr);

    let c_slice = c_arr.as_slice().ok_or(ComputeError::Internal {
        detail: "matmul_blas: result array not contiguous (unexpected)".into(),
    })?;

    Tensor::from_slice(c_slice, [m, n].into())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn blas_matmul_2x2_known_values() {
        // [[1,2],[3,4]] @ [[5,6],[7,8]] = [[19,22],[43,50]]
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [2, 2].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&[5.0, 6.0, 7.0, 8.0], [2, 2].into()).unwrap();
        let c = matmul_blas(&a, &b, 2, 2, 2).unwrap();
        assert_eq!(c.shape().dims(), &[2, 2]);
        let s = c.as_slice().unwrap();
        assert!((s[0] - 19.0).abs() < 1e-5);
        assert!((s[1] - 22.0).abs() < 1e-5);
        assert!((s[2] - 43.0).abs() < 1e-5);
        assert!((s[3] - 50.0).abs() < 1e-5);
    }

    #[test]
    fn blas_matmul_identity_preserves() {
        // 3×3 identity times random 3×4 = same 3×4
        let id: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let a = Tensor::<f32>::from_slice(&id, [3, 3].into()).unwrap();
        let b_data: Vec<f32> = (0..12).map(|i| i as f32 * 0.5 + 0.3).collect();
        let b = Tensor::<f32>::from_slice(&b_data, [3, 4].into()).unwrap();
        let c = matmul_blas(&a, &b, 3, 3, 4).unwrap();
        for i in 0..12 {
            assert!((c.as_slice().unwrap()[i] - b_data[i]).abs() < 1e-5);
        }
    }

    #[test]
    fn blas_matmul_rectangular() {
        // A: (3, 4), B: (4, 2) → C: (3, 2)
        let a_data: Vec<f32> = (0..12).map(|i| i as f32).collect();
        let b_data: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let a = Tensor::<f32>::from_slice(&a_data, [3, 4].into()).unwrap();
        let b = Tensor::<f32>::from_slice(&b_data, [4, 2].into()).unwrap();
        let c = matmul_blas(&a, &b, 3, 4, 2).unwrap();
        // Expected (computed manually / checked with numpy):
        //   row 0: [28, 34], row 1: [76, 98], row 2: [124, 162]
        let s = c.as_slice().unwrap();
        assert!((s[0] - 28.0).abs() < 1e-4);
        assert!((s[1] - 34.0).abs() < 1e-4);
        assert!((s[2] - 76.0).abs() < 1e-4);
        assert!((s[3] - 98.0).abs() < 1e-4);
        assert!((s[4] - 124.0).abs() < 1e-4);
        assert!((s[5] - 162.0).abs() < 1e-4);
    }

    #[test]
    fn blas_matmul_output_has_fresh_storage() {
        // Verify that the returned tensor has a brand-new Arc
        // (ref-count = 1) so that the fused epilog in `matmul_fused_gelu`
        // can successfully `as_mut_slice` without a clone.
        let a = Tensor::<f32>::zeros([4, 4].into()).unwrap();
        let b = Tensor::<f32>::zeros([4, 4].into()).unwrap();
        let mut c = matmul_blas(&a, &b, 4, 4, 4).unwrap();
        // This call succeeds only if the storage Arc is unique.
        assert!(c.as_mut_slice().is_ok());
    }
}
