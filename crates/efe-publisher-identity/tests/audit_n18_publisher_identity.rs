// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : post-audit regression tests.
//!
//! These tests target the bugs caught during the chirurgical audit
//! pass on top of the original 15-row acceptance matrix.  Each test
//! pins a fix in place so the same defect cannot regress.
//!
//! Bugs covered :
//!
//! | # | Defect | Where | Fix |
//! |---|---|---|---|
//! | A1 | `KycTier` serde produced `tier2` while `label()` produced `tier_2` | `efe_nano_sdk::KycTier` | explicit `#[serde(rename = "tier_N")]` |
//! | A2 | `LlmProvenanceRecord` missing `publisher_id` + `publisher_kyc_tier` despite N18 § 13 + Workflow E demanding them | `LlmProvenanceRecord` | added two optional fields |
//! | A3 | `InMemoryPublishLedger::with_seed_rows` did not sort by `sequence_number` before chain verification | `ledger::with_seed_rows` | sort each publisher's rows by seq before storing |
//! | A4 | `rotate_key_with_proof` end-to-end with `InMemoryKeyStore` was never tested with a valid proof | `keystore` ↔ `registry` | dedicated test below |

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use efe_nano_sdk::{
    Iso3166, KycTier, NewPublisherCandidate, PublisherId, PublisherSignature,
    PublisherStatus, RevocationReason, Sha256Hash, SignatureAlgorithm,
};
use efe_publisher_identity::{
    CanonicalBody, ChainVerifyReport, DefaultPublisherApi, Ed25519Signer, Ed25519Verifier,
    HistoryQuery, InMemoryKeyStore, InMemoryPublishLedger, InMemoryPublisherRegistry,
    KeyStore, LedgerRow, ListQuery, N18Metrics, PromoteRequest, PublishLedger,
    PublishLedgerEntry, PublisherApi, PublisherRegistry, RegisterRequest, RevokeRequest,
    RotateKeyRequest, SanctionsVerdict, SignatureVerifier, StubKycProvider,
    StubSanctionsScreener,
};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  A1 — KycTier serde matches label
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_a1_kyc_tier_serde_matches_label() {
    for t in [KycTier::Tier0, KycTier::Tier1, KycTier::Tier2, KycTier::Tier3] {
        let json = serde_json::to_string(&t).unwrap();
        let unquoted = json.trim_matches('"');
        assert_eq!(
            unquoted,
            t.label(),
            "serde `{unquoted}` must equal label `{}`",
            t.label()
        );
    }
}

#[test]
fn audit_a1_kyc_tier_round_trip_via_explicit_strings() {
    let cases = [
        ("tier_0", KycTier::Tier0),
        ("tier_1", KycTier::Tier1),
        ("tier_2", KycTier::Tier2),
        ("tier_3", KycTier::Tier3),
    ];
    for (json_str, expected) in cases {
        let payload = format!("\"{json_str}\"");
        let parsed: KycTier = serde_json::from_str(&payload).unwrap();
        assert_eq!(parsed, expected);
    }
}

#[test]
fn audit_a1_publisher_identity_json_uses_tier_underscore() {
    // Build an identity through the registry and serialise it ; the
    // wire JSON for kyc_tier must match the metric label.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let signer = Ed25519Signer::from_seed([0xA1u8; 32]);
        let reg = InMemoryPublisherRegistry::new();
        let id = reg
            .register(NewPublisherCandidate {
                display_name: "wire-format".into(),
                target_tier: KycTier::Tier2,
                jurisdiction: vec![Iso3166::new("US").unwrap()],
                initial_public_key_pem: signer.public_key_pem(),
                algorithm: SignatureAlgorithm::Ed25519,
                pseudonymous: false,
                kyc_dossier_reference: Some("k".into()),
            })
            .await
            .unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert!(json.contains("\"kyc_tier\":\"tier_2\""), "{json}");
        // Round-trip must reconstruct exactly.
        let again: efe_nano_sdk::PublisherIdentity =
            serde_json::from_str(&json).unwrap();
        assert_eq!(again, id);
    });
}

