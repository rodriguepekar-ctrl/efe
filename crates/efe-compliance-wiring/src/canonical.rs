// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + ADR-2 + Annex A1** : reconstruct the
//! `CanonicalBody` from a `(NanoAgent, PublisherSignature)` pair.
//!
//! The catalogue does NOT persist the canonical body bytes — only
//! the signature.  The verifier rebuilds the canonical body on
//! every verification.  This avoids the F2 risk of "the bytes
//! drifted from the struct" and keeps the wire size minimal.
//!
//! The recomputation is **deterministic** : it walks the same
//! field set the signer used (annex A2 of N18) :
//! - `schema_version`  : `CANONICAL_SCHEMA_VERSION` constant.
//! - `agent_id`        : `nano.id.as_str()`.
//! - `version`         : `nano.version.as_str()`.
//! - `publisher_id`    : `signature.publisher_id`.
//! - `kyc_tier`        : from `nano.author` if `Publisher`, else `Tier0`.
//! - `agent_body_sha256` : SHA-256 of a canonical serialisation of
//!   `nano` with `signature` and `co_publishing` stripped (chicken-
//!   and-egg : those fields depend on the signature itself).
//! - `timestamp_epoch_seconds` : derived from `signature.signed_at`.

use std::time::UNIX_EPOCH;

use efe_nano_sdk::{Author, KycTier, NanoAgent, PublisherSignature, Sha256Hash};
use efe_publisher_identity::canonical::{CanonicalBody, CANONICAL_SCHEMA_VERSION};
use sha2::{Digest, Sha256};

/// **N24 + Annex A1** : reconstruct the canonical body that the
/// signer hashed when producing `signature`.
///
/// The catalogue's `signature.canonical_hash` is then expected to
/// equal `SHA-256(canonical_body.canonical_bytes())` — the
/// underlying `Ed25519Verifier::verify` enforces this equality and
/// returns `SignatureError::CanonicalHashMismatch` on drift.
pub fn nano_to_canonical_body(
    nano: &NanoAgent,
    signature: &PublisherSignature,
) -> CanonicalBody {
    // KYC tier : `Author::Publisher { kyc_tier }` captures the
    // tier at sign time (architecture I9).  For non-publisher
    // authors we use Tier0 — these dispatch only under the grace
    // window per ADR-4.
    let kyc_tier = match &nano.author {
        Author::Publisher { kyc_tier, .. } => *kyc_tier,
        _ => KycTier::Tier0,
    };

    let agent_body_sha256 = compute_agent_body_sha256(nano);
    let timestamp_epoch_seconds = signature
        .signed_at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    CanonicalBody {
        schema_version: CANONICAL_SCHEMA_VERSION,
        agent_id: nano.id.as_str().to_string(),
        version: nano.version.as_str().to_string(),
        publisher_id: signature.publisher_id,
        kyc_tier,
        agent_body_sha256,
        timestamp_epoch_seconds,
    }
}

