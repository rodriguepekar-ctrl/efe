// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : `BypassSignatureVerifier` — § 7.7 + G10.
//!
//! ADR-5 : reuses N18 `Ed25519Verifier` ; no crypto re-implementation.
//!
//! Canonical message format (annex A2) :
//! ```text
//! [0..16]  : gold_run_id  (UUID big-endian bytes)
//! [16..32] : bypass_id    (UUID big-endian bytes)
//! [32..40] : valid_until_unix (i64 big-endian)
//! [40..44] : ticket_id_len (u32 BE)
//! [44..N]  : ticket_id bytes (UTF-8)
//! [N..N+4] : rationale_len (u32 BE)
//! [N+4..M] : rationale bytes (UTF-8)
//! ```

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use efe_nano_sdk::{
    BypassCertificate, GoldGateError, GoldRunId, OperatorSignature, PublisherSignature,
    Sha256Hash, SignatureAlgorithm,
};
use efe_publisher_identity::{SignatureError, SignatureVerifier};
use uuid::Uuid;

/// **N20 + ADR-5** : verifier wired to N18's signature verifier.
///
/// The verifier needs a way to *resolve operator public keys* by
/// `operator_id` — Sprint 12 wires this via a closure (Sprint 13
/// adds a `PublisherRegistry::lookup_operator_key` method).
pub struct BypassSignatureVerifier {
    inner: Arc<dyn SignatureVerifier>,
    key_resolver: Arc<dyn OperatorKeyResolver>,
}

impl std::fmt::Debug for BypassSignatureVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BypassSignatureVerifier").finish_non_exhaustive()
    }
}

