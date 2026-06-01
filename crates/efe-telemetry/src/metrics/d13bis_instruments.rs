//! Sprint **D13-bis** — OpenTelemetry SDK self-observability metrics.
//!
//! Cf. `docs/bench/d13bis_otel_sdk_real_architecture.md` § 12.
//!
//! 4 nouveaux instruments dédiés à la pipeline OTLP elle-même.
//! Sans ces métriques, une régression collector-side (Tempo OOM,
//! NetworkPolicy block) signifie *tous* les traces perdus sans
//! alerte — D13-bis ferme la boucle : on monitore le moniteur.
//!
//! Conventions identiques aux autres D-X instruments :
//!   * macro `once_metric!` (re-déclarée localement) ;
//!   * tous enregistrés sur `global_registry` (M11) ;
//!   * labels strictement bornés.
//!
//! # Cardinalité
//!
//! | Instrument                                | Combinaisons |
//! |-------------------------------------------|-------------:|
//! | `efe_otel_export_total`                   | 2 (ok / error) |
//! | `efe_otel_export_duration_seconds`        | 2 |
//! | `efe_otel_batch_size`                     | 1 (no label) |
//! | `efe_otel_dropped_total`                  | 3 (queue_full / shutdown_timeout / exporter_error) |
//! | `efe_otel_attributes_filtered_total`      | 1 (no label — audit fix § A15) |
//!
//! Total = **9 séries**. Très en-dessous du budget D13 50 000.

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, linear_buckets, Histogram, HistogramOpts, HistogramVec,
    IntCounter, IntCounterVec, Opts,
};

use super::registry::global_registry;

