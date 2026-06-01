// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : Prometheus surface — § 7.5.
//!
//! 12 surfaces in the `n22_*` namespace.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use dashmap::DashMap;

use crate::state::AppState;

/// **Sprint 14 — N22** : the 12 Prometheus surfaces.
pub struct N22GatewayMetrics {
    // (1) http_requests_total{method, path, status}
    requests: DashMap<(String, String, u16), AtomicU64>,
    // (2) http_request_duration_ms{method, path}
    duration_sum_ms: DashMap<(String, String), AtomicU64>,
    duration_count: DashMap<(String, String), AtomicU64>,
    // (3) http_in_flight_requests (gauge)
    in_flight: AtomicU64,
    // (4) http_request_body_bytes_total{method, path}
    req_body_bytes: DashMap<(String, String), AtomicU64>,
    // (5) http_response_body_bytes_total{method, path}
    resp_body_bytes: DashMap<(String, String), AtomicU64>,
    // (6) auth_failures_total{reason}
    auth_failures: DashMap<String, AtomicU64>,
    // (7) rate_limit_rejections_total{tenant_tier}
    rate_rejections: DashMap<String, AtomicU64>,
    // (8) idempotency_cache_hits_total{outcome}
    idempotency: DashMap<String, AtomicU64>,
    // (9) tls_handshake_failures_total{reason}
    tls_failures: DashMap<String, AtomicU64>,
    // (10) graceful_shutdowns_total{outcome}
    shutdowns: DashMap<String, AtomicU64>,
    // (11) panic_recoveries_total{handler}
    panics: DashMap<String, AtomicU64>,
    // (12) concurrent_connections (gauge)
    concurrent_conn: AtomicU64,
}

impl std::fmt::Debug for N22GatewayMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("N22GatewayMetrics")
            .field("in_flight", &self.in_flight.load(Ordering::Relaxed))
            .field("concurrent_conn", &self.concurrent_conn.load(Ordering::Relaxed))
            .finish()
    }
}

impl Default for N22GatewayMetrics {
    fn default() -> Self {
        Self::zeroed()
    }
}

