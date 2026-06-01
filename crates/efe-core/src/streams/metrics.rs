//! Atomic metrics for the stream pool.

use std::sync::atomic::{AtomicU64, Ordering};

/// Live atomic counters for the stream pool.
#[derive(Debug, Default)]
pub struct StreamMetrics {
    /// Total `acquire` calls.
    pub acquire_count: AtomicU64,
    /// Acquires that found a free stream immediately.
    pub fast_acquire_count: AtomicU64,
    /// Acquires that had to wait on a busy pool.
    pub contended_acquire_count: AtomicU64,
    /// Cumulative wait time in nanoseconds.
    pub acquire_wait_ns_total: AtomicU64,
    /// Total synchronize calls.
    pub synchronize_count: AtomicU64,
    /// Streams currently held by handles.
    pub active_streams: AtomicU64,
}

impl StreamMetrics {
    /// Zero-initialized.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot of the live counters.
    ///
    /// # Atomicity contract
    ///
    /// Each individual atomic load uses `Ordering::Acquire`, but the
    /// snapshot **is not globally atomic**: between consecutive loads
    /// other threads may advance the counters. The fields of
    /// [`StreamMetricsSnapshot`] therefore satisfy each pairwise
    /// invariant **separately** but the sum
    /// `acquire_count == fast + contended` may be off by `O(N_threads)`
    /// for an instant.
    ///
    /// Audit-B8 fix #7 documents this explicitly: snapshots are good
    /// enough for observability but should not be used for
    /// correctness assertions in tight loops. The stress tests in
    /// `tests/streams_stress.rs` only check the invariant on the
    /// post-join snapshot, when no thread is mutating the counters.
    #[must_use]
    pub fn snapshot(&self) -> StreamMetricsSnapshot {
        let acquires = self.acquire_count.load(Ordering::Acquire);
        let wait_ns = self.acquire_wait_ns_total.load(Ordering::Acquire);
        let mean_wait_ns = if acquires == 0 {
            0.0
        } else {
            wait_ns as f32 / acquires as f32
        };
        StreamMetricsSnapshot {
            acquire_count: acquires,
            fast_acquire_count: self.fast_acquire_count.load(Ordering::Acquire),
            contended_acquire_count: self.contended_acquire_count.load(Ordering::Acquire),
            synchronize_count: self.synchronize_count.load(Ordering::Acquire),
            active_streams: self.active_streams.load(Ordering::Acquire),
            mean_acquire_wait_ns: mean_wait_ns,
        }
    }
}

/// Immutable snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamMetricsSnapshot {
    /// Total acquires.
    pub acquire_count: u64,
    /// Acquires served without waiting.
    pub fast_acquire_count: u64,
    /// Acquires that contended on the free-list.
    pub contended_acquire_count: u64,
    /// Total `synchronize` calls.
    pub synchronize_count: u64,
    /// Streams currently in flight.
    pub active_streams: u64,
    /// Mean wait on contended acquires (ns).
    pub mean_acquire_wait_ns: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_on_fresh() {
        let m = StreamMetrics::new();
        let s = m.snapshot();
        assert_eq!(s.acquire_count, 0);
        assert!(s.mean_acquire_wait_ns.abs() < f32::EPSILON);
    }

    #[test]
    fn mean_computed() {
        let m = StreamMetrics::new();
        m.acquire_count.store(4, Ordering::Release);
        m.acquire_wait_ns_total.store(2000, Ordering::Release);
        let s = m.snapshot();
        assert!((s.mean_acquire_wait_ns - 500.0).abs() < 1e-3);
    }
}
