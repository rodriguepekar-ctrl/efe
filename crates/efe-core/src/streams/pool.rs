//! Stream pool implementation.
//!
//! Concurrency: the semaphore bounds the number of in-flight handles
//! to `config.size`. Each [`StreamHandle`] owns an
//! `OwnedSemaphorePermit` that is released when the handle drops, so
//! the refill is automatic and can't be forgotten.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use crate::errors::{EfeError, Result};
use tokio::sync::Semaphore;

use crate::streams::config::StreamPoolConfig;
use crate::streams::handle::StreamHandle;
use crate::streams::metrics::{StreamMetrics, StreamMetricsSnapshot};

/// MVP pseudo-stream representation. Real `CUstream` arrives in Sprint 2.
#[derive(Debug)]
pub(crate) struct Stream {
    pub(crate) id: u32,
}

pub(crate) struct StreamPoolInner {
    free_list: Mutex<VecDeque<Stream>>,
    pub(crate) metrics: StreamMetrics,
    slots: Arc<Semaphore>,
    config: StreamPoolConfig,
}

impl StreamPoolInner {
    pub(crate) fn return_stream(&self, stream: Stream) {
        let mut guard = self.free_list.lock().expect("stream free_list poisoned");
        if guard.len() < self.config.size {
            guard.push_back(stream);
        }
        // Else drop: defensive, should not happen because the semaphore
        // bounds concurrency to `config.size`.
    }

    pub(crate) fn take_stream(&self) -> Stream {
        let mut guard = self.free_list.lock().expect("free_list poisoned");
        guard.pop_front().unwrap_or_else(|| {
            // **Audit-B8 fix #2** : the previous implementation returned
            // `Stream { id: u32::MAX }` as a sentinel here. That made a
            // "free-list desync vs semaphore" bug invisible — downstream
            // code would happily use a fake id with no observable
            // failure until much later. The semaphore guarantees this
            // path is unreachable under correct usage (every acquired
            // permit has a matching stream pushed at init or returned
            // afterwards), so we now panic loudly. If this fires in
            // production it is a use-after-drop or double-release bug
            // in the pool's caller and must be reported.
            tracing::error!("stream free_list empty while permit granted");
            panic!(
                "StreamPool invariant violated: free_list empty while \
                 a semaphore permit was granted. This indicates a \
                 use-after-drop or double-release bug. Pool size = {}.",
                self.config.size,
            );
        })
    }
}

/// Pool of CUDA streams.
pub struct StreamPool {
    inner: Arc<StreamPoolInner>,
}

impl StreamPool {
    /// Build a pool with `config.size` streams pre-created.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] on bad config.
    pub fn new(config: StreamPoolConfig) -> Result<Self> {
        config.validate()?;
        let mut free = VecDeque::with_capacity(config.size);
        for id in 0..config.size {
            free.push_back(Stream {
                id: u32::try_from(id).unwrap_or(u32::MAX),
            });
        }
        let slots = Arc::new(Semaphore::new(config.size));
        Ok(Self {
            inner: Arc::new(StreamPoolInner {
                free_list: Mutex::new(free),
                metrics: StreamMetrics::new(),
                slots,
                config,
            }),
        })
    }

