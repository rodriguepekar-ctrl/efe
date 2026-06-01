//! PR #17 Group A — Iterate metrics (5 series).
//!
//! Architecture reference : `docs/EFE_A1_PHASE2_PR17_ARCHITECTURE.md` § 6.
//!
//! | Series                                          | Type        | Labels                          |
//! | ----------------------------------------------- | ----------- | ------------------------------- |
//! | `efe_meta_swarm_iterate_duration_seconds`       | HistogramVec| `result`,`manifest_version`     |
//! | `efe_meta_swarm_iterate_total`                  | IntCounterVec | `result`,`failure_reason`      |
//! | `efe_meta_swarm_iterate_inflight`               | IntGauge    | —                               |
//! | `efe_meta_swarm_iterate_iterations`             | HistogramVec| `convergence`                   |
//! | `efe_meta_swarm_iterate_convergence_error`      | HistogramVec| `convergence`                   |
//!
//! Pre-touch policy : every closed-taxonomy combination is materialised
//! at registration time so `gather()` emits the series with `0` from
//! boot (AC5 / § 15).

use std::time::Duration;

use prometheus::{
    register_histogram_vec_with_registry, register_int_counter_vec_with_registry,
    register_int_gauge_with_registry, HistogramVec, IntCounterVec, IntGauge, Registry,
};

use super::taxonomies::{Convergence, FailureReason, Outcome};

// ─────────────────────────────────────────────────────────────────────
// Bucket policies — frozen per chart version (AD6 / AD16).
// ─────────────────────────────────────────────────────────────────────