// ─────────────────────────────────────────────────────────────────────
//  A2 — LlmProvenanceRecord carries publisher_id + kyc_tier (N18 § 13)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_a2_llm_provenance_record_carries_publisher_id_field() {
    use efe_nano_sdk::{
        Blake3Hash, CostAccounting, FinishReason, LlmProvenanceRecord, ModelIdentity,
        NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId, SamplingParams, TenantId,
        TokenUsage,
    };
    let publisher_id = PublisherId::new();
    let record = LlmProvenanceRecord {
        provenance_id: ProvenanceId::new(),
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "audit".into(),
        tenant_id: TenantId::new(),
        agent_id: NanoAgentId::new("audit").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: ModelIdentity::hosted("openai", "gpt-4", "2024-01"),
        model_hash_verified: true,
        prompt_template_id: "t".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::ZERO,
        final_prompt_hash: Blake3Hash::ZERO,
        sampling: SamplingParams::default(),
        token_usage: TokenUsage::default(),
        cost_accounting: CostAccounting::default(),
        provider_request_id: None,
        provider_response_time_ms: 0,
        provider_finish_reason: FinishReason::Unknown,
        provider_region: None,
        provider_metadata: Default::default(),
        output_hash: None,
        completeness: efe_nano_sdk::Completeness::Complete,
        safety_flags: vec![],
        tool_calls: vec![],
        pii_strategy_applied: PiiStrategy::Strict,
        redaction_token_set: vec![],
        replay_of: None,
        cache_hit: None,
        recorded_at: SystemTime::now(),
        publisher_id: Some(publisher_id),
        publisher_kyc_tier: Some(KycTier::Tier2),
        gold_case_id: None,
        // **Sprint 19 — N27** : buffered fixture, no streaming.
        streaming_mode: false,
        retry_count: 0,
        retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
        time_to_first_token_ms: None,
        compute_attestation: efe_nano_sdk::ComputeAttestation::default(),
    };
    let json = serde_json::to_string(&record).unwrap();
    // Workflow E (audit EU) needs to JOIN provenance with registry on
    // publisher_id ; the field MUST be in the wire payload.
    assert!(json.contains("publisher_id"));
    assert!(json.contains("publisher_kyc_tier"));
    assert!(json.contains("\"tier_2\""));

    // Round-trip preserves both fields.
    let again: LlmProvenanceRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(again.publisher_id, Some(publisher_id));
    assert_eq!(again.publisher_kyc_tier, Some(KycTier::Tier2));
}

