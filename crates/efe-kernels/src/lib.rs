//! CUDA kernel bindings for EFE — Sprint **D28 (Φ-D)**.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md`.
//!
//! # What this crate does
//!
//! `efe-kernels` provides a GPU implementation of the
//! [`efe_core::compute::ComputeBackend`] trait via the
//! [`CudaBackend`] type. The trait is the single contract between
//! algorithmic code in `efe-core` (fusion, consensus, dissolution)
//! and the underlying compute substrate (CPU or GPU).
//!
//! # Dependency direction
//!
//! This crate **depends on** `efe-core` and **NEVER the reverse**.
//! That keeps `efe-core` compilable on machines without a CUDA
//! toolkit and lets `efe-server` choose at runtime which backend
//! to wire (CPU fallback / CUDA single-GPU / multi-GPU sharded).
//!
//! # Cargo features
//!
//! * `cuda`              — enables the real `CudaBackend`. Pulls
//!                         `cudarc` and `cc` build-dep. Default
//!                         **off** so a `cargo build --workspace`
//!                         on Mac stays green.
//! * `nccl`              — enables multi-GPU collectives via NCCL
//!                         (D30 sharded fusion).
//! * `fp16`              — enables half-precision support in
//!                         downstream kernels (post-GA).
//! * `auto_check_finite` — debug-only NaN/Inf scan after every
//!                         kernel ; expensive, off in prod.
//! * `mock_gpu`          — software simulator used by CI when no
//!                         GPU runner is available.
//!
//! # Quick example
//!
//! ```no_run
//! # #[cfg(feature = "cuda")] fn _example() -> Result<(), Box<dyn std::error::Error>> {
//! use efe_kernels::CudaBackend;
//! use efe_core::compute::{ComputeBackend, Tensor, Shape};
//!
//! let backend = CudaBackend::new(0)?; // ordinal 0 = first GPU
//! let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], Shape::from_slice(&[4]))?;
//! let y = backend.softmax(&x, -1)?;
//! assert_eq!(y.numel(), 4);
//! # Ok(()) }
//! ```

#![warn(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]

pub mod error;
pub mod observability;

#[cfg(feature = "cuda")]
pub mod backend;
#[cfg(feature = "cuda")]
pub mod fallback;
#[cfg(feature = "cuda")]
pub mod kernels;
#[cfg(feature = "cuda")]
pub mod memory;
#[cfg(feature = "cuda")]
pub mod streams;
#[cfg(feature = "cuda")]
pub mod multi_gpu;

/// Sprint **Φ-D D32 (Bundle completion)** — extended GPU host
/// fingerprint pour opposabilité juridique. Compile sans CUDA
/// (les fields sont `Option<*>` et restent `None`).
pub mod fingerprint;

/// Sprint **Φ-D D32 (Bundle completion)** — auto-tune CLI + per-arch
/// tile size lookup. Compile sans CUDA.
pub mod tune;

/// Sprint **Φ-D D29 (Bundle completion)** — framework de validation
/// numerical equivalence (= compare CudaBackend outputs vs CpuBackend
/// reference). Compile sans CUDA (la partie "actually run on GPU"
/// est gatée par `#[cfg(feature = "cuda")]` dans le test).
pub mod numerical;

// ─────────────────────────────────────────────────────────────────────
// Public surface
// ─────────────────────────────────────────────────────────────────────

pub use error::CudaError;

#[cfg(feature = "cuda")]
pub use backend::CudaBackend;

#[cfg(feature = "cuda")]
pub use fallback::{detect_gpu_topology, GpuInfo, GpuTopology};

#[cfg(feature = "cuda")]
pub use multi_gpu::ShardedBackend;

// ─────────────────────────────────────────────────────────────────────
// Crate-level helpers
// ─────────────────────────────────────────────────────────────────────

/// Returns the crate version (`CARGO_PKG_VERSION` at compile time).
///
/// Used by `efe-server` to log the GPU substrate version at boot
/// alongside the matching `efe-core` version — divergence is a sign
/// of an incomplete rebuild.
#[must_use]
pub const fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// `true` iff this crate was compiled with the `cuda` feature
/// active. **Compile-time** — so callers can const-branch.
///
/// Note that this is *not* the same as "a GPU is available at
/// runtime" — for that, see [`fallback::detect_gpu_topology`].
#[must_use]
pub const fn is_cuda_available() -> bool {
    cfg!(feature = "cuda")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_non_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn is_cuda_available_matches_cfg() {
        // The two should always agree.
        let active = cfg!(feature = "cuda");
        assert_eq!(is_cuda_available(), active);
    }
}