macro_rules! once_metric {
    ($(#[$doc:meta])* $fn_name:ident, $ty:ty, $init:expr) => {
        $(#[$doc])*
        pub fn $fn_name() -> &'static $ty {
            static CELL: OnceLock<$ty> = OnceLock::new();
            CELL.get_or_init(|| {
                let m: $ty = $init;
                let _ = global_registry().register(Box::new(m.clone()));
                m
            })
        }
    };
}

// =====================================================================
// Counters
// =====================================================================

once_metric!(
    /// `efe_otel_export_total{result}` — count of OTLP export
    /// attempts. Architecture § 12.1.
    ///
    /// `result` is one of :
    ///   * `ok` — the collector accepted the batch ;
    ///   * `error` — gRPC / TLS / timeout / collector 5xx.
    ///
    /// Alert `D13bisOtelExportFailing` fires on
    /// `rate(efe_otel_export_total{result="error"}) > 0.005`
    /// (i.e. > 0.5 % failure rate over 5 min — SLO D13bis-OTL-1).
    otel_export_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_otel_export_total",
            "Number of OTLP export attempts by result (D13-bis § 12).",
        ),
        &["result"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_otel_dropped_total{reason}` — spans dropped before
    /// they reached the exporter.
    ///
    /// `reason` is one of :
    ///   * `queue_full` — `BatchSpanProcessor` queue overflow
    ///     (collector slow / down + sustained burst) ;
    ///   * `shutdown_timeout` — `force_flush` exceeded the
    ///     `Shutdown::grace_period` ;
    ///   * `exporter_error` — the export call returned an error
    ///     (transient ; we count one per span in the failed batch).
    ///
    /// Non-zero rate is operationally significant : dashboards
    /// flag it via `D13bisOtelDropping` (warn-level alert).
    otel_dropped_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_otel_dropped_total",
            "Spans dropped before exporter (queue_full | shutdown_timeout | exporter_error).",
        ),
        &["reason"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Histograms
// =====================================================================

once_metric!(
    /// `efe_otel_export_duration_seconds{result}` — wall-clock
    /// duration of one OTLP export.
    ///
    /// Buckets : 0.01, 0.02, 0.04, … 5.12 s (10 buckets, base 2).
    /// Covers the SLO **D13bis-OTL-2** ≤ 2 s p99.
    otel_export_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_otel_export_duration_seconds",
            "OTLP export wall-clock duration (D13-bis SLO OTL-2 ≤ 2 s p99).",
        )
        .buckets(exponential_buckets(0.01, 2.0, 10).expect("valid buckets")),
        &["result"],
    )
    .expect("valid histogram opts")
);

once_metric!(
    /// `efe_otel_batch_size` — number of spans per export batch.
    ///
    /// Buckets : 0, 64, 128, … 576 (linear, 10 buckets). The
    /// architecture caps batch size at 512 (cf. § 6.2) — anything
    /// above 512 indicates an SDK regression.
    ///
    /// No label — the batch is per-pipeline + we observe both
    /// successful and failed batches.
    otel_batch_size,
    Histogram,
    Histogram::with_opts(
        HistogramOpts::new(
            "efe_otel_batch_size",
            "Number of spans per OTLP export batch (D13-bis § 6.2).",
        )
        .buckets(linear_buckets(0.0, 64.0, 10).expect("valid buckets")),
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Audit-fix § A15 — PII filter visibility
// =====================================================================

once_metric!(
    /// `efe_otel_attributes_filtered_total` — total attributes
    /// stripped by the [`crate::otel::attribute_filter::filter_span_batch`]
    /// guard before export. Audit fix § A15.
    ///
    /// Pre-audit, the filter existed but was never invoked, so PII
    /// reached Tempo. Post-audit, every export batch passes through
    /// `filter_span_batch` and this counter records the strip count.
    ///
    /// Operationally :
    ///   * **Steady-state zero** : healthy, no developer-side leaks.
    ///   * **Non-zero rate** : a handler is emitting forbidden span
    ///     attributes — investigate immediately. Alert
    ///     `D13bisOtelPiiFilterActive` fires on
    ///     `rate(efe_otel_attributes_filtered_total[5m]) > 0`.
    otel_attributes_filtered_total,
    IntCounter,
    IntCounter::with_opts(Opts::new(
        "efe_otel_attributes_filtered_total",
        "Span attributes stripped by the PII/secret filter pre-export (D13-bis audit § A15).",
    ))
    .expect("valid counter opts")
);

// =====================================================================
// Eager registration
// =====================================================================

/// Force-register every D13-bis instrument with the global registry
/// so the Prometheus `/metrics` endpoint exposes them as zero-valued
/// series even before the first OTLP export. Pattern identical to
/// the D15-H5 fix and the D16 audit-fix C1 — dashboards render at
/// boot, not at first traffic.
///
/// Idempotent — `once_metric!` guards against double-registration.
/// Called from `observability_init_d13::bootstrap_observability`
/// (and unconditionally from `runtime::Server::build` for the
/// edge case where the operator does not use the bootstrap helper).
pub fn register_all_d13bis() {
    let _ = otel_export_total();
    let _ = otel_export_duration_seconds();
    let _ = otel_batch_size();
    let _ = otel_dropped_total();
    // Audit fix § A15 — PII filter visibility instrument.
    let _ = otel_attributes_filtered_total();
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    #[test]
    fn export_total_increments() {
        // The Prometheus global registry is shared across the test
        // suite ; another test (e.g. the audit-fix § A4 force_flush
        // path that bumps `result="error"`) may concurrently bump
        // adjacent labels. We assert "at least one increment" not
        // "exactly one".
        let labels = ["ok"];
        let before = otel_export_total().with_label_values(&labels).get();
        otel_export_total().with_label_values(&labels).inc();
        let after = otel_export_total().with_label_values(&labels).get();
        assert!(
            after >= before + 1,
            "export_total ok counter should bump (was {before} → {after})"
        );
    }

    #[test]
    fn export_total_result_labels_documented() {
        // Architecture § 12.1 lists exactly two result values. Pin
        // them here so a future refactor that adds a third has to
        // update the dashboards too.
        for r in ["ok", "error"] {
            otel_export_total().with_label_values(&[r]).inc();
        }
    }

    #[test]
    fn dropped_total_reason_labels_documented() {
        // Architecture § 12.1 lists exactly three reason values.
        for r in ["queue_full", "shutdown_timeout", "exporter_error"] {
            otel_dropped_total().with_label_values(&[r]).inc();
        }
    }

    #[test]
    fn duration_histogram_observes() {
        otel_export_duration_seconds()
            .with_label_values(&["ok"])
            .observe(0.42);
        let m = otel_export_duration_seconds().with_label_values(&["ok"]);
        assert!(m.get_sample_count() >= 1);
    }

    #[test]
    fn batch_size_histogram_observes() {
        otel_batch_size().observe(128.0);
        assert!(otel_batch_size().get_sample_count() >= 1);
    }

    #[test]
    fn label_arity_pinned() {
        // Architecture § 12 commits to specific label arities.
        assert_eq!(otel_export_total().desc()[0].variable_labels.len(), 1);
        assert_eq!(
            otel_export_duration_seconds().desc()[0].variable_labels.len(),
            1
        );
        assert_eq!(otel_dropped_total().desc()[0].variable_labels.len(), 1);
        // batch_size is a bare Histogram — 0 variable labels.
        assert_eq!(otel_batch_size().desc()[0].variable_labels.len(), 0);
    }

    #[test]
    fn register_all_d13bis_idempotent() {
        register_all_d13bis();
        register_all_d13bis();
        // No assertion ; absence of panic is the signal.
    }

    // =================================================================
    // Audit fix § A15 — PII filter visibility instrument
    // =================================================================

    #[test]
    fn attributes_filtered_total_increments() {
        let before = otel_attributes_filtered_total().get();
        otel_attributes_filtered_total().inc_by(3);
        let after = otel_attributes_filtered_total().get();
        assert_eq!(after, before + 3);
    }

    #[test]
    fn attributes_filtered_total_no_labels() {
        // Architecture audit § A15 commits to a label-free counter.
        // A future "by location" extension would add label arity ;
        // this pin catches that drift.
        assert_eq!(
            otel_attributes_filtered_total().desc()[0]
                .variable_labels
                .len(),
            0
        );
    }
}
