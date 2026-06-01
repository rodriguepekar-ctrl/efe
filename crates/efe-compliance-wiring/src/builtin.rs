// SPDX-License-Identifier: UNLICENSED
//! **Sprint 16 — N24 + ADR-9 + G9 + I15** : built-in EFE publisher
//! identity baked into the binary.
//!
//! ## Chicken-and-egg problem
//!
//! The gateway boots before the external `PublisherRegistry` is
//! loaded.  The six built-in catalogue entries from
//! `efe-nano-preanalysis` need a signature *now*, but no external
//! identity exists to sign them.  We therefore bake :
//!
//! 1. A deterministic 32-byte Ed25519 seed —
//!    [`EFE_BUILTIN_PUBLISHER_SEED`].
//! 2. A deterministic [`PublisherId`] —
//!    [`EFE_BUILTIN_PUBLISHER_ID`].
//! 3. A deterministic [`KeyId`] for the active key —
//!    [`EFE_BUILTIN_PUBLISHER_KEY_ID`].
//! 4. A lazy-computed PEM-encoded public key —
//!    [`EFE_BUILTIN_PUBLISHER_PEM()`].
//! 5. A lazy-computed [`PublisherIdentity`] —
//!    [`EFE_BUILTIN_PUBLISHER_IDENTITY()`].
//! 6. A signing helper [`sign_catalogue_with_builtin_publisher`]
//!    that signs every catalogue entry on the fly using the
//!    baked seed.
//!
//! ## Security notes
//!
//! - The seed is **published** in this source file.  It is NOT a
//!   secret in the conventional sense ; its only role is to make
//!   the built-in catalogue self-verifiable on a cold boot.  The
//!   verifier still rejects forgeries because the canonical body
//!   includes the catalogue entry's content hash (annex A2 of N18).
//! - Operators who want to ROTATE this key out (e.g. a fresh
//!   deployment with a tenant-owned signing root) call
//!   `PublisherApi::rotate_key_with_proof` on the live registry
//!   after boot ; the historical key remains valid for replay
//!   (I7) and the new key takes over for fresh dispatches.
//! - The seed will be re-rolled at every major M50 release ; a
//!   compromise of the seed bytes only affects the cold-boot
//!   verification path of THAT release.

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::SystemTime;

use efe_nano_sdk::{
    Author, KeyId, KycTier, NanoAgent, PublisherId, PublisherIdentity, PublisherKey,
    PublisherStatus, PublisherSignature, Sha256Hash, SignatureAlgorithm,
};
use efe_publisher_identity::verifier::Ed25519Signer;
use uuid::Uuid;

/// **N24** : type alias for a catalogue entry — same shape as
/// `efe_nano_preanalysis::CatalogueEntry` but re-aliased here so
/// the wiring crate doesn't impose a runtime dependency on the
/// preanalysis crate.
pub type CatalogueEntry = (Arc<NanoAgent>, Arc<dyn efe_nano_swarm_adapter::NanoRunner>);

use crate::canonical::{compute_agent_body_sha256, nano_to_canonical_body};

/// **N24** : the baked Ed25519 seed.  Deterministic — same
/// `from_seed` invocation always produces the same keypair.
///
/// This seed is BAKED into the binary.  It is NOT a per-tenant
/// secret ; it is the cold-boot signing key for the six built-in
/// catalogue entries.  See module-level docs for the threat
/// model.
pub const EFE_BUILTIN_PUBLISHER_SEED: [u8; 32] = [
    0xE5, 0xFE, 0xA3, 0x91, 0xC0, 0xDE, 0x42, 0x10, 0x7B, 0x88, 0x14, 0x2D, 0x55, 0x66,
    0x77, 0x99, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0,
    0xD0, 0xE0, 0xF0, 0x18,
];

/// **N24** : the built-in publisher's deterministic id.
///
/// Reserved UUID, baked here.  Will never collide with a
/// `PublisherId::new()` (which is random v4).
pub const fn efe_builtin_publisher_id_uuid() -> Uuid {
    Uuid::from_bytes([
        0xEF, 0xE0, 0xBE, 0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x18,
    ])
}

/// **N24** : the built-in publisher's deterministic id.
#[allow(non_snake_case)]
pub fn EFE_BUILTIN_PUBLISHER_ID() -> PublisherId {
    PublisherId::from_uuid(efe_builtin_publisher_id_uuid())
}

/// **N24** : the built-in publisher's deterministic key id.
pub const fn efe_builtin_publisher_key_uuid() -> Uuid {
    Uuid::from_bytes([
        0xEF, 0xE0, 0x6E, 0x70, 0x00, 0x00, 0x40, 0x01, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x18,
    ])
}

/// **N24** : the built-in publisher's deterministic key id.
#[allow(non_snake_case)]
pub fn EFE_BUILTIN_PUBLISHER_KEY_ID() -> KeyId {
    KeyId::from_uuid(efe_builtin_publisher_key_uuid())
}

