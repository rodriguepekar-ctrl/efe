//! Sprint **D13-bis** — metric-instrumented OTLP span exporter.
//!
//! Cf. `docs/bench/d13bis_otel_sdk_real_architecture.md` § 12.4.
//!
//! Wraps a real `SpanExporter` (typically
//! `opentelemetry_otlp::SpanExporter` over tonic gRPC) and
//! observes each export via the D13-bis Prometheus instruments :
//!
//!   * `efe_otel_export_total{result}`
//!   * `efe_otel_export_duration_seconds{result}`
//!   * `efe_otel_batch_size`
//!   * `efe_otel_dropped_total{reason="exporter_error"}`
//!   * `efe_otel_attributes_filtered_total` (audit fix § A15)
//!
//! Without this wrapper, ops have **zero visibility** into the
//! OTel pipeline health — a Tempo OOM or NetworkPolicy block
//! drops every span silently.
//!
//! # Audit fix § A15 — PII filter authoritative gate
//!
//! Before forwarding the batch to the inner exporter, we call
//! [`filter_span_batch`] which strips every forbidden attribute
//! (PII / secret / unbounded-cardinality keys defined in
//! `spans::FORBIDDEN_ATTR_PREFIXES`) from the span / event /
//! link levels. Pre-audit, the filter existed but was never
//! invoked — handlers writing `email = ...` had their PII flow
//! straight to Tempo. Post-audit, this is the **authoritative
//! gate** that defends in depth.

use futures::future::BoxFuture;
use std::time::Instant;

use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};

use crate::metrics::d13bis_instruments::{
    otel_batch_size, otel_dropped_total, otel_export_duration_seconds,
    otel_export_total,
};
use crate::otel::attribute_filter::filter_span_batch;

/// `SpanExporter` wrapper that emits Prometheus metrics on each
/// call. Drops in transparently — `BatchSpanProcessor` doesn't
/// care that it's wrapped.
#[derive(Debug)]
pub struct MeasuringSpanExporter<E: SpanExporter> {
    inner: E,
}

impl<E: SpanExporter> MeasuringSpanExporter<E> {
    /// Wrap an inner exporter.
    pub fn wrap(inner: E) -> Self {
        Self { inner }
    }
}

impl<E: SpanExporter> SpanExporter for MeasuringSpanExporter<E> {
    fn export(
        &mut self,
        mut batch: Vec<SpanData>,
    ) -> BoxFuture<'static, ExportResult> {
        // **Audit fix § A15** — strip forbidden attributes
        // (PII / secrets / unbounded-cardinality keys) BEFORE
        // forwarding to the OTLP exporter. The filter mutates the
        // batch in place and bumps `efe_otel_attributes_filtered_total`.
        // Defense in depth : even if a handler dev forgets and
        // emits `email = %user.email`, the OTel pipeline sees a
        // span with that field stripped and `dropped_attributes_count`
        // incremented.
        let _stripped = filter_span_batch(&mut batch);

