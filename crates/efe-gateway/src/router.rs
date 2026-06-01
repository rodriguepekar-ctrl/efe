// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : `RouterBuilder` — § 7.4.
//!
//! Assembles all routes + middleware in the correct order.

use axum::{
    routing::{get, post},
    Router,
};
use tower_http::{
    catch_panic::CatchPanicLayer, limit::RequestBodyLimitLayer, trace::TraceLayer,
};

use crate::handlers;
use crate::observability::{healthz_handler, metrics_handler, readyz_handler};
use crate::state::AppState;

/// **N22** : assembled router builder.
pub struct RouterBuilder {
    body_limit_bytes: usize,
    state: AppState,
}

impl RouterBuilder {
    pub fn new(state: AppState) -> Self {
        Self {
            body_limit_bytes: crate::DEFAULT_BODY_LIMIT_BYTES,
            state,
        }
    }

    pub fn with_body_limit(mut self, bytes: usize) -> Self {
        self.body_limit_bytes = bytes;
        self
    }

    /// **N22** : Build the `axum::Router`.
    pub fn build(self) -> Router {
        build_router(self.state, self.body_limit_bytes)
    }
}

/// **N22** : Convenience freestanding builder for tests + main.
pub fn build_router(state: AppState, body_limit_bytes: usize) -> Router {
    Router::new()
        // ─── Public routes (no auth) ─────────────────────────────
        .route("/healthz", get(healthz_handler))
        .route("/readyz", get(readyz_handler))
        .route("/metrics", get(metrics_handler))
        .route("/openapi.json", get(handlers::openapi_json_handler))
        // ─── Authenticated v1 routes ─────────────────────────────
        // Swarm (N21)
        .route("/v1/swarms/run", post(handlers::swarms_run_handler))
        // Agentic (N21)
        .route("/v1/agentic/call", post(handlers::agentic_call_handler))
        // **Sprint 19 — N27** : streaming variant.  Same auth +
        // tenant-scoping discipline ; response is SSE.
        .route(
            "/v1/agentic/call-stream",
            post(handlers::agentic_call_stream_handler),
        )
        // Nano catalogue (N23)
        .route(
            "/v1/nano-catalogue",
            get(handlers::nano_catalogue_handler),
        )
        // Publishers (N18)
        .route(
            "/v1/publishers",
            post(handlers::publishers::register_publisher_handler),
        )
        .route(
            "/v1/publishers/:publisher_id",
            get(handlers::publishers::lookup_publisher_handler),
        )
        .route(
            "/v1/publishers/:publisher_id/rotate-key",
            post(handlers::publishers::rotate_key_handler),
        )
        .route(
            "/v1/publishers/:publisher_id/revoke",
            post(handlers::publishers::revoke_publisher_handler),
        )
        .route(
            "/v1/publishers/:publisher_id/promote",
            post(handlers::publishers::promote_publisher_handler),
        )
        // Gold (N19)
        .route(
            "/v1/gold-sets",
            post(handlers::gold::register_gold_set_handler),
        )
        .route(
            "/v1/gold-sets/:gold_set_id/approve",
            post(handlers::gold::approve_gold_set_handler),
        )
        .route(
            "/v1/gold-runs",
            post(handlers::gold::run_gold_gate_handler),
        )
        .route(
            "/v1/gold-runs/:gold_run_id",
            get(handlers::gold::lookup_gold_run_handler),
        )
        // Admin (operator scope)
        .route(
            "/v1/admin/circuit-breakers",
            get(handlers::circuit_breakers_handler),
        )
        .route(
            "/v1/admin/drift-sweep/trigger",
            post(handlers::drift_sweep_trigger_handler),
        )
        // **N24 + G13** : compliance status snapshot.
        .route(
            "/v1/admin/compliance-status",
            get(handlers::compliance_status_handler),
        )
        // **N26 + G13** : LLM provenance read endpoint.
        .route(
            "/v1/llm-provenance",
            get(handlers::llm_provenance_handler),
        )
        .with_state(state)
        // ─── Middleware stack (outer→inner) ─────────────────────
        // CatchPanic wraps everything : any panic becomes 500.
        .layer(CatchPanicLayer::new())
        // Body limit BEFORE handlers (parsing fails fast).
        .layer(RequestBodyLimitLayer::new(body_limit_bytes))
        // Tracing : observe every request/response.
        .layer(TraceLayer::new_for_http())
}

#[cfg(test)]
mod tests {
    // Tests live in tests/phase_7_n22_*.rs.
}
