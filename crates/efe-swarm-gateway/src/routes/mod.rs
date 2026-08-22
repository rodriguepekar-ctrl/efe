// SPDX-License-Identifier: UNLICENSED
//! Les handlers HTTP, regroupés par route. `routes()` assemble le `Router` des routes
//! d'essaim (relatif, sans préfixe — montable via `.nest("/v1/swarm", …)`).

pub mod bundle; // Fil P (P.2) : /v1/swarm/bundle/:id — l'export re-prouvable par verify-mini
pub mod dashboard;
pub mod events;
pub mod gaps; // M2 : /v1/swarm/gaps — le journal des runs servis mais NON persistés
pub mod health; // Sondes k8s (M4 §G4) : /healthz (liveness) + /readyz (readiness réelle)
pub mod memory;
pub mod metrics;
pub mod records;
pub mod replay;
pub mod run;
pub mod verify;
#[cfg(feature = "frontier-llm")]
pub mod llm; // Couche 2 (M3) : /v1/llm/* — les 5 missions frontière servies + le checker d'arité

use axum::routing::{get, post};
use axum::Router;

use crate::state::SwarmGatewayState;

/// Le `Router` des routes d'essaim (**relatif** : `run`, `verify/:id`, `runs/:id`,
/// `runs/:id/records`, `runs/:id/events`, `events`). Montez-le sous `/v1/swarm` (`.nest`).
pub fn routes() -> Router<SwarmGatewayState> {
    Router::new()
        .route("/run", post(run::run))
        .route("/verify/:id", get(verify::verify))
        // M2 : le canal de l'AUDITEUR. Monté dans les deux postures — sous `blocking` il rend un
        // journal vide en le disant, ce qui vaut mieux qu'une route absente qu'on interpréterait.
        .route("/gaps", get(gaps::gaps))
        // ADR-017 : rejeu déterministe — R0 (GET, re-sceller) + R1 (POST, re-exécuter).
        .route("/replay/:id", get(replay::get_replay).post(replay::post_replay))
        .route("/runs/:id", get(records::get_run))
        .route("/runs/:id/records", get(records::get_records))
        .route("/runs/:id/n17", get(records::get_n17)) // ADR-016 : export forensique offline
        .route("/runs/:id/events", get(events::run_events)) // SSE : rejeu + live (Inc.3)
        .route("/events", get(events::tenant_events)) // SSE : firehose tenant (Inc.3)
        // ADR-019 (MEM-3) : mémoire inter-runs — instantané + re-preuve du memory_root.
        .route("/memory/:family", get(memory::get_memory))
        .route("/memory/:family/verify", get(memory::verify_memory_chain))
}
