// SPDX-License-Identifier: UNLICENSED
//! Intégration HTTP des topologies itératives : le contrat de fil COMPLET —
//! `POST /v1/swarm/run` (gossip · debate escaladé) puis `GET /v1/swarm/verify/:id`
//! (l'ADR-009 appliqué à l'itératif : la re-preuve par l'API, lectures comprises).

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
async fn gossip_over_http_runs_then_reproves() {
    let app = app();

    // 1) le run — la forme de fil du DTO (kind: gossip), keyless.
    let resp = app
        .clone()
        .oneshot(post_run(json!({
            "task": { "initial": [90.0, 110.0, 95.0, 105.0] },
            "topology": { "kind": "gossip", "refiner": "mean_pull",
                          "k": 4, "max_rounds": 6, "epsilon": 0.1, "metric": "variance" },
            "budget": { "max_cents": 100 }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK);
    let run = body_json(resp).await;

    assert_eq!(run["run"]["provenance"]["verified"], json!(true), "scellé + re-prouvé");
    let v = run["run"]["output"]["value"].as_f64().expect("valeur");
    assert!((v - 100.0).abs() < 0.5, "converge vers 100, eu {v}");
    assert_eq!(run["run"]["output"]["gossip"]["converged"], json!(true));
    let run_id = run["run_id"].as_str().expect("run_id").to_string();

    // 2) la re-preuve par l'API (le moat, ADR-009) — l'itératif s'y plie comme T1.
    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/v1/swarm/verify/{run_id}"))
                .body(Body::empty())
                .expect("requête"),
        )
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK);
    let verify = body_json(resp).await;
    assert_eq!(verify["verified"], json!(true), "re-preuve indépendante OK : {verify}");
    assert_eq!(verify["dag_consistent"], json!(true), "le graphe d'influence (reads) passe le DAG");
    assert_eq!(
        verify["root"], run["run"]["provenance"]["root"],
        "la racine reconstruite égale la racine annoncée"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn debate_with_escalation_over_http_economises() {
    let app = app();
    // const_generator → candidats unanimes → l'étage ① (Vote gratuit) suffit :
    // l'escalade traverse le fil (DTO `escalate`) jusqu'à la topologie.
    let resp = app
        .oneshot(post_run(json!({
            "task": { "text": "t", "expected": "t" },
            "topology": { "kind": "debate", "generator": "const_generator",
                          "judges": ["j_equals_expected"], "k": 4, "rounds": 3,
                          "escalate": true },
            "budget": { "max_cents": 100 }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK);
    let run = body_json(resp).await;
    assert_eq!(run["run"]["provenance"]["verified"], json!(true));
    assert_eq!(
        run["run"]["accounting"]["agents_used"],
        json!(4),
        "K seulement : ni juge ni débat payés (étage ①)"
    );
    assert_eq!(run["run"]["budget"]["spent_cents"], json!(4));
}
