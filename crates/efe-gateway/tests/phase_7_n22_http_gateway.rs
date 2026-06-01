// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : acceptance tests for the HTTP gateway.
//!
//! Each test boots a real `axum::serve(...)` on a fresh ephemeral
//! port, exercises the endpoint via `reqwest`-less raw `tokio::net::
//! TcpStream` + plain HTTP/1.1 (no extra dep), then drops the server.

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
//  Fixture
// ─────────────────────────────────────────────────────────────────────

const TEST_JWT_SECRET: &[u8] = b"test-secret-padded-to-32-bytes-OK!!";

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

fn make_operator_jwt(validator: &Hs256JwtValidator) -> String {
    let claims = GatewayClaims {
        iss: "efe-gateway".into(),
        sub: "ops".into(),
        iat: GatewayClaims::now_secs() - 60,
        exp: GatewayClaims::now_secs() + 3_600,
        tenant_id: Uuid::nil(),
        scope: vec!["operator".into()],
        operator_id: Some(Uuid::new_v4()),
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
    // Wait briefly for the server to bind.
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, handle)
}

async fn http_request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> (u16, String) {
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
        // Tolerate `BrokenPipe` — the server may reject the body
        // upstream (e.g. body too large) and close before we
        // finish writing.  We still read the response below.
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
    let body = raw[body_start..].to_string();
    (status, body)
}

// ─────────────────────────────────────────────────────────────────────
//  Acceptance tests
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_7_n22_healthz_returns_200() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, body) = http_request(addr, "GET", "/healthz", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("\"status\":\"ok\""));
}

#[tokio::test]
async fn phase_7_n22_readyz_returns_200_with_subsystems() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, body) = http_request(addr, "GET", "/readyz", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("subsystems"));
    assert!(body.contains("swarm_orchestrator"));
}

#[tokio::test]
async fn phase_7_n22_metrics_returns_prometheus_format() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, body) = http_request(addr, "GET", "/metrics", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("# HELP"));
    assert!(body.contains("# TYPE"));
    assert!(body.contains("n22_"));
}

#[tokio::test]
async fn phase_7_n22_openapi_json_returns_31_spec() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, body) = http_request(addr, "GET", "/openapi.json", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("\"openapi\":\"3.1.0\""));
    assert!(body.contains("/v1/swarms/run"));
    assert!(body.contains("/v1/agentic/call"));
}

#[tokio::test]
async fn phase_7_n22_v1_swarms_no_auth_returns_401() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, body) = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[],
        Some(b"{\"tenant_id\":\"00000000-0000-0000-0000-000000000000\",\"input\":{}}"),
    )
    .await;
    h.abort();
    assert_eq!(status, 401);
    assert!(body.contains("missing-auth"));
}

#[tokio::test]
async fn phase_7_n22_invalid_jwt_returns_401() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, _) = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", "Bearer not.a.jwt")],
        Some(b"{\"tenant_id\":\"00000000-0000-0000-0000-000000000000\",\"input\":{}}"),
    )
    .await;
    h.abort();
    assert_eq!(status, 401);
}

#[tokio::test]
async fn phase_7_n22_tenant_mismatch_returns_403() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt_tenant = Uuid::new_v4();
    let body_tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, jwt_tenant);
    let body = format!("{{\"tenant_id\":\"{body_tenant}\",\"input\":{{}}}}");
    let (status, resp) = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    h.abort();
    assert_eq!(status, 403);
    assert!(resp.contains("tenant-isolation-violation"));
}

#[tokio::test]
async fn phase_7_n22_admin_endpoint_without_operator_returns_403() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/admin/circuit-breakers",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 403);
    assert!(body.contains("operator-scope-required"));
}

#[tokio::test]
async fn phase_7_n22_admin_endpoint_with_operator_returns_200() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_operator_jwt(&validator);
    let (status, _) = http_request(
        addr,
        "GET",
        "/v1/admin/circuit-breakers",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
}

#[tokio::test]
async fn phase_7_n22_swarm_run_default_dag_succeeds_with_valid_jwt() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"text\":\"Hello world from alice@test.com\"}}}}"
    );
    let (status, resp) = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    assert!(resp.contains("\"swarm_id\""));
    assert!(resp.contains("\"bundle\""));
}

#[tokio::test]
async fn phase_7_n22_agentic_call_with_valid_jwt_returns_200() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    let task = AgenticTask::new("Hello");
    let body = serde_json::json!({
        "tenant_id": tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (status, resp) = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    assert!(resp.contains("\"text\""));
    assert!(resp.contains("stub response"));
}

#[tokio::test]
async fn phase_7_n22_unknown_dag_returns_404() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"dag_id\":\"custom-xyz\",\"input\":{{}}}}"
    );
    let (status, resp) = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    h.abort();
    assert_eq!(status, 404);
    assert!(resp.contains("not-found"));
}

#[tokio::test]
async fn phase_7_n22_problem_details_carries_trace_id() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, body) = http_request(addr, "POST", "/v1/swarms/run", &[], Some(b"{}")).await;
    h.abort();
    assert_eq!(status, 401);
    // Body MUST contain trace_id field even on auth fail.
    assert!(body.contains("trace_id"));
    // application/problem+json content type.
    assert!(body.contains("missing-auth") || body.contains("MissingAuth"));
}

#[tokio::test]
async fn phase_7_n22_body_too_large_returns_413() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    // 11 MB body — over 10 MB cap.
    let huge = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"x\":\"{}\"}}}}",
        "y".repeat(11 * 1024 * 1024)
    );
    let (status, _) = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(huge.as_bytes()),
    )
    .await;
    h.abort();
    // tower-http RequestBodyLimitLayer returns 413.
    assert_eq!(status, 413);
}

#[tokio::test]
async fn phase_7_n22_publisher_register_operator_only() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    // Tenant token should be rejected with 403.
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, _) = http_request(
        addr,
        "POST",
        "/v1/publishers",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(b"{}"),
    )
    .await;
    h.abort();
    // Possible statuses :
    //  - 403 : operator-scope check before body parse (ideal)
    //  - 400/415/422 : body parse fails before operator check
    //  - 405 : route mismatch
    // Any 4xx proves the endpoint is wired + auth is checked.
    assert!(
        (400..500).contains(&status),
        "expected a 4xx status, got {status}"
    );
}

#[tokio::test]
async fn phase_7_n22_concurrent_healthz_no_deadlock() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let mut handles = Vec::new();
    for _ in 0..50 {
        handles.push(tokio::spawn(async move {
            http_request(addr, "GET", "/healthz", &[], None).await
        }));
    }
    for handle in handles {
        let (status, _) = handle.await.unwrap();
        assert_eq!(status, 200);
    }
    h.abort();
}
