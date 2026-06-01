// SPDX-License-Identifier: UNLICENSED
//! M31 — Prometheus metrics for the metering plane.
//!
//! Cardinality budget per ADR-M31-005 :
//! - `meter` : 8 values (one per [`Meter`] variant).
//! - `outcome` : 2 values (`ok`, `err`).
//! - `kind` (Redis errors) : 3 values (`transport`, `encode`, `capacity`).
//!
//! ## M31 perf
//!
//! The hot-path `meter_inc` call would normally pay a HashMap lookup
//! through `IntCounterVec::with_label_values(&[meter.name()])` on
//! every call. We cache the resolved `IntCounter` per [`Meter`]
//! variant at first use via [`meter_inc_counter`], turning the
//! lookup into an `&[IntCounter; 8]` array access.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram, register_histogram_vec, register_int_counter_vec,
    register_int_gauge_vec, Histogram, HistogramVec, IntCounter, IntCounterVec,
    IntGaugeVec,
};

use crate::meter::{Meter, METER_COUNT};

/// Histogram buckets in microseconds for the hot `meter_inc` path
/// (buffered, no Redis round-trip).
pub const METER_INC_BUCKETS_US: &[f64] = &[
    1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 5_000.0,
];

/// Histogram buckets in milliseconds for the flush path (Redis
/// round-trip).
pub const METER_FLUSH_BUCKETS_MS: &[f64] = &[
    1.0, 2.5, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0,
];

/// Per-meter increment count. Labelled by meter name.
pub static METER_INC_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_meter_inc_total",
        "Per-meter increment count.",
        &["meter"]
    )
    .expect("register efe_meter_inc_total")
});

/// **M31 perf** : per-meter `IntCounter` cache. The hot path uses
/// [`meter_inc_counter`] to avoid the per-call HashMap lookup that
/// `METER_INC_TOTAL.with_label_values(&[...])` performs.
static METER_INC_COUNTERS: Lazy<[IntCounter; METER_COUNT]> = Lazy::new(|| {
    // SAFETY : Meter::all() is guaranteed to return exactly
    // `METER_COUNT` entries by `meter::tests::all_count_matches_constant`.
    let arr: Vec<IntCounter> = Meter::all()
        .iter()
        .map(|m| METER_INC_TOTAL.with_label_values(&[m.name()]))
        .collect();
    arr.try_into().expect("METER_INC_COUNTERS sized correctly")
});

/// Returns the cached `IntCounter` for `meter`. The hot path uses
/// this instead of `METER_INC_TOTAL.with_label_values(...)`.
#[inline]
#[must_use]
pub fn meter_inc_counter(meter: Meter) -> &'static IntCounter {
    let idx = meter_index(meter);
    &METER_INC_COUNTERS[idx]
}

/// O(1) mapping from [`Meter`] to its index in `Meter::all()`.
/// The compiler folds this into a simple `match` jump table.
#[inline]
#[must_use]
fn meter_index(meter: Meter) -> usize {
    match meter {
        Meter::ClassifyRequests => 0,
        Meter::AgentRunSteps => 1,
        Meter::AgentRunSeconds => 2,
        Meter::MultimodalVisionRequests => 3,
        Meter::MultimodalAudioSeconds => 4,
        Meter::WebhookDeliveries => 5,
        Meter::BatchClassifiedRows => 6,
        Meter::AuditExportGb => 7,
        // Sprint 5 (M50) additions :
        Meter::NanoAgentCalls => 8,
        Meter::NanoAgentLlmTokens => 9,
        Meter::NanoAgentVmSeconds => 10,
    }
}

/// Hot-path `meter_inc` latency (microseconds).
pub static METER_INC_DURATION_US: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_meter_inc_duration_us",
        "Latency of meter_inc calls in microseconds.",
        &["outcome"],
        METER_INC_BUCKETS_US.to_vec()
    )
    .expect("register efe_meter_inc_duration_us")
});

/// Buffered batch size at flush time.
pub static METER_FLUSH_BATCH_SIZE: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "efe_meter_flush_batch_size",
        "Batched events flushed in one go.",
        vec![1.0, 8.0, 32.0, 128.0, 512.0, 1_024.0, 4_096.0]
    )
    .expect("register efe_meter_flush_batch_size")
});

