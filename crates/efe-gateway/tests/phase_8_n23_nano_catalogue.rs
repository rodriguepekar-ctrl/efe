// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + G13 + Workflow C** : acceptance tests for
//! the `GET /v1/nano-catalogue` HTTP endpoint and the
//! end-to-end behaviour of the N23-rewritten default DAG.
//!
//! Same harness shape as `phase_7_n22_http_gateway.rs` : raw
//! `tokio::net::TcpStream` + plain HTTP/1.1 against an
//! ephemeral-port `axum::serve` instance.

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
            text: "stub".into(),
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
async fn phase_8_n23_nano_catalogue_without_auth_returns_401() {
    let (state, _) = build_test_state();
    let (addr, h) = start_server(state).await;
    let (status, body) =
        http_request(addr, "GET", "/v1/nano-catalogue", &[], None).await;
    h.abort();
    assert_eq!(status, 401);
    assert!(body.contains("missing-auth"));
}

#[tokio::test]
async fn phase_8_n23_nano_catalogue_with_valid_jwt_returns_200() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/nano-catalogue",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("\"version\":\"1.0\""));
    assert!(body.contains("\"entries\""));
    assert!(body.contains("\"builtin_canonical_hash\""));
}

#[tokio::test]
async fn phase_8_n23_nano_catalogue_returns_six_entries() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/nano-catalogue",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
    let entries = v["entries"].as_array().expect("entries array");
    assert_eq!(
        entries.len(),
        6,
        "expected 6 built-in pre-analysis entries, got {}",
        entries.len()
    );
}

#[tokio::test]
async fn phase_8_n23_nano_catalogue_entries_carry_expected_ids() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/nano-catalogue",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let entries = v["entries"].as_array().unwrap();
    let ids: Vec<&str> = entries.iter().map(|e| e["id"].as_str().unwrap()).collect();
    for expected in [
        "ner_extractor",
        "language_detector",
        "schema_validator",
        "sentiment_classifier",
        "pii_redactor",
        "summary_writer",
    ] {
        assert!(
            ids.contains(&expected),
            "missing catalogue id {expected} ; got {ids:?}"
        );
    }
}

#[tokio::test]
async fn phase_8_n23_catalogue_canonical_hash_stable_across_calls() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let mut hashes = Vec::new();
    for _ in 0..3 {
        let (status, body) = http_request(
            addr,
            "GET",
            "/v1/nano-catalogue",
            &[("Authorization", &format!("Bearer {jwt}"))],
            None,
        )
        .await;
        assert_eq!(status, 200);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        hashes.push(v["builtin_canonical_hash"].as_str().unwrap().to_string());
    }
    h.abort();
    let first = &hashes[0];
    assert_eq!(first.len(), 64);
    for h in &hashes[1..] {
        assert_eq!(h, first, "canonical hash drifted between calls");
    }
}

#[tokio::test]
async fn phase_8_n23_default_dag_still_completes_via_adapter() {
    // After rewriting dag_registry on top of NanoToSwarm, the
    // default DAG must still complete a swarm run end-to-end.
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"text\":\"hello world contact alice@x.co\"}}}}"
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
    assert_eq!(status, 200, "default DAG must still succeed end-to-end via adapter ; body: {resp}");
    assert!(resp.contains("\"swarm_id\""));
}

#[tokio::test]
async fn phase_8_n23_nano_catalogue_tenant_id_surfaces_from_jwt() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/nano-catalogue",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v["tenant_id"].as_str().unwrap_or(""),
        tenant.to_string(),
        "tenant_id field must reflect the JWT tenant"
    );
}

#[tokio::test]
async fn phase_8_n23_nano_catalogue_categories_match_purpose() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (_, body) = http_request(
        addr,
        "GET",
        "/v1/nano-catalogue",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let entries = v["entries"].as_array().unwrap();
    let by_id: std::collections::HashMap<&str, &str> = entries
        .iter()
        .map(|e| (e["id"].as_str().unwrap(), e["category"].as_str().unwrap()))
        .collect();
    assert_eq!(by_id["ner_extractor"], "entity_recognition");
    assert_eq!(by_id["language_detector"], "classification");
    assert_eq!(by_id["schema_validator"], "validation");
    assert_eq!(by_id["sentiment_classifier"], "classification");
    assert_eq!(by_id["pii_redactor"], "transformation");
    assert_eq!(by_id["summary_writer"], "summarization");
}

#[tokio::test]
async fn phase_8_n23_nano_catalogue_pii_redactor_is_audit_tier() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (_, body) = http_request(
        addr,
        "GET",
        "/v1/nano-catalogue",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let entries = v["entries"].as_array().unwrap();
    let pii = entries.iter().find(|e| e["id"] == "pii_redactor").unwrap();
    assert_eq!(pii["stability_tier"], "tier_audit");
    assert_eq!(pii["compliance"]["dpia_required"], true);
}

#[tokio::test]
async fn phase_8_n23_nano_catalogue_modes_correctly_set() {
    let (state, validator) = build_test_state();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (_, body) = http_request(
        addr,
        "GET",
        "/v1/nano-catalogue",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let entries = v["entries"].as_array().unwrap();
    // NER + language + schema + PII : Mode A pure-rust.
    for id in ["ner_extractor", "language_detector", "schema_validator", "pii_redactor"] {
        let e = entries.iter().find(|e| e["id"] == id).unwrap();
        assert_eq!(
            e["execution_mode"]["mode"], "pure_rust",
            "{id} should be Mode A"
        );
    }
    // Sentiment : Mode B local_model.
    let sentiment = entries.iter().find(|e| e["id"] == "sentiment_classifier").unwrap();
    assert_eq!(sentiment["execution_mode"]["mode"], "local_model");
    // Summary : Mode C llm_augmented.
    let summary = entries.iter().find(|e| e["id"] == "summary_writer").unwrap();
    assert_eq!(summary["execution_mode"]["mode"], "llm_augmented");
}