impl N22GatewayMetrics {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::zeroed())
    }

    fn zeroed() -> Self {
        Self {
            requests: DashMap::new(),
            duration_sum_ms: DashMap::new(),
            duration_count: DashMap::new(),
            in_flight: AtomicU64::new(0),
            req_body_bytes: DashMap::new(),
            resp_body_bytes: DashMap::new(),
            auth_failures: DashMap::new(),
            rate_rejections: DashMap::new(),
            idempotency: DashMap::new(),
            tls_failures: DashMap::new(),
            shutdowns: DashMap::new(),
            panics: DashMap::new(),
            concurrent_conn: AtomicU64::new(0),
        }
    }

    // ─── Tick helpers ─────────────────────────────────────────────

    pub fn tick_request(&self, method: &str, path: &str, status: u16) {
        let key = (method.to_string(), path.to_string(), status);
        self.requests
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn observe_duration_ms(&self, method: &str, path: &str, ms: u64) {
        let key = (method.to_string(), path.to_string());
        self.duration_sum_ms
            .entry(key.clone())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(ms, Ordering::Relaxed);
        self.duration_count
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_in_flight(&self) {
        self.in_flight.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dec_in_flight(&self) {
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn in_flight_now(&self) -> u64 {
        self.in_flight.load(Ordering::Relaxed)
    }

    pub fn add_req_body(&self, method: &str, path: &str, bytes: u64) {
        self.req_body_bytes
            .entry((method.into(), path.into()))
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_resp_body(&self, method: &str, path: &str, bytes: u64) {
        self.resp_body_bytes
            .entry((method.into(), path.into()))
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn tick_auth_failure(&self, reason: &str) {
        self.auth_failures
            .entry(reason.into())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn tick_rate_rejection(&self, tier: &str) {
        self.rate_rejections
            .entry(tier.into())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn tick_idempotency(&self, outcome: &str) {
        self.idempotency
            .entry(outcome.into())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn tick_tls_failure(&self, reason: &str) {
        self.tls_failures
            .entry(reason.into())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn tick_shutdown(&self, outcome: &str) {
        self.shutdowns
            .entry(outcome.into())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn tick_panic(&self, handler: &str) {
        self.panics
            .entry(handler.into())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_concurrent_conn(&self, n: u64) {
        self.concurrent_conn.store(n, Ordering::Relaxed);
    }

    // ─── Read helpers (for tests) ──────────────────────────────────

    pub fn requests_total(&self, method: &str, path: &str, status: u16) -> u64 {
        self.requests
            .get(&(method.into(), path.into(), status))
            .map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    pub fn auth_failures_total(&self, reason: &str) -> u64 {
        self.auth_failures
            .get(reason)
            .map_or(0, |r| r.value().load(Ordering::Relaxed))
    }

    // ─── Render ────────────────────────────────────────────────────

    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4_096);

        out.push_str("# HELP n22_http_requests_total HTTP requests.\n");
        out.push_str("# TYPE n22_http_requests_total counter\n");
        for r in self.requests.iter() {
            let (method, path, status) = r.key();
            out.push_str(&format!(
                "n22_http_requests_total{{method=\"{method}\",path=\"{path}\",status=\"{status}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_http_request_duration_ms HTTP duration.\n");
        out.push_str("# TYPE n22_http_request_duration_ms summary\n");
        for r in self.duration_sum_ms.iter() {
            let (method, path) = r.key();
            let sum = r.value().load(Ordering::Relaxed);
            let count = self
                .duration_count
                .get(&(method.clone(), path.clone()))
                .map_or(0, |c| c.value().load(Ordering::Relaxed));
            out.push_str(&format!(
                "n22_http_request_duration_ms_sum{{method=\"{method}\",path=\"{path}\"}} {sum}\n",
            ));
            out.push_str(&format!(
                "n22_http_request_duration_ms_count{{method=\"{method}\",path=\"{path}\"}} {count}\n",
            ));
        }
        out.push_str("# HELP n22_http_in_flight_requests In-flight requests gauge.\n");
        out.push_str("# TYPE n22_http_in_flight_requests gauge\n");
        out.push_str(&format!(
            "n22_http_in_flight_requests {}\n",
            self.in_flight.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP n22_http_request_body_bytes_total Request body bytes.\n");
        out.push_str("# TYPE n22_http_request_body_bytes_total counter\n");
        for r in self.req_body_bytes.iter() {
            let (method, path) = r.key();
            out.push_str(&format!(
                "n22_http_request_body_bytes_total{{method=\"{method}\",path=\"{path}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_http_response_body_bytes_total Response body bytes.\n");
        out.push_str("# TYPE n22_http_response_body_bytes_total counter\n");
        for r in self.resp_body_bytes.iter() {
            let (method, path) = r.key();
            out.push_str(&format!(
                "n22_http_response_body_bytes_total{{method=\"{method}\",path=\"{path}\"}} {}\n",
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_auth_failures_total Auth failures.\n");
        out.push_str("# TYPE n22_auth_failures_total counter\n");
        for r in self.auth_failures.iter() {
            out.push_str(&format!(
                "n22_auth_failures_total{{reason=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_rate_limit_rejections_total Rate limit rejections.\n");
        out.push_str("# TYPE n22_rate_limit_rejections_total counter\n");
        for r in self.rate_rejections.iter() {
            out.push_str(&format!(
                "n22_rate_limit_rejections_total{{tenant_tier=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_idempotency_cache_hits_total Idempotency outcomes.\n");
        out.push_str("# TYPE n22_idempotency_cache_hits_total counter\n");
        for r in self.idempotency.iter() {
            out.push_str(&format!(
                "n22_idempotency_cache_hits_total{{outcome=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_tls_handshake_failures_total TLS handshake failures.\n");
        out.push_str("# TYPE n22_tls_handshake_failures_total counter\n");
        for r in self.tls_failures.iter() {
            out.push_str(&format!(
                "n22_tls_handshake_failures_total{{reason=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_graceful_shutdowns_total Shutdowns.\n");
        out.push_str("# TYPE n22_graceful_shutdowns_total counter\n");
        for r in self.shutdowns.iter() {
            out.push_str(&format!(
                "n22_graceful_shutdowns_total{{outcome=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_panic_recoveries_total Handler panics recovered.\n");
        out.push_str("# TYPE n22_panic_recoveries_total counter\n");
        for r in self.panics.iter() {
            out.push_str(&format!(
                "n22_panic_recoveries_total{{handler=\"{}\"}} {}\n",
                r.key(),
                r.value().load(Ordering::Relaxed),
            ));
        }
        out.push_str("# HELP n22_concurrent_connections Active TCP connections.\n");
        out.push_str("# TYPE n22_concurrent_connections gauge\n");
        out.push_str(&format!(
            "n22_concurrent_connections {}\n",
            self.concurrent_conn.load(Ordering::Relaxed)
        ));
        out
    }
}

/// **N22** : axum handler for `/metrics`.
pub async fn metrics_handler(State(state): State<AppState>) -> Response {
    let body = state.metrics.render_prometheus();
    let mut resp = (StatusCode::OK, body).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_metrics_zero() {
        let m = N22GatewayMetrics::new();
        assert_eq!(m.in_flight_now(), 0);
        assert_eq!(m.requests_total("GET", "/x", 200), 0);
    }

    #[test]
    fn tick_request_increments() {
        let m = N22GatewayMetrics::new();
        m.tick_request("GET", "/v1/swarms", 200);
        m.tick_request("GET", "/v1/swarms", 200);
        assert_eq!(m.requests_total("GET", "/v1/swarms", 200), 2);
    }

    #[test]
    fn in_flight_increments_and_decrements() {
        let m = N22GatewayMetrics::new();
        m.inc_in_flight();
        m.inc_in_flight();
        m.inc_in_flight();
        assert_eq!(m.in_flight_now(), 3);
        m.dec_in_flight();
        assert_eq!(m.in_flight_now(), 2);
    }

    #[test]
    fn render_contains_all_12_surfaces() {
        let m = N22GatewayMetrics::new();
        m.tick_request("GET", "/x", 200);
        m.observe_duration_ms("GET", "/x", 50);
        m.inc_in_flight();
        m.add_req_body("POST", "/x", 100);
        m.add_resp_body("POST", "/x", 200);
        m.tick_auth_failure("missing");
        m.tick_rate_rejection("tier_1");
        m.tick_idempotency("hit");
        m.tick_tls_failure("bad_cert");
        m.tick_shutdown("ok");
        m.tick_panic("h");
        m.set_concurrent_conn(7);
        let out = m.render_prometheus();
        for name in [
            "n22_http_requests_total",
            "n22_http_request_duration_ms",
            "n22_http_in_flight_requests",
            "n22_http_request_body_bytes_total",
            "n22_http_response_body_bytes_total",
            "n22_auth_failures_total",
            "n22_rate_limit_rejections_total",
            "n22_idempotency_cache_hits_total",
            "n22_tls_handshake_failures_total",
            "n22_graceful_shutdowns_total",
            "n22_panic_recoveries_total",
            "n22_concurrent_connections",
        ] {
            assert!(out.contains(name), "missing : {name}");
        }
    }

    #[test]
    fn auth_failures_per_reason() {
        let m = N22GatewayMetrics::new();
        m.tick_auth_failure("missing");
        m.tick_auth_failure("expired");
        m.tick_auth_failure("missing");
        assert_eq!(m.auth_failures_total("missing"), 2);
        assert_eq!(m.auth_failures_total("expired"), 1);
    }

    #[test]
    fn debug_format_no_panic() {
        let m = N22GatewayMetrics::new();
        let _ = format!("{m:?}");
    }
}
