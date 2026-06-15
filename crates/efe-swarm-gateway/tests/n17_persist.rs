// SPDX-License-Identifier: UNLICENSED
//! **ADR-016 sur le fil** — la persistance N17 + la re-preuve forensique, en HTTP.
//!
//! Volet keyless (Mode A) : un run sans appel LLM persiste 0 forensique et reste
//! `verified` trivialement (I-PN6 : la feature est invisible au chemin keyless), et les
//! deux endpoints neufs répondent — `verify/:id` expose le verdict N17, `/runs/:id/n17`
//! exporte la forensique (vide ici). Le chemin Mode C (forensique non vide, falsification
//! détectée) est prouvé au niveau provenance (`n17_chain`) et à la passerelle (lib tests
//! synthétiques) ; le bout-en-bout LLM réel est un test ignoré-sans-Ollama.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use efe_swarm_gateway::{router, SwarmGatewayState};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

fn app() -> axum::Router {
    router(SwarmGatewayState::in_memory())
}

fn post_run(body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/swarm/run")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("requête")
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = resp.into_body().collect().await.expect("corps").to_bytes();
    serde_json::from_slice(&bytes).expect("json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keyless_run_persists_zero_n17_and_stays_verified() {
    // I-PN6 : Mode A no-op. Le run keyless (0 appel LLM) persiste 0 forensique ; la
    // re-preuve reste verified, le verdict N17 est trivial (chaîne intacte, pont cohérent).
    let app = app();
    let resp = app
        .clone()
        .oneshot(post_run(json!({
            "task": { "text": "x", "expected": "x" },
            "topology": { "kind": "consensus", "generator": "echo_generator",
                          "judges": ["consistency", "length_band", "j_non_empty"], "k": 5 },
            "budget": { "max_cents": 1000 }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK);
    let run = body_json(resp).await;
    let run_id = run["run_id"].as_str().expect("run_id").to_string();

    // re-preuve : le verdict GLOBAL reste verified, et le bloc N17 est présent et trivial.
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(format!("/v1/swarm/verify/{run_id}")).body(Body::empty()).unwrap())
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["verified"], json!(true), "Mode A : re-preuve complète triviale");
    assert_eq!(v["n17"]["records"], json!(0), "0 forensique persistée (keyless)");
    assert_eq!(v["n17"]["chain_intact"], json!(true), "chaîne N17 vide = intègre");
    assert_eq!(v["n17"]["bridge_consistent"], json!(true), "0 Mode C = pont cohérent");
    assert_eq!(v["n17"]["mode_c_consistent"], json!(0));
    assert_eq!(v["n17"]["root"], Value::Null, "pas de n17_root pour un run Mode A");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n17_export_endpoint_round_trips_empty_for_mode_a() {
    // L'export jumeau /n17 répond (le tiers récupère la 2ᵉ moitié) ; vide en keyless.
    let app = app();
    let resp = app
        .clone()
        .oneshot(post_run(json!({
            "task": { "text": "x", "expected": "x" }, "topology": { "kind": "auto" }
        })))
        .await
        .expect("réponse");
    let run = body_json(resp).await;
    let run_id = run["run_id"].as_str().expect("run_id").to_string();

    let resp = app
        .oneshot(Request::builder().uri(format!("/v1/swarm/runs/{run_id}/n17")).body(Body::empty()).unwrap())
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK);
    let export = body_json(resp).await;
    assert_eq!(export["run_id"], json!(run_id));
    assert_eq!(export["llm_records"], json!([]), "0 forensique exportée (Mode A)");
    assert_eq!(export["n17_leaves"], json!([]));
    assert_eq!(export["n17_root"], Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn n17_export_unknown_run_is_404_and_tenant_scoped() {
    let app = app();
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/swarm/runs/{}/n17", uuid::Uuid::new_v4()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND, "run inconnu → 404 (comme /records)");
}