/// **N20** : resolve an operator's public-key PEM by their id.
/// Sprint 12 ships an `InMemoryOperatorKeyResolver` ; Sprint 13
/// wires this through the N18 publisher registry directly.
#[async_trait::async_trait]
pub trait OperatorKeyResolver: Send + Sync + 'static {
    async fn lookup(
        &self,
        operator_id: Uuid,
    ) -> Result<OperatorKey, OperatorKeyResolutionError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperatorKey {
    pub operator_id: Uuid,
    pub algorithm: SignatureAlgorithm,
    pub public_key_pem: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum OperatorKeyResolutionError {
    #[error("operator {operator_id} not found")]
    NotFound { operator_id: Uuid },

    #[error("operator {operator_id} key revoked")]
    Revoked { operator_id: Uuid },

    #[error("backend : {message}")]
    Backend { message: String },
}

impl BypassSignatureVerifier {
    pub fn new(
        inner: Arc<dyn SignatureVerifier>,
        key_resolver: Arc<dyn OperatorKeyResolver>,
    ) -> Self {
        Self { inner, key_resolver }
    }

    /// **N20 + G10** : verify EVERY approver's signature against the
    /// canonical message.  Returns `Ok(())` only if all approvers
    /// pass AND the certificate carries enough approvers
    /// (≥ `GOLD_SET_MIN_APPROVERS`).
    ///
    /// `bypass_id` is required to make the canonical message
    /// uniquely bound to this bypass (defense against replay).
    ///
    /// **Post-audit fix B2** : enforces the minimum-approvers floor
    /// internally — callers that skip `BypassCertificate::validate_
    /// shape()` (e.g. tests, future API endpoints) cannot
    /// accidentally vacuous-pass a cert with 0 approvers.
    pub async fn verify(
        &self,
        cert: &BypassCertificate,
        gold_run_id: GoldRunId,
        bypass_id: Uuid,
    ) -> Result<(), GoldGateError> {
        if cert.approvers.len() < efe_nano_sdk::GOLD_SET_MIN_APPROVERS {
            return Err(GoldGateError::BypassInsufficientApprovers {
                got: cert.approvers.len() as u32,
                need: efe_nano_sdk::GOLD_SET_MIN_APPROVERS as u32,
            });
        }
        // Defense in depth : reject duplicate approver ids.
        let mut ids: Vec<Uuid> = cert.approvers.iter().map(|a| a.operator_id).collect();
        ids.sort();
        let pre = ids.len();
        ids.dedup();
        if ids.len() != pre {
            // The first repeated id is reported by index of its
            // second occurrence (defense against subtle replay
            // attacks where the same operator signs twice).
            for i in 1..cert.approvers.len() {
                let dup = cert.approvers[..i]
                    .iter()
                    .any(|p| p.operator_id == cert.approvers[i].operator_id);
                if dup {
                    return Err(GoldGateError::BypassSignatureInvalid {
                        approver_index: i as u32,
                    });
                }
            }
        }
        let canonical = canonical_bypass_message(gold_run_id, bypass_id, cert);
        let canonical_hash = sha256(&canonical);

        for (idx, sig) in cert.approvers.iter().enumerate() {
            self.verify_one(sig, idx, &canonical, canonical_hash)
                .await?;
        }
        Ok(())
    }

    async fn verify_one(
        &self,
        sig: &OperatorSignature,
        index: usize,
        canonical: &[u8],
        canonical_hash: Sha256Hash,
    ) -> Result<(), GoldGateError> {
        // Defensive : empty signature bytes is a known N19 gap that
        // we explicitly catch here.  P12 / Incident I-2026-07-25.
        if sig.signature.is_empty() {
            return Err(GoldGateError::BypassSignatureInvalid {
                approver_index: index as u32,
            });
        }
        if sig.signature.len() != 64 {
            return Err(GoldGateError::BypassSignatureInvalid {
                approver_index: index as u32,
            });
        }
        let key = self
            .key_resolver
            .lookup(sig.operator_id)
            .await
            .map_err(|_| GoldGateError::BypassSignatureInvalid {
                approver_index: index as u32,
            })?;
        // Wrap into the N18 PublisherSignature shape so we can call
        // the existing verifier.
        let publisher_sig = PublisherSignature {
            publisher_id: efe_nano_sdk::PublisherId::new(),
            key_id: efe_nano_sdk::KeyId::new(),
            algorithm: key.algorithm,
            signature: sig.signature.clone(),
            signed_at: sig.signed_at,
            canonical_hash,
        };
        self.inner
            .verify(&publisher_sig, canonical, &key.public_key_pem)
            .await
            .map_err(|e| match e {
                SignatureError::VerifyFailed
                | SignatureError::MalformedSignature(_)
                | SignatureError::MalformedPublicKey(_)
                | SignatureError::CanonicalHashMismatch
                | SignatureError::UnsupportedAlgorithm(_) => {
                    GoldGateError::BypassSignatureInvalid {
                        approver_index: index as u32,
                    }
                }
            })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Canonical message construction
// ─────────────────────────────────────────────────────────────────────

/// **N20 + A2** : Build the canonical message that approvers MUST
/// sign.  Stable byte layout — any field reorder / format change is
/// a wire break.
///
/// Layout :
/// ```text
/// [0..16]  gold_run_id  (UUID big-endian)
/// [16..32] bypass_id    (UUID big-endian)
/// [32..40] valid_until_unix (i64 big-endian)
/// [40..44] ticket_id_len (u32 big-endian)
/// [44..N]  ticket_id (UTF-8 bytes)
/// [N..N+4] rationale_len (u32 big-endian)
/// [N+4..M] rationale (UTF-8 bytes)
/// ```
pub fn canonical_bypass_message(
    gold_run_id: GoldRunId,
    bypass_id: Uuid,
    cert: &BypassCertificate,
) -> Vec<u8> {
    let valid_until_unix = cert
        .valid_until
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let ticket = cert.ticket_id.as_bytes();
    let rationale = cert.rationale.as_bytes();

    let cap = 16 + 16 + 8 + 4 + ticket.len() + 4 + rationale.len();
    let mut out = Vec::with_capacity(cap);
    out.extend_from_slice(gold_run_id.as_uuid().as_bytes());
    out.extend_from_slice(bypass_id.as_bytes());
    out.extend_from_slice(&valid_until_unix.to_be_bytes());
    out.extend_from_slice(&(ticket.len() as u32).to_be_bytes());
    out.extend_from_slice(ticket);
    out.extend_from_slice(&(rationale.len() as u32).to_be_bytes());
    out.extend_from_slice(rationale);
    out
}

fn sha256(bytes: &[u8]) -> Sha256Hash {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&digest);
    Sha256Hash::from_bytes(arr)
}

/// Suppress unused-import warning if a future refactor needs
/// `SystemTime` directly.
#[allow(dead_code)]
fn _systemtime_keep_alive() -> Option<SystemTime> {
    None
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryOperatorKeyResolver — tests & dev
// ─────────────────────────────────────────────────────────────────────

/// **N20** : in-memory key resolver.  Sprint 13 wires the N18 publisher
/// registry directly.
pub struct InMemoryOperatorKeyResolver {
    keys: dashmap::DashMap<Uuid, OperatorKey>,
}

impl Default for InMemoryOperatorKeyResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for InMemoryOperatorKeyResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryOperatorKeyResolver")
            .field("keys", &self.keys.len())
            .finish()
    }
}

impl InMemoryOperatorKeyResolver {
    pub fn new() -> Self {
        Self {
            keys: dashmap::DashMap::new(),
        }
    }

    pub fn insert(&self, key: OperatorKey) {
        self.keys.insert(key.operator_id, key);
    }

    pub fn revoke(&self, operator_id: Uuid) -> bool {
        self.keys.remove(&operator_id).is_some()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[async_trait::async_trait]
impl OperatorKeyResolver for InMemoryOperatorKeyResolver {
    async fn lookup(
        &self,
        operator_id: Uuid,
    ) -> Result<OperatorKey, OperatorKeyResolutionError> {
        self.keys
            .get(&operator_id)
            .map(|r| r.value().clone())
            .ok_or(OperatorKeyResolutionError::NotFound { operator_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_publisher_identity::{Ed25519Signer, Ed25519Verifier};

    fn cert(ticket: &str, rationale: &str) -> BypassCertificate {
        BypassCertificate {
            approvers: vec![],
            ticket_id: ticket.into(),
            rationale: rationale.into(),
            valid_until: SystemTime::now() + std::time::Duration::from_secs(86_400),
        }
    }

    // ─── canonical_bypass_message determinism ───────────────────

    #[test]
    fn canonical_message_deterministic() {
        let run_id = GoldRunId::from_uuid(Uuid::nil());
        let bypass_id = Uuid::from_u128(0x42);
        let c = cert("INC-1", "x".repeat(64).as_str());
        let m1 = canonical_bypass_message(run_id, bypass_id, &c);
        let m2 = canonical_bypass_message(run_id, bypass_id, &c);
        assert_eq!(m1, m2);
    }

    #[test]
    fn canonical_message_layout_known_bytes() {
        let run_id = GoldRunId::from_uuid(Uuid::nil());
        let bypass_id = Uuid::nil();
        // Construct a cert with a deterministic valid_until.
        let c = BypassCertificate {
            approvers: vec![],
            ticket_id: "AB".into(),
            rationale: "CD".into(),
            valid_until: UNIX_EPOCH + std::time::Duration::from_secs(0),
        };
        let m = canonical_bypass_message(run_id, bypass_id, &c);
        // [0..16] = 16 zero bytes (run_id nil)
        // [16..32] = 16 zero bytes (bypass_id nil)
        // [32..40] = 8 zero bytes (valid_until 0)
        // [40..44] = ticket len 2 → 0x00 0x00 0x00 0x02
        // [44..46] = "AB"
        // [46..50] = rationale len 2 → 0x00 0x00 0x00 0x02
        // [50..52] = "CD"
        assert_eq!(m.len(), 52);
        assert_eq!(&m[0..16], &[0u8; 16]);
        assert_eq!(&m[16..32], &[0u8; 16]);
        assert_eq!(&m[32..40], &[0u8; 8]);
        assert_eq!(&m[40..44], &[0, 0, 0, 2]);
        assert_eq!(&m[44..46], b"AB");
        assert_eq!(&m[46..50], &[0, 0, 0, 2]);
        assert_eq!(&m[50..52], b"CD");
    }

    #[test]
    fn canonical_message_changes_on_field_tamper() {
        let run_id = GoldRunId::from_uuid(Uuid::nil());
        let bypass_id = Uuid::new_v4();
        let c1 = cert("INC-1", &"x".repeat(64));
        let c2 = cert("INC-2", &"x".repeat(64));
        assert_ne!(
            canonical_bypass_message(run_id, bypass_id, &c1),
            canonical_bypass_message(run_id, bypass_id, &c2)
        );
    }

    #[test]
    fn canonical_message_changes_on_rationale_tamper() {
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let c1 = cert("INC-1", &"x".repeat(64));
        let c2 = {
            let mut c = c1.clone();
            c.rationale = "y".repeat(64);
            c
        };
        let _ = c1.approvers.clone(); // pin clone exists
        assert_ne!(
            canonical_bypass_message(run_id, bypass_id, &c1),
            canonical_bypass_message(run_id, bypass_id, &c2)
        );
    }

    // ─── BypassSignatureVerifier — happy + sad paths ────────────

    fn build_verifier() -> (BypassSignatureVerifier, Arc<InMemoryOperatorKeyResolver>) {
        let resolver = Arc::new(InMemoryOperatorKeyResolver::new());
        let v = BypassSignatureVerifier::new(
            Arc::new(Ed25519Verifier::new()),
            resolver.clone(),
        );
        (v, resolver)
    }

    fn sign_cert(
        signer: &Ed25519Signer,
        operator_id: Uuid,
        gold_run_id: GoldRunId,
        bypass_id: Uuid,
        cert: &mut BypassCertificate,
    ) {
        let canonical = canonical_bypass_message(gold_run_id, bypass_id, cert);
        let sig = signer.sign(&canonical).to_vec();
        cert.approvers.push(OperatorSignature {
            operator_id,
            signed_at: SystemTime::now(),
            ticket_id: cert.ticket_id.clone(),
            signature: sig,
        });
    }

    #[tokio::test]
    async fn verify_with_two_valid_signatures_passes() {
        let (verifier, resolver) = build_verifier();
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

        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        sign_cert(&s1, op1, run_id, bypass_id, &mut c);
        sign_cert(&s2, op2, run_id, bypass_id, &mut c);

        verifier.verify(&c, run_id, bypass_id).await.unwrap();
    }

    #[tokio::test]
    async fn empty_signature_bytes_rejected() {
        // **Post-audit fix B2** : verifier enforces floor (≥ 2
        // approvers).  We provide 2 approvers ; the first has an
        // empty signature and MUST be rejected at index 0.
        let (verifier, resolver) = build_verifier();
        let op_bad = Uuid::from_u128(1);
        let op_good = Uuid::from_u128(2);
        let s_good = Ed25519Signer::from_seed([2u8; 32]);
        resolver.insert(OperatorKey {
            operator_id: op_bad,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: Ed25519Signer::from_seed([1u8; 32]).public_key_pem(),
        });
        resolver.insert(OperatorKey {
            operator_id: op_good,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s_good.public_key_pem(),
        });
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        c.approvers.push(OperatorSignature {
            operator_id: op_bad,
            signed_at: SystemTime::now(),
            ticket_id: "INC-1".into(),
            signature: vec![], // ← P12 / Incident I-2026-07-25
        });
        // Pad with a second valid approver so we exercise the
        // signature check, not the floor check.
        sign_cert(&s_good, op_good, run_id, bypass_id, &mut c);
        let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
        match err {
            GoldGateError::BypassSignatureInvalid { approver_index } => {
                assert_eq!(approver_index, 0);
            }
            other => panic!("expected BypassSignatureInvalid at index 0, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wrong_length_signature_rejected() {
        let (verifier, resolver) = build_verifier();
        let op_bad = Uuid::from_u128(1);
        let op_good = Uuid::from_u128(2);
        let s_good = Ed25519Signer::from_seed([2u8; 32]);
        resolver.insert(OperatorKey {
            operator_id: op_bad,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: Ed25519Signer::from_seed([1u8; 32]).public_key_pem(),
        });
        resolver.insert(OperatorKey {
            operator_id: op_good,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s_good.public_key_pem(),
        });
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        c.approvers.push(OperatorSignature {
            operator_id: op_bad,
            signed_at: SystemTime::now(),
            ticket_id: "INC-1".into(),
            signature: vec![0u8; 32], // wrong length
        });
        sign_cert(&s_good, op_good, run_id, bypass_id, &mut c);
        let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
        assert!(matches!(err, GoldGateError::BypassSignatureInvalid { .. }));
    }

    #[tokio::test]
    async fn signature_for_wrong_message_rejected() {
        let (verifier, resolver) = build_verifier();
        let s_bad = Ed25519Signer::from_seed([5u8; 32]);
        let s_good = Ed25519Signer::from_seed([6u8; 32]);
        let op_bad = Uuid::from_u128(1);
        let op_good = Uuid::from_u128(2);
        resolver.insert(OperatorKey {
            operator_id: op_bad,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s_bad.public_key_pem(),
        });
        resolver.insert(OperatorKey {
            operator_id: op_good,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s_good.public_key_pem(),
        });
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        // Sign a DIFFERENT canonical message than the one verified.
        let sig = s_bad.sign(b"completely different bytes").to_vec();
        c.approvers.push(OperatorSignature {
            operator_id: op_bad,
            signed_at: SystemTime::now(),
            ticket_id: "INC-1".into(),
            signature: sig,
        });
        sign_cert(&s_good, op_good, run_id, bypass_id, &mut c);
        let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
        assert!(matches!(err, GoldGateError::BypassSignatureInvalid { .. }));
    }

    #[tokio::test]
    async fn unknown_operator_id_rejected() {
        let (verifier, resolver) = build_verifier();
        let s = Ed25519Signer::from_seed([3u8; 32]);
        let s_good = Ed25519Signer::from_seed([8u8; 32]);
        let unknown_op = Uuid::from_u128(999);
        let op_good = Uuid::from_u128(2);
        // Register only the good operator ; the unknown remains
        // unresolvable.
        resolver.insert(OperatorKey {
            operator_id: op_good,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s_good.public_key_pem(),
        });
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        sign_cert(&s, unknown_op, run_id, bypass_id, &mut c);
        sign_cert(&s_good, op_good, run_id, bypass_id, &mut c);
        let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
        assert!(matches!(err, GoldGateError::BypassSignatureInvalid { .. }));
    }

    #[tokio::test]
    async fn one_valid_one_invalid_rejected() {
        let (verifier, resolver) = build_verifier();
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
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        sign_cert(&s1, op1, run_id, bypass_id, &mut c);
        // Second approver provides garbage.
        c.approvers.push(OperatorSignature {
            operator_id: op2,
            signed_at: SystemTime::now(),
            ticket_id: "INC-1".into(),
            signature: vec![0xFFu8; 64],
        });
        let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
        match err {
            GoldGateError::BypassSignatureInvalid { approver_index } => {
                assert_eq!(approver_index, 1);
            }
            other => panic!("expected BypassSignatureInvalid, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn revoke_key_then_verify_fails() {
        let (verifier, resolver) = build_verifier();
        let s = Ed25519Signer::from_seed([7u8; 32]);
        let s_good = Ed25519Signer::from_seed([8u8; 32]);
        let op = Uuid::from_u128(42);
        let op_good = Uuid::from_u128(43);
        resolver.insert(OperatorKey {
            operator_id: op,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s.public_key_pem(),
        });
        resolver.insert(OperatorKey {
            operator_id: op_good,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s_good.public_key_pem(),
        });
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        sign_cert(&s, op, run_id, bypass_id, &mut c);
        sign_cert(&s_good, op_good, run_id, bypass_id, &mut c);
        // Revoke the FIRST operator's key before verify.
        assert!(resolver.revoke(op));
        let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
        match err {
            GoldGateError::BypassSignatureInvalid { approver_index } => {
                assert_eq!(approver_index, 0);
            }
            other => panic!("expected index 0, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_approvers_list_rejected() {
        // **Post-audit fix B2** : the verifier MUST enforce the
        // minimum-approvers floor itself (defense in depth) ; a
        // certificate with 0 approvers is rejected with
        // `BypassInsufficientApprovers`, not vacuously accepted.
        let (verifier, _) = build_verifier();
        let c = cert("INC-1", &"x".repeat(64));
        let err = verifier
            .verify(&c, GoldRunId::new(), Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            GoldGateError::BypassInsufficientApprovers { .. }
        ));
    }

    #[tokio::test]
    async fn duplicate_approver_rejected() {
        let (verifier, resolver) = build_verifier();
        let s = Ed25519Signer::from_seed([4u8; 32]);
        let op = Uuid::from_u128(7);
        resolver.insert(OperatorKey {
            operator_id: op,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: s.public_key_pem(),
        });
        let run_id = GoldRunId::new();
        let bypass_id = Uuid::new_v4();
        let mut c = cert("INC-1", &"x".repeat(64));
        // Same operator signs twice → defense against replay.
        sign_cert(&s, op, run_id, bypass_id, &mut c);
        sign_cert(&s, op, run_id, bypass_id, &mut c);
        let err = verifier.verify(&c, run_id, bypass_id).await.unwrap_err();
        assert!(matches!(
            err,
            GoldGateError::BypassSignatureInvalid { approver_index: 1 }
        ));
    }

    #[test]
    fn in_memory_resolver_len_and_revoke() {
        let r = InMemoryOperatorKeyResolver::new();
        assert!(r.is_empty());
        let op = Uuid::from_u128(1);
        r.insert(OperatorKey {
            operator_id: op,
            algorithm: SignatureAlgorithm::Ed25519,
            public_key_pem: "x".into(),
        });
        assert_eq!(r.len(), 1);
        assert!(r.revoke(op));
        assert!(r.is_empty());
        // Idempotent revoke.
        assert!(!r.revoke(op));
    }

    #[test]
    fn debug_format_no_panic() {
        let r = InMemoryOperatorKeyResolver::new();
        let _ = format!("{r:?}");
    }
}
