// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : gateway-side E2E acceptance tests for
//! the `POST /v1/agentic/call-stream` SSE endpoint.
//!
//! Mirrors the N26 phase test pattern : boot a real axum router
//! (with a wiremock-backed Anthropic stub), hit the streaming
//! endpoint via a raw TCP socket, parse the SSE response.
//!
//! Covered scenarios :
//! 1. Happy path : tool_use turn 1 + final text turn 2 → 3+ delta events + 1 done
//! 2. Kill-switch : EFE_BRIDGE_STREAM_ENABLED=0 → 404
//! 3. JWT auth required (no Bearer → 401)
//! 4. Tenant scoping : JWT-tenant != body-tenant → 403
//! 5. OpenAPI documents the route

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use efe_agentic_bridge::{
    AgenticAuditContext, AgenticBridge, AgenticTask, ClaudeBridge, ExponentialBackoff,
    StreamConfig,
};
use efe_gateway::{
    auth::Hs256JwtValidator, build_router, state::AppState, GatewayClaims, N22GatewayMetrics,
};
use efe_gold_test_gate::{
    DefaultAssertionEngine, DefaultGoldApi, GoldGate, InMemoryGoldRunLedger,
    InMemoryGoldRunner, InMemoryGoldSetRegistry, N19Metrics, StubEmbeddingProvider,
};
use efe_llm_provenance::{
    DefaultLlmProvenanceCollector, InMemoryLlmProvenanceStore, InMemoryModelRegistry,
    LlmProvenanceStore, ModelRegistry, RegisteredModel,
};
use efe_nano_sdk::{
    ContextBundle, ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy, TenantId,
};
use efe_publisher_identity::{
    DefaultPublisherApi, Ed25519Verifier, InMemoryPublishLedger, InMemoryPublisherRegistry,
    N18Metrics, StubKycProvider, StubSanctionsScreener,
};
use efe_swarm_orchestrator::SwarmOrchestrator;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_JWT_SECRET: &[u8] = b"test-secret-padded-to-32-bytes-OK!!";

// ─── SSE body fixtures ──────────────────────────────────────────────

fn sse_body(events: Vec<String>) -> Vec<u8> {
    let mut buf = Vec::new();
    for ev in events {
        buf.extend_from_slice(ev.as_bytes());
        buf.extend_from_slice(b"\n\n");
    }
    buf
}

fn sse_turn1_tool_use() -> Vec<u8> {
    sse_body(vec![
        r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_t1","model":"claude-sonnet-4-6","usage":{"input_tokens":50,"output_tokens":1}}}"#.into(),
        r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tu_1","name":"swarm_context","input":{}}}"#.into(),
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#.into(),
        r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}"#.into(),
        r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":10}}"#.into(),
        r#"event: message_stop
data: {"type":"message_stop"}"#.into(),
    ])
}

fn sse_turn2_text() -> Vec<u8> {
    sse_body(vec![
        r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_t2","model":"claude-sonnet-4-6","usage":{"input_tokens":120,"output_tokens":1}}}"#.into(),
        r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#.into(),
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello, "}}"#.into(),
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"world!"}}"#.into(),
        r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}"#.into(),
        r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":7}}"#.into(),
        r#"event: message_stop
data: {"type":"message_stop"}"#.into(),
    ])
}

async fn mount_two_turn_streamed(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse_turn1_tool_use(), "text/event-stream"),
        )
        .up_to_n_times(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse_turn2_text(), "text/event-stream"),
        )
        .mount(server)
        .await;
}

// ─── Fixture builder ────────────────────────────────────────────────

async fn fresh_collector_and_store() -> (
    Arc<DefaultLlmProvenanceCollector>,
    Arc<dyn LlmProvenanceStore>,
) {
    let store: Arc<dyn LlmProvenanceStore> =
        Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let mut registered = RegisteredModel::hosted(
        ModelIdentity::hosted("anthropic", "claude-sonnet-4-6", "2023-06-01"),
        0.0,
        0.0,
        1,
    );
    registered.dev_skip_hash_check = true;
    registry.register(registered).await.unwrap();
    let collector = DefaultLlmProvenanceCollector::new(
        store.clone(),
        registry as Arc<dyn ModelRegistry>,
    );
    (collector, store)
}

async fn build_state(mock_uri: String) -> (AppState, Hs256JwtValidator) {
    build_state_inner(mock_uri, true).await
}

async fn build_state_with_stream_disabled(
    mock_uri: String,
) -> (AppState, Hs256JwtValidator) {
    build_state_inner(mock_uri, false).await
}