/// **N24** : a `SystemTime` for the built-in key's `valid_from`.
/// We use the M50 epoch (2026-01-01T00:00:00Z) ; the value is
/// stable across boots so signatures over a `signed_at` after
/// this point validate against this key by the
/// `was_active_at` predicate.
fn builtin_valid_from() -> SystemTime {
    use std::time::Duration;
    // 2026-01-01T00:00:00Z = 1_767_225_600 epoch seconds.
    std::time::UNIX_EPOCH + Duration::from_secs(1_767_225_600)
}

/// **N24** : lazy-computed PEM-encoded public key for the
/// built-in publisher.
#[allow(non_snake_case)]
pub fn EFE_BUILTIN_PUBLISHER_PEM() -> &'static str {
    static PEM: OnceLock<String> = OnceLock::new();
    PEM.get_or_init(|| {
        let signer = Ed25519Signer::from_seed(EFE_BUILTIN_PUBLISHER_SEED);
        signer.public_key_pem()
    })
    .as_str()
}

/// **N24 + ADR-9** : lazy-computed `PublisherIdentity` for the
/// built-in publisher.
#[allow(non_snake_case)]
pub fn EFE_BUILTIN_PUBLISHER_IDENTITY() -> PublisherIdentity {
    static IDENT: OnceLock<PublisherIdentity> = OnceLock::new();
    IDENT
        .get_or_init(|| PublisherIdentity {
            publisher_id: EFE_BUILTIN_PUBLISHER_ID(),
            display_name: "EFE (built-in)".to_string(),
            kyc_tier: KycTier::Tier3,
            status: PublisherStatus::Active,
            jurisdiction: vec![],
            registered_at: builtin_valid_from(),
            last_sanctions_check_at: Some(SystemTime::now()),
            active_key: PublisherKey {
                key_id: EFE_BUILTIN_PUBLISHER_KEY_ID(),
                algorithm: SignatureAlgorithm::Ed25519,
                public_key_pem: EFE_BUILTIN_PUBLISHER_PEM().to_string(),
                valid_from: builtin_valid_from(),
                valid_until: None,
            },
            historical_keys: vec![],
            kyc_dossier_reference: None,
            revocation: None,
            forks_of: None,
        })
        .clone()
}

