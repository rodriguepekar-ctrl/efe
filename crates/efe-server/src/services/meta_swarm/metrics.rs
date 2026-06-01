//! Prometheus metrics for the meta-swarm subsystem.
//!
//! Names live in the `efe_meta_swarm_*` namespace to avoid collision
//! with the Phase-1 swarm metrics (`efe_swarm_*`).  Registered only
//! when `EFE_META_SWARM_ENABLED=true` (cf. PR #11 architecture § 9.5
//! and AC12 / AC13).
//!
//! PR #11 ships three metrics (Health-probe surface).  PR #15 will
//! add `iterate_calls_total`, `iterate_call_duration_seconds`,
//! `iterate_errors_total{code}` for the dispatched calls.

use once_cell::sync::OnceCell;
use prometheus::{Counter, Histogram, HistogramOpts, Opts, Registry};

/// Process-wide memoisation of the meta-swarm metrics handle for the
/// global `prometheus::default_registry()`.  First call registers ;
/// subsequent calls return the same `Arc`-shared inner counters.
///
/// Production rationale : `Server::build()` may be called multiple
/// times in one process (e.g. unit tests that exercise the boot path
/// with different configs).  Prometheus rejects duplicate metric
/// names in the same registry with `Error::AlreadyReg`, which would
/// turn the second build into a noisy panic.
///
/// Tests that need a fresh, isolated registry continue to call
/// [`MetaSwarmMetrics::register`] directly with `Registry::new()`.
static DEFAULT_REGISTRY_HANDLE: OnceCell<MetaSwarmMetrics> = OnceCell::new();

/// Health-probe metrics for the meta-swarm subsystem.  Cheap to
/// clone (each `Counter` / `Histogram` is internally `Arc`-shared).
#[derive(Clone, Debug)]
pub struct MetaSwarmMetrics {
    /// Total Health probes issued by `efe-server` against the
    /// controller (counter, monotonic, increments by 1 per probe).
    pub health_probes_total: Counter,

    /// Total Health probes that returned non-SERVING or failed
    /// with a transport error.  Alertable signal for the SLO.
    pub health_probe_errors_total: Counter,

    /// Health probe latency in seconds.  Histogram with buckets
    /// tuned for localhost (1 ms – 5 s).
    pub health_probe_duration_seconds: Histogram,
}

impl MetaSwarmMetrics {
    /// Register into the global `prometheus::default_registry()` ;
    /// idempotent through a process-wide `OnceCell`.  This is the
    /// production path used by `Server::build()` so a multi-build
    /// process doesn't panic on duplicate registration.
    ///
    /// Concurrency : uses [`OnceCell::get_or_try_init`] so a single
    /// thread runs the registration closure even under contention.
    /// Without this, a naïve check-then-set pattern would let two
    /// threads BOTH pass the `get()` check, BOTH try to register,
    /// and the **second** would return `AlreadyReg` (bug).
    ///
    /// Tests with isolated registries should use [`Self::register`]
    /// directly.
    ///
    /// # Errors
    ///
    /// Returns the underlying [`prometheus::Error`] only on the
    /// FIRST call across the process lifetime ; subsequent calls
    /// always return the memoised handle.
    pub fn register_into_default_registry() -> Result<Self, prometheus::Error> {
        DEFAULT_REGISTRY_HANDLE
            .get_or_try_init(|| Self::register(prometheus::default_registry()))
            .cloned()
    }

    /// Register the three meta-swarm metrics into the provided
    /// `prometheus::Registry`.  Used by tests with isolated
    /// registries and (indirectly via [`Self::register_into_default_registry`])
    /// by `Server::build()` for the production path.
    ///
    /// # Errors
    ///
    /// Returns [`prometheus::Error::AlreadyReg`] if any metric name
    /// is already registered in `reg`.  Defence in depth : the
    /// caller (the harness) should own a fresh `Registry` per
    /// `Server` lifetime.
    pub fn register(reg: &Registry) -> Result<Self, prometheus::Error> {
        let health_probes_total = Counter::with_opts(Opts::new(
            "efe_meta_swarm_health_probes_total",
            "Number of MetaSwarmService::Health probes issued by efe-server",
        ))?;
        let health_probe_errors_total = Counter::with_opts(Opts::new(
            "efe_meta_swarm_health_probe_errors_total",
            "Number of Health probes that returned non-SERVING or a transport error",
        ))?;
        let health_probe_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "efe_meta_swarm_health_probe_duration_seconds",
                "Health probe latency in seconds",
            )
            .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
        )?;
        reg.register(Box::new(health_probes_total.clone()))?;
        reg.register(Box::new(health_probe_errors_total.clone()))?;
        reg.register(Box::new(health_probe_duration_seconds.clone()))?;
        Ok(Self {
            health_probes_total,
            health_probe_errors_total,
            health_probe_duration_seconds,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::Encoder as _;

    #[test]
    fn register_into_fresh_registry_succeeds_and_exposes_three_series() {
        let reg = Registry::new();
        let m = MetaSwarmMetrics::register(&reg).expect("register");
        // Bump every metric to make sure they're alive and labelled.
        m.health_probes_total.inc();
        m.health_probe_errors_total.inc();
        m.health_probe_duration_seconds.observe(0.0042);

        let mut buf = Vec::new();
        let encoder = prometheus::TextEncoder::new();
        encoder.encode(&reg.gather(), &mut buf).expect("encode");
        let text = String::from_utf8(buf).expect("utf8");
        assert!(
            text.contains("efe_meta_swarm_health_probes_total"),
            "missing probes_total : {text}",
        );
        assert!(
            text.contains("efe_meta_swarm_health_probe_errors_total"),
            "missing errors_total : {text}",
        );
        assert!(
            text.contains("efe_meta_swarm_health_probe_duration_seconds"),
            "missing duration : {text}",
        );
    }

    #[test]
    fn double_register_into_same_registry_fails() {
        // Defence in depth : if the caller mistakenly registers twice
        // (e.g. SIGHUP reload bug), Prometheus rejects it.  The
        // harness must build a fresh `Registry` per `efe-server`
        // RuntimeState (cf. risk PR11-R4).
        let reg = Registry::new();
        MetaSwarmMetrics::register(&reg).expect("first register");
        let err = MetaSwarmMetrics::register(&reg).expect_err("double register must fail");
        assert!(matches!(err, prometheus::Error::AlreadyReg), "{err:?}",);
    }
}