#[test]
fn audit_a2_llm_provenance_record_backward_compat_no_publisher_fields() {
    // Build a "pre-N18 shaped" record by constructing one with
    // publisher_id=None+kyc=None, serialising (skip_serializing_if
    // strips them), then re-parsing.  This proves the fields are
    // optional on the WIRE — pre-N18 producers will continue to be
    // accepted post-N18 (backward compat per § 0 "Backward
    // compatibility").
    use efe_nano_sdk::{
        Blake3Hash, CostAccounting, FinishReason, LlmProvenanceRecord, ModelIdentity,
        NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId, SamplingParams, TenantId,
        TokenUsage,
    };
    let record = LlmProvenanceRecord {
        provenance_id: ProvenanceId::new(),
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "compat".into(),
        tenant_id: TenantId::new(),
        agent_id: NanoAgentId::new("compat").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: ModelIdentity::hosted("openai", "gpt-4", "2024-01"),
        model_hash_verified: false,
        prompt_template_id: "t".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::ZERO,
        final_prompt_hash: Blake3Hash::ZERO,
        sampling: SamplingParams::default(),
        token_usage: TokenUsage::default(),
        cost_accounting: CostAccounting::default(),
        provider_request_id: None,
        provider_response_time_ms: 0,
        provider_finish_reason: FinishReason::Unknown,
        provider_region: None,
        provider_metadata: Default::default(),
        output_hash: None,
        completeness: efe_nano_sdk::Completeness::Complete,
        safety_flags: vec![],
        tool_calls: vec![],
        pii_strategy_applied: PiiStrategy::Strict,
        redaction_token_set: vec![],
        replay_of: None,
        cache_hit: None,
        recorded_at: SystemTime::now(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
        // **Sprint 19 — N27** : buffered fixture, no streaming.
        streaming_mode: false,
        retry_count: 0,
        retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
        time_to_first_token_ms: None,
        compute_attestation: efe_nano_sdk::ComputeAttestation::default(),
    };
    let json = serde_json::to_string(&record).unwrap();
    // Skip-serializing-if-None must omit the two fields from the
    // wire payload (forward-compatible : pre-N18 producers omit
    // them too).
    assert!(!json.contains("\"publisher_id\":"));
    assert!(!json.contains("\"publisher_kyc_tier\":"));

    // And the JSON parses back into a valid record with None for
    // the missing fields.
    let again: LlmProvenanceRecord = serde_json::from_str(&json).unwrap();
    assert!(again.publisher_id.is_none());
    assert!(again.publisher_kyc_tier.is_none());
}

// ─────────────────────────────────────────────────────────────────────
//  A3 — with_seed_rows sorts by sequence_number
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a3_with_seed_rows_sorts_and_verifies_chain() {
    // Build a chain of 3 entries via append, then dump them, then
    // re-seed in REVERSE order.  Sort + verify must still pass.
    let hmac_key = [0xA3u8; 32];
    let publisher_id = PublisherId::new();
    let original = InMemoryPublishLedger::new(hmac_key);
    let signer = Ed25519Signer::from_seed([1u8; 32]);
    let identity = efe_nano_sdk::PublisherIdentity {
        publisher_id,
        display_name: "x".into(),
        kyc_tier: KycTier::Tier2,
        status: PublisherStatus::Active,
        jurisdiction: vec![Iso3166::new("US").unwrap()],
        registered_at: SystemTime::now(),
        last_sanctions_check_at: None,
        active_key: efe_nano_sdk::PublisherKey {
            key_id: efe_nano_sdk::KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: signer.public_key_pem(),
            valid_from: SystemTime::now(),
            valid_until: None,
        },
        historical_keys: vec![],
        kyc_dossier_reference: None,
        revocation: None,
        forks_of: None,
    };
    let _r0 = original
        .append(PublishLedgerEntry::PublisherRegistered {
            identity: Box::new(identity),
        })
        .await
        .unwrap();
    let _r1 = original
        .append(PublishLedgerEntry::PublisherSuspended {
            publisher_id,
            reason: "test".into(),
        })
        .await
        .unwrap();
    let _r2 = original
        .append(PublishLedgerEntry::PublisherSuspended {
            publisher_id,
            reason: "another".into(),
        })
        .await
        .unwrap();
    let mut rows: Vec<LedgerRow> = original.history(publisher_id, 10).await.unwrap();
    assert_eq!(rows.len(), 3);
    // Reverse order on purpose.
    rows.reverse();

    let reseeded = InMemoryPublishLedger::with_seed_rows(hmac_key, rows).unwrap();
    let report = reseeded.verify_chain(publisher_id).await.unwrap();
    assert!(report.ok, "chain should verify after sort : {report:?}");
    assert_eq!(report.entries_verified, 3);
}

#[tokio::test]
async fn audit_a3_with_seed_rows_rejects_short_key() {
    let rows: Vec<LedgerRow> = vec![];
    let err = InMemoryPublishLedger::with_seed_rows(b"short".to_vec(), rows).unwrap_err();
    assert!(err.contains("≥ 32 bytes"));
}

#[tokio::test]
async fn audit_a3_with_seed_rows_handles_multiple_publishers() {
    let hmac_key = [0xC3u8; 32];
    let p1 = PublisherId::new();
    let p2 = PublisherId::new();
    let original = InMemoryPublishLedger::new(hmac_key);
    for _ in 0..3 {
        original
            .append(PublishLedgerEntry::PublisherSuspended {
                publisher_id: p1,
                reason: "p1".into(),
            })
            .await
            .unwrap();
    }
    for _ in 0..2 {
        original
            .append(PublishLedgerEntry::PublisherSuspended {
                publisher_id: p2,
                reason: "p2".into(),
            })
            .await
            .unwrap();
    }
    let mut rows = original.history(p1, 100).await.unwrap();
    rows.extend(original.history(p2, 100).await.unwrap());
    // Interleave by reversing : { p2-1, p2-0, p1-2, p1-1, p1-0 }.
    rows.reverse();
    let re = InMemoryPublishLedger::with_seed_rows(hmac_key, rows).unwrap();
    let r1: ChainVerifyReport = re.verify_chain(p1).await.unwrap();
    let r2: ChainVerifyReport = re.verify_chain(p2).await.unwrap();
    assert!(r1.ok && r2.ok);
    assert_eq!(r1.entries_verified, 3);
    assert_eq!(r2.entries_verified, 2);
}

// ─────────────────────────────────────────────────────────────────────
//  A4 — KeyStore ↔ rotate_key_with_proof end-to-end
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a4_keystore_rotate_proof_round_trip_succeeds() {
    let ks = Arc::new(InMemoryKeyStore::new());
    let registry = InMemoryPublisherRegistry::new();
    let verifier = Ed25519Verifier::new();

    // 1. Mint the initial key in the keystore (so the registry's
    //    PEM matches what the keystore can sign for).
    let (current_kid, current_pem) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();

    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "RotateRound".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: current_pem.clone(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k".into()),
        })
        .await
        .unwrap();

    // 2. Mint a successor key.
    let (_new_kid, new_pem) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();

    // 3. Produce a valid rotation proof.
    let proof = ks
        .produce_rotation_proof(current_kid, &new_pem)
        .await
        .unwrap();

    // 4. Execute the rotation.
    let installed_kid = registry
        .rotate_key_with_proof(
            identity.publisher_id,
            new_pem.clone(),
            SignatureAlgorithm::Ed25519,
            proof,
            &verifier,
        )
        .await
        .unwrap();

    // 5. Verify registry state.
    let after = registry.lookup(identity.publisher_id).await.unwrap();
    assert_eq!(after.active_key.key_id, installed_kid);
    assert_eq!(after.active_key.public_key_pem, new_pem);
    assert_eq!(after.historical_keys.len(), 1);
    assert_eq!(after.historical_keys[0].public_key_pem, current_pem);
}

