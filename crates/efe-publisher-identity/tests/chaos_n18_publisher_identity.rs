// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : chaos tests (architecture § 12.3).
//!
//! Three chaos scenarios from the design doc :
//!
//! 1. `chaos_sanctions_service_down` — F4 enforcement (Inconclusive
//!    verdict → SanctionsCheckStale flag).
//! 2. `chaos_revocation_mid_dispatch` — F6 + P14 (in-flight dispatch
//!    completes ; new dispatches blocked).
//! 3. `chaos_concurrent_key_rotations` — F8 race resolution.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use efe_nano_sdk::{
    Iso3166, KeyId, KycTier, NewPublisherCandidate, PublisherSignature, PublisherStatus,
    RevocationReason, Sha256Hash, SignatureAlgorithm,
};
use efe_publisher_identity::{
    CanonicalBody, Ed25519Signer, Ed25519Verifier, InMemoryKeyStore, InMemoryPublishLedger,
    InMemoryPublisherRegistry, KeyStore, PublishLedger, PublishLedgerEntry,
    PublisherRegistry, SanctionsVerdict, SignatureVerifier, StubSanctionsScreener,
};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  1. Sanctions service down — F4
// ─────────────────────────────────────────────────────────────────────

/// **F4** : sanctions screener returns `Inconclusive` → registry
/// must transition publisher to `SanctionsCheckStale` (defense in
/// depth — don't admit a sanctioned entity while service is degraded).
#[tokio::test]
async fn chaos_sanctions_service_down_marks_stale() {
    let registry = InMemoryPublisherRegistry::new();
    let sanctions = StubSanctionsScreener::clear();
    let signer = Ed25519Signer::from_seed([0xAAu8; 32]);
    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "Acme".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: signer.public_key_pem(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k-1".into()),
        })
        .await
        .unwrap();

    // Flip the screener into outage mode.
    sanctions.set_force_inconclusive(true);
    let v = registry
        .refresh_sanctions(identity.publisher_id, &sanctions)
        .await
        .unwrap();
    assert!(matches!(v, SanctionsVerdict::Inconclusive));

    let after = registry.lookup(identity.publisher_id).await.unwrap();
    assert!(matches!(after.status, PublisherStatus::SanctionsCheckStale));
}

/// **F4 recovery** : once the service comes back, a `Clear` verdict
/// restores `Active`.
#[tokio::test]
async fn chaos_sanctions_service_recovery_restores_active() {
    let registry = InMemoryPublisherRegistry::new();
    let sanctions = StubSanctionsScreener::clear();
    let signer = Ed25519Signer::from_seed([0xBBu8; 32]);
    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "RecoverCo".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: signer.public_key_pem(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k-2".into()),
        })
        .await
        .unwrap();

    // 1. Outage : stale.
    sanctions.set_force_inconclusive(true);
    let _ = registry
        .refresh_sanctions(identity.publisher_id, &sanctions)
        .await
        .unwrap();
    assert!(matches!(
        registry.lookup(identity.publisher_id).await.unwrap().status,
        PublisherStatus::SanctionsCheckStale
    ));

    // 2. Recovery : flip back, screening returns Clear → Active.
    sanctions.set_force_inconclusive(false);
    let v = registry
        .refresh_sanctions(identity.publisher_id, &sanctions)
        .await
        .unwrap();
    assert!(v.is_clear());
    let after = registry.lookup(identity.publisher_id).await.unwrap();
    assert!(matches!(after.status, PublisherStatus::Active));
}

// ─────────────────────────────────────────────────────────────────────
//  2. Revocation mid-dispatch — F6 + P14
// ─────────────────────────────────────────────────────────────────────

/// **F6 + P14** : revocation happens between two dispatches.  The
/// already-resolved signature remains verifiable (forensic) but a
/// post-revocation lookup of the publisher's status reflects
/// `Revoked` so the dispatch layer can reject new dispatches.
#[tokio::test]
async fn chaos_revocation_mid_dispatch_blocks_new_calls() {
    let registry = InMemoryPublisherRegistry::new();
    let verifier = Ed25519Verifier::new();
    let signer = Ed25519Signer::from_seed([0xCCu8; 32]);
    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "MidFlight".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: signer.public_key_pem(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k-3".into()),
        })
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;

    // Dispatch 1 : build a signed body BEFORE revocation.
    let body = CanonicalBody::new(
        "agent",
        "1.0.0",
        publisher_id,
        KycTier::Tier2,
        Sha256Hash::from_bytes([0u8; 32]),
        1_716_000_000,
    );
    let body_bytes = body.canonical_bytes();
    let sig_bytes = signer.sign(&body_bytes);
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&body_bytes);
    let d = h.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&d);
    let sig = PublisherSignature {
        publisher_id,
        key_id: identity.active_key.key_id,
        algorithm: SignatureAlgorithm::Ed25519,
        signature: sig_bytes.to_vec(),
        signed_at: SystemTime::now(),
        canonical_hash: Sha256Hash::from_bytes(hash),
    };
    // Signature OK pre-revocation.
    verifier
        .verify(&sig, &body_bytes, &identity.active_key.public_key_pem)
        .await
        .unwrap();

    // Revocation happens.
    let _ = registry
        .revoke(
            publisher_id,
            RevocationReason::KeyCompromise,
            None,
            Uuid::new_v4(),
        )
        .await
        .unwrap();

    // P14 : in-flight signature still verifies (forensic / I3).
    verifier
        .verify(&sig, &body_bytes, &identity.active_key.public_key_pem)
        .await
        .unwrap();

    // But the publisher's status is Revoked — dispatch layer must
    // block new calls.
    let after = registry.lookup(publisher_id).await.unwrap();
    assert_eq!(after.status, PublisherStatus::Revoked);
    assert!(after.dispatch_blocked());
}

