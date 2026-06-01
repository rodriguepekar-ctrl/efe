// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + G15** : chaos tests for the wired
//! compliance stack under adversarial conditions.
//!
//! | F-# | Failure scenario                                                  |
//! |-----|-------------------------------------------------------------------|
//! | F1  | Forged signature (sign with wrong key) — fail closed              |
//! | F2  | Tampered nano body                                                |
//! | F5  | Key rotation mid-flight — historical key still verifies           |
//! | F8  | Revoked publisher dispatch → status gate fails                    |
//! | F9  | Suspended publisher via stale cache → status gate fails           |
//! | F10 | Registry lookup latency exceeds timeout → soft fail              |
//! | F11 | 50 concurrent cache lookups → no torn reads                       |
//! | F13 | Unsigned tenant within grace window → resolver returns true       |

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::Utc;
use efe_compliance_wiring::{
    sign_catalogue_with_builtin_publisher, EFE_BUILTIN_PUBLISHER_IDENTITY,
    GraceWindowResolver, InMemoryGraceWindow, N24ComplianceMetrics, PublisherIdentityCache,
    WiredPublisherStatusGate, WiredSignatureGate,
};
use efe_nano_sdk::{
    Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, KeyId, KycTier,
    NanoAgent, NanoAgentId, NanoAgentVersion, PublisherId, PublisherIdentity, PublisherKey,
    PublisherSignature, PublisherStatus, ResourceLimits, Sha256Hash, SignatureAlgorithm,
    StabilityTier,
};
use efe_nano_swarm_adapter::{PublisherStatusGate, SignatureGate};
use efe_publisher_identity::registry::{
    InMemoryPublisherRegistry, PublisherRegistry, RegistryError,
};
use efe_publisher_identity::verifier::{Ed25519Signer, Ed25519Verifier};
use uuid::Uuid;

// ─── Helpers ────────────────────────────────────────────────────────

fn sample_nano(author: Author) -> NanoAgent {
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
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: Utc::now(),
    }
}

