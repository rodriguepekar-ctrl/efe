//! Pool of pre-allocated arenas.
//!
//! The pool maintains a free-list and a concurrency semaphore sized to
//! `max_arenas`. Acquiring returns an [`ArenaHandle`]; dropping the
//! handle returns the arena and releases the semaphore slot.
//!
//! # Thread safety
//!
//! - The free-list is guarded by a `Mutex` (contention only on
//!   acquire/release boundaries, not during intra-arena bump allocation).
//! - Metrics are atomic and lock-free.
//! - The semaphore uses `tokio::sync::Semaphore` so waiters are async.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use tokio::sync::Semaphore;

use crate::errors::{EfeError, Result};
use crate::memory::arena::Arena;
use crate::memory::backend::DeviceBackend;
use crate::memory::config::PoolConfig;
use crate::memory::handle::ArenaHandle;
use crate::memory::metrics::{PoolMetrics, PoolMetricsSnapshot};

/// Thread-safe pool of arenas.
pub struct MemoryPool {
    inner: Arc<MemoryPoolInner>,
}

/// Inner shared state. Exposed via `pub(crate)` for [`ArenaHandle`]
/// which needs to return arenas.
pub(crate) struct MemoryPoolInner {
    pub(crate) free_list: Mutex<Vec<Arena>>,
    pub(crate) config: PoolConfig,
    pub(crate) metrics: PoolMetrics,
    pub(crate) backend: Arc<dyn DeviceBackend>,
    pub(crate) slots: Arc<Semaphore>,
}

impl MemoryPoolInner {
    /// Return an arena to the free-list. Called by [`ArenaHandle::drop`].
    pub(crate) fn return_arena(&self, arena: Arena) {
        let mut guard = self.free_list.lock().expect("free_list mutex poisoned");
        // If the pool is already full (shouldn't happen because of the
        // semaphore), drop the arena rather than grow unboundedly.
        if guard.len() < self.config.max_arenas {
            guard.push(arena);
        } else {
            tracing::warn!("free_list full on return; dropping arena");
            // Arena dropped → backend deallocate.
            drop(arena);
        }
    }
}

impl MemoryPool {
    /// Build a pool with the given configuration and backend.
    ///
    /// Eagerly allocates `config.prealloc_arenas` arenas.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] on bad [`PoolConfig`].
    /// - [`EfeError::HbmAllocFailed`] if preallocation cannot succeed.
    pub fn new(config: PoolConfig, backend: Arc<dyn DeviceBackend>) -> Result<Self> {
        config.validate()?;
        let slots = Arc::new(Semaphore::new(config.max_arenas));
        let metrics = PoolMetrics::new();
        let mut preallocated = Vec::with_capacity(config.prealloc_arenas);
        for _ in 0..config.prealloc_arenas {
            let arena = Arena::new(config.default_arena_size, backend.clone(), config.device_id)?;
            metrics
                .arenas_allocated_total
                .fetch_add(1, Ordering::AcqRel);
            preallocated.push(arena);
        }
        let inner = Arc::new(MemoryPoolInner {
            free_list: Mutex::new(preallocated),
            config,
            metrics,
            backend,
            slots,
        });
        Ok(Self { inner })
    }

    /// Read-only snapshot of live metrics.
    #[must_use]
    pub fn metrics(&self) -> PoolMetricsSnapshot {
        self.inner.metrics.snapshot()
    }

    /// Configured `max_arenas`.
    #[must_use]
    pub fn max_arenas(&self) -> usize {
        self.inner.config.max_arenas
    }

