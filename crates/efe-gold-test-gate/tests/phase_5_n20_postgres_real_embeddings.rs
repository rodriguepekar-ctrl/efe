// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : end-to-end acceptance tests for the
//! production-readiness layer.
//!
//! Tests grouped per commitment :
//! - **G4 / G5** OpenAI + Cohere providers via `wiremock`
//! - **G7** MultiProviderEmbeddingClient circuit-breaker failover
//! - **G8 / G9** DriftSweepCron tick + idempotency
//! - **G10** BypassSignatureVerifier Ed25519 reject empty sig
//! - **G11** N18 `AgentPromotedToTier1` ledger variant
//!
//! Tests for G1-G3 (Postgres) + G6 (local sentence-transformers) +
//! G13/G14 (Firecracker + eBPF) are gated behind feature flags in
//! Sprint 13 — they require infrastructure (Postgres docker, CAP_BPF,
//! candle weights) that doesn't fit the unit-test harness.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use efe_gold_test_gate::{
    canonical_bypass_message, BypassSignatureVerifier, CircuitBreakerState,
    CohereEmbeddingProvider, DriftSweepCron, DriftSweepCronConfig, DriftSweepState,
    DriftSweepStateEntry, EmbeddingProvider, EmbeddingProviderError, InMemoryDriftSweepState,
    InMemoryOperatorKeyResolver, MultiProviderEmbeddingClient, NamedProvider,
    OpenAiEmbeddingProvider, OperatorKey, StubEmbeddingProvider,
};
use efe_nano_sdk::{
    BypassCertificate, GoldRunId, ModelIdentity, NanoAgentId, NanoAgentVersion,
    OperatorSignature, SignatureAlgorithm,
};
use efe_publisher_identity::{
    Ed25519Signer, Ed25519Verifier, InMemoryPublishLedger, PublishLedger,
    PublishLedgerEntry,
};
use std::sync::atomic::Ordering;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ─────────────────────────────────────────────────────────────────────
//  G4 — OpenAI provider returns embedding via real reqwest
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_openai_provider_returns_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{"embedding": vec![0.1_f64; 1536]}],
            "model": "text-embedding-3-small"
        })))
        .mount(&server)
        .await;
    let p = OpenAiEmbeddingProvider::with_endpoint("test", server.uri());
    let model = ModelIdentity::hosted("openai", "text-embedding-3-small", "1");
    let e = p.embed(&model, "hello").await.unwrap();
    assert_eq!(e.vector.len(), 1536);
}

// ─────────────────────────────────────────────────────────────────────
//  G5 — Cohere provider returns embedding
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_cohere_provider_returns_embedding() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embed"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "embeddings": [vec![0.2_f64; 1024]]
        })))
        .mount(&server)
        .await;
    let p = CohereEmbeddingProvider::with_endpoint("test", server.uri());
    let model = ModelIdentity::hosted("cohere", "embed-multilingual-v3.0", "1");
    let e = p.embed(&model, "hello").await.unwrap();
    assert_eq!(e.vector.len(), 1024);
}

// ─────────────────────────────────────────────────────────────────────
//  G7 — Multi-provider failover under rate limit
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_multi_provider_failover_under_rate_limit() {
    // Primary returns 429 ; secondary stub succeeds.
    let openai_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&openai_server)
        .await;
    let primary = Arc::new(OpenAiEmbeddingProvider::with_endpoint(
        "key",
        openai_server.uri(),
    ));
    let secondary = Arc::new(StubEmbeddingProvider::new());
    let client = MultiProviderEmbeddingClient::new(vec![
        NamedProvider::new("openai", primary),
        NamedProvider::new("stub", secondary),
    ]);
    let model = ModelIdentity::hosted("openai", "text-embedding-3-small", "1");
    let e = client.embed(&model, "hello").await.unwrap();
    assert!(!e.vector.is_empty());
}

