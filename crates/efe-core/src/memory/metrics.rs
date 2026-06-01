//! Lock-free atomic metrics for the memory pool.
//!
//! Metrics are read by M11 (telemetry) via [`PoolMetrics::snapshot`],
//! which is a cheap AcqRel load of each counter. No Mutex, no Prometheus
//! dependency in this module.

use std::sync::atomic::{AtomicU64, Ordering};

/// Live atomic counters tracking pool activity.
///
/// Clients of the metrics (M11) should take [`PoolMetrics::snapshot`]
/// rather than reading individual fields to obtain a consistent view.
#[derive(Debug, Default)]
pub struct PoolMetrics {
    /// Total arenas allocated since boot (lifetime counter).
    pub arenas_allocated_total: AtomicU64,
    /// Arenas currently held by handles (`acquired - released`).
    pub arenas_active: AtomicU64,
    /// Bytes currently allocated across active arenas.
    pub bytes_active: AtomicU64,
    /// Total `acquire` calls.
    pub acquire_count: AtomicU64,
    /// Acquires satisfied from the free-list (warm).
    pub hit_count: AtomicU64,
    /// Acquires that required a fresh `cudaMalloc` (cold).
    pub miss_count: AtomicU64,
    /// Acquires that failed with OOM.
    pub oom_count: AtomicU64,
    /// Cumulative time spent waiting on the concurrency semaphore (µs).
    pub acquire_wait_us_total: AtomicU64,
}

impl PoolMetrics {
    /// Zero-initialized metrics.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Take a consistent snapshot.
    ///
    /// Each field is loaded with `Ordering::Acquire`; the snapshot is
    /// *approximately* consistent (no global lock), which is acceptable
    /// for monitoring but not for auditing.
    #[must_use]
    pub fn snapshot(&self) -> PoolMetricsSnapshot {
        let acquire_count = self.acquire_count.load(Ordering::Acquire);
        let hit_count = self.hit_count.load(Ordering::Acquire);
        let miss_count = self.miss_count.load(Ordering::Acquire);
        let wait_us = self.acquire_wait_us_total.load(Ordering::Acquire);

        let hit_ratio = if hit_count + miss_count == 0 {
            0.0
        } else {
            hit_count as f32 / (hit_count + miss_count) as f32
        };
        let mean_wait_us = if acquire_count == 0 {
            0.0
        } else {
            wait_us as f32 / acquire_count as f32
        };

        PoolMetricsSnapshot {
            arenas_allocated_total: self.arenas_allocated_total.load(Ordering::Acquire),
            arenas_active: self.arenas_active.load(Ordering::Acquire),
            bytes_active: self.bytes_active.load(Ordering::Acquire),
            acquire_count,
            hit_count,
            miss_count,
            oom_count: self.oom_count.load(Ordering::Acquire),
            hit_ratio,
            mean_acquire_wait_us: mean_wait_us,
        }
    }

    /// Reset all counters to zero. Intended for tests only.
    #[doc(hidden)]
    pub fn reset(&self) {
        for counter in [
            &self.arenas_allocated_total,
            &self.arenas_active,
            &self.bytes_active,
            &self.acquire_count,
            &self.hit_count,
            &self.miss_count,
            &self.oom_count,
            &self.acquire_wait_us_total,
        ] {
            counter.store(0, Ordering::Release);
        }
    }
}

/// Immutable snapshot of pool metrics.
#[derive(Clone, Copy, Debug, Default)]
pub struct PoolMetricsSnapshot {
    /// Arenas allocated since boot.
    pub arenas_allocated_total: u64,
    /// Arenas currently in flight.
    pub arenas_active: u64,
    /// Bytes currently held.
    pub bytes_active: u64,
    /// Acquire calls.
    pub acquire_count: u64,
    /// Free-list hits.
    pub hit_count: u64,
    /// Free-list misses (needed fresh alloc).
    pub miss_count: u64,
    /// OOM failures.
    pub oom_count: u64,
    /// Ratio `hit / (hit + miss)`, `0.0` if never acquired.
    pub hit_ratio: f32,
    /// Mean wait on semaphore in µs.
    pub mean_acquire_wait_us: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_zero_when_fresh() {
        let m = PoolMetrics::new();
        let s = m.snapshot();
        assert_eq!(s.arenas_active, 0);
        assert_eq!(s.acquire_count, 0);
        assert!(s.hit_ratio.abs() < f32::EPSILON);
    }

    #[test]
    fn hit_ratio_computed() {
        let m = PoolMetrics::new();
        m.hit_count.store(8, Ordering::Release);
        m.miss_count.store(2, Ordering::Release);
        m.acquire_count.store(10, Ordering::Release);
        let s = m.snapshot();
        assert!((s.hit_ratio - 0.8).abs() < 1e-6);
    }

    #[test]
    fn mean_wait_computed() {
        let m = PoolMetrics::new();
        m.acquire_count.store(10, Ordering::Release);
        m.acquire_wait_us_total.store(500, Ordering::Release);
        let s = m.snapshot();
        assert!((s.mean_acquire_wait_us - 50.0).abs() < 1e-6);
    }

    #[test]
    fn reset_clears_all() {
        let m = PoolMetrics::new();
        m.acquire_count.store(42, Ordering::Release);
        m.reset();
        assert_eq!(m.acquire_count.load(Ordering::Acquire), 0);
    }
}
