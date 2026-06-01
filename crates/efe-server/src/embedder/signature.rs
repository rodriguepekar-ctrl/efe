//! Sprint **D16** — Ed25519 detached-signature verification on the
//! bundle manifest.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 5.
//!
//! # Threat-model role
//!
//! The signature is the **last line of defense** against a
//! supply-chain attack where the bundle bytes have been tampered
//! with after CI publish (T-16-1, T-16-2, T-16-3). We sign the
//! `manifest.json` bytes ; the manifest itself contains the SHA-256
//! of every other component. So a single Ed25519 verify protects
//! the entire bundle.
//!
//! # Why detached
//!
//! If the signature lived inside `manifest.json`, the
//! `bundle_fingerprint = sha256(manifest.json)` would depend on the
//! signature, which would prevent rotating the signing key without
//! changing every catalog's bound fingerprint. Detached signature
//! decouples key rotation from fingerprint stability — same pattern
//! as Cosign and most package registries.
//!
//! # Failure modes
//!
//! All errors are surfaced as [`SigError`] so the loader can map
//! to `Outcome::SignatureFailed` for telemetry and reject at boot.

use std::time::SystemTime;

use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey, SIGNATURE_LENGTH};
use thiserror::Error;

use super::trust::TrustStore;

/// Error returned by [`verify_manifest_signature`].
#[derive(Debug, Error)]
pub enum SigError {
    /// The signature bytes aren't 64 bytes (Ed25519 spec).
    #[error("malformed signature: expected {SIGNATURE_LENGTH} bytes, got {0}")]
    MalformedSig(usize),

    /// A trust-store entry's public key is unparseable. Should never
    /// happen at runtime if [`super::trust::load_trust_store_file`]
    /// succeeded — but we keep the variant for defense-in-depth.
    #[error("trust store contains an unparseable Ed25519 public key (key_id={0:?})")]
    BadPublicKey(String),

    /// No key in the trust store validated the signature against the
    /// manifest bytes. Could be tampering, key rotation gap, or a
    /// stale trust-store on the verifier.
    #[error(
        "no trust-store key validated the manifest signature (tried {tried} keys, all rejected)"
    )]
    NoMatchingKey {
        /// How many keys were attempted before giving up.
        tried: usize,
    },

    /// The trust store contained at least one key but **none of them**
    /// were within their `valid_from..=valid_until` window for the
    /// current wall clock time. Operator forgot to roll the trust
    /// store before the previous keys expired.
    #[error("trust store has {n} keys but none currently valid (all expired or not yet active)")]
    NoCurrentlyValidKey {
        /// Total keys in the store.
        n: usize,
    },
}

/// Successful verify outcome — surfaces the `key_id` of the key that
/// validated, used as a Prometheus label and in audit logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedSignature {
    /// Operator-chosen id of the key that validated the signature.
    pub key_id: String,
}

