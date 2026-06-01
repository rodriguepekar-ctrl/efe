//! Prometheus metrics for the cache crate.
//!
//! All counters / histograms registered in the **default** Prometheus
//! registry — same one efe-server scrapes.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec,
    HistogramVec, IntCounterVec, IntGaugeVec,
};

/// `efe_cache_hit_total{cache, backend}` — every successful GET that
/// returned `Some`.
pub static CACHE_HIT_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_cache_hit_total",
        "Cache hits (GET returned Some).",
        &["cache", "backend"],
    )
    .expect("register efe_cache_hit_total")
});

/// `efe_cache_miss_total{cache, backend}` — GET returned `None`.
pub static CACHE_MISS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_cache_miss_total",
        "Cache misses (GET returned None).",
        &["cache", "backend"],
    )
    .expect("register efe_cache_miss_total")
});

/// `efe_cache_set_total{cache, backend, outcome}` — every SET attempt.
pub static CACHE_SET_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_cache_set_total",
        "Cache SET attempts.",
        &["cache", "backend", "outcome"],
    )
    .expect("register efe_cache_set_total")
});

/// `efe_cache_poisoned_total{cache}` — HMAC verification failures.
/// Critical : non-zero requires P1 incident.
pub static CACHE_POISONED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_cache_poisoned_total",
        "Cache entries that failed HMAC verification (poisoning indicator).",
        &["cache"],
    )
    .expect("register efe_cache_poisoned_total")
});

/// `efe_cache_op_duration_seconds{op, backend}` — latency histogram for
/// each cache operation.
pub static CACHE_OP_DURATION: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_cache_op_duration_seconds",
        "Cache op latency.",
        &["op", "backend"],
        vec![
            0.0005, 0.001, 0.002, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0
        ],
    )
    .expect("register efe_cache_op_duration_seconds")
});

/// `efe_quota_current{tenant, endpoint, unit}` — current counter value.
pub static QUOTA_CURRENT: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_quota_current",
        "Current quota counter (post-increment).",
        &["tenant", "endpoint", "unit"],
    )
    .expect("register efe_quota_current")
});

/// `efe_quota_exceeded_total{tenant, endpoint}` — counter of requests
/// rejected because of quota.
pub static QUOTA_EXCEEDED_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_quota_exceeded_total",
        "Requests rejected because tenant quota was exceeded.",
        &["tenant", "endpoint"],
    )
    .expect("register efe_quota_exceeded_total")
});

/// Force lazy initialization of all metrics at process boot. Call once.
pub fn init() {
    // Touch every metric so they appear in `/metrics` even before first
    // op. This is the "register-at-boot" pattern.
    let _ = &*CACHE_HIT_TOTAL;
    let _ = &*CACHE_MISS_TOTAL;
    let _ = &*CACHE_SET_TOTAL;
    let _ = &*CACHE_POISONED_TOTAL;
    let _ = &*CACHE_OP_DURATION;
    let _ = &*QUOTA_CURRENT;
    let _ = &*QUOTA_EXCEEDED_TOTAL;
}
