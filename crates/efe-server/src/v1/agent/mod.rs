//! M26 — REST gateway for the agent runtime under `/v1/agent/*`.
//!
//! Mounts as an axum sub-router on the efe-server HTTP listener. Every
//! handler accepts `AuthCtx` (extracted by the auth middleware) ; every
//! handler dispatches in-process to the `AgentRuntimeService` implemented
//! in `efe-agent-runner` and re-exported here behind a thin façade.
//!
//! See `docs/EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` § 7.

pub mod adapter; // D93 — bridge JSON ↔ typed AgentRuntimeService
pub mod auth;
pub mod errors;

pub mod approvals;
pub mod cost;
pub mod dpia;
pub mod graduate;
pub mod post_market;
pub mod runs;
pub mod templates;

pub use adapter::AgentRuntimeServiceAdapter;

use axum::{
    routing::{delete, get, post},
    Router,
};
use std::sync::Arc;

pub use auth::AuthCtx;
pub use errors::AgentApiError;

/// D93 — convenience constructor : takes the M25 in-process service +
/// a PgPool, returns a wired-up `AgentApiState` ready to feed `router()`.
pub fn build_agent_state(
    service: std::sync::Arc<efe_agent_runner::service::AgentRuntimeService>,
    pg: sqlx::PgPool,
) -> AgentApiState {
    AgentApiState {
        service: std::sync::Arc::new(AgentRuntimeServiceAdapter::new(service)),
        pg,
    }
}

/// D93 — convenience alias for `router()` that an operator binary
/// integrator can use without thinking about the trait surface.
///
/// Example :
///
/// ```ignore
/// let state = build_agent_state(agent_service.clone(), pg.clone());
/// let app = build_agent_router(state);
/// let listener = tokio::net::TcpListener::bind("0.0.0.0:8081").await?;
/// axum::serve(listener, app).await?;
/// ```
///
/// The REST gateway listens on a separate port from the tonic gRPC
/// server (gRPC : `addr` from `Runtime::serve` ; REST : configurable
/// via `EFE_AGENT_REST_ADDR`). Run both concurrently with
/// `tokio::join!`.
pub fn build_agent_router(state: AgentApiState) -> axum::Router {
    router(state)
}

/// State threaded through every handler. Holds the agent runtime
/// service handle + a reference to the global Postgres pool (used by
/// readout endpoints that don't need to round-trip through the gRPC
/// service).
#[derive(Clone)]
pub struct AgentApiState {
    pub service: Arc<dyn AgentRuntimeFacade>,
    pub pg: sqlx::PgPool,
}

/// Trait that abstracts the M25 `AgentRuntimeService` for testability.
/// Implemented by the real service in `efe-agent-runner` ; mock-able in
/// tests under `tests/`.
#[async_trait::async_trait]
pub trait AgentRuntimeFacade: Send + Sync {
    async fn create_run(
        &self,
        caller: &auth::AuthCtx,
        req: serde_json::Value,
    ) -> Result<serde_json::Value, AgentApiError>;

    async fn get_run(
        &self,
        caller: &auth::AuthCtx,
        run_id: uuid::Uuid,
    ) -> Result<serde_json::Value, AgentApiError>;

    async fn list_runs(
        &self,
        caller: &auth::AuthCtx,
        include_completed: bool,
        limit: i32,
    ) -> Result<serde_json::Value, AgentApiError>;

    async fn pause_run(
        &self,
        caller: &auth::AuthCtx,
        run_id: uuid::Uuid,
        reason: Option<String>,
    ) -> Result<(), AgentApiError>;

    async fn resume_run(
        &self,
        caller: &auth::AuthCtx,
        run_id: uuid::Uuid,
    ) -> Result<(), AgentApiError>;

    async fn terminate_run(
        &self,
        caller: &auth::AuthCtx,
        run_id: uuid::Uuid,
        reason: String,
    ) -> Result<(), AgentApiError>;

    async fn approve_step(
        &self,
        caller: &auth::AuthCtx,
        approval_id: uuid::Uuid,
        reason: String,
    ) -> Result<serde_json::Value, AgentApiError>;

    async fn reject_step(
        &self,
        caller: &auth::AuthCtx,
        approval_id: uuid::Uuid,
        reason: String,
    ) -> Result<(), AgentApiError>;
}

/// Build the axum Router for `/v1/agent/*`.
pub fn router(state: AgentApiState) -> Router {
    Router::new()
        // ── Run lifecycle ────────────────────────────────────────────
        .route("/runs", post(runs::create).get(runs::list))
        .route("/runs/:id", get(runs::get_one))
        .route("/runs/:id/pause", post(runs::pause))
        .route("/runs/:id/resume", post(runs::resume))
        .route("/runs/:id/terminate", post(runs::terminate))
        .route("/runs/:id/steps", get(runs::list_steps))
        .route("/runs/:id/steps/stream", get(runs::stream_steps))
        // ── Approvals ────────────────────────────────────────────────
        .route("/approvals/approve", post(approvals::approve))
        .route("/approvals/reject", post(approvals::reject))
        .route("/approvals/pending", get(approvals::list_pending))
        // ── Templates ────────────────────────────────────────────────
        .route("/templates", post(templates::upsert).get(templates::list))
        .route("/templates/:tenant/:slug", get(templates::get_one).delete(templates::deprecate))
        // ── Cost ─────────────────────────────────────────────────────
        .route("/cost/daily", get(cost::daily))
        .route("/cost/run/:id", get(cost::per_run))
        // ── DPIA ─────────────────────────────────────────────────────
        .route("/dpia/sign", post(dpia::sign))
        .route("/dpia/status", get(dpia::status))
        .route("/dpia/list", get(dpia::list))
        .route("/dpia/refresh", post(dpia::refresh))
        // ── Graduation ───────────────────────────────────────────────
        .route("/graduate/check", post(graduate::check))
        .route("/graduate/confirm", post(graduate::confirm))
        .route("/graduate/revoke", post(graduate::revoke))
        // ── Post-market ──────────────────────────────────────────────
        .route("/post-market/quarterly", get(post_market::quarterly))
        .with_state(state)
}
