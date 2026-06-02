// SPDX-License-Identifier: UNLICENSED
//! **P4.S2** — Tests du bridge HTTP/SSE (`efe-preview-server`).
//!
//! Offline : `mock_state()` (MockLlmProvider, aucune clé). Pilotage du Router
//! via `tower::ServiceExt::oneshot` (pas de socket réel).
//!
//! - **T1** : `POST /v1/missions/a` → flux SSE ordonné (`SWARM_STARTED` …
//!   `SWARM_FINISHED`), frames `data: {json}` valides, content-type SSE, et
//!   **pas de fuite** de subscriber après fin du stream.
//! - **T3** : `GET /v1/schema` → version `1.0.0` + 19 variants.
//! - liveness `/healthz`, validation `400`, et auth bearer.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use tower::ServiceExt; // `oneshot`

use efe_preview_server::{
    build_app, mock_coordinator, mock_state, AppState, PreviewConfig, MOCK_LLMS,
};
use std::sync::Arc;

/// Parse le body SSE en une liste de JSON (les frames `data: {...}`).
fn parse_sse_events(text: &str) -> Vec<serde_json::Value> {
    text.split("\n\n")
        .filter_map(|frame| {
            frame
                .lines()
                .find_map(|l| l.strip_prefix("data:").map(str::trim))
        })
        .filter_map(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .collect()
}

/// POST un body JSON et collecte le flux SSE en une liste d'events.
async fn sse_events(app: Router, uri: &str, body: &str) -> (StatusCode, Vec<serde_json::Value>) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    (status, parse_sse_events(&text))
}

// ─────────────────────────────────────────────────────────────────────────
// T1 — contrat SSE + no-leak
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t1_post_mission_a_streams_sse_contract() {
    let state = mock_state();
    let coord = state.coord.clone();
    let app = build_app(state);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/missions/a")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            r#"{"user_text":"Quelle est la capitale de la France ?"}"#,
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/event-stream"), "content-type SSE attendu, got {ct}");

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let events = parse_sse_events(&text);

    assert!(!events.is_empty(), "au moins un event SSE");
    assert_eq!(
        events.first().unwrap()["type"], "SWARM_STARTED",
        "1er event = SWARM_STARTED (abonnement sans course)"
    );
    assert_eq!(
        events.last().unwrap()["type"], "SWARM_FINISHED",
        "dernier event = SWARM_FINISHED (terminaison propre du stream)"
    );

    // Tous les events portent le même swarm_id (filtre par swarm).
    let sid = events[0]["swarm_id"].clone();
    assert!(!sid.is_null(), "swarm_id présent");
    for e in &events {
        assert_eq!(e["swarm_id"], sid, "event hors swarm: {}", e["type"]);
    }

    // No-leak : stream fini → MissionStream droppé → unsubscribe.
    assert_eq!(
        coord.bus.subscriber_count(),
        0,
        "pas de fuite de subscriber après la fin du stream"
    );
}

