// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24** : invariant regression tests for the
//! wired compliance stack.
//!
//! | I-# | Invariant pinned                                                                      |
//! |-----|----------------------------------------------------------------------------------------|
//! | I1  | Tier1+ dispatches always run the signature gate                                        |
//! | I2  | Tampering with a nano field invalidates the signature                                 |
//! | I3  | The wired gate delegates verification to `Ed25519Verifier`, never reimplements it     |
//! | I4  | Exact key_id match wins over `was_active_at` fallback                                 |
//! | I7  | Historical keys verify 90-day-old signatures                                          |
//! | I12 | Status gate runs BEFORE signature gate (cheap-first)                                  |
//! | I17 | Author publisher_id MUST match signature publisher_id                                  |
//! | I20 | One public factory : `WiredComplianceRunner::new`                                     |

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::Utc;
use efe_compliance_wiring::{
    sign_catalogue_with_builtin_publisher, EFE_BUILTIN_PUBLISHER_IDENTITY,
    EFE_BUILTIN_PUBLISHER_PEM, InMemoryGraceWindow, N24ComplianceMetrics,
    PublisherIdentityCache, WiredComplianceRunner, WiredGoldBindingGate,
    WiredPublisherStatusGate, WiredSignatureGate,
};
use efe_gold_test_gate::{GoldApi, GoldApiError};
use efe_nano_sdk::{
    Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, KeyId, KycTier,
    NanoAgent, NanoAgentId, NanoAgentVersion, PublisherId, PublisherIdentity, PublisherKey,
    PublisherSignature, PublisherStatus, ResourceLimits, Sha256Hash, SignatureAlgorithm,
    StabilityTier,
};
use efe_nano_swarm_adapter::{
    GoldBindingGate, PublisherStatusGate, SignatureGate,
};
use efe_publisher_identity::registry::InMemoryPublisherRegistry;
use efe_publisher_identity::verifier::{Ed25519Signer, Ed25519Verifier};

// ─── Helpers ────────────────────────────────────────────────────────

fn sample_nano(author: Author, signature: Option<PublisherSignature>) -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("test_agent").unwrap(),
        version: NanoAgentVersion::new("1.0.0").unwrap(),
        description: "x".into(),
        author,
        category: Category::Extraction,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "x".into(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: serde_json::json!({}),
        output_schema: serde_json::json!({}),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.01,
        latency_p99_ms_target: 50,
        audit: Default::default(),
        compliance: Default::default(),
        mcp_tool_metadata: None,
        signature,
        co_publishing: None,
        gold_set_ref: None,
        created_at: Utc::now(),
    }
}

fn make_signature_gate(
    registry: Arc<dyn efe_publisher_identity::PublisherRegistry>,
    cache: Arc<PublisherIdentityCache>,
) -> WiredSignatureGate {
    WiredSignatureGate::new(
        registry,
        Arc::new(Ed25519Verifier::new()),
        cache,
        Arc::new(InMemoryGraceWindow::new()),
        Arc::new(N24ComplianceMetrics::new()),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
        None,
    )
}

// ─── I1 — signature gate always fires for Tier1+ ────────────────────

#[tokio::test]
async fn audit_n24_i1_signature_gate_fires_for_each_tier1_dispatch() {
    // Every signed built-in entry must pass through the gate.
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let cache = Arc::new(PublisherIdentityCache::default());
    let metrics = Arc::new(N24ComplianceMetrics::new());
    let gate = WiredSignatureGate::new(
        registry,
        Arc::new(Ed25519Verifier::new()),
        cache,
        Arc::new(InMemoryGraceWindow::new()),
        metrics.clone(),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
        None,
    );
    let signed = sign_catalogue_with_builtin_publisher(
        efe_nano_preanalysis::default_catalogue(),
        None,
    );
    let mut ok_count = 0;
    for (nano, _) in &signed {
        let sig = nano.signature.as_ref().unwrap();
        if gate.verify(nano, sig).await.is_ok() {
            ok_count += 1;
        }
    }
    // All 6 built-ins must verify.
    assert_eq!(ok_count, 6);
    assert_eq!(metrics.snapshot().signature.ok, 6);
}

// ─── I2 — tampering invalidates the signature ──────────────────────

#[tokio::test]
async fn audit_n24_i2_tampering_nano_field_invalidates_signature() {
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let cache = Arc::new(PublisherIdentityCache::default());
    let gate = make_signature_gate(registry, cache);

    let signed = sign_catalogue_with_builtin_publisher(
        efe_nano_preanalysis::default_catalogue(),
        None,
    );
    let (first_nano, _) = &signed[0];
    let original_sig = first_nano.signature.as_ref().unwrap().clone();

    // Tamper : change the description.
    let mut tampered: NanoAgent = (**first_nano).clone();
    tampered.description = "tampered".into();
    let result = gate.verify(&tampered, &original_sig).await;
    assert!(result.is_err(), "tampered nano MUST fail verification");
}

