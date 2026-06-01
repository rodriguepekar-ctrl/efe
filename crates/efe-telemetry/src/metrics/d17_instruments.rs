//! Sprint **D17** — Backup + DR Prometheus instruments.
//!
//! Cf. `docs/bench/d17_backup_dr_architecture.md` § 19.
//!
//! 8 instruments couvrant la pipeline backup et la DR drill :
//!
//! | # | Métrique                                  | Type      | Labels             |
//! |--:|-------------------------------------------|-----------|--------------------|
//! | 1 | `efe_backup_runs_total`                   | Counter   | source, type, result |
//! | 2 | `efe_backup_duration_seconds`             | Histogram | source, type        |
//! | 3 | `efe_backup_size_bytes`                   | Histogram | source, type        |
//! | 4 | `efe_backup_last_success_timestamp`       | Gauge     | source, type        |
//! | 5 | `efe_backup_rpo_seconds`                  | Gauge     | (no label)          |
//! | 6 | `efe_dr_drill_runs_total`                 | Counter   | result              |
//! | 7 | `efe_dr_drill_duration_seconds`           | Histogram | (no label)          |
//! | 8 | `efe_dr_drill_last_success_timestamp`     | Gauge     | (no label)          |
//!
//! Cardinalité totale ~67 séries (architecture § 19.1) — sous le
//! budget D13 50 000.

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, Histogram, HistogramOpts, HistogramVec, IntCounter,
    IntCounterVec, IntGauge, IntGaugeVec, Opts,
};

use super::registry::global_registry;

/// Internal once-per-call macro identical to the one in d13bis_instruments.rs.
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

// ====================================================================
// 1. Counter — backup attempts by source × type × result
// ====================================================================

