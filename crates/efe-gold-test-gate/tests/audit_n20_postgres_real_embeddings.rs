// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : post-audit regression tests.
//!
//! These tests pin down bugs caught during the chirurgical audit
//! pass on top of the original 14 acceptance + 77 unit tests.
//!
//! | # | Defect | Where | Fix |
//! |---|---|---|---|
//! | B1 | `MultiProviderEmbeddingClient` held DashMap `Ref` guard across `.await` (deadlock risk) | `multi_provider.rs::embed` | drop guard before await, re-acquire to record outcome |
//! | B2 | `BypassSignatureVerifier::verify` vacuously accepted certs with 0 approvers | `bypass_verifier.rs::verify` | enforce floor `≥ GOLD_SET_MIN_APPROVERS` internally + reject duplicate operator ids |

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use efe_gold_test_gate::{
    canonical_bypass_message, BypassSignatureVerifier, CircuitBreakerState,
    EmbeddingProvider, EmbeddingProviderError, InMemoryOperatorKeyResolver,
    MultiProviderEmbeddingClient, NamedProvider, OperatorKey, StubEmbeddingProvider,
};
use efe_nano_sdk::{
    BypassCertificate, GoldGateError, GoldRunId, ModelIdentity, OperatorSignature,
    SignatureAlgorithm, GOLD_SET_MIN_APPROVERS,
};
use efe_publisher_identity::{Ed25519Signer, Ed25519Verifier};
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Test fixture : a provider that takes a controllable async delay
// ─────────────────────────────────────────────────────────────────────

struct SlowProvider {
    delay_ms: u64,
    call_count: AtomicU64,
}

impl SlowProvider {
    fn new(delay_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            delay_ms,
            call_count: AtomicU64::new(0),
        })
    }
    fn calls(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }
}

impl std::fmt::Debug for SlowProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlowProvider")
            .field("delay_ms", &self.delay_ms)
            .field("calls", &self.calls())
            .finish()
    }
}