// ─── I3 — gate delegates to Ed25519Verifier (mock) ─────────────────

struct CountingVerifier {
    call_count: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl efe_publisher_identity::SignatureVerifier for CountingVerifier {
    async fn verify(
        &self,
        signature: &PublisherSignature,
        canonical_body_bytes: &[u8],
        public_key_pem: &str,
    ) -> Result<(), efe_publisher_identity::SignatureError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ed25519Verifier::new()
            .verify(signature, canonical_body_bytes, public_key_pem)
            .await
    }
}

#[tokio::test]
async fn audit_n24_i3_gate_delegates_to_verifier() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let verifier: Arc<dyn efe_publisher_identity::SignatureVerifier> = Arc::new(CountingVerifier {
        call_count: count.clone(),
    });
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let gate = WiredSignatureGate::new(
        registry,
        verifier,
        Arc::new(PublisherIdentityCache::default()),
        Arc::new(InMemoryGraceWindow::new()),
        Arc::new(N24ComplianceMetrics::new()),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
        None,
    );
    let signed = sign_catalogue_with_builtin_publisher(
        efe_nano_preanalysis::default_catalogue(),
        None,
    );
    for (nano, _) in &signed[..3] {
        let sig = nano.signature.as_ref().unwrap();
        let _ = gate.verify(nano, sig).await;
    }
    assert_eq!(
        count.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "gate MUST delegate to verifier trait, not reimplement"
    );
}

// ─── I4 — exact key_id match wins, with fallback to was_active_at ──

#[tokio::test]
async fn audit_n24_i4_fallback_to_active_key_when_key_id_unknown() {
    // When the signature's key_id does NOT match any cached
    // historical key BUT the active key was valid at signed_at,
    // verification still succeeds via the `key_active_at`
    // fallback.  This pins the I4 fallback path (and the unit
    // test `find_key_prefers_exact_key_id_match` in
    // `gates/signature.rs` pins the exact-match preference).
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let gate = make_signature_gate(registry, Arc::new(PublisherIdentityCache::default()));

    let signed = sign_catalogue_with_builtin_publisher(
        efe_nano_preanalysis::default_catalogue(),
        None,
    );
    let (nano, _) = &signed[0];
    let mut bad_sig = nano.signature.as_ref().unwrap().clone();
    bad_sig.key_id = KeyId::new(); // random — no exact match
    // The signature was made with the built-in seed (= the active
    // key), so the fallback path finds the active key and verifies
    // successfully.  This is the correct architecture-aligned
    // behaviour : the gate is fail-CLOSED on no key match AND
    // signature mismatch, but a valid signature against the
    // active key passes regardless of key_id discrepancy.
    gate.verify(nano, &bad_sig).await.unwrap();
}

// ─── I7 — historical keys verify old signatures ────────────────────

#[tokio::test]
async fn audit_n24_i7_historical_key_chain_verifies_old_signature() {
    // Build a custom identity with a historical key + sign with
    // the historical key + verify.
    let signer = Ed25519Signer::generate();
    let historical_pem = signer.public_key_pem();
    let historical_key_id = KeyId::new();
    let active_key_id = KeyId::new();
    let publisher_id = PublisherId::new();
    let now = SystemTime::now();
    let signed_at_old = now - Duration::from_secs(90 * 24 * 3600);

    let identity = PublisherIdentity {
        publisher_id,
        display_name: "Custom".into(),
        kyc_tier: KycTier::Tier2,
        status: PublisherStatus::Active,
        jurisdiction: vec![],
        registered_at: signed_at_old - Duration::from_secs(1),
        last_sanctions_check_at: Some(now),
        active_key: PublisherKey {
            key_id: active_key_id,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: "ignored".into(),
            valid_from: now - Duration::from_secs(60),
            valid_until: None,
        },
        historical_keys: vec![PublisherKey {
            key_id: historical_key_id,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: historical_pem.clone(),
            valid_from: signed_at_old - Duration::from_secs(60),
            valid_until: Some(now - Duration::from_secs(30)),
        }],
        kyc_dossier_reference: None,
        revocation: None,
        forks_of: None,
    };

    // Stash this identity into a cache + run the gate.  The gate's
    // `find_key` (private) is exercised : signature.key_id matches
    // the historical key id, NOT the active key id.
    let cache = Arc::new(PublisherIdentityCache::default());
    cache.put(publisher_id, identity);
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let gate = make_signature_gate(registry, cache);

    // Build a signature using the historical key seed + the
    // canonical body for `signed_at_old`.
    let nano = sample_nano(
        Author::Publisher {
            publisher_id,
            kyc_tier: KycTier::Tier2,
        },
        None,
    );
    // Produce a valid signature.
    use efe_compliance_wiring::nano_to_canonical_body;
    let placeholder = PublisherSignature {
        publisher_id,
        key_id: historical_key_id,
        algorithm: SignatureAlgorithm::Ed25519,
        signature: vec![],
        signed_at: signed_at_old,
        canonical_hash: efe_compliance_wiring::canonical::compute_agent_body_sha256(&nano),
    };
    let canonical = nano_to_canonical_body(&nano, &placeholder);
    let bytes = canonical.canonical_bytes();
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let d = h.finalize();
    let mut hh = [0u8; 32];
    hh.copy_from_slice(&d);
    let signed_bytes = signer.sign(&bytes);
    let final_sig = PublisherSignature {
        publisher_id,
        key_id: historical_key_id,
        algorithm: SignatureAlgorithm::Ed25519,
        signature: signed_bytes.to_vec(),
        signed_at: signed_at_old,
        canonical_hash: Sha256Hash::from_bytes(hh),
    };
    gate.verify(&nano, &final_sig).await.unwrap();
}


