// SPDX-License-Identifier: UNLICENSED
//! **Test d'intégration Network EN-PROCESS, sans aucune clé.**
//!
//! On démarre un **faux routeur OpenAI-compatible** in-process (un mini-serveur axum sur un port
//! éphémère) qui répond à `POST /v1/chat/completions`. On câble le mode `Network` de la passerelle
//! dessus et on lance un run : ça valide, en CI et **sans provider ni clé**, tout le chemin Network —
//! `resolve_mode` (Network→SelfHost-vers-routeur), le mapping provider→modèle, l'appel OpenAI-compat,
//! la provenance re-prouvée, le tarif Network préservé (I13), et le pont N17.
//!
//! C'est la « preuve Network sans clé » : la passerelle parle au routeur **sans en-tête d'auth** ;
//! ici le routeur est un bouchon, mais le contrat (forme de requête/réponse) est réel.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use efe_swarm_gateway::{router, RouterMap, SwarmGatewayState};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

/// Démarre un faux routeur OpenAI-compat sur 127.0.0.1:0 ; renvoie sa base URL suffixée `/v1`.
async fn spawn_fake_router() -> String {
    async fn chat() -> Json<Value> {
        // Réponse OpenAI-compat minimale : le backend lit choices[0].message.content.
        Json(json!({ "choices": [ { "message": { "role": "assistant", "content": "legal" } } ] }))
    }
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        // Réponse OK sur tout le reste (sonde de santé du backend : curl GET {endpoint}).
        .fallback(get(|| async { (StatusCode::OK, "ok") }).post(|| async { (StatusCode::OK, "ok") }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind faux routeur");
    let addr = listener.local_addr().expect("addr locale");
    // Port déjà en écoute (bind effectué) → les connexions sont acceptées dès le spawn ; pas de course.
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/v1")
}

fn post_run(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/swarm/run")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("requête valide")
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.expect("corps").to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn network_run_via_inprocess_router_without_any_key() {
    let router_url = spawn_fake_router().await;

    // Passerelle câblée sur le faux routeur ; mapping provider→modèle explicite (non-secret).
    let state = SwarmGatewayState::in_memory()
        .with_router_endpoint(&router_url)
        .with_router_models(RouterMap::empty().with("anthropic", "anthropic/claude-3-5-sonnet"));
    let app = router(state);

    let resp = app
        .oneshot(post_run(json!({
            "task": { "text": "The court ruled the clause unenforceable.", "expected": "legal" },
            "topology": { "kind": "consensus",
                          "generator": "Classify the topic in one word.",
                          "judges": ["Judge how well the label fits."],
                          "k": 3, "pattern": "borda" },
            "budget": { "max_cents": 5000 },
            "mode": { "network": { "provider": "anthropic" } }
        })))
        .await
        .expect("réponse passerelle");

    assert_eq!(resp.status(), StatusCode::OK, "Network câblé → 200 (pas 501)");
    let run = body_json(resp).await;

    // 1) la provenance est re-prouvée.
    assert!(run["run"]["provenance"]["verified"].as_bool().unwrap_or(false), "run re-prouvé");
    // 2) le routage OpenAI-compat a opéré + le mapping provider→modèle (Inc.4.b).
    assert_eq!(run["run"]["backend"]["engine"].as_str(), Some("openai_local"));
    assert_eq!(run["run"]["backend"]["model"].as_str(), Some("anthropic/claude-3-5-sonnet"), "provider→modèle mappé");
    // 3) cost-governance : le tarif Network est facturé (≠ 0¢), jamais au-delà du plafond (I13).
    let spent = run["run"]["budget"]["spent_cents"].as_u64().unwrap_or(0);
    assert!(spent > 0, "tarif Network facturé (spent_cents > 0), eu {spent}");
    assert!(spent <= 5000, "I13 : jamais au-delà du plafond");
    // 4) pont N17 cohérent (chaque appel LLM lié à un record forensique).
    assert_eq!(run["run"]["n17"]["broken"].as_u64(), Some(0), "aucun lien N17 rompu");
    assert_eq!(run["run"]["n17"]["all_consistent"].as_bool(), Some(true), "N17 cohérent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn network_without_router_is_501() {
    // Sans routeur câblé : Network → 501 (déploiement souverain par défaut). Aucun appel sortant.
    let app = router(SwarmGatewayState::in_memory());
    let resp = app
        .oneshot(post_run(json!({
            "task": { "text": "x" },
            "topology": { "kind": "auto" },
            "mode": { "network": { "provider": "anthropic" } }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}