#[tokio::test]
async fn poc_mission_a_emits_pre_chewing_metrics() {
    // **PoC — contrat télémétrique de la démo.** Tout flux Mission A DOIT émettre
    // `PRE_CHEWING_METRICS` : c'est la donnée que l'UI affiche live (tokens
    // entrée/sortie, ratio de compression). `bridge.rs` verrouillait la séquence
    // SWARM_STARTED…SWARM_FINISHED mais PAS l'émission de cet event dans un flux
    // réel (seul le SCHÉMA le déclarait, cf. t3). En mock (sans SLM) l'event est
    // publié inconditionnellement (`mission_a.rs`), avec `input_tokens_raw > 0` et
    // `compressed ≤ raw`. En keyless + SLM réel, `compressed < raw` (compression
    // effective) — non vérifiable ici sans llama-server.
    let app = build_app(mock_state());
    let (status, events) = sse_events(
        app,
        "/v1/missions/a",
        r#"{"user_text":"Quelle est la capitale de la France ?"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let pcm = events
        .iter()
        .find(|e| e["type"] == "PRE_CHEWING_METRICS")
        .expect("PRE_CHEWING_METRICS émis dans tout flux Mission A");
    let raw = pcm["metrics"]["input_tokens_raw"]
        .as_u64()
        .expect("metrics.input_tokens_raw présent (u64)");
    assert!(raw > 0, "input_tokens_raw > 0 (le prompt a une taille)");
    let comp = pcm["metrics"]["input_tokens_compressed"]
        .as_u64()
        .expect("metrics.input_tokens_compressed présent (u64)");
    assert!(comp <= raw, "invariant : compressed ({comp}) ≤ raw ({raw})");
}

#[tokio::test]
async fn mission_a_reliably_reaches_swarm_finished() {
    // Smoke déterministe du chemin de streaming lib : 8 runs de Mission A
    // doivent TOUS aller de SWARM_STARTED jusqu'à SWARM_FINISHED. Ne couvre PAS
    // le bug `default_llms` (qui vivait dans main.rs, hors de ce harness) — cf.
    // `mock_default_llms_are_all_registered_providers` pour cet invariant.
    for i in 0..8 {
        let app = build_app(mock_state());
        let (status, events) =
            sse_events(app, "/v1/missions/a", r#"{"user_text":"Capitale ?"}"#).await;
        assert_eq!(status, StatusCode::OK, "run {i}");
        assert_eq!(events.first().unwrap()["type"], "SWARM_STARTED", "run {i}");
        assert_eq!(
            events.last().unwrap()["type"],
            "SWARM_FINISHED",
            "run {i}: Mission A doit router de façon fiable et aller jusqu'au bout"
        );
    }
}

#[tokio::test]
async fn mock_default_llms_are_all_registered_providers() {
    // Garde anti-régression du bug Phase 4 : `default_llms` contenait un id
    // (`"mock"`) qu'aucun provider enregistré ne servait → Mission A routait
    // vers un provider inexistant et mourait après SWARM_STARTED. On épingle
    // l'invariant à la SOURCE : chaque id de `MOCK_LLMS` (partagé par `for_mock`
    // ET le binaire) DOIT être un provider réellement enregistré, donc une
    // Mission A ciblant explicitement cet id atteint SWARM_FINISHED.
    assert!(!MOCK_LLMS.is_empty(), "MOCK_LLMS ne doit pas être vide");
    for id in MOCK_LLMS {
        let app = build_app(mock_state());
        let body = format!(r#"{{"user_text":"Capitale ?","available_llms":["{id}"]}}"#);
        let (status, events) = sse_events(app, "/v1/missions/a", &body).await;
        assert_eq!(status, StatusCode::OK, "llm {id}");
        let tags: Vec<_> = events.iter().map(|e| e["type"].clone()).collect();
        assert_eq!(
            events.last().unwrap()["type"],
            "SWARM_FINISHED",
            "llm {id}: provider doit être enregistré ; events = {tags:?}"
        );
        // Le provider effectivement utilisé doit être l'id demandé (pas un fallback).
        let used = events
            .iter()
            .find(|e| e["type"] == "LLM_FINISHED")
            .and_then(|e| e["provider"].as_str());
        assert_eq!(used, Some(*id), "Mission A doit servir le provider {id}");
    }
}

// ─────────────────────────────────────────────────────────────────────────
// T3 — schéma (garde-fou non-dérive)
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t3_schema_lists_version_and_19_variants() {
    let app = build_app(mock_state());
    let req = Request::builder()
        .uri("/v1/schema")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["event_schema_version"], "1.0.0");
    let variants = json["variants"].as_array().expect("variants[]");
    assert_eq!(variants.len(), 19, "19 variants canoniques");
    assert!(variants.iter().any(|v| v == "SWARM_STARTED"));
    assert!(variants.iter().any(|v| v == "PRE_CHEWING_METRICS"));
    assert!(variants.iter().any(|v| v == "DRAFT_REJECTED_AS_NOISE"));
}

// ─────────────────────────────────────────────────────────────────────────
// Liveness
// ─────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────
// T6 — CORS (origine du dev-server autorisée)
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cors_allows_configured_origin() {
    let app = build_app(mock_state());

    // Requête simple avec Origin → header allow-origin présent.
    let req = Request::builder()
        .uri("/healthz")
        .header("origin", "http://localhost:3000")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let allow = resp
        .headers()
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        allow,
        Some("http://localhost:3000"),
        "CORS allow-origin pour le dev-server Next.js"
    );
}

#[tokio::test]
async fn cors_preflight_returns_allow_methods() {
    let app = build_app(mock_state());
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/v1/missions/a")
        .header("origin", "http://localhost:3000")
        .header("access-control-request-method", "POST")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // tower-http CorsLayer répond au preflight (2xx) avec les headers CORS.
    assert!(
        resp.status().is_success(),
        "preflight CORS doit réussir, got {}",
        resp.status()
    );
    assert!(
        resp.headers().contains_key("access-control-allow-origin"),
        "header allow-origin sur le preflight"
    );
}

#[tokio::test]
async fn healthz_ok() {
    let app = build_app(mock_state());
    let req = Request::builder().uri("/healthz").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..], b"ok\n");
}

