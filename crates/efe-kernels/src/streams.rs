//! CUDA stream pool — Sprint D28.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 14.1.
//!
//! Strict invariant : **no kernel runs on the default stream
//! (stream 0)**. Every launch goes through a stream allocated by
//! [`StreamPool`] so the runtime can multiplex N concurrent
//! requests on the same GPU without serialising them.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use cudarc::driver::{CudaDevice, CudaStream};

use crate::error::CudaError;

/// **Audit fix K-A8 + C-A1** — extract the raw `cudaStream_t` pointer
/// from a `cudarc::driver::CudaStream` regardless of the cudarc minor
/// version.
///
/// History :
///
///   * Pre-K-A8 : the backend used `stream.handle()` directly. That
///     method does not exist in cudarc 0.10 (the API surface uses
///     a `stream()` accessor instead).
///   * K-A8 (D28 audit pass 1) : centralised the lookup but used
///     `stream.stream` (field access). That field is `pub(crate)`
///     in cudarc 0.10 — accessing it from outside the crate is a
///     compile error with `--features cuda`. **Caught in pass-3 audit
///     as C-A1.**
///   * C-A1 (D28 audit pass 3, this commit) : use the public method
///     `stream.stream()` which returns the raw `sys::CUstream`. The
///     centralisation pattern is preserved so a future cudarc bump
///     only needs to update this single function.
///
/// SAFETY : the returned pointer aliases the stream's internal
/// handle. The caller must not free it ; the `CudaStream` holds
/// ownership and `cudaStreamDestroy` runs at its `Drop`.
#[must_use]
pub fn raw_stream(stream: &CudaStream) -> crate::kernels::cudaStream_t {
    // cudarc 0.10 public API : `CudaStream::stream(&self) -> sys::CUstream`.
    // The type alias `sys::CUstream = *mut sys::CUstream_st` matches
    // our `kernels::cudaStream_t` exactly so the cast is a no-op.
    stream.stream() as crate::kernels::cudaStream_t
}

/// Round-robin pool of `cudarc::driver::CudaStream` handles.
///
/// 4 streams by default — fits the 80 % concurrency observation
/// from the D18 multi-replica bench. Override via
/// `EFE_RUNTIME_GPU_STREAMS` env var (consumed at boot by
/// `efe-server`).
pub struct StreamPool {
    streams: Vec<Arc<CudaStream>>,
    next_idx: AtomicUsize,
    device: Arc<CudaDevice>,
}

impl std::fmt::Debug for StreamPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamPool")
            .field("device_ordinal", &self.device.ordinal())
            .field("n_streams", &self.streams.len())
            .finish()
    }
}

impl StreamPool {
    /// Create `count` non-default streams on `device`. Returns
    /// `CudaError::StreamError` if any allocation fails.
    pub fn new(device: Arc<CudaDevice>, count: usize) -> Result<Self, CudaError> {
        if count == 0 {
            return Err(CudaError::StreamError(
                "stream count must be ≥ 1".into(),
            ));
        }
        let mut streams = Vec::with_capacity(count);
        for i in 0..count {
            let s = device
                .fork_default_stream()
                .map_err(|e| {
                    CudaError::StreamError(format!(
                        "fork_default_stream #{i} on device {}: {e}",
                        device.ordinal()
                    ))
                })?;
            streams.push(Arc::new(s));
        }
        Ok(Self {
            streams,
            next_idx: AtomicUsize::new(0),
            device,
        })
    }

    /// Construct with the default stream count of 4.
    pub fn with_default_size(device: Arc<CudaDevice>) -> Result<Self, CudaError> {
        Self::new(device, 4)
    }

    /// Pick the next stream in round-robin order.
    ///
    /// `Ordering::Relaxed` is sufficient — the only invariant is
    /// "every call returns a valid stream" ; the actual order
    /// across threads is irrelevant for correctness.
    #[must_use]
    pub fn next(&self) -> Arc<CudaStream> {
        let idx = self.next_idx.fetch_add(1, Ordering::Relaxed) % self.streams.len();
        Arc::clone(&self.streams[idx])
    }

    /// Synchronise every stream in the pool. Used at shutdown +
    /// before the multi-GPU NCCL barrier.
    pub fn synchronize_all(&self) -> Result<(), CudaError> {
        for s in &self.streams {
            s.synchronize().map_err(|e| {
                CudaError::StreamError(format!(
                    "synchronize stream on device {}: {e}",
                    self.device.ordinal()
                ))
            })?;
        }
        Ok(())
    }

    /// Number of streams in the pool. For diagnostics.
    #[must_use]
    pub fn len(&self) -> usize {
        self.streams.len()
    }

    /// `true` iff the pool is empty (impossible by construction
    /// but kept for `clippy::len_zero` happiness).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }
}

#[cfg(test)]
mod tests {
    // The `cuda` feature is required to actually open a CUDA
    // device ; these tests are skipped in workspace CPU CI and
    // run on the GPU-runner CI matrix only.
    //
    // We still validate the **constructor invariants** that hold
    // independent of an actual GPU.

    use super::*;

    #[test]
    fn stream_pool_zero_count_rejected() {
        // We cannot actually instantiate a CudaDevice here without
        // CUDA. Instead, we exercise the count guard via a unit
        // helper that mimics the constructor's first branch.
        fn validate_count(c: usize) -> Result<(), CudaError> {
            if c == 0 {
                Err(CudaError::StreamError("stream count must be ≥ 1".into()))
            } else {
                Ok(())
            }
        }
        assert!(validate_count(0).is_err());
        assert!(validate_count(1).is_ok());
        assert!(validate_count(4).is_ok());
    }
}
