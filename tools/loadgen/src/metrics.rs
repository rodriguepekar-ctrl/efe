//! PR #20 § 14 — Prometheus self-instrumentation.
//!
//! 5 STABLE series committed in STABILITY_CONTRACT v1.2 § 9.10 :
//!
//! ```text
//! efe_msw_loadgen_request_total            Counter   verb, result, target
//! efe_msw_loadgen_request_duration_seconds Histogram verb, result
//! efe_msw_loadgen_inflight                 Gauge     verb
//! efe_msw_loadgen_target_up                Gauge     target
//! efe_msw_loadgen_build_info               Gauge     version, git_sha, rustc_version, build_date
//! ```
//!
//! Histogram buckets match the server-side `iterate_duration_seconds`
//! buckets (AD23) so operators can subtract loadgen-side vs server-side
//! latency directly.

use std::sync::Arc;

use prometheus::{
    register_histogram_vec_with_registry, register_int_counter_vec_with_registry,
    register_int_gauge_vec_with_registry, HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec,
    Registry,
};

use crate::error::LoadgenResult;
use crate::stats::{RequestResult, Verb};

/// Histogram buckets in seconds — matches PR #17 server side (AD23).
const LOADGEN_HISTOGRAM_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Bundle of registered loadgen self-metrics.
///
/// Cloned cheaply (it's `Arc`-shared internally) so every dispatch task
/// can update without contention.
#[derive(Clone)]
pub struct LoadgenMetrics {
    pub registry: Arc<Registry>,
    pub request_total: IntCounterVec,
    pub request_duration: HistogramVec,
    pub inflight: IntGaugeVec,
    pub target_up: IntGaugeVec,
    pub build_info: IntGaugeVec,
}

impl LoadgenMetrics {
    /// Register all 5 STABLE series into a fresh registry.
    pub fn register() -> LoadgenResult<Self> {
        let registry = Arc::new(Registry::new());
        let request_total = register_int_counter_vec_with_registry!(
            "efe_msw_loadgen_request_total",
            "Loadgen requests sent, partitioned by verb / result / target.",
            &["verb", "result", "target"],
            registry
        )
        .map_err(|e| crate::error::LoadgenError::Internal(format!("metrics register: {e}")))?;

        let request_duration = register_histogram_vec_with_registry!(
            HistogramOpts::new(
                "efe_msw_loadgen_request_duration_seconds",
                "Loadgen-observed wall-clock duration per request.",
            )
            .buckets(LOADGEN_HISTOGRAM_BUCKETS.to_vec()),
            &["verb", "result"],
            registry
        )
        .map_err(|e| crate::error::LoadgenError::Internal(format!("metrics register: {e}")))?;

        let inflight = register_int_gauge_vec_with_registry!(
            "efe_msw_loadgen_inflight",
            "Current in-flight loadgen requests per verb.",
            &["verb"],
            registry
        )
        .map_err(|e| crate::error::LoadgenError::Internal(format!("metrics register: {e}")))?;

        let target_up = register_int_gauge_vec_with_registry!(
            "efe_msw_loadgen_target_up",
            "Last response state per target (1 = ok, 0 = error/down).",
            &["target"],
            registry
        )
        .map_err(|e| crate::error::LoadgenError::Internal(format!("metrics register: {e}")))?;

        let build_info = register_int_gauge_vec_with_registry!(
            "efe_msw_loadgen_build_info",
            "Loadgen build metadata.  Always 1.",
            &["version", "git_sha", "rustc_version", "build_date"],
            registry
        )
        .map_err(|e| crate::error::LoadgenError::Internal(format!("metrics register: {e}")))?;

        // Set the build_info gauge once at startup (AD28).
        build_info
            .with_label_values(&[
                crate::build::VERSION,
                crate::build::GIT_SHA,
                crate::build::RUSTC_VERSION,
                crate::build::BUILD_DATE,
            ])
            .set(1);

        Ok(Self {
            registry,
            request_total,
            request_duration,
            inflight,
            target_up,
            build_info,
        })
    }

