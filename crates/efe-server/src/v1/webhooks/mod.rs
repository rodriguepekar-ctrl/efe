//! M28 — Webhook subscription system.
//!
//! Five sub-modules :
//! - [`validation`] : URL validation (SSRF defense, scheme, private IP rejection).
//! - [`signing`] : HMAC-SHA256 signing + timestamp construction.
//! - [`delivery`] : per-attempt log + retry policy + circuit breaker.
//! - [`handlers`] : axum handlers for the public REST API.
//! - [`worker`] : background task that consumes NATS + delivers.

pub mod delivery;
pub mod handlers;
pub mod signing;
pub mod validation;
pub mod worker;

use axum::Router;

/// Build the `/v1/webhooks` axum sub-router.
pub fn router<S>(state: S) -> Router
where
    S: handlers::WebhookHandlerState,
{
    use axum::routing::{get, post};
    Router::new()
        .route(
            "/webhooks",
            post(handlers::create::<S>).get(handlers::list::<S>),
        )
        .route(
            "/webhooks/:id",
            get(handlers::get_one::<S>)
                .patch(handlers::update::<S>)
                .delete(handlers::delete_one::<S>),
        )
        .route("/webhooks/:id/replay", post(handlers::replay::<S>))
        .route(
            "/webhooks/:id/deliveries",
            get(handlers::list_deliveries::<S>),
        )
        .route("/webhooks/:id/verify", post(handlers::verify::<S>))
        .with_state(state)
}
