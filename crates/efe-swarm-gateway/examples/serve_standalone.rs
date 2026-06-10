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
        // Inc.4.b : table de routage provider→modèle (non-secrète). Surcharge la table par
        // défaut depuis l'env si fournie (`famille=modèle,famille=modèle`).
        if let Ok(models) = std::env::var("EFE_SWARM_ROUTER_MODELS") {
            state = state.with_router_models(efe_swarm_gateway::RouterMap::from_env_str(&models));
            println!("→ mapping provider→modèle surchargé depuis EFE_SWARM_ROUTER_MODELS.");
        }
    }

    // Prod (feature `postgres`) : persistance durable des records si DATABASE_URL est défini.
    #[cfg(feature = "postgres")]
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pg = efe_swarm_gateway::PgRunStore::connect(&url).await?;
        pg.migrate().await?;
        state = state.with_store(std::sync::Arc::new(pg));
        println!("→ PgRunStore câblé (persistance durable, write-once).");
    }

    // Prod (feature `oidc`) : auth JWT OIDC ES256 si EFE_OIDC_PUBLIC_KEY_PEM est défini
    // (clé PUBLIQUE server-side ; jamais une clé privée).
    #[cfg(feature = "oidc")]
    if let Some(verifier) = efe_swarm_gateway::OidcVerifier::from_env()? {
        state = state.with_oidc(std::sync::Arc::new(verifier));
        println!("→ auth OIDC activée (JWT ES256, clé publique server-side).");
    }

    // G9 (feature `pii`) : politique PII par défaut depuis l'env (off/detect/redact/strict).
    // Détection LOCALE (souverain). Off par défaut.
    #[cfg(feature = "pii")]
    if let Ok(p) = std::env::var("EFE_PII_DEFAULT_POLICY") {
        if let Ok(policy) = p.parse::<efe_swarm_gateway::PolicyKind>() {
            state = state.with_pii(efe_swarm_gateway::PiiConfig::new(policy));
            println!("→ PII opt-in : politique par défaut = {}", policy.as_str());
        }
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