fn make_signature_gate(
    registry: Arc<dyn PublisherRegistry>,
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

// ─── F1 — Forged signature ─────────────────────────────────────────

#[tokio::test]
async fn chaos_n24_f1_forged_signature_fails_closed() {
    // Sign the catalogue with a foreign key, then attempt verify
    // against the built-in identity — every entry must fail.
    let foreign_signer = Ed25519Signer::from_seed([0xAAu8; 32]);
    let foreign_pem = foreign_signer.public_key_pem();
    assert_ne!(foreign_pem, efe_compliance_wiring::EFE_BUILTIN_PUBLISHER_PEM());

    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let cache = Arc::new(PublisherIdentityCache::default());
    let gate = make_signature_gate(registry, cache);

    let signed = sign_catalogue_with_builtin_publisher(
        efe_nano_preanalysis::default_catalogue(),
        None,
    );
    for (nano, _) in &signed {
        // Tamper with the signature bytes (forge with foreign key
        // signing a different blob).
        let mut bad_sig = nano.signature.as_ref().unwrap().clone();
        bad_sig.signature = foreign_signer.sign(b"different payload").to_vec();
        let result = gate.verify(nano, &bad_sig).await;
        assert!(
            result.is_err(),
            "forged signature MUST fail closed for {}",
            nano.id.as_str()
        );
    }
}

// ─── F2 — Tampered nano body ────────────────────────────────────────

#[tokio::test]
async fn chaos_n24_f2_tampered_body_fails_closed() {
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let cache = Arc::new(PublisherIdentityCache::default());
    let gate = make_signature_gate(registry, cache);

    let signed = sign_catalogue_with_builtin_publisher(
        efe_nano_preanalysis::default_catalogue(),
        None,
    );
    for (nano, _) in &signed {
        // Build a tampered clone with a different description.
        let mut tampered = (**nano).clone();
        tampered.description = format!("[TAMPERED] {}", nano.description);
        let sig = nano.signature.as_ref().unwrap();
        let result = gate.verify(&tampered, sig).await;
        assert!(
            result.is_err(),
            "tampered body MUST fail for {}",
            nano.id.as_str()
        );
    }
}

// ─── F5 — Key rotation mid-flight ──────────────────────────────────

#[tokio::test]
async fn chaos_n24_f5_historical_key_verifies_after_rotation() {
    // Build a custom identity : the historical key is the one we
    // signed with, the active key is fresh and unrelated.  The
    // gate's find_key falls through to was_active_at (since the
    // signature's key_id matches the historical key id) — verify
    // succeeds via the historical PEM.
    let historical_signer = Ed25519Signer::from_seed([0xBBu8; 32]);
    let historical_pem = historical_signer.public_key_pem();
    let historical_key_id = KeyId::new();
    let active_signer = Ed25519Signer::from_seed([0xCCu8; 32]);
    let active_pem = active_signer.public_key_pem();
    let active_key_id = KeyId::new();
    let publisher_id = PublisherId::new();
    let now = SystemTime::now();
    let signed_at_old = now - Duration::from_secs(30 * 24 * 3600);

    let identity = PublisherIdentity {
        publisher_id,
        display_name: "RotatedPub".into(),
        kyc_tier: KycTier::Tier2,
        status: PublisherStatus::Active,
        jurisdiction: vec![],
        registered_at: signed_at_old - Duration::from_secs(60),
        last_sanctions_check_at: Some(now),
        active_key: PublisherKey {
            key_id: active_key_id,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: active_pem,
            valid_from: now - Duration::from_secs(60),
            valid_until: None,
        },
        historical_keys: vec![PublisherKey {
            key_id: historical_key_id,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: historical_pem,
            valid_from: signed_at_old - Duration::from_secs(60),
            valid_until: Some(now - Duration::from_secs(60)),
        }],
        kyc_dossier_reference: None,
        revocation: None,
        forks_of: None,
    };
    let cache = Arc::new(PublisherIdentityCache::default());
    cache.put(publisher_id, identity);
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let gate = make_signature_gate(registry, cache);

    // Build a signature using the historical key seed at the old
    // timestamp.
    let nano = sample_nano(Author::Publisher {
        publisher_id,
        kyc_tier: KycTier::Tier2,
    });
    let placeholder_canonical_hash =
        efe_compliance_wiring::compute_agent_body_sha256(&nano);
    let placeholder = PublisherSignature {
        publisher_id,
        key_id: historical_key_id,
        algorithm: SignatureAlgorithm::Ed25519,
        signature: vec![],
        signed_at: signed_at_old,
        canonical_hash: placeholder_canonical_hash,
    };
    let canonical = efe_compliance_wiring::nano_to_canonical_body(&nano, &placeholder);
    let bytes = canonical.canonical_bytes();
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let d = h.finalize();
    let mut hh = [0u8; 32];
    hh.copy_from_slice(&d);
    let sig_bytes = historical_signer.sign(&bytes);
    let final_sig = PublisherSignature {
        publisher_id,
        key_id: historical_key_id,
        algorithm: SignatureAlgorithm::Ed25519,
        signature: sig_bytes.to_vec(),
        signed_at: signed_at_old,
        canonical_hash: Sha256Hash::from_bytes(hh),
    };

    gate.verify(&nano, &final_sig).await.unwrap();
}

// ─── F8 — Revoked publisher fails closed ───────────────────────────

#[tokio::test]
async fn chaos_n24_f8_revoked_publisher_fails_closed() {
    let id = PublisherId::new();
    let cache = Arc::new(PublisherIdentityCache::default());
    cache.put(
        id,
        PublisherIdentity {
            publisher_id: id,
            display_name: "Banned".into(),
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
    let gate = WiredPublisherStatusGate::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        cache,
        metrics.clone(),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
    );
    let nano = sample_nano(Author::Publisher {
        publisher_id: id,
        kyc_tier: KycTier::Tier1,
    });
    let result = gate.assert_active(&nano).await;
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("revoked"));
    assert_eq!(metrics.snapshot().status.revoked, 1);
}

// ─── F9 — Suspended publisher via stale cache ──────────────────────

#[tokio::test]
async fn chaos_n24_f9_suspended_publisher_via_stale_cache() {
    let id = PublisherId::new();
    let cache = Arc::new(PublisherIdentityCache::default());
    cache.put(
        id,
        PublisherIdentity {
            publisher_id: id,
            display_name: "OnHold".into(),
            kyc_tier: KycTier::Tier1,
            status: PublisherStatus::Suspended {
                reason: "kyc_review".into(),
                suspended_at: SystemTime::now(),
            },
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
    let gate = WiredPublisherStatusGate::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        cache,
        metrics.clone(),
        Duration::from_millis(500),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
    );
    let nano = sample_nano(Author::Publisher {
        publisher_id: id,
        kyc_tier: KycTier::Tier1,
    });
    let result = gate.assert_active(&nano).await;
    assert!(result.is_err());
    assert_eq!(metrics.snapshot().status.suspended, 1);
}

// ─── F10 — Registry lookup timeout fires ───────────────────────────

/// A registry that simulates an unbounded hang.
struct HangingRegistry;

#[async_trait]
impl PublisherRegistry for HangingRegistry {
    async fn register(
        &self,
        _candidate: efe_nano_sdk::NewPublisherCandidate,
    ) -> Result<PublisherIdentity, RegistryError> {
        unimplemented!()
    }
    async fn lookup(&self, _: PublisherId) -> Result<PublisherIdentity, RegistryError> {
        // Sleep WAY past the timeout.
        tokio::time::sleep(Duration::from_secs(60)).await;
        unimplemented!()
    }
    async fn rotate_key(
        &self,
        _: PublisherId,
        _: String,
        _: SignatureAlgorithm,
    ) -> Result<KeyId, RegistryError> {
        unimplemented!()
    }
    async fn rotate_key_with_proof(
        &self,
        _: PublisherId,
        _: String,
        _: SignatureAlgorithm,
        _: Vec<u8>,
        _: &dyn efe_publisher_identity::verifier::SignatureVerifier,
    ) -> Result<KeyId, RegistryError> {
        unimplemented!()
    }
    async fn suspend(&self, _: PublisherId, _: String) -> Result<(), RegistryError> {
        unimplemented!()
    }
    async fn unsuspend(&self, _: PublisherId) -> Result<(), RegistryError> {
        unimplemented!()
    }
    async fn revoke(
        &self,
        _: PublisherId,
        _: efe_nano_sdk::RevocationReason,
        _: Option<SystemTime>,
        _: Uuid,
    ) -> Result<efe_publisher_identity::registry::RevocationCascade, RegistryError> {
        unimplemented!()
    }
    async fn promote_tier(
        &self,
        _: PublisherId,
        _: KycTier,
        _: Vec<efe_nano_sdk::OperatorSignature>,
    ) -> Result<(), RegistryError> {
        unimplemented!()
    }
    async fn record_sanctions_check(
        &self,
        _: PublisherId,
        _: SystemTime,
    ) -> Result<(), RegistryError> {
        unimplemented!()
    }
    async fn refresh_sanctions(
        &self,
        _: PublisherId,
        _: &dyn efe_publisher_identity::sanctions::SanctionsScreener,
    ) -> Result<efe_publisher_identity::sanctions::SanctionsVerdict, RegistryError> {
        unimplemented!()
    }
    async fn record_published_version(
        &self,
        _: PublisherId,
        _: String,
        _: String,
        _: SystemTime,
    ) -> Result<(), RegistryError> {
        unimplemented!()
    }
    async fn list_active(
        &self,
        _: KycTier,
    ) -> Result<Vec<PublisherIdentity>, RegistryError> {
        unimplemented!()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chaos_n24_f10_registry_timeout_fires_soft_fail() {
    let cache = Arc::new(PublisherIdentityCache::default());
    let metrics = Arc::new(N24ComplianceMetrics::new());
    // 50 ms timeout — registry hangs forever — timeout fires.
    let gate = WiredPublisherStatusGate::new(
        Arc::new(HangingRegistry),
        cache,
        metrics.clone(),
        Duration::from_millis(50),
        EFE_BUILTIN_PUBLISHER_IDENTITY(),
    );
    let nano = sample_nano(Author::Publisher {
        publisher_id: PublisherId::new(),
        kyc_tier: KycTier::Tier1,
    });
    let started = std::time::Instant::now();
    let result = gate.assert_active(&nano).await;
    let elapsed = started.elapsed();
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("timed out"),
        "expected timeout error, got : {msg}"
    );
    // Elapsed should be close to the timeout, not 60 s.
    assert!(
        elapsed < Duration::from_secs(2),
        "timeout did not fire in time : {elapsed:?}"
    );
    assert_eq!(metrics.snapshot().status.registry_timeout, 1);
}

// ─── F11 — 50 concurrent cache lookups ──────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_n24_f11_50_concurrent_cache_lookups_no_torn_reads() {
    let id = PublisherId::new();
    let cache = Arc::new(PublisherIdentityCache::default());
    cache.put(
        id,
        PublisherIdentity {
            publisher_id: id,
            display_name: "Hot".into(),
            kyc_tier: KycTier::Tier2,
            status: PublisherStatus::Active,
            jurisdiction: vec![],
            registered_at: SystemTime::now(),
            last_sanctions_check_at: Some(SystemTime::now()),
            active_key: PublisherKey {
                key_id: KeyId::new(),
                algorithm: SignatureAlgorithm::Ed25519,
                public_key_pem: "pem".into(),
                valid_from: SystemTime::now(),
                valid_until: None,
            },
            historical_keys: vec![],
            kyc_dossier_reference: None,
            revocation: None,
            forks_of: None,
        },
    );
    let mut handles = Vec::new();
    for _ in 0..50 {
        let c = cache.clone();
        handles.push(tokio::spawn(async move { c.get(id) }));
    }
    for h in handles {
        let got = h.await.unwrap().expect("must be cached");
        assert_eq!(got.publisher_id, id);
        assert_eq!(got.display_name, "Hot");
    }
    assert_eq!(cache.hit_count(), 50);
}

// ─── F13 — Unsigned tenant within grace window ─────────────────────

#[tokio::test]
async fn chaos_n24_f13_in_grace_window_returns_true() {
    let grace = InMemoryGraceWindow::new();
    let tenant = Uuid::new_v4();
    grace.grant_default_grace(tenant, Utc::now());
    assert!(grace.in_grace_window(tenant, SystemTime::now()).await);
    // A different tenant has no grace.
    let other = Uuid::new_v4();
    assert!(!grace.in_grace_window(other, SystemTime::now()).await);
}

// ─── F13 (boundary) — grace expiry boundary ────────────────────────

#[tokio::test]
async fn chaos_n24_f13_grace_expiry_boundary_excludes_equal() {
    let grace = InMemoryGraceWindow::new();
    let tenant = Uuid::new_v4();
    let now = Utc::now();
    grace.set_deadline(tenant, now);
    // Boundary excludes equal — already expired.
    let at: SystemTime = now.into();
    assert!(!grace.in_grace_window(tenant, at).await);
}