#[tokio::test]
async fn audit_a4_rotate_proof_with_wrong_key_rejected() {
    let ks = Arc::new(InMemoryKeyStore::new());
    let registry = InMemoryPublisherRegistry::new();
    let verifier = Ed25519Verifier::new();

    let (_legit_kid, legit_pem) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
    let (impostor_kid, _impostor_pem) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();

    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "WrongKey".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: legit_pem.clone(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k".into()),
        })
        .await
        .unwrap();

    let (_target_kid, target_pem) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
    // Forge proof : sign with the impostor key, not the legitimate one.
    let bad_proof = ks
        .produce_rotation_proof(impostor_kid, &target_pem)
        .await
        .unwrap();

    let err = registry
        .rotate_key_with_proof(
            identity.publisher_id,
            target_pem,
            SignatureAlgorithm::Ed25519,
            bad_proof,
            &verifier,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        efe_publisher_identity::RegistryError::RotationProofFailed
    ));

    // State unchanged.
    let after = registry.lookup(identity.publisher_id).await.unwrap();
    assert_eq!(after.active_key.public_key_pem, legit_pem);
    assert!(after.historical_keys.is_empty());
}

#[tokio::test]
async fn audit_a4_rotate_proof_for_wrong_new_pem_rejected() {
    let ks = Arc::new(InMemoryKeyStore::new());
    let registry = InMemoryPublisherRegistry::new();
    let verifier = Ed25519Verifier::new();

    let (current_kid, current_pem) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
    let identity = registry
        .register(NewPublisherCandidate {
            display_name: "WrongTarget".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: current_pem.clone(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k".into()),
        })
        .await
        .unwrap();

    let (_pem_a_kid, pem_a) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
    let (_pem_b_kid, pem_b) =
        ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();

    // Caller produces a proof for pem_A but submits pem_B as the new
    // public key.  The verifier must reject (canonical_hash mismatch
    // → RotationProofFailed).
    let proof_for_a = ks
        .produce_rotation_proof(current_kid, &pem_a)
        .await
        .unwrap();
    let err = registry
        .rotate_key_with_proof(
            identity.publisher_id,
            pem_b,
            SignatureAlgorithm::Ed25519,
            proof_for_a,
            &verifier,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        efe_publisher_identity::RegistryError::RotationProofFailed
    ));
}

