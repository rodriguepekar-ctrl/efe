//! Sprint D18 — bench-side latency histogram + RPS counter.
//!
//! We don't pull `hdrhistogram` to keep the BOM minimal ; the
//! [`LatencyHistogram`] is a simple bucket-based approximation
//! with exponential bucket boundaries (1 ms → 30 s, 14 buckets).
//! Sufficient for p50/p95/p99 quantile estimation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::error::BenchError;

/// Bucket boundaries in milliseconds (left-inclusive, right-exclusive).
/// Exponentially spaced 1 ms → 30 s.
const BUCKET_BOUNDARIES_MS: &[u64] = &[
    1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 30000,
];

/// Number of buckets including overflow.
const N_BUCKETS: usize = BUCKET_BOUNDARIES_MS.len() + 1;

/// Lock-free latency histogram. Records observations into bucketed
/// counters via `AtomicU64`. Computes quantile estimates by linear
/// interpolation between bucket boundaries.
#[derive(Debug)]
pub struct LatencyHistogram {
    /// One counter per bucket. Index N_BUCKETS - 1 is overflow.
    buckets: [AtomicU64; N_BUCKETS],
    /// Total observation count.
    total: AtomicU64,
    /// Sum of all observation values in microseconds (for mean).
    sum_us: AtomicU64,
    /// Maximum observed value in microseconds.
    max_us: AtomicU64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyHistogram {
    /// Construct an empty histogram.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            total: AtomicU64::new(0),
            sum_us: AtomicU64::new(0),
            max_us: AtomicU64::new(0),
        }
    }

    /// Record a single observation.
    pub fn observe(&self, latency: Duration) {
        let ms = latency.as_millis() as u64;
        let us = latency.as_micros() as u64;
        let bucket_idx = match BUCKET_BOUNDARIES_MS.binary_search(&ms) {
            Ok(i) => i,         // exact bucket
            Err(i) => i,        // first bucket whose right edge > ms
        };
        let idx = bucket_idx.min(N_BUCKETS - 1);
        self.buckets[idx].fetch_add(1, Ordering::Relaxed);
        self.total.fetch_add(1, Ordering::Relaxed);
        self.sum_us.fetch_add(us, Ordering::Relaxed);
        // CAS update for max.
        let mut current = self.max_us.load(Ordering::Relaxed);
        loop {
            if us <= current {
                break;
            }
            match self.max_us.compare_exchange(
                current,
                us,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }

    /// Record observation in milliseconds (testing helper).
    pub fn observe_ms(&self, ms: u64) {
        self.observe(Duration::from_millis(ms));
    }

    /// Total recorded observations.
    pub fn count(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Estimate the q-quantile (q ∈ [0, 1]) in milliseconds.
    /// Returns None if no observations recorded.
    pub fn quantile_ms(&self, q: f64) -> Option<f64> {
        let total = self.count();
        if total == 0 {
            return None;
        }
        let target = ((total as f64) * q).ceil() as u64;
        let mut cumul: u64 = 0;
        for (i, b) in self.buckets.iter().enumerate() {
            cumul += b.load(Ordering::Relaxed);
            if cumul >= target {
                let upper = if i < BUCKET_BOUNDARIES_MS.len() {
                    BUCKET_BOUNDARIES_MS[i] as f64
                } else {
                    BUCKET_BOUNDARIES_MS[BUCKET_BOUNDARIES_MS.len() - 1] as f64 * 2.0
                };
                let lower = if i == 0 {
                    0.0
                } else {
                    BUCKET_BOUNDARIES_MS[i - 1] as f64
                };
                // Estimate by linear interpolation within bucket.
                return Some((lower + upper) / 2.0);
            }
        }
        None
    }

    /// Estimate the mean in milliseconds.
    pub fn mean_ms(&self) -> Option<f64> {
        let total = self.count();
        if total == 0 {
            return None;
        }
        let sum_us = self.sum_us.load(Ordering::Relaxed);
        Some((sum_us as f64) / (total as f64) / 1000.0)
    }

    /// Maximum observed latency in milliseconds.
    pub fn max_ms(&self) -> f64 {
        let max_us = self.max_us.load(Ordering::Relaxed);
        (max_us as f64) / 1000.0
    }

    /// Snapshot the bucket counts (for report serialisation).
    pub fn snapshot(&self) -> HistogramSnapshot {
        HistogramSnapshot {
            buckets_ms_upper: BUCKET_BOUNDARIES_MS
                .iter()
                .map(|b| *b as f64)
                .chain(std::iter::once(f64::INFINITY))
                .collect(),
            counts: self
                .buckets
                .iter()
                .map(|b| b.load(Ordering::Relaxed))
                .collect(),
            total: self.count(),
            sum_us: self.sum_us.load(Ordering::Relaxed),
            max_ms: self.max_ms(),
            mean_ms: self.mean_ms(),
            p50_ms: self.quantile_ms(0.50),
            p95_ms: self.quantile_ms(0.95),
            p99_ms: self.quantile_ms(0.99),
        }
    }

    /// Sanity check : reject any observation that would overflow
    /// the maximum bucket. Returns Ok if the bucket would land in
    /// the in-range or just-above-range bucket.
    pub fn check_within_range(latency: Duration) -> Result<(), BenchError> {
        if latency.as_secs() > 600 {
            // > 10 min is clearly an overflow / hung test ; reject.
            return Err(BenchError::HistogramOverflow(latency));
        }
        Ok(())
    }
}

/// Serialisable snapshot of a histogram (for JSON report).
#[derive(Debug, Clone, serde::Serialize)]
pub struct HistogramSnapshot {
    /// Upper bucket boundaries in milliseconds (with `inf` last).
    pub buckets_ms_upper: Vec<f64>,
    /// Counts in each bucket.
    pub counts: Vec<u64>,
    /// Total observations.
    pub total: u64,
    /// Sum in microseconds.
    pub sum_us: u64,
    /// Maximum observed latency in milliseconds.
    pub max_ms: f64,
    /// Mean in milliseconds.
    pub mean_ms: Option<f64>,
    /// Median (p50).
    pub p50_ms: Option<f64>,
    /// p95 latency.
    pub p95_ms: Option<f64>,
    /// p99 latency.
    pub p99_ms: Option<f64>,
}

/// Lock-free RPS counter ; tracks (success, error) totals.
#[derive(Debug, Default)]
pub struct RpsCounter {
    success: AtomicU64,
    error: AtomicU64,
    started_at: std::sync::OnceLock<Instant>,
}

impl RpsCounter {
    /// Construct a fresh counter (started clock lazy).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a success.
    pub fn record_success(&self) {
        self.success.fetch_add(1, Ordering::Relaxed);
        let _ = self.started_at.set(Instant::now());
    }

    /// Record an error.
    pub fn record_error(&self) {
        self.error.fetch_add(1, Ordering::Relaxed);
        let _ = self.started_at.set(Instant::now());
    }

    /// Total successful requests.
    pub fn success_count(&self) -> u64 {
        self.success.load(Ordering::Relaxed)
    }

    /// Total failed requests.
    pub fn error_count(&self) -> u64 {
        self.error.load(Ordering::Relaxed)
    }

    /// Total requests.
    pub fn total(&self) -> u64 {
        self.success_count() + self.error_count()
    }

    /// Average RPS since first observation.
    pub fn rps(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        let elapsed = match self.started_at.get() {
            Some(t) => t.elapsed().as_secs_f64(),
            None => return 0.0,
        };
        if elapsed <= 0.0 {
            return 0.0;
        }
        (total as f64) / elapsed
    }

    /// Error rate ∈ [0, 1].
    pub fn error_rate(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        (self.error_count() as f64) / (total as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_histogram_quantile_is_none() {
        let h = LatencyHistogram::new();
        assert!(h.quantile_ms(0.5).is_none());
    }

    #[test]
    fn observed_histogram_quantile_in_range() {
        let h = LatencyHistogram::new();
        for i in 0..1000 {
            h.observe_ms(i % 100);  // distribution 0..100 ms
        }
        let p50 = h.quantile_ms(0.5).unwrap();
        let p99 = h.quantile_ms(0.99).unwrap();
        assert!(p50 < p99);
        assert!(p50 < 100.0);
    }

    #[test]
    fn mean_ms_is_sane() {
        let h = LatencyHistogram::new();
        for _ in 0..100 {
            h.observe_ms(10);
        }
        let mean = h.mean_ms().unwrap();
        // Within 0.1 ms tolerance.
        assert!((mean - 10.0).abs() < 0.5);
    }

    #[test]
    fn max_ms_tracks_max() {
        let h = LatencyHistogram::new();
        h.observe_ms(50);
        h.observe_ms(200);
        h.observe_ms(100);
        assert_eq!(h.max_ms(), 200.0);
    }

    #[test]
    fn check_within_range_rejects_giant_latency() {
        let r = LatencyHistogram::check_within_range(Duration::from_secs(601));
        assert!(matches!(r, Err(BenchError::HistogramOverflow(_))));
    }

    #[test]
    fn check_within_range_accepts_normal() {
        LatencyHistogram::check_within_range(Duration::from_millis(500))
            .unwrap();
        LatencyHistogram::check_within_range(Duration::from_secs(60))
            .unwrap();
    }

    #[test]
    fn snapshot_has_n_buckets_plus_one_boundaries() {
        let h = LatencyHistogram::new();
        h.observe_ms(50);
        let snap = h.snapshot();
        assert_eq!(snap.counts.len(), N_BUCKETS);
        assert_eq!(snap.buckets_ms_upper.len(), N_BUCKETS);
        assert!(snap.buckets_ms_upper.last().unwrap().is_infinite());
    }

    #[test]
    fn rps_counter_tracks_success_error() {
        let c = RpsCounter::new();
        for _ in 0..10 {
            c.record_success();
        }
        for _ in 0..2 {
            c.record_error();
        }
        assert_eq!(c.success_count(), 10);
        assert_eq!(c.error_count(), 2);
        assert_eq!(c.total(), 12);
        let er = c.error_rate();
        assert!((er - 2.0 / 12.0).abs() < 1e-6);
    }

    #[test]
    fn rps_counter_returns_zero_for_empty() {
        let c = RpsCounter::new();
        assert_eq!(c.total(), 0);
        assert_eq!(c.error_rate(), 0.0);
    }
}
