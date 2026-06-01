// SPDX-License-Identifier: UNLICENSED
//! M30 — Multimodal Prometheus metrics.
//!
//! All histograms are global and lazy-initialized via `once_cell`. The
//! cardinality budget per the M30 ADR-M30-005 is :
//!
//! - modality ∈ {`vision`, `audio`, `long_text`, `fusion`} (4 values)
//! - outcome ∈ {`ok`, `err`} (2 values)
//! - region ∈ {`eu-west-1`, `eu-west-3`} (2 values, M30 phase 3)
//!
//! Total : ≤ 16 series per histogram × ~ 15 histograms = 240 series.
//! Comfortably within the Prometheus single-shard scrape budget
//! (recommended ≤ 1 M series per shard).

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
    HistogramVec, IntCounterVec, IntGaugeVec,
};

/// Histogram buckets in milliseconds for end-to-end multimodal inference
/// latency. Tuned for the SigLIP/Whisper p99 budget of ~ 2 s.
pub const E2E_LATENCY_BUCKETS_MS: &[f64] = &[
    5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1_500.0, 3_000.0, 6_000.0, 12_000.0,
];

/// Histogram buckets in milliseconds for Triton-side inference latency
/// (sub-budget of E2E ; should sit ~ 100-1200 ms).
pub const TRITON_LATENCY_BUCKETS_MS: &[f64] = &[
    2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1_500.0, 3_000.0,
];

/// Histogram buckets for input size in bytes. Vision images sit
/// 50 kB – 5 MB ; audio sits 100 kB – 25 MB.
pub const INPUT_SIZE_BUCKETS_BYTES: &[f64] = &[
    1_000.0, 10_000.0, 50_000.0, 200_000.0, 1_000_000.0, 5_000_000.0, 15_000_000.0, 50_000_000.0,
];

/// End-to-end multimodal request duration (preprocessing + Triton +
/// postprocessing), labelled by modality and outcome.
pub static MULTIMODAL_E2E_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_multimodal_e2e_duration_ms",
        "End-to-end multimodal inference latency in milliseconds.",
        &["modality", "outcome"],
        E2E_LATENCY_BUCKETS_MS.to_vec()
    )
    .expect("register efe_multimodal_e2e_duration_ms")
});

/// Triton-side inference duration. The diff
/// (`e2e - triton`) is the preprocessing + network overhead.
pub static MULTIMODAL_TRITON_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_multimodal_triton_duration_ms",
        "Triton inference latency in milliseconds.",
        &["modality", "model_name", "outcome"],
        TRITON_LATENCY_BUCKETS_MS.to_vec()
    )
    .expect("register efe_multimodal_triton_duration_ms")
});

/// Input size in bytes. Cardinality bounded by `modality` only.
pub static MULTIMODAL_INPUT_SIZE_BYTES: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_multimodal_input_size_bytes",
        "Multimodal input size in bytes.",
        &["modality"],
        INPUT_SIZE_BUCKETS_BYTES.to_vec()
    )
    .expect("register efe_multimodal_input_size_bytes")
});

/// Number of accepted (post-guard) multimodal requests.
pub static MULTIMODAL_REQUESTS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_multimodal_requests_total",
        "Multimodal requests accepted (post-input-guard).",
        &["modality", "outcome"]
    )
    .expect("register efe_multimodal_requests_total")
});

/// Number of multimodal requests rejected by the input guard, labelled
/// by the rejection reason (`too_large`, `invalid_format`,
/// `invalid_duration`, `modality_disabled`).
pub static MULTIMODAL_REJECTIONS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_multimodal_rejections_total",
        "Multimodal requests rejected before reaching Triton.",
        &["modality", "reason"]
    )
    .expect("register efe_multimodal_rejections_total")
});

/// Number of Triton transport errors (TCP reset, DNS failure, 5xx).
/// Labelled by `kind` ∈ {`transport`, `timeout`, `bad_response`}.
pub static MULTIMODAL_TRITON_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_multimodal_triton_errors_total",
        "Triton transport-layer error count.",
        &["modality", "model_name", "kind"]
    )
    .expect("register efe_multimodal_triton_errors_total")
});

/// Number of in-flight multimodal requests per modality. Used by the
/// backpressure layer to shed load early when Triton is saturated.
pub static MULTIMODAL_INFLIGHT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_multimodal_inflight",
        "In-flight multimodal requests per modality.",
        &["modality"]
    )
    .expect("register efe_multimodal_inflight")
});

/// Per-modality chunk count (long-context only). Records the number of
/// chunks produced by the hierarchical chunker.
pub static MULTIMODAL_CHUNK_COUNT: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_multimodal_chunk_count",
        "Number of chunks produced per long-context request.",
        &["outcome"],
        vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0]
    )
    .expect("register efe_multimodal_chunk_count")
});

/// Fusion strategy distribution counter. Records which fusion strategy
/// was applied per request.
pub static MULTIMODAL_FUSION_STRATEGY: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_multimodal_fusion_strategy_total",
        "Count of fusion calls by strategy.",
        &["strategy"]
    )
    .expect("register efe_multimodal_fusion_strategy_total")
});

/// Convenience inflight-tracking RAII guard. Increments the
/// `MULTIMODAL_INFLIGHT` gauge on construction and decrements on drop,
/// guaranteeing exception safety even if the request panics.
pub struct InflightGuard {
    modality: &'static str,
}

impl InflightGuard {
    /// Create a new inflight guard for the given modality.
    pub fn new(modality: &'static str) -> Self {
        MULTIMODAL_INFLIGHT.with_label_values(&[modality]).inc();
        Self { modality }
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        MULTIMODAL_INFLIGHT.with_label_values(&[self.modality]).dec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflight_guard_increments_and_decrements() {
        let before = MULTIMODAL_INFLIGHT.with_label_values(&["vision"]).get();
        {
            let _g = InflightGuard::new("vision");
            let during = MULTIMODAL_INFLIGHT.with_label_values(&["vision"]).get();
            assert_eq!(during, before + 1);
        }
        let after = MULTIMODAL_INFLIGHT.with_label_values(&["vision"]).get();
        assert_eq!(after, before);
    }

    #[test]
    fn histograms_have_expected_label_cardinality() {
        // Touch every histogram to register them and verify the labels
        // are accepted (would panic at register time if invalid).
        MULTIMODAL_E2E_DURATION_MS
            .with_label_values(&["vision", "ok"])
            .observe(10.0);
        MULTIMODAL_TRITON_DURATION_MS
            .with_label_values(&["audio", "whisper_tiny", "ok"])
            .observe(20.0);
        MULTIMODAL_INPUT_SIZE_BYTES
            .with_label_values(&["long_text"])
            .observe(50_000.0);
        MULTIMODAL_REQUESTS_TOTAL
            .with_label_values(&["fusion", "ok"])
            .inc();
        MULTIMODAL_REJECTIONS_TOTAL
            .with_label_values(&["vision", "too_large"])
            .inc();
        MULTIMODAL_TRITON_ERRORS_TOTAL
            .with_label_values(&["vision", "siglip_base", "timeout"])
            .inc();
        MULTIMODAL_CHUNK_COUNT.with_label_values(&["ok"]).observe(8.0);
        MULTIMODAL_FUSION_STRATEGY
            .with_label_values(&["concat_linear"])
            .inc();
    }
}
