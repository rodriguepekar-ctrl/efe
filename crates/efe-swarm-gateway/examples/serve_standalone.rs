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

    // Inc.4 : le binaire (server-side) lit l'URL du routeur frontière depuis l'env — une URL
    // NON-SECRÈTE ; les clés provider vivent DANS le routeur (efe-llm-router, env/Vault).
    // Absent ⇒ `ClientMode::Network` renvoie 501 (déploiement souverain par défaut).
    let mut state = SwarmGatewayState::in_memory();
    if let Ok(router_ep) = std::env::var("EFE_SWARM_ROUTER_ENDPOINT") {
        state = state.with_router_endpoint(router_ep);
        println!("→ routeur frontière câblé (Network activé) — clés détenues server-side par le routeur.");
    }
    let app = router(state);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("efe-swarm-gateway en écoute sur http://{addr}");
    println!("  POST   http://{addr}/v1/swarm/run                  (sync ; ?stream=true = job live)");
    println!("  GET    http://{addr}/v1/swarm/verify/:id           (le moat : re-preuve)");
    println!("  GET    http://{addr}/v1/swarm/runs/:id/records      (export hors-ligne)");
    println!("  GET    http://{addr}/v1/swarm/runs/:id/events       (SSE : dashboard live)");
    println!("  GET    http://{addr}/dashboard                     (UI live 4 panneaux)");
    println!("  GET    http://{addr}/metrics");
    println!();
    println!("→ ouvre http://{addr}/dashboard puis clique « Lancer un run démo ».");

    axum::serve(listener, app).await?;
    Ok(())
}
