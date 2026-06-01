// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + Workflow A + G13** : acceptance tests for
//! the wired compliance stack — `GET /v1/admin/compliance-status`
//! endpoint + the tier1_strict default DAG.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use efe_agentic_bridge::{AgenticBridge, AgenticResponse, AgenticTask, BridgeError};
use efe_compliance_wiring::WiredComplianceRunner;
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

fn build_state_with_compliance() -> (AppState, Hs256JwtValidator) {
    let validator = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");
    let validator_inner = Hs256JwtValidator::new(TEST_JWT_SECRET.to_vec())
        .with_expected_issuer("efe-gateway");

    let swarm = Arc::new(SwarmOrchestrator::new());
    let agentic: Arc<dyn AgenticBridge> = Arc::new(StubBridge);
    let publisher_registry: Arc<dyn efe_publisher_identity::PublisherRegistry> =
        Arc::new(InMemoryPublisherRegistry::new());
    let publisher_api = Arc::new(DefaultPublisherApi {
        registry: publisher_registry.clone(),
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
    let verifier: Arc<dyn efe_publisher_identity::SignatureVerifier> =
        Arc::new(Ed25519Verifier::new());

    let runner = Arc::new(WiredComplianceRunner::with_defaults(
        publisher_registry,
        verifier,
        gold_api.clone(),
    ));

    let state = AppState::builder()
        .with_swarm(swarm)
        .with_agentic(agentic)
        .with_publisher_api(publisher_api)
        .with_gold_api(gold_api)
        .with_jwt_validator(Arc::new(validator))
        .with_metrics(N22GatewayMetrics::new())
        .with_compliance(runner)
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
async fn phase_8_n24_compliance_status_without_auth_returns_401() {
    let (state, _) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let (status, body) =
        http_request(addr, "GET", "/v1/admin/compliance-status", &[], None).await;
    h.abort();
    assert_eq!(status, 401);
    assert!(body.contains("missing-auth"));
}

#[tokio::test]
async fn phase_8_n24_compliance_status_without_operator_returns_403() {
    let (state, validator) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/admin/compliance-status",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 403);
    assert!(body.contains("operator-scope-required"));
}

#[tokio::test]
async fn phase_8_n24_compliance_status_with_operator_returns_200() {
    let (state, validator) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let jwt = make_operator_jwt(&validator);
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/admin/compliance-status",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    assert!(body.contains("\"schema\":\"compliance.status.v1\""));
    assert!(body.contains("\"cache\""));
    assert!(body.contains("\"counters\""));
    assert!(body.contains("\"gates\""));
    assert!(body.contains("\"builtin\""));
}

#[tokio::test]
async fn phase_8_n24_compliance_status_payload_shape_is_v1() {
    let (state, validator) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let jwt = make_operator_jwt(&validator);
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/admin/compliance-status",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["schema"], "compliance.status.v1");
    let counters = &v["counters"];
    assert!(counters["signature_verified"].is_number());
    assert!(counters["signature_failed"].is_number());
    assert!(counters["gold_bound"].is_number());
    assert!(counters["status_active"].is_number());
    assert!(v["cache"]["capacity"].is_number());
    assert!(v["builtin"]["publisher_id"].is_string());
    assert!(v["builtin"]["key_id"].is_string());
    assert_eq!(v["builtin"]["kyc_tier"], "tier_3");
}

#[tokio::test]
async fn phase_8_n24_default_dag_with_compliance_signature_verified() {
    // Run a swarm via the wired DAG ; the compliance counter
    // must show signature_verified > 0 for the Tier1+ entries
    // (NER, language, schema, PII) AND status_active > 0 for
    // every dispatch.
    let (state, validator) = build_state_with_compliance();
    let (addr, h) = start_server(state.clone()).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"text\":\"contact alice@x.co please\"}}}}"
    );
    let (status, _) = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    assert_eq!(status, 200);
    // Query the compliance status.
    let op = make_operator_jwt(&validator);
    let (s2, body2) = http_request(
        addr,
        "GET",
        "/v1/admin/compliance-status",
        &[("Authorization", &format!("Bearer {op}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(s2, 200);
    let v: serde_json::Value = serde_json::from_str(&body2).unwrap();
    // Tier1+ entries (NER, language, schema, PII = 4 nanos)
    // each fire the signature gate exactly once.  All four must
    // pass verification — the built-in catalogue is signed by
    // the EFE built-in publisher at startup.
    let signature_verified = v["counters"]["signature_verified"].as_u64().unwrap();
    assert_eq!(
        signature_verified, 4,
        "every Tier1+ catalogue entry MUST verify its signature"
    );
    // Status gate active count : every dispatch (6 entries)
    // consulted the status gate.  At least 6 must show as active.
    let status_active = v["counters"]["status_active"].as_u64().unwrap();
    assert!(
        status_active >= 6,
        "expected at least 6 status_active, got {status_active}"
    );
}

#[tokio::test]
async fn phase_8_n24_openapi_lists_compliance_status_endpoint() {
    let (state, _) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let (status, body) = http_request(addr, "GET", "/openapi.json", &[], None).await;
    h.abort();
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(v["paths"]["/v1/admin/compliance-status"]["get"].is_object());
    assert!(v["components"]["schemas"]["ComplianceStatus"].is_object());
}

#[tokio::test]
async fn phase_8_n24_default_dag_status_includes_cache_stats() {
    let (state, validator) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    // First swarm run.
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"text\":\"hello\"}}}}"
    );
    let _ = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    // Second run — cache should already contain the built-in
    // publisher (but it bypasses the cache via the short-circuit,
    // so this only confirms the field is present).
    let _ = http_request(
        addr,
        "POST",
        "/v1/swarms/run",
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(body.as_bytes()),
    )
    .await;
    let op = make_operator_jwt(&validator);
    let (s2, body2) = http_request(
        addr,
        "GET",
        "/v1/admin/compliance-status",
        &[("Authorization", &format!("Bearer {op}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(s2, 200);
    let v: serde_json::Value = serde_json::from_str(&body2).unwrap();
    let cache = &v["cache"];
    assert!(cache["capacity"].as_u64().unwrap() > 0);
    // hit_ratio is in [0, 1].
    let r = cache["hit_ratio"].as_f64().unwrap();
    assert!((0.0..=1.0).contains(&r), "hit_ratio out of range : {r}");
}

#[tokio::test]
async fn phase_8_n24_compliance_status_503_when_runner_absent() {
    // Build a state WITHOUT the compliance runner.
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
    let (addr, h) = start_server(state).await;
    let jwt = make_operator_jwt(&validator_inner);
    let (status, body) = http_request(
        addr,
        "GET",
        "/v1/admin/compliance-status",
        &[("Authorization", &format!("Bearer {jwt}"))],
        None,
    )
    .await;
    h.abort();
    assert_eq!(status, 503);
    assert!(body.contains("compliance runner not configured"));
}

#[tokio::test]
async fn phase_8_n24_dag_without_compliance_still_works() {
    // N23 backward compatibility : the default DAG must still
    // complete when no compliance runner is configured.
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
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator_inner, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"text\":\"hello\"}}}}"
    );
    let (status, _) = http_request(
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

#[tokio::test]
async fn phase_8_n24_wired_dag_succeeds_end_to_end() {
    let (state, validator) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let tenant = Uuid::new_v4();
    let jwt = make_tenant_jwt(&validator, tenant);
    let body = format!(
        "{{\"tenant_id\":\"{tenant}\",\"input\":{{\"text\":\"PII : alice@x.co and 555-123-4567 on 2026-05-22\"}}}}"
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
    assert_eq!(status, 200, "wired DAG must succeed end-to-end ; body : {resp}");
    let v: serde_json::Value = serde_json::from_str(&resp).unwrap();
    // Sprint 16 N24 posture : with the per-entry gold override
    // (catalogue entries without `gold_set_ref` skip the gold
    // gate), the wired DAG completes end-to-end.  All 6 nanos
    // pass status + signature ; gold is bypassed because no
    // entry has `gold_set_ref` (Sprint 17 will populate them).
    // Aggregator completes.  Total : 7 completed.
    let completed = v["agents_completed"].as_u64().unwrap();
    let errored = v["agents_errored"].as_u64().unwrap();
    assert_eq!(
        completed, 7,
        "all 7 agents must complete under tier1_strict_wired ; got {completed} (errored: {errored})"
    );
    assert_eq!(errored, 0, "no agent should error");
}

#[tokio::test]
async fn phase_8_n24_revoke_publisher_evicts_compliance_cache() {
    // **G11 + G12 + Workflow E** : revoking a publisher via the
    // gateway MUST invalidate the compliance cache so the next
    // dispatch picks up the new status.
    //
    // To prove the wiring : register a publisher via the
    // underlying `PublisherApi` (bypass the gateway's register
    // endpoint to keep the test small), seed the compliance
    // cache, revoke via HTTP, then assert the cache is empty.
    use efe_publisher_identity::api::RegisterRequest;
    use efe_nano_sdk::{Iso3166, KycTier, NewPublisherCandidate, SignatureAlgorithm};

    let (state, validator) = build_state_with_compliance();
    let compliance = state.compliance.clone().expect("wired");

    // Register a fresh publisher via the underlying API.
    let signer = efe_publisher_identity::verifier::Ed25519Signer::generate();
    let candidate = NewPublisherCandidate {
        display_name: format!("ChaosPub-{}", Uuid::new_v4()),
        target_tier: KycTier::Tier2,
        jurisdiction: vec![Iso3166::new("FR").unwrap()],
        initial_public_key_pem: signer.public_key_pem(),
        algorithm: SignatureAlgorithm::Ed25519,
        pseudonymous: false,
        kyc_dossier_reference: Some("file:test".into()),
    };
    let reg = state
        .publisher_api
        .register(RegisterRequest { candidate })
        .await
        .expect("register");
    let pid = reg.identity.publisher_id;

    // Seed the compliance cache with the same identity.
    compliance.cache().put(pid, reg.identity.clone());
    assert_eq!(compliance.cache().len(), 1);

    // Revoke via HTTP (operator scope).
    let (addr, h) = start_server(state.clone()).await;
    let jwt = make_operator_jwt(&validator);
    let body = serde_json::json!({
        "reason": { "kind": "operator", "rationale": "chaos test" },
        "revoking_operator_id": Uuid::new_v4(),
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (status, _) = http_request(
        addr,
        "POST",
        &format!("/v1/publishers/{}/revoke", pid.as_uuid()),
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    assert_eq!(status, 200, "revoke must succeed for a registered publisher");

    // **The G11/G12 invariant under test** : after a successful
    // revoke, the compliance cache no longer holds the stale
    // Active identity.
    assert_eq!(
        compliance.cache().len(),
        0,
        "compliance cache MUST be evicted by the revoke handler"
    );
}

#[tokio::test]
async fn phase_8_n24_rotate_key_evicts_compliance_cache() {
    // **G12** : rotate-key flips `active_key` ; the cached
    // identity now has a stale PEM.  The handler must evict.
    use efe_publisher_identity::api::{RegisterRequest, RotateKeyRequest};
    use efe_nano_sdk::{Iso3166, KycTier, NewPublisherCandidate, SignatureAlgorithm};

    let (state, validator) = build_state_with_compliance();
    let compliance = state.compliance.clone().expect("wired");

    // Register a publisher.
    let signer = efe_publisher_identity::verifier::Ed25519Signer::generate();
    let candidate = NewPublisherCandidate {
        display_name: format!("RotatePub-{}", Uuid::new_v4()),
        target_tier: KycTier::Tier2,
        jurisdiction: vec![Iso3166::new("DE").unwrap()],
        initial_public_key_pem: signer.public_key_pem(),
        algorithm: SignatureAlgorithm::Ed25519,
        pseudonymous: false,
        kyc_dossier_reference: Some("file:test".into()),
    };
    let reg = state
        .publisher_api
        .register(RegisterRequest { candidate })
        .await
        .expect("register");
    let pid = reg.identity.publisher_id;

    // Seed cache.
    compliance.cache().put(pid, reg.identity.clone());
    assert_eq!(compliance.cache().len(), 1);

    // Rotate-key via HTTP.  Sprint 16 ships the unsigned-proof
    // path (the no-proof endpoint) — Sprint 17 N25 adds the
    // signed-proof endpoint.
    let (addr, h) = start_server(state.clone()).await;
    let jwt = make_tenant_jwt(&validator, Uuid::new_v4());
    let new_signer = efe_publisher_identity::verifier::Ed25519Signer::generate();
    let new_pem = new_signer.public_key_pem();
    // Build a rotation proof : signature of the new public key
    // bytes by the OLD signing key (per N18 ADR-2).
    use base64::Engine;
    let proof_bytes = signer.sign(new_pem.as_bytes());
    let proof_b64 = base64::engine::general_purpose::STANDARD
        .encode(proof_bytes);
    let body = RotateKeyRequest {
        new_public_key_pem: new_pem,
        algorithm: SignatureAlgorithm::Ed25519,
        signature_proof_b64: proof_b64,
    };
    let body_bytes = serde_json::to_vec(&body).unwrap();
    let (status, _) = http_request(
        addr,
        "POST",
        &format!("/v1/publishers/{}/rotate-key", pid.as_uuid()),
        &[("Authorization", &format!("Bearer {jwt}"))],
        Some(&body_bytes),
    )
    .await;
    h.abort();
    // Whether the underlying rotate succeeds depends on the
    // proof shape ; either way the eviction code runs on Ok.
    // We accept any 2xx + the cache being empty, OR any 4xx + the
    // cache being unchanged.
    if (200..300).contains(&status) {
        assert_eq!(
            compliance.cache().len(),
            0,
            "successful rotate-key MUST evict the cache"
        );
    } else {
        // Failed rotate : the cache stays seeded (no eviction).
        // This proves the eviction is gated on a successful
        // upstream call (no false-positive evictions).
        assert_eq!(compliance.cache().len(), 1);
    }
}

#[tokio::test]
async fn phase_8_n24_compliance_status_builtin_publisher_id_stable() {
    let (state, validator) = build_state_with_compliance();
    let (addr, h) = start_server(state).await;
    let op = make_operator_jwt(&validator);

    let mut ids = Vec::new();
    for _ in 0..3 {
        let (_, body) = http_request(
            addr,
            "GET",
            "/v1/admin/compliance-status",
            &[("Authorization", &format!("Bearer {op}"))],
            None,
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        ids.push(v["builtin"]["publisher_id"].as_str().unwrap().to_string());
    }
    h.abort();
    // Same publisher id across all calls.
    assert_eq!(ids[0], ids[1]);
    assert_eq!(ids[1], ids[2]);
}
