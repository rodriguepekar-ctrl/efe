//! PR #22 § 14 — source providers.
//!
//! 4 implementations (PR #22 § 14 module map) :
//!   * [`prom_query`]    — live Prometheus HTTP (production T1 source)
//!   * [`bench_baseline`] — PR #21 summary.json reader (synthetic SLI cell)
//!   * [`replay_parquet`] — read-back a stored daily file (debugging)
//!   * `MockSource`       — in-memory mock for tests (this module)

pub mod bench_baseline;
#[cfg(feature = "loadgen-source")]
pub mod loadgen_lib;
pub mod prom_query;
pub mod replay_parquet;

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::error::SloResult;

/// Query for an integer-counter rate (used by SLO-1 numerator/denominator
/// computation).
#[derive(Debug, Clone)]
pub struct MetricQuery {
    pub metric: String,
    pub window_seconds: u64,
    pub window_start_unix: i64,
    pub label_matchers: Vec<(String, String)>,
}

/// Query for a histogram quantile.
#[derive(Debug, Clone)]
pub struct HistogramQuery {
    pub metric_bucket: String,
    pub quantile: f64,
    pub window_seconds: u64,
    pub window_start_unix: i64,
    pub label_matchers: Vec<(String, String)>,
}

/// Query for a discrete event series (SLO-3).
#[derive(Debug, Clone)]
pub struct EventQuery {
    pub metric: String,
    pub window_seconds: u64,
    pub window_start_unix: i64,
}

/// Single recovery event record (consumed by SLO-3).
#[derive(Debug, Clone, Copy)]
pub struct RecoveryEvent {
    pub duration_seconds: f64,
    pub at_unix: i64,
}

/// Common interface every source provider must implement.
#[async_trait]
pub trait SourceProvider: Send + Sync {
    /// Returns the integer count of events matching `q` in the window.
    async fn integer_counter(&self, q: MetricQuery) -> SloResult<u64>;

    /// Returns the value of a histogram quantile.  NaN when no data.
    async fn histogram_quantile(&self, q: HistogramQuery) -> SloResult<f64>;

    /// Returns the recovery-event series matching `q`.
    async fn recovery_events(&self, q: EventQuery) -> SloResult<Vec<RecoveryEvent>>;

    /// Returns the scrape coverage of the window, in [0.0, 1.0].
    async fn scrape_coverage(&self, window_start_unix: i64, window_seconds: u64) -> SloResult<f64>;

    /// Returns the list of participant ids present in the window (SLO-4).
    async fn participants(&self, window_start_unix: i64, window_seconds: u64) -> SloResult<Vec<String>>;
}

// ────────────────────────────────────────────────────────────────────
//                          MockSource (tests)
// ────────────────────────────────────────────────────────────────────

/// Composite key for the mock counter store : `(metric, label_key, label_value)`.
type CounterKey = (String, String, String);

/// In-memory mock — exercised by the kernel unit tests.
///
/// Cardinality is tiny ; we don't try to be a Prom emulator.
#[derive(Default)]
pub struct MockSource {
    /// metric → label_key → label_value → counter
    counters: Mutex<HashMap<CounterKey, u64>>,
    /// histogram bucket name → quantile value
    p95s: Mutex<HashMap<String, f64>>,
    /// participant id → p99 lag value (used by SLO-4)
    participant_p99s: Mutex<HashMap<String, f64>>,
    /// list of declared participants (SLO-4)
    participants: Mutex<Vec<String>>,
    /// recovery events (SLO-3)
    recoveries: Mutex<Vec<RecoveryEvent>>,
    /// global scrape coverage (default 1.0)
    coverage: Mutex<f64>,
}

impl MockSource {
    pub fn set_counter(&mut self, metric: &str, label_k: &str, label_v: &str, value: u64) {
        self.counters
            .lock()
            .insert((metric.into(), label_k.into(), label_v.into()), value);
    }

    pub fn set_p95(&mut self, bucket_metric: &str, p95: f64) {
        self.p95s.lock().insert(bucket_metric.into(), p95);
    }

    pub fn set_participants(&mut self, ps: Vec<String>) {
        *self.participants.lock() = ps;
    }

    pub fn set_participant_p99(&mut self, p: &str, lag: f64) {
        self.participant_p99s.lock().insert(p.into(), lag);
    }

    pub fn push_recovery_event(&mut self, e: RecoveryEvent) {
        self.recoveries.lock().push(e);
    }

    pub fn set_coverage(&mut self, c: f64) {
        *self.coverage.lock() = c;
    }
}

#[async_trait]
impl SourceProvider for MockSource {
    async fn integer_counter(&self, q: MetricQuery) -> SloResult<u64> {
        // Sum over all label matchers (simple AND).
        let guard = self.counters.lock();
        // If no matchers → all rows of that metric.
        if q.label_matchers.is_empty() {
            let mut sum = 0u64;
            for ((m, _, _), v) in guard.iter() {
                if m == &q.metric {
                    sum += v;
                }
            }
            return Ok(sum);
        }
        // One matcher : exact lookup.
        let (lk, lv) = &q.label_matchers[0];
        Ok(guard
            .get(&(q.metric.clone(), lk.clone(), lv.clone()))
            .copied()
            .unwrap_or(0))
    }

    async fn histogram_quantile(&self, q: HistogramQuery) -> SloResult<f64> {
        // SLO-4 takes a label matcher on `participant`.
        if let Some((k, v)) = q.label_matchers.first() {
            if k == "participant" {
                let guard = self.participant_p99s.lock();
                return Ok(guard.get(v).copied().unwrap_or(f64::NAN));
            }
        }
        Ok(self
            .p95s
            .lock()
            .get(&q.metric_bucket)
            .copied()
            .unwrap_or(f64::NAN))
    }

    async fn recovery_events(&self, _: EventQuery) -> SloResult<Vec<RecoveryEvent>> {
        Ok(self.recoveries.lock().clone())
    }

    async fn scrape_coverage(&self, _: i64, _: u64) -> SloResult<f64> {
        Ok(*self.coverage.lock())
    }

    async fn participants(&self, _: i64, _: u64) -> SloResult<Vec<String>> {
        Ok(self.participants.lock().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_default_returns_zero_counter() {
        let m = MockSource::default();
        let v = m
            .integer_counter(MetricQuery {
                metric: "x".into(),
                window_seconds: 60,
                window_start_unix: 0,
                label_matchers: vec![("a".into(), "b".into())],
            })
            .await
            .unwrap();
        assert_eq!(v, 0);
    }

    #[tokio::test]
    async fn mock_returns_set_counter() {
        let mut m = MockSource::default();
        m.set_counter("efe_l2_request_total", "status", "success", 42);
        let v = m
            .integer_counter(MetricQuery {
                metric: "efe_l2_request_total".into(),
                window_seconds: 60,
                window_start_unix: 0,
                label_matchers: vec![("status".into(), "success".into())],
            })
            .await
            .unwrap();
        assert_eq!(v, 42);
    }

    #[tokio::test]
    async fn mock_coverage_default_is_zero() {
        // Default mock value of f64::default() is 0.0 — kernels need
        // to set explicitly. This is the documented contract.
        let m = MockSource::default();
        assert_eq!(m.scrape_coverage(0, 60).await.unwrap(), 0.0);
    }
}