/// Flush wall-clock latency (milliseconds).
pub static METER_FLUSH_DURATION_MS: Lazy<Histogram> = Lazy::new(|| {
    register_histogram!(
        "efe_meter_flush_duration_ms",
        "Flush wall-clock latency in milliseconds.",
        METER_FLUSH_BUCKETS_MS.to_vec()
    )
    .expect("register efe_meter_flush_duration_ms")
});

/// Redis error counter. Labelled by `kind` ∈ {transport, encode,
/// capacity}.
pub static METER_REDIS_ERRORS_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_meter_redis_errors_total",
        "Redis transport / encode / capacity errors.",
        &["kind"]
    )
    .expect("register efe_meter_redis_errors_total")
});

/// Rollup lag in seconds (set by the rollup runner). Labelled by
/// period ∈ {hour, day}.
pub static ROLLUP_LAG_SECONDS: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_rollup_lag_seconds",
        "Seconds since last successful rollup.",
        &["period"]
    )
    .expect("register efe_rollup_lag_seconds")
});

/// Hot-path cached `IntCounter` accessor for `efe_meter_inc_duration_us`
/// keyed by `outcome` — same speedup story as
/// [`meter_inc_counter`].
static METER_INC_DURATION_OK: Lazy<Histogram> = Lazy::new(|| {
    METER_INC_DURATION_US.with_label_values(&["ok"])
});

static METER_INC_DURATION_ERR: Lazy<Histogram> = Lazy::new(|| {
    METER_INC_DURATION_US.with_label_values(&["err"])
});

/// Cached histogram for the "ok" outcome on the hot path.
#[inline]
#[must_use]
pub fn meter_inc_duration_ok() -> &'static Histogram {
    &METER_INC_DURATION_OK
}

/// Cached histogram for the "err" outcome on the hot path.
#[inline]
#[must_use]
pub fn meter_inc_duration_err() -> &'static Histogram {
    &METER_INC_DURATION_ERR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_register_cleanly() {
        METER_INC_TOTAL
            .with_label_values(&["classify_requests"])
            .inc();
        METER_INC_DURATION_US
            .with_label_values(&["ok"])
            .observe(20.0);
        METER_FLUSH_BATCH_SIZE.observe(128.0);
        METER_FLUSH_DURATION_MS.observe(15.0);
        METER_REDIS_ERRORS_TOTAL
            .with_label_values(&["transport"])
            .inc();
        ROLLUP_LAG_SECONDS.with_label_values(&["hour"]).set(45);
    }

    #[test]
    fn meter_index_is_total_function() {
        // For every Meter, the index is < METER_COUNT and round-trip
        // via Meter::all() is the identity.
        for m in Meter::all() {
            let idx = meter_index(*m);
            assert!(idx < METER_COUNT);
            assert_eq!(Meter::all()[idx], *m);
        }
    }

    #[test]
    fn meter_inc_counter_returns_distinct_pointers() {
        // Different meters → different counters.
        let c1 = meter_inc_counter(Meter::ClassifyRequests);
        let c2 = meter_inc_counter(Meter::AgentRunSteps);
        // Pointer inequality between counters (same meter would give same pointer).
        let p1: *const IntCounter = c1;
        let p2: *const IntCounter = c2;
        assert_ne!(p1, p2);
    }

    #[test]
    fn meter_inc_counter_returns_stable_pointer() {
        let p1: *const IntCounter = meter_inc_counter(Meter::ClassifyRequests);
        let p2: *const IntCounter = meter_inc_counter(Meter::ClassifyRequests);
        assert_eq!(p1, p2, "cache must return identity");
    }

    #[test]
    fn meter_inc_counter_increments_observable_in_global() {
        let before = METER_INC_TOTAL
            .with_label_values(&["webhook_deliveries"])
            .get();
        meter_inc_counter(Meter::WebhookDeliveries).inc();
        let after = METER_INC_TOTAL
            .with_label_values(&["webhook_deliveries"])
            .get();
        assert_eq!(after, before + 1);
    }
}
