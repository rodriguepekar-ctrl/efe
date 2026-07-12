// SPDX-License-Identifier: UNLICENSED
//! **Binaire production** de la passerelle d'essaim souveraine (`efe-swarm-gateway`).
//!
//! ```bash
//! # prod (signe la provenance avec le master server-side) :
//! EFE_NANO_AUDIT_HMAC_MASTER=… DATABASE_URL=… cargo run -p efe-swarm-gateway \
//!     --features "postgres oidc pii" --bin efe-swarm-gateway
//! ```
//!
//! Différences vs l'exemple `serve_standalone` (dev, en mémoire, `test_key`) :
//! - **Clé de provenance** : sélectionne [`HmacKeySource::EnvHkdf`] (master HKDF lu
//!   **server-side** via `EFE_NANO_AUDIT_HMAC_MASTER`) dès qu'il est **présent** ; sinon repli
//!   `test_key()` **dev** avec un AVERTISSEMENT bruyant — car le master de test est **public**
//!   (dans la source), donc la re-preuve (`verify`, le *moat*) serait **forgeable**. C'est le
//!   trou que ce binaire ferme : le chart Helm monte un Secret HKDF que l'exemple n'utilisait pas.
//! - **Bind** par défaut `0.0.0.0:8080` (dans le pod) au lieu de `127.0.0.1`.
//! - **Arrêt gracieux** (SIGTERM k8s / Ctrl-C) → rollouts propres, connexions drainées.
//! - Câble store (`postgres`) / auth (`oidc`) / PII (`pii`) / routeur Network **depuis l'env**.
//!
//! **Aucune clé manipulée ici** : on teste seulement la **présence** du master (jamais sa
//! valeur, jamais loggée) pour choisir la source ; `EnvHkdf::derive` lit l'env **au moment de
//! signer**, server-side. Le routeur Network détient les clés provider (jamais ce binaire).