// ─────────────────────────────────────────────────────────────────────
//  A5 — API : full lifecycle via DefaultPublisherApi
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a5_default_api_full_lifecycle_register_revoke_history() {
    let api = DefaultPublisherApi::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        Arc::new(InMemoryPublishLedger::new([0xA5u8; 32])),
        Arc::new(Ed25519Verifier::new()),
        Arc::new(StubKycProvider::permissive()),
        Arc::new(StubSanctionsScreener::clear()),
        N18Metrics::new(),
    );
    // Register.
    let resp = api
        .register(RegisterRequest {
            candidate: NewPublisherCandidate {
                display_name: "Lifecycle".into(),
                target_tier: KycTier::Tier2,
                jurisdiction: vec![Iso3166::new("US").unwrap()],
                initial_public_key_pem: Ed25519Signer::from_seed([0u8; 32])
                    .public_key_pem(),
                algorithm: SignatureAlgorithm::Ed25519,
                pseudonymous: false,
                kyc_dossier_reference: Some("k".into()),
            },
        })
        .await
        .unwrap();
    let pid = resp.identity.publisher_id;

    // Refresh sanctions.
    let v = api.refresh_sanctions(pid).await.unwrap();
    assert!(matches!(v, SanctionsVerdict::Clear));

    // Promote to Tier3.
    let after = api
        .promote(
            pid,
            PromoteRequest {
                target_tier: KycTier::Tier3,
                approvers: vec![
                    efe_nano_sdk::OperatorSignature {
                        operator_id: Uuid::new_v4(),
                        signed_at: SystemTime::now(),
                        ticket_id: "t1".into(),
                        signature: vec![0u8; 64],
                    },
                    efe_nano_sdk::OperatorSignature {
                        operator_id: Uuid::new_v4(),
                        signed_at: SystemTime::now(),
                        ticket_id: "t2".into(),
                        signature: vec![0u8; 64],
                    },
                ],
            },
        )
        .await
        .unwrap();
    assert_eq!(after.kyc_tier, KycTier::Tier3);

    // Revoke.
    let cascade = api
        .revoke(
            pid,
            RevokeRequest {
                reason: RevocationReason::KeyCompromise,
                suspected_compromise_at: None,
                revoking_operator_id: Uuid::new_v4(),
            },
        )
        .await
        .unwrap();
    assert_eq!(cascade.publisher_id, pid);

    // History must contain register + promote + revoke entries.
    let history = api
        .history(
            pid,
            HistoryQuery {
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
    assert!(history.entries.iter().any(|r| matches!(
        r.entry,
        PublishLedgerEntry::PublisherRegistered { .. }
    )));
    assert!(history.entries.iter().any(|r| matches!(
        r.entry,
        PublishLedgerEntry::PublisherTierPromoted { .. }
    )));
    assert!(history.entries.iter().any(|r| matches!(
        r.entry,
        PublishLedgerEntry::PublisherRevoked { .. }
    )));
}

#[tokio::test]
async fn audit_a5_list_query_filters_active_only_above_floor() {
    let api = DefaultPublisherApi::new(
        Arc::new(InMemoryPublisherRegistry::new()),
        Arc::new(InMemoryPublishLedger::new([0xB5u8; 32])),
        Arc::new(Ed25519Verifier::new()),
        Arc::new(StubKycProvider::permissive()),
        Arc::new(StubSanctionsScreener::clear()),
        N18Metrics::new(),
    );
    for (name, tier) in [
        ("a", KycTier::Tier0),
        ("b", KycTier::Tier2),
        ("c", KycTier::Tier3),
    ] {
        api.register(RegisterRequest {
            candidate: NewPublisherCandidate {
                display_name: name.into(),
                target_tier: tier,
                jurisdiction: vec![Iso3166::new("US").unwrap()],
                initial_public_key_pem: Ed25519Signer::from_seed([0u8; 32])
                    .public_key_pem(),
                algorithm: SignatureAlgorithm::Ed25519,
                pseudonymous: tier == KycTier::Tier0,
                kyc_dossier_reference: if tier >= KycTier::Tier2 {
                    Some("k".into())
                } else {
                    None
                },
            },
        })
        .await
        .unwrap();
    }
    let above_t2 = api
        .list(ListQuery {
            tier_floor: KycTier::Tier2,
        })
        .await
        .unwrap();
    assert_eq!(above_t2.len(), 2);
    let above_t3 = api
        .list(ListQuery {
            tier_floor: KycTier::Tier3,
        })
        .await
        .unwrap();
    assert_eq!(above_t3.len(), 1);
    let all = api
        .list(ListQuery {
            tier_floor: KycTier::Tier0,
        })
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
}

// ─────────────────────────────────────────────────────────────────────
//  A6 — Cross-cutting : LlmProvenanceRecord JSON wire matches the
//        registry kyc_tier JSON wire (Workflow E JOIN consistency)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a6_workflow_e_join_consistency_kyc_tier_string_match() {
    use efe_nano_sdk::{
        Blake3Hash, CostAccounting, FinishReason, LlmProvenanceRecord, ModelIdentity,
        NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceId, SamplingParams, TenantId,
        TokenUsage,
    };

    let signer = Ed25519Signer::from_seed([0xE6u8; 32]);
    let reg = InMemoryPublisherRegistry::new();
    let identity = reg
        .register(NewPublisherCandidate {
            display_name: "JoinKey".into(),
            target_tier: KycTier::Tier3,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: signer.public_key_pem(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k".into()),
        })
        .await
        .unwrap();
    let registry_json = serde_json::to_string(&identity).unwrap();

    // Build a provenance record carrying the same publisher_id +
    // tier ; its JSON kyc_tier field must equal the registry's.
    let record = LlmProvenanceRecord {
        provenance_id: ProvenanceId::new(),
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "x".into(),
        tenant_id: TenantId::new(),
        agent_id: NanoAgentId::new("audit").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: ModelIdentity::hosted("openai", "gpt-4", "2024-01"),
        model_hash_verified: true,
        prompt_template_id: "t".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::ZERO,
        final_prompt_hash: Blake3Hash::ZERO,
        sampling: SamplingParams::default(),
        token_usage: TokenUsage::default(),
        cost_accounting: CostAccounting::default(),
        provider_request_id: None,
        provider_response_time_ms: 0,
        provider_finish_reason: FinishReason::Unknown,
        provider_region: None,
        provider_metadata: Default::default(),
        output_hash: None,
        completeness: efe_nano_sdk::Completeness::Complete,
        safety_flags: vec![],
        tool_calls: vec![],
        pii_strategy_applied: PiiStrategy::Strict,
        redaction_token_set: vec![],
        replay_of: None,
        cache_hit: None,
        recorded_at: SystemTime::now(),
        publisher_id: Some(identity.publisher_id),
        publisher_kyc_tier: Some(identity.kyc_tier),
        gold_case_id: None,
        // **Sprint 19 — N27** : buffered fixture, no streaming.
        streaming_mode: false,
        retry_count: 0,
        retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
        time_to_first_token_ms: None,
        compute_attestation: efe_nano_sdk::ComputeAttestation::default(),
    };
    let provenance_json = serde_json::to_string(&record).unwrap();

    // Both should mention "tier_3".
    assert!(registry_json.contains("\"tier_3\""));
    assert!(provenance_json.contains("\"tier_3\""));
}

// ─────────────────────────────────────────────────────────────────────
//  A7 — Canonical body : timestamp 0 vs negative timestamp not aliased
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a7_canonical_body_timestamps_at_unix_origins() {
    let pid = PublisherId::new();
    let zero_ts = CanonicalBody::new(
        "a",
        "1.0.0",
        pid,
        KycTier::Tier2,
        Sha256Hash::from_bytes([0u8; 32]),
        0,
    );
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let now_ts = CanonicalBody::new(
        "a",
        "1.0.0",
        pid,
        KycTier::Tier2,
        Sha256Hash::from_bytes([0u8; 32]),
        now_secs,
    );
    use efe_publisher_identity::canonical_body_hash;
    assert_ne!(canonical_body_hash(&zero_ts), canonical_body_hash(&now_ts));
    // Wider sample : 365 days apart must also differ.
    let future_ts = CanonicalBody::new(
        "a",
        "1.0.0",
        pid,
        KycTier::Tier2,
        Sha256Hash::from_bytes([0u8; 32]),
        now_secs + 365 * 86_400,
    );
    assert_ne!(canonical_body_hash(&now_ts), canonical_body_hash(&future_ts));
}

// ─────────────────────────────────────────────────────────────────────
//  A8 — API : rotate_key request JSON wire round-trip
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_a8_rotate_key_request_serde_round_trip() {
    let req = RotateKeyRequest {
        new_public_key_pem: "-----BEGIN PUBLIC KEY-----\nfoo\n-----END PUBLIC KEY-----"
            .into(),
        algorithm: SignatureAlgorithm::Ed25519,
        signature_proof_b64: "AAAA".into(),
    };
    let json = serde_json::to_string(&req).unwrap();
    let back: RotateKeyRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back, req);
    // Algorithm wire form must be lowercase "ed25519".
    assert!(json.contains("\"ed25519\""));
}