async fn build_state_inner(
    mock_uri: String,
    stream_enabled: bool,
) -> (AppState, Hs256JwtValidator) {
    let validator = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");
    let validator_inner = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");

    let swarm = Arc::new(SwarmOrchestrator::new());
    let (collector, store) = fresh_collector_and_store().await;
    let bridge = ClaudeBridge::with_endpoint("test-key", mock_uri)
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector)
        .with_retry_policy(ExponentialBackoff {
            base: Duration::from_millis(10),
            max_per_retry: Duration::from_millis(30),
            jitter_pct: 0.0,
            ..Default::default()
        })
        .with_stream_config(StreamConfig {
            read_timeout: Duration::from_secs(5),
            total_timeout: Duration::from_secs(15),
            connect_timeout: Duration::from_secs(2),
            channel_cap: 16,
        });
    let agentic: Arc<dyn AgenticBridge> = Arc::new(bridge);

    let publisher_api = Arc::new(DefaultPublisherApi {
        registry: Arc::new(InMemoryPublisherRegistry::new()),
        ledger: Arc::new(InMemoryPublishLedger::new([3u8; 32])),
        kyc: Arc::new(StubKycProvider::permissive()),
        sanctions: Arc::new(StubSanctionsScreener::clear()),
        verifier: Arc::new(Ed25519Verifier::new()),
        metrics: N18Metrics::new(),
    });
    let gold_registry = Arc::new(InMemoryGoldSetRegistry::new());
    let gold_ledger = Arc::new(InMemoryGoldRunLedger::new([4u8; 32]));
    let gate = Arc::new(GoldGate::new(
        gold_registry.clone(),
        Arc::new(InMemoryGoldRunner::echo()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        gold_ledger.clone(),
        N19Metrics::new(),
    ));
    let gold_api: Arc<dyn efe_gold_test_gate::GoldApi> =
        Arc::new(DefaultGoldApi::new(gold_registry, gate, gold_ledger));

    let state = AppState::builder()
        .with_swarm(swarm)
        .with_agentic(agentic)
        .with_publisher_api(publisher_api)
        .with_gold_api(gold_api)
        .with_jwt_validator(Arc::new(validator))
        .with_metrics(N22GatewayMetrics::new())
        .with_llm_provenance_store(store)
        .with_stream_endpoint_enabled(stream_enabled)
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

/// **N27** : send a POST with body + bearer JWT, read the FULL
/// raw response (status line + headers + body) within `timeout`.
async fn http_post_sse(
    addr: SocketAddr,
    path: &str,
    bearer: Option<&str>,
    body: &[u8],
    timeout: Duration,
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut req = format!("POST {path} HTTP/1.1\r\nHost: localhost\r\n");
    if let Some(jwt) = bearer {
        req.push_str(&format!("Authorization: Bearer {jwt}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    req.push_str("Content-Type: application/json\r\n");
    req.push_str("Accept: text/event-stream\r\n");
    req.push_str("Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();

    let mut buf = Vec::new();
    let _ = tokio::time::timeout(timeout, stream.read_to_end(&mut buf)).await;
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

fn audit_ctx_for(tenant: TenantId) -> AgenticAuditContext {
    AgenticAuditContext {
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "test_n27_stream".into(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("test_agent").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "test_template_v1".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    }
}

// ═════════════════════════════════════════════════════════════════════
//  Phase tests
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn phase_9_n27_gateway_sse_happy_path_two_turn() {
    // Make sure the kill-switch isn't lingering from another test.
    std::env::remove_var("EFE_BRIDGE_STREAM_ENABLED");

    let mock = MockServer::start().await;
    mount_two_turn_streamed(&mock).await;
    let (state, validator) = build_state(mock.uri()).await;
    let (addr, _h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);

    let mut task = AgenticTask::new("greet");
    task.audit_context = Some(audit_ctx_for(TenantId::new()));
    let body = serde_json::to_vec(&serde_json::json!({
        "tenant_id": tenant,
        "bundle": ContextBundle::empty(efe_nano_sdk::SwarmId::new()),
        "task": task,
    }))
    .unwrap();

    let (status, body_str) = http_post_sse(
        addr,
        "/v1/agentic/call-stream",
        Some(&jwt),
        &body,
        Duration::from_secs(8),
    )
    .await;

    assert_eq!(status, 200, "expected HTTP 200, got {status} : {body_str}");
    // Body should be SSE-formatted.  We expect at least :
    //  - 1 delta event with "Hello, "
    //  - 1 delta event with "world!"
    //  - 1 done event with stop_reason=end_turn
    assert!(
        body_str.contains("event: delta"),
        "expected delta events in SSE body : {body_str}"
    );
    assert!(
        body_str.contains("Hello, "),
        "expected 'Hello, ' delta in : {body_str}"
    );
    assert!(
        body_str.contains("world!"),
        "expected 'world!' delta in : {body_str}"
    );
    assert!(
        body_str.contains("event: done"),
        "expected done event in : {body_str}"
    );
    assert!(
        body_str.contains("end_turn"),
        "expected stop_reason=end_turn in done payload"
    );
    // **N27 + I24** : streaming mode → record was emitted.  We
    // can't directly read the store from here without exposing it
    // ; the buffered handler's existing tests cover that path.
    // For the streaming surface the assertion is the SSE-wire
    // observability above.
}

#[tokio::test]
async fn phase_9_n27_gateway_sse_kill_switch_returns_404() {
    // **N27 + § 17.1 + § 17.5** : AppState::stream_endpoint_enabled = false
    // (set per-instance via the builder ; in production read once at
    // boot from EFE_BRIDGE_STREAM_ENABLED).  Per-instance scoping
    // avoids the env-var-race-across-parallel-tests pitfall.
    let mock = MockServer::start().await;
    mount_two_turn_streamed(&mock).await;
    let (state, validator) =
        build_state_with_stream_disabled(mock.uri()).await;
    let (addr, _h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);

    let body = serde_json::to_vec(&serde_json::json!({
        "tenant_id": tenant,
        "bundle": ContextBundle::empty(efe_nano_sdk::SwarmId::new()),
        "task": AgenticTask::new("x"),
    }))
    .unwrap();

    let (status, _body_str) = http_post_sse(
        addr,
        "/v1/agentic/call-stream",
        Some(&jwt),
        &body,
        Duration::from_secs(3),
    )
    .await;

    assert_eq!(status, 404, "expected 404 with kill-switch on");
}

#[tokio::test]
async fn phase_9_n27_gateway_sse_no_jwt_returns_401() {
    std::env::remove_var("EFE_BRIDGE_STREAM_ENABLED");

    let mock = MockServer::start().await;
    mount_two_turn_streamed(&mock).await;
    let (state, _validator) = build_state(mock.uri()).await;
    let (addr, _h) = start_server(state).await;

    let body = serde_json::to_vec(&serde_json::json!({
        "tenant_id": Uuid::new_v4(),
        "bundle": ContextBundle::empty(efe_nano_sdk::SwarmId::new()),
        "task": AgenticTask::new("x"),
    }))
    .unwrap();
    let (status, _) = http_post_sse(
        addr,
        "/v1/agentic/call-stream",
        None, // no JWT
        &body,
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn phase_9_n27_gateway_sse_cross_tenant_returns_403() {
    std::env::remove_var("EFE_BRIDGE_STREAM_ENABLED");

    let mock = MockServer::start().await;
    mount_two_turn_streamed(&mock).await;
    let (state, validator) = build_state(mock.uri()).await;
    let (addr, _h) = start_server(state).await;
    let jwt_tenant = Uuid::new_v4();
    let body_tenant = Uuid::new_v4(); // mismatch
    let jwt = make_tenant_jwt(&validator, jwt_tenant);

    let body = serde_json::to_vec(&serde_json::json!({
        "tenant_id": body_tenant,
        "bundle": ContextBundle::empty(efe_nano_sdk::SwarmId::new()),
        "task": AgenticTask::new("x"),
    }))
    .unwrap();
    let (status, _) = http_post_sse(
        addr,
        "/v1/agentic/call-stream",
        Some(&jwt),
        &body,
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn phase_9_n27_gateway_openapi_documents_streaming_endpoint() {
    // **N27 audit-pass-4** : the OpenAPI surface must list
    // /v1/agentic/call-stream so client SDKs can codegen the route.
    let mock = MockServer::start().await;
    let (state, _validator) = build_state(mock.uri()).await;
    let (addr, _h) = start_server(state).await;
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req =
        "GET /openapi.json HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    let _ =
        tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut buf))
            .await;
    let raw = String::from_utf8_lossy(&buf).to_string();
    let body_start = raw.find("\r\n\r\n").map(|i| i + 4).unwrap_or(raw.len());
    let body = &raw[body_start..];
    let v: serde_json::Value =
        serde_json::from_str(body).expect("openapi should be valid JSON");
    assert!(
        v["paths"]["/v1/agentic/call-stream"]["post"].is_object(),
        "OpenAPI must document /v1/agentic/call-stream"
    );
    // Confirm the SSE response shape is described.
    let ok_response =
        &v["paths"]["/v1/agentic/call-stream"]["post"]["responses"]["200"];
    assert!(ok_response.is_object());
    assert!(ok_response["content"]["text/event-stream"].is_object());
}
