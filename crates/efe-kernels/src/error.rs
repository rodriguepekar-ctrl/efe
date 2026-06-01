//! Error taxonomy for the CUDA substrate — Sprint D28.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 22.
//!
//! `CudaError` is the **internal** error type of this crate. It is
//! converted into the public `efe_core::compute::ComputeError` at
//! the trait boundary so that callers (`efe-core`, `efe-server`)
//! see a single unified error surface regardless of which backend
//! produced it.

use efe_core::compute::ComputeError;

/// Errors raised by the CUDA substrate.
///
/// Variants are deliberately fine-grained so an operator reading
/// the logs can fix the offending input without re-reading the
/// whole architecture document. The `From<CudaError> for
/// ComputeError` impl below collapses these into the public surface.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CudaError {
    /// Wrapping of a raw CUDA driver / runtime error code.
    /// `code` matches the value returned by `cudaGetLastError` or
    /// the negative `CUresult` from the driver API.
    #[error("CUDA driver error: code {code} ({msg})")]
    FromCuda {
        /// Numeric error code as reported by CUDA.
        code: i32,
        /// Best-effort string interpretation (`cudaGetErrorString`).
        msg: String,
    },

    /// VRAM allocation failed because the request would push the
    /// total beyond the configured high-water mark (default 80 %
    /// of `cudaMemGetInfo` free).
    #[error(
        "VRAM exhausted on device {device}: requested {requested_bytes} bytes, \
         available {available_bytes}"
    )]
    OutOfMemory {
        /// Device ordinal.
        device: u32,
        /// Bytes requested.
        requested_bytes: usize,
        /// Bytes still available before the high-water mark.
        available_bytes: usize,
    },

    /// `<<<grid, block, smem, stream>>>` launch returned non-`cudaSuccess`.
    #[error("kernel `{kernel}` launch failed: {detail}")]
    LaunchFailed {
        /// Kernel name (e.g. `efe_softmax_launch`).
        kernel: &'static str,
        /// Human-readable detail.
        detail: String,
    },

    /// NCCL communication error (multi-GPU only).
    #[error("NCCL error: {0}")]
    Nccl(String),

    /// The requested device ordinal does not exist or is not
    /// available (driver missing, no GPU, …). Returned by
    /// `CudaBackend::new` when the boot fallback path triggers.
    #[error("CUDA device {0} not available")]
    DeviceUnavailable(u32),

    /// The compute capability of the detected device is below the
    /// minimum required (`sm_75`). Surfaced at boot.
    #[error(
        "device {device} compute capability sm_{got_cc} is below \
         the minimum sm_{min_cc} required by efe-kernels"
    )]
    UnsupportedCapability {
        /// Device ordinal.
        device: u32,
        /// Compute capability of the device, encoded as MAJOR*10+MINOR.
        got_cc: u32,
        /// Minimum required (`75` for SM 7.5 = Turing).
        min_cc: u32,
    },

    /// A multi-GPU run produced non-deterministic results because
    /// `NCCL_DETERMINISTIC=1` was not set. Surfaced when the
    /// determinism pin test fires.
    #[error("multi-GPU determinism violation: {0}")]
    DeterminismViolation(String),

    /// `cudaStreamCreate` / `cudaStreamDestroy` failed.
    #[error("stream error: {0}")]
    StreamError(String),

    /// Watchdog killed a kernel that exceeded its timeout budget.
    /// Typically signals a hung kernel (deadlock, infinite loop in
    /// custom code).
    #[error("kernel `{0}` exceeded watchdog timeout")]
    KernelTimeout(&'static str),

    /// Operation not yet wired in the current sprint. Replaces
    /// `ComputeError::NotImplemented` for the GPU path during the
    /// progressive D28-D32 implementation phase.
    #[error("operation `{0}` not yet wired on the CUDA backend (sprint Dxx)")]
    NotYetWired(&'static str),

    /// Generic driver init failure (cudaSetDevice, cudaGetDeviceProperties).
    #[error("CUDA initialization failed: {0}")]
    InitFailed(String),
}

impl CudaError {
    /// Construct a `FromCuda` from a raw error code, looking up the
    /// human-readable message via `cudaGetErrorString` when available.
    ///
    /// In a stub build (no `cuda` feature), the message is a generic
    /// `"unknown CUDA error"` since we cannot call into the runtime.
    #[must_use]
    pub fn from_cuda_code(code: i32) -> Self {
        let msg = lookup_cuda_message(code);
        Self::FromCuda { code, msg }
    }

    /// Bump the `efe_gpu_kernel_calls_total{outcome="fail"}` counter
    /// at construction. Convenience for paths that always observe
    /// the error.
    #[must_use]
    pub fn with_observability(self, kernel: &'static str) -> Self {
        crate::observability::record_kernel_outcome(kernel, "fail");
        self
    }
}

/// Lookup helper — converts a raw CUDA error code into a string.
///
/// **Audit fix K-A4** — pre-fix this function did
/// `unsafe { transmute::<i32, cudaError>(code) }` which is undefined
/// behaviour when `code` is not a valid `cudaError` discriminant
/// (e.g. when the wire surface returns an unknown future error
/// code). The current implementation calls `cudaGetErrorString`
/// directly via a `extern "C"` declaration that takes a raw `c_int`
/// — no transmute, no enum coercion. If the code is unknown the
/// driver returns the literal string `"unrecognized error code"`,
/// which we forward verbatim. Cf. CUDA Runtime API doc.
#[cfg(feature = "cuda")]
fn lookup_cuda_message(code: i32) -> String {
    // The C signature is `const char* cudaGetErrorString(cudaError_t)`.
    // `cudaError_t` is defined as a C enum which on every supported
    // platform is ABI-equivalent to `int` (= `c_int`). Calling the
    // function with a plain `c_int` is therefore well-defined and
    // avoids the UB that `transmute` to a Rust `enum` would risk.
    extern "C" {
        fn cudaGetErrorString(error: std::os::raw::c_int)
            -> *const std::os::raw::c_char;
    }
    // SAFETY: the documented API never returns NULL ; the returned
    // pointer references static driver memory and is safe to read
    // without a length prefix (it is a NUL-terminated C string).
    unsafe {
        let raw = cudaGetErrorString(code as std::os::raw::c_int);
        if raw.is_null() {
            "unknown CUDA error".to_string()
        } else {
            std::ffi::CStr::from_ptr(raw)
                .to_string_lossy()
                .into_owned()
        }
    }
}

#[cfg(not(feature = "cuda"))]
fn lookup_cuda_message(_code: i32) -> String {
    "CUDA toolkit absent (cuda feature off)".to_string()
}

// ─────────────────────────────────────────────────────────────────────
// Conversions to the public `ComputeError` surface
// ─────────────────────────────────────────────────────────────────────

impl From<CudaError> for ComputeError {
    fn from(e: CudaError) -> Self {
        // The architecture doc § 22.2 documents this mapping.
        // `ComputeError` does not currently carry `ResourceExhausted`
        // / `BackendUnavailable` variants — we route through the
        // generic ones with a structured prefix so log greppers
        // can still distinguish the families.
        match e {
            CudaError::OutOfMemory { device, requested_bytes, available_bytes } => {
                ComputeError::NumericalInstability {
                    op: "cuda.alloc",
                    detail: format!(
                        "VRAM exhausted on device {device}: requested {requested_bytes}, \
                         available {available_bytes}"
                    ),
                }
            }
            CudaError::DeviceUnavailable(d) => {
                ComputeError::NumericalInstability {
                    op: "cuda.boot",
                    detail: format!("device {d} not available"),
                }
            }
            CudaError::UnsupportedCapability { device, got_cc, min_cc } => {
                ComputeError::NumericalInstability {
                    op: "cuda.boot",
                    detail: format!(
                        "device {device} sm_{got_cc} below minimum sm_{min_cc}"
                    ),
                }
            }
            CudaError::NotYetWired(op) => {
                ComputeError::NumericalInstability {
                    op: "cuda.not_wired",
                    detail: op.to_string(),
                }
            }
            other => {
                ComputeError::NumericalInstability {
                    op: "cuda",
                    detail: other.to_string(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_cuda_code_carries_message() {
        let e = CudaError::from_cuda_code(2); // cudaErrorMemoryAllocation
        match e {
            CudaError::FromCuda { code, msg } => {
                assert_eq!(code, 2);
                assert!(!msg.is_empty(), "message must not be empty");
            }
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn out_of_memory_to_compute_error_carries_detail() {
        let e = CudaError::OutOfMemory {
            device: 0,
            requested_bytes: 1024,
            available_bytes: 512,
        };
        let ce: ComputeError = e.into();
        let msg = format!("{ce}");
        assert!(msg.contains("VRAM exhausted"));
        assert!(msg.contains("1024"));
    }

    #[test]
    fn not_yet_wired_to_compute_error() {
        let e = CudaError::NotYetWired("matmul");
        let ce: ComputeError = e.into();
        assert!(format!("{ce}").contains("matmul"));
    }

    #[test]
    fn unsupported_capability_to_compute_error() {
        let e = CudaError::UnsupportedCapability {
            device: 0,
            got_cc: 60,
            min_cc: 75,
        };
        let ce: ComputeError = e.into();
        let msg = format!("{ce}");
        assert!(msg.contains("sm_60"));
        assert!(msg.contains("sm_75"));
    }
}
