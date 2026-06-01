//! Prometheus metrics for the PII crate.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter, register_int_counter_vec,
    HistogramVec, IntCounter, IntCounterVec,
};

/// Detected entities by type. `efe_pii_detections_total{type}`.
pub static DETECTIONS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_pii_detections_total",
        "PII spans detected.",
        &["entity_type"],
    )
    .expect("register efe_pii_detections_total")
});

/// `efe_pii_rejections_total{type}` — strict-policy rejections.
pub static REJECTIONS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_pii_rejections_total",
        "Requests rejected by strict PII policy.",
        &["entity_type"],
    )
    .expect("register efe_pii_rejections_total")
});

/// `efe_pii_analyze_failures_total{reason}` — backend failures.
pub static ANALYZE_FAILURES_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_pii_analyze_failures_total",
        "PII analyzer backend failures.",
        &["reason"],
    )
    .expect("register efe_pii_analyze_failures_total")
});

/// `efe_pii_analyze_circuit_breaks_total` — count of short-circuited calls.
pub static ANALYZE_CIRCUIT_BREAKS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    register_int_counter!(
        "efe_pii_analyze_circuit_breaks_total",
        "Number of analyze calls short-circuited by the breaker.",
    )
    .expect("register efe_pii_analyze_circuit_breaks_total")
});

/// `efe_pii_analyze_duration_seconds` — latency histogram.
pub static ANALYZE_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_pii_analyze_duration_seconds",
        "Latency of PII analyzer calls.",
        &["outcome"],
        vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0],
    )
    .expect("register efe_pii_analyze_duration_seconds")
});

/// Touch every metric at boot so `/metrics` exposes them immediately.
pub fn init() {
    let _ = &*DETECTIONS_TOTAL;
    let _ = &*REJECTIONS_TOTAL;
    let _ = &*ANALYZE_FAILURES_TOTAL;
    let _ = &*ANALYZE_CIRCUIT_BREAKS_TOTAL;
    let _ = &*ANALYZE_DURATION;
}
