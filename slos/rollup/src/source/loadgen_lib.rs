//! PR #22 § 18 — in-process loadgen source (feature-gated).
//!
//! When compiled with `--features loadgen-source`, the rollup binary
//! can consume `efe-meta-swarm-loadgen` as a library and aggregate the
//! synthetic traffic it generates into `source=synthetic_loadgen` SLI
//! cells (AD18 — strict separation from `source=prod`).
//!
//! Phase-2 ships a **minimal** implementation : it exposes a
//! [`LoadgenLibSource`] that owns an in-memory buffer of synthetic
//! events fed by the test harness ; full streaming-callback integration
//! (the `SloSourceSink` trait described in arch doc § 18.1) is a
//! Phase-3 follow-up (G.10).
//!
//! NB : without the feature flag this module is empty.

#![cfg(feature = "loadgen-source")]

use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::SloResult;
use crate::source::{EventQuery, HistogramQuery, MetricQuery, RecoveryEvent, SourceProvider};

/// In-process source backed by an event buffer fed by the loadgen
/// library.  Use [`Self::push_*`] to feed events from the loadgen
/// callback.
#[derive(Default)]
pub struct LoadgenLibSource {
    success_count: Mutex<u64>,
    server_error_count: Mutex<u64>,
    transport_error_count: Mutex<u64>,
    latencies_ms: Mutex<Vec<f64>>,
    recovery_events: Mutex<Vec<RecoveryEvent>>,
}

impl LoadgenLibSource {
    /// Record a request outcome (called from the loadgen callback).
    pub fn record_request(&self, success: bool, status_class: RequestStatusClass, latency_ms: f64) {
        if success {
            *self.success_count.lock().unwrap() += 1;
        } else {
            match status_class {
                RequestStatusClass::ServerError => *self.server_error_count.lock().unwrap() += 1,
                RequestStatusClass::TransportError => *self.transport_error_count.lock().unwrap() += 1,
                RequestStatusClass::ClientError => { /* T96 — excluded */ }
            }
        }
        self.latencies_ms.lock().unwrap().push(latency_ms);
    }

    /// Record a chaos-recovery event.
    pub fn record_recovery(&self, duration_seconds: f64, at_unix: i64) {
        self.recovery_events
            .lock()
            .unwrap()
            .push(RecoveryEvent { duration_seconds, at_unix });
    }
}

#[derive(Debug, Clone, Copy)]
pub enum RequestStatusClass {
    ServerError,
    TransportError,
    ClientError,
}

#[async_trait]
impl SourceProvider for LoadgenLibSource {
    async fn integer_counter(&self, q: MetricQuery) -> SloResult<u64> {
        if let Some((k, v)) = q.label_matchers.first() {
            if k == "status" {
                let n = match v.as_str() {
                    "success" => *self.success_count.lock().unwrap(),
                    "server_error" => *self.server_error_count.lock().unwrap(),
                    "transport_error" => *self.transport_error_count.lock().unwrap(),
                    _ => 0,
                };
                return Ok(n);
            }
        }
        let total = *self.success_count.lock().unwrap()
            + *self.server_error_count.lock().unwrap()
            + *self.transport_error_count.lock().unwrap();
        Ok(total)
    }

    async fn histogram_quantile(&self, q: HistogramQuery) -> SloResult<f64> {
        let mut lats = self.latencies_ms.lock().unwrap().clone();
        if lats.is_empty() {
            return Ok(f64::NAN);
        }
        lats.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((q.quantile.clamp(0.0, 1.0) * (lats.len() - 1) as f64).round() as usize).min(lats.len() - 1);
        Ok(lats[idx] / 1000.0)  // ms → s
    }

    async fn recovery_events(&self, _q: EventQuery) -> SloResult<Vec<RecoveryEvent>> {
        Ok(self.recovery_events.lock().unwrap().clone())
    }

    async fn scrape_coverage(&self, _: i64, _: u64) -> SloResult<f64> {
        // Synthetic source is always at full coverage by construction.
        Ok(1.0)
    }

    async fn participants(&self, _: i64, _: u64) -> SloResult<Vec<String>> {
        // SLO-4 doesn't apply to synthetic source ; surface empty.
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn records_and_queries_counters() {
        let s = LoadgenLibSource::default();
        s.record_request(true, RequestStatusClass::ServerError, 10.0);
        s.record_request(false, RequestStatusClass::ServerError, 5.0);
        s.record_request(false, RequestStatusClass::ClientError, 100.0);

        let succ = s.integer_counter(MetricQuery {
            metric: "x".into(),
            window_seconds: 60,
            window_start_unix: 0,
            label_matchers: vec![("status".into(), "success".into())],
        }).await.unwrap();
        assert_eq!(succ, 1);

        let serr = s.integer_counter(MetricQuery {
            metric: "x".into(),
            window_seconds: 60,
            window_start_unix: 0,
            label_matchers: vec![("status".into(), "server_error".into())],
        }).await.unwrap();
        assert_eq!(serr, 1);

        // T96 : client error is recorded for latency but NOT counted.
        let cerr = s.integer_counter(MetricQuery {
            metric: "x".into(),
            window_seconds: 60,
            window_start_unix: 0,
            label_matchers: vec![("status".into(), "client_error".into())],
        }).await.unwrap();
        assert_eq!(cerr, 0);
    }

    #[tokio::test]
    async fn latencies_are_in_seconds() {
        let s = LoadgenLibSource::default();
        for i in 1..=100 {
            s.record_request(true, RequestStatusClass::ServerError, i as f64);
        }
        let p95 = s.histogram_quantile(HistogramQuery {
            metric_bucket: "x".into(),
            quantile: 0.95,
            window_seconds: 60,
            window_start_unix: 0,
            label_matchers: vec![],
        }).await.unwrap();
        // 95th of 1..100 ≈ 95 ms ⇒ 0.095 s
        assert!((p95 - 0.095).abs() < 0.01);
    }
}