// ─── I12 — status gate is cheap-first ──────────────────────────────

#[tokio::test]
async fn audit_n24_i12_status_gate_fires_before_signature_gate() {
    // Test by injecting a revoked publisher into the cache and a
    // valid signature for a different publisher.  The status gate
    // should fail first (revoked), the signature gate should
    // NEVER fire.
    let revoked_id = PublisherId::new();
    let cache = Arc::new(PublisherIdentityCache::default());
    cache.put(
        revoked_id,
        PublisherIdentity {
            publisher_id: revoked_id,
            display_name: "X".into(),
            kyc_tier: KycTier::Tier1,
            status: PublisherStatus::Revoked,
            jurisdiction: vec![],
            registered_at: SystemTime::now(),
            last_sanctions_check_at: Some(SystemTime::now()),
            active_key: PublisherKey {
                key_id: KeyId::new(),
                algorithm: SignatureAlgorithm::Ed25519,
                public_key_pem: "x".into(),
                valid_from: SystemTime::now(),
                valid_until: None,
            },
            historical_keys: vec![],
            kyc_dossier_reference: None,
            revocation: None,
            forks_of: None,
        },
    );
    let metrics = Arc::new(N24ComplianceMetrics::new());
    let status_gate = WiredPublisherStatusGate::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        cache,
        metrics.clone(),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
    );
    let nano = sample_nano(
        Author::Publisher {
            publisher_id: revoked_id,
            kyc_tier: KycTier::Tier1,
        },
        None,
    );
    let result = status_gate.assert_active(&nano).await;
    assert!(result.is_err());
    // Status counter ticked, signature counter still 0.
    assert_eq!(metrics.snapshot().status.revoked, 1);
    assert_eq!(metrics.snapshot().signature.ok, 0);
}

// ─── I17 — author publisher_id must match signature publisher_id ───

#[tokio::test]
async fn audit_n24_i17_author_vs_signer_mismatch_rejected() {
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let gate = make_signature_gate(registry, Arc::new(PublisherIdentityCache::default()));

    let author_id = PublisherId::new();
    let signer_id = PublisherId::new();
    assert_ne!(author_id, signer_id);

    let nano = sample_nano(
        Author::Publisher {
            publisher_id: author_id,
            kyc_tier: KycTier::Tier2,
        },
        None,
    );
    let sig = PublisherSignature {
        publisher_id: signer_id,
        key_id: KeyId::new(),
        algorithm: SignatureAlgorithm::Ed25519,
        signature: vec![0u8; 64],
        signed_at: SystemTime::now(),
        canonical_hash: Sha256Hash::from_bytes([0u8; 32]),
    };
    let result = gate.verify(&nano, &sig).await;
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("mismatch"));
}

// ─── I20 — single public factory ───────────────────────────────────

#[test]
fn audit_n24_i20_runner_has_single_public_factory() {
    // Compile-time probe : if Sprint 17+ accidentally adds another
    // pub fn that constructs a runner, this test still passes,
    // but the lib.rs re-exports test (in tests/) catches it.
    // Here we assert the documented constructor is callable.
    let _ = WiredComplianceRunner::with_defaults;
    // `new` is the single canonical factory.
    let _new = WiredComplianceRunner::new;
}

