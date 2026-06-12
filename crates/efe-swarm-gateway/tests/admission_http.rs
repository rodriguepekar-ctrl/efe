// SPDX-License-Identifier: UNLICENSED
//! **ADR-014 sur le fil** — admission process (CP-2) + K adaptatif (K-2) en HTTP :
//! `POST /v1/swarm/run` → 429 honnête à capacité ; `"k"` nombre OU objet ; la trace
//! de sizing dans la réponse ; `GET /v1/swarm/verify/:id` re-prouve l'adaptatif ;
//! la largeur traverse sans toucher la sémantique (I-CP1).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use efe_swarm_gateway::{router, SwarmGatewayState};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

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
async fn gateway_rejects_429_when_at_capacity_then_recovers() {
    // 1 permit, tenu artificiellement → la passerelle est « pleine » : 429 + Retry-After
    // + corps stable — du backpressure VISIBLE, jamais une file invisible.
    let state = SwarmGatewayState::in_memory().with_admission_permits(1);
    let app = router(state.clone());
    let held = state.admission.clone().try_acquire_owned().expect("tient l'unique permit");

    let run_body = json!({
        "task": { "text": "x", "expected": "x" },
        "topology": { "kind": "consensus", "generator": "echo_generator", "judges": ["consistency"], "k": 3 },
        "budget": { "max_cents": 100 }
    });
    let resp = app.clone().oneshot(post_run(run_body.clone())).await.expect("réponse");
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS, "à capacité → 429");
    assert_eq!(
        resp.headers().get("retry-after").and_then(|v| v.to_str().ok()),
        Some("1"),
        "Retry-After présent (le client sait quoi faire)"
    );
    let body = body_json(resp).await;
    assert_eq!(body["error"], json!("too_many_requests"), "le code machine de l'enveloppe stable");
    assert_eq!(body["inflight_max"], json!(1));

    // le permit se libère → la même requête passe (FIFO, pas d'état rémanent).
    drop(held);
    let resp = app.oneshot(post_run(run_body)).await.expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK, "capacité rendue → admis");
    let run = body_json(resp).await;
    assert_eq!(run["run"]["provenance"]["verified"], json!(true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adaptive_k_over_http_returns_sizing_and_reproves() {
    let app = router(SwarmGatewayState::in_memory());

    // `"k"` objet → adaptatif ; const_generator unanime → arrêt au plancher.
    let resp = app
        .clone()
        .oneshot(post_run(json!({
            "task": { "text": "easy" },
            "topology": { "kind": "consensus", "generator": "const_generator",
                          "judges": ["consistency"],
                          "k": { "min": 4, "max": 24, "agreement": 0.9 } },
            "budget": { "max_cents": 100 }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::OK);
    let run = body_json(resp).await;
    assert_eq!(run["run"]["output"]["sizing"]["k_used"], json!(4), "la trace voyage sur le fil");
    assert_eq!(run["run"]["output"]["sizing"]["stop_reason"], json!("target"));
    assert_eq!(run["run"]["accounting"]["agents_used"], json!(5), "4 + 1 juge — l'économie est réelle");
    assert_eq!(run["run"]["provenance"]["verified"], json!(true));
    let run_id = run["run_id"].as_str().expect("run_id").to_string();

    // la re-preuve par l'API (ADR-009) — l'adaptatif s'y plie comme tout le monde.
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
    assert_eq!(verify["verified"], json!(true), "re-prouvé depuis les records persistés");
    assert_eq!(verify["dag_consistent"], json!(true));
    assert_eq!(verify["records"], json!(5), "4 générations + 1 ballot");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn width_traverses_the_wire_without_touching_semantics() {
    // I-CP1 en HTTP : largeurs 1 / 32 / absente — même tâche, même sortie. (32 sera
    // plafonné serveur si EFE_GATEWAY_MAX_WIDTH < 32 — sans effet observable : c'est
    // tout le point.)
    let app = router(SwarmGatewayState::in_memory());
    let mk = |width: Option<usize>| {
        let mut b = json!({
            "task": { "text": "w probe", "expected": "w probe" },
            "topology": { "kind": "consensus", "generator": "echo_generator",
                          "judges": ["consistency", "length_band"], "k": 8 },
            "budget": { "max_cents": 100 }
        });
        if let Some(w) = width {
            b.as_object_mut().unwrap().insert("width".into(), json!(w));
        }
        b
    };
    let mut outputs = Vec::new();
    for w in [Some(1), Some(32), None] {
        let resp = app.clone().oneshot(post_run(mk(w))).await.expect("réponse");
        assert_eq!(resp.status(), StatusCode::OK, "width={w:?}");
        let run = body_json(resp).await;
        assert_eq!(run["run"]["provenance"]["verified"], json!(true));
        outputs.push(run["run"]["output"].clone());
    }
    assert_eq!(outputs[0], outputs[1], "largeur 1 == largeur 32 (sémantique intacte)");
    assert_eq!(outputs[0], outputs[2], "largeur explicite == Auto");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adaptive_k_bad_combos_are_400_on_the_wire() {
    let app = router(SwarmGatewayState::in_memory());
    // verified_quota sur consensus : 400 NET (le contrat dit où aller).
    let resp = app
        .clone()
        .oneshot(post_run(json!({
            "task": {},
            "topology": { "kind": "consensus", "generator": "g", "judges": ["consistency"],
                          "k": { "min": 2, "max": 8, "verified_quota": 1 } }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert!(
        body["message"].as_str().unwrap_or("").contains("generate_verify"),
        "l'erreur oriente vers la bonne topologie : {body}"
    );
    // zéro cible → 400 aussi.
    let resp = app
        .clone()
        .oneshot(post_run(json!({
            "task": {},
            "topology": { "kind": "consensus", "generator": "g", "judges": ["consistency"],
                          "k": { "min": 2, "max": 8 } }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Revue ADR-014 : un corps qui ÉCHOUE LA DÉSÉRIALISATION (k: 3.5, typo de champ)
    // sort en 400 + enveloppe JSON stable {error, message} — plus jamais le 422
    // text/plain opaque de l'extracteur nu.
    for bad in [
        json!({ "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"], "k": 3.5 } }),
        json!({ "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                                           "k": { "min": 2, "max": 4, "se_targt": 0.1, "agreement": 0.0 } } }),
        json!({ "task": {}, "topology": { "kind": "auto" }, "widht": 4 }),
    ] {
        let resp = app.clone().oneshot(post_run(bad.clone())).await.expect("réponse");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "corps malformé : {bad}");
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        assert!(ct.starts_with("application/json"), "enveloppe JSON stable, eu {ct}");
        let body = body_json(resp).await;
        assert_eq!(body["error"], json!("bad_request"));
        assert!(body["message"].is_string());
    }

    // et une cible hors domaine → 400 net (jamais une facture k_max silencieuse).
    let resp = app
        .oneshot(post_run(json!({
            "task": {},
            "topology": { "kind": "consensus", "generator": "g", "judges": ["consistency"],
                          "k": { "min": 2, "max": 8, "agreement": 1.7 } }
        })))
        .await
        .expect("réponse");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