    /// Backend identity.
    #[must_use]
    pub fn backend_name(&self) -> &'static str {
        self.inner.backend.name()
    }

    /// Acquire an arena with at least `min_bytes` capacity.
    ///
    /// If the pool is saturated, waits on the semaphore until a handle
    /// is returned or `deadline` (when provided) is reached.
    ///
    /// # Errors
    ///
    /// - [`EfeError::HbmAllocFailed`] when no arena can satisfy
    ///   `min_bytes`.
    /// - [`EfeError::BudgetExhausted { kind: "wall_clock" }`] when
    ///   `deadline` is reached while waiting.
    pub async fn acquire(&self, min_bytes: u64, deadline: Option<Instant>) -> Result<ArenaHandle> {
        self.inner
            .metrics
            .acquire_count
            .fetch_add(1, Ordering::AcqRel);

        let wait_start = Instant::now();
        let permit = if let Some(d) = deadline {
            let timeout = d.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                return Err(EfeError::BudgetExhausted { kind: "wall_clock" });
            }
            match tokio::time::timeout(timeout, self.inner.slots.clone().acquire_owned()).await {
                Ok(Ok(p)) => p,
                Ok(Err(_)) => {
                    return Err(EfeError::Internal("semaphore closed"));
                }
                Err(_elapsed) => {
                    return Err(EfeError::BudgetExhausted { kind: "wall_clock" });
                }
            }
        } else {
            self.inner
                .slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| EfeError::Internal("semaphore closed"))?
        };
        let wait_us = elapsed_us(wait_start);
        self.inner
            .metrics
            .acquire_wait_us_total
            .fetch_add(wait_us, Ordering::AcqRel);

        // Try free-list first.
        let arena_result = {
            let mut guard = self
                .inner
                .free_list
                .lock()
                .expect("free_list mutex poisoned");
            guard
                .iter()
                .position(|a| a.capacity_bytes() >= min_bytes)
                .map(|idx| guard.swap_remove(idx))
        };

        let arena = if let Some(a) = arena_result {
            self.inner.metrics.hit_count.fetch_add(1, Ordering::AcqRel);
            a
        } else {
            // Allocate fresh arena, sized to max(min_bytes, default).
            let size = min_bytes.max(self.inner.config.default_arena_size);
            let fresh = Arena::new(
                size,
                self.inner.backend.clone(),
                self.inner.config.device_id,
            );
            match fresh {
                Ok(a) => {
                    self.inner
                        .metrics
                        .arenas_allocated_total
                        .fetch_add(1, Ordering::AcqRel);
                    self.inner.metrics.miss_count.fetch_add(1, Ordering::AcqRel);
                    a
                }
                Err(e) => {
                    self.inner.metrics.oom_count.fetch_add(1, Ordering::AcqRel);
                    return Err(e);
                }
            }
        };

        let bytes = arena.capacity_bytes();
        self.inner
            .metrics
            .arenas_active
            .fetch_add(1, Ordering::AcqRel);
        self.inner
            .metrics
            .bytes_active
            .fetch_add(bytes, Ordering::AcqRel);

        Ok(ArenaHandle::new(arena, Arc::downgrade(&self.inner), permit))
    }

    /// Non-async variant for synchronous callers. Fails immediately if
    /// the pool is saturated.
    ///
    /// # Errors
    ///
    /// - [`EfeError::BudgetExhausted { kind: "wall_clock" }`] if the
    ///   pool has no free slot.
    /// - See also [`Self::acquire`].
    pub fn try_acquire(&self, min_bytes: u64) -> Result<ArenaHandle> {
        self.inner
            .metrics
            .acquire_count
            .fetch_add(1, Ordering::AcqRel);

        let permit = self
            .inner
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| EfeError::BudgetExhausted { kind: "wall_clock" })?;

        let arena_result = {
            let mut guard = self
                .inner
                .free_list
                .lock()
                .expect("free_list mutex poisoned");
            guard
                .iter()
                .position(|a| a.capacity_bytes() >= min_bytes)
                .map(|idx| guard.swap_remove(idx))
        };

        let arena = if let Some(a) = arena_result {
            self.inner.metrics.hit_count.fetch_add(1, Ordering::AcqRel);
            a
        } else {
            let size = min_bytes.max(self.inner.config.default_arena_size);
            let a = Arena::new(
                size,
                self.inner.backend.clone(),
                self.inner.config.device_id,
            )
            .inspect_err(|_| {
                self.inner.metrics.oom_count.fetch_add(1, Ordering::AcqRel);
            })?;
            self.inner
                .metrics
                .arenas_allocated_total
                .fetch_add(1, Ordering::AcqRel);
            self.inner.metrics.miss_count.fetch_add(1, Ordering::AcqRel);
            a
        };

        let bytes = arena.capacity_bytes();
        self.inner
            .metrics
            .arenas_active
            .fetch_add(1, Ordering::AcqRel);
        self.inner
            .metrics
            .bytes_active
            .fetch_add(bytes, Ordering::AcqRel);
        Ok(ArenaHandle::new(arena, Arc::downgrade(&self.inner), permit))
    }

    /// Weak reference to the inner state. Used by tests.
    #[doc(hidden)]
    #[allow(dead_code)]
    #[must_use]
    pub(crate) fn inner_weak(&self) -> Weak<MemoryPoolInner> {
        Arc::downgrade(&self.inner)
    }
}

impl Clone for MemoryPool {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl std::fmt::Debug for MemoryPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryPool")
            .field("max_arenas", &self.inner.config.max_arenas)
            .field("default_arena_size", &self.inner.config.default_arena_size)
            .field("backend", &self.inner.backend.name())
            .field("metrics", &self.metrics())
            .finish()
    }
}

