// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : end-to-end acceptance tests.
//!
//! Matches the 15-row acceptance matrix in architecture § 12.1.
//!
//! Each test sets up a tiny fixture (registry + ledger + verifier +
//! sanctions + cooldown + metrics) and walks one publisher through a
//! single life-cycle path that exercises one or two architectural
//! invariants.  The asserts target invariant satisfaction, not the
//! plumbing of any one impl — that way a Sprint 11 swap to Postgres
//! should keep these tests green unchanged.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use efe_nano_sdk::{
    Iso3166, KycTier, NewPublisherCandidate, OperatorSignature, PublisherId,
    PublisherSignature, PublisherStatus, RevocationReason, Sha256Hash, SignatureAlgorithm,
};
use efe_publisher_identity::{
    canonical_body_hash, CanonicalBody, CooldownEnforcer, CooldownError, Ed25519Signer,
    Ed25519Verifier, InMemoryPublishLedger, InMemoryPublisherRegistry, N18Metrics,
    PublishLedger, PublishLedgerEntry, PublisherRegistry, SanctionsScreener,
    SanctionsVerdict, SignatureVerifier, StubSanctionsScreener,
};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Fixture helpers
// ─────────────────────────────────────────────────────────────────────

struct Fixture {
    registry: Arc<InMemoryPublisherRegistry>,
    ledger: Arc<InMemoryPublishLedger>,
    verifier: Ed25519Verifier,
    sanctions: Arc<StubSanctionsScreener>,
    metrics: Arc<N18Metrics>,
}

fn build_fixture() -> Fixture {
    Fixture {
        registry: Arc::new(InMemoryPublisherRegistry::new()),
        ledger: Arc::new(InMemoryPublishLedger::new([0x42u8; 32])),
        verifier: Ed25519Verifier::new(),
        sanctions: Arc::new(StubSanctionsScreener::clear()),
        metrics: N18Metrics::new(),
    }
}

fn candidate(
    name: &str,
    tier: KycTier,
    initial_public_key_pem: String,
) -> NewPublisherCandidate {
    NewPublisherCandidate {
        display_name: name.into(),
        target_tier: tier,
        jurisdiction: vec![Iso3166::new("US").unwrap()],
        initial_public_key_pem,
        algorithm: SignatureAlgorithm::Ed25519,
        pseudonymous: tier == KycTier::Tier0,
        kyc_dossier_reference: if tier >= KycTier::Tier2 {
            Some("kyc-dossier-1".into())
        } else {
            None
        },
    }
}

fn sign_canonical(
    signer: &Ed25519Signer,
    publisher_id: PublisherId,
    key_id: efe_nano_sdk::KeyId,
    agent_id: &str,
    version: &str,
    tier: KycTier,
) -> (CanonicalBody, PublisherSignature) {
    let now = SystemTime::now();
    let epoch_seconds = now.duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let body = CanonicalBody::new(
        agent_id,
        version,
        publisher_id,
        tier,
        Sha256Hash::from_bytes([0xDE; 32]),
        epoch_seconds,
    );
    let body_bytes = body.canonical_bytes();
    let sig_bytes = signer.sign(&body_bytes);
    let signature = PublisherSignature {
        publisher_id,
        key_id,
        algorithm: SignatureAlgorithm::Ed25519,
        signature: sig_bytes.to_vec(),
        signed_at: now,
        canonical_hash: canonical_body_hash(&body),
    };
    (body, signature)
}

fn operator_approval() -> OperatorSignature {
    OperatorSignature {
        operator_id: Uuid::new_v4(),
        signed_at: SystemTime::now(),
        ticket_id: "OPS-1234".into(),
        signature: vec![0u8; 64],
    }
}

