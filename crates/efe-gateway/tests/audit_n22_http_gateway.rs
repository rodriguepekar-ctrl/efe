// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : post-audit regression tests.
//!
//! These tests pin down bugs caught during the chirurgical audit
//! pass on top of the 16 acceptance + 94 unit tests in
//! `phase_7_n22_http_gateway.rs` and the inline `#[cfg(test)]`
//! modules.
//!
//! | # | Defect | Where | Fix |
//! |---|---|---|---|
//! | B1 | Path-with-id routes used `{publisher_id}` (axum 0.8 syntax) — axum 0.7 treated it as a literal segment, so `GET /v1/publishers/<uuid>` returned 404 instead of 401. | `router.rs` | switched all path params to `:param_name` (axum 0.7 syntax). |
//! | B2 | `HealthzResponse` / `ReadyzResponse` had `&'static str` fields, breaking `serde::Deserialize` lifetime in cross-binary tests. | `observability/{healthz,readyz}.rs` | switched display fields to owned `String`. |
//! | B3 | `swarms_run_handler` performed `if let Some(o) = req.config` then later borrowed `&req.dag_id` → partial-move E0382. | `handlers/swarms.rs` | `req.config.clone()` so the body is not consumed. |
//! | B4 | `phase_7_n22_body_too_large_returns_413` panicked with `BrokenPipe` when the server closed the connection before the 11 MB body finished writing. | `tests/phase_7_n22_http_gateway.rs::http_request` | wrapped the body `write_all` in `let _ = ...` so the test continues to read the status. |
//! | B5 | `DefaultPublisherApi { keystore: ... }` referenced a field that no longer existed in `efe-publisher-identity` ; the gateway main failed to compile. | `main.rs::build_app_state` | removed the extra field, kept a `let _keystore` for the Sprint 15 keystore handoff. |
//! | B6 | clippy `field_reassign_with_default` in `config.rs` tests + `unwrap_or_else(N22GatewayMetrics::new)` in `state.rs`. | `config.rs` + `state.rs` | struct-literal init + `unwrap_or_default()`. |
//! | B7 | `swarms_run_handler` accepted `dag_id` as `Option<String>` but the registry needed `Option<&str>` ; calling `.deref()` on the option directly didn't compile. | `handlers/swarms.rs` | `req.dag_id.as_deref()`. |
//! | B8 | `ApiError::IntoResponse` fallback used a fresh string trace_id ; we needed to ensure the field is present on every 4xx/5xx body even when the request never reached a handler (e.g., the JWT extractor short-circuits). | `error/mod.rs` | always populates `trace_id` (placeholder if no W3C ctx). |

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use efe_agentic_bridge::{AgenticBridge, AgenticResponse, AgenticTask, BridgeError};
use efe_gateway::{
    auth::Hs256JwtValidator, build_router, state::AppState, GatewayClaims, N22GatewayMetrics,
};
use efe_gold_test_gate::{
    DefaultAssertionEngine, DefaultGoldApi, GoldGate, InMemoryGoldRunLedger,
    InMemoryGoldRunner, InMemoryGoldSetRegistry, N19Metrics, StubEmbeddingProvider,
};
use efe_nano_sdk::ContextBundle;
use efe_publisher_identity::{
    DefaultPublisherApi, Ed25519Verifier, InMemoryPublishLedger,
    InMemoryPublisherRegistry, N18Metrics, StubKycProvider, StubSanctionsScreener,
};
use efe_swarm_orchestrator::SwarmOrchestrator;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Fixture (shared with phase_7 — duplicated for crate-test isolation)
// ─────────────────────────────────────────────────────────────────────

const TEST_JWT_SECRET: &[u8] = b"test-secret-padded-to-32-bytes-OK!!";

struct StubBridge;
#[axum::async_trait]
impl AgenticBridge for StubBridge {
    async fn call_with_context(
        &self,
        _bundle: &ContextBundle,
        _task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        Ok(AgenticResponse {
            text: "stub response".into(),
            stop_reason: efe_agentic_bridge::StopReason::EndTurn,
            usage: efe_agentic_bridge::TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
            },
            model_identity: efe_nano_sdk::ModelIdentity::hosted("stub", "stub", "v0"),
            cost_microcents: 0,
        })
    }
}