fn elapsed_us(start: Instant) -> u64 {
    let d: Duration = start.elapsed();
    let secs = d.as_secs().saturating_mul(1_000_000);
    let micros = u64::from(d.subsec_micros());
    secs.saturating_add(micros)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::backend::HostBackend;

    fn make_pool(max: usize, prealloc: usize) -> MemoryPool {
        let cfg = PoolConfig::default()
            .with_max_arenas(max)
            .with_prealloc(prealloc)
            .with_arena_size(4096);
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        MemoryPool::new(cfg, backend).expect("pool")
    }

    #[tokio::test]
    async fn acquire_returns_valid_arena() {
        let pool = make_pool(4, 2);
        let h = pool.acquire(1024, None).await.expect("acquire");
        assert!(h.arena().capacity_bytes() >= 1024);
    }

    #[tokio::test]
    async fn preallocated_arenas_are_hits() {
        let pool = make_pool(4, 4);
        let _h1 = pool.acquire(1024, None).await.expect("a1");
        let _h2 = pool.acquire(1024, None).await.expect("a2");
        let m = pool.metrics();
        assert_eq!(m.hit_count, 2);
        assert_eq!(m.miss_count, 0);
    }

    #[tokio::test]
    async fn fresh_alloc_when_pool_cold() {
        let pool = make_pool(4, 0);
        let _h = pool.acquire(1024, None).await.expect("a");
        let m = pool.metrics();
        assert_eq!(m.hit_count, 0);
        assert_eq!(m.miss_count, 1);
        assert_eq!(m.arenas_allocated_total, 1);
    }

    #[tokio::test]
    async fn release_returns_to_pool() {
        let pool = make_pool(2, 0);
        {
            let _h = pool.acquire(1024, None).await.expect("a");
            assert_eq!(pool.metrics().arenas_active, 1);
        }
        // Give the runtime a tick for drop to propagate (sync drop, not needed actually).
        assert_eq!(pool.metrics().arenas_active, 0);
    }

    #[tokio::test]
    async fn saturation_blocks_until_release() {
        let pool = make_pool(1, 0);
        let h1 = pool.acquire(512, None).await.expect("a1");
        let pool2 = pool.clone();
        let fut = tokio::spawn(async move {
            pool2
                .acquire(512, Some(Instant::now() + Duration::from_millis(500)))
                .await
        });
        // No release yet, the future shouldn't resolve.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!fut.is_finished());
        drop(h1);
        let h2 = fut.await.expect("join").expect("acquire after release");
        assert!(h2.arena().capacity_bytes() >= 512);
    }

    #[tokio::test]
    async fn deadline_triggers_budget_exhausted() {
        let pool = make_pool(1, 0);
        let _h1 = pool.acquire(512, None).await.expect("a1");
        let err = pool
            .acquire(512, Some(Instant::now() + Duration::from_millis(30)))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            EfeError::BudgetExhausted { kind: "wall_clock" }
        ));
    }

    #[tokio::test]
    async fn try_acquire_fails_on_saturation() {
        let pool = make_pool(1, 0);
        let _h = pool.acquire(512, None).await.expect("a1");
        let err = pool.try_acquire(512).unwrap_err();
        assert!(matches!(
            err,
            EfeError::BudgetExhausted { kind: "wall_clock" }
        ));
    }

    #[tokio::test]
    async fn try_acquire_succeeds_when_free() {
        let pool = make_pool(2, 0);
        let h = pool.try_acquire(512).expect("ok");
        assert!(h.arena().capacity_bytes() >= 512);
    }

    #[tokio::test]
    async fn soak_no_leak_after_many_cycles() {
        let pool = make_pool(4, 2);
        for _ in 0..1_000 {
            let _h = pool.acquire(128, None).await.expect("a");
        }
        let m = pool.metrics();
        assert_eq!(m.arenas_active, 0);
        assert_eq!(m.bytes_active, 0);
        assert!(m.hit_count + m.miss_count >= 1_000);
    }

    #[tokio::test]
    async fn large_request_gets_fresh_arena() {
        let pool = make_pool(4, 2); // prealloc are 4 KiB.
        let h = pool.acquire(1 << 20, None).await.expect("a"); // 1 MiB
        assert!(h.arena().capacity_bytes() >= 1 << 20);
        let m = pool.metrics();
        assert_eq!(m.miss_count, 1);
    }

    #[tokio::test]
    async fn concurrent_acquires_respect_max_arenas() {
        let pool = Arc::new(make_pool(3, 0));
        let barrier = Arc::new(tokio::sync::Barrier::new(5));
        let mut tasks = vec![];
        for _ in 0..5 {
            let p = pool.clone();
            let b = barrier.clone();
            tasks.push(tokio::spawn(async move {
                b.wait().await;
                let h = p.acquire(128, None).await.expect("a");
                tokio::time::sleep(Duration::from_millis(20)).await;
                drop(h);
            }));
        }
        for t in tasks {
            t.await.expect("join");
        }
        let m = pool.metrics();
        // Upper bound: 5 acquires, but only 3 slots → at any point
        // arenas_allocated_total <= 3 (we never exceed max_arenas worth
        // of fresh allocs because release cycles the free-list).
        assert!(m.arenas_allocated_total <= 3);
    }
}
