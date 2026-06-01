//! PR #18 § 14.2 + § 14.3 + § 24 — tonic Server + Client
//! interceptors implementing W3C Trace Context propagation.
//!
//! ## Server interceptor
//!
//! Runs on every inbound gRPC call.  Extracts `traceparent` from
//! the request metadata, builds an OTel parent context, and sets
//! it on the **current `tracing` span** via
//! `OpenTelemetrySpanExt::set_parent`.  Subsequent
//! `tracing::info_span!` invocations in the handler inherit the
//! propagated parent.
//!
//! ## Client interceptor
//!
//! Runs on every outbound gRPC call.  Reads the current OTel context
//! (typically the controller's `efe.meta_swarm.iterate` span) and
//! injects `traceparent` into the outgoing metadata.

use opentelemetry_sdk::propagation::TraceContextPropagator;
use tonic::service::Interceptor;
use tonic::{Request, Status};
use tracing::Span;

use super::propagator::{extract_parent_context, inject_current_context, OpenTelemetrySpanExt};

// ─────────────────────────────────────────────────────────────────────
// Server interceptor.
// ─────────────────────────────────────────────────────────────────────

/// Server-side interceptor : extract `traceparent` from request
/// metadata and bind it to the current tracing span as parent.
///
/// Wire-up : pass to `MetaSwarmServiceServer::with_interceptor(svc,
/// ServerTraceInterceptor::new())` in the binary's gRPC server
/// builder.
#[derive(Debug, Clone)]
pub struct ServerTraceInterceptor {
    propagator: TraceContextPropagator,
}

impl Default for ServerTraceInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerTraceInterceptor {
    /// Construct a fresh interceptor with the canonical W3C
    /// propagator.
    #[must_use]
    pub fn new() -> Self {
        Self {
            propagator: TraceContextPropagator::new(),
        }
    }
}

impl Interceptor for ServerTraceInterceptor {
    fn call(&mut self, request: Request<()>) -> Result<Request<()>, Status> {
        let parent_cx = extract_parent_context(&self.propagator, request.metadata());
        // Bind the extracted context to the current tracing span so
        // downstream `info_span!` calls in the handler inherit it.
        Span::current().set_parent(parent_cx);
        Ok(request)
    }
}

// ─────────────────────────────────────────────────────────────────────
// Client interceptor.
// ─────────────────────────────────────────────────────────────────────

/// Client-side interceptor : inject the current OTel context into the
/// outgoing request metadata as `traceparent` / `tracestate`.
///
/// Wire-up : `MetaParticipantServiceClient::with_interceptor(channel,
/// ClientTraceInterceptor::new())` in the controller's outbound
/// channel construction.
#[derive(Debug, Clone)]
pub struct ClientTraceInterceptor {
    propagator: TraceContextPropagator,
}

impl Default for ClientTraceInterceptor {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientTraceInterceptor {
    /// Construct a fresh interceptor.
    #[must_use]
    pub fn new() -> Self {
        Self {
            propagator: TraceContextPropagator::new(),
        }
    }
}

impl Interceptor for ClientTraceInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        inject_current_context(&self.propagator, request.metadata_mut());
        Ok(request)
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::metadata::MetadataValue;

    #[test]
    fn server_interceptor_passes_request_through() {
        let mut interceptor = ServerTraceInterceptor::new();
        let req = Request::new(());
        let out = interceptor.call(req).expect("ok");
        // The interceptor doesn't modify the body ; just extracts
        // the context.  Request type stays Request<()>.
        let _: &Request<()> = &out;
    }

    #[test]
    fn server_interceptor_passes_through_when_no_traceparent() {
        let mut interceptor = ServerTraceInterceptor::new();
        let req = Request::new(());
        // No traceparent in metadata ; should still return Ok.
        assert!(interceptor.call(req).is_ok());
    }

    #[test]
    fn server_interceptor_accepts_traceparent_header() {
        let mut interceptor = ServerTraceInterceptor::new();
        let mut req = Request::new(());
        let tp: MetadataValue<_> = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01"
            .parse()
            .unwrap();
        req.metadata_mut().insert("traceparent", tp);
        // Should not panic.
        assert!(interceptor.call(req).is_ok());
    }

    #[test]
    fn client_interceptor_passes_request_through() {
        let mut interceptor = ClientTraceInterceptor::new();
        let req = Request::new(());
        assert!(interceptor.call(req).is_ok());
    }

    #[test]
    fn server_and_client_interceptors_are_clone() {
        // Compile-time check : both interceptors must be Clone +
        // Send + Sync for use in tonic's interceptor builder.
        let s = ServerTraceInterceptor::new();
        let _s2 = s.clone();
        let c = ClientTraceInterceptor::new();
        let _c2 = c.clone();
    }
}