use efe_swarm_gateway::{router, SwarmGatewayState};
use efe_swarm_provenance::{HmacKeySource, MASTER_ENV};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::var("EFE_SWARM_GATEWAY_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    // ── Source de clé de provenance — le cœur du durcissement prod ──────────────
    let master_present = std::env::var(MASTER_ENV).map(|v| !v.trim().is_empty()).unwrap_or(false);
    let (keys, key_label) = key_source_for(master_present);
    if !master_present {
        eprintln!(
            "AVERTISSEMENT: {MASTER_ENV} absent ou vide → repli sur la clé de TEST PUBLIQUE. \
             La provenance serait FORGEABLE (verify non fiable). NE JAMAIS utiliser en production."
        );
    }

    let mut state = SwarmGatewayState::in_memory().with_keys(keys);
    println!("→ clé de provenance : {key_label}");

    // ── Routeur frontière (Network) + mapping provider→modèle (Inc.4 / Inc.4.b) ──
    if let Ok(router_ep) = std::env::var("EFE_SWARM_ROUTER_ENDPOINT") {
        state = state.with_router_endpoint(router_ep);
        println!("→ routeur frontière câblé (Network activé) — clés provider détenues server-side.");
        if let Ok(models) = std::env::var("EFE_SWARM_ROUTER_MODELS") {
            state = state.with_router_models(efe_swarm_gateway::RouterMap::from_env_str(&models));
            println!("→ mapping provider→modèle surchargé (EFE_SWARM_ROUTER_MODELS).");
        }
    }

    // ── Couche 2 (feature `frontier-llm`) : coordination multi-LLM (Claude/Gemini/GPT) ──
    // Runtime-gated par EFE_LLM_FRONTIER_ENABLED (défaut off) ; les clés provider sont lues
    // server-side par les `from_env` du coordinator. Off, ou aucune clé ⇒ frontière NON
    // servie (aucune route `/v1/llm/*` ne pourra l'utiliser — 501). Aucune clé loggée.
    #[cfg(feature = "frontier-llm")]
    if std::env::var("EFE_LLM_FRONTIER_ENABLED").map(|v| v == "true").unwrap_or(false) {
        match efe_swarm_gateway::coordinator_from_env() {
            Some(coord) => {
                state = state.with_coordinator(coord);
                println!("→ coordination frontière (Couche 2) activée — providers câblés depuis l'env.");
            }
            None => eprintln!(
                "AVERTISSEMENT: EFE_LLM_FRONTIER_ENABLED=true mais aucune clé provider \
                 (ANTHROPIC_API_KEY / OPENAI_API_KEY / GOOGLE_AI_KEY) — frontière NON servie."
            ),
        }
    }

    // ── Prod (feature `postgres`) : persistance durable write-once des records ───
    #[cfg(feature = "postgres")]
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pg = efe_swarm_gateway::PgRunStore::connect(&url).await?;
        pg.migrate().await?;
        state = state.with_store(std::sync::Arc::new(pg));
        // ADR-019 (MEM-3) : la mémoire inter-runs partage le backend durable + la MÊME source
        // de clé tenant que le reste (sinon le memory_root serait scellé sous une autre clé).
        let pg_mem = efe_swarm_gateway::PgMemoryStore::connect(&url, state.keys.clone()).await?;
        pg_mem.migrate().await?;
        state = state.with_memory_store(std::sync::Arc::new(pg_mem));
        println!("→ PgRunStore + PgMemoryStore câblés (persistance durable, write-once).");
    }

    // ── Prod (feature `oidc`) : auth JWT OIDC ES256 (clé PUBLIQUE server-side) ───
    #[cfg(feature = "oidc")]
    if let Some(verifier) = efe_swarm_gateway::OidcVerifier::from_env()? {
        state = state.with_oidc(std::sync::Arc::new(verifier));
        println!("→ auth OIDC activée (JWT ES256, clé publique server-side).");
    }

    // ── Couche 2 (M3, feature `oidc`) : auth DÉDIÉE du chemin frontière (aud=efe-llm) ───
    // Séparée de l'auth essaim : un jeton souverain ne déverrouille pas l'egress tiers coûteux.
    #[cfg(feature = "oidc")]
    if let Some(verifier) = efe_swarm_gateway::OidcVerifier::llm_from_env()? {
        state = state.with_oidc_llm(std::sync::Arc::new(verifier));
        println!("→ auth OIDC frontière activée (JWT aud=efe-llm, clé publique server-side).");
    }

    // ── G9 (feature `pii`) : politique PII par défaut depuis l'env (détection locale) ──
    #[cfg(feature = "pii")]
    if let Ok(p) = std::env::var("EFE_PII_DEFAULT_POLICY") {
        if let Ok(policy) = p.parse::<efe_swarm_gateway::PolicyKind>() {
            state = state.with_pii(efe_swarm_gateway::PiiConfig::new(policy));
            println!("→ PII opt-in : politique par défaut = {}", policy.as_str());
        }
    }

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("efe-swarm-gateway (prod) en écoute sur http://{addr}");
    println!("  POST http://{addr}/v1/swarm/run   ·   GET …/verify/:id (le moat)   ·   GET /metrics");

    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
    println!("arrêt gracieux terminé — connexions drainées.");
    Ok(())
}

/// Décision **pure** de la source de clé selon la présence (non sa valeur) du master.
/// Extraite pour être testable sans muter l'environnement du process.
fn key_source_for(master_present: bool) -> (HmacKeySource, &'static str) {
    if master_present {
        (HmacKeySource::EnvHkdf, "EnvHkdf (master server-side — PRODUCTION)")
    } else {
        (HmacKeySource::test_key(), "test_key() (DEV — provenance NON sécurisée)")
    }
}

/// Attend **SIGTERM** (rollout k8s) ou **Ctrl-C** (local) → arrêt gracieux.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    println!("signal d'arrêt reçu — fermeture en cours…");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_present_selects_envhkdf() {
        let (src, label) = key_source_for(true);
        assert!(matches!(src, HmacKeySource::EnvHkdf), "master présent → EnvHkdf (prod)");
        assert!(label.contains("PRODUCTION"));
    }

    #[test]
    fn master_absent_falls_back_to_test_key() {
        let (src, label) = key_source_for(false);
        assert!(matches!(src, HmacKeySource::StaticMaster(_)), "master absent → repli test_key");
        assert!(label.contains("DEV"));
    }
}