    /// Acquire a stream (async, blocks when all are busy).
    ///
    /// # Errors
    ///
    /// - [`EfeError::Internal`] if the underlying semaphore is closed.
    pub async fn acquire(&self) -> Result<StreamHandle> {
        self.inner
            .metrics
            .acquire_count
            .fetch_add(1, Ordering::AcqRel);

        // Fast path: try to grab a permit immediately.
        if let Ok(permit) = self.inner.slots.clone().try_acquire_owned() {
            self.inner
                .metrics
                .fast_acquire_count
                .fetch_add(1, Ordering::AcqRel);
            return Ok(self.assemble(permit));
        }

        // Slow path: wait for a permit.
        self.inner
            .metrics
            .contended_acquire_count
            .fetch_add(1, Ordering::AcqRel);
        let start = Instant::now();
        let permit = self
            .inner
            .slots
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| EfeError::Internal("stream pool semaphore closed"))?;
        let waited_ns = u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX);
        self.inner
            .metrics
            .acquire_wait_ns_total
            .fetch_add(waited_ns, Ordering::AcqRel);

        Ok(self.assemble(permit))
    }

    /// Try to acquire without waiting.
    ///
    /// # Errors
    ///
    /// - [`EfeError::BudgetExhausted { kind: "stream" }`] when no stream
    ///   is available.
    pub fn try_acquire(&self) -> Result<StreamHandle> {
        self.inner
            .metrics
            .acquire_count
            .fetch_add(1, Ordering::AcqRel);
        let permit = self
            .inner
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| EfeError::BudgetExhausted { kind: "stream" })?;
        self.inner
            .metrics
            .fast_acquire_count
            .fetch_add(1, Ordering::AcqRel);
        Ok(self.assemble(permit))
    }

    fn assemble(&self, permit: tokio::sync::OwnedSemaphorePermit) -> StreamHandle {
        let stream = self.inner.take_stream();
        self.inner
            .metrics
            .active_streams
            .fetch_add(1, Ordering::AcqRel);
        StreamHandle::new(stream, Arc::downgrade(&self.inner), permit)
    }

    /// Read-only metrics snapshot.
    #[must_use]
    pub fn metrics(&self) -> StreamMetricsSnapshot {
        self.inner.metrics.snapshot()
    }

    /// Configured pool size.
    #[must_use]
    pub fn size(&self) -> usize {
        self.inner.config.size
    }

    /// Test-only: weak ref to inner state.
    #[doc(hidden)]
    #[allow(dead_code)]
    #[must_use]
    pub(crate) fn inner_weak(&self) -> Weak<StreamPoolInner> {
        Arc::downgrade(&self.inner)
    }
}

impl Clone for StreamPool {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl std::fmt::Debug for StreamPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamPool")
            .field("size", &self.inner.config.size)
            .field("metrics", &self.metrics())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_returns_handle() {
        let pool = StreamPool::new(StreamPoolConfig::default().with_size(2)).expect("pool");
        let h = pool.acquire().await.expect("ok");
        let _ = h.id();
    }

    #[tokio::test]
    async fn fast_path_when_available() {
        let pool = StreamPool::new(StreamPoolConfig::default().with_size(4)).expect("pool");
        let _h = pool.acquire().await.expect("ok");
        assert!(pool.metrics().fast_acquire_count >= 1);
    }

    #[tokio::test]
    async fn try_acquire_fails_when_saturated() {
        let pool = StreamPool::new(StreamPoolConfig::default().with_size(1)).expect("pool");
        let _h = pool.acquire().await.expect("first");
        let err = pool.try_acquire().unwrap_err();
        assert!(matches!(err, EfeError::BudgetExhausted { kind: "stream" }));
    }

    #[tokio::test]
    async fn synchronize_is_noop() {
        let pool = StreamPool::new(StreamPoolConfig::default().with_size(1)).expect("pool");
        let h = pool.acquire().await.expect("ok");
        h.synchronize().await.expect("noop");
        assert_eq!(pool.metrics().synchronize_count, 1);
    }

    #[tokio::test]
    async fn release_returns_to_pool() {
        let pool = StreamPool::new(StreamPoolConfig::default().with_size(1)).expect("pool");
        {
            let _h = pool.acquire().await.expect("a");
            assert_eq!(pool.metrics().active_streams, 1);
        }
        assert_eq!(pool.metrics().active_streams, 0);
        // Next acquire should succeed (slot freed).
        let _h2 = pool.acquire().await.expect("second");
    }

    #[tokio::test]
    async fn concurrent_acquires_serialize() {
        use std::sync::Arc as StdArc;
        let pool =
            StdArc::new(StreamPool::new(StreamPoolConfig::default().with_size(2)).expect("pool"));
        let mut tasks = vec![];
        for _ in 0..8 {
            let p = pool.clone();
            tasks.push(tokio::spawn(async move {
                let h = p.acquire().await.expect("ok");
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                drop(h);
            }));
        }
        for t in tasks {
            t.await.expect("join");
        }
        assert_eq!(pool.metrics().active_streams, 0);
    }

    #[tokio::test]
    async fn size_reported() {
        let pool = StreamPool::new(StreamPoolConfig::default().with_size(7)).expect("pool");
        assert_eq!(pool.size(), 7);
    }
}
