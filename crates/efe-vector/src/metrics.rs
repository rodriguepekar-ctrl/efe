// SPDX-License-Identifier: UNLICENSED
//! M32 — Vector plane Prometheus metrics.
//!
//! Cardinality budget per ADR-M32-005 :
//! - `distance` ∈ {cosine, dot, l2} — 3 values.
//! - `mode` (search) ∈ {flat, hnsw, hybrid_rrf, hybrid_weighted} — 4.
//! - `outcome` ∈ {ok, err} — 2.
//! - `model` (recall gauge) ∈ {efe-classifier-768, siglip-base-768, ...} — ≤ 8.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter_vec,
    register_int_gauge_vec, Histogram, HistogramVec, IntCounterVec, IntGaugeVec,
};

/// Histogram buckets in milliseconds for search latency.
pub const SEARCH_BUCKETS_MS: &[f64] = &[
    0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 1_000.0,
];

/// Insert count per distance metric.
pub static VECTOR_INSERT_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_vector_insert_total",
        "Vector inserts (post-validation).",
        &["distance"]
    )
    .expect("register efe_vector_insert_total")
});

/// Search count by mode + distance.
pub static VECTOR_SEARCH_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_vector_search_total",
        "Vector searches issued.",
        &["mode", "distance"]
    )
    .expect("register efe_vector_search_total")
});

/// Search duration by mode.
pub static VECTOR_SEARCH_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_vector_search_duration_ms",
        "Vector search latency in milliseconds.",
        &["mode"],
        SEARCH_BUCKETS_MS.to_vec()
    )
    .expect("register efe_vector_search_duration_ms")
});

/// Collection cardinality gauge.
pub static VECTOR_COLLECTION_SIZE: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_vector_collection_size",
        "Document cardinality per collection.",
        &["collection"]
    )
    .expect("register efe_vector_collection_size")
});

/// HNSW build duration.
pub static VECTOR_HNSW_BUILD_DURATION_MS: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "efe_vector_hnsw_build_duration_ms",
        "Full HNSW build wall-clock latency (snapshot replay).",
        vec![10.0, 100.0, 1_000.0, 10_000.0, 60_000.0]
    )
    .expect("register efe_vector_hnsw_build_duration_ms")
});

/// Snapshot duration by outcome.
pub static VECTOR_SNAPSHOT_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_vector_snapshot_duration_ms",
        "Snapshot wall-clock latency (serialize + S3 PUT).",
        &["outcome"],
        vec![10.0, 100.0, 1_000.0, 10_000.0, 60_000.0]
    )
    .expect("register efe_vector_snapshot_duration_ms")
});

/// Recall @ 10 vs flat baseline. Set by the bench harness.
pub static VECTOR_RECALL_AT_10: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_vector_recall_at_10",
        "Recall @ 10 vs flat baseline (×1000 for integer gauge).",
        &["model"]
    )
    .expect("register efe_vector_recall_at_10")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_all_metrics() {
        VECTOR_INSERT_TOTAL.with_label_values(&["cosine"]).inc();
        VECTOR_SEARCH_TOTAL
            .with_label_values(&["flat", "cosine"])
            .inc();
        VECTOR_SEARCH_DURATION_MS
            .with_label_values(&["hnsw"])
            .observe(5.0);
        VECTOR_COLLECTION_SIZE
            .with_label_values(&["tickets"])
            .set(42);
        VECTOR_HNSW_BUILD_DURATION_MS.observe(120.0);
        VECTOR_SNAPSHOT_DURATION_MS
            .with_label_values(&["ok"])
            .observe(2_500.0);
        VECTOR_RECALL_AT_10
            .with_label_values(&["siglip-base-768"])
            .set(951);
    }
}