// ─────────────────────────────────────────────────────────────────────
//  3. Concurrent key rotations — F8
// ─────────────────────────────────────────────────────────────────────

/// **F8** : two `rotate_key` calls race.  Both must observe linearised
/// outcomes ; the final state must reference exactly one active key,
/// and ALL keys ever installed must appear in `active_key` ∪
/// `historical_keys`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_concurrent_key_rotations_linearise() {
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let signer = Ed25519Signer::from_seed([0xDDu8; 32]);
    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "Rotator".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: signer.public_key_pem(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k-4".into()),
        })
        .await
        .unwrap();

    let publisher_id = identity.publisher_id;
    let mut handles = Vec::new();
    for i in 0..8u32 {
        let registry = registry.clone();
        handles.push(tokio::spawn(async move {
            let signer = Ed25519Signer::from_seed([i as u8 + 1; 32]);
            registry
                .rotate_key(
                    publisher_id,
                    signer.public_key_pem(),
                    SignatureAlgorithm::Ed25519,
                )
                .await
        }));
    }
    let mut new_kids: Vec<KeyId> = Vec::new();
    for h in handles {
        new_kids.push(h.await.unwrap().unwrap());
    }
    // All eight rotations succeeded.
    assert_eq!(new_kids.len(), 8);
    // The final state has exactly ONE active key.
    let after = registry.lookup(publisher_id).await.unwrap();
    assert!(new_kids.contains(&after.active_key.key_id));
    // Each rotation produced a unique new KeyId.
    let mut sorted = new_kids.clone();
    sorted.sort_by_key(|k| k.as_uuid());
    sorted.dedup();
    assert_eq!(sorted.len(), 8);
    // All historical + active KeyIds add up to ORIGINAL + 8 rotations
    // = 9 distinct keys.
    let mut all_kids = vec![after.active_key.key_id];
    for hk in &after.historical_keys {
        all_kids.push(hk.key_id);
    }
    all_kids.sort_by_key(|k| k.as_uuid());
    all_kids.dedup();
    assert_eq!(all_kids.len(), 9);
}

/// **F8 + rotate_key_with_proof** : verify that the proof-bound
/// rotation reliably rejects a forged proof under stress.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_rotate_with_proof_rejects_forgeries_under_load() {
    let registry = Arc::new(InMemoryPublisherRegistry::new());
    let verifier: Arc<dyn SignatureVerifier> = Arc::new(Ed25519Verifier::new());
    let ks = Arc::new(InMemoryKeyStore::new());

    let (initial_kid, initial_pem) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
    let _ = initial_kid;

    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "Forgery".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: initial_pem.clone(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k-5".into()),
        })
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;

    // 16 concurrent rotation attempts with forged (random) proofs.
    let mut handles = Vec::new();
    for i in 0..16u32 {
        let registry = registry.clone();
        let verifier = verifier.clone();
        handles.push(tokio::spawn(async move {
            let new_signer = Ed25519Signer::from_seed([i as u8 + 64; 32]);
            // Forged proof : 64 random bytes that won't verify.
            let fake_proof = vec![(i as u8).wrapping_mul(7); 64];
            registry
                .rotate_key_with_proof(
                    publisher_id,
                    new_signer.public_key_pem(),
                    SignatureAlgorithm::Ed25519,
                    fake_proof,
                    verifier.as_ref(),
                )
                .await
        }));
    }
    for h in handles {
        let res = h.await.unwrap();
        // Every forged attempt must fail.
        assert!(res.is_err());
    }
    // No rotation happened : active key is still the original.
    let after = registry.lookup(publisher_id).await.unwrap();
    assert_eq!(after.active_key.public_key_pem, initial_pem);
    assert!(after.historical_keys.is_empty());
}

// ─────────────────────────────────────────────────────────────────────
//  Cascade chaos — many versions, large revocation window
// ─────────────────────────────────────────────────────────────────────

/// **G15 stress** : revoke with a wide suspected_compromise_at window
/// and 200 published versions in-window.  The cascade must list ALL
/// 200 of them atomically and the ledger must record them all.
#[tokio::test]
async fn chaos_large_revocation_cascade_atomic() {
    let registry = InMemoryPublisherRegistry::new();
    let ledger = InMemoryPublishLedger::new([7u8; 32]);
    let signer = Ed25519Signer::from_seed([0xEEu8; 32]);
    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "CascadeStress".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: signer.public_key_pem(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k-6".into()),
        })
        .await
        .unwrap();
    let publisher_id = identity.publisher_id;
    let suspected = SystemTime::now() - Duration::from_secs(7 * 24 * 3_600);

    for i in 0..200u32 {
        registry
            .record_published_version(
                publisher_id,
                format!("agent-{i}"),
                "1.0.0".into(),
                suspected + Duration::from_secs(i as u64 * 60),
            )
            .await
            .unwrap();
    }
    let cascade = registry
        .revoke(
            publisher_id,
            RevocationReason::KeyCompromise,
            Some(suspected),
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    assert_eq!(cascade.revoked_versions.len(), 200);

    // Append all version revocations to the ledger.
    for v in &cascade.revoked_versions {
        ledger
            .append(PublishLedgerEntry::VersionRevoked {
                publisher_id,
                agent_id: v.agent_id.clone(),
                version: v.version.clone(),
            })
            .await
            .unwrap();
    }
    let report = ledger.verify_chain(publisher_id).await.unwrap();
    assert!(report.ok);
    assert_eq!(report.entries_verified, 200);
}