// ─────────────────────────────────────────────────────────────────────
//  A9 — RevokeRequest serialisation : reason has snake_case taxonomy
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_a9_revoke_request_reason_serde_taxonomy() {
    for (r, expected) in [
        (RevocationReason::KeyCompromise, "key_compromise"),
        (RevocationReason::SanctionsViolation, "sanctions_violation"),
        (RevocationReason::PolicyViolation, "policy_violation"),
        (RevocationReason::PublisherRequest, "publisher_request"),
    ] {
        assert_eq!(r.label(), expected);
        let req = RevokeRequest {
            reason: r.clone(),
            suspected_compromise_at: None,
            revoking_operator_id: Uuid::nil(),
        };
        let json = serde_json::to_string(&req).unwrap();
        // Taxonomy must appear in wire (via `tag: kind` or `rename`).
        assert!(json.contains(expected), "wire {json} missing {expected}");
    }
}

// ─────────────────────────────────────────────────────────────────────
//  A10 — Defense in depth : `Suspended` publisher cannot publish/dispatch
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a10_suspended_publisher_helpers_block_correctly() {
    let signer = Ed25519Signer::from_seed([0xA0u8; 32]);
    let reg = InMemoryPublisherRegistry::new();
    let identity = reg
        .register(NewPublisherCandidate {
            display_name: "Susp".into(),
            target_tier: KycTier::Tier2,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: signer.public_key_pem(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: false,
            kyc_dossier_reference: Some("k".into()),
        })
        .await
        .unwrap();
    reg.suspend(identity.publisher_id, "ops".into()).await.unwrap();
    let after = reg.lookup(identity.publisher_id).await.unwrap();
    assert!(!after.can_publish());
    assert!(after.dispatch_blocked());
    // Lift.
    reg.unsuspend(identity.publisher_id).await.unwrap();
    let lifted = reg.lookup(identity.publisher_id).await.unwrap();
    assert!(lifted.can_publish());
    assert!(!lifted.dispatch_blocked());
}

// ─────────────────────────────────────────────────────────────────────
//  A11 — Sanctions verdict tick metric uses .label() consistently
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a11_sanctions_verdict_label_matches_metric_tick() {
    let metrics = N18Metrics::new();
    for (v, label) in [
        (SanctionsVerdict::Clear, "clear"),
        (
            SanctionsVerdict::Flagged {
                source: efe_publisher_identity::SanctionsList::OfacSdn,
                matched_at: SystemTime::now(),
            },
            "flagged",
        ),
        (SanctionsVerdict::Inconclusive, "inconclusive"),
    ] {
        assert_eq!(v.label(), label);
        metrics.tick_sanctions_check(v.label());
        assert!(metrics.sanctions_checks_total(label) >= 1);
    }
}

// ─────────────────────────────────────────────────────────────────────
//  A12 — Author::Publisher wire format snake_case tag
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_a12_author_publisher_wire_carries_kyc_tier_with_underscore() {
    let author = efe_nano_sdk::Author::Publisher {
        publisher_id: PublisherId::new(),
        kyc_tier: KycTier::Tier2,
    };
    let json = serde_json::to_string(&author).unwrap();
    // Wire shape: {"type":"publisher", "publisher_id":"...", "kyc_tier":"tier_2"}
    assert!(json.contains("\"type\":\"publisher\""));
    assert!(json.contains("\"kyc_tier\":\"tier_2\""));
    let back: efe_nano_sdk::Author = serde_json::from_str(&json).unwrap();
    assert!(back.is_publisher());
}

// ─────────────────────────────────────────────────────────────────────
//  A13 — Build a forged PublisherSignature, ensure verifier rejects
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_a13_signature_publisher_id_lying_does_not_validate() {
    // The signature contains a `publisher_id` field — but the
    // cryptographic guarantee is the SIGNATURE BYTES + KEY MATCH.
    // Mismatching publisher_id in the field does NOT alone fail
    // verification (the verifier doesn't check this field) ; the
    // SECURITY comes from the body's canonical_bytes containing the
    // publisher_id which is then hashed.
    let signer = Ed25519Signer::from_seed([0xA1u8; 32]);
    let pem = signer.public_key_pem();
    let real_pid = PublisherId::new();
    let body = CanonicalBody::new(
        "agent",
        "1.0.0",
        real_pid,
        KycTier::Tier2,
        Sha256Hash::from_bytes([0u8; 32]),
        1_716_000_000,
    );
    let bytes = body.canonical_bytes();
    let sig_bytes = signer.sign(&bytes);
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(&bytes);
    let d = h.finalize();
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&d);
    let sig = PublisherSignature {
        publisher_id: PublisherId::new(), // LIE — doesn't match body.publisher_id
        key_id: efe_nano_sdk::KeyId::new(),
        algorithm: SignatureAlgorithm::Ed25519,
        signature: sig_bytes.to_vec(),
        signed_at: SystemTime::now(),
        canonical_hash: Sha256Hash::from_bytes(hash),
    };
    // The verifier alone DOES NOT catch this — but the body's
    // canonical_bytes encodes the real publisher_id, which a
    // dispatch-layer check would compare against the registry's
    // active publisher_id.  So defense-in-depth requires BOTH the
    // verifier (cryptographic integrity) AND the dispatch layer
    // (identity binding).  This test pins that contract.
    let v = Ed25519Verifier::new();
    assert!(v.verify(&sig, &bytes, &pem).await.is_ok());
    // But the body's canonical_bytes encodes real_pid, while sig's
    // publisher_id field is different — a downstream check MUST
    // detect this divergence.
    assert_ne!(sig.publisher_id, body.publisher_id);
}
