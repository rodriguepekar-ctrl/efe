//! The 12 MVP Prometheus instruments.
//!
//! All instruments are **namespace-prefixed** with `efe_` and use
//! finite-set labels (see `M11_ARCHITECTURE.md` §13.2 + §22 R2) to
//! prevent cardinality explosion.
//!
//! Each instrument is registered on first access against the global
//! registry (see [`super::registry::global_registry`]).

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec,
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
                // Registration is idempotent per instrument; if the
                // registry already holds a metric by that name we still
                // want to surface it, so unregister errors are swallowed.
                let _ = global_registry().register(Box::new(m.clone()));
                m
            })
        }
    };
}

// =====================================================================
// gRPC surface (emitted by MetricsLayer in M10)
// =====================================================================

once_metric!(
    /// `efe_grpc_requests_total{method, code, tenant}` — lifetime
    /// counter of gRPC requests handled by the server.
    grpc_requests_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_grpc_requests_total",
            "Total gRPC requests handled, labelled by method, gRPC status code and tenant."
        ),
        &["method", "code", "tenant"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_grpc_request_duration_seconds{method, tenant}` — histogram
    /// of end-to-end gRPC request latencies.
    grpc_request_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_grpc_request_duration_seconds",
            "End-to-end gRPC request latency in seconds."
        )
        .buckets(exponential_buckets(0.001, 2.0, 12).expect("valid buckets")),
        &["method", "tenant"],
    )
    .expect("valid histogram opts")
);

once_metric!(
    /// `efe_grpc_requests_in_flight{method}` — gauge of concurrent
    /// gRPC requests.
    grpc_requests_in_flight,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_grpc_requests_in_flight",
            "Number of gRPC requests currently being processed."
        ),
        &["method"],
    )
    .expect("valid gauge opts")
);

// =====================================================================
// Swarm execution
// =====================================================================

once_metric!(
    /// `efe_swarm_rounds_total` — global counter of fusion rounds
    /// executed across every swarm.
    swarm_rounds_total,
    IntCounter,
    IntCounter::new(
        "efe_swarm_rounds_total",
        "Total fusion rounds executed across all swarms."
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_swarm_duration_seconds{verdict, tenant}` — histogram of
    /// end-to-end swarm execution latency.
    swarm_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_swarm_duration_seconds",
            "Full swarm execution latency in seconds, labelled by verdict and tenant."
        )
        .buckets(exponential_buckets(0.005, 2.0, 12).expect("valid buckets")),
        &["verdict", "tenant"],
    )
    .expect("valid histogram opts")
);

once_metric!(
    /// `efe_swarm_confidence{tenant}` — distribution of final
    /// confidence values on converged swarms.
    swarm_confidence,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_swarm_confidence",
            "Final confidence of converged swarms (0..1)."
        )
        .buckets(vec![0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99, 1.0]),
        &["tenant"],
    )
    .expect("valid histogram opts")
);

// =====================================================================
// Errors
// =====================================================================

once_metric!(
    /// `efe_errors_total{kind, tenant}` — counter of domain errors
    /// classified by `EfeError::code()`.
    errors_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_errors_total",
            "Domain errors observed, labelled by EfeError::code()."
        ),
        &["kind", "tenant"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Alignment (M9)
// =====================================================================

once_metric!(
    /// `efe_breaches_total{tenant, task, context}` — counter of
    /// alignment breaches reported to the registry.
    breaches_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_breaches_total",
            "Alignment breaches reported to the registry."
        ),
        &["tenant", "task", "context"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_breach_magnitude` — histogram of `observed_delta -
    /// cone_radius` values at breach time.
    breach_magnitude,
    Histogram,
    Histogram::with_opts(
        HistogramOpts::new(
            "efe_breach_magnitude",
            "observed_delta - cone_radius at breach (unitless, >= 0)."
        )
        .buckets(vec![0.0, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
    )
    .expect("valid histogram opts")
);

once_metric!(
    /// `efe_cone_registrations_total{event, tenant}` — counter of
    /// cone registry mutations (registered / evolved / deleted).
    cone_registrations_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_cone_registrations_total",
            "Registry events, labelled by event kind (registered/evolved/deleted)."
        ),
        &["event", "tenant"],
    )
    .expect("valid counter opts")
);

once_metric!(
    /// `efe_policy_rejections_total{reason}` — counter of registry
    /// policy rejections keyed by invariant code (I1..I7 or free text).
    policy_rejections_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_policy_rejections_total",
            "Registry policy rejections, labelled by invariant code (I1..I7 or free text)."
        ),
        &["reason"],
    )
    .expect("valid counter opts")
);

