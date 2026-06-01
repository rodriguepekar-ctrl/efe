//! Sprint **D18** — Multi-replica + bench D12 Prometheus instruments.
//!
//! Cf. `docs/bench/d18_multi_replica_bench_d12_architecture.md` § 17.
//!
//! 9 instruments couvrant le runtime multi-réplica et la suite bench :
//!
//! | # | Métrique                                  | Type      | Labels             |
//! |--:|-------------------------------------------|-----------|--------------------|
//! | 1 | `efe_replica_id`                          | Gauge     | replica_id         |
//! | 2 | `efe_idempotency_hits_total`              | Counter   | outcome            |
//! | 3 | `efe_idempotency_size`                    | Gauge     | (no label)         |
//! | 4 | `efe_ratelimit_decisions_total`           | Counter   | tenant_id, outcome |
//! | 5 | `efe_ratelimit_bucket_available`          | Gauge     | tenant_id          |
//! | 6 | `efe_leader_is_leader`                    | Gauge     | (no label)         |
//! | 7 | `efe_leader_acquire_attempts_total`       | Counter   | outcome            |
//! | 8 | `efe_shutdown_phase_seconds`              | Histogram | phase              |
//! | 9 | `efe_bench_runs_total`                    | Counter   | workload, result   |
//!
//! Cardinalité projetée (architecture § 17.1) : ≤ 5 séries pour
//! les compteurs sans tenant + ≤ 200 séries pour les
//! `ratelimit_*` × 100 tenants × 2 outcomes = 200. Toujours sous le
//! budget D13 50 000 séries.
//!
//! **Audit fix D18-A3** — pre-audit code shipped 9 metrics in the
//! architecture but `d18_instruments.rs` was never written. Operators
//! got "no data" panels in the D18 dashboard. This module restores
//! the contract ; the multi_replica module impls (`idempotency_pg`,
//! `ratelimit_pg`, `leader_election`, `shutdown_sequence`) bump
//! these counters at the relevant transition points.

use std::sync::OnceLock;

use prometheus::{
    Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec,
    IntGauge, IntGaugeVec, Opts,
};

use super::registry::global_registry;

/// Internal once-per-call macro identical to the one in d17_instruments.rs.
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
// 1. Gauge — replica identity
// ====================================================================

