//! Sprint **D14-bis** — HMAC-chained audit log instrumentation.
//!
//! 6 nouveaux instruments dédiés au sink `HmacChainedAuditSink`
//! (cf. `docs/bench/d14bis_hmac_audit_chain_architecture.md` § 17).
//!
//! Conventions identiques à `d14_instruments.rs` :
//!   * macro `once_metric!` (re-déclarée localement) ;
//!   * tous les instruments enregistrés sur `global_registry` ;
//!   * labels strictement bornés.
//!
//! # Cardinalité (estim.)
//!
//! Pour 100 tenants, 1 chaîne par tenant, 4 keys actives (rotation in flight) :
//!
//! | Instrument                            | Combinations |
//! |---------------------------------------|-------------:|
//! | `efe_audit_events_signed_total`       | 3 actions = 3 |
//! | `efe_audit_sign_failures_total`       | 3 actions × ~5 codes = 15 |
//! | `efe_audit_chain_head_seq`            | 100 chains = 100 |
//! | `efe_audit_sign_duration_seconds`     | 3 actions = 3 |
//! | `efe_audit_keys_active`               | 1 (no label) |
//! | `efe_audit_chain_break_detected_total`| 100 chains = 100 |
//!
//! Total ~220 séries — négligeable vs le budget D13 (50 000 panic).

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts,
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
// Counters — happy path + failures
// =====================================================================

once_metric!(
    /// `efe_audit_events_signed_total{action}` — count of audit events
    /// successfully signed AND persisted to the `audit_events` table.
    /// Increments only after the COMMIT lands.
    audit_events_signed_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_audit_events_signed_total",
            "Number of audit events successfully signed and persisted (post-COMMIT).",
        ),
        &["action"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_audit_sign_failures_total{action, error}` — count of audit
    /// emit failures. Critical alert : non-zero rate triggers
    /// `D14bisAuditSignFailing` (cf. architecture doc § 17.3).
    audit_sign_failures_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_audit_sign_failures_total",
            "Number of audit emit failures by action + error class.",
        ),
        &["action", "error"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Gauges — chain head + key inventory
// =====================================================================

once_metric!(
    /// `efe_audit_chain_head_seq{chain_id}` — last committed `seq` per
    /// chain. Useful for monitoring chain growth + catching stalled
    /// chains (no new events for > X minutes triggers
    /// `D14bisAuditChainStalled`).
    audit_chain_head_seq,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_audit_chain_head_seq",
            "Last committed seq per audit chain (set by self-verify pass).",
        ),
        &["chain_id"],
    )
    .expect("valid gauge opts")
);

once_metric!(
    /// `efe_audit_keys_active` — number of HMAC keys loaded
    /// (active + verify-only). Sanity check : 0 ⇒ misconfiguration
    /// (alert `D14bisAuditKeysMissing`).
    audit_keys_active,
    IntGauge,
    IntGauge::new(
        "efe_audit_keys_active",
        "Number of HMAC keys loaded for audit signing (active + verify-only).",
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Histogram — sign latency
// =====================================================================

once_metric!(
    /// `efe_audit_sign_duration_seconds{action}` — wall-clock duration
    /// of one sign+insert (begin tx → commit). Buckets : 1ms .. 4.096s
    /// exponential base 2 (12 buckets). Cf. SLO `D14bis-AUD-2` ≤ 10 ms p99.
    audit_sign_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_audit_sign_duration_seconds",
            "Wall-clock duration of one HmacChainedAuditSink sign+insert.",
        )
        .buckets(exponential_buckets(0.001, 2.0, 12).expect("valid buckets")),
        &["action"],
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Critical : chain break detection
// =====================================================================

once_metric!(
    /// `efe_audit_chain_break_detected_total{chain_id}` — cumulative
    /// count of chain breaks detected by the embedded periodic
    /// self-verify task. Fires the alert
    /// `D14bisAuditChainCompromised` on any non-zero increment —
    /// **page-level severity**.
    audit_chain_break_detected_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_audit_chain_break_detected_total",
            "Number of chain breaks detected by self-verify (tampering or deletion).",
        ),
        &["chain_id"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Eager registration
// =====================================================================

/// Force-register every D14-bis instrument with the global registry
/// so the Prometheus `/metrics` endpoint exposes them as zero-valued
/// series even before the first audit event. Without this, panels
/// in the D13 dashboards show "no data" until the first register
/// RPC ; with it, dashboards render immediately and alerts have
/// known label sets to match.
///
/// Idempotent — the `once_metric!` macros guard against
/// double-registration. Called from
/// `runtime::Server::build` when audit signing is enabled.
pub fn register_all_d14bis() {
    let _ = audit_events_signed_total();
    let _ = audit_sign_failures_total();
    let _ = audit_chain_head_seq();
    let _ = audit_keys_active();
    let _ = audit_sign_duration_seconds();
    let _ = audit_chain_break_detected_total();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_counter_increments() {
        let labels = ["register"];
        let before = audit_events_signed_total().with_label_values(&labels).get();
        audit_events_signed_total().with_label_values(&labels).inc();
        let after = audit_events_signed_total().with_label_values(&labels).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn sign_failures_counter_increments() {
        let labels = ["evolve", "persist"];
        let before = audit_sign_failures_total().with_label_values(&labels).get();
        audit_sign_failures_total().with_label_values(&labels).inc();
        let after = audit_sign_failures_total().with_label_values(&labels).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn chain_head_seq_is_settable() {
        audit_chain_head_seq()
            .with_label_values(&["tenant:acme"])
            .set(12_345);
        assert_eq!(
            audit_chain_head_seq()
                .with_label_values(&["tenant:acme"])
                .get(),
            12_345
        );
    }

    #[test]
    fn keys_active_gauge_is_settable() {
        audit_keys_active().set(3);
        assert_eq!(audit_keys_active().get(), 3);
    }

    #[test]
    fn sign_duration_histogram_observes() {
        audit_sign_duration_seconds()
            .with_label_values(&["register"])
            .observe(0.005);
        let metric = audit_sign_duration_seconds().with_label_values(&["register"]);
        assert!(metric.get_sample_count() >= 1);
    }

    #[test]
    fn chain_break_counter_increments() {
        let labels = ["tenant:acme"];
        let before = audit_chain_break_detected_total()
            .with_label_values(&labels)
            .get();
        audit_chain_break_detected_total()
            .with_label_values(&labels)
            .inc();
        let after = audit_chain_break_detected_total()
            .with_label_values(&labels)
            .get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn register_all_d14bis_idempotent() {
        register_all_d14bis();
        register_all_d14bis();
        // No assertion ; absence of panic is the signal.
    }
}