/// Wide-range latency buckets : 1 ms → 30 s, 13 buckets.
///
/// Sized for end-to-end Iterate calls, whose realistic upper bound is
/// `EFE_META_SWARM_DEFAULT_MAX_WALL_MS = 15 000` with 100 % margin.
pub const ITERATE_DURATION_BUCKETS_SECONDS: &[f64] = &[
    0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// Iteration-count buckets : log-spaced 1 → 1 000.  +∞ implicit bucket
/// captures the `max_iters` overflow case.
pub const ITERATE_ITERATIONS_BUCKETS: &[f64] =
    &[1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1_000.0];

/// Convergence-error buckets : log-spaced 1e-8 → 10, 10 buckets,
/// spanning 9 decades.  Captures the realistic range of `tolerance`
/// settings (1e-4 default, 1e-6 strict, 1e-2 loose).
pub const ITERATE_CONVERGENCE_ERROR_BUCKETS: &[f64] =
    &[1e-8, 1e-7, 1e-6, 1e-5, 1e-4, 1e-3, 1e-2, 1e-1, 1.0, 10.0];

// ─────────────────────────────────────────────────────────────────────
// IterateMetrics — the 5-series bundle.
// ─────────────────────────────────────────────────────────────────────

/// Group A — 5 Prometheus series covering the controller-side Iterate
/// call path.  Cheap to clone (all internals are `Arc`-shared).
#[derive(Debug, Clone)]
pub struct IterateMetrics {
    /// `efe_meta_swarm_iterate_duration_seconds` — wall-clock latency
    /// histogram of an end-to-end Iterate gRPC call.
    pub duration_seconds: HistogramVec,
    /// `efe_meta_swarm_iterate_total` — counter of Iterate completions,
    /// partitioned by `result` + `failure_reason`.
    pub total: IntCounterVec,
    /// `efe_meta_swarm_iterate_inflight` — gauge of currently-running
    /// Iterate calls on this pod.  Bounded by `InflightSemaphore`.
    pub inflight: IntGauge,
    /// `efe_meta_swarm_iterate_iterations` — histogram of inner Phase-1
    /// iteration counts per Iterate call.
    pub iterations: HistogramVec,
    /// `efe_meta_swarm_iterate_convergence_error` — histogram of final
    /// L2 residual at Iterate termination.
    pub convergence_error: HistogramVec,
}

impl IterateMetrics {
    /// Register every series on `reg` + pre-touch the closed-taxonomy
    /// product.
    pub fn register(reg: &Registry) -> prometheus::Result<Self> {
        let duration_seconds = register_histogram_vec_with_registry!(
            "efe_meta_swarm_iterate_duration_seconds",
            "End-to-end Iterate gRPC call duration in seconds, partitioned by outcome and manifest version.",
            &["result", "manifest_version"],
            ITERATE_DURATION_BUCKETS_SECONDS.to_vec(),
            reg,
        )?;

        let total = register_int_counter_vec_with_registry!(
            "efe_meta_swarm_iterate_total",
            "Total Iterate calls completed, partitioned by outcome and failure reason.",
            &["result", "failure_reason"],
            reg,
        )?;

        let inflight = register_int_gauge_with_registry!(
            "efe_meta_swarm_iterate_inflight",
            "Number of Iterate calls currently in flight on this pod.",
            reg,
        )?;

        let iterations = register_histogram_vec_with_registry!(
            "efe_meta_swarm_iterate_iterations",
            "Number of inner-loop iterations consumed by an Iterate call before convergence or termination.",
            &["convergence"],
            ITERATE_ITERATIONS_BUCKETS.to_vec(),
            reg,
        )?;

        let convergence_error = register_histogram_vec_with_registry!(
            "efe_meta_swarm_iterate_convergence_error",
            "Final L2 residual error at Iterate termination, partitioned by convergence outcome.",
            &["convergence"],
            ITERATE_CONVERGENCE_ERROR_BUCKETS.to_vec(),
            reg,
        )?;

        let m = Self {
            duration_seconds,
            total,
            inflight,
            iterations,
            convergence_error,
        };
        m.pretouch_closed_taxonomies();
        Ok(m)
    }

    /// Pre-touch every closed-taxonomy combination so `gather()` emits
    /// the series at `0` before any observation (AC5 / § 15).
    ///
    /// The `manifest_version` label is OPEN (varies with deploy).  We
    /// use a sentinel `"(unknown)"` so the HELP+TYPE comments appear
    /// in `/metrics` before the first real observation ; once the
    /// first Iterate fires, the real manifest version label coexists
    /// alongside the sentinel.
    fn pretouch_closed_taxonomies(&self) {
        for outcome in Outcome::all() {
            for reason in FailureReason::all() {
                self.total
                    .with_label_values(&[outcome.as_str(), reason.as_str()])
                    .inc_by(0);
            }
            // Pre-touch `duration_seconds` with the "(unknown)"
            // manifest_version sentinel.  Sentinel preferred over an
            // open metric-family with no children because the
            // Prometheus text encoder skips empty families entirely
            // (HELP / TYPE comments not emitted) — dashboards bound to
            // the metric name would render "no data" until the first
            // Iterate call.
            self.duration_seconds
                .with_label_values(&[outcome.as_str(), "(unknown)"])
                .observe(0.0);
        }
        for conv in Convergence::all() {
            // Pre-touch by recording a 0-bucket entry.
            self.iterations
                .with_label_values(&[conv.as_str()])
                .observe(0.0);
            self.convergence_error
                .with_label_values(&[conv.as_str()])
                .observe(0.0);
        }
    }

    /// Observe an Iterate call's duration ; both labels supplied by
    /// the handler.  Wraps a single `HistogramVec::observe`.
    pub fn observe_duration(&self, duration: Duration, outcome: Outcome, manifest_version: &str) {
        self.duration_seconds
            .with_label_values(&[outcome.as_str(), manifest_version])
            .observe(duration.as_secs_f64());
    }

    /// Increment the total counter on Iterate completion.
    pub fn record_total(&self, outcome: Outcome, reason: FailureReason) {
        self.total
            .with_label_values(&[outcome.as_str(), reason.as_str()])
            .inc();
    }

    /// Mark an Iterate as starting (inflight gauge ++).
    pub fn enter(&self) {
        self.inflight.inc();
    }

    /// Mark an Iterate as ending (inflight gauge --).  Symmetric to
    /// [`Self::enter`].  Safe to call from a `Drop` impl.
    pub fn exit(&self) {
        self.inflight.dec();
    }

    /// Observe the number of inner iterations consumed by an
    /// Iterate call.
    pub fn observe_iterations(&self, iterations: u32, convergence: Convergence) {
        self.iterations
            .with_label_values(&[convergence.as_str()])
            .observe(f64::from(iterations));
    }

    /// Observe the final L2 residual at Iterate termination.
    ///
    /// `residual` must be finite ; NaN / inf observations are silently
    /// dropped (Prometheus histograms reject these).  Operators
    /// chasing missing histogram counts should check the trace for
    /// `iterate_observe: residual was non-finite`.
    pub fn observe_convergence_error(&self, residual: f64, convergence: Convergence) {
        if !residual.is_finite() {
            tracing::warn!(
                residual,
                convergence = ?convergence,
                "iterate_observe: residual was non-finite — dropped",
            );
            return;
        }
        self.convergence_error
            .with_label_values(&[convergence.as_str()])
            .observe(residual);
    }
}

/// RAII guard incrementing [`IterateMetrics::inflight`] on construction
/// and decrementing on drop.  Use this in async handlers to ensure
/// the gauge tracks panics / cancellations correctly.
#[derive(Debug)]
pub struct InflightGuard<'a> {
    metrics: &'a IterateMetrics,
}

impl<'a> InflightGuard<'a> {
    /// Construct + increment the gauge.
    pub fn new(metrics: &'a IterateMetrics) -> Self {
        metrics.enter();
        Self { metrics }
    }
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.metrics.exit();
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> Registry {
        Registry::new()
    }

    #[test]
    fn register_exposes_all_five_series() {
        let r = reg();
        let _m = IterateMetrics::register(&r).expect("register");
        let body = prometheus::TextEncoder::new()
            .encode_to_string(&r.gather())
            .expect("encode");
        for name in [
            "efe_meta_swarm_iterate_duration_seconds",
            "efe_meta_swarm_iterate_total",
            "efe_meta_swarm_iterate_inflight",
            "efe_meta_swarm_iterate_iterations",
            "efe_meta_swarm_iterate_convergence_error",
        ] {
            assert!(body.contains(name), "missing series : {name}");
        }
    }

