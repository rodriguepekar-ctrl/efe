//! **Sprint 7 — N9** : External attestation seam for sealed Merkle roots.
//!
//! # The threat model
//!
//! Without an external notary, a compromised EFE operator could
//! rewrite the entire Merkle epoch chain : tamper a trace, recompute
//! every downstream `payload_hash` and `root_hash`, re-sign the
//! `EpochRoot`s with the new (compromised) HMAC key.  Inside the EFE
//! instance the chain still looks internally consistent.
//!
//! The external notary defeats this : at seal time, the `EpochRoot`'s
//! `(tenant_id, epoch_id, root_hash, previous_epoch_root, sealed_at)`
//! tuple is **submitted to a third party** (one EFE does not control)
//! which records it write-once and returns a [`NotaryReceipt`].  After
//! the fact, anyone holding a receipt can verify the original root
//! existed at the original `sealed_at` time — independent of whether
//! EFE was later compromised.
//!
//! # Notary candidates
//!
//! - **Sigstore Rekor** (transparency log, public, free) — the
//!   audit-grade default for Phase 4.  Append-only, witnessed,
//!   monitored by `rekor-monitor`.
//! - **AWS QLDB / S3 Object Lock** — alternative when the operator
//!   wants an in-house notary.
//! - **Internal MultiSig** (3-of-5 EFE staff) — fallback for
//!   air-gapped deployments.
//!
//! Sprint 7 ships the [`Notary`] trait + an [`InMemoryNotary`] impl
//! that signs with a notary-local HMAC key.  The trait surface is
//! frozen — Phase 4 swap-in to Sigstore Rekor requires NO changes to
//! [`crate::audit::merkle::epoch::EpochSealer`].

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::audit::merkle::epoch::EpochRoot;

// ─────────────────────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum NotaryError {
    /// Notary backend unreachable / network failure.
    #[error("notary unavailable : {0}")]
    Unavailable(String),

    /// Receipt verification failed (signature mismatch or unknown id).
    #[error("notary receipt invalid : {0}")]
    InvalidReceipt(String),

    /// Internal error (rare ; should be unreachable).
    #[error("notary internal : {0}")]
    Internal(String),
}

// ─────────────────────────────────────────────────────────────────────
//  Receipt
// ─────────────────────────────────────────────────────────────────────

/// Proof issued by a [`Notary`] that an [`EpochRoot`] was witnessed
/// at a specific point in time.
///
/// The receipt is **separable** from the `EpochRoot` itself : an
/// auditor receives `(EpochRoot, NotaryReceipt)` and can verify both
/// independently — first that the receipt matches the root's content
/// (`commitment_hash`), then that the receipt's `signature` is valid
/// against the notary's public verification material.
///
/// For the Sprint 7 [`InMemoryNotary`] impl, `signature` is an
/// HMAC-SHA256 over the receipt fields, keyed by a notary-local secret.
/// Phase 4 Sigstore impl will use Ed25519 signatures + an X.509 cert
/// chain + an SCT (Signed Certificate Timestamp).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotaryReceipt {
    /// Unique receipt id.  Persisted by the notary ; used by `verify`
    /// to look up the original record.
    pub receipt_id: Uuid,

    /// SHA-256 over the canonical-serialized receipt-relevant fields
    /// of the [`EpochRoot`].  Auditor recomputes this from a known
    /// `EpochRoot` and compares to detect tampering.
    pub commitment_hash: [u8; 32],

    /// When the notary witnessed the root.  Distinct from
    /// `EpochRoot.sealed_at` — `notarized_at >= sealed_at` always.
    pub notarized_at: DateTime<Utc>,

    /// HMAC-SHA256 (Sprint 7) or Ed25519 (Phase 4) over the receipt
    /// fields, by the notary's signing key.
    pub signature: Vec<u8>,

    /// Free-form notary identifier ("rekor.sigstore.dev", "efe-internal",
    /// etc.).  Lets the auditor pick the right verification key.
    pub notary_id: String,
}

