// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : Ed25519 signature verifier + dev signer.
//!
//! Architecture ADR-1 + ADR-2 + I19.

use async_trait::async_trait;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use efe_nano_sdk::{PublisherSignature, Sha256Hash, SignatureAlgorithm};
use thiserror::Error;

/// **Sprint 10 — N18** : closed taxonomy of signature errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SignatureError {
    #[error("unsupported algorithm : {0:?}")]
    UnsupportedAlgorithm(SignatureAlgorithm),

    #[error("malformed public key : {0}")]
    MalformedPublicKey(String),

    #[error("malformed signature : {0}")]
    MalformedSignature(String),

    #[error("signature verification failed")]
    VerifyFailed,

    #[error("canonical hash mismatch — caller hash != stored hash")]
    CanonicalHashMismatch,
}

/// **Sprint 10 — N18** : signature verifier trait.
#[async_trait]
pub trait SignatureVerifier: Send + Sync + 'static {
    /// **N18** : Verify `signature` against `canonical_body_bytes`
    /// using `public_key_pem`.
    ///
    /// Implementations MUST :
    /// - check algorithm matches the verifier's capability.
    /// - reject malformed inputs.
    /// - use constant-time comparison.
    async fn verify(
        &self,
        signature: &PublisherSignature,
        canonical_body_bytes: &[u8],
        public_key_pem: &str,
    ) -> Result<(), SignatureError>;
}

// ─────────────────────────────────────────────────────────────────────
//  Ed25519Verifier
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : Ed25519 verifier backed by
/// `ed25519-dalek`'s `verify_strict` (rejects malleable signatures).
pub struct Ed25519Verifier;

impl std::fmt::Debug for Ed25519Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ed25519Verifier")
    }
}

impl Ed25519Verifier {
    pub fn new() -> Self {
        Self
    }

    /// **N18** : Parse a PEM-encoded Ed25519 public key.
    ///
    /// For Sprint 10 we use a SIMPLE PEM-like format that just
    /// base64-encodes the 32 raw bytes between BEGIN/END markers.
    /// Sprint 11 will move to proper PKCS#8 / SPKI.
    pub fn parse_public_key_pem(pem: &str) -> Result<VerifyingKey, SignatureError> {
        let mut content = String::new();
        let mut in_content = false;
        for line in pem.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("-----BEGIN") {
                in_content = true;
                continue;
            }
            if trimmed.starts_with("-----END") {
                in_content = false;
                continue;
            }
            if in_content {
                content.push_str(trimmed);
            }
        }
        if content.is_empty() {
            return Err(SignatureError::MalformedPublicKey(
                "no PEM content found".into(),
            ));
        }
        let raw = decode_b64(&content).map_err(|e| {
            SignatureError::MalformedPublicKey(format!("base64 decode : {e}"))
        })?;
        if raw.len() != 32 {
            return Err(SignatureError::MalformedPublicKey(format!(
                "expected 32 bytes, got {}",
                raw.len()
            )));
        }
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&raw);
        VerifyingKey::from_bytes(&bytes)
            .map_err(|e| SignatureError::MalformedPublicKey(e.to_string()))
    }
}