// ─────────────────────────────────────────────────────────────────────
//  G7 — Circuit breaker opens after 3 consecutive failures
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_circuit_breaker_opens_after_three_failures() {
    let openai_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&openai_server)
        .await;
    let primary = Arc::new(OpenAiEmbeddingProvider::with_endpoint(
        "key",
        openai_server.uri(),
    ));
    let secondary = Arc::new(StubEmbeddingProvider::new());
    let client = MultiProviderEmbeddingClient::with_breaker_thresholds(
        vec![
            NamedProvider::new("openai", primary),
            NamedProvider::new("stub", secondary),
        ],
        3,
        30,
    );
    let model = ModelIdentity::hosted("openai", "text-embedding-3-small", "1");
    // Three calls : each triggers OpenAI fail + stub success.  After
    // 3 OpenAI failures, the breaker for "openai" opens.
    for _ in 0..3 {
        let _ = client.embed(&model, "x").await.unwrap();
    }
    assert_eq!(
        client.breaker_state("openai"),
        Some(CircuitBreakerState::Open)
    );
}

// ─────────────────────────────────────────────────────────────────────
//  G8 — DriftSweepCron fires on notify and processes stale entries
// ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn phase_5_n20_drift_sweep_cron_fires_and_processes_stale() {
    let state = Arc::new(InMemoryDriftSweepState::new());
    let now = SystemTime::now();
    state
        .upsert(DriftSweepStateEntry {
            agent_id: NanoAgentId::new("stale_agent").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            last_run_at: now - Duration::from_secs(86_400),
            last_gold_run_id: GoldRunId::new(),
            last_outcome: "passed".into(),
        })
        .await
        .unwrap();
    let cron = DriftSweepCron::new(
        state.clone() as Arc<dyn DriftSweepState>,
        DriftSweepCronConfig {
            tick_interval: Duration::from_secs(86_400),
            drift_threshold: Duration::from_secs(60),
        },
    );
    let observed = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = observed.clone();
    let handle = cron.clone().start(move |_entry| {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    });
    tokio::task::yield_now().await;
    cron.notify_tick();
    tokio::time::sleep(Duration::from_millis(10)).await;
    handle.abort();
    assert!(observed.load(Ordering::Relaxed) >= 1);
}