/// Compute the canonical commitment hash for an [`EpochRoot`].
///
/// This is the bytes that the notary signs.  Stable across versions :
/// any change to this serialization is a breaking change to the
/// notary contract.
pub fn commitment_hash(root: &EpochRoot) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"efe-nano-merkle-epoch-v1");
    h.update(root.tenant_id.to_string().as_bytes());
    h.update(root.epoch_id.to_be_bytes());
    h.update(root.leaf_count.to_be_bytes());
    h.update(root.root_hash);
    if let Some(p) = root.previous_epoch_root {
        h.update([1u8]);
        h.update(p);
    } else {
        h.update([0u8]);
    }
    h.update(root.start_trace_id.as_bytes());
    h.update(root.end_trace_id.as_bytes());
    h.update(root.sealed_at.timestamp_micros().to_be_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

// ─────────────────────────────────────────────────────────────────────
//  Trait
// ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait Notary: Send + Sync + 'static {
    /// Stable notary identifier.
    fn id(&self) -> &str;

    /// Submit a sealed [`EpochRoot`] to the notary.  Returns a
    /// [`NotaryReceipt`] that the auditor can later verify.
    async fn notarize(&self, root: &EpochRoot) -> Result<NotaryReceipt, NotaryError>;

    /// Verify a `(root, receipt)` pair :
    /// 1. Recompute the commitment_hash from `root`.
    /// 2. Confirm it equals `receipt.commitment_hash`.
    /// 3. Verify the receipt's signature with the notary's
    ///    verification material.
    ///
    /// Returns `Ok(())` on success.
    async fn verify(
        &self,
        root: &EpochRoot,
        receipt: &NotaryReceipt,
    ) -> Result<(), NotaryError>;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryNotary — HMAC-signed, in-process
// ─────────────────────────────────────────────────────────────────────

/// Sprint 7 default impl : in-process notary signing with a local
/// HMAC-SHA256 key.
///
/// **NOT suitable for production audit-grade attestation** — the
/// signing key lives in the same process as the sealer, so a
/// compromised process can forge receipts.  Production wires
/// Sigstore Rekor via a Phase 4 `RekorNotary` impl.
///
/// Useful for :
/// - Unit + integration tests.
/// - Smoke E2E that the trait + receipt round-trip work end-to-end.
/// - Local dev where the operator just wants tamper-evidence
///   inside the same process boundary.
// Index of issued receipts : `receipt_id → (issued NotaryReceipt, commitment_hash)`.
// Aliased to silence `clippy::type_complexity` ; semantically still a `HashMap`.
type IssuedIndex = std::collections::HashMap<Uuid, (NotaryReceipt, [u8; 32])>;

pub struct InMemoryNotary {
    id: String,
    signing_key: Vec<u8>,
    /// `receipt_id → (root_commitment_hash, signature)` for verification.
    issued: Mutex<IssuedIndex>,
}

impl std::fmt::Debug for InMemoryNotary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryNotary")
            .field("id", &self.id)
            .field("signing_key", &"[redacted]")
            .field("issued_count", &"(async)")
            .finish()
    }
}