once_metric!(
    /// `efe_backup_runs_total{source,type,result}` — count of backup
    /// runs per data source, kind, and outcome.
    ///
    /// `source` ∈ `postgres` / `vault` / `bundles`.
    /// `type`   ∈ `full` / `diff` / `incr` / `wal` / `vault_snapshot` / `bundle_sync`.
    /// `result` ∈ `ok` / `failed` / `partial`.
    ///
    /// Alert `D17BackupFailureRate` fires on
    /// `rate(efe_backup_runs_total{result="failed"}) / rate(efe_backup_runs_total) > 0.1`.
    backup_runs_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_backup_runs_total",
            "Number of backup runs per source × kind × result (D17 § 19.1).",
        ),
        &["source", "type", "result"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 2. Histogram — backup duration
// ====================================================================

once_metric!(
    /// `efe_backup_duration_seconds{source,type}` — wall-clock
    /// duration of one backup run.
    ///
    /// Buckets : 60 s, 5 min, 15 min, 30 min, 1 h, 2 h, 4 h
    /// (covering the architecture § 6.2 commitment of ≤ 30 min for
    /// a 100 GB base backup).
    backup_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_backup_duration_seconds",
            "Wall-clock duration of one backup run (D17 § 19.2).",
        )
        .buckets(vec![60.0, 300.0, 900.0, 1800.0, 3600.0, 7200.0, 14400.0]),
        &["source", "type"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 3. Histogram — backup size
// ====================================================================

once_metric!(
    /// `efe_backup_size_bytes{source,type}` — total bytes pushed
    /// to the backup repo by one run.
    ///
    /// Buckets exponential 1 MB → 100 GB.
    backup_size_bytes,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_backup_size_bytes",
            "Bytes pushed to the backup repo per run (D17 § 19.2).",
        )
        .buckets(exponential_buckets(1.0e6, 10.0, 6).expect("valid buckets")),
        &["source", "type"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 4. Gauge — last successful backup timestamp
// ====================================================================

once_metric!(
    /// `efe_backup_last_success_timestamp{source,type}` — Unix
    /// seconds of the last successful backup completion.
    ///
    /// `time() - efe_backup_last_success_timestamp` gives the age
    /// of the most recent backup ; the alert
    /// `D17BackupStale` fires if `age > 24 h` for a base backup.
    backup_last_success_timestamp,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_backup_last_success_timestamp",
            "Unix timestamp (seconds) of the last successful backup (D17 § 19.2).",
        ),
        &["source", "type"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 5. Gauge — RPO (recovery point objective) seconds
// ====================================================================

once_metric!(
    /// `efe_backup_rpo_seconds` — current Recovery Point Objective
    /// in seconds. Computed as `time() - last WAL archive ts`.
    ///
    /// Alert `D17BackupRpoViolation` fires if `> 300` (5 min) for
    /// 5 minutes.
    backup_rpo_seconds,
    IntGauge,
    IntGauge::with_opts(Opts::new(
        "efe_backup_rpo_seconds",
        "Current RPO in seconds (D17 § 15.3).",
    ))
    .expect("valid gauge opts")
);

// ====================================================================
// 6. Counter — DR drill runs
// ====================================================================

once_metric!(
    /// `efe_dr_drill_runs_total{result}` — count of DR drill runs.
    ///
    /// `result` ∈ `success` / `failure`.
    ///
    /// Alert `D17DrDrillStale` fires if `time() - efe_dr_drill_last_success_timestamp > 14 d`.
    dr_drill_runs_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_dr_drill_runs_total",
            "Count of DR drill runs by result (D17 § 16.3).",
        ),
        &["result"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 7. Histogram — DR drill duration
// ====================================================================

once_metric!(
    /// `efe_dr_drill_duration_seconds` — full RTO measurement
    /// (drill kick → smoke test pass).
    ///
    /// Buckets cover 1 min → 50 min so the SLO target (30 min)
    /// sits inside a bucket.
    dr_drill_duration_seconds,
    Histogram,
    Histogram::with_opts(
        HistogramOpts::new(
            "efe_dr_drill_duration_seconds",
            "End-to-end DR drill duration (D17 § 16.3 RTO measurement).",
        )
        .buckets(vec![60.0, 300.0, 600.0, 900.0, 1200.0, 1500.0, 1800.0, 2400.0, 3000.0]),
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 8. Gauge — last successful drill timestamp
// ====================================================================

once_metric!(
    /// `efe_dr_drill_last_success_timestamp` — Unix seconds of the
    /// last successful drill end.
    dr_drill_last_success_timestamp,
    IntGauge,
    IntGauge::with_opts(Opts::new(
        "efe_dr_drill_last_success_timestamp",
        "Unix timestamp of the last successful DR drill (D17 § 16.3).",
    ))
    .expect("valid gauge opts")
);

// ====================================================================
// Eager registration helper
// ====================================================================

/// Force-register every D17 instrument with the global registry so
/// the Prometheus `/metrics` endpoint exposes them as zero-valued
/// series even before the first backup run. Pattern identical to
/// `register_all_d13bis()` (D13-bis audit § A21).
///
/// Idempotent — `OnceLock` guards re-entries.
pub fn register_all_d17() {
    let _ = backup_runs_total();
    let _ = backup_duration_seconds();
    let _ = backup_size_bytes();
    let _ = backup_last_success_timestamp();
    let _ = backup_rpo_seconds();
    let _ = dr_drill_runs_total();
    let _ = dr_drill_duration_seconds();
    let _ = dr_drill_last_success_timestamp();
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    #[test]
    fn register_all_d17_idempotent() {
        register_all_d17();
        register_all_d17();
    }

    #[test]
    fn backup_runs_total_increments() {
        let labels = ["postgres", "full", "ok"];
        let before = backup_runs_total().with_label_values(&labels).get();
        backup_runs_total().with_label_values(&labels).inc();
        let after = backup_runs_total().with_label_values(&labels).get();
        // Shared global state ; allow concurrent observations.
        assert!(after >= before + 1);
    }

    #[test]
    fn backup_duration_observes() {
        backup_duration_seconds()
            .with_label_values(&["postgres", "full"])
            .observe(120.0);
        let m = backup_duration_seconds().with_label_values(&["postgres", "full"]);
        assert!(m.get_sample_count() >= 1);
    }

    #[test]
    fn backup_size_observes() {
        backup_size_bytes()
            .with_label_values(&["postgres", "full"])
            .observe(5.0e9);
        let m = backup_size_bytes().with_label_values(&["postgres", "full"]);
        assert!(m.get_sample_count() >= 1);
    }

    #[test]
    fn backup_last_success_timestamp_set_get() {
        backup_last_success_timestamp()
            .with_label_values(&["postgres", "full"])
            .set(1_700_000_000);
        let v = backup_last_success_timestamp()
            .with_label_values(&["postgres", "full"])
            .get();
        assert!(v >= 1_700_000_000);
    }

    #[test]
    fn rpo_gauge_no_labels() {
        backup_rpo_seconds().set(120);
        assert_eq!(backup_rpo_seconds().desc()[0].variable_labels.len(), 0);
    }

    #[test]
    fn dr_drill_runs_label_arity() {
        assert_eq!(
            dr_drill_runs_total().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn label_arity_pinned() {
        // Architecture § 19.1 pins these arities.
        assert_eq!(
            backup_runs_total().desc()[0].variable_labels.len(),
            3
        );
        assert_eq!(
            backup_duration_seconds().desc()[0].variable_labels.len(),
            2
        );
        assert_eq!(
            backup_size_bytes().desc()[0].variable_labels.len(),
            2
        );
        assert_eq!(
            backup_last_success_timestamp().desc()[0]
                .variable_labels
                .len(),
            2
        );
        assert_eq!(backup_rpo_seconds().desc()[0].variable_labels.len(), 0);
        assert_eq!(dr_drill_runs_total().desc()[0].variable_labels.len(), 1);
        assert_eq!(
            dr_drill_duration_seconds().desc()[0].variable_labels.len(),
            0
        );
        assert_eq!(
            dr_drill_last_success_timestamp().desc()[0]
                .variable_labels
                .len(),
            0
        );
    }

    #[test]
    fn result_labels_pinned() {
        // Architecture § 19.1 commits to specific result strings.
        for r in ["ok", "failed", "partial"] {
            backup_runs_total()
                .with_label_values(&["postgres", "full", r])
                .inc();
        }
        for r in ["success", "failure"] {
            dr_drill_runs_total().with_label_values(&[r]).inc();
        }
    }
}
