// SPDX-License-Identifier: UNLICENSED
//! M33 — KG plane Prometheus metrics.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter_vec,
    register_int_gauge_vec, Histogram, HistogramVec, IntCounterVec, IntGaugeVec,
};

/// Histogram buckets in milliseconds for traversal latency.
pub const TRAVERSAL_BUCKETS_MS: &[f64] = &[
    0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 1_000.0,
];

/// Per-kind entity insert count.
pub static KG_ENTITY_INSERT_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_kg_entity_insert_total",
        "Entities inserted per kind.",
        &["kind"]
    )
    .expect("register efe_kg_entity_insert_total")
});

/// Per-predicate relation insert count.
pub static KG_RELATION_INSERT_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_kg_relation_insert_total",
        "Relations inserted per predicate.",
        &["predicate"]
    )
    .expect("register efe_kg_relation_insert_total")
});

/// Traversal duration per algorithm.
pub static KG_TRAVERSAL_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_kg_traversal_duration_ms",
        "Graph traversal latency in milliseconds.",
        &["algo"],
        TRAVERSAL_BUCKETS_MS.to_vec()
    )
    .expect("register efe_kg_traversal_duration_ms")
});

/// NER extractor duration per outcome.
pub static KG_EXTRACTOR_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_kg_extractor_duration_ms",
        "NER extractor latency in milliseconds.",
        &["outcome"],
        vec![5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 5_000.0]
    )
    .expect("register efe_kg_extractor_duration_ms")
});

/// Per-tenant graph cardinality.
pub static KG_GRAPH_SIZE: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_kg_graph_size",
        "Per-tenant graph size : entities + relations.",
        &["side"]
    )
    .expect("register efe_kg_graph_size")
});

/// Resolution merge count.
pub static KG_RESOLUTION_MERGES_TOTAL: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "efe_kg_resolution_merges_total",
        "Number of entity-resolution merges per batch.",
        vec![1.0, 2.0, 4.0, 8.0, 16.0, 64.0, 256.0]
    )
    .expect("register efe_kg_resolution_merges_total")
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_all_metrics() {
        KG_ENTITY_INSERT_TOTAL
            .with_label_values(&["person"])
            .inc();
        KG_RELATION_INSERT_TOTAL
            .with_label_values(&["works_at"])
            .inc();
        KG_TRAVERSAL_DURATION_MS
            .with_label_values(&["bfs"])
            .observe(2.5);
        KG_EXTRACTOR_DURATION_MS
            .with_label_values(&["ok"])
            .observe(45.0);
        KG_GRAPH_SIZE.with_label_values(&["entities"]).set(1234);
        KG_RESOLUTION_MERGES_TOTAL.observe(5.0);
    }
}
