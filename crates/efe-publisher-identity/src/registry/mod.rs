// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : Publisher registry — register, lookup,
//! rotate, suspend, revoke.
//!
//! Architecture § 7.3.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

use async_trait::async_trait;
use dashmap::DashMap;
use efe_nano_sdk::{
    KeyId, KycTier, NewPublisherCandidate, OperatorSignature, PublisherError, PublisherId,
    PublisherIdentity, PublisherKey, PublisherStatus, RevocationCertificate, RevocationReason,
    SignatureAlgorithm,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  RegistryError
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : closed taxonomy of registry errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RegistryError {
    #[error("publisher {publisher_id} not found")]
    NotFound { publisher_id: PublisherId },

    #[error("display name {name:?} already in use at tier {tier:?}")]
    DisplayNameCollision { name: String, tier: KycTier },

    #[error("invalid candidate : {reason}")]
    InvalidCandidate { reason: String },

    #[error("publisher already revoked")]
    AlreadyRevoked,

    #[error("operator approvers below threshold : got {got}, need {need}")]
    InsufficientApprovers { got: u32, need: u32 },

    #[error("tier downgrade forbidden (I5)")]
    TierDowngradeForbidden,

    /// **Sprint 10 — N18 + F8** : key rotation proof failed
    /// cryptographic verification.  Either the proof bytes are
    /// malformed or the signature was not produced by the current
    /// active key.
    #[error("rotation proof signature failed")]
    RotationProofFailed,

    #[error("backend : {message}")]
    Backend { message: String },
}

impl From<RegistryError> for PublisherError {
    fn from(e: RegistryError) -> Self {
        match e {
            RegistryError::NotFound { publisher_id } => {
                PublisherError::NotFound { publisher_id }
            }
            RegistryError::DisplayNameCollision { name, tier } => {
                PublisherError::DisplayNameCollision { name, tier }
            }
            RegistryError::InvalidCandidate { reason } => {
                PublisherError::InvalidCandidate { reason }
            }
            RegistryError::RotationProofFailed => PublisherError::RotationProofFailed,
            other => PublisherError::Backend {
                message: other.to_string(),
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  RevocationCascade
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : result of a revocation operation.
///
/// Architecture G4 + G15 + ADR-10.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationCascade {
    pub publisher_id: PublisherId,
    /// Versions known to have been published in the suspected
    /// compromise window — these are tombstoned.
    pub revoked_versions: Vec<RevokedVersion>,
    pub revocation_certificate: RevocationCertificate,
    /// `(from, to)` time window suggested for N16 replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay_window: Option<(SystemTime, SystemTime)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokedVersion {
    pub agent_id: String,
    pub version: String,
    pub published_at: SystemTime,
}

// ─────────────────────────────────────────────────────────────────────
//  PublisherRegistry trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : publisher registry abstraction.
#[async_trait]
pub trait PublisherRegistry: Send + Sync + 'static {
    /// Register a fresh publisher.  Architecture G2 + I10 + F11.
    async fn register(
        &self,
        candidate: NewPublisherCandidate,
    ) -> Result<PublisherIdentity, RegistryError>;

    /// Look up an existing publisher.
    async fn lookup(
        &self,
        publisher_id: PublisherId,
    ) -> Result<PublisherIdentity, RegistryError>;

    /// **G12** : Rotate the signing key.  Old key moves to historical.
    ///
    /// Sprint 10 shape : the **rotation proof** is a signature of the
    /// new public key bytes, made with the **current** key.  Verifying
    /// the proof binds the rotation to the current key holder (F8 :
    /// prevents a key compromise from being used to silently migrate
    /// to an attacker key while preserving signing history).
    async fn rotate_key(
        &self,
        publisher_id: PublisherId,
        new_public_key_pem: String,
        algorithm: SignatureAlgorithm,
    ) -> Result<KeyId, RegistryError>;

    /// **G12 + F8 + ADR-2** : Rotate the signing key with a
    /// cryptographic rotation proof.
    ///
    /// `signature_proof` MUST be a signature of `new_public_key_pem`'s
    /// raw bytes using the **current** active key (the one being
    /// rotated out).  This proves the rotation request came from the
    /// current key holder — an attacker who steals the current
    /// `public` key but not the `private` cannot forge a rotation.
    ///
    /// Verification is delegated to `verifier` (the same Sprint 10
    /// `Ed25519Verifier` used for publishes).
    ///
    /// On success, the new key takes effect immediately ; old
    /// signatures continue to verify against the historical key (I7).
    async fn rotate_key_with_proof(
        &self,
        publisher_id: PublisherId,
        new_public_key_pem: String,
        algorithm: SignatureAlgorithm,
        signature_proof: Vec<u8>,
        verifier: &dyn crate::verifier::SignatureVerifier,
    ) -> Result<KeyId, RegistryError>;

    /// Suspend a publisher (block publishes + dispatches).
    async fn suspend(
        &self,
        publisher_id: PublisherId,
        reason: String,
    ) -> Result<(), RegistryError>;

    /// Lift a suspension (return to Active).  Operator action only.
    async fn unsuspend(&self, publisher_id: PublisherId) -> Result<(), RegistryError>;

    /// **G4 + G15** : Revoke permanently.  Atomically tombstones the
    /// publisher + their versions published in the suspected window.
    async fn revoke(
        &self,
        publisher_id: PublisherId,
        reason: RevocationReason,
        suspected_compromise_at: Option<SystemTime>,
        revoking_operator_id: Uuid,
    ) -> Result<RevocationCascade, RegistryError>;

    /// **G14** : Promote a publisher to a higher tier.
    async fn promote_tier(
        &self,
        publisher_id: PublisherId,
        new_tier: KycTier,
        operator_approvals: Vec<OperatorSignature>,
    ) -> Result<(), RegistryError>;

    /// **G11** : Record that a sanctions check ran for `publisher_id`.
    async fn record_sanctions_check(
        &self,
        publisher_id: PublisherId,
        at: SystemTime,
    ) -> Result<(), RegistryError>;

    /// **G11 + F4 + ADR architecture § 7.3** : Run a fresh sanctions
    /// screening for this publisher via `screener` and apply the
    /// result to the registry state.
    ///
    /// Side effects per architecture § 10.4 :
    /// - `Verdict::Clear` → `last_sanctions_check_at = now` ; if the
    ///   publisher was `SanctionsCheckStale`, restore to `Active`.
    /// - `Verdict::Flagged { source, .. }` → suspend with reason
    ///   `"sanctions match : {source.label}"`.
    /// - `Verdict::Inconclusive` → mark as `SanctionsCheckStale`
    ///   (defense in depth — F4 prefers to block legitimate publishes
    ///   for a few hours than to admit a sanctioned entity).
    ///
    /// Returns the verdict for caller-side metric tracking.
    async fn refresh_sanctions(
        &self,
        publisher_id: PublisherId,
        screener: &dyn crate::sanctions::SanctionsScreener,
    ) -> Result<crate::sanctions::SanctionsVerdict, RegistryError>;

    /// **G11** : Record a tracked-version publish for the cascade.
    async fn record_published_version(
        &self,
        publisher_id: PublisherId,
        agent_id: String,
        version: String,
        published_at: SystemTime,
    ) -> Result<(), RegistryError>;

    /// List active publishers at or above `tier_floor`.
    async fn list_active(
        &self,
        tier_floor: KycTier,
    ) -> Result<Vec<PublisherIdentity>, RegistryError>;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryPublisherRegistry
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : in-memory implementation.
pub struct InMemoryPublisherRegistry {
    publishers: DashMap<PublisherId, PublisherIdentity>,
    /// `(display_name, tier)` → publisher_id ; enforces I10 tier-
    /// scoped uniqueness.
    name_index: DashMap<(String, KycTier), PublisherId>,
    /// `publisher_id` → (agent_id, version, published_at) for the
    /// revocation cascade.
    published_versions: DashMap<PublisherId, Vec<(String, String, SystemTime)>>,
    /// Synchronisation point for atomic revocation (Postgres TX in
    /// Sprint 11).
    revoke_lock: Mutex<()>,
}

impl std::fmt::Debug for InMemoryPublisherRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryPublisherRegistry")
            .field("publishers", &self.publishers.len())
            .finish()
    }
}

impl Default for InMemoryPublisherRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryPublisherRegistry {
    pub fn new() -> Self {
        Self {
            publishers: DashMap::new(),
            name_index: DashMap::new(),
            published_versions: DashMap::new(),
            revoke_lock: Mutex::new(()),
        }
    }

    pub fn len(&self) -> usize {
        self.publishers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.publishers.is_empty()
    }
}

#[async_trait]
impl PublisherRegistry for InMemoryPublisherRegistry {
    async fn register(
        &self,
        candidate: NewPublisherCandidate,
    ) -> Result<PublisherIdentity, RegistryError> {
        candidate.validate().map_err(|e| RegistryError::InvalidCandidate {
            reason: e.to_string(),
        })?;
        let key = (candidate.display_name.clone(), candidate.target_tier);
        if self.name_index.contains_key(&key) {
            return Err(RegistryError::DisplayNameCollision {
                name: candidate.display_name,
                tier: candidate.target_tier,
            });
        }
        let id = PublisherId::new();
        let now = SystemTime::now();
        let initial_key = PublisherKey {
            key_id: KeyId::new(),
            algorithm: candidate.algorithm,
            public_key_pem: candidate.initial_public_key_pem.clone(),
            valid_from: now,
            valid_until: None,
        };
        let identity = PublisherIdentity {
            publisher_id: id,
            display_name: candidate.display_name.clone(),
            kyc_tier: candidate.target_tier,
            status: PublisherStatus::Active,
            jurisdiction: candidate.jurisdiction.clone(),
            registered_at: now,
            last_sanctions_check_at: None,
            active_key: initial_key,
            historical_keys: vec![],
            kyc_dossier_reference: candidate.kyc_dossier_reference.clone(),
            revocation: None,
            forks_of: None,
        };
        self.publishers.insert(id, identity.clone());
        self.name_index.insert(key, id);
        self.published_versions.insert(id, Vec::new());
        Ok(identity)
    }

    async fn lookup(
        &self,
        publisher_id: PublisherId,
    ) -> Result<PublisherIdentity, RegistryError> {
        self.publishers
            .get(&publisher_id)
            .map(|r| r.value().clone())
            .ok_or(RegistryError::NotFound { publisher_id })
    }

    async fn rotate_key(
        &self,
        publisher_id: PublisherId,
        new_public_key_pem: String,
        algorithm: SignatureAlgorithm,
    ) -> Result<KeyId, RegistryError> {
        let mut entry = self
            .publishers
            .get_mut(&publisher_id)
            .ok_or(RegistryError::NotFound { publisher_id })?;
        if matches!(entry.status, PublisherStatus::Revoked) {
            return Err(RegistryError::AlreadyRevoked);
        }
        let now = SystemTime::now();
        // Move current key to historical with valid_until = now.
        let mut prev = entry.active_key.clone();
        prev.valid_until = Some(now);
        entry.historical_keys.push(prev);
        // Install new key.
        let new_key_id = KeyId::new();
        entry.active_key = PublisherKey {
            key_id: new_key_id,
            algorithm,
            public_key_pem: new_public_key_pem,
            valid_from: now,
            valid_until: None,
        };
        Ok(new_key_id)
    }

    async fn rotate_key_with_proof(
        &self,
        publisher_id: PublisherId,
        new_public_key_pem: String,
        algorithm: SignatureAlgorithm,
        signature_proof: Vec<u8>,
        verifier: &dyn crate::verifier::SignatureVerifier,
    ) -> Result<KeyId, RegistryError> {
        use crate::verifier::SignatureError;
        use efe_nano_sdk::{KeyId as SdkKeyId, PublisherSignature, Sha256Hash};
        use sha2::{Digest, Sha256};

        // Read current key PEM under a short-lived borrow to avoid
        // holding the DashMap shard lock across `verifier.verify(...)`.
        let (current_pem, current_algorithm) = {
            let entry = self
                .publishers
                .get(&publisher_id)
                .ok_or(RegistryError::NotFound { publisher_id })?;
            if matches!(entry.status, PublisherStatus::Revoked) {
                return Err(RegistryError::AlreadyRevoked);
            }
            (
                entry.active_key.public_key_pem.clone(),
                entry.active_key.algorithm,
            )
        };

        // Build a PublisherSignature wrapper for the proof so we can
        // reuse the SignatureVerifier API (the verifier expects this
        // type — see crate::verifier::SignatureVerifier::verify).
        let new_key_bytes = new_public_key_pem.as_bytes();
        let mut hasher = Sha256::new();
        hasher.update(new_key_bytes);
        let digest = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&digest);
        let proof = PublisherSignature {
            publisher_id,
            // The proof is keyed to the *current* key being rotated
            // out ; we don't know its KeyId here without a second
            // borrow.  Use a fresh KeyId — the verifier doesn't read
            // this field for the cryptographic check.
            key_id: SdkKeyId::new(),
            algorithm: current_algorithm,
            signature: signature_proof,
            signed_at: SystemTime::now(),
            canonical_hash: Sha256Hash::from_bytes(h),
        };
        verifier
            .verify(&proof, new_key_bytes, &current_pem)
            .await
            .map_err(|e| match e {
                // Any cryptographic verification failure on a
                // rotation proof maps to RotationProofFailed — a
                // bad proof is a bad proof regardless of whether
                // the bytes are malformed or simply wrong.
                SignatureError::VerifyFailed
                | SignatureError::MalformedSignature(_)
                | SignatureError::MalformedPublicKey(_)
                | SignatureError::CanonicalHashMismatch
                | SignatureError::UnsupportedAlgorithm(_) => {
                    RegistryError::RotationProofFailed
                }
            })?;

        // Proof OK — perform the rotation.
        self.rotate_key(publisher_id, new_public_key_pem, algorithm)
            .await
    }

    async fn suspend(
        &self,
        publisher_id: PublisherId,
        reason: String,
    ) -> Result<(), RegistryError> {
        let mut entry = self
            .publishers
            .get_mut(&publisher_id)
            .ok_or(RegistryError::NotFound { publisher_id })?;
        if matches!(entry.status, PublisherStatus::Revoked) {
            return Err(RegistryError::AlreadyRevoked);
        }
        entry.status = PublisherStatus::Suspended {
            reason,
            suspended_at: SystemTime::now(),
        };
        Ok(())
    }

    async fn unsuspend(&self, publisher_id: PublisherId) -> Result<(), RegistryError> {
        let mut entry = self
            .publishers
            .get_mut(&publisher_id)
            .ok_or(RegistryError::NotFound { publisher_id })?;
        if matches!(entry.status, PublisherStatus::Revoked) {
            return Err(RegistryError::AlreadyRevoked);
        }
        entry.status = PublisherStatus::Active;
        Ok(())
    }

    async fn revoke(
        &self,
        publisher_id: PublisherId,
        reason: RevocationReason,
        suspected_compromise_at: Option<SystemTime>,
        revoking_operator_id: Uuid,
    ) -> Result<RevocationCascade, RegistryError> {
        // Atomic cascade (ADR-10 + I17).
        let _guard = self
            .revoke_lock
            .lock()
            .map_err(|e| RegistryError::Backend {
                message: e.to_string(),
            })?;
        let now = SystemTime::now();
        let mut entry = self
            .publishers
            .get_mut(&publisher_id)
            .ok_or(RegistryError::NotFound { publisher_id })?;
        if matches!(entry.status, PublisherStatus::Revoked) {
            return Err(RegistryError::AlreadyRevoked);
        }
        // Build revocation cert.
        let cert = RevocationCertificate {
            revoked_at: now,
            suspected_compromise_at,
            reason: reason.clone(),
            revoking_operator_id,
            // Sprint 11 : real EFE platform signature.  Sprint 10 :
            // placeholder.
            signature: vec![0u8; 64],
        };
        // Cascade : find versions published in the suspected window.
        let window_start = suspected_compromise_at.unwrap_or(now);
        let published = self
            .published_versions
            .get(&publisher_id)
            .map(|r| r.value().clone())
            .unwrap_or_default();
        let mut revoked_versions = Vec::new();
        for (agent_id, version, published_at) in published {
            if published_at >= window_start {
                revoked_versions.push(RevokedVersion {
                    agent_id,
                    version,
                    published_at,
                });
            }
        }
        // Apply atomically.
        entry.status = PublisherStatus::Revoked;
        entry.revocation = Some(cert.clone());
        Ok(RevocationCascade {
            publisher_id,
            revoked_versions,
            revocation_certificate: cert,
            replay_window: suspected_compromise_at.map(|c| (c, now)),
        })
    }

    async fn promote_tier(
        &self,
        publisher_id: PublisherId,
        new_tier: KycTier,
        operator_approvals: Vec<OperatorSignature>,
    ) -> Result<(), RegistryError> {
        let mut entry = self
            .publishers
            .get_mut(&publisher_id)
            .ok_or(RegistryError::NotFound { publisher_id })?;
        if new_tier <= entry.kyc_tier {
            return Err(RegistryError::TierDowngradeForbidden);
        }
        // Tier 3 promotion requires 2 approvals ; lower tiers : 1.
        let required: u32 = if new_tier == KycTier::Tier3 { 2 } else { 1 };
        let got = operator_approvals.len() as u32;
        if got < required {
            return Err(RegistryError::InsufficientApprovers {
                got,
                need: required,
            });
        }
        // Update name index if display name was tier-scoped uniqueness.
        let old_key = (entry.display_name.clone(), entry.kyc_tier);
        let new_key = (entry.display_name.clone(), new_tier);
        if self.name_index.contains_key(&new_key) {
            return Err(RegistryError::DisplayNameCollision {
                name: entry.display_name.clone(),
                tier: new_tier,
            });
        }
        self.name_index.remove(&old_key);
        self.name_index.insert(new_key, publisher_id);
        entry.kyc_tier = new_tier;
        Ok(())
    }

    async fn record_sanctions_check(
        &self,
        publisher_id: PublisherId,
        at: SystemTime,
    ) -> Result<(), RegistryError> {
        let mut entry = self
            .publishers
            .get_mut(&publisher_id)
            .ok_or(RegistryError::NotFound { publisher_id })?;
        entry.last_sanctions_check_at = Some(at);
        // If publisher was in SanctionsCheckStale, restore to Active.
        if matches!(entry.status, PublisherStatus::SanctionsCheckStale) {
            entry.status = PublisherStatus::Active;
        }
        Ok(())
    }

    async fn refresh_sanctions(
        &self,
        publisher_id: PublisherId,
        screener: &dyn crate::sanctions::SanctionsScreener,
    ) -> Result<crate::sanctions::SanctionsVerdict, RegistryError> {
        use crate::sanctions::SanctionsVerdict;

        // Read identity snapshot under a short borrow.
        let identity = {
            let r = self
                .publishers
                .get(&publisher_id)
                .ok_or(RegistryError::NotFound { publisher_id })?;
            r.value().clone()
        };
        // Already revoked publishers : screening is a no-op (the
        // verdict still returns for the caller's tally).
        if matches!(identity.status, PublisherStatus::Revoked) {
            return Ok(SanctionsVerdict::Clear);
        }
        let verdict = screener
            .check(&identity)
            .await
            .map_err(|e| RegistryError::Backend {
                message: format!("sanctions screener : {e}"),
            })?;
        let now = SystemTime::now();
        match &verdict {
            SanctionsVerdict::Clear => {
                let mut entry = self
                    .publishers
                    .get_mut(&publisher_id)
                    .ok_or(RegistryError::NotFound { publisher_id })?;
                entry.last_sanctions_check_at = Some(now);
                // Restore from SanctionsCheckStale (F4 recovery).
                if matches!(entry.status, PublisherStatus::SanctionsCheckStale) {
                    entry.status = PublisherStatus::Active;
                }
            }
            SanctionsVerdict::Flagged { source, .. } => {
                // P5 + F4 : flagged → suspend.
                let mut entry = self
                    .publishers
                    .get_mut(&publisher_id)
                    .ok_or(RegistryError::NotFound { publisher_id })?;
                entry.last_sanctions_check_at = Some(now);
                entry.status = PublisherStatus::Suspended {
                    reason: format!("sanctions match : {}", source.label()),
                    suspended_at: now,
                };
            }
            SanctionsVerdict::Inconclusive => {
                // F4 : defense in depth — mark as stale (block publishes
                // until next successful check).
                let mut entry = self
                    .publishers
                    .get_mut(&publisher_id)
                    .ok_or(RegistryError::NotFound { publisher_id })?;
                // Don't update last_sanctions_check_at — we want the
                // staleness alarm to keep firing until a real result.
                if !matches!(entry.status, PublisherStatus::Suspended { .. }) {
                    entry.status = PublisherStatus::SanctionsCheckStale;
                }
            }
        }
        Ok(verdict)
    }

    async fn record_published_version(
        &self,
        publisher_id: PublisherId,
        agent_id: String,
        version: String,
        published_at: SystemTime,
    ) -> Result<(), RegistryError> {
        if !self.publishers.contains_key(&publisher_id) {
            return Err(RegistryError::NotFound { publisher_id });
        }
        let mut list = self
            .published_versions
            .entry(publisher_id)
            .or_default();
        list.push((agent_id, version, published_at));
        Ok(())
    }

    async fn list_active(
        &self,
        tier_floor: KycTier,
    ) -> Result<Vec<PublisherIdentity>, RegistryError> {
        let _ = tier_floor;
        Ok(self
            .publishers
            .iter()
            .filter(|r| {
                matches!(r.value().status, PublisherStatus::Active)
                    && r.value().kyc_tier >= tier_floor
            })
            .map(|r| r.value().clone())
            .collect::<Vec<_>>())
    }
}

// Suppress an unused-imports warning for `HashMap` if a future
// refactor needs it.
#[allow(dead_code)]
fn _reserved() -> HashMap<(), ()> {
    HashMap::new()
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::Iso3166;

    fn candidate(tier: KycTier, name: &str) -> NewPublisherCandidate {
        NewPublisherCandidate {
            display_name: name.into(),
            target_tier: tier,
            jurisdiction: vec![Iso3166::new("US").unwrap()],
            initial_public_key_pem: "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----"
                .into(),
            algorithm: SignatureAlgorithm::Ed25519,
            pseudonymous: tier == KycTier::Tier0,
            kyc_dossier_reference: if tier >= KycTier::Tier2 {
                Some("kyc-x".into())
            } else {
                None
            },
        }
    }

    #[tokio::test]
    async fn register_then_lookup() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let got = r.lookup(id.publisher_id).await.unwrap();
        assert_eq!(got, id);
    }

    #[tokio::test]
    async fn lookup_unknown_returns_not_found() {
        let r = InMemoryPublisherRegistry::new();
        let err = r.lookup(PublisherId::new()).await.unwrap_err();
        assert!(matches!(err, RegistryError::NotFound { .. }));
    }

    #[tokio::test]
    async fn display_name_collision_within_tier() {
        let r = InMemoryPublisherRegistry::new();
        r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let err = r
            .register(candidate(KycTier::Tier2, "ACME"))
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::DisplayNameCollision { .. }));
    }

    #[tokio::test]
    async fn same_name_different_tier_allowed() {
        let r = InMemoryPublisherRegistry::new();
        r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let other = r.register(candidate(KycTier::Tier0, "ACME")).await.unwrap();
        assert_eq!(other.display_name, "ACME");
    }

    #[tokio::test]
    async fn rotate_key_moves_old_to_historical() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let old_key_id = id.active_key.key_id;
        let new_id = r
            .rotate_key(
                id.publisher_id,
                "-----BEGIN PUBLIC KEY-----\nBBBB\n-----END PUBLIC KEY-----".into(),
                SignatureAlgorithm::Ed25519,
            )
            .await
            .unwrap();
        let got = r.lookup(id.publisher_id).await.unwrap();
        assert_eq!(got.active_key.key_id, new_id);
        assert_eq!(got.historical_keys.len(), 1);
        assert_eq!(got.historical_keys[0].key_id, old_key_id);
        assert!(got.historical_keys[0].valid_until.is_some());
    }

    #[tokio::test]
    async fn suspend_then_unsuspend() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        r.suspend(id.publisher_id, "test".into()).await.unwrap();
        let got = r.lookup(id.publisher_id).await.unwrap();
        assert!(matches!(got.status, PublisherStatus::Suspended { .. }));
        r.unsuspend(id.publisher_id).await.unwrap();
        let got = r.lookup(id.publisher_id).await.unwrap();
        assert!(matches!(got.status, PublisherStatus::Active));
    }

    #[tokio::test]
    async fn revoke_marks_status_revoked() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let cascade = r
            .revoke(
                id.publisher_id,
                RevocationReason::KeyCompromise,
                None,
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        assert_eq!(cascade.publisher_id, id.publisher_id);
        let got = r.lookup(id.publisher_id).await.unwrap();
        assert!(matches!(got.status, PublisherStatus::Revoked));
        assert!(got.revocation.is_some());
    }

    #[tokio::test]
    async fn revoke_twice_fails() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        r.revoke(
            id.publisher_id,
            RevocationReason::KeyCompromise,
            None,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
        let err = r
            .revoke(
                id.publisher_id,
                RevocationReason::PolicyViolation,
                None,
                Uuid::new_v4(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::AlreadyRevoked));
    }

    #[tokio::test]
    async fn backdated_revocation_lists_window_versions() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let now = SystemTime::now();
        let one_day_ago = now - std::time::Duration::from_secs(86_400);
        let ten_days_ago = now - std::time::Duration::from_secs(86_400 * 10);
        // Record 2 publishes : 1 inside window (1 day ago), 1 outside (10 days ago).
        r.record_published_version(id.publisher_id, "a".into(), "1.0".into(), one_day_ago)
            .await
            .unwrap();
        r.record_published_version(id.publisher_id, "a".into(), "0.9".into(), ten_days_ago)
            .await
            .unwrap();
        let cascade = r
            .revoke(
                id.publisher_id,
                RevocationReason::KeyCompromise,
                Some(now - std::time::Duration::from_secs(86_400 * 3)),
                Uuid::new_v4(),
            )
            .await
            .unwrap();
        assert_eq!(cascade.revoked_versions.len(), 1);
        assert_eq!(cascade.revoked_versions[0].version, "1.0");
        assert!(cascade.replay_window.is_some());
    }

    #[tokio::test]
    async fn rotate_after_revoke_fails() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        r.revoke(
            id.publisher_id,
            RevocationReason::KeyCompromise,
            None,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
        let err = r
            .rotate_key(id.publisher_id, "new".into(), SignatureAlgorithm::Ed25519)
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::AlreadyRevoked));
    }

    #[tokio::test]
    async fn promote_tier_one_approval_for_lower_tiers() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier1, "ACME")).await.unwrap();
        r.promote_tier(
            id.publisher_id,
            KycTier::Tier2,
            vec![OperatorSignature {
                operator_id: Uuid::new_v4(),
                signed_at: SystemTime::now(),
                ticket_id: "JIRA-1".into(),
                signature: vec![0u8; 64],
            }],
        )
        .await
        .unwrap();
        let got = r.lookup(id.publisher_id).await.unwrap();
        assert_eq!(got.kyc_tier, KycTier::Tier2);
    }

    #[tokio::test]
    async fn promote_tier_3_requires_two_approvals() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let err = r
            .promote_tier(
                id.publisher_id,
                KycTier::Tier3,
                vec![OperatorSignature {
                    operator_id: Uuid::new_v4(),
                    signed_at: SystemTime::now(),
                    ticket_id: "JIRA-1".into(),
                    signature: vec![0u8; 64],
                }],
            )
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::InsufficientApprovers { .. }));
    }

    #[tokio::test]
    async fn promote_tier_rejects_downgrade() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        let err = r
            .promote_tier(id.publisher_id, KycTier::Tier1, vec![])
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::TierDowngradeForbidden));
    }

    #[tokio::test]
    async fn record_sanctions_check_clears_stale_flag() {
        let r = InMemoryPublisherRegistry::new();
        let id = r.register(candidate(KycTier::Tier2, "ACME")).await.unwrap();
        // Manually mark stale.
        {
            let mut e = r.publishers.get_mut(&id.publisher_id).unwrap();
            e.status = PublisherStatus::SanctionsCheckStale;
        }
        r.record_sanctions_check(id.publisher_id, SystemTime::now())
            .await
            .unwrap();
        let got = r.lookup(id.publisher_id).await.unwrap();
        assert!(matches!(got.status, PublisherStatus::Active));
        assert!(got.last_sanctions_check_at.is_some());
    }

    #[tokio::test]
    async fn list_active_filters_by_tier_floor() {
        let r = InMemoryPublisherRegistry::new();
        r.register(candidate(KycTier::Tier0, "Alice")).await.unwrap();
        r.register(candidate(KycTier::Tier2, "Beta")).await.unwrap();
        r.register(candidate(KycTier::Tier3, "Gamma")).await.unwrap();
        let active = r.list_active(KycTier::Tier2).await.unwrap();
        assert_eq!(active.len(), 2);
        for p in active {
            assert!(p.kyc_tier >= KycTier::Tier2);
        }
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let r = InMemoryPublisherRegistry::new();
        let _ = format!("{r:?}");
    }
}