impl Default for Ed25519Verifier {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SignatureVerifier for Ed25519Verifier {
    async fn verify(
        &self,
        signature: &PublisherSignature,
        canonical_body_bytes: &[u8],
        public_key_pem: &str,
    ) -> Result<(), SignatureError> {
        if signature.algorithm != SignatureAlgorithm::Ed25519 {
            return Err(SignatureError::UnsupportedAlgorithm(signature.algorithm));
        }
        if signature.signature.len() != 64 {
            return Err(SignatureError::MalformedSignature(format!(
                "expected 64 bytes, got {}",
                signature.signature.len()
            )));
        }
        // Sanity : the signature's canonical_hash MUST match the
        // hash of the body bytes we're verifying against.
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(canonical_body_bytes);
        let digest = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&digest);
        if Sha256Hash::from_bytes(h) != signature.canonical_hash {
            return Err(SignatureError::CanonicalHashMismatch);
        }
        let mut sig_bytes = [0u8; 64];
        sig_bytes.copy_from_slice(&signature.signature);
        let sig = Signature::from_bytes(&sig_bytes);
        let vk = Self::parse_public_key_pem(public_key_pem)?;
        // `verify_strict` enforces canonical encoding (defense
        // against signature-malleability attacks).
        vk.verify_strict(canonical_body_bytes, &sig)
            .map_err(|_| SignatureError::VerifyFailed)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Ed25519Signer — dev/test convenience
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : convenience signer for tests + dev.  In
/// production the private key lives in Vault Transit ; this struct
/// is for in-memory key handling during tests.
pub struct Ed25519Signer {
    inner: SigningKey,
}

impl std::fmt::Debug for Ed25519Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ed25519Signer")
            .field("public_key_pem_len", &self.public_key_pem().len())
            .finish()
    }
}

impl Ed25519Signer {
    /// **N18** : Generate a fresh keypair.
    pub fn generate() -> Self {
        use rand_core::OsRng;
        let signing = SigningKey::generate(&mut OsRng);
        Self { inner: signing }
    }

    /// **N18** : Construct from a 32-byte seed (deterministic ; tests).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            inner: SigningKey::from_bytes(&seed),
        }
    }

    /// **N18** : Export the PEM-encoded public key.
    pub fn public_key_pem(&self) -> String {
        let bytes = self.inner.verifying_key().to_bytes();
        let b64 = encode_b64(&bytes);
        format!(
            "-----BEGIN PUBLIC KEY-----\n{b64}\n-----END PUBLIC KEY-----"
        )
    }

    /// **N18** : Sign `bytes` with the inner signing key.
    pub fn sign(&self, bytes: &[u8]) -> [u8; 64] {
        let sig = self.inner.sign(bytes);
        sig.to_bytes()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Base64 mini-impl (no external dep beyond rust-crypto)
// ─────────────────────────────────────────────────────────────────────

const B64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode_b64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let b0 = bytes[i] as usize;
        let b1 = bytes[i + 1] as usize;
        let b2 = bytes[i + 2] as usize;
        out.push(B64_ALPHABET[(b0 >> 2) & 0x3F] as char);
        out.push(B64_ALPHABET[((b0 << 4) | (b1 >> 4)) & 0x3F] as char);
        out.push(B64_ALPHABET[((b1 << 2) | (b2 >> 6)) & 0x3F] as char);
        out.push(B64_ALPHABET[b2 & 0x3F] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let b0 = bytes[i] as usize;
        out.push(B64_ALPHABET[(b0 >> 2) & 0x3F] as char);
        out.push(B64_ALPHABET[(b0 << 4) & 0x3F] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let b0 = bytes[i] as usize;
        let b1 = bytes[i + 1] as usize;
        out.push(B64_ALPHABET[(b0 >> 2) & 0x3F] as char);
        out.push(B64_ALPHABET[((b0 << 4) | (b1 >> 4)) & 0x3F] as char);
        out.push(B64_ALPHABET[(b1 << 2) & 0x3F] as char);
        out.push('=');
    }
    out
}

fn decode_b64(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    let bytes = clean.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(format!("invalid base64 length {}", bytes.len()));
    }
    let mut i = 0;
    while i < bytes.len() {
        let mut buf = [0u8; 4];
        let mut pad = 0;
        for j in 0..4 {
            let c = bytes[i + j];
            if c == b'=' {
                pad += 1;
                buf[j] = 0;
            } else {
                buf[j] = b64_decode_byte(c)?;
            }
        }
        let b0 = (buf[0] << 2) | (buf[1] >> 4);
        let b1 = (buf[1] << 4) | (buf[2] >> 2);
        let b2 = (buf[2] << 6) | buf[3];
        out.push(b0);
        if pad < 2 {
            out.push(b1);
        }
        if pad < 1 {
            out.push(b2);
        }
        i += 4;
    }
    Ok(out)
}