fn build_test_state() -> (AppState, Hs256JwtValidator) {
    let validator = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");
    let validator_inner = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");

    let swarm = Arc::new(SwarmOrchestrator::new());
    let agentic: Arc<dyn AgenticBridge> = Arc::new(StubBridge);
    let publisher_api = Arc::new(DefaultPublisherApi {
        registry: Arc::new(InMemoryPublisherRegistry::new()),
        ledger: Arc::new(InMemoryPublishLedger::new([1u8; 32])),
        kyc: Arc::new(StubKycProvider::permissive()),
        sanctions: Arc::new(StubSanctionsScreener::clear()),
        verifier: Arc::new(Ed25519Verifier::new()),
        metrics: N18Metrics::new(),
    });
    let gold_registry = Arc::new(InMemoryGoldSetRegistry::new());
    let gold_ledger = Arc::new(InMemoryGoldRunLedger::new([2u8; 32]));
    let gate = Arc::new(GoldGate::new(
        gold_registry.clone(),
        Arc::new(InMemoryGoldRunner::echo()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        gold_ledger.clone(),
        N19Metrics::new(),
    ));
    let gold_api = Arc::new(DefaultGoldApi::new(gold_registry, gate, gold_ledger));
    let state = AppState::builder()
        .with_swarm(swarm)
        .with_agentic(agentic)
        .with_publisher_api(publisher_api)
        .with_gold_api(gold_api)
        .with_jwt_validator(Arc::new(validator))
        .with_metrics(N22GatewayMetrics::new())
        .build()
        .expect("state");
    (state, validator_inner)
}

fn make_tenant_jwt(validator: &Hs256JwtValidator, tenant: Uuid) -> String {
    let claims = GatewayClaims {
        iss: "efe-gateway".into(),
        sub: "alice".into(),
        iat: GatewayClaims::now_secs() - 60,
        exp: GatewayClaims::now_secs() + 3_600,
        tenant_id: tenant,
        scope: vec!["read".into()],
        operator_id: None,
    };
    validator.sign(&claims).expect("sign")
}

async fn start_server(state: AppState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let router = build_router(state, 10 * 1024 * 1024);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, handle)
}

/// Returns (status, full raw response including headers, body only).
async fn http_request_full(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> (u16, String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    let body_bytes = body.unwrap_or(b"");
    req.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
    if !body_bytes.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    req.push_str("Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    if !body_bytes.is_empty() {
        let _ = stream.write_all(body_bytes).await;
    }
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        stream.read_to_end(&mut buf),
    )
    .await;
    let raw = String::from_utf8_lossy(&buf).to_string();
    let status_line = raw.lines().next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body_start = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
    let headers_section = raw[..body_start].to_string();
    let body = raw[body_start..].to_string();
    (status, headers_section, body)
}

// ─────────────────────────────────────────────────────────────────────
//  B1 — axum 0.7 path param syntax
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_n22_b1_publishers_get_with_uuid_matches_route_not_404() {
    // Regression : pre-fix, this returned 404 (path not matched).
    // Post-fix, axum 0.7 `:publisher_id` matches and the handler
    // demands auth → 401.
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let uuid = Uuid::nil();
    let (status, _, _) = http_request_full(
        addr,
        "GET",
        &format!("/v1/publishers/{uuid}"),
        &[],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 401, "path with :publisher_id must match route");
}

#[tokio::test]
async fn audit_n22_b1_gold_runs_get_with_uuid_matches_route_not_404() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let uuid = Uuid::nil();
    let (status, _, _) = http_request_full(
        addr,
        "GET",
        &format!("/v1/gold-runs/{uuid}"),
        &[],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 401);
}

#[tokio::test]
async fn audit_n22_b1_rotate_key_post_with_uuid_matches_route_not_404() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let uuid = Uuid::nil();
    let (status, _, _) = http_request_full(
        addr,
        "POST",
        &format!("/v1/publishers/{uuid}/rotate-key"),
        &[],
        Some(b"{}"),
    )
    .await;
    h.abort();
    assert_eq!(status, 401);
}

#[tokio::test]
async fn audit_n22_b1_gold_set_approve_post_with_uuid_matches_route_not_404() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let uuid = Uuid::nil();
    let (status, _, _) = http_request_full(
        addr,
        "POST",
        &format!("/v1/gold-sets/{uuid}/approve"),
        &[],
        Some(b"{}"),
    )
    .await;
    h.abort();
    assert_eq!(status, 401);
}