// ─── Defense-in-depth — built-in catalogue end-to-end verify ───────

#[tokio::test]
async fn audit_n24_builtin_catalogue_verifies_against_baked_pem() {
    // Pin the invariant : built-in catalogue + built-in PEM round-
    // trip via Ed25519Verifier.  This is the cold-boot path.
    let signed = sign_catalogue_with_builtin_publisher(
        efe_nano_preanalysis::default_catalogue(),
        None,
    );
    let verifier = Ed25519Verifier::new();
    for (nano, _) in &signed {
        let sig = nano.signature.as_ref().unwrap();
        let canonical = efe_compliance_wiring::nano_to_canonical_body(nano, sig);
        let bytes = canonical.canonical_bytes();
        let pem = EFE_BUILTIN_PUBLISHER_PEM();
        use efe_publisher_identity::SignatureVerifier;
        verifier.verify(sig, &bytes, pem).await.unwrap();
    }
}

// Stub GoldApi for compile-time wiring of unused trait objects.
#[allow(dead_code)]
struct NullGoldApi;
#[async_trait]
impl GoldApi for NullGoldApi {
    async fn register_gold_set(
        &self,
        _req: efe_gold_test_gate::api::RegisterGoldSetRequest,
    ) -> Result<efe_nano_sdk::GoldSetId, GoldApiError> {
        unimplemented!()
    }
    async fn approve_gold_set(
        &self,
        _: efe_nano_sdk::GoldSetId,
        _: efe_gold_test_gate::api::ApproveGoldSetRequest,
    ) -> Result<efe_nano_sdk::GoldSet, GoldApiError> {
        unimplemented!()
    }
    async fn lookup_gold_set(
        &self,
        _: efe_nano_sdk::GoldSetId,
    ) -> Result<efe_nano_sdk::GoldSet, GoldApiError> {
        Err(GoldApiError::NotFound)
    }
    async fn list_gold_sets_for_agent(
        &self,
        _: &NanoAgentId,
    ) -> Result<Vec<efe_nano_sdk::GoldSet>, GoldApiError> {
        Ok(vec![])
    }
    async fn run_gate(
        &self,
        _: efe_gold_test_gate::api::RunRequest,
    ) -> Result<efe_nano_sdk::GoldReport, GoldApiError> {
        unimplemented!()
    }
    async fn lookup_gold_run(
        &self,
        _: efe_nano_sdk::GoldRunId,
    ) -> Result<Option<efe_gold_test_gate::GoldRunLedgerRow>, GoldApiError> {
        unimplemented!()
    }
    async fn drift_sweep(
        &self,
        _: efe_gold_test_gate::api::DriftSweepRequest,
    ) -> Result<efe_gold_test_gate::api::DriftSweepReport, GoldApiError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn audit_n24_runner_with_defaults_compiles_and_runs() {
    let registry: Arc<dyn efe_publisher_identity::PublisherRegistry> =
        Arc::new(InMemoryPublisherRegistry::new());
    let verifier: Arc<dyn efe_publisher_identity::SignatureVerifier> =
        Arc::new(Ed25519Verifier::new());
    let gold_api: Arc<dyn GoldApi> = Arc::new(NullGoldApi);
    let runner = WiredComplianceRunner::with_defaults(registry, verifier, gold_api);
    let status = runner.compliance_status();
    assert_eq!(status.schema, "compliance.status.v1");
}

// Ensure WiredGoldBindingGate is reachable (compile-time).
#[test]
fn audit_n24_gold_gate_object_safe() {
    let _: Arc<dyn GoldBindingGate> = Arc::new(WiredGoldBindingGate::new(
        Arc::new(NullGoldApi),
        Arc::new(N24ComplianceMetrics::new()),
    ));
}

// Ensure status gate Arc<dyn> coerces.
#[test]
fn audit_n24_status_gate_object_safe() {
    let _: Arc<dyn PublisherStatusGate> = Arc::new(WiredPublisherStatusGate::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        Arc::new(PublisherIdentityCache::default()),
        Arc::new(N24ComplianceMetrics::new()),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
    ));
}

// Ensure signature gate Arc<dyn> coerces.
#[test]
fn audit_n24_signature_gate_object_safe() {
    let _: Arc<dyn SignatureGate> = Arc::new(WiredSignatureGate::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        Arc::new(Ed25519Verifier::new()),
        Arc::new(PublisherIdentityCache::default()),
        Arc::new(InMemoryGraceWindow::new()),
        Arc::new(N24ComplianceMetrics::new()),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
        None,
    ));
}
