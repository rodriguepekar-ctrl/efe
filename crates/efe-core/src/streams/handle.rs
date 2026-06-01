//! RAII wrapper around a borrowed CUDA stream.

use std::sync::atomic::Ordering;
use std::sync::Weak;

use crate::errors::Result;
use tokio::sync::OwnedSemaphorePermit;

use crate::streams::pool::{Stream, StreamPoolInner};

/// Handle to a stream borrowed from a [`crate::streams::StreamPool`].
///
/// The stream is returned to the pool when the handle drops, and the
/// semaphore permit is released automatically.
pub struct StreamHandle {
    stream: Option<Stream>,
    pool: Weak<StreamPoolInner>,
    /// Released on drop — refills the pool's semaphore.
    _permit: OwnedSemaphorePermit,
}

impl StreamHandle {
    pub(crate) fn new(
        stream: Stream,
        pool: Weak<StreamPoolInner>,
        permit: OwnedSemaphorePermit,
    ) -> Self {
        Self {
            stream: Some(stream),
            pool,
            _permit: permit,
        }
    }

    /// Underlying stream id (opaque for MVP, matches `CUstream` later).
    ///
    /// # Panics
    ///
    /// Never — the `Option` is only `None` inside `drop`.
    #[must_use]
    pub fn id(&self) -> u32 {
        self.stream
            .as_ref()
            .expect("StreamHandle: stream taken before drop")
            .id
    }

    /// Wait for all work submitted on this stream to complete.
    ///
    /// # Errors
    ///
    /// MVP backend is a no-op and never fails. Sprint 2 CUDA backend
    /// returns `EfeError::KernelFailure` on any `cudaError_t` other
    /// than `cudaSuccess`.
    ///
    /// The function is `async` because Sprint 2+ will offload the
    /// blocking `cudaStreamSynchronize` to `spawn_blocking`.
    #[allow(clippy::unused_async)]
    pub async fn synchronize(&self) -> Result<()> {
        if let Some(pool) = self.pool.upgrade() {
            pool.metrics
                .synchronize_count
                .fetch_add(1, Ordering::AcqRel);
        }
        // MVP: no actual work to synchronize.
        Ok(())
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            if let Some(pool) = self.pool.upgrade() {
                pool.return_stream(stream);
                pool.metrics.active_streams.fetch_sub(1, Ordering::AcqRel);
            }
        }
        // `_permit` drops here → semaphore refilled automatically.
    }
}

impl std::fmt::Debug for StreamHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamHandle")
            .field("id", &self.stream.as_ref().map(|s| s.id))
            .finish_non_exhaustive()
    }
}
