//! PR #18 § 17.3 — exporter health Prometheus metrics.
//!
//! Three series complementing PR #17's 14-metric surface :
//!
//!   * `efe_meta_swarm_otel_exported_spans_total` (Counter)
//!     — incremented after every successful OTLP batch export.
//!   * `efe_meta_swarm_otel_dropped_spans_total{reason}` (CounterVec)
//!     — incremented on queue overflow, export failure, shutdown,
//!     timeout.
//!   * `efe_meta_swarm_otel_export_duration_seconds` (Histogram)
//!     — wall-clock duration of one OTLP batch export.
//!
//! These series back the 3 PrometheusRule alerts from § 25.4 of the
//! architecture doc :
//!
//!   * `MetaSwarmOtelExporterDown` — no exports in 5 min while
//!     traffic is flowing.
//!   * `MetaSwarmOtelDropRateHigh` — drop rate > 1 %.
//!   * `MetaSwarmOtelSamplingDrift` — observed sample rate diverges
//!     > 50 % from target.

use prometheus::{
    register_histogram_with_registry, register_int_counter_vec_with_registry,
    register_int_counter_with_registry, Histogram, HistogramOpts, IntCounter, IntCounterVec,
    Registry,
};
use std::time::Duration;

/// Histogram bucket policy for export duration : 1ms → 5s, 11
/// buckets.  Matches the BatchSpanProcessor's `max_export_timeout=5s`
/// budget.
pub const OTEL_EXPORT_DURATION_BUCKETS_SECONDS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
];

/// Closed taxonomy for the `reason` label on
/// `efe_meta_swarm_otel_dropped_spans_total`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OtelExporterReason {
    /// Batch queue was full ; oldest span dropped.
    QueueFull,
    /// OTLP export returned an error (network, TLS, deadline).
    ExportError,
    /// Provider shutdown drained pending batches with timeout.
    Shutdown,
    /// Per-export timeout exceeded (slow collector).
    Timeout,
}

impl OtelExporterReason {
    /// Lowercase snake-case Prometheus label form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QueueFull => "queue_full",
            Self::ExportError => "export_error",
            Self::Shutdown => "shutdown",
            Self::Timeout => "timeout",
        }
    }

    /// All 4 variants — used by [`OtelExporterMetrics::register`] to
    /// pre-touch the label space.
    #[must_use]
    pub fn all() -> &'static [Self] {
        &[
            Self::QueueFull,
            Self::ExportError,
            Self::Shutdown,
            Self::Timeout,
        ]
    }
}

/// Bundle of all 3 PR #18 exporter-health metrics.  Cheap to clone.
#[derive(Debug, Clone)]
pub struct OtelExporterMetrics {
    /// `efe_meta_swarm_otel_exported_spans_total` — total spans
    /// successfully exported (across batches).
    pub exported_spans_total: IntCounter,
    /// `efe_meta_swarm_otel_dropped_spans_total{reason}` — total
    /// spans dropped, partitioned by reason.
    pub dropped_spans_total: IntCounterVec,
    /// `efe_meta_swarm_otel_export_duration_seconds` — histogram of
    /// per-batch export wall-clock duration.
    pub export_duration_seconds: Histogram,
}

impl OtelExporterMetrics {
    /// Register all 3 series on `reg` + pre-touch the closed-taxonomy
    /// `reason` label.
    pub fn register(reg: &Registry) -> prometheus::Result<Self> {
        let exported_spans_total = register_int_counter_with_registry!(
            "efe_meta_swarm_otel_exported_spans_total",
            "Total OpenTelemetry spans successfully exported to the OTLP collector.",
            reg,
        )?;
        let dropped_spans_total = register_int_counter_vec_with_registry!(
            "efe_meta_swarm_otel_dropped_spans_total",
            "Total OpenTelemetry spans dropped before reaching the OTLP collector, partitioned by reason.",
            &["reason"],
            reg,
        )?;
        let export_duration_seconds = register_histogram_with_registry!(
            HistogramOpts::new(
                "efe_meta_swarm_otel_export_duration_seconds",
                "Wall-clock duration of one OTLP batch export, in seconds.",
            )
            .buckets(OTEL_EXPORT_DURATION_BUCKETS_SECONDS.to_vec()),
            reg,
        )?;

        // Pre-touch every reason label so the series shows up in
        // /metrics at boot (AC5 / PR #17 § 15 pattern).
        for r in OtelExporterReason::all() {
            dropped_spans_total
                .with_label_values(&[r.as_str()])
                .inc_by(0);
        }

        Ok(Self {
            exported_spans_total,
            dropped_spans_total,
            export_duration_seconds,
        })
    }