// ─────────────────────────────────────────────────────────────────────
//  1. publish signature verifies
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_publish_signature_verifies() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([1u8; 32]);
    let identity = fx
        .registry
        .register(candidate("ACME", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let (body, sig) = sign_canonical(
        &signer,
        identity.publisher_id,
        identity.active_key.key_id,
        "summarize_strict",
        "1.2.3",
        KycTier::Tier2,
    );
    fx.verifier
        .verify(&sig, &body.canonical_bytes(), &identity.active_key.public_key_pem)
        .await
        .unwrap();
    fx.metrics
        .tick_signature_verify(SignatureAlgorithm::Ed25519.label(), "verified");
    assert_eq!(
        fx.metrics
            .signature_verify_total(SignatureAlgorithm::Ed25519.label(), "verified"),
        1
    );
}

// ─────────────────────────────────────────────────────────────────────
//  2. KYC tier gates catalog
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_kyc_tier_gates_catalog() {
    // Per architecture G2 + I9 : Tier 0 / 1 must NOT publish to the
    // production catalog ; only Tier 2 + 3 may.
    assert!(!KycTier::Tier0.permits_production_catalog());
    assert!(!KycTier::Tier1.permits_production_catalog());
    assert!(KycTier::Tier2.permits_production_catalog());
    assert!(KycTier::Tier3.permits_production_catalog());

    // The registry must accept Tier 0 / 1 ; the dispatch / catalog
    // layer is the one that enforces the gate.  Round-trip a Tier 1
    // publisher and confirm the SDK label.
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([2u8; 32]);
    let identity = fx
        .registry
        .register(candidate("ACME-sandbox", KycTier::Tier1, signer.public_key_pem()))
        .await
        .unwrap();
    assert_eq!(identity.kyc_tier.label(), "tier_1");
    assert!(!identity.kyc_tier.permits_production_catalog());
}

// ─────────────────────────────────────────────────────────────────────
//  3. publish ledger chain verifies
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_publish_ledger_chain_verifies() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([3u8; 32]);
    let identity = fx
        .registry
        .register(candidate("LedgerCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;

    // Append a chain of three entries.
    fx.ledger
        .append(PublishLedgerEntry::PublisherRegistered {
            identity: Box::new(identity.clone()),
        })
        .await
        .unwrap();
    let (_, sig) = sign_canonical(
        &signer,
        publisher_id,
        identity.active_key.key_id,
        "agent-a",
        "1.0.0",
        KycTier::Tier2,
    );
    fx.ledger
        .append(PublishLedgerEntry::AgentPublished {
            publisher_id,
            agent_id: "agent-a".into(),
            version: "1.0.0".into(),
            signature: sig,
            kyc_tier_snapshot: KycTier::Tier2,
        })
        .await
        .unwrap();
    fx.ledger
        .append(PublishLedgerEntry::PublisherSuspended {
            publisher_id,
            reason: "ops-review".into(),
        })
        .await
        .unwrap();
    let report = fx.ledger.verify_chain(publisher_id).await.unwrap();
    assert!(report.ok);
    assert_eq!(report.entries_verified, 3);
    assert!(report.head_trace_id.is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  4. revocation cascades to versions
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_revocation_cascades_to_versions() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([4u8; 32]);
    let identity = fx
        .registry
        .register(candidate("CascadeCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;
    let compromise_at = SystemTime::now() - Duration::from_secs(3_600);

    // Two versions published in window, one published before.
    fx.registry
        .record_published_version(
            publisher_id,
            "agent-x".into(),
            "1.0.0".into(),
            compromise_at - Duration::from_secs(7_200), // before window → preserved
        )
        .await
        .unwrap();
    fx.registry
        .record_published_version(
            publisher_id,
            "agent-x".into(),
            "1.1.0".into(),
            compromise_at + Duration::from_secs(120),
        )
        .await
        .unwrap();
    fx.registry
        .record_published_version(
            publisher_id,
            "agent-y".into(),
            "0.9.1".into(),
            compromise_at + Duration::from_secs(600),
        )
        .await
        .unwrap();

    let cascade = fx
        .registry
        .revoke(
            publisher_id,
            RevocationReason::KeyCompromise,
            Some(compromise_at),
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    assert_eq!(cascade.publisher_id, publisher_id);
    assert_eq!(cascade.revoked_versions.len(), 2);
    assert!(cascade.replay_window.is_some());

    fx.metrics
        .observe_revocation_cascade("key_compromise", cascade.revoked_versions.len() as u64);
    assert_eq!(fx.metrics.revocation_cascade_count("key_compromise"), 1);
    assert_eq!(fx.metrics.revocation_cascade_sum("key_compromise"), 2);

    // Lookup must now reflect Revoked status (G15).
    let after = fx.registry.lookup(publisher_id).await.unwrap();
    assert_eq!(after.status, PublisherStatus::Revoked);
    assert!(after.revocation.is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  5. cooldown blocks promotion
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_cooldown_blocks_promotion() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([5u8; 32]);
    let identity = fx
        .registry
        .register(candidate("CoolDownCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();

    // 1 hour after publish — must block.
    let published_at = SystemTime::now() - Duration::from_secs(3_600);
    let err = CooldownEnforcer::check(&identity, published_at, SystemTime::now())
        .unwrap_err();
    assert!(matches!(err, CooldownError::NotElapsed { .. }));
    fx.metrics.tick_cooldown_block(identity.kyc_tier.label());
    assert_eq!(fx.metrics.cooldown_blocks_total("tier_2"), 1);

    // 25 hours after publish — must pass.
    let published_at = SystemTime::now() - Duration::from_secs(25 * 3_600);
    assert!(CooldownEnforcer::check(&identity, published_at, SystemTime::now()).is_ok());
}

// ─────────────────────────────────────────────────────────────────────
//  6. signature proves publisher action
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_signature_proves_publisher_action() {
    // Tampered body → verify fails.  Wrong key → verify fails.
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([6u8; 32]);
    let attacker = Ed25519Signer::from_seed([66u8; 32]);
    let identity = fx
        .registry
        .register(candidate("ProveCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let (body, sig) = sign_canonical(
        &signer,
        identity.publisher_id,
        identity.active_key.key_id,
        "summarize",
        "1.0.0",
        KycTier::Tier2,
    );
    // Body tamper.
    let mut tampered = body.canonical_bytes();
    tampered[0] ^= 0x01;
    let err = fx
        .verifier
        .verify(&sig, &tampered, &identity.active_key.public_key_pem)
        .await
        .unwrap_err();
    let _ = err; // any verify error is enough
    fx.metrics
        .tick_signature_verify(SignatureAlgorithm::Ed25519.label(), "rejected");

    // Wrong key.
    let err = fx
        .verifier
        .verify(&sig, &body.canonical_bytes(), &attacker.public_key_pem())
        .await
        .unwrap_err();
    let _ = err;
    fx.metrics
        .tick_signature_verify(SignatureAlgorithm::Ed25519.label(), "rejected");

    assert_eq!(
        fx.metrics
            .signature_verify_total(SignatureAlgorithm::Ed25519.label(), "rejected"),
        2
    );
}

// ─────────────────────────────────────────────────────────────────────
//  7. dispatch trace carries publisher id
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_dispatch_trace_carries_publisher_id() {
    // Sprint 10 surface : the SDK exposes `Author::Publisher` so the
    // dispatch trace can record the publisher id.  Sprint 11 wires the
    // actual nano-dispatch loop ; for now we assert the SDK shape.
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([7u8; 32]);
    let identity = fx
        .registry
        .register(candidate("TraceCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let author = efe_nano_sdk::Author::Publisher {
        publisher_id: identity.publisher_id,
        kyc_tier: KycTier::Tier2,
    };
    assert!(author.is_publisher());
    assert_eq!(author.publisher_id(), Some(identity.publisher_id));
    // Round-trip through serde so we know the trace can carry it.
    let json = serde_json::to_string(&author).unwrap();
    let back: efe_nano_sdk::Author = serde_json::from_str(&json).unwrap();
    assert!(back.is_publisher());
}

// ─────────────────────────────────────────────────────────────────────
//  8. co-publishing format round-trip
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_co_publishing_format_round_trip() {
    let policy = efe_nano_sdk::CoPublishingPolicy {
        signers: vec![PublisherId::new(), PublisherId::new(), PublisherId::new()],
        threshold: 2,
    };
    policy.validate().unwrap();
    let json = serde_json::to_string(&policy).unwrap();
    let back: efe_nano_sdk::CoPublishingPolicy = serde_json::from_str(&json).unwrap();
    assert_eq!(back, policy);
    // Invalid : threshold > signers.
    let bad = efe_nano_sdk::CoPublishingPolicy {
        signers: vec![PublisherId::new()],
        threshold: 2,
    };
    assert!(bad.validate().is_err());
}

// ─────────────────────────────────────────────────────────────────────
//  9. registry shared dispatch tenant-scoped
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_registry_shared_dispatch_tenant_scoped() {
    // The registry is a single source of truth that all tenants
    // dispatch against ; identities are platform-global by publisher
    // id but display-name uniqueness is tier-scoped (I10).
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([9u8; 32]);
    let id_a = fx
        .registry
        .register(candidate("SharedCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    // Same display name, different tier → allowed (different namespace).
    let id_b = fx
        .registry
        .register(candidate("SharedCo", KycTier::Tier0, signer.public_key_pem()))
        .await
        .unwrap();
    assert_ne!(id_a.publisher_id, id_b.publisher_id);
    assert_eq!(id_a.display_name, id_b.display_name);
    assert_ne!(id_a.kyc_tier, id_b.kyc_tier);

    // Shared lookup from a second Arc clone — same identities visible.
    let registry2: Arc<dyn PublisherRegistry> = fx.registry.clone();
    let again = registry2.lookup(id_a.publisher_id).await.unwrap();
    assert_eq!(again.publisher_id, id_a.publisher_id);
}

// ─────────────────────────────────────────────────────────────────────
//  10. publisher history returns ledger entries
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_publisher_history_returns_ledger_entries() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([10u8; 32]);
    let identity = fx
        .registry
        .register(candidate("HistoryCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;
    fx.ledger
        .append(PublishLedgerEntry::PublisherRegistered {
            identity: Box::new(identity.clone()),
        })
        .await
        .unwrap();
    fx.ledger
        .append(PublishLedgerEntry::PublisherSuspended {
            publisher_id,
            reason: "test".into(),
        })
        .await
        .unwrap();
    let history = fx.ledger.history(publisher_id, 10).await.unwrap();
    assert_eq!(history.len(), 2);
    assert!(matches!(history[0].entry, PublishLedgerEntry::PublisherRegistered { .. }));
    assert!(matches!(history[1].entry, PublishLedgerEntry::PublisherSuspended { .. }));
    assert!(history[1].prev_trace_id == Some(history[0].trace_id));
}

// ─────────────────────────────────────────────────────────────────────
//  11. sanctions flag suspends publisher
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_sanctions_flag_suspends_publisher() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([11u8; 32]);
    let identity = fx
        .registry
        .register(candidate("Sanctioned", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();

    // Initially clear.
    let v = fx.sanctions.check(&identity).await.unwrap();
    assert!(v.is_clear());
    fx.metrics.tick_sanctions_check(v.label());

    // Flag the publisher.
    fx.sanctions.flag("Sanctioned");
    let v = fx.sanctions.check(&identity).await.unwrap();
    assert!(matches!(v, SanctionsVerdict::Flagged { .. }));
    fx.metrics.tick_sanctions_check(v.label());

    // The orchestration layer (Sprint 11) will call `suspend` on a
    // flag.  Here we assert the suspend mechanism works.
    fx.registry
        .suspend(identity.publisher_id, "sanctions_match".into())
        .await
        .unwrap();
    let after = fx.registry.lookup(identity.publisher_id).await.unwrap();
    assert!(matches!(after.status, PublisherStatus::Suspended { .. }));
    assert!(!after.can_publish());
    assert!(after.dispatch_blocked());

    assert_eq!(fx.metrics.sanctions_checks_total("clear"), 1);
    assert_eq!(fx.metrics.sanctions_checks_total("flagged"), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  12. key rotation preserves old signatures
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_key_rotation_preserves_old_signatures() {
    let fx = build_fixture();
    let signer_old = Ed25519Signer::from_seed([12u8; 32]);
    let signer_new = Ed25519Signer::from_seed([212u8; 32]);
    let identity = fx
        .registry
        .register(candidate("RotateCo", KycTier::Tier2, signer_old.public_key_pem()))
        .await
        .unwrap();
    let old_key_id = identity.active_key.key_id;
    let old_pem = identity.active_key.public_key_pem.clone();
    let publisher_id = identity.publisher_id;

    // Sign before rotation.
    let (body, sig) = sign_canonical(
        &signer_old,
        publisher_id,
        old_key_id,
        "agent-z",
        "1.0.0",
        KycTier::Tier2,
    );

    // Rotate.
    let new_key_id = fx
        .registry
        .rotate_key(
            publisher_id,
            signer_new.public_key_pem(),
            SignatureAlgorithm::Ed25519,
        )
        .await
        .unwrap();
    assert_ne!(new_key_id, old_key_id);
    let after = fx.registry.lookup(publisher_id).await.unwrap();
    assert_eq!(after.active_key.key_id, new_key_id);
    assert_eq!(after.historical_keys.len(), 1);
    assert_eq!(after.historical_keys[0].key_id, old_key_id);

    // The signature pre-rotation still verifies under the historical
    // key (Architecture I7 + N17 replay invariant).
    fx.verifier
        .verify(&sig, &body.canonical_bytes(), &old_pem)
        .await
        .unwrap();
    fx.metrics.tick_key_rotation(SignatureAlgorithm::Ed25519.label());
    assert_eq!(fx.metrics.key_rotations_total("ed25519"), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  13. tier 0 pseudonym KYC separation
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_tier_0_pseudonym_kyc_separation() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([13u8; 32]);

    // Tier 0 with `pseudonymous=true` and no dossier → allowed.
    let candidate_ok = NewPublisherCandidate {
        display_name: "pseudonym_x".into(),
        target_tier: KycTier::Tier0,
        jurisdiction: vec![Iso3166::new("US").unwrap()],
        initial_public_key_pem: signer.public_key_pem(),
        algorithm: SignatureAlgorithm::Ed25519,
        pseudonymous: true,
        kyc_dossier_reference: None,
    };
    let id = fx.registry.register(candidate_ok).await.unwrap();
    assert_eq!(id.kyc_tier, KycTier::Tier0);
    assert!(id.kyc_dossier_reference.is_none());

    // Tier 2 with `pseudonymous=true` → rejected (I9).
    let candidate_bad = NewPublisherCandidate {
        display_name: "pseudonym_promoted".into(),
        target_tier: KycTier::Tier2,
        jurisdiction: vec![Iso3166::new("US").unwrap()],
        initial_public_key_pem: signer.public_key_pem(),
        algorithm: SignatureAlgorithm::Ed25519,
        pseudonymous: true,
        kyc_dossier_reference: None,
    };
    let err = fx.registry.register(candidate_bad).await.unwrap_err();
    let _ = err; // any invalid-candidate variant is acceptable

    // Tier 2 without dossier → rejected.
    let candidate_no_dossier = NewPublisherCandidate {
        display_name: "no_dossier".into(),
        target_tier: KycTier::Tier2,
        jurisdiction: vec![Iso3166::new("US").unwrap()],
        initial_public_key_pem: signer.public_key_pem(),
        algorithm: SignatureAlgorithm::Ed25519,
        pseudonymous: false,
        kyc_dossier_reference: None,
    };
    let err = fx.registry.register(candidate_no_dossier).await.unwrap_err();
    let _ = err;
}

// ─────────────────────────────────────────────────────────────────────
//  14. tier promotion recorded
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_tier_promotion_recorded() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([14u8; 32]);
    let identity = fx
        .registry
        .register(candidate("PromoteCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;

    // Tier 3 requires 2 approvals (G14).
    let approvals = vec![operator_approval(), operator_approval()];
    fx.registry
        .promote_tier(publisher_id, KycTier::Tier3, approvals.clone())
        .await
        .unwrap();
    let after = fx.registry.lookup(publisher_id).await.unwrap();
    assert_eq!(after.kyc_tier, KycTier::Tier3);

    // Append a ledger entry recording the promotion.
    fx.ledger
        .append(PublishLedgerEntry::PublisherTierPromoted {
            publisher_id,
            from: KycTier::Tier2,
            to: KycTier::Tier3,
            approvers: approvals,
        })
        .await
        .unwrap();
    let history = fx.ledger.history(publisher_id, 10).await.unwrap();
    assert!(history
        .iter()
        .any(|r| matches!(r.entry, PublishLedgerEntry::PublisherTierPromoted { .. })));

    // Downgrade is forbidden (I5).
    let err = fx
        .registry
        .promote_tier(publisher_id, KycTier::Tier1, vec![operator_approval()])
        .await
        .unwrap_err();
    let _ = err;
}

// ─────────────────────────────────────────────────────────────────────
//  15. backdated revocation lists window
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n18_backdated_revocation_lists_window() {
    let fx = build_fixture();
    let signer = Ed25519Signer::from_seed([15u8; 32]);
    let identity = fx
        .registry
        .register(candidate("BackdateCo", KycTier::Tier2, signer.public_key_pem()))
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;
    let suspected = SystemTime::now() - Duration::from_secs(2 * 3_600);

    // A version published right at suspected time + a later version.
    fx.registry
        .record_published_version(
            publisher_id,
            "agent-q".into(),
            "1.0.0".into(),
            suspected + Duration::from_secs(60),
        )
        .await
        .unwrap();
    fx.registry
        .record_published_version(
            publisher_id,
            "agent-q".into(),
            "1.0.1".into(),
            suspected + Duration::from_secs(3_500),
        )
        .await
        .unwrap();

    let cascade = fx
        .registry
        .revoke(
            publisher_id,
            RevocationReason::KeyCompromise,
            Some(suspected),
            Uuid::new_v4(),
        )
        .await
        .unwrap();

    // Replay window covers [suspected, revoked_at].
    let (start, end) = cascade.replay_window.unwrap();
    assert!(end >= start);
    assert!(start <= SystemTime::now());
    assert_eq!(cascade.revoked_versions.len(), 2);
    // Both versions have published_at >= suspected.
    for v in &cascade.revoked_versions {
        assert!(v.published_at >= suspected);
    }
}