// ─────────────────────────────────────────────────────────────────────
//  G9 — DriftSweepCron idempotent across restart
// ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn phase_5_n20_drift_sweep_idempotent_after_state_refresh() {
    let state = Arc::new(InMemoryDriftSweepState::new());
    let now = SystemTime::now();
    state
        .upsert(DriftSweepStateEntry {
            agent_id: NanoAgentId::new("agent").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            last_run_at: now - Duration::from_secs(86_400),
            last_gold_run_id: GoldRunId::new(),
            last_outcome: "passed".into(),
        })
        .await
        .unwrap();
    let cron = DriftSweepCron::new(
        state.clone() as Arc<dyn DriftSweepState>,
        DriftSweepCronConfig {
            tick_interval: Duration::from_secs(86_400),
            drift_threshold: Duration::from_secs(60),
        },
    );
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let cc = counter.clone();
    let state_for_cb = state.clone();
    let handle = cron.clone().start(move |entry| {
        let cc = cc.clone();
        let st = state_for_cb.clone();
        async move {
            cc.fetch_add(1, Ordering::Relaxed);
            // Simulate gate completion : refresh last_run_at.
            let mut fresh = entry;
            fresh.last_run_at = SystemTime::now();
            let _ = st.upsert(fresh).await;
        }
    });
    cron.notify_tick();
    tokio::time::sleep(Duration::from_millis(10)).await;
    // Second notify after the entry was refreshed.  No new
    // processing expected.
    cron.notify_tick();
    tokio::time::sleep(Duration::from_millis(10)).await;
    handle.abort();
    assert_eq!(counter.load(Ordering::Relaxed), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  G10 — BypassSignatureVerifier rejects empty signature bytes
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_bypass_empty_signature_rejected() {
    // **Post-audit fix B2** : verifier enforces ≥ 2 approvers
    // floor.  We provide 2 approvers — the first has an empty
    // signature (the P12 / Incident I-2026-07-25 vector) so the
    // floor check passes and the signature check rejects at idx 0.
    let resolver = Arc::new(InMemoryOperatorKeyResolver::new());
    let op_bad = Uuid::from_u128(1);
    let op_good = Uuid::from_u128(2);
    let s_good = Ed25519Signer::from_seed([2u8; 32]);
    resolver.insert(OperatorKey {
        operator_id: op_bad,
        algorithm: SignatureAlgorithm::Ed25519,
        public_key_pem: Ed25519Signer::from_seed([1u8; 32]).public_key_pem(),
    });
    resolver.insert(OperatorKey {
        operator_id: op_good,
        algorithm: SignatureAlgorithm::Ed25519,
        public_key_pem: s_good.public_key_pem(),
    });
    let verifier = BypassSignatureVerifier::new(
        Arc::new(Ed25519Verifier::new()),
        resolver.clone(),
    );
    let run_id = GoldRunId::new();
    let bypass_id = Uuid::new_v4();
    let mut cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    cert.approvers.push(OperatorSignature {
        operator_id: op_bad,
        signed_at: SystemTime::now(),
        ticket_id: "INC-1".into(),
        signature: vec![], // ← P12 / Incident I-2026-07-25
    });
    // Pad with a valid signer so the floor check passes and we hit
    // the signature check.
    let canonical = canonical_bypass_message(run_id, bypass_id, &cert);
    cert.approvers.push(OperatorSignature {
        operator_id: op_good,
        signed_at: SystemTime::now(),
        ticket_id: "INC-1".into(),
        signature: s_good.sign(&canonical).to_vec(),
    });
    let err = verifier.verify(&cert, run_id, bypass_id).await.unwrap_err();
    match err {
        efe_nano_sdk::GoldGateError::BypassSignatureInvalid { approver_index } => {
            assert_eq!(approver_index, 0);
        }
        other => panic!("expected BypassSignatureInvalid at idx 0, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  G10 — Two valid Ed25519 signatures pass verification
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_bypass_two_valid_signatures_pass() {
    let resolver = Arc::new(InMemoryOperatorKeyResolver::new());
    let s1 = Ed25519Signer::from_seed([1u8; 32]);
    let s2 = Ed25519Signer::from_seed([2u8; 32]);
    let op1 = Uuid::from_u128(1);
    let op2 = Uuid::from_u128(2);
    resolver.insert(OperatorKey {
        operator_id: op1,
        algorithm: SignatureAlgorithm::Ed25519,
        public_key_pem: s1.public_key_pem(),
    });
    resolver.insert(OperatorKey {
        operator_id: op2,
        algorithm: SignatureAlgorithm::Ed25519,
        public_key_pem: s2.public_key_pem(),
    });
    let verifier = BypassSignatureVerifier::new(
        Arc::new(Ed25519Verifier::new()),
        resolver,
    );
    let run_id = GoldRunId::new();
    let bypass_id = Uuid::new_v4();
    let mut cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-9117".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let canonical = canonical_bypass_message(run_id, bypass_id, &cert);
    cert.approvers.push(OperatorSignature {
        operator_id: op1,
        signed_at: SystemTime::now(),
        ticket_id: cert.ticket_id.clone(),
        signature: s1.sign(&canonical).to_vec(),
    });
    cert.approvers.push(OperatorSignature {
        operator_id: op2,
        signed_at: SystemTime::now(),
        ticket_id: cert.ticket_id.clone(),
        signature: s2.sign(&canonical).to_vec(),
    });
    verifier.verify(&cert, run_id, bypass_id).await.unwrap();
}

// ─────────────────────────────────────────────────────────────────────
//  G11 — N18 ledger AgentPromotedToTier1 variant exists + works
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_n18_agent_promoted_to_tier_1_ledger_entry() {
    let ledger = InMemoryPublishLedger::new([7u8; 32]);
    let publisher_id = efe_nano_sdk::PublisherId::new();
    let gold_run_id = Uuid::new_v4();
    let entry = PublishLedgerEntry::AgentPromotedToTier1 {
        publisher_id,
        agent_id: "summarize_strict".into(),
        version: "1.2.3".into(),
        gold_run_id,
        bypass: None,
    };
    let row = ledger.append(entry).await.unwrap();
    assert_eq!(row.entry.kind_label(), "agent_promoted_to_tier_1");
    let history = ledger.history(publisher_id, 10).await.unwrap();
    let found = history.iter().find_map(|r| match &r.entry {
        PublishLedgerEntry::AgentPromotedToTier1 { gold_run_id, .. } => Some(*gold_run_id),
        _ => None,
    });
    assert_eq!(found, Some(gold_run_id));
}

#[tokio::test]
async fn phase_5_n20_n18_agent_promoted_with_bypass_certificate() {
    let ledger = InMemoryPublishLedger::new([8u8; 32]);
    let publisher_id = efe_nano_sdk::PublisherId::new();
    let cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-9117".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let entry = PublishLedgerEntry::AgentPromotedToTier1 {
        publisher_id,
        agent_id: "x".into(),
        version: "1.0.0".into(),
        gold_run_id: Uuid::new_v4(),
        bypass: Some(cert.clone()),
    };
    let row = ledger.append(entry).await.unwrap();
    match &row.entry {
        PublishLedgerEntry::AgentPromotedToTier1 { bypass: Some(c), .. } => {
            assert_eq!(c.ticket_id, "INC-9117");
        }
        _ => panic!("expected AgentPromotedToTier1 with bypass"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Backward-compatibility — in-memory impls still work after N20
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_in_memory_impls_still_work() {
    // The Stub provider is unchanged ; N19 acceptance still passes.
    let p = StubEmbeddingProvider::new();
    let model = ModelIdentity::hosted("stub", "stub", "1");
    let e1 = p.embed(&model, "hello").await.unwrap();
    let e2 = p.embed(&model, "hello").await.unwrap();
    assert_eq!(e1.vector, e2.vector);
    let cosine = e1.cosine(&e2).unwrap();
    assert!((cosine - 1.0).abs() < 1e-9);
}

// ─────────────────────────────────────────────────────────────────────
//  Canonical bypass message is deterministic across calls
// ─────────────────────────────────────────────────────────────────────

#[test]
fn phase_5_n20_canonical_bypass_message_deterministic() {
    let run_id = GoldRunId::from_uuid(Uuid::nil());
    let bypass_id = Uuid::from_u128(42);
    let cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    };
    let m1 = canonical_bypass_message(run_id, bypass_id, &cert);
    let m2 = canonical_bypass_message(run_id, bypass_id, &cert);
    assert_eq!(m1, m2);
    // Layout : 16 + 16 + 8 + 4 + 5 (INC-1) + 4 + 64 = 117 bytes.
    assert_eq!(m1.len(), 117);
}

// ─────────────────────────────────────────────────────────────────────
//  Cohere serves multiple texts at different shapes
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_cohere_with_input_type_override() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embed"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({
            "input_type": "search_query"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "embeddings": [vec![0.5_f64; 1024]]
        })))
        .mount(&server)
        .await;
    let p = CohereEmbeddingProvider::with_endpoint("k", server.uri())
        .with_input_type("search_query");
    let model = ModelIdentity::hosted("cohere", "embed-multilingual-v3.0", "1");
    let e = p.embed(&model, "x").await.unwrap();
    assert_eq!(e.vector.len(), 1024);
}

// ─────────────────────────────────────────────────────────────────────
//  Multi-provider all-fail returns last error
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_5_n20_multi_provider_all_fail_returns_last_error() {
    let openai_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&openai_server)
        .await;
    let cohere_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/embed"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&cohere_server)
        .await;
    let client = MultiProviderEmbeddingClient::new(vec![
        NamedProvider::new(
            "openai",
            Arc::new(OpenAiEmbeddingProvider::with_endpoint(
                "k",
                openai_server.uri(),
            )),
        ),
        NamedProvider::new(
            "cohere",
            Arc::new(CohereEmbeddingProvider::with_endpoint(
                "k",
                cohere_server.uri(),
            )),
        ),
    ]);
    let model = ModelIdentity::hosted("openai", "text-embedding-3-small", "1");
    let err = client.embed(&model, "x").await.unwrap_err();
    assert!(matches!(err, EmbeddingProviderError::Backend { .. }));
}