        let n = batch.len();
        // Batch size observation, captured BEFORE the move into the
        // inner exporter.
        otel_batch_size().observe(n as f64);
        let start = Instant::now();
        let inner_fut = self.inner.export(batch);
        Box::pin(async move {
            let result = inner_fut.await;
            let elapsed = start.elapsed();
            let label = if result.is_ok() { "ok" } else { "error" };
            otel_export_total().with_label_values(&[label]).inc();
            otel_export_duration_seconds()
                .with_label_values(&[label])
                .observe(elapsed.as_secs_f64());
            if result.is_err() {
                otel_dropped_total()
                    .with_label_values(&["exporter_error"])
                    .inc_by(n as u64);
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

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::trace::TraceError;

    /// Mock inner exporter that always succeeds.
    #[derive(Debug)]
    struct OkExporter;

    impl SpanExporter for OkExporter {
        fn export(
            &mut self,
            _batch: Vec<SpanData>,
        ) -> BoxFuture<'static, ExportResult> {
            Box::pin(async { Ok(()) })
        }
    }

    /// Mock inner exporter that always fails.
    #[derive(Debug)]
    struct FailExporter;

    impl SpanExporter for FailExporter {
        fn export(
            &mut self,
            _batch: Vec<SpanData>,
        ) -> BoxFuture<'static, ExportResult> {
            Box::pin(async {
                Err(TraceError::Other(
                    "mock exporter error".to_string().into(),
                ))
            })
        }
    }

    #[tokio::test]
    async fn ok_export_bumps_ok_counter() {
        let mut exp = MeasuringSpanExporter::wrap(OkExporter);
        let before = otel_export_total().with_label_values(&["ok"]).get();
        let res = exp.export(vec![]).await;
        assert!(res.is_ok());
        let after = otel_export_total().with_label_values(&["ok"]).get();
        // Shared global state ; allow concurrent observations.
        assert!(
            after >= before + 1,
            "ok counter should bump (was {before} → {after})"
        );
    }

    #[tokio::test]
    async fn err_export_bumps_error_counter_and_dropped() {
        let mut exp = MeasuringSpanExporter::wrap(FailExporter);
        let err_before =
            otel_export_total().with_label_values(&["error"]).get();
        let drop_before = otel_dropped_total()
            .with_label_values(&["exporter_error"])
            .get();
        // Empty batch → no drops counted (correct semantics).
        let res = exp.export(vec![]).await;
        assert!(res.is_err());
        let err_after =
            otel_export_total().with_label_values(&["error"]).get();
        assert!(err_after >= err_before + 1);
        // Drops counter stays equal because batch was empty.
        let drop_after = otel_dropped_total()
            .with_label_values(&["exporter_error"])
            .get();
        assert_eq!(drop_after, drop_before);
    }

    #[tokio::test]
    async fn batch_size_histogram_observed() {
        // The Prometheus global registry is shared across the test
        // suite ; another test in this module may observe between
        // our `before` snapshot and our `after` snapshot. Check
        // that we ADD at least one observation, not exactly one.
        let mut exp = MeasuringSpanExporter::wrap(OkExporter);
        let before = otel_batch_size().get_sample_count();
        let _ = exp.export(vec![]).await;
        let after = otel_batch_size().get_sample_count();
        assert!(
            after >= before + 1,
            "batch_size histogram should observe at least once (was {before} → {after})"
        );
    }

    // =================================================================
    // Audit fix § A15 — PII filter is authoritative gate
    // =================================================================

    /// Capturing exporter — records the batch it was handed so we
    /// can assert post-filter shape.
    #[derive(Debug, Default)]
    struct CapturingExporter {
        captured: std::sync::Arc<std::sync::Mutex<Vec<SpanData>>>,
    }

    impl SpanExporter for CapturingExporter {
        fn export(
            &mut self,
            batch: Vec<SpanData>,
        ) -> BoxFuture<'static, ExportResult> {
            let captured = self.captured.clone();
            Box::pin(async move {
                captured.lock().unwrap().extend(batch);
                Ok(())
            })
        }
    }

    fn make_test_span(attrs: Vec<opentelemetry::KeyValue>) -> SpanData {
        use opentelemetry::trace::{
            SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
        };
        use opentelemetry::InstrumentationScope;
        use opentelemetry_sdk::trace::{SpanEvents, SpanLinks};
        use std::borrow::Cow;
        use std::time::SystemTime;

        SpanData {
            span_context: SpanContext::new(
                TraceId::from_bytes([1u8; 16]),
                SpanId::from_bytes([1u8; 8]),
                TraceFlags::default(),
                false,
                TraceState::default(),
            ),
            parent_span_id: SpanId::INVALID,
            span_kind: SpanKind::Server,
            name: Cow::Borrowed("audit-fix-a15"),
            start_time: SystemTime::now(),
            end_time: SystemTime::now(),
            attributes: attrs,
            dropped_attributes_count: 0,
            events: SpanEvents::default(),
            links: SpanLinks::default(),
            status: Status::Unset,
            instrumentation_scope: InstrumentationScope::builder("efe").build(),
        }
    }

    #[tokio::test]
    async fn pii_attributes_stripped_before_inner_exporter() {
        // **Audit fix § A15 regression** : before this fix,
        // `MeasuringSpanExporter::export` forwarded the batch
        // unchanged. PII reached Tempo. Post-fix, the inner
        // exporter MUST receive a batch with email/password
        // already stripped.
        use opentelemetry::KeyValue;

        let captor = CapturingExporter::default();
        let captured_handle = captor.captured.clone();
        let mut exp = MeasuringSpanExporter::wrap(captor);

        let batch = vec![make_test_span(vec![
            KeyValue::new("tenant", "acme"),
            KeyValue::new("email", "alice@example.com"),
            KeyValue::new("password", "hunter2"),
            KeyValue::new("n_classes", 5_i64),
        ])];

        let _ = exp.export(batch).await;

        let captured = captured_handle.lock().unwrap();
        assert_eq!(captured.len(), 1);
        let kept_keys: Vec<&str> = captured[0]
            .attributes
            .iter()
            .map(|kv| kv.key.as_str())
            .collect();
        // PII / secrets MUST be absent.
        assert!(
            !kept_keys.contains(&"email"),
            "email still present in exported batch: {kept_keys:?}"
        );
        assert!(
            !kept_keys.contains(&"password"),
            "password still present in exported batch: {kept_keys:?}"
        );
        // Allowed attrs must be preserved.
        assert!(kept_keys.contains(&"tenant"));
        assert!(kept_keys.contains(&"n_classes"));
        // dropped_attributes_count must be bumped so collectors can
        // surface the strip in their metadata.
        assert_eq!(captured[0].dropped_attributes_count, 2);
    }
}
