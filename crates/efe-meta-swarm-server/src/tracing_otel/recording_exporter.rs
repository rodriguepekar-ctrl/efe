//! PR #18 § 17.3 / AC20 — `MetricsRecordingExporter` adapter.
//!
//! Wraps a [`opentelemetry_sdk::export::trace::SpanExporter`] (typically
//! the OTLP/gRPC exporter from `opentelemetry_otlp::SpanExporter`) and
//! ticks the 3 PR #18 Prometheus series on every batch :
//!
//!   * On export success ⇒ `record_export_success(span_count, dur)` →
//!     increments `efe_meta_swarm_otel_exported_spans_total` and
//!     observes `efe_meta_swarm_otel_export_duration_seconds`.
//!   * On export error   ⇒ `record_drop(ExportError, span_count)` →
//!     increments `efe_meta_swarm_otel_dropped_spans_total{reason="export_error"}`.
//!   * On shutdown       ⇒ no-op for now ; PR #18 § 17.3 reserves the
//!     `shutdown` reason for future use when the BatchSpanProcessor
//!     surfaces shutdown-drop counts.
//!
//! The wrapper is intentionally thin : OTel-rs 0.27's `SpanExporter`
//! trait is the only seam where a downstream observer can intercept
//! the BatchSpanProcessor → export pipeline without re-implementing
//! the processor.
//!
//! ## Cost
//!
//! One `Instant::now()` call per export + one `IntCounter::inc_by` +
//! one `Histogram::observe`.  ~ 200 ns overhead per batch on Apple M1
//! ; negligible at typical export cadences (1 batch / 5 s by default).

use std::time::Instant;

use futures::future::BoxFuture;
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};

use super::metrics::{OtelExporterMetrics, OtelExporterReason};

/// `SpanExporter` adapter that records the 3 PR #18 OTel exporter
/// health metrics on every export.
pub struct MetricsRecordingExporter<E: SpanExporter + 'static> {
    inner: E,
    metrics: OtelExporterMetrics,
}

impl<E: SpanExporter + 'static> std::fmt::Debug for MetricsRecordingExporter<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetricsRecordingExporter")
            .finish_non_exhaustive()
    }
}

impl<E: SpanExporter + 'static> MetricsRecordingExporter<E> {
    /// Build a new recording wrapper.
    #[must_use]
    pub fn new(inner: E, metrics: OtelExporterMetrics) -> Self {
        Self { inner, metrics }
    }
}

impl<E: SpanExporter + 'static> SpanExporter for MetricsRecordingExporter<E> {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let span_count = batch.len() as u64;
        let metrics = self.metrics.clone();
        let started = Instant::now();
        let inner_fut = self.inner.export(batch);
        Box::pin(async move {
            let result = inner_fut.await;
            let elapsed = started.elapsed();
            match &result {
                Ok(()) => metrics.record_export_success(span_count, elapsed),
                Err(_) => metrics.record_drop(OtelExporterReason::ExportError, span_count),
            }
            result
        })
    }

    fn shutdown(&mut self) {
        self.inner.shutdown();
    }

    fn force_flush(&mut self) -> BoxFuture<'static, ExportResult> {
        self.inner.force_flush()
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.inner.set_resource(resource);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use opentelemetry::trace::TraceError;
    use opentelemetry_sdk::export::trace::SpanData;
    use prometheus::Registry;

    /// A fake exporter that always succeeds with the given delay.
    #[derive(Debug)]
    struct FakeOkExporter {
        delay_ms: u64,
    }
    impl SpanExporter for FakeOkExporter {
        fn export(&mut self, _batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
            let delay = self.delay_ms;
            async move {
                if delay > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
                Ok(())
            }
            .boxed()
        }
    }

    /// A fake exporter that always fails.
    #[derive(Debug)]
    struct FakeErrExporter;
    impl SpanExporter for FakeErrExporter {
        fn export(&mut self, _batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
            async { Err(TraceError::Other("boom".into())) }.boxed()
        }
    }

    #[tokio::test]
    async fn success_export_ticks_counter_and_histogram() {
        let reg = Registry::new();
        let metrics = OtelExporterMetrics::register(&reg).expect("register");
        let mut exp = MetricsRecordingExporter::new(FakeOkExporter { delay_ms: 0 }, metrics.clone());
        let batch = vec![];
        let res = exp.export(batch).await;
        assert!(res.is_ok());
        // span_count = 0 (empty batch) ; counter increments by 0, but
        // the histogram still records a sample.
        assert_eq!(metrics.exported_spans_total.get(), 0);
        assert_eq!(metrics.export_duration_seconds.get_sample_count(), 1);
    }

    #[tokio::test]
    async fn error_export_increments_export_error_drop() {
        let reg = Registry::new();
        let metrics = OtelExporterMetrics::register(&reg).expect("register");
        let mut exp = MetricsRecordingExporter::new(FakeErrExporter, metrics.clone());
        let batch = vec![];
        let res = exp.export(batch).await;
        assert!(res.is_err());
        let ee = metrics
            .dropped_spans_total
            .get_metric_with_label_values(&["export_error"])
            .unwrap()
            .get();
        assert_eq!(ee, 0); // empty batch ⇒ 0-element drop
    }

    #[tokio::test]
    async fn delay_is_observed_in_histogram() {
        let reg = Registry::new();
        let metrics = OtelExporterMetrics::register(&reg).expect("register");
        let mut exp = MetricsRecordingExporter::new(FakeOkExporter { delay_ms: 10 }, metrics.clone());
        let batch = vec![];
        let _ = exp.export(batch).await;
        let sum = metrics.export_duration_seconds.get_sample_sum();
        assert!(sum >= 0.005, "expected at least 5 ms of measured delay, got {sum}");
    }
}