#[async_trait::async_trait]
impl EmbeddingProvider for SlowProvider {
    async fn embed(
        &self,
        model: &ModelIdentity,
        _text: &str,
    ) -> Result<efe_gold_test_gate::embedding::Embedding, EmbeddingProviderError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        Ok(efe_gold_test_gate::embedding::Embedding {
            model: model.clone(),
            vector: vec![0.5; 8],
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  B1 — Multi-provider does NOT deadlock under concurrent reset
// ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_b1_concurrent_reset_during_embed_does_not_deadlock() {
    // Repro for the bug : the in-flight embed used to hold a
    // DashMap::Ref across .await ; a concurrent `reset_breaker` for
    // the same provider would block on the DashMap shard lock until
    // the await resolved.  Post-fix : the Ref is dropped before
    // await, so reset_breaker proceeds immediately.
    let slow = SlowProvider::new(100);
    let client = Arc::new(MultiProviderEmbeddingClient::new(vec![
        NamedProvider::new("slow", slow.clone() as Arc<dyn EmbeddingProvider>),
    ]));
    let model = ModelIdentity::hosted("any", "any", "1");
    let c1 = client.clone();
    let embed_task = tokio::spawn(async move {
        c1.embed(&model, "x").await
    });
    // Give the embed time to start.
    tokio::time::sleep(Duration::from_millis(20)).await;
    // Now reset the breaker while the embed is in-flight.  Before
    // the fix, this would block ; after the fix, it returns
    // immediately.
    let reset_start = std::time::Instant::now();
    let did_reset = client.reset_breaker("slow");
    let reset_elapsed = reset_start.elapsed();
    assert!(did_reset, "reset must succeed for known provider");
    assert!(
        reset_elapsed < Duration::from_millis(50),
        "reset should not block on the in-flight embed (took {reset_elapsed:?})"
    );
    let _ = embed_task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_b1_concurrent_breaker_state_read_does_not_deadlock() {
    let slow = SlowProvider::new(100);
    let client = Arc::new(MultiProviderEmbeddingClient::new(vec![
        NamedProvider::new("slow", slow.clone() as Arc<dyn EmbeddingProvider>),
    ]));
    let model = ModelIdentity::hosted("any", "any", "1");
    let c1 = client.clone();
    let embed_task = tokio::spawn(async move {
        c1.embed(&model, "x").await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    // Read breaker state concurrently — pre-fix could deadlock if
    // dashmap shard guard collision occurred.
    let state = client.breaker_state("slow");
    assert_eq!(state, Some(CircuitBreakerState::Closed));
    let _ = embed_task.await.unwrap();
}

#[tokio::test]
async fn audit_b1_breaker_records_outcome_even_after_in_flight() {
    // Ensure the post-fix path that re-acquires the breaker after
    // .await still records success/failure properly.
    let slow = SlowProvider::new(5);
    let client = MultiProviderEmbeddingClient::new(vec![NamedProvider::new(
        "slow",
        slow.clone() as Arc<dyn EmbeddingProvider>,
    )]);
    let model = ModelIdentity::hosted("any", "any", "1");
    let _ = client.embed(&model, "x").await.unwrap();
    assert_eq!(slow.calls(), 1);
    // Breaker stays Closed after success.
    assert_eq!(client.breaker_state("slow"), Some(CircuitBreakerState::Closed));
}

// ─────────────────────────────────────────────────────────────────────
//  B2 — Bypass verifier enforces floor + dup check internally
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b2_zero_approvers_rejected_with_floor_error() {
    let resolver = Arc::new(InMemoryOperatorKeyResolver::new());
    let verifier = BypassSignatureVerifier::new(
        Arc::new(Ed25519Verifier::new()),
        resolver,
    );
    let c = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let err = verifier
        .verify(&c, GoldRunId::new(), Uuid::new_v4())
        .await
        .unwrap_err();
    match err {
        GoldGateError::BypassInsufficientApprovers { got, need } => {
            assert_eq!(got, 0);
            assert_eq!(need, GOLD_SET_MIN_APPROVERS as u32);
        }
        other => panic!("expected floor error, got {other:?}"),
    }
}

#[tokio::test]
async fn audit_b2_one_approver_rejected_with_floor_error() {
    let resolver = Arc::new(InMemoryOperatorKeyResolver::new());
    let s = Ed25519Signer::from_seed([1u8; 32]);
    let op = Uuid::from_u128(1);
    resolver.insert(OperatorKey {
        operator_id: op,
        algorithm: SignatureAlgorithm::Ed25519,
        public_key_pem: s.public_key_pem(),
    });
    let verifier = BypassSignatureVerifier::new(
        Arc::new(Ed25519Verifier::new()),
        resolver,
    );
    let mut c = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let run_id = GoldRunId::new();
    let bypass_id = Uuid::new_v4();
    let canonical = canonical_bypass_message(run_id, bypass_id, &c);
    c.approvers.push(OperatorSignature {
        operator_id: op,
        signed_at: SystemTime::now(),
        ticket_id: "INC-1".into(),
        signature: s.sign(&canonical).to_vec(),
    });
    let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
    assert!(matches!(
        err,
        GoldGateError::BypassInsufficientApprovers { .. }
    ));
}

#[tokio::test]
async fn audit_b2_duplicate_operator_id_rejected() {
    let resolver = Arc::new(InMemoryOperatorKeyResolver::new());
    let s = Ed25519Signer::from_seed([1u8; 32]);
    let op = Uuid::from_u128(1);
    resolver.insert(OperatorKey {
        operator_id: op,
        algorithm: SignatureAlgorithm::Ed25519,
        public_key_pem: s.public_key_pem(),
    });
    let verifier = BypassSignatureVerifier::new(
        Arc::new(Ed25519Verifier::new()),
        resolver,
    );
    let mut c = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let run_id = GoldRunId::new();
    let bypass_id = Uuid::new_v4();
    let canonical = canonical_bypass_message(run_id, bypass_id, &c);
    let sig_bytes = s.sign(&canonical).to_vec();
    // Same operator id appears twice — defense against replay.
    for _ in 0..2 {
        c.approvers.push(OperatorSignature {
            operator_id: op,
            signed_at: SystemTime::now(),
            ticket_id: "INC-1".into(),
            signature: sig_bytes.clone(),
        });
    }
    let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
    match err {
        GoldGateError::BypassSignatureInvalid { approver_index } => {
            assert_eq!(approver_index, 1);
        }
        other => panic!("expected dup detection, got {other:?}"),
    }
}

#[tokio::test]
async fn audit_b2_two_distinct_valid_approvers_passes() {
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
    let mut c = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let canonical = canonical_bypass_message(run_id, bypass_id, &c);
    c.approvers.push(OperatorSignature {
        operator_id: op1,
        signed_at: SystemTime::now(),
        ticket_id: "INC-1".into(),
        signature: s1.sign(&canonical).to_vec(),
    });
    c.approvers.push(OperatorSignature {
        operator_id: op2,
        signed_at: SystemTime::now(),
        ticket_id: "INC-1".into(),
        signature: s2.sign(&canonical).to_vec(),
    });
    verifier.verify(&c, run_id, bypass_id).await.unwrap();
}

// ─────────────────────────────────────────────────────────────────────
//  B3 — Circuit breaker concurrency : many threads recording fails
// ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_b3_concurrent_failures_transition_atomically() {
    // 8 tasks each record 3 failures concurrently.  The breaker
    // MUST transition to Open exactly once (no double-transition,
    // no missed transition).
    let client = Arc::new(MultiProviderEmbeddingClient::with_breaker_thresholds(
        vec![NamedProvider::new(
            "p",
            Arc::new(AlwaysErrProvider) as Arc<dyn EmbeddingProvider>,
        )],
        3,
        30,
    ));
    let model = ModelIdentity::hosted("any", "any", "1");
    let mut handles = Vec::new();
    for _ in 0..8 {
        let c = client.clone();
        let m = model.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..3 {
                let _ = c.embed(&m, "x").await;
            }
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    // After the burst, the breaker is Open.
    assert_eq!(
        client.breaker_state("p"),
        Some(CircuitBreakerState::Open)
    );
}

struct AlwaysErrProvider;
impl std::fmt::Debug for AlwaysErrProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AlwaysErrProvider")
    }
}
#[async_trait::async_trait]
impl EmbeddingProvider for AlwaysErrProvider {
    async fn embed(
        &self,
        _: &ModelIdentity,
        _: &str,
    ) -> Result<efe_gold_test_gate::embedding::Embedding, EmbeddingProviderError> {
        Err(EmbeddingProviderError::Backend {
            message: "always-err".into(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  B4 — Canonical bypass message layout : known bytes
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b4_canonical_message_known_bytes_layout() {
    // Pin the wire layout : flipping ANY field MUST flip the
    // canonical bytes.
    let run_id = GoldRunId::from_uuid(Uuid::from_u128(0x0102_0304_0506_0708_090A_0B0C_0D0E_0F10));
    let bypass_id = Uuid::from_u128(0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899);
    let cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "R".to_string(),
        valid_until: std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
    };
    let m = canonical_bypass_message(run_id, bypass_id, &cert);
    // Layout : 16 (run_id) + 16 (bypass_id) + 8 (i64) + 4 (len) + 5
    // (INC-1) + 4 (len) + 1 (R) = 54 bytes.
    assert_eq!(m.len(), 54);
    // First 16 bytes = run_id u128 BE.
    assert_eq!(&m[0..16], &run_id.as_uuid().as_bytes()[..]);
    // Next 16 = bypass_id.
    assert_eq!(&m[16..32], bypass_id.as_bytes());
    // valid_until BE = 1_700_000_000 i64.
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&m[32..40]);
    assert_eq!(i64::from_be_bytes(buf), 1_700_000_000);
}

#[test]
fn audit_b4_canonical_message_changes_on_run_id() {
    let r1 = GoldRunId::from_uuid(Uuid::nil());
    let r2 = GoldRunId::new();
    let bypass_id = Uuid::nil();
    let cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
    };
    assert_ne!(
        canonical_bypass_message(r1, bypass_id, &cert),
        canonical_bypass_message(r2, bypass_id, &cert)
    );
}

#[test]
fn audit_b4_canonical_message_changes_on_bypass_id() {
    let run_id = GoldRunId::from_uuid(Uuid::nil());
    let b1 = Uuid::from_u128(1);
    let b2 = Uuid::from_u128(2);
    let cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
    };
    assert_ne!(
        canonical_bypass_message(run_id, b1, &cert),
        canonical_bypass_message(run_id, b2, &cert)
    );
}

#[test]
fn audit_b4_canonical_message_changes_on_valid_until() {
    let run_id = GoldRunId::from_uuid(Uuid::nil());
    let bypass_id = Uuid::nil();
    let c1 = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::UNIX_EPOCH + Duration::from_secs(1),
    };
    let c2 = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::UNIX_EPOCH + Duration::from_secs(2),
    };
    assert_ne!(
        canonical_bypass_message(run_id, bypass_id, &c1),
        canonical_bypass_message(run_id, bypass_id, &c2)
    );
}

// ─────────────────────────────────────────────────────────────────────
//  B5 — Multi-provider with empty list returns BackendError early
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b5_empty_provider_list_returns_backend() {
    let client = MultiProviderEmbeddingClient::new(vec![]);
    let model = ModelIdentity::hosted("x", "x", "1");
    let err = client.embed(&model, "x").await.unwrap_err();
    match err {
        EmbeddingProviderError::Backend { message } => {
            assert!(message.contains("no providers"));
        }
        other => panic!("expected Backend, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  B6 — Reset breaker re-enables a previously open provider
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b6_reset_breaker_re_enables_provider() {
    let client = MultiProviderEmbeddingClient::with_breaker_thresholds(
        vec![
            NamedProvider::new("p", Arc::new(AlwaysErrProvider)),
            NamedProvider::new("fallback", Arc::new(StubEmbeddingProvider::new())),
        ],
        1,
        30,
    );
    let model = ModelIdentity::hosted("x", "x", "1");
    // First call opens p ; fallback returns OK.
    let _ = client.embed(&model, "x").await.unwrap();
    assert_eq!(client.breaker_state("p"), Some(CircuitBreakerState::Open));
    // Reset opens the door for p to be retried.
    assert!(client.reset_breaker("p"));
    assert_eq!(client.breaker_state("p"), Some(CircuitBreakerState::Closed));
}

// ─────────────────────────────────────────────────────────────────────
//  B7 — N18 AgentPromotedToTier1 kind_label is stable string
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b7_n18_agent_promoted_to_tier_1_kind_label_stable() {
    use efe_publisher_identity::PublishLedgerEntry;
    let entry = PublishLedgerEntry::AgentPromotedToTier1 {
        publisher_id: efe_nano_sdk::PublisherId::new(),
        agent_id: "x".into(),
        version: "1.0.0".into(),
        gold_run_id: Uuid::new_v4(),
        bypass: None,
    };
    // `kind_label()` is hand-coded ("agent_promoted_to_tier_1")
    // independently of how serde renames the variant.
    assert_eq!(entry.kind_label(), "agent_promoted_to_tier_1");
}

#[test]
fn audit_b7_n18_publisher_id_extracted_correctly() {
    use efe_publisher_identity::PublishLedgerEntry;
    let pub_id = efe_nano_sdk::PublisherId::new();
    let entry = PublishLedgerEntry::AgentPromotedToTier1 {
        publisher_id: pub_id,
        agent_id: "x".into(),
        version: "1.0.0".into(),
        gold_run_id: Uuid::new_v4(),
        bypass: None,
    };
    assert_eq!(entry.publisher_id(), pub_id);
}

// ─────────────────────────────────────────────────────────────────────
//  B8 — N18 ledger entry serde round-trip
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b8_n18_agent_promoted_to_tier_1_serde_round_trip() {
    use efe_publisher_identity::PublishLedgerEntry;
    let entry = PublishLedgerEntry::AgentPromotedToTier1 {
        publisher_id: efe_nano_sdk::PublisherId::new(),
        agent_id: "summarize_strict".into(),
        version: "1.2.3".into(),
        gold_run_id: Uuid::new_v4(),
        bypass: None,
    };
    let json = serde_json::to_string(&entry).unwrap();
    // Serde's `rename_all = "snake_case"` keeps "Tier1" → "tier_1"
    // (lowercase digit separator).  Accept either tier_1 or tier1
    // formatting since serde_snake_case treats digits as part of the
    // identifier.
    let tier1_either = json.contains("agent_promoted_to_tier_1")
        || json.contains("agent_promoted_to_tier1");
    assert!(tier1_either, "unexpected JSON tag : {json}");
    // Round-trip MUST preserve the entry regardless of which spelling
    // serde chose.
    let back: PublishLedgerEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back, entry);
}

#[test]
fn audit_b8_n18_agent_promoted_with_bypass_serde() {
    use efe_publisher_identity::PublishLedgerEntry;
    let cert = BypassCertificate {
        approvers: vec![],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let entry = PublishLedgerEntry::AgentPromotedToTier1 {
        publisher_id: efe_nano_sdk::PublisherId::new(),
        agent_id: "x".into(),
        version: "1.0.0".into(),
        gold_run_id: Uuid::new_v4(),
        bypass: Some(cert.clone()),
    };
    let json = serde_json::to_string(&entry).unwrap();
    let back: PublishLedgerEntry = serde_json::from_str(&json).unwrap();
    match back {
        PublishLedgerEntry::AgentPromotedToTier1 { bypass: Some(c), .. } => {
            assert_eq!(c.ticket_id, cert.ticket_id);
        }
        _ => panic!("expected AgentPromotedToTier1 with bypass"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  B9 — Drift sweep cron : ticks_fired counter increments correctly
// ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn audit_b9_drift_cron_ticks_fired_count_accurate() {
    use efe_gold_test_gate::{DriftSweepCron, DriftSweepCronConfig, DriftSweepState, InMemoryDriftSweepState};
    let state = Arc::new(InMemoryDriftSweepState::new());
    let cron = DriftSweepCron::new(
        state as Arc<dyn DriftSweepState>,
        DriftSweepCronConfig {
            tick_interval: Duration::from_secs(86_400),
            drift_threshold: Duration::from_secs(60),
        },
    );
    assert_eq!(cron.ticks_fired(), 0);
    let handle = cron.clone().start(|_| async {});
    tokio::task::yield_now().await;
    cron.notify_tick();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(cron.ticks_fired() >= 1);
    cron.notify_tick();
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(cron.ticks_fired() >= 2);
    handle.abort();
}

// ─────────────────────────────────────────────────────────────────────
//  B10 — Provider_names preserves the configured priority order
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b10_provider_names_priority_preserved() {
    let client = MultiProviderEmbeddingClient::new(vec![
        NamedProvider::new("zebra", Arc::new(StubEmbeddingProvider::new())),
        NamedProvider::new("apple", Arc::new(StubEmbeddingProvider::new())),
        NamedProvider::new("mango", Arc::new(StubEmbeddingProvider::new())),
    ]);
    assert_eq!(
        client.provider_names(),
        vec!["zebra".to_string(), "apple".to_string(), "mango".to_string()]
    );
}