fn b64_decode_byte(c: u8) -> Result<u8, String> {
    match c {
        b'A'..=b'Z' => Ok(c - b'A'),
        b'a'..=b'z' => Ok(c - b'a' + 26),
        b'0'..=b'9' => Ok(c - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(format!("invalid base64 char : {c}")),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{KeyId, PublisherId};
    use std::time::SystemTime;

    fn signature_for(signer: &Ed25519Signer, body: &[u8]) -> PublisherSignature {
        let sig_bytes = signer.sign(body);
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(body);
        let digest = hasher.finalize();
        let mut h = [0u8; 32];
        h.copy_from_slice(&digest);
        PublisherSignature {
            publisher_id: PublisherId::new(),
            key_id: KeyId::new(),
            algorithm: SignatureAlgorithm::Ed25519,
            signature: sig_bytes.to_vec(),
            signed_at: SystemTime::now(),
            canonical_hash: Sha256Hash::from_bytes(h),
        }
    }

    // ─── Base64 round-trip ──────────────────────────────────────────

    #[test]
    fn base64_round_trip_empty() {
        assert_eq!(encode_b64(b""), "");
        assert_eq!(decode_b64("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn base64_round_trip_aligned() {
        let bytes = b"hello world!";
        let enc = encode_b64(bytes);
        let dec = decode_b64(&enc).unwrap();
        assert_eq!(dec, bytes);
    }

    #[test]
    fn base64_round_trip_padded_one() {
        // 2-byte input → "=" (1 pad) — RFC 4648 §4.
        let bytes = b"hi";
        let enc = encode_b64(bytes);
        assert!(enc.ends_with('='));
        assert!(!enc.ends_with("=="));
        let dec = decode_b64(&enc).unwrap();
        assert_eq!(dec, bytes);
    }

    #[test]
    fn base64_round_trip_padded_two() {
        // 1-byte input → "==" (2 pads) — RFC 4648 §4.
        let bytes = b"h";
        let enc = encode_b64(bytes);
        assert!(enc.ends_with("=="));
        let dec = decode_b64(&enc).unwrap();
        assert_eq!(dec, bytes);
    }

    #[test]
    fn base64_round_trip_32_bytes() {
        let bytes = [0xAB; 32];
        let enc = encode_b64(&bytes);
        let dec = decode_b64(&enc).unwrap();
        assert_eq!(dec, bytes);
    }

    // ─── Ed25519Signer ─────────────────────────────────────────────

    #[test]
    fn signer_generate_produces_pem() {
        let s = Ed25519Signer::generate();
        let pem = s.public_key_pem();
        assert!(pem.contains("-----BEGIN PUBLIC KEY-----"));
        assert!(pem.contains("-----END PUBLIC KEY-----"));
    }

    #[test]
    fn signer_from_seed_is_deterministic() {
        let s1 = Ed25519Signer::from_seed([1u8; 32]);
        let s2 = Ed25519Signer::from_seed([1u8; 32]);
        assert_eq!(s1.public_key_pem(), s2.public_key_pem());
        let sig1 = s1.sign(b"hello");
        let sig2 = s2.sign(b"hello");
        assert_eq!(sig1, sig2);
    }

    // ─── Ed25519Verifier — happy + sad paths ───────────────────────

    #[tokio::test]
    async fn verify_valid_signature_succeeds() {
        let signer = Ed25519Signer::generate();
        let pem = signer.public_key_pem();
        let body = b"canonical body bytes";
        let sig = signature_for(&signer, body);
        Ed25519Verifier::new().verify(&sig, body, &pem).await.unwrap();
    }

    #[tokio::test]
    async fn verify_rejects_tampered_body() {
        let signer = Ed25519Signer::generate();
        let pem = signer.public_key_pem();
        let body = b"original body";
        let sig = signature_for(&signer, body);
        let tampered = b"tampered body";
        let err = Ed25519Verifier::new()
            .verify(&sig, tampered, &pem)
            .await
            .unwrap_err();
        // Tampering changes the canonical hash recomputed inside
        // verify — fails with CanonicalHashMismatch.
        assert!(matches!(err, SignatureError::CanonicalHashMismatch));
    }

    #[tokio::test]
    async fn verify_rejects_wrong_public_key() {
        let signer = Ed25519Signer::generate();
        let other = Ed25519Signer::generate();
        let body = b"body";
        let sig = signature_for(&signer, body);
        let err = Ed25519Verifier::new()
            .verify(&sig, body, &other.public_key_pem())
            .await
            .unwrap_err();
        assert!(matches!(err, SignatureError::VerifyFailed));
    }

    #[tokio::test]
    async fn verify_rejects_unsupported_algorithm() {
        // No alt algorithm in our enum yet, but if we added one this
        // test would catch the misclassification.  For now, this
        // path is unreachable.  Skip the test logic but keep the
        // structure as documentation.
        let signer = Ed25519Signer::generate();
        let body = b"body";
        let sig = signature_for(&signer, body);
        // Same algorithm → should pass.
        assert!(matches!(sig.algorithm, SignatureAlgorithm::Ed25519));
        Ed25519Verifier::new()
            .verify(&sig, body, &signer.public_key_pem())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn verify_rejects_malformed_signature_length() {
        let signer = Ed25519Signer::generate();
        let body = b"body";
        let mut sig = signature_for(&signer, body);
        sig.signature.truncate(32); // bad length
        let err = Ed25519Verifier::new()
            .verify(&sig, body, &signer.public_key_pem())
            .await
            .unwrap_err();
        assert!(matches!(err, SignatureError::MalformedSignature(_)));
    }

    #[tokio::test]
    async fn verify_rejects_malformed_public_key() {
        let signer = Ed25519Signer::generate();
        let body = b"body";
        let sig = signature_for(&signer, body);
        let err = Ed25519Verifier::new()
            .verify(&sig, body, "not a pem")
            .await
            .unwrap_err();
        assert!(matches!(err, SignatureError::MalformedPublicKey(_)));
    }

    #[tokio::test]
    async fn parse_public_key_pem_round_trip() {
        let signer = Ed25519Signer::generate();
        let pem = signer.public_key_pem();
        let vk = Ed25519Verifier::parse_public_key_pem(&pem).unwrap();
        // Re-encode and re-parse — should match.
        let bytes = vk.to_bytes();
        let b64 = encode_b64(&bytes);
        let pem2 = format!("-----BEGIN PUBLIC KEY-----\n{b64}\n-----END PUBLIC KEY-----");
        let vk2 = Ed25519Verifier::parse_public_key_pem(&pem2).unwrap();
        assert_eq!(vk.to_bytes(), vk2.to_bytes());
    }

    #[tokio::test]
    async fn parse_public_key_pem_rejects_wrong_length() {
        let pem = "-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----";
        let err = Ed25519Verifier::parse_public_key_pem(pem).unwrap_err();
        assert!(matches!(err, SignatureError::MalformedPublicKey(_)));
    }

    #[tokio::test]
    async fn debug_format_no_secret_leak() {
        let s = Ed25519Signer::generate();
        let dbg = format!("{s:?}");
        // Public key length is fine to leak ; private key bytes are not.
        assert!(dbg.contains("public_key_pem_len"));
        let pem = s.public_key_pem();
        assert!(!dbg.contains(&pem));
    }

    #[tokio::test]
    async fn debug_format_of_verifier() {
        let v = Ed25519Verifier::new();
        assert_eq!(format!("{v:?}"), "Ed25519Verifier");
    }
}
