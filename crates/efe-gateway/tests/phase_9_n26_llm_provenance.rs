// SPDX-License-Identifier: UNLICENSED
//! **Sprint 18 — N26 + G13 + Workflow A + D** : acceptance
//! tests for the wired LlmProvenanceRecord emission + the
//! `GET /v1/llm-provenance` operator endpoint.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use efe_agentic_bridge::{
    AgenticAuditContext, AgenticBridge, AgenticResponse, AgenticTask, BridgeError, ClaudeBridge,
};
use efe_compliance_wiring::WiredComplianceRunner;
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
    DefaultPublisherApi, Ed25519Verifier, InMemoryPublishLedger,
    InMemoryPublisherRegistry, N18Metrics, StubKycProvider, StubSanctionsScreener,
};
use efe_swarm_orchestrator::SwarmOrchestrator;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ─────────────────────────────────────────────────────────────────────
//  Fixture
// ─────────────────────────────────────────────────────────────────────

const TEST_JWT_SECRET: &[u8] = b"test-secret-padded-to-32-bytes-OK!!";

async fn mount_two_turn(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_t1_n26",
            "content": [{
                "type": "tool_use",
                "id": "tu_01",
                "name": "swarm_context",
                "input": {}
            }],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 100, "output_tokens": 8}
        })))
        .up_to_n_times(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_t2_n26",
            "content": [{"type": "text", "text": "N26 wired."}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 250, "output_tokens": 12}
        })))
        .mount(server)
        .await;
}