once_metric!(
    /// `efe_replica_id{replica_id}` — set to 1 by each pod with its
    /// own `HOSTNAME` (or env `EFE_REPLICA_ID` override) as label.
    /// Used by Grafana to partition panels per replica.
    ///
    /// Cardinality cap = `replicas.max` (default 10).
    replica_id,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_replica_id",
            "Static gauge tagging the replica's HOSTNAME (D18 § 17.1 #1).",
        ),
        &["replica_id"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 2. Counter — idempotency cache decisions
// ====================================================================

once_metric!(
    /// `efe_idempotency_hits_total{outcome}` — count of idempotency
    /// resolutions per outcome.
    ///
    /// `outcome` ∈ `leader` / `in_flight` / `cached` / `error`.
    ///
    /// Recording rule `idempotency:hit_ratio:5m` divides
    /// `cached + in_flight` by total to derive the cross-replica
    /// dedup rate (architecture § 17.2).
    idempotency_hits_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_idempotency_hits_total",
            "Idempotency lookup outcomes (D18 § 17.1 #2).",
        ),
        &["outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 3. Gauge — idempotency cache size
// ====================================================================

once_metric!(
    /// `efe_idempotency_size` — current row count in
    /// `efe_idempotency`. Updated by the leader-elected
    /// `expire_sweep` task.
    ///
    /// Alert `D18IdempotencyTableGrowth` fires if this gauge
    /// exceeds 1M for 30 min.
    idempotency_size,
    IntGauge,
    IntGauge::with_opts(Opts::new(
        "efe_idempotency_size",
        "Row count in efe_idempotency (D18 § 17.1 #3).",
    ))
    .expect("valid gauge opts")
);

// ====================================================================
// 4. Counter — rate-limit decisions per tenant
// ====================================================================

once_metric!(
    /// `efe_ratelimit_decisions_total{tenant_id, outcome}` — count
    /// of rate-limit check_and_decrement calls per tenant per
    /// outcome.
    ///
    /// `outcome` ∈ `allowed` / `throttled` / `unconfigured` / `error`.
    ///
    /// Recording rule `ratelimit:throttle_rate:5m` slices
    /// `throttled / total` per tenant for the D18 dashboard.
    ratelimit_decisions_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_ratelimit_decisions_total",
            "Rate-limit decisions per tenant × outcome (D18 § 17.1 #4).",
        ),
        &["tenant_id", "outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 5. Gauge — bucket available tokens per tenant
// ====================================================================

once_metric!(
    /// `efe_ratelimit_bucket_available{tenant_id}` — current
    /// available token count per tenant. Useful for diagnosing why
    /// a tenant is suddenly seeing 429 storms.
    ratelimit_bucket_available,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_ratelimit_bucket_available",
            "Tokens currently available in tenant bucket (D18 § 17.1 #5).",
        ),
        &["tenant_id"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 6. Gauge — leader-election held flag
// ====================================================================

once_metric!(
    /// `efe_leader_is_leader` — 1 if this replica currently holds
    /// the lease, 0 otherwise. Multiplied by the per-replica
    /// `efe_replica_id` gauge in Grafana to identify the leader.
    ///
    /// Alert `D18LeaderUnstable` fires when leadership flips more
    /// than 5×/10min (sign of lease misconfiguration).
    leader_is_leader,
    IntGauge,
    IntGauge::with_opts(Opts::new(
        "efe_leader_is_leader",
        "1 iff this replica holds the K8s coordination lease (D18 § 17.1 #6).",
    ))
    .expect("valid gauge opts")
);

// ====================================================================
// 7. Counter — lease acquire attempts
// ====================================================================

once_metric!(
    /// `efe_leader_acquire_attempts_total{outcome}` — count of
    /// `LeaderLock::try_acquire` calls per outcome.
    ///
    /// `outcome` ∈ `acquired` / `held_by_other` / `error`.
    leader_acquire_attempts_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_leader_acquire_attempts_total",
            "Lease acquire attempts per outcome (D18 § 17.1 #7).",
        ),
        &["outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 8. Histogram — shutdown phase duration
// ====================================================================

once_metric!(
    /// `efe_shutdown_phase_seconds{phase}` — wall-clock duration of
    /// each of the 3 shutdown phases.
    ///
    /// `phase` ∈ `drain_rpc` / `audit_flush` / `pg_close`.
    ///
    /// Buckets cover 100 ms → 60 s, with denser resolution at the
    /// 10 s phase budget so we can detect regressions.
    shutdown_phase_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_shutdown_phase_seconds",
            "Wall-clock duration per shutdown phase (D18 § 17.1 #8).",
        )
        .buckets(vec![
            0.1, 0.5, 1.0, 2.0, 5.0, 8.0, 10.0, 15.0, 20.0, 30.0, 45.0, 60.0,
        ]),
        &["phase"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 9. Counter — bench runs
// ====================================================================

once_metric!(
    /// `efe_bench_runs_total{workload, result}` — count of bench
    /// runs per workload pattern × outcome.
    ///
    /// `workload` ∈ `read-heavy` / `write-heavy` / `balanced` /
    /// `burst` / `multi-tenant` / `soak`.
    /// `result`   ∈ `pass` / `fail`.
    ///
    /// Alert `D18BenchSloMissed` fires on
    /// `rate(efe_bench_runs_total{result="fail"}) > 0` for 1h —
    /// indicates a regression in any of the 7 D18 SLOs.
    bench_runs_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_bench_runs_total",
            "Bench runs per workload × result (D18 § 17.1 #9).",
        ),
        &["workload", "result"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// Standalone helper — one-shot Histogram for the global runner phase.
// (Not part of the canonical 9 ; used internally by `BenchReport`.)
// ====================================================================

once_metric!(
    /// `efe_bench_run_duration_seconds` — total wall-clock duration
    /// of one bench run. Buckets : 1 s → 30 min.
    ///
    /// Diagnostic-only ; not advertised in the architecture as a
    /// formal SLO instrument, but useful for the operator running
    /// `efe-bench run` interactively.
    bench_run_duration_seconds,
    Histogram,
    Histogram::with_opts(
        HistogramOpts::new(
            "efe_bench_run_duration_seconds",
            "Total wall-clock duration of one bench run (D18 internal).",
        )
        .buckets(vec![1.0, 5.0, 30.0, 60.0, 300.0, 600.0, 1200.0, 1800.0]),
    )
    .expect("valid histogram opts")
);

// ====================================================================
// Eager registration helper
// ====================================================================

/// Force-register every D18 instrument with the global registry.
/// Pattern identical to `register_all_d17` ; idempotent thanks to
/// `OnceLock`.
///
/// **Audit fix D18-A3** — `Server::build` MUST call this so the
/// Prometheus `/metrics` endpoint exposes the 9 D18 series as
/// zero-valued (vs "no data") even before any traffic.
pub fn register_all_d18() {
    let _ = replica_id();
    let _ = idempotency_hits_total();
    let _ = idempotency_size();
    let _ = ratelimit_decisions_total();
    let _ = ratelimit_bucket_available();
    let _ = leader_is_leader();
    let _ = leader_acquire_attempts_total();
    let _ = shutdown_phase_seconds();
    let _ = bench_runs_total();
    let _ = bench_run_duration_seconds();
}

// ====================================================================
// Convenience bump helpers — one entry point per call site.
// (Multi_replica module impls call these so the metric paths are
// centralised + audited rather than scattered.)
// ====================================================================

/// Stable string for `idempotency_hits_total` outcome label.
#[must_use]
pub const fn idempotency_outcome_str(outcome: IdempotencyOutcomeLabel) -> &'static str {
    match outcome {
        IdempotencyOutcomeLabel::Leader => "leader",
        IdempotencyOutcomeLabel::InFlight => "in_flight",
        IdempotencyOutcomeLabel::Cached => "cached",
        IdempotencyOutcomeLabel::Error => "error",
    }
}

/// Mirror of `efe_server::multi_replica::IdempotencyOutcome` discriminants
/// used purely for metric labelling. Decoupled from the runtime enum so
/// the telemetry crate stays dependency-free of efe-server.
#[derive(Debug, Clone, Copy)]
pub enum IdempotencyOutcomeLabel {
    /// Won the INSERT race.
    Leader,
    /// Another replica is processing.
    InFlight,
    /// Cached completed row.
    Cached,
    /// Lookup errored.
    Error,
}

/// Stable string for `ratelimit_decisions_total` outcome label.
#[must_use]
pub const fn ratelimit_outcome_str(outcome: RateLimitOutcomeLabel) -> &'static str {
    match outcome {
        RateLimitOutcomeLabel::Allowed => "allowed",
        RateLimitOutcomeLabel::Throttled => "throttled",
        RateLimitOutcomeLabel::Unconfigured => "unconfigured",
        RateLimitOutcomeLabel::Error => "error",
    }
}

/// Mirror of the rate-limit decision used purely for metric labelling.
#[derive(Debug, Clone, Copy)]
pub enum RateLimitOutcomeLabel {
    /// Token taken — request proceeds.
    Allowed,
    /// Bucket empty — throttled.
    Throttled,
    /// Tenant not configured.
    Unconfigured,
    /// Lookup errored.
    Error,
}

/// Stable string for `leader_acquire_attempts_total` outcome label.
#[must_use]
pub const fn leader_outcome_str(outcome: LeaderOutcomeLabel) -> &'static str {
    match outcome {
        LeaderOutcomeLabel::Acquired => "acquired",
        LeaderOutcomeLabel::HeldByOther => "held_by_other",
        LeaderOutcomeLabel::Error => "error",
    }
}

/// Mirror of the leader-election outcome used purely for metric
/// labelling.
#[derive(Debug, Clone, Copy)]
pub enum LeaderOutcomeLabel {
    /// Lease acquired by this replica.
    Acquired,
    /// Lease held by another replica.
    HeldByOther,
    /// API error from the lock.
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    #[test]
    fn register_all_d18_idempotent() {
        register_all_d18();
        register_all_d18();
    }

    #[test]
    fn replica_id_label_arity_is_one() {
        assert_eq!(replica_id().desc()[0].variable_labels.len(), 1);
    }

    #[test]
    fn idempotency_hits_label_arity_is_one() {
        assert_eq!(
            idempotency_hits_total().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn ratelimit_decisions_label_arity_is_two() {
        assert_eq!(
            ratelimit_decisions_total().desc()[0].variable_labels.len(),
            2
        );
    }

    #[test]
    fn ratelimit_bucket_available_label_arity_is_one() {
        assert_eq!(
            ratelimit_bucket_available().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn leader_is_leader_no_labels() {
        assert_eq!(leader_is_leader().desc()[0].variable_labels.len(), 0);
    }

    #[test]
    fn leader_acquire_attempts_label_arity_is_one() {
        assert_eq!(
            leader_acquire_attempts_total().desc()[0]
                .variable_labels
                .len(),
            1
        );
    }

    #[test]
    fn shutdown_phase_label_arity_is_one() {
        assert_eq!(
            shutdown_phase_seconds().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn bench_runs_label_arity_is_two() {
        assert_eq!(bench_runs_total().desc()[0].variable_labels.len(), 2);
    }

    #[test]
    fn idempotency_outcome_strings_are_stable() {
        // Architecture § 17.1 commits to these exact strings.
        assert_eq!(
            idempotency_outcome_str(IdempotencyOutcomeLabel::Leader),
            "leader"
        );
        assert_eq!(
            idempotency_outcome_str(IdempotencyOutcomeLabel::InFlight),
            "in_flight"
        );
        assert_eq!(
            idempotency_outcome_str(IdempotencyOutcomeLabel::Cached),
            "cached"
        );
        assert_eq!(
            idempotency_outcome_str(IdempotencyOutcomeLabel::Error),
            "error"
        );
    }

    #[test]
    fn ratelimit_outcome_strings_are_stable() {
        assert_eq!(
            ratelimit_outcome_str(RateLimitOutcomeLabel::Allowed),
            "allowed"
        );
        assert_eq!(
            ratelimit_outcome_str(RateLimitOutcomeLabel::Throttled),
            "throttled"
        );
        assert_eq!(
            ratelimit_outcome_str(RateLimitOutcomeLabel::Unconfigured),
            "unconfigured"
        );
        assert_eq!(
            ratelimit_outcome_str(RateLimitOutcomeLabel::Error),
            "error"
        );
    }

    #[test]
    fn leader_outcome_strings_are_stable() {
        assert_eq!(
            leader_outcome_str(LeaderOutcomeLabel::Acquired),
            "acquired"
        );
        assert_eq!(
            leader_outcome_str(LeaderOutcomeLabel::HeldByOther),
            "held_by_other"
        );
        assert_eq!(leader_outcome_str(LeaderOutcomeLabel::Error), "error");
    }

    #[test]
    fn idempotency_hits_increments() {
        let before = idempotency_hits_total()
            .with_label_values(&["leader"])
            .get();
        idempotency_hits_total()
            .with_label_values(&["leader"])
            .inc();
        let after = idempotency_hits_total()
            .with_label_values(&["leader"])
            .get();
        assert!(after >= before + 1);
    }

    #[test]
    fn shutdown_phase_observes() {
        shutdown_phase_seconds()
            .with_label_values(&["drain_rpc"])
            .observe(5.0);
        let m = shutdown_phase_seconds().with_label_values(&["drain_rpc"]);
        assert!(m.get_sample_count() >= 1);
    }

    #[test]
    fn bench_runs_label_values_pinned() {
        // Architecture § 17.1 commits to result ∈ {pass, fail}.
        for r in ["pass", "fail"] {
            bench_runs_total()
                .with_label_values(&["read-heavy", r])
                .inc();
        }
    }

    #[test]
    fn replica_id_set_get() {
        replica_id().with_label_values(&["pod-0"]).set(1);
        let v = replica_id().with_label_values(&["pod-0"]).get();
        assert_eq!(v, 1);
    }
}
