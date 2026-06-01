//! Integration smoke tests for `efe-kernels` — Sprint D28.
//!
//! These tests run on **every** workspace `cargo test` (CPU CI),
//! but the GPU paths are gated behind `#[cfg(feature = "cuda")]`
//! and skipped when no GPU runner is available. The CI matrix
//! `test-gpu-smoke` enables `--features cuda` to actually exercise
//! the device.

use efe_kernels::{is_cuda_available, version};

#[test]
fn version_is_non_empty() {
    assert!(!version().is_empty());
}

#[test]
fn is_cuda_available_compile_time() {
    let want = cfg!(feature = "cuda");
    assert_eq!(is_cuda_available(), want);
}

// ─────────────────────────────────────────────────────────────────────
// GPU-only smoke : these run in `test-gpu-smoke` CI workflow only.
// ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "cuda")]
mod gpu_smoke {
    use efe_core::compute::{ComputeBackend, Shape, Tensor};
    use efe_kernels::CudaBackend;

    fn require_gpu() -> Option<CudaBackend> {
        match CudaBackend::new(0) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("[gpu_smoke] CUDA unavailable, skipping: {e}");
                None
            }
        }
    }

    #[test]
    fn cuda_backend_reports_correct_device() {
        let Some(b) = require_gpu() else { return; };
        let dev = b.device();
        assert!(matches!(
            dev,
            efe_core::compute::Device::Cuda(_)
        ));
        assert_eq!(b.name(), "cuda");
    }

    #[test]
    fn cuda_backend_softmax_smoke() {
        let Some(b) = require_gpu() else { return; };
        let x = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0],
            Shape::from_slice(&[5]),
        )
        .unwrap();
        let y = b.softmax(&x, -1).unwrap();
        let sum: f32 = y.as_slice().unwrap().iter().sum();
        assert!((sum - 1.0).abs() < 1e-4, "softmax must sum to 1, got {sum}");
    }

    #[test]
    fn cuda_backend_synchronize_no_panic() {
        let Some(b) = require_gpu() else { return; };
        b.synchronize().unwrap();
    }

    #[test]
    fn cuda_compute_capability_at_least_75() {
        let Some(b) = require_gpu() else { return; };
        // Architecture invariant : we refuse devices below sm_75.
        assert!(b.compute_capability() >= 75);
    }
}
