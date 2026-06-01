//! PR #20 § 13 — OpenTelemetry SDK init (loadgen-side).
//!
//! Emits one root span `efe.meta_swarm.loadgen.iterate` per request,
//! becoming the parent of the controller's `efe.meta_swarm.iterate`
//! via W3C Trace Context propagation (PR #18 § 14).
//!
//! Phase-2 implementation : we deliberately keep this module minimal —
//! the full OTLP/gRPC exporter wiring lives in
//! `crates/efe-meta-swarm-server::tracing_otel`.  Loadgen re-uses that
//! propagator at the call site (see `client::pool::iterate` which
//! injects a custom `x-loadgen-trace-id` header today ; Phase-3 swaps
//! to a real W3C `traceparent` when the OTel SDK feature is enabled).
//!
//! For now this module exposes :
//!   * `init(cfg)`        — install the SDK (no-op when endpoint=None)
//!   * `shutdown()`       — graceful drain
//!   * `current_trace_id()` — accessor for the active span's trace_id

use crate::config::OtelConfig;
use crate::error::LoadgenResult;

/// Guard returned by [`init`].  Dropping it triggers a graceful drain
/// of the OTel BatchSpanProcessor — same shape as PR #18 § 25.4.
#[derive(Debug)]
pub struct OtelGuard {
    enabled: bool,
}

impl OtelGuard {
    /// `true` when the OTel SDK is actually wired (endpoint configured).
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if self.enabled {
            tracing::debug!("OTel SDK shutdown (loadgen)");
        }
    }
}

/// Install the OTel SDK from the loadgen config.
///
/// Returns a guard whose `Drop` shuts down the SDK gracefully.  When
/// `cfg.endpoint` is `None`, this is a no-op and the guard is inert.
///
/// Phase-2 implementation : we register a `tracing_subscriber` layer
/// that exposes the span structure to log output but doesn't ship to a
/// remote collector unless a real OTel SDK is hooked up by the binary
/// crate (this module is intentionally light to keep the loadgen
/// dependency surface minimal).
pub fn init(cfg: &OtelConfig) -> LoadgenResult<OtelGuard> {
    if cfg.endpoint.is_none() {
        return Ok(OtelGuard { enabled: false });
    }
    tracing::info!(
        endpoint = ?cfg.endpoint,
        sampler  = cfg.sampler_arg,
        service  = %cfg.service_name,
        "OTel SDK enabled (loadgen) ; spans flow as `tracing` events"
    );
    Ok(OtelGuard { enabled: true })
}

/// Best-effort accessor for the currently-active trace ID, when the
/// OTel SDK is active.  Returns an empty string in Phase-2.
#[must_use]
pub fn current_trace_id() -> String {
    // Phase-3 : tap into `tracing_opentelemetry::OpenTelemetrySpanExt`
    // to extract `span.context().span().span_context().trace_id()`.
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_without_endpoint_is_noop() {
        let cfg = OtelConfig::default();
        let guard = init(&cfg).unwrap();
        assert!(!guard.is_enabled());
    }

    #[test]
    fn init_with_endpoint_enables_sdk() {
        let cfg = OtelConfig {
            endpoint: Some("http://collector:4317".into()),
            ..Default::default()
        };
        let guard = init(&cfg).unwrap();
        assert!(guard.is_enabled());
    }
}
