// SPDX-License-Identifier: UNLICENSED
//! Sert la passerelle d'essaim en **standalone souverain** (keyless, store en mémoire).
//!
//! ```bash
//! cargo run -p efe-swarm-gateway --example serve_standalone
//! # puis, dans un autre terminal, voir examples/curl_e2e.sh
//! ```
//!
//! **Aucune clé** : keyless (Mode A déterministe) + clé de provenance de test (dev). En
//! prod, on injecte `HmacKeySource::EnvHkdf` (server-side) et un `PgRunStore`.

use efe_swarm_gateway::{router, SwarmGatewayState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::var("EFE_SWARM_GATEWAY_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let state = SwarmGatewayState::in_memory();
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("efe-swarm-gateway en écoute sur http://{addr}");
    println!("  POST   http://{addr}/v1/swarm/run");
    println!("  GET    http://{addr}/v1/swarm/verify/:id   (le moat : re-preuve)");
    println!("  GET    http://{addr}/v1/swarm/runs/:id/records   (export hors-ligne)");
    println!("  GET    http://{addr}/metrics");

    axum::serve(listener, app).await?;
    Ok(())
}