async fn fresh_collector_and_store(
    model: &str,
) -> (
    Arc<DefaultLlmProvenanceCollector>,
    Arc<dyn LlmProvenanceStore>,
) {
    let store: Arc<dyn LlmProvenanceStore> =
        Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let mut registered = RegisteredModel::hosted(
        ModelIdentity::hosted("anthropic", model, "2023-06-01"),
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

async fn build_state_with_provenance(
    mock_uri: String,
) -> (AppState, Hs256JwtValidator, Arc<dyn LlmProvenanceStore>) {
    let validator = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");
    let validator_inner = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");

    let swarm = Arc::new(SwarmOrchestrator::new());
    let (collector, store) = fresh_collector_and_store("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("test-key", mock_uri)
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let agentic: Arc<dyn AgenticBridge> = Arc::new(bridge);

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
    let gold_api: Arc<dyn efe_gold_test_gate::GoldApi> =
        Arc::new(DefaultGoldApi::new(gold_registry, gate, gold_ledger));

    let state = AppState::builder()
        .with_swarm(swarm)
        .with_agentic(agentic)
        .with_publisher_api(publisher_api)
        .with_gold_api(gold_api)
        .with_jwt_validator(Arc::new(validator))
        .with_metrics(N22GatewayMetrics::new())
        .with_llm_provenance_store(store.clone())
        .build()
        .expect("state");
    (state, validator_inner, store)
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
async fn phase_9_n26_provenance_emitted_on_agentic_call() {
    let mock = MockServer::start().await;
    mount_two_turn(&mock).await;
    let (state, validator, store) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    let mut task = AgenticTask::new("Summarise");
    // Wire a real audit context.
    task.audit_context = Some(AgenticAuditContext {
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "test_op".into(),
        // Note : this tenant_id will be OVERWRITTEN by the handler.
        tenant_id: TenantId::new(),
        agent_id: NanoAgentId::new("test_agent").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "test_template_v1".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    });
    let expected_trace = task.audit_context.as_ref().unwrap().trace_id;
    let body = serde_json::json!({
        "tenant_id": tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (status, _) = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    // Record present in the store under the JWT-claim tenant
    // (G11 overwrite).
    let record = store
        .get_by_trace(expected_trace, TenantId::from_uuid(tenant))
        .await
        .expect("record persisted");
    assert_eq!(record.trace_id, expected_trace);
    assert_eq!(record.model_identity.provider, "anthropic");
    assert_eq!(record.token_usage.canonical_input(), 350); // 100 + 250
    assert_eq!(record.token_usage.canonical_output(), 20); // 8 + 12
}

#[tokio::test]
async fn phase_9_n26_handler_overwrites_audit_context_tenant() {
    let mock = MockServer::start().await;
    mount_two_turn(&mock).await;
    let (state, validator, store) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let jwt_tenant = Uuid::new_v4();
    let attacker_tenant = TenantId::new(); // distinct
    let jwt = make_tenant_jwt(&validator, jwt_tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    let mut task = AgenticTask::new("Forge");
    task.audit_context = Some(AgenticAuditContext {
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "forge".into(),
        // Maliciously try to impersonate a different tenant.
        tenant_id: attacker_tenant,
        agent_id: NanoAgentId::new("attacker").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "forge_v1".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    });
    let expected_trace = task.audit_context.as_ref().unwrap().trace_id;
    let body = serde_json::json!({
        "tenant_id": jwt_tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (status, _) = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    // The record MUST be stored under the JWT tenant, not the
    // attacker-supplied one.
    let record = store
        .get_by_trace(expected_trace, TenantId::from_uuid(jwt_tenant))
        .await
        .expect("record under JWT tenant");
    assert_eq!(record.tenant_id, TenantId::from_uuid(jwt_tenant));
    // The attacker tenant has no such record.
    let attacker_lookup = store.get_by_trace(expected_trace, attacker_tenant).await;
    assert!(
        attacker_lookup.is_err(),
        "attacker tenant must NOT see the record"
    );
}

#[tokio::test]
async fn phase_9_n26_handler_populates_default_audit_context_when_missing() {
    let mock = MockServer::start().await;
    mount_two_turn(&mock).await;
    let (state, validator, store) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    // No audit_context.
    let task = AgenticTask::new("Default");
    let body = serde_json::json!({
        "tenant_id": tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (status, _) = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    // Some record exists for this tenant.  list_by_swarm with a
    // random uuid won't find it, so we use list_by_model.
    let id = ModelIdentity::hosted("anthropic", "claude-sonnet-4-6", "2023-06-01");
    let window = efe_llm_provenance::DateRange {
        from: std::time::SystemTime::UNIX_EPOCH,
        to: std::time::SystemTime::now() + Duration::from_secs(60),
    };
    let rows = store
        .list_by_model(&id, TenantId::from_uuid(tenant), window)
        .await
        .unwrap();
    assert!(
        !rows.is_empty(),
        "the default audit_context path MUST emit at least one record"
    );
    assert_eq!(rows[0].tenant_id, TenantId::from_uuid(tenant));
    assert_eq!(rows[0].agent_id.as_str(), "agentic_proxy");
}

#[tokio::test]
async fn phase_9_n26_llm_provenance_endpoint_without_auth_returns_401() {
    let mock = MockServer::start().await;
    let (state, _, _) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let (status, body) =
        http_request(addr, "GET", "/v1/llm-provenance", &[], None).await;
    h.abort();
    assert_eq!(status, 401);
    assert!(body.contains("missing-auth"));
}

#[tokio::test]
async fn phase_9_n26_llm_provenance_endpoint_without_operator_returns_403() {
    let mock = MockServer::start().await;
    let (state, validator, _) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/llm-provenance",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 403);
    assert!(body.contains("operator-scope-required"));
}

#[tokio::test]
async fn phase_9_n26_llm_provenance_endpoint_with_operator_returns_200() {
    let mock = MockServer::start().await;
    let (state, validator, _) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let jwt = make_operator_jwt(&validator);
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/llm-provenance",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("\"schema\":\"llm_provenance.v1\""));
    assert!(body.contains("\"records\""));
}

#[tokio::test]
async fn phase_9_n26_llm_provenance_endpoint_503_when_store_absent() {
    // Build state WITHOUT a provenance store.
    let validator = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");
    let validator_inner = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");
    let state = AppState::builder()
        .with_swarm(Arc::new(SwarmOrchestrator::new()))
        .with_agentic(Arc::new(StubBridge))
        .with_publisher_api(Arc::new(DefaultPublisherApi {
            registry: Arc::new(InMemoryPublisherRegistry::new()),
            ledger: Arc::new(InMemoryPublishLedger::new([0u8; 32])),
            kyc: Arc::new(StubKycProvider::permissive()),
            sanctions: Arc::new(StubSanctionsScreener::clear()),
            verifier: Arc::new(Ed25519Verifier::new()),
            metrics: N18Metrics::new(),
        }))
        .with_gold_api({
            let gr = Arc::new(InMemoryGoldSetRegistry::new());
            let gl = Arc::new(InMemoryGoldRunLedger::new([0u8; 32]));
            let gg = Arc::new(GoldGate::new(
                gr.clone(),
                Arc::new(InMemoryGoldRunner::echo()),
                Arc::new(DefaultAssertionEngine::new(Arc::new(
                    StubEmbeddingProvider::new(),
                ))),
                gl.clone(),
                N19Metrics::new(),
            ));
            Arc::new(DefaultGoldApi::new(gr, gg, gl))
        })
        .with_jwt_validator(Arc::new(validator))
        .with_metrics(N22GatewayMetrics::new())
        .build()
        .expect("state");
    let (addr, h) = start_server(state).await;
    let jwt = make_operator_jwt(&validator_inner);
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/llm-provenance",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 503);
    assert!(body.contains("llm provenance store not configured"));
}

#[tokio::test]
async fn phase_9_n26_llm_provenance_endpoint_lists_records_after_call() {
    let mock = MockServer::start().await;
    mount_two_turn(&mock).await;
    let (state, validator, _) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    let task = AgenticTask::new("hi");
    let body = serde_json::json!({
        "tenant_id": tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (s1, _) = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    assert_eq!(s1, 200);

    let operator_jwt = make_operator_jwt(&validator);
    let url = format!("/v1/llm-provenance?tenant_id={tenant}");
    let (s2, body) = http_request(
        addr,
        "GET",
        &url,
        &[("Authorization", &format!("Bearer {operator_jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(s2, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let records = v["records"].as_array().unwrap();
    assert!(
        !records.is_empty(),
        "operator endpoint MUST return the record produced by the prior call"
    );
}

#[tokio::test]
async fn phase_9_n26_openapi_lists_llm_provenance_endpoint() {
    let mock = MockServer::start().await;
    let (state, _, _) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let (status, body) =
        http_request(addr, "GET", "/openapi.json", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["paths"]["/v1/llm-provenance"]["get"].is_object());
    assert!(v["components"]["schemas"]["LlmProvenanceListResponse"].is_object());
}

#[tokio::test]
async fn phase_9_n26_token_usage_summed_across_turns() {
    let mock = MockServer::start().await;
    mount_two_turn(&mock).await;
    let (state, validator, store) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    let mut task = AgenticTask::new("Sum tokens");
    let trace = Uuid::new_v4();
    task.audit_context = Some(AgenticAuditContext {
        trace_id: trace,
        swarm_request_id: Uuid::new_v4(),
        op_id: "sum".into(),
        tenant_id: TenantId::from_uuid(tenant),
        agent_id: NanoAgentId::new("sum_agent").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "sum_v1".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    });
    let body = serde_json::json!({
        "tenant_id": tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (status, resp_body) = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    let resp: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
    assert_eq!(resp["input_tokens"], 350);
    assert_eq!(resp["output_tokens"], 20);

    let record = store
        .get_by_trace(trace, TenantId::from_uuid(tenant))
        .await
        .unwrap();
    assert_eq!(record.token_usage.canonical_input(), 350);
    assert_eq!(record.token_usage.canonical_output(), 20);
}

#[tokio::test]
async fn phase_9_n26_record_carries_provider_request_id() {
    let mock = MockServer::start().await;
    mount_two_turn(&mock).await;
    let (state, validator, store) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    let mut task = AgenticTask::new("hi");
    let trace = Uuid::new_v4();
    task.audit_context = Some(AgenticAuditContext {
        trace_id: trace,
        swarm_request_id: Uuid::new_v4(),
        op_id: "x".into(),
        tenant_id: TenantId::from_uuid(tenant),
        agent_id: NanoAgentId::new("x").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "x".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    });
    let body = serde_json::json!({
        "tenant_id": tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let _ = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    let record = store
        .get_by_trace(trace, TenantId::from_uuid(tenant))
        .await
        .unwrap();
    assert_eq!(
        record.provider_request_id.as_deref(),
        Some("msg_t2_n26"),
        "record MUST carry the Anthropic turn-2 response id"
    );
}

#[tokio::test]
async fn phase_9_n26_pricing_table_version_stamped_on_record() {
    let mock = MockServer::start().await;
    mount_two_turn(&mock).await;
    let (state, validator, store) = build_state_with_provenance(mock.uri()).await;
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let bundle = ContextBundle::empty(efe_nano_sdk::SwarmId::new());
    let mut task = AgenticTask::new("hi");
    let trace = Uuid::new_v4();
    task.audit_context = Some(AgenticAuditContext {
        trace_id: trace,
        swarm_request_id: Uuid::new_v4(),
        op_id: "x".into(),
        tenant_id: TenantId::from_uuid(tenant),
        agent_id: NanoAgentId::new("x").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "x".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    });
    let body = serde_json::json!({
        "tenant_id": tenant,
        "bundle": bundle,
        "task": task,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let _ = http_request(
        addr,
        "POST",
        "/v1/agentic/call",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    let record = store
        .get_by_trace(trace, TenantId::from_uuid(tenant))
        .await
        .unwrap();
    assert_eq!(
        record.cost_accounting.prices_table_version,
        efe_agentic_bridge::PRICES_TABLE_VERSION,
        "record's prices_table_version MUST match the baked Anthropic table"
    );
    assert!(record.cost_accounting.cost_cents > 0.0);
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers — stub bridge for the no-store test
// ─────────────────────────────────────────────────────────────────────

struct StubBridge;

#[axum::async_trait]
impl AgenticBridge for StubBridge {
    async fn call_with_context(
        &self,
        _bundle: &ContextBundle,
        _task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        Ok(AgenticResponse {
            text: "stub".into(),
            stop_reason: efe_agentic_bridge::StopReason::EndTurn,
            usage: efe_agentic_bridge::TokenUsage {
                input_tokens: 0,
                output_tokens: 0,
            },
            model_identity: ModelIdentity::hosted("stub", "stub", "v0"),
            cost_microcents: 0,
        })
    }
}

// Silence unused import warnings if reachable through cargo
// magic — keep imports tight.
#[allow(dead_code)]
fn _force_imports() {
    let _ = WiredComplianceRunner::with_defaults;
    let _ = StubEmbeddingProvider::new;
}
