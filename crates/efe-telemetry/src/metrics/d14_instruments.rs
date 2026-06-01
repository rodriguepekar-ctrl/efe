//! Sprint **D14** — Postgres backend instrumentation.
//!
//! 4 nouveaux instruments dédiés au connection pool et aux requêtes
//! SQL exécutées par [`efe_server::catalog::PostgresCatalogRegistry`]
//! (cf. `docs/bench/d14_postgres_catalog_backend_architecture.md` § 12).
//!
//! Conventions identiques à `d13_instruments.rs` :
//!   * macro `once_metric!` (re-déclarée ici localement) ;
//!   * tous les instruments sont enregistrés sur le `global_registry`
//!     M11 ;
//!   * labels strictement bornés.
//!
//! # Cardinalité
//!
//! Pour 1 pool catalog par réplique (cible v0.1.14) :
//!
//! | Instrument                            | Combinations |
//! |---------------------------------------|-------------:|
//! | `efe_pg_pool_active_connections`        | 1 (single label `pool="catalog"`) |
//! | `efe_pg_pool_idle_connections`          | 1 |
//! | `efe_pg_query_duration_seconds`         | 6 ops × 2 results = 12 |
//! | `efe_pg_tx_failures_total`              | 6 ops × ~10 codes = 60 |
//! | `efe_pg_migration_version`              | 1 (no label) |
//!
//! Total ~75 séries — négligeable vs le budget D13 (50 000 panic).

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts,
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
// Pool gauges
// =====================================================================

once_metric!(
    /// `efe_pg_pool_active_connections{pool}` — number of currently
    /// in-use Postgres connections per pool. Tick-published by the
    /// background task `pool_metrics_ticker` every 10 s.
    pg_pool_active_connections,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_pg_pool_active_connections",
            "Number of currently in-use Postgres connections per pool.",
        ),
        &["pool"],
    )
    .expect("valid gauge opts")
);

once_metric!(
    /// `efe_pg_pool_idle_connections{pool}` — number of currently
    /// idle Postgres connections per pool.
    pg_pool_idle_connections,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_pg_pool_idle_connections",
            "Number of currently idle Postgres connections per pool.",
        ),
        &["pool"],
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Query latency histogram
// =====================================================================
//
// Buckets calibrated against the D14 SLOs :
//   - register p99 ≤ 20 ms
//   - evolve p99 ≤ 40 ms (8-concurrent)
//   - get p50 ≤ 3 ms
//
// Buckets in seconds : 0.001 .. 4.096 s (12 buckets, exponential
// base 2). Captures sub-millisecond resolution for `get` while still
// bracketing the worst-case `evolve` under contention.

once_metric!(
    /// `efe_pg_query_duration_seconds{op, result}` — wall-clock
    /// duration of individual SQL operations in
    /// [`efe_server::catalog::PostgresCatalogRegistry`].
    ///
    /// `op` ∈ { register, evolve, get, list_catalogs, list_versions,
    /// delete } ; `result` ∈ { ok, error }.
    pg_query_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_pg_query_duration_seconds",
            "Duration of individual SQL operations to the catalog database.",
        )
        .buckets(exponential_buckets(0.001, 2.0, 12).expect("valid buckets")),
        &["op", "result"],
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Transaction failure counters
// =====================================================================

once_metric!(
    /// `efe_pg_tx_failures_total{op, code}` — count of distinct
    /// Postgres error codes encountered, grouped by trait operation.
    /// Useful for detecting deadlock storms, serialisation failures,
    /// or unique-violation races.
    pg_tx_failures_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_pg_tx_failures_total",
            "Postgres transaction failures by error code.",
        ),
        &["op", "code"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Migration version
// =====================================================================

once_metric!(
    /// `efe_pg_migration_version` — last applied sqlx migration
    /// version, or `-1` if none applied yet (fresh DB or pre-migration
    /// boot). Set by `runtime::Server::build` after `run_migrations`.
    pg_migration_version,
    IntGauge,
    IntGauge::new(
        "efe_pg_migration_version",
        "Last applied sqlx migration version (or -1 if none yet).",
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Eager registration
// =====================================================================

/// Force-register every D14 instrument with the global registry so
/// the Prometheus `/metrics` endpoint exposes them as zero-valued
/// series even before the first SQL operation. Without this, panels
/// in the D13 dashboards would show "no data" until the first
/// register call hits ; with it, dashboards render immediately and
/// alerts have known label sets to match.
///
/// Idempotent — the [`once_metric!`] macros guard against
/// double-registration. Called from
/// `efe_telemetry::observability_init_d13::bootstrap_observability`.
pub fn register_all_d14() {
    let _ = pg_pool_active_connections();
    let _ = pg_pool_idle_connections();
    let _ = pg_query_duration_seconds();
    let _ = pg_tx_failures_total();
    let _ = pg_migration_version();
    // Initialise the migration gauge to -1 so the dashboard can
    // distinguish "not yet booted" from "booted with v0".
    pg_migration_version().set(-1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_active_gauge_is_settable() {
        pg_pool_active_connections()
            .with_label_values(&["catalog"])
            .set(7);
        assert_eq!(
            pg_pool_active_connections()
                .with_label_values(&["catalog"])
                .get(),
            7
        );
    }

    #[test]
    fn pool_idle_gauge_is_settable() {
        pg_pool_idle_connections()
            .with_label_values(&["catalog"])
            .set(3);
        assert_eq!(
            pg_pool_idle_connections()
                .with_label_values(&["catalog"])
                .get(),
            3
        );
    }

    #[test]
    fn query_duration_histogram_observes() {
        pg_query_duration_seconds()
            .with_label_values(&["register", "ok"])
            .observe(0.012);
        let metric = pg_query_duration_seconds()
            .with_label_values(&["register", "ok"]);
        assert!(metric.get_sample_count() >= 1);
    }

    #[test]
    fn tx_failures_counter_increments() {
        let labels = ["evolve", "23505"];
        let before = pg_tx_failures_total().with_label_values(&labels).get();
        pg_tx_failures_total().with_label_values(&labels).inc();
        let after = pg_tx_failures_total().with_label_values(&labels).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn migration_gauge_set() {
        pg_migration_version().set(42);
        assert_eq!(pg_migration_version().get(), 42);
    }

    #[test]
    fn register_all_d14_idempotent() {
        register_all_d14();
        register_all_d14();
        // No assertions ; absence of panic is the signal.
    }
}
