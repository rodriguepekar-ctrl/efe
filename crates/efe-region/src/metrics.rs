// SPDX-License-Identifier: UNLICENSED
//! M30 — Region Prometheus metrics.

use once_cell::sync::Lazy;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec, HistogramVec,
    IntCounterVec, IntGaugeVec,
};

/// Per-peer health gauge : 1 = up, 0 = down. Labelled by `region`.
pub static REGION_PEER_UP: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_region_peer_up",
        "Cross-region peer health: 1 = up, 0 = down.",
        &["region"]
    )
    .expect("register efe_region_peer_up")
});

/// Per-peer probe duration in milliseconds.
pub static REGION_PEER_PROBE_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_region_peer_probe_duration_ms",
        "Cross-region peer probe duration in milliseconds.",
        &["region"],
        vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0, 2_500.0]
    )
    .expect("register efe_region_peer_probe_duration_ms")
});

/// Number of failover events. Labelled by `target_region`.
pub static REGION_FAILOVER_TOTAL: Lazy<IntCounterVec> = Lazy::new(|| {
    register_int_counter_vec!(
        "efe_region_failover_total",
        "Number of failover events (manual + drill).",
        &["target_region"]
    )
    .expect("register efe_region_failover_total")
});

/// Failover duration in milliseconds. Labelled by `target_region` and
/// `outcome` ∈ {`succeeded`, `noop`, `failed`}.
pub static REGION_FAILOVER_DURATION_MS: Lazy<HistogramVec> = Lazy::new(|| {
    register_histogram_vec!(
        "efe_region_failover_duration_ms",
        "Failover orchestration wall-clock duration.",
        &["target_region", "outcome"],
        vec![100.0, 500.0, 1_000.0, 2_500.0, 5_000.0, 10_000.0, 30_000.0, 60_000.0]
    )
    .expect("register efe_region_failover_duration_ms")
});

/// Replication lag in seconds. Set by the PG monitor sidecar
/// (efe-server publishes this from `pg_stat_replication`). Labelled
/// by `slot_name`.
pub static REGION_REPLICATION_LAG_SECS: Lazy<IntGaugeVec> = Lazy::new(|| {
    register_int_gauge_vec!(
        "efe_region_replication_lag_secs",
        "Postgres logical replication lag in seconds (rounded).",
        &["slot_name"]
    )
    .expect("register efe_region_replication_lag_secs")
});

/// Public setter for [`REGION_REPLICATION_LAG_SECS`]. M30 audit fix :
/// previously the gauge was registered but had no writer, leaving the
/// `M30RegionHighReplicationLag` alert as dead code. This setter is
/// called by the `replication_monitor::run_lag_monitor` loop.
///
/// `lag_secs` is rounded to the nearest integer second (negative
/// values clamped to 0).
pub fn set_replication_lag_secs(slot_name: &str, lag_secs: f64) {
    let rounded = if lag_secs.is_nan() || lag_secs < 0.0 {
        0
    } else {
        lag_secs.round() as i64
    };
    REGION_REPLICATION_LAG_SECS
        .with_label_values(&[slot_name])
        .set(rounded);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_all_metrics() {
        REGION_PEER_UP.with_label_values(&["eu-west-3"]).set(1);
        REGION_PEER_PROBE_DURATION_MS
            .with_label_values(&["eu-west-3"])
            .observe(15.0);
        REGION_FAILOVER_TOTAL
            .with_label_values(&["eu-west-3"])
            .inc();
        REGION_FAILOVER_DURATION_MS
            .with_label_values(&["eu-west-3", "succeeded"])
            .observe(120.0);
        REGION_REPLICATION_LAG_SECS
            .with_label_values(&["m30_main"])
            .set(2);
    }

    #[test]
    fn set_replication_lag_clamps_negatives_and_nan() {
        set_replication_lag_secs("slot_unit_test_a", 12.7);
        assert_eq!(
            REGION_REPLICATION_LAG_SECS
                .with_label_values(&["slot_unit_test_a"])
                .get(),
            13 // rounded
        );
        set_replication_lag_secs("slot_unit_test_b", -5.0);
        assert_eq!(
            REGION_REPLICATION_LAG_SECS
                .with_label_values(&["slot_unit_test_b"])
                .get(),
            0
        );
        set_replication_lag_secs("slot_unit_test_c", f64::NAN);
        assert_eq!(
            REGION_REPLICATION_LAG_SECS
                .with_label_values(&["slot_unit_test_c"])
                .get(),
            0
        );
    }
}
