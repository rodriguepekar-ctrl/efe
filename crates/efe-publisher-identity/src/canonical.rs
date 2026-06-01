// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : canonical body hashing.
//!
//! Architecture annex A2.  The signature covers a stable hash over
//! the agent's identity + body bytes.

use efe_nano_sdk::{KycTier, PublisherId, Sha256Hash};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// **Sprint 10 — N18** : canonical body for hashing + signing.
///
/// Fields are explicitly ordered ; serialisation uses a stable JSON
/// representation.  Any change here is a wire-incompatibility +
/// breaks all existing signatures — bump `CANONICAL_SCHEMA_VERSION`
/// if you must.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBody {
    pub schema_version: u32,
    pub agent_id: String,
    pub version: String,
    pub publisher_id: PublisherId,
    pub kyc_tier: KycTier,
    /// SHA-256 of the agent body bytes (caller responsibility ; the
    /// agent serialisation is mode-specific).
    pub agent_body_sha256: Sha256Hash,
    /// Epoch seconds at sign time.
    pub timestamp_epoch_seconds: i64,
}

/// **Sprint 10 — N18** : current canonical schema version.
pub const CANONICAL_SCHEMA_VERSION: u32 = 1;

impl CanonicalBody {
    /// **N18** : Construct with the current canonical schema version.
    pub fn new(
        agent_id: impl Into<String>,
        version: impl Into<String>,
        publisher_id: PublisherId,
        kyc_tier: KycTier,
        agent_body_sha256: Sha256Hash,
        timestamp_epoch_seconds: i64,
    ) -> Self {
        Self {
            schema_version: CANONICAL_SCHEMA_VERSION,
            agent_id: agent_id.into(),
            version: version.into(),
            publisher_id,
            kyc_tier,
            agent_body_sha256,
            timestamp_epoch_seconds,
        }
    }

    /// **N18** : Serialise to canonical bytes for hashing.
    ///
    /// Uses serde_json with sorted keys for stability.  In production
    /// (Sprint 11+) we may switch to CBOR — the field schema is the
    /// same.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // serde_json is stable for our struct field order (declaration
        // order).  Sort by field name explicitly here for stability
        // across rustc versions.
        let v = serde_json::json!({
            "agent_body_sha256": self.agent_body_sha256.to_hex(),
            "agent_id": self.agent_id,
            "kyc_tier": self.kyc_tier.label(),
            "publisher_id": self.publisher_id.as_uuid().to_string(),
            "schema_version": self.schema_version,
            "timestamp_epoch_seconds": self.timestamp_epoch_seconds,
            "version": self.version,
        });
        serde_json::to_vec(&v).expect("canonical body always serialises")
    }
}

/// **Sprint 10 — N18** : compute the canonical body hash to be signed.
pub fn canonical_body_hash(body: &CanonicalBody) -> Sha256Hash {
    let bytes = body.canonical_bytes();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Sha256Hash::from_bytes(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> CanonicalBody {
        CanonicalBody::new(
            "summarize_strict",
            "1.2.3",
            PublisherId::new(),
            KycTier::Tier2,
            Sha256Hash::from_bytes([0xAB; 32]),
            1_716_300_000,
        )
    }

    #[test]
    fn canonical_body_hash_stable_across_calls() {
        let b = body();
        let h1 = canonical_body_hash(&b);
        let h2 = canonical_body_hash(&b);
        assert_eq!(h1, h2);
    }

    #[test]
    fn canonical_body_hash_changes_with_agent_id() {
        let b1 = body();
        let mut b2 = b1.clone();
        b2.agent_id = "summarize_strict_v2".into();
        assert_ne!(canonical_body_hash(&b1), canonical_body_hash(&b2));
    }

    #[test]
    fn canonical_body_hash_changes_with_version() {
        let b1 = body();
        let mut b2 = b1.clone();
        b2.version = "1.2.4".into();
        assert_ne!(canonical_body_hash(&b1), canonical_body_hash(&b2));
    }

    #[test]
    fn canonical_body_hash_changes_with_publisher_id() {
        let b1 = body();
        let mut b2 = b1.clone();
        b2.publisher_id = PublisherId::new();
        assert_ne!(canonical_body_hash(&b1), canonical_body_hash(&b2));
    }

    #[test]
    fn canonical_body_hash_changes_with_tier() {
        let b1 = body();
        let mut b2 = b1.clone();
        b2.kyc_tier = KycTier::Tier3;
        assert_ne!(canonical_body_hash(&b1), canonical_body_hash(&b2));
    }

    #[test]
    fn canonical_body_hash_changes_with_body_hash() {
        let b1 = body();
        let mut b2 = b1.clone();
        b2.agent_body_sha256 = Sha256Hash::from_bytes([0xCD; 32]);
        assert_ne!(canonical_body_hash(&b1), canonical_body_hash(&b2));
    }

    #[test]
    fn canonical_body_hash_changes_with_timestamp() {
        let b1 = body();
        let mut b2 = b1.clone();
        b2.timestamp_epoch_seconds = 0;
        assert_ne!(canonical_body_hash(&b1), canonical_body_hash(&b2));
    }

    #[test]
    fn canonical_schema_version_constant() {
        assert_eq!(CANONICAL_SCHEMA_VERSION, 1);
    }

    #[test]
    fn canonical_body_serde() {
        let b = body();
        let json = serde_json::to_string(&b).unwrap();
        let b2: CanonicalBody = serde_json::from_str(&json).unwrap();
        assert_eq!(b, b2);
    }

    #[test]
    fn canonical_body_hash_non_zero() {
        let b = body();
        let h = canonical_body_hash(&b);
        assert!(!h.is_zero());
    }
}
