// SPDX-License-Identifier: UNLICENSED
//! Les handlers HTTP, regroupés par route. `routes()` assemble le `Router` des routes
//! d'essaim (relatif, sans préfixe — montable via `.nest("/v1/swarm", …)`).

pub mod dashboard;
pub mod events;
pub mod metrics;
pub mod records;
pub mod run;
pub mod verify;

use axum::routing::{get, post};
use axum::Router;

use crate::state::SwarmGatewayState;

/// Le `Router` des routes d'essaim (**relatif** : `run`, `verify/:id`, `runs/:id`,
/// `runs/:id/records`, `runs/:id/events`, `events`). Montez-le sous `/v1/swarm` (`.nest`).
pub fn routes() -> Router<SwarmGatewayState> {
    Router::new()
        .route("/run", post(run::run))
        .route("/verify/:id", get(verify::verify))
        .route("/runs/:id", get(records::get_run))
        .route("/runs/:id/records", get(records::get_records))
        .route("/runs/:id/n17", get(records::get_n17)) // ADR-016 : export forensique offline
        .route("/runs/:id/events", get(events::run_events)) // SSE : rejeu + live (Inc.3)
        .route("/events", get(events::tenant_events)) // SSE : firehose tenant (Inc.3)
}