/// **N24** : compute the SHA-256 of a canonical serialisation of
/// the nano **minus the signature** (and `co_publishing`, which
/// also references the signature graph).
///
/// The serialisation is JSON via serde ; this is good enough for
/// Sprint 16 because the `NanoAgent` struct fields are declared
/// in a fixed order and serde_json's `Map` is `BTreeMap`-backed.
/// Sprint 18 may swap for CBOR if performance benchmarks demand.
pub fn compute_agent_body_sha256(nano: &NanoAgent) -> Sha256Hash {
    let mut clone = nano.clone();
    clone.signature = None;
    clone.co_publishing = None;
    let bytes = serde_json::to_vec(&clone).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(&bytes);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    Sha256Hash::from_bytes(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use efe_nano_sdk::{
        Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, KeyId, NanoAgentId,
        NanoAgentVersion, PublisherId, ResourceLimits, SignatureAlgorithm, StabilityTier,
    };
    use std::time::SystemTime;

    fn sample_nano(author: Author) -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test_agent").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author,
            category: Category::Extraction,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "test::run".into(),
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

    fn sample_signature(publisher_id: PublisherId, signed_at: SystemTime) -> PublisherSignature {
        PublisherSignature {
            publisher_id,
            key_id: KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            signature: vec![0u8; 64],
            signed_at,
            canonical_hash: Sha256Hash::from_bytes([0u8; 32]),
        }
    }

    #[test]
    fn canonical_body_publisher_author_keeps_tier() {
        let pub_id = PublisherId::new();
        let nano = sample_nano(Author::Publisher {
            publisher_id: pub_id,
            kyc_tier: KycTier::Tier2,
        });
        let sig = sample_signature(pub_id, SystemTime::now());
        let cb = nano_to_canonical_body(&nano, &sig);
        assert_eq!(cb.kyc_tier, KycTier::Tier2);
        assert_eq!(cb.publisher_id, pub_id);
    }

    #[test]
    fn canonical_body_non_publisher_author_defaults_tier0() {
        let nano = sample_nano(Author::Efe);
        let sig = sample_signature(PublisherId::new(), SystemTime::now());
        let cb = nano_to_canonical_body(&nano, &sig);
        assert_eq!(cb.kyc_tier, KycTier::Tier0);
    }

    #[test]
    fn canonical_body_carries_signature_publisher_id() {
        let author_pub = PublisherId::new();
        let signer_pub = PublisherId::new();
        let nano = sample_nano(Author::Publisher {
            publisher_id: author_pub,
            kyc_tier: KycTier::Tier1,
        });
        let sig = sample_signature(signer_pub, SystemTime::now());
        // The canonical body carries the SIGNATURE's publisher_id,
        // not the author's — the WiredSignatureGate is responsible
        // for asserting they match before this function is called.
        let cb = nano_to_canonical_body(&nano, &sig);
        assert_eq!(cb.publisher_id, signer_pub);
    }

    #[test]
    fn canonical_body_timestamp_derived_from_signed_at() {
        let signed_at =
            UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let nano = sample_nano(Author::Efe);
        let sig = sample_signature(PublisherId::new(), signed_at);
        let cb = nano_to_canonical_body(&nano, &sig);
        assert_eq!(cb.timestamp_epoch_seconds, 1_700_000_000);
    }

    #[test]
    fn canonical_body_uses_schema_version() {
        let nano = sample_nano(Author::Efe);
        let sig = sample_signature(PublisherId::new(), SystemTime::now());
        let cb = nano_to_canonical_body(&nano, &sig);
        assert_eq!(cb.schema_version, CANONICAL_SCHEMA_VERSION);
    }

    #[test]
    fn canonical_body_agent_id_and_version_match_nano() {
        let nano = sample_nano(Author::Efe);
        let sig = sample_signature(PublisherId::new(), SystemTime::now());
        let cb = nano_to_canonical_body(&nano, &sig);
        assert_eq!(cb.agent_id, "test_agent");
        assert_eq!(cb.version, "1.0.0");
    }

    #[test]
    fn agent_body_sha256_strips_signature() {
        // Hash without signature.
        let nano = sample_nano(Author::Efe);
        let h1 = compute_agent_body_sha256(&nano);

        // Add a signature ; hash MUST be unchanged (signature is
        // stripped before hashing).
        let mut nano2 = nano.clone();
        nano2.signature = Some(sample_signature(PublisherId::new(), SystemTime::now()));
        let h2 = compute_agent_body_sha256(&nano2);
        assert_eq!(h1, h2, "signature presence must not affect agent_body_sha256");
    }

    #[test]
    fn agent_body_sha256_strips_co_publishing() {
        // Hash without co_publishing.
        let nano = sample_nano(Author::Efe);
        let h1 = compute_agent_body_sha256(&nano);

        // Add a co_publishing policy ; hash MUST be unchanged.
        let mut nano2 = nano.clone();
        nano2.co_publishing = Some(efe_nano_sdk::CoPublishingPolicy {
            signers: vec![],
            threshold: 1,
        });
        let h2 = compute_agent_body_sha256(&nano2);
        assert_eq!(h1, h2);
    }

    #[test]
    fn agent_body_sha256_changes_with_id() {
        let nano1 = sample_nano(Author::Efe);
        let mut nano2 = nano1.clone();
        nano2.id = NanoAgentId::new("other_agent").unwrap();
        assert_ne!(
            compute_agent_body_sha256(&nano1),
            compute_agent_body_sha256(&nano2)
        );
    }

    #[test]
    fn agent_body_sha256_changes_with_version() {
        let nano1 = sample_nano(Author::Efe);
        let mut nano2 = nano1.clone();
        nano2.version = NanoAgentVersion::new("2.0.0").unwrap();
        assert_ne!(
            compute_agent_body_sha256(&nano1),
            compute_agent_body_sha256(&nano2)
        );
    }

    #[test]
    fn agent_body_sha256_stable_across_calls() {
        let nano = sample_nano(Author::Efe);
        let h1 = compute_agent_body_sha256(&nano);
        let h2 = compute_agent_body_sha256(&nano);
        assert_eq!(h1, h2);
    }

    #[test]
    fn canonical_bytes_produce_valid_json() {
        let nano = sample_nano(Author::Efe);
        let sig = sample_signature(PublisherId::new(), SystemTime::now());
        let cb = nano_to_canonical_body(&nano, &sig);
        let bytes = cb.canonical_bytes();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v.is_object());
        assert!(v["schema_version"].is_number());
        assert!(v["agent_id"].is_string());
    }
}