/// Verify the detached Ed25519 signature `sig_bytes` over
/// `manifest_bytes` against the keys in `trust`.
///
/// Returns the `key_id` of the verifying key on success, or a
/// [`SigError`] enumerating the failure mode.
///
/// **`now`** is injected for testability — production callers pass
/// `SystemTime::now()`.
pub fn verify_manifest_signature(
    manifest_bytes: &[u8],
    sig_bytes: &[u8],
    trust: &TrustStore,
    now: SystemTime,
) -> Result<VerifiedSignature, SigError> {
    if sig_bytes.len() != SIGNATURE_LENGTH {
        return Err(SigError::MalformedSig(sig_bytes.len()));
    }
    let mut sig_array = [0u8; SIGNATURE_LENGTH];
    sig_array.copy_from_slice(sig_bytes);
    let signature = Signature::from_bytes(&sig_array);

    let now_dt: DateTime<Utc> = now.into();
    // Track how many keys we attempted so we can surface a helpful
    // error variant. We split "no key valid at this clock" from
    // "tried valid keys but none verified" because they imply
    // different operator actions (rotate trust store vs investigate
    // tamper).
    let mut tried = 0usize;
    let mut had_currently_valid = false;
    for key in &trust.keys {
        if key.valid_from > now_dt || key.valid_until < now_dt {
            continue;
        }
        had_currently_valid = true;
        tried += 1;
        let vk = match VerifyingKey::from_bytes(&key.public_key) {
            Ok(vk) => vk,
            // Defensive — should be caught at trust-store load time.
            Err(_) => return Err(SigError::BadPublicKey(key.key_id.clone())),
        };
        if vk.verify(manifest_bytes, &signature).is_ok() {
            return Ok(VerifiedSignature {
                key_id: key.key_id.clone(),
            });
        }
    }
    if !had_currently_valid {
        return Err(SigError::NoCurrentlyValidKey {
            n: trust.keys.len(),
        });
    }
    Err(SigError::NoMatchingKey { tried })
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::trust::TrustedKey;
    use chrono::TimeZone;
    use ed25519_dalek::{Signer, SigningKey};
    use rand_core::OsRng;

    fn make_keypair() -> (SigningKey, [u8; 32]) {
        let sk = SigningKey::generate(&mut OsRng);
        let pk_bytes = sk.verifying_key().to_bytes();
        (sk, pk_bytes)
    }

    fn now_dt() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
    }

    fn now_systemtime() -> SystemTime {
        let dt = now_dt();
        dt.into()
    }

    fn trust_with_pk(key_id: &str, pk: [u8; 32]) -> TrustStore {
        TrustStore {
            keys: vec![TrustedKey {
                key_id: key_id.to_string(),
                public_key: pk,
                active_for_signing: true,
                valid_from: Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
                valid_until: Utc.with_ymd_and_hms(2026, 12, 31, 23, 59, 59).unwrap(),
                comment: None,
            }],
        }
    }

    #[test]
    fn verify_happy_path_returns_key_id() {
        let (sk, pk) = make_keypair();
        let manifest = b"hello manifest";
        let sig = sk.sign(manifest).to_bytes();
        let trust = trust_with_pk("k1", pk);
        let verified = verify_manifest_signature(manifest, &sig, &trust, now_systemtime())
            .expect("happy path");
        assert_eq!(verified.key_id, "k1");
    }

    #[test]
    fn verify_rejects_tampered_manifest() {
        let (sk, pk) = make_keypair();
        let original = b"hello manifest";
        let tampered = b"hello tampered";
        let sig = sk.sign(original).to_bytes();
        let trust = trust_with_pk("k1", pk);
        let err = verify_manifest_signature(tampered, &sig, &trust, now_systemtime()).unwrap_err();
        assert!(matches!(err, SigError::NoMatchingKey { tried: 1 }));
    }

    #[test]
    fn verify_rejects_wrong_signature_length() {
        let trust = trust_with_pk("k1", [0u8; 32]);
        let err =
            verify_manifest_signature(b"x", b"too-short", &trust, now_systemtime()).unwrap_err();
        assert!(matches!(err, SigError::MalformedSig(9)));
    }

    #[test]
    fn verify_rejects_signature_signed_by_unknown_key() {
        let (sk_a, _pk_a) = make_keypair();
        let (_sk_b, pk_b) = make_keypair(); // different keypair
        let manifest = b"hello";
        let sig = sk_a.sign(manifest).to_bytes(); // signed by A
        let trust = trust_with_pk("k_b", pk_b); // trust only B
        let err = verify_manifest_signature(manifest, &sig, &trust, now_systemtime()).unwrap_err();
        assert!(matches!(err, SigError::NoMatchingKey { tried: 1 }));
    }

    #[test]
    fn verify_skips_keys_outside_validity_window() {
        let (sk, pk) = make_keypair();
        let manifest = b"hello";
        let sig = sk.sign(manifest).to_bytes();
        // Trust window 2025 only — but `now` is 2026.
        let mut trust = trust_with_pk("k1", pk);
        trust.keys[0].valid_from = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        trust.keys[0].valid_until = Utc.with_ymd_and_hms(2025, 12, 31, 23, 59, 59).unwrap();
        let err = verify_manifest_signature(manifest, &sig, &trust, now_systemtime()).unwrap_err();
        assert!(matches!(err, SigError::NoCurrentlyValidKey { n: 1 }));
    }

    #[test]
    fn verify_succeeds_via_second_trust_key_after_rotation() {
        let (sk_old, pk_old) = make_keypair();
        let (sk_new, pk_new) = make_keypair();
        let manifest = b"hello";
        // Bundle was signed by NEW key.
        let sig_new = sk_new.sign(manifest).to_bytes();
        let _ = sk_old; // silence unused warning
        let mut trust = trust_with_pk("k_old", pk_old);
        trust.keys.push(TrustedKey {
            key_id: "k_new".into(),
            public_key: pk_new,
            active_for_signing: true,
            valid_from: trust.keys[0].valid_from,
            valid_until: trust.keys[0].valid_until,
            comment: None,
        });
        let verified = verify_manifest_signature(manifest, &sig_new, &trust, now_systemtime())
            .expect("new key validates");
        assert_eq!(verified.key_id, "k_new");
    }
}
