//! Compute backend abstraction layer.
//!
//! This module defines a device-agnostic surface ([`ComputeBackend`]) used
//! by the EFE core algorithms (alignment, consensus, dissolution, fusion,
//! genesis) to execute numerical primitives on either CPU ([`cpu::CpuBackend`])
//! or GPU ([`cuda::CudaBackend`], behind the `cuda` feature).
//!
//! # Design invariants
//!
//! 1. **Portable first** — every operation must work on CPU before any GPU
//!    implementation is written. CPU is the correctness oracle.
//!
//! 2. **Deterministic on fixed seeds** — all sampling ops take an explicit
//!    `u64` seed. No global RNG state.
//!
//! 3. **Zero-copy views** — [`Tensor`] is a cheap handle with an `Arc`
//!    storage; slices, transposes, reshapes do not clone data.
//!
//! 4. **No panics on well-formed input** — mismatched shapes return
//!    `Err(ComputeError::ShapeMismatch)`, never panic. Panics are reserved
//!    for true internal invariant violations.
//!
//! 5. **Feature-gated backends** — `cpu` is the default and always compiled.
//!    `cuda` is opt-in; on non-CUDA hosts (e.g. macOS) the crate still builds.
//!
//! # Module layout
//!
//! ```text
//! compute/
//! ├── mod.rs          ← this file (re-exports)
//! ├── device.rs       ← Device enum
//! ├── dtype.rs        ← DType trait
//! ├── shape.rs        ← Shape, Strides
//! ├── tensor.rs       ← Tensor<T>
//! ├── error.rs        ← ComputeError, Result
//! ├── backend.rs      ← trait ComputeBackend
//! ├── cpu/            ← CpuBackend
//! └── cuda/           ← CudaBackend (feature = "cuda")
//! ```

#![allow(clippy::must_use_candidate)]
// Every `Result<_, ComputeError>`-returning function in this module can
// fail with one of the ten variants documented exhaustively in
// `error.rs`. Repeating that list as a `# Errors` doc-section on each
// of the 30+ trait methods would be pure noise, so we opt out here.
#![allow(clippy::missing_errors_doc)]

pub mod backend;
pub mod cpu;
pub mod device;
pub mod dtype;
pub mod error;
pub mod global;
pub mod shape;
pub mod tensor;
pub mod utils;

#[cfg(feature = "cuda")]
pub mod cuda;

pub use backend::ComputeBackend;
pub use cpu::CpuBackend;
pub use device::Device;
pub use dtype::DType;
pub use error::{ComputeError, Result};
pub use global::global_backend;
pub use shape::{Shape, Strides};
pub use tensor::Tensor;