// ─────────────────────────────────────────────────────────────────────────
// Validation — user_text vide → 400
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_user_text_is_bad_request() {
    let app = build_app(mock_state());
    let req = Request::builder()
        .method("POST")
        .uri("/v1/missions/a")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"user_text":"   "}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ─────────────────────────────────────────────────────────────────────────
// Auth bearer — exigé quand un token est configuré
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn bearer_required_when_token_configured() {
    let mut cfg = PreviewConfig::for_mock();
    cfg.token = Some("s3cret".to_string());
    let state = AppState {
        coord: efe_preview_server::mock_coordinator(),
        config: Arc::new(cfg),
    };
    let app = build_app(state);

    // Sans bearer → 401 sur une route protégée.
    let req = Request::builder()
        .uri("/v1/schema")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Avec bearer correct → 200.
    let req_ok = Request::builder()
        .uri("/v1/schema")
        .header(header::AUTHORIZATION, "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    let resp_ok = app.oneshot(req_ok).await.unwrap();
    assert_eq!(resp_ok.status(), StatusCode::OK);

    // /healthz reste ouvert (non authentifié).
    let app2 = build_app({
        let mut c = PreviewConfig::for_mock();
        c.token = Some("s3cret".into());
        AppState { coord: mock_coordinator(), config: Arc::new(c) }
    });
    let req_h = Request::builder().uri("/healthz").body(Body::empty()).unwrap();
    assert_eq!(app2.oneshot(req_h).await.unwrap().status(), StatusCode::OK);
}

// ─────────────────────────────────────────────────────────────────────────
// Mission B (vote) + Speculative — contrat SSE
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn post_mission_b_streams_vote() {
    let app = build_app(mock_state());
    let (status, events) = sse_events(
        app,
        "/v1/missions/b",
        r#"{"user_text":"Quelle est la capitale de la France ?","llms":["mock-claude","mock-gemini","mock-gpt"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!events.is_empty());
    assert_eq!(events.first().unwrap()["type"], "SWARM_STARTED");
    assert_eq!(events.first().unwrap()["mission"], "MISSION_B");
    assert_eq!(events.last().unwrap()["type"], "SWARM_FINISHED");
    assert!(
        events.iter().any(|e| e["type"] == "VOTE_RESOLVED"),
        "un VOTE_RESOLVED attendu en Mission B"
    );
}

#[tokio::test]
async fn post_mission_speculative_streams() {
    let app = build_app(mock_state());
    let (status, events) = sse_events(
        app,
        "/v1/missions/speculative",
        r#"{"user_text":"Capitale de la France ?","drafter_llm":"mock-gemini","verifier_llm":"mock-claude"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!events.is_empty());
    assert_eq!(events.first().unwrap()["type"], "SWARM_STARTED");
    assert_eq!(events.first().unwrap()["mission"], "SPECULATIVE");
    assert_eq!(events.last().unwrap()["type"], "SWARM_FINISHED");
}

#[tokio::test]
async fn mission_b_too_few_llms_is_bad_request() {
    let app = build_app(mock_state());
    let (status, _) = sse_events(
        app,
        "/v1/missions/b",
        r#"{"user_text":"x","llms":["mock-claude"]}"#,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ─────────────────────────────────────────────────────────────────────────
// T5 — /v1/savings-report
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t5_savings_report_served_when_present() {
    // Fixture : un SavingsReport valide (le champ titre `savings_ratio_microcents`).
    let json = efe_coordinator::SavingsReport::new()
        .to_json()
        .expect("to_json");
    let path = std::env::temp_dir().join("efe_preview_savings_present.json");
    std::fs::write(&path, &json).unwrap();

    let mut cfg = PreviewConfig::for_mock();
    cfg.savings_report_path = path.clone();
    let app = build_app(AppState {
        coord: mock_coordinator(),
        config: Arc::new(cfg),
    });

    let req = Request::builder()
        .uri("/v1/savings-report")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("application/json"), "content-type JSON, got {ct}");

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.contains("savings_ratio_microcents"),
        "le rapport doit contenir le champ titre"
    );

    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn savings_report_404_when_missing() {
    let mut cfg = PreviewConfig::for_mock();
    cfg.savings_report_path = std::env::temp_dir().join("efe_preview_does_not_exist_zzz.json");
    let app = build_app(AppState {
        coord: mock_coordinator(),
        config: Arc::new(cfg),
    });
    let req = Request::builder()
        .uri("/v1/savings-report")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ─────────────────────────────────────────────────────────────────────────
// Resilience Theater — POST /v1/resilience/run
// ─────────────────────────────────────────────────────────────────────────

async fn assert_resilience(scenario: &str, expected_tag: &str) {
    let app = build_app(mock_state());
    let (status, events) =
        sse_events(app, "/v1/resilience/run", &format!(r#"{{"scenario":"{scenario}"}}"#)).await;
    assert_eq!(status, StatusCode::OK, "scénario {scenario}");
    assert!(
        events.iter().any(|e| e["type"] == expected_tag),
        "{expected_tag} attendu pour {scenario}, got: {:?}",
        events.iter().map(|e| e["type"].clone()).collect::<Vec<_>>()
    );
    // La dégradation est RÉCUPÉRÉE → la mission se termine (pas un crash).
    assert!(
        events
            .iter()
            .any(|e| e["type"] == "SWARM_FINISHED" || e["type"] == "SWARM_FAILED"),
        "terminaison attendue pour {scenario}"
    );
}

#[tokio::test]
async fn resilience_fallback_emits_llm_fallback() {
    assert_resilience("fallback", "LLM_FALLBACK_TRIGGERED").await;
}

#[tokio::test]
async fn resilience_drafter_bypass_emits_bypass() {
    assert_resilience("drafter_bypass", "DRAFTER_BYPASSED").await;
}

#[tokio::test]
async fn resilience_noise_rejection_emits_noise() {
    assert_resilience("noise_rejection", "DRAFT_REJECTED_AS_NOISE").await;
}

// ─────────────────────────────────────────────────────────────────────────
// /metrics — réexposition Prometheus (R3.S7)
// ─────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────
// A/B Lab — POST /v1/ab/run
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ab_run_segregates_control_vs_experiment() {
    let app = build_app(mock_state());
    let req = Request::builder()
        .method("POST")
        .uri("/v1/ab/run")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"n":16}"#))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["n"], 16);

    let c = &json["comparison"]["control"];
    let e = &json["comparison"]["experiment"];
    let cn = c["n_runs"].as_u64().unwrap();
    let en = e["n_runs"].as_u64().unwrap();
    assert!(cn > 0, "control reçoit du trafic");
    assert!(en > 0, "experiment reçoit du trafic");
    assert_eq!(cn + en, 16, "tous les runs étiquetés");

    // experiment (cost-weighted → gemini cheap) doit être moins cher que
    // control (quality-weighted → claude expensive).
    let c_cost = c["avg_cost_microcents"].as_f64().unwrap();
    let e_cost = e["avg_cost_microcents"].as_f64().unwrap();
    assert!(e_cost < c_cost, "experiment ({e_cost} µ¢) < control ({c_cost} µ¢)");

    // Versions de table tracées.
    assert_eq!(c["table_version"], "control-quality");
    assert_eq!(e["table_version"], "experiment-cost");
}

// ─────────────────────────────────────────────────────────────────────────
// ServeDir — frontend statique servi en fallback (P4.S6)
// ─────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn static_dir_serves_index_html() {
    let dir = std::env::temp_dir().join("efe_preview_static_test");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<!doctype html><h1>EFE Preview Lab</h1>").unwrap();

    let mut cfg = PreviewConfig::for_mock();
    cfg.static_dir = Some(dir.clone());
    let app = build_app(AppState {
        coord: mock_coordinator(),
        config: Arc::new(cfg),
    });

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("EFE Preview Lab"), "index.html servi en fallback");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn metrics_endpoint_renders_prometheus() {
    let app = build_app(mock_state());
    let req = Request::builder().uri("/metrics").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/plain"), "content-type Prometheus, got {ct}");

    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.contains("efe_missions_total"),
        "exposition Prometheus EFE attendue"
    );
}