impl InMemoryNotary {
    /// Construct with a static notary id and a signing key.
    /// The signing key MUST be ≥ 32 random bytes for HMAC-SHA256.
    pub fn new(id: impl Into<String>, signing_key: impl Into<Vec<u8>>) -> Self {
        Self {
            id: id.into(),
            signing_key: signing_key.into(),
            issued: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Convenience : construct with a fixed test key.
    pub fn test() -> Self {
        Self::new("efe-test-notary", b"test-notary-key-32-bytes-xxxxxxxx".to_vec())
    }

    fn sign(&self, payload: &[u8]) -> Vec<u8> {
        use hmac::{Hmac, Mac};
        let mut mac = <Hmac<Sha256>>::new_from_slice(&self.signing_key)
            .expect("HMAC accepts any key length");
        mac.update(payload);
        mac.finalize().into_bytes().to_vec()
    }

    /// Number of receipts issued so far (for tests / metrics).
    pub async fn issued_count(&self) -> usize {
        self.issued.lock().await.len()
    }
}

#[async_trait]
impl Notary for InMemoryNotary {
    fn id(&self) -> &str {
        &self.id
    }

    async fn notarize(&self, root: &EpochRoot) -> Result<NotaryReceipt, NotaryError> {
        let commitment = commitment_hash(root);
        let now = Utc::now();
        let receipt_id = Uuid::new_v4();
        // Sign over (commitment ‖ notarized_at ‖ receipt_id ‖ notary_id).
        let mut payload = Vec::with_capacity(32 + 8 + 16 + self.id.len());
        payload.extend_from_slice(&commitment);
        payload.extend_from_slice(&now.timestamp_micros().to_be_bytes());
        payload.extend_from_slice(receipt_id.as_bytes());
        payload.extend_from_slice(self.id.as_bytes());
        let signature = self.sign(&payload);
        let receipt = NotaryReceipt {
            receipt_id,
            commitment_hash: commitment,
            notarized_at: now,
            signature,
            notary_id: self.id.clone(),
        };
        self.issued
            .lock()
            .await
            .insert(receipt_id, (receipt.clone(), commitment));
        Ok(receipt)
    }

    async fn verify(
        &self,
        root: &EpochRoot,
        receipt: &NotaryReceipt,
    ) -> Result<(), NotaryError> {
        // 1. commitment_hash matches the root we're verifying.
        let expected = commitment_hash(root);
        if expected != receipt.commitment_hash {
            return Err(NotaryError::InvalidReceipt(
                "commitment_hash does not match the supplied EpochRoot".into(),
            ));
        }
        // 2. We issued this receipt and the commitment is recorded.
        let issued = self.issued.lock().await;
        let Some((on_file, commitment_on_file)) = issued.get(&receipt.receipt_id) else {
            return Err(NotaryError::InvalidReceipt(
                "receipt_id unknown to this notary".into(),
            ));
        };
        if *commitment_on_file != expected {
            return Err(NotaryError::InvalidReceipt(
                "receipt was issued for a different root".into(),
            ));
        }
        // 3. Signature matches (we re-derive deterministically).
        let mut payload = Vec::with_capacity(32 + 8 + 16 + self.id.len());
        payload.extend_from_slice(&on_file.commitment_hash);
        payload.extend_from_slice(&on_file.notarized_at.timestamp_micros().to_be_bytes());
        payload.extend_from_slice(on_file.receipt_id.as_bytes());
        payload.extend_from_slice(on_file.notary_id.as_bytes());
        let recomputed = self.sign(&payload);
        if recomputed != receipt.signature {
            return Err(NotaryError::InvalidReceipt("signature mismatch".into()));
        }
        if on_file.notarized_at != receipt.notarized_at {
            return Err(NotaryError::InvalidReceipt(
                "notarized_at mismatch with on-file record".into(),
            ));
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::merkle::epoch::EpochRoot;
    use efe_nano_sdk::TenantId;

    fn fake_root(t: TenantId, id: u64, root: [u8; 32], prev: Option<[u8; 32]>) -> EpochRoot {
        EpochRoot {
            tenant_id: t,
            epoch_id: id,
            start_trace_id: Uuid::from_u128(id as u128),
            end_trace_id: Uuid::from_u128((id + 1) as u128),
            leaf_count: 1024,
            root_hash: root,
            previous_epoch_root: prev,
            sealed_at: Utc::now(),
            signature: vec![],
        }
    }

    #[tokio::test]
    async fn notarize_then_verify_round_trip() {
        let notary = InMemoryNotary::test();
        let root = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let receipt = notary.notarize(&root).await.unwrap();
        notary.verify(&root, &receipt).await.unwrap();
        assert_eq!(notary.issued_count().await, 1);
        assert_eq!(receipt.notary_id, "efe-test-notary");
    }

    #[tokio::test]
    async fn verify_fails_when_root_was_tampered() {
        let notary = InMemoryNotary::test();
        let mut root = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let receipt = notary.notarize(&root).await.unwrap();
        // Tamper the root after the receipt was issued.
        root.root_hash[0] ^= 0xff;
        let err = notary.verify(&root, &receipt).await.unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("commitment_hash"));
    }

    #[tokio::test]
    async fn verify_fails_for_unknown_receipt() {
        let notary_a = InMemoryNotary::test();
        let notary_b = InMemoryNotary::new("other", b"different-key-32-bytes-xxxxxxxxxx".to_vec());
        let root = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let receipt_a = notary_a.notarize(&root).await.unwrap();
        // notary_b never issued this receipt — must reject.
        let err = notary_b.verify(&root, &receipt_a).await.unwrap_err();
        assert!(err.to_string().contains("unknown"));
    }

    #[tokio::test]
    async fn verify_fails_when_signature_modified() {
        let notary = InMemoryNotary::test();
        let root = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let mut receipt = notary.notarize(&root).await.unwrap();
        receipt.signature[0] ^= 0xff;
        let err = notary.verify(&root, &receipt).await.unwrap_err();
        assert!(err.to_string().contains("signature"));
    }

    #[tokio::test]
    async fn verify_fails_when_notarized_at_modified() {
        let notary = InMemoryNotary::test();
        let root = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let mut receipt = notary.notarize(&root).await.unwrap();
        receipt.notarized_at += chrono::Duration::hours(24);
        let err = notary.verify(&root, &receipt).await.unwrap_err();
        assert!(err.to_string().contains("notarized_at"));
    }

    #[test]
    fn commitment_hash_is_deterministic() {
        let root = fake_root(TenantId::system(), 1, [0xcd; 32], None);
        let h1 = commitment_hash(&root);
        let h2 = commitment_hash(&root);
        assert_eq!(h1, h2);
    }

    #[test]
    fn commitment_hash_changes_when_root_hash_changes() {
        let a = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let b = fake_root(TenantId::system(), 1, [0xff; 32], None);
        assert_ne!(commitment_hash(&a), commitment_hash(&b));
    }

    #[test]
    fn commitment_hash_changes_when_epoch_id_changes() {
        let a = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let b = fake_root(TenantId::system(), 2, [0xab; 32], Some([0x11; 32]));
        assert_ne!(commitment_hash(&a), commitment_hash(&b));
    }

    #[test]
    fn commitment_hash_changes_with_tenant() {
        let a = fake_root(TenantId::new(), 1, [0xab; 32], None);
        let b = fake_root(TenantId::new(), 1, [0xab; 32], None);
        assert_ne!(commitment_hash(&a), commitment_hash(&b));
    }

    #[tokio::test]
    async fn two_notarizations_of_same_root_yield_different_receipts() {
        let notary = InMemoryNotary::test();
        let root = fake_root(TenantId::system(), 1, [0xab; 32], None);
        let r1 = notary.notarize(&root).await.unwrap();
        let r2 = notary.notarize(&root).await.unwrap();
        assert_ne!(r1.receipt_id, r2.receipt_id);
        // Both receipts validate against the same root.
        notary.verify(&root, &r1).await.unwrap();
        notary.verify(&root, &r2).await.unwrap();
    }
}