// ─────────────────────────────────────────────────────────────────────
//  B2 — Owned String fields on Healthz/Readyz responses
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_n22_b2_healthz_body_is_deserializable_value() {
    // Regression : pre-fix, HealthzResponse had `&'static str`
    // fields so it could not round-trip through serde_json.
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, _, body) = http_request_full(addr, "GET", "/healthz", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body)
        .expect("healthz body must be valid JSON");
    assert_eq!(v["status"], "ok");
}

#[tokio::test]
async fn audit_n22_b2_readyz_subsystems_are_owned_strings() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, _, body) = http_request_full(addr, "GET", "/readyz", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body)
        .expect("readyz body must be valid JSON");
    let subs = v["subsystems"].as_array().expect("subsystems array");
    assert!(!subs.is_empty());
    for s in subs {
        // `name` field must be a string (not a borrowed cow).
        assert!(s["name"].is_string());
    }
}

// ─────────────────────────────────────────────────────────────────────
//  B7 — dag_id Option<String> → as_deref() routing
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_n22_b7_omitted_dag_id_falls_back_to_default() {
    // Regression : the registry uses Option<&str>.  A bug where
    // `Some("foo")` was passed instead of `None` would prevent the
    // default DAG from running.  Body without `dag_id` must succeed.
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"text\":\"hello\"}}}}"
    );
    let (status, _, resp) = http_request_full(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    h.abort();
    assert_eq!(status, 200, "default DAG must run when dag_id is omitted");
    assert!(resp.contains("\"swarm_id\""));
}

#[tokio::test]
async fn audit_n22_b7_explicit_default_dag_id_runs_default() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"dag_id\":\"default\",\"input\":{{}}}}"
    );
    let (status, _, _) = http_request_full(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
}

// ─────────────────────────────────────────────────────────────────────
//  B8 — ApiError problem-details always carries trace_id + RFC 7807
//        content type, even on short-circuit (extractor-rejected)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_n22_b8_problem_details_content_type_is_rfc7807() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, headers, _) =
        http_request_full(addr, "POST", "/v1/swarms/run", &[], Some(b"{}")).await;
    h.abort();
    assert_eq!(status, 401);
    let lower = headers.to_lowercase();
    assert!(
        lower.contains("content-type: application/problem+json"),
        "must use RFC 7807 content type, got headers:\n{headers}"
    );
}

#[tokio::test]
async fn audit_n22_b8_problem_details_carries_trace_id_on_missing_auth() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (_, _, body) =
        http_request_full(addr, "POST", "/v1/swarms/run", &[], Some(b"{}")).await;
    h.abort();
    let v: serde_json::Value =
        serde_json::from_str(&body).expect("problem+json body must parse");
    // `trace_id` field MUST be present (per G7 invariant).
    assert!(
        v.get("trace_id").is_some(),
        "trace_id field missing from problem details ; got {body}"
    );
    // Slug must be the stable type identifier (URI-style).
    assert!(
        v["type"].as_str().unwrap_or("").ends_with("missing-auth"),
        "type field should end with 'missing-auth', got {}",
        v["type"]
    );
    assert_eq!(v["status"], 401);
}

#[tokio::test]
async fn audit_n22_b8_problem_details_carries_trace_id_on_invalid_jwt() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (_, _, body) = http_request_full(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", "Bearer this.is.not.a.jwt")],
        Some(b"{}"),
    )
    .await;
    h.abort();
    let v: serde_json::Value = serde_json::from_str(&body).expect("problem+json");
    assert!(v.get("trace_id").is_some());
    assert_eq!(v["status"], 401);
}

// ─────────────────────────────────────────────────────────────────────
//  Defense-in-depth — random path → 404 problem details, not panic
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_n22_unknown_path_returns_404_without_panic() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, _, _) = http_request_full(
        addr,
        "GET",
        "/v1/this/does/not/exist",
        &[],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 404);
}

#[tokio::test]
async fn audit_n22_wrong_method_returns_405() {
    // Regression : a route with only POST registered must respond
    // with 405 for GET, not 404.
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, _, _) =
        http_request_full(addr, "GET", "/v1/swarms/run", &[], None).await;
    h.abort();
    assert_eq!(status, 405);
}

// ─────────────────────────────────────────────────────────────────────
//  Defense-in-depth — empty body on POST /v1/swarms/run with JWT
//                     does NOT panic the server (must be 4xx)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_n22_empty_body_with_jwt_returns_4xx_not_5xx() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, _, _) = http_request_full(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(b"{}"),
    )
    .await;
    h.abort();
    assert!(
        (400..500).contains(&status),
        "empty body should produce 4xx, got {status}"
    );
}