// =====================================================================
// Rate limit (M10)
// =====================================================================

once_metric!(
    /// `efe_ratelimit_drops_total{tenant}` — counter of requests
    /// rejected by the server-side rate limiter.
    ratelimit_drops_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_ratelimit_drops_total",
            "Requests rejected by the token-bucket rate limiter."
        ),
        &["tenant"],
    )
    .expect("valid counter opts")
);

/// Touch every instrument so that each `OnceLock` is initialised and
/// the underlying collector is registered against the global registry.
///
/// Safe to call multiple times. Audit finding M11-#4: we used to warm
/// each `*Vec` with a sentinel `"__init__"` label combination so that
/// `Registry::gather()` would emit the family even before any traffic.
/// That polluted dashboards (`{reason="__init__"}` looked like a real
/// policy rejection). We now register descriptors only — empty
/// families simply won't appear in `/metrics` output until first
/// traffic, which is the idiomatic Prometheus behaviour.
pub fn register_all() {
    let _ = grpc_requests_total();
    let _ = grpc_request_duration_seconds();
    let _ = grpc_requests_in_flight();
    let _ = swarm_rounds_total();
    let _ = swarm_duration_seconds();
    let _ = swarm_confidence();
    let _ = errors_total();
    let _ = breaches_total();
    let _ = breach_magnitude();
    let _ = cone_registrations_total();
    let _ = policy_rejections_total();
    let _ = ratelimit_drops_total();
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::proto::MetricFamily;

    fn family_names() -> Vec<String> {
        global_registry()
            .gather()
            .into_iter()
            .map(|mf: MetricFamily| mf.get_name().to_string())
            .collect()
    }

    // After the audit #4 fix, scalar families always appear in
    // `gather()`; `*Vec` families only appear after at least one sample.
    // We therefore assert only the scalars here and cover vec families
    // through the per-metric tests that touch their labels.
    #[test]
    fn register_all_exposes_scalar_instruments_unconditionally() {
        register_all();
        let names = family_names();
        for expected in ["efe_swarm_rounds_total", "efe_breach_magnitude"] {
            assert!(
                names.iter().any(|n| n == expected),
                "scalar metric {expected} missing from registry; got {names:?}"
            );
        }
    }

    // Regression for audit finding M11-#4: `register_all` must NOT
    // introduce phantom `__init__` series in `/metrics`. We verify by
    // encoding the registry and grepping the body.
    #[test]
    fn register_all_does_not_emit_init_sentinel_series() {
        use prometheus::Encoder;
        register_all();
        let mut buf = Vec::new();
        let encoder = prometheus::TextEncoder::new();
        encoder
            .encode(
                &super::super::registry::global_registry().gather(),
                &mut buf,
            )
            .unwrap();
        let body = String::from_utf8(buf).unwrap();
        assert!(
            !body.contains("__init__"),
            "`__init__` sentinel label leaked into /metrics output:\n{body}"
        );
    }

    #[test]
    fn grpc_requests_counter_increments() {
        let c = grpc_requests_total();
        let before = c.with_label_values(&["/test/Method", "OK", "acme"]).get();
        c.with_label_values(&["/test/Method", "OK", "acme"]).inc();
        let after = c.with_label_values(&["/test/Method", "OK", "acme"]).get();
        assert_eq!(after, before + 1);
    }

    #[test]
    fn histogram_observes_without_panicking() {
        let h = swarm_duration_seconds();
        h.with_label_values(&["converged", "acme"]).observe(0.1);
        h.with_label_values(&["mode_collapse", "acme"]).observe(0.5);
    }

    #[test]
    fn gauge_dec_after_inc_returns_to_zero() {
        let g = grpc_requests_in_flight();
        g.with_label_values(&["/p1"]).inc();
        g.with_label_values(&["/p1"]).dec();
        assert_eq!(g.with_label_values(&["/p1"]).get(), 0);
    }
}
