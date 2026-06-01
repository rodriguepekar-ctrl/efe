//! PR #17 Group E — Manifest-reload extension metrics (2 series).
//!
//! Architecture reference : `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 10.
//!
//! | Series                                              | Type           | Labels             |
//! | --------------------------------------------------- | -------------- | ------------------ |
//! | `efe_meta_swarm_manifest_reload_duration_seconds`   | Histogram      | —                  |
//! | `efe_meta_swarm_manifest_reload_total`              | IntCounterVec  | `result`,`trigger` |
//!
//! ## Relation to PR #12
//!
//! PR #12 already shipped `efe_meta_swarm_reload_duration_microseconds`
//! (covers the write-lock window) + `efe_meta_swarm_reload_attempts_
//! total` (no breakdown).  PR #17 adds two complementary series :
//!
//!   * The `_seconds` histogram covers end-to-end reload (disk-read →
//!     in-memory swap), giving the operator the same unit as the rest
//!     of the metric surface.
//!   * The `_total{result,trigger}` counter lets the operator
//!     distinguish "sighup reloads always succeed but grpc reloads
//!     fail often" → likely a gRPC API misuse, not a manifest content
//!     problem (architecture § 10.2).
//!
//! Both series coexist with PR #12's microseconds variant — no
//! deprecation per AD25.

use std::time::Duration;

use prometheus::{
    register_histogram_with_registry, register_int_counter_vec_with_registry, Histogram,
    HistogramOpts, IntCounterVec, Registry,
};

use super::taxonomies::{ReloadOutcome, ReloadTrigger};

/// Bucket policy for the end-to-end reload duration.  Spans 1 ms → 5 s
/// to capture both fast manifest swaps and slow disk-bound reloads.
pub const MANIFEST_RELOAD_DURATION_BUCKETS_SECONDS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
];

/// Group E — 2 Prometheus series covering the manifest reload pathway.
#[derive(Debug, Clone)]
pub struct ManifestReloadMetrics {
    /// `efe_meta_swarm_manifest_reload_duration_seconds` — wall-clock
    /// duration of a manifest reload (disk read → in-memory swap).
    pub duration_seconds: Histogram,
    /// `efe_meta_swarm_manifest_reload_total` — counter of reload
    /// attempts, labelled by outcome + trigger.
    pub total: IntCounterVec,
}

impl ManifestReloadMetrics {
    /// Register both series + pre-touch the closed taxonomy product.
    pub fn register(reg: &Registry) -> prometheus::Result<Self> {
        let duration_seconds = register_histogram_with_registry!(
            HistogramOpts::new(
                "efe_meta_swarm_manifest_reload_duration_seconds",
                "Wall-clock duration of a manifest reload from disk-read to in-memory swap, in seconds.",
            )
            .buckets(MANIFEST_RELOAD_DURATION_BUCKETS_SECONDS.to_vec()),
            reg,
        )?;

        let total = register_int_counter_vec_with_registry!(
            "efe_meta_swarm_manifest_reload_total",
            "Total manifest reload attempts, partitioned by outcome and trigger.",
            &["result", "trigger"],
            reg,
        )?;

        let m = Self {
            duration_seconds,
            total,
        };
        m.pretouch_closed_taxonomies();
        Ok(m)
    }

    /// Pre-touch every (result × trigger) = 2 × 3 = 6 child series.
    fn pretouch_closed_taxonomies(&self) {
        for outcome in ReloadOutcome::all() {
            for trigger in ReloadTrigger::all() {
                self.total
                    .with_label_values(&[outcome.as_str(), trigger.as_str()])
                    .inc_by(0);
            }
        }
    }

    /// Observe the duration of one reload attempt.
    pub fn observe_duration(&self, duration: Duration) {
        self.duration_seconds.observe(duration.as_secs_f64());
    }

    /// Increment the counter on reload completion.
    pub fn record_total(&self, outcome: ReloadOutcome, trigger: ReloadTrigger) {
        self.total
            .with_label_values(&[outcome.as_str(), trigger.as_str()])
            .inc();
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::TextEncoder;

    fn body(reg: &Registry) -> String {
        TextEncoder::new()
            .encode_to_string(&reg.gather())
            .expect("encode")
    }

    #[test]
    fn register_creates_both_series() {
        let r = Registry::new();
        let _m = ManifestReloadMetrics::register(&r).expect("register");
        let b = body(&r);
        assert!(b.contains("efe_meta_swarm_manifest_reload_duration_seconds"));
        assert!(b.contains("efe_meta_swarm_manifest_reload_total"));
    }

    #[test]
    fn pretouch_emits_all_six_combinations() {
        let r = Registry::new();
        let _m = ManifestReloadMetrics::register(&r).expect("register");
        let b = body(&r);
        for outcome in ReloadOutcome::all() {
            for trigger in ReloadTrigger::all() {
                let needle = format!(
                    "efe_meta_swarm_manifest_reload_total{{result=\"{}\",trigger=\"{}\"}} 0",
                    outcome.as_str(),
                    trigger.as_str(),
                );
                assert!(b.contains(&needle), "missing : {needle}\n{b}");
            }
        }
    }

    #[test]
    fn observe_duration_records_sample() {
        let r = Registry::new();
        let m = ManifestReloadMetrics::register(&r).expect("register");
        m.observe_duration(Duration::from_millis(75));
        assert_eq!(m.duration_seconds.get_sample_count(), 1);
    }

    #[test]
    fn record_total_increments_per_pair() {
        let r = Registry::new();
        let m = ManifestReloadMetrics::register(&r).expect("register");
        m.record_total(ReloadOutcome::Ok, ReloadTrigger::Sighup);
        m.record_total(ReloadOutcome::Ok, ReloadTrigger::Sighup);
        m.record_total(ReloadOutcome::Error, ReloadTrigger::Grpc);
        let ok_sighup = m
            .total
            .get_metric_with_label_values(&["ok", "sighup"])
            .unwrap()
            .get();
        let err_grpc = m
            .total
            .get_metric_with_label_values(&["error", "grpc"])
            .unwrap()
            .get();
        assert_eq!(ok_sighup, 2);
        assert_eq!(err_grpc, 1);
    }

    #[test]
    fn bucket_list_matches_architecture() {
        assert_eq!(MANIFEST_RELOAD_DURATION_BUCKETS_SECONDS.len(), 11);
    }
}