    #[test]
    fn pretouch_materialises_closed_product_for_total() {
        let r = reg();
        let _m = IterateMetrics::register(&r).expect("register");
        let body = String::from_utf8(
            prometheus::TextEncoder::new()
                .encode_to_string(&r.gather())
                .unwrap()
                .into_bytes(),
        )
        .unwrap();
        // 5 outcomes × 8 failure_reasons = 40 child series, each at 0.
        for outcome in Outcome::all() {
            for reason in FailureReason::all() {
                let key = format!(
                    "efe_meta_swarm_iterate_total{{failure_reason=\"{}\",result=\"{}\"}} 0",
                    reason.as_str(),
                    outcome.as_str(),
                );
                assert!(
                    body.contains(&key),
                    "pretouch missing : {key}\nbody=\n{body}"
                );
            }
        }
    }

    #[test]
    fn observe_duration_records_sample() {
        let r = reg();
        let m = IterateMetrics::register(&r).expect("register");
        m.observe_duration(Duration::from_millis(42), Outcome::Ok, "v0.2.0");
        let body = String::from_utf8(
            prometheus::TextEncoder::new()
                .encode_to_string(&r.gather())
                .unwrap()
                .into_bytes(),
        )
        .unwrap();
        // 42 ms falls into the 0.05 bucket (50 ms).
        assert!(body.contains("efe_meta_swarm_iterate_duration_seconds_count{manifest_version=\"v0.2.0\",result=\"ok\"} 1"));
    }

    #[test]
    fn record_total_increments_correct_label() {
        let r = reg();
        let m = IterateMetrics::register(&r).expect("register");
        m.record_total(Outcome::Error, FailureReason::DimMismatch);
        let count = m
            .total
            .get_metric_with_label_values(&["error", "dim_mismatch"])
            .expect("label exists")
            .get();
        assert_eq!(count, 1);
    }

    #[test]
    fn inflight_guard_increments_and_decrements() {
        let r = reg();
        let m = IterateMetrics::register(&r).expect("register");
        assert_eq!(m.inflight.get(), 0);
        {
            let _g = InflightGuard::new(&m);
            assert_eq!(m.inflight.get(), 1);
            {
                let _g2 = InflightGuard::new(&m);
                assert_eq!(m.inflight.get(), 2);
            }
            assert_eq!(m.inflight.get(), 1);
        }
        assert_eq!(m.inflight.get(), 0);
    }

    #[test]
    fn observe_iterations_records_count_with_convergence() {
        let r = reg();
        let m = IterateMetrics::register(&r).expect("register");
        m.observe_iterations(47, Convergence::Converged);
        let body = String::from_utf8(
            prometheus::TextEncoder::new()
                .encode_to_string(&r.gather())
                .unwrap()
                .into_bytes(),
        )
        .unwrap();
        // 47 iterations fall in the 50-bucket.  After pretouch (count=1)
        // + this observation (count=1), total count == 2.
        assert!(
            body.contains("efe_meta_swarm_iterate_iterations_count{convergence=\"converged\"} 2")
        );
    }

    #[test]
    fn observe_convergence_error_with_residual() {
        let r = reg();
        let m = IterateMetrics::register(&r).expect("register");
        m.observe_convergence_error(1e-5, Convergence::Converged);
        let body = String::from_utf8(
            prometheus::TextEncoder::new()
                .encode_to_string(&r.gather())
                .unwrap()
                .into_bytes(),
        )
        .unwrap();
        // Pretouch (0.0) + this (1e-5) = 2 samples.
        assert!(body.contains(
            "efe_meta_swarm_iterate_convergence_error_count{convergence=\"converged\"} 2"
        ));
    }

    #[test]
    fn observe_convergence_error_drops_non_finite() {
        let r = reg();
        let m = IterateMetrics::register(&r).expect("register");
        m.observe_convergence_error(f64::NAN, Convergence::Diverged);
        m.observe_convergence_error(f64::INFINITY, Convergence::Diverged);
        let body = String::from_utf8(
            prometheus::TextEncoder::new()
                .encode_to_string(&r.gather())
                .unwrap()
                .into_bytes(),
        )
        .unwrap();
        // Only the pretouch sample (0.0) lands ; non-finite dropped.
        assert!(body.contains(
            "efe_meta_swarm_iterate_convergence_error_count{convergence=\"diverged\"} 1"
        ));
    }

    #[test]
    fn bucket_lists_are_sized_per_architecture() {
        assert_eq!(ITERATE_DURATION_BUCKETS_SECONDS.len(), 13);
        assert_eq!(ITERATE_ITERATIONS_BUCKETS.len(), 9);
        assert_eq!(ITERATE_CONVERGENCE_ERROR_BUCKETS.len(), 10);
    }
}