    /// Convenience : observe a finished request.  All 3 series are
    /// updated atomically (`inc` + `observe` + `target_up`).
    pub fn observe(&self, verb: Verb, result: RequestResult, target: &str, duration_secs: f64) {
        self.request_total
            .with_label_values(&[verb.as_str(), result.as_str(), target])
            .inc();
        self.request_duration
            .with_label_values(&[verb.as_str(), result.as_str()])
            .observe(duration_secs);
        let up = i64::from(result.is_ok());
        self.target_up.with_label_values(&[target]).set(up);
    }

    /// In-flight delta : positive when entering a request, negative on exit.
    pub fn inflight_delta(&self, verb: Verb, delta: i64) {
        self.inflight.with_label_values(&[verb.as_str()]).add(delta);
    }

    /// Render `/metrics` content in the Prometheus text format.
    pub fn render(&self) -> Vec<u8> {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        let _ = encoder.encode(&self.registry.gather(), &mut buf);
        buf
    }
}

/// Spawn an HTTP listener serving the metrics on the configured address.
/// Returns the [`tokio::task::JoinHandle`] for graceful shutdown.
pub async fn spawn_listener(
    metrics: LoadgenMetrics,
    addr: std::net::SocketAddr,
) -> LoadgenResult<tokio::task::JoinHandle<()>> {
    use std::net::TcpListener as StdTcpListener;
    use tokio::net::TcpListener;

    let std_listener = StdTcpListener::bind(addr).map_err(|e| {
        crate::error::LoadgenError::Internal(format!("prom listen {addr} : {e}"))
    })?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| crate::error::LoadgenError::Internal(format!("set_nonblocking : {e}")))?;
    let listener = TcpListener::from_std(std_listener)
        .map_err(|e| crate::error::LoadgenError::Internal(format!("from_std : {e}")))?;

    let handle = tokio::spawn(async move {
        // Minimal HTTP/1.1 metrics endpoint.  Accepts connections in a
        // tight loop ; each is handled by a one-shot tokio::spawn so a
        // slow client can't block subsequent scrapes.
        loop {
            match listener.accept().await {
                Ok((mut sock, _)) => {
                    let body = metrics.render();
                    tokio::spawn(async move {
                        use tokio::io::AsyncWriteExt;
                        // We don't parse the request line — Prometheus
                        // scrapes always use GET /metrics, and this is
                        // an internal-only endpoint.  Bare 200 + body.
                        let response = format!(
                            "HTTP/1.1 200 OK\r\n\
                             Content-Type: text/plain; version=0.0.4\r\n\
                             Content-Length: {}\r\n\
                             Connection: close\r\n\
                             \r\n",
                            body.len()
                        );
                        let _ = sock.write_all(response.as_bytes()).await;
                        let _ = sock.write_all(&body).await;
                        let _ = sock.shutdown().await;
                    });
                }
                // Brief back-off on accept errors so a runaway error
                // (FD exhaustion, etc.) doesn't spin-CPU.
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
            }
        }
    });
    Ok(handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_register_succeeds() {
        let m = LoadgenMetrics::register().unwrap();
        // build_info is set at register time and ALWAYS appears.
        let body = m.render();
        let s = String::from_utf8_lossy(&body);
        assert!(s.contains("efe_msw_loadgen_build_info"));
        // The counter is registered but emits no samples until first
        // observation ; touch it once to ensure the series appears.
        m.observe(Verb::Iterate, RequestResult::Ok, "test", 0.001);
        let body = m.render();
        let s = String::from_utf8_lossy(&body);
        assert!(s.contains("efe_msw_loadgen_request_total"));
    }

    #[test]
    fn observe_increments_counter_and_histogram() {
        let m = LoadgenMetrics::register().unwrap();
        m.observe(Verb::Iterate, RequestResult::Ok, "t1", 0.012);
        m.observe(Verb::Iterate, RequestResult::Ok, "t1", 0.020);
        let body = m.render();
        let s = String::from_utf8_lossy(&body);
        assert!(s.contains("efe_msw_loadgen_request_duration_seconds_count"));
        assert!(s.contains("efe_msw_loadgen_target_up"));
    }
}