    /// Record a successful batch export.
    pub fn record_export_success(&self, span_count: u64, duration: Duration) {
        self.exported_spans_total.inc_by(span_count);
        self.export_duration_seconds.observe(duration.as_secs_f64());
    }

    /// Record a drop.
    pub fn record_drop(&self, reason: OtelExporterReason, span_count: u64) {
        self.dropped_spans_total
            .with_label_values(&[reason.as_str()])
            .inc_by(span_count);
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::TextEncoder;

    fn body(reg: &Registry) -> String {
        TextEncoder::new()
            .encode_to_string(&reg.gather())
            .expect("encode")
    }

    #[test]
    fn register_creates_all_three_series() {
        let reg = Registry::new();
        let _m = OtelExporterMetrics::register(&reg).expect("register");
        let b = body(&reg);
        for n in [
            "efe_meta_swarm_otel_exported_spans_total",
            "efe_meta_swarm_otel_dropped_spans_total",
            "efe_meta_swarm_otel_export_duration_seconds",
        ] {
            assert!(b.contains(n), "missing {n}");
        }
    }

    #[test]
    fn pretouch_emits_all_four_reasons() {
        let reg = Registry::new();
        let _m = OtelExporterMetrics::register(&reg).expect("register");
        let b = body(&reg);
        for r in OtelExporterReason::all() {
            let needle = format!(
                "efe_meta_swarm_otel_dropped_spans_total{{reason=\"{}\"}} 0",
                r.as_str()
            );
            assert!(b.contains(&needle), "missing pretouch : {needle}\n{b}");
        }
    }

    #[test]
    fn record_export_success_increments_counter_and_observes_histogram() {
        let reg = Registry::new();
        let m = OtelExporterMetrics::register(&reg).expect("register");
        m.record_export_success(42, Duration::from_millis(15));
        assert_eq!(m.exported_spans_total.get(), 42);
        assert_eq!(m.export_duration_seconds.get_sample_count(), 1);
    }

    #[test]
    fn record_drop_increments_per_reason() {
        let reg = Registry::new();
        let m = OtelExporterMetrics::register(&reg).expect("register");
        m.record_drop(OtelExporterReason::QueueFull, 5);
        m.record_drop(OtelExporterReason::QueueFull, 3);
        m.record_drop(OtelExporterReason::Timeout, 1);
        let qf = m
            .dropped_spans_total
            .get_metric_with_label_values(&["queue_full"])
            .unwrap()
            .get();
        let to = m
            .dropped_spans_total
            .get_metric_with_label_values(&["timeout"])
            .unwrap()
            .get();
        assert_eq!(qf, 8);
        assert_eq!(to, 1);
    }

    #[test]
    fn reason_label_strings_are_stable() {
        assert_eq!(OtelExporterReason::QueueFull.as_str(), "queue_full");
        assert_eq!(OtelExporterReason::ExportError.as_str(), "export_error");
        assert_eq!(OtelExporterReason::Shutdown.as_str(), "shutdown");
        assert_eq!(OtelExporterReason::Timeout.as_str(), "timeout");
    }

    #[test]
    fn export_duration_buckets_match_architecture() {
        assert_eq!(OTEL_EXPORT_DURATION_BUCKETS_SECONDS.len(), 11);
    }
}