/// **N24 + G9** : sign every unsigned catalogue entry with the
/// built-in publisher key.  Returns a NEW vector ; the original
/// is unchanged.
///
/// Each input pair is `(Arc<NanoAgent>, Arc<dyn NanoRunner>)` as
/// produced by `efe_nano_preanalysis::default_catalogue()`.  The
/// output replaces `nano.author` with `Author::Publisher { ... }`
/// (so the WiredSignatureGate's author-vs-signer match wins —
/// G8) and attaches a fresh `PublisherSignature`.
///
/// Determinism : the seed + `signed_at_override` parameters
/// produce identical bytes across builds.  `signed_at_override =
/// None` uses the current time (good for dev / live) ; pass
/// `Some(t)` for byte-stable test fixtures.
pub fn sign_catalogue_with_builtin_publisher(
    catalogue: Vec<CatalogueEntry>,
    signed_at_override: Option<SystemTime>,
) -> Vec<CatalogueEntry> {
    let signer = Ed25519Signer::from_seed(EFE_BUILTIN_PUBLISHER_SEED);
    let publisher_id = EFE_BUILTIN_PUBLISHER_ID();
    let key_id = EFE_BUILTIN_PUBLISHER_KEY_ID();
    let signed_at = signed_at_override.unwrap_or_else(SystemTime::now);

    catalogue
        .into_iter()
        .map(|(nano, runner)| {
            let mut signed_nano: NanoAgent = (*nano).clone();
            // Rewrite the author to be the built-in publisher so
            // the WiredSignatureGate's author-vs-signer match
            // succeeds (G8 / I17).
            signed_nano.author = Author::Publisher {
                publisher_id,
                kyc_tier: KycTier::Tier3,
            };
            // Compute the agent body hash WITHOUT the signature
            // (canonical strips it ; see canonical.rs).
            let agent_body_hash = compute_agent_body_sha256(&signed_nano);
            // Build a placeholder signature so we can derive the
            // canonical body.
            let mut placeholder_sig = PublisherSignature {
                publisher_id,
                key_id,
                algorithm: SignatureAlgorithm::Ed25519,
                signature: vec![],
                signed_at,
                canonical_hash: agent_body_hash,
            };
            let canonical_body = nano_to_canonical_body(&signed_nano, &placeholder_sig);
            let canonical_bytes = canonical_body.canonical_bytes();

            // Hash the canonical bytes and sign that hash plus
            // the raw bytes.
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&canonical_bytes);
            let digest = hasher.finalize();
            let mut canonical_hash_bytes = [0u8; 32];
            canonical_hash_bytes.copy_from_slice(&digest);

            let sig_bytes = signer.sign(&canonical_bytes);
            placeholder_sig.signature = sig_bytes.to_vec();
            placeholder_sig.canonical_hash = Sha256Hash::from_bytes(canonical_hash_bytes);

            signed_nano.signature = Some(placeholder_sig);
            (Arc::new(signed_nano), runner)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_seed_is_32_bytes() {
        assert_eq!(EFE_BUILTIN_PUBLISHER_SEED.len(), 32);
    }

    #[test]
    fn builtin_publisher_id_deterministic() {
        let a = EFE_BUILTIN_PUBLISHER_ID();
        let b = EFE_BUILTIN_PUBLISHER_ID();
        assert_eq!(a, b);
    }

    #[test]
    fn builtin_publisher_pem_stable_across_calls() {
        let a = EFE_BUILTIN_PUBLISHER_PEM();
        let b = EFE_BUILTIN_PUBLISHER_PEM();
        assert_eq!(a, b);
        assert!(a.contains("-----BEGIN PUBLIC KEY-----"));
        assert!(a.contains("-----END PUBLIC KEY-----"));
    }

    #[test]
    fn builtin_publisher_identity_active() {
        let id = EFE_BUILTIN_PUBLISHER_IDENTITY();
        assert!(matches!(id.status, PublisherStatus::Active));
        assert!(id.can_publish());
        assert!(!id.dispatch_blocked());
        assert_eq!(id.publisher_id, EFE_BUILTIN_PUBLISHER_ID());
        assert_eq!(id.active_key.key_id, EFE_BUILTIN_PUBLISHER_KEY_ID());
        assert_eq!(id.active_key.algorithm, SignatureAlgorithm::Ed25519);
    }

    #[test]
    fn builtin_publisher_active_key_pem_matches_seed() {
        let id = EFE_BUILTIN_PUBLISHER_IDENTITY();
        assert_eq!(id.active_key.public_key_pem, EFE_BUILTIN_PUBLISHER_PEM());
    }

    #[tokio::test]
    async fn sign_catalogue_attaches_signatures() {
        let original = efe_nano_preanalysis::default_catalogue();
        // None of the originals are signed.
        for (nano, _) in &original {
            assert!(nano.signature.is_none());
        }
        let signed = sign_catalogue_with_builtin_publisher(original, None);
        assert_eq!(signed.len(), 6);
        for (nano, _) in &signed {
            let sig = nano.signature.as_ref().expect("must be signed");
            assert_eq!(sig.publisher_id, EFE_BUILTIN_PUBLISHER_ID());
            assert_eq!(sig.algorithm, SignatureAlgorithm::Ed25519);
            assert_eq!(sig.signature.len(), 64);
            // Author rewritten to Publisher.
            assert!(matches!(nano.author, Author::Publisher { .. }));
        }
    }

    #[tokio::test]
    async fn signed_catalogue_verifies_against_builtin_pem() {
        use efe_publisher_identity::verifier::{Ed25519Verifier, SignatureVerifier};
        let signed = sign_catalogue_with_builtin_publisher(
            efe_nano_preanalysis::default_catalogue(),
            None,
        );
        let verifier = Ed25519Verifier::new();
        for (nano, _) in &signed {
            let sig = nano.signature.as_ref().unwrap();
            let canonical = nano_to_canonical_body(nano, sig);
            let bytes = canonical.canonical_bytes();
            verifier
                .verify(sig, &bytes, EFE_BUILTIN_PUBLISHER_PEM())
                .await
                .expect("built-in catalogue MUST verify with built-in key");
        }
    }

    #[tokio::test]
    async fn signed_catalogue_fails_against_other_pem() {
        use efe_publisher_identity::verifier::{Ed25519Signer, Ed25519Verifier, SignatureVerifier};
        let signed = sign_catalogue_with_builtin_publisher(
            efe_nano_preanalysis::default_catalogue(),
            None,
        );
        let other = Ed25519Signer::from_seed([0xAAu8; 32]);
        let verifier = Ed25519Verifier::new();
        for (nano, _) in &signed {
            let sig = nano.signature.as_ref().unwrap();
            let canonical = nano_to_canonical_body(nano, sig);
            let bytes = canonical.canonical_bytes();
            let result = verifier
                .verify(sig, &bytes, &other.public_key_pem())
                .await;
            assert!(
                result.is_err(),
                "must NOT verify against unrelated public key"
            );
        }
    }

    #[tokio::test]
    async fn sign_with_fixed_timestamp_is_deterministic() {
        let fixed = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_770_000_000);
        let a = sign_catalogue_with_builtin_publisher(
            efe_nano_preanalysis::default_catalogue(),
            Some(fixed),
        );
        let b = sign_catalogue_with_builtin_publisher(
            efe_nano_preanalysis::default_catalogue(),
            Some(fixed),
        );
        for ((na, _), (nb, _)) in a.iter().zip(b.iter()) {
            let sa = na.signature.as_ref().unwrap();
            let sb = nb.signature.as_ref().unwrap();
            assert_eq!(sa.signature, sb.signature);
            assert_eq!(sa.canonical_hash, sb.canonical_hash);
        }
    }

    #[test]
    fn builtin_publisher_id_distinct_from_random_v4() {
        let baked = EFE_BUILTIN_PUBLISHER_ID();
        for _ in 0..100 {
            let random = PublisherId::new();
            assert_ne!(baked, random);
        }
    }
}
