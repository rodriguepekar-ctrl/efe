// SPDX-License-Identifier: UNLICENSED
//! **Sprint 10 — N18** : Private-key custody abstraction (ADR-2).
//!
//! ADR-2 mandates production private keys live in HashiCorp Vault
//! Transit (HSM-backed, signing via API, key never leaves Vault) while
//! dev / staging may use in-memory keys.  This module formalises that
//! split as a `KeyStore` trait.
//!
//! Sprint 10 ships :
//! - the trait (provider-agnostic interface),
//! - `InMemoryKeyStore` (tests + dev),
//! - `KeyStoreError` (closed taxonomy).
//!
//! Sprint 11 adds `VaultTransitKeyStore` (real Vault wire integration)
//! and `AwsKmsKeyStore` (rejected alternative — kept as a stub for
//! future evaluation).
//!
//! # Custody guarantees
//!
//! - `sign(key_id, body)` performs a signing operation **inside the
//!   custody boundary** (Vault HSM in prod ; in-memory in dev).  The
//!   raw private key is NEVER returned by the trait.
//! - `public_key_pem(key_id)` returns the PEM-encoded public half ;
//!   safe to share / display / store in the registry.
//! - `generate(algorithm)` mints a fresh keypair *inside* the custody
//!   boundary ; returns the new `KeyId` + public PEM.  The private
//!   half stays in the store.
//! - `rotate(old_key_id, algorithm)` is sugar for `generate` + audit
//!   tag — the old key remains in the store (I7 : old signatures
//!   continue to verify).

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use efe_nano_sdk::{KeyId, SignatureAlgorithm};
use thiserror::Error;

use crate::verifier::Ed25519Signer;

/// **Sprint 10 — N18** : closed taxonomy of key-store errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum KeyStoreError {
    #[error("key {key_id:?} not found in custody")]
    KeyNotFound { key_id: KeyId },

    #[error("algorithm {algorithm:?} not supported by this store")]
    UnsupportedAlgorithm { algorithm: SignatureAlgorithm },

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **Sprint 10 — N18 + ADR-2** : custody-bounded keypair store.
///
/// In production, the implementation talks to Vault Transit ; in
/// tests, the `InMemoryKeyStore` keeps raw bytes (acceptable because
/// test keys are throwaway).
#[async_trait]
pub trait KeyStore: Send + Sync + 'static {
    /// **N18** : Mint a fresh keypair.  Returns `(key_id, public_pem)`.
    async fn generate(
        &self,
        algorithm: SignatureAlgorithm,
    ) -> Result<(KeyId, String), KeyStoreError>;

    /// **N18** : Sign `body` using the key identified by `key_id`.
    ///
    /// Returns the raw signature bytes (64 bytes for Ed25519).  The
    /// private key NEVER leaves the custody boundary.
    async fn sign(
        &self,
        key_id: KeyId,
        body: &[u8],
    ) -> Result<Vec<u8>, KeyStoreError>;

    /// **N18** : Return the PEM-encoded public key for `key_id`.
    /// Returns `KeyNotFound` if the key was never generated / has been
    /// purged.
    async fn public_key_pem(
        &self,
        key_id: KeyId,
    ) -> Result<String, KeyStoreError>;

    /// **N18 + I7** : Mint a successor key + return both ids + the new
    /// PEM.  The old key remains in the store (verification of
    /// historical signatures continues to work).
    async fn rotate(
        &self,
        old_key_id: KeyId,
        algorithm: SignatureAlgorithm,
    ) -> Result<(KeyId, String), KeyStoreError> {
        // Default impl : confirm old exists, then `generate` a fresh.
        let _ = self.public_key_pem(old_key_id).await?;
        self.generate(algorithm).await
    }

    /// **N18 + ADR-2** : Sign the bytes of `new_public_key_pem` using
    /// `current_key_id` to produce a **rotation proof**.  This proof
    /// is consumed by `PublisherRegistry::rotate_key_with_proof`.
    async fn produce_rotation_proof(
        &self,
        current_key_id: KeyId,
        new_public_key_pem: &str,
    ) -> Result<Vec<u8>, KeyStoreError> {
        self.sign(current_key_id, new_public_key_pem.as_bytes()).await
    }
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryKeyStore (dev / tests)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 10 — N18** : in-memory implementation backed by
/// `Ed25519Signer` keypairs.  NEVER use in production — `VaultTransit
/// KeyStore` is the production-grade alternative (Sprint 11).
pub struct InMemoryKeyStore {
    /// `key_id` → signer (signer holds the private half).
    keys: Mutex<HashMap<KeyId, Ed25519Signer>>,
}

impl std::fmt::Debug for InMemoryKeyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryKeyStore")
            .field("keys", &self.keys.lock().map(|m| m.len()).unwrap_or(0))
            .finish()
    }
}

impl Default for InMemoryKeyStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryKeyStore {
    pub fn new() -> Self {
        Self {
            keys: Mutex::new(HashMap::new()),
        }
    }

    /// Inject a pre-built signer under a fresh `KeyId` (test-only).
    pub fn install(&self, signer: Ed25519Signer) -> KeyId {
        let id = KeyId::new();
        if let Ok(mut m) = self.keys.lock() {
            m.insert(id, signer);
        }
        id
    }

    /// Test-only : number of stored keys.
    pub fn len(&self) -> usize {
        self.keys.lock().map(|m| m.len()).unwrap_or(0)
    }

    /// Test-only : true if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl KeyStore for InMemoryKeyStore {
    async fn generate(
        &self,
        algorithm: SignatureAlgorithm,
    ) -> Result<(KeyId, String), KeyStoreError> {
        match algorithm {
            SignatureAlgorithm::Ed25519 => {
                let signer = Ed25519Signer::generate();
                let pem = signer.public_key_pem();
                let id = self.install(signer);
                Ok((id, pem))
            }
        }
    }

    async fn sign(
        &self,
        key_id: KeyId,
        body: &[u8],
    ) -> Result<Vec<u8>, KeyStoreError> {
        let m = self.keys.lock().map_err(|e| KeyStoreError::Backend {
            message: e.to_string(),
        })?;
        let signer = m.get(&key_id).ok_or(KeyStoreError::KeyNotFound { key_id })?;
        let sig = signer.sign(body);
        Ok(sig.to_vec())
    }

    async fn public_key_pem(
        &self,
        key_id: KeyId,
    ) -> Result<String, KeyStoreError> {
        let m = self.keys.lock().map_err(|e| KeyStoreError::Backend {
            message: e.to_string(),
        })?;
        let signer = m.get(&key_id).ok_or(KeyStoreError::KeyNotFound { key_id })?;
        Ok(signer.public_key_pem())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn generate_then_sign_round_trip() {
        let ks = InMemoryKeyStore::new();
        let (id, pem) = ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
        let sig = ks.sign(id, b"hello").await.unwrap();
        assert_eq!(sig.len(), 64);
        // Verify via the public key.
        use crate::verifier::Ed25519Verifier;
        use efe_nano_sdk::{PublisherId, PublisherSignature, Sha256Hash, SignatureAlgorithm as Algo};
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(b"hello");
        let d = h.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&d);
        let proof = PublisherSignature {
            publisher_id: PublisherId::new(),
            key_id: id,
            algorithm: Algo::Ed25519,
            signature: sig,
            signed_at: std::time::SystemTime::now(),
            canonical_hash: Sha256Hash::from_bytes(hash),
        };
        let v = Ed25519Verifier::new();
        use crate::verifier::SignatureVerifier;
        v.verify(&proof, b"hello", &pem).await.unwrap();
    }

    #[tokio::test]
    async fn sign_unknown_key_returns_not_found() {
        let ks = InMemoryKeyStore::new();
        let err = ks.sign(KeyId::new(), b"x").await.unwrap_err();
        assert!(matches!(err, KeyStoreError::KeyNotFound { .. }));
    }

    #[tokio::test]
    async fn public_key_pem_unknown_returns_not_found() {
        let ks = InMemoryKeyStore::new();
        let err = ks.public_key_pem(KeyId::new()).await.unwrap_err();
        assert!(matches!(err, KeyStoreError::KeyNotFound { .. }));
    }

    #[tokio::test]
    async fn rotate_preserves_old_key() {
        let ks = InMemoryKeyStore::new();
        let (old_id, _old_pem) = ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
        let (new_id, _new_pem) = ks.rotate(old_id, SignatureAlgorithm::Ed25519).await.unwrap();
        assert_ne!(old_id, new_id);
        // Old key still usable (I7).
        let sig = ks.sign(old_id, b"replay").await.unwrap();
        assert_eq!(sig.len(), 64);
        // New key also signs.
        let sig2 = ks.sign(new_id, b"fresh").await.unwrap();
        assert_eq!(sig2.len(), 64);
        assert_ne!(sig, sig2);
    }

    #[tokio::test]
    async fn rotate_unknown_old_key_fails() {
        let ks = InMemoryKeyStore::new();
        let err = ks
            .rotate(KeyId::new(), SignatureAlgorithm::Ed25519)
            .await
            .unwrap_err();
        assert!(matches!(err, KeyStoreError::KeyNotFound { .. }));
    }

    #[tokio::test]
    async fn produce_rotation_proof_round_trip() {
        let ks = InMemoryKeyStore::new();
        let (cur_id, cur_pem) = ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
        let (_new_id, new_pem) = ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
        let proof = ks.produce_rotation_proof(cur_id, &new_pem).await.unwrap();
        // Verify the proof manually : sig of new_pem bytes by cur key.
        assert_eq!(proof.len(), 64);
        use crate::verifier::Ed25519Verifier;
        use efe_nano_sdk::{
            PublisherId, PublisherSignature, Sha256Hash, SignatureAlgorithm as Algo,
        };
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(new_pem.as_bytes());
        let d = h.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&d);
        let sig_struct = PublisherSignature {
            publisher_id: PublisherId::new(),
            key_id: cur_id,
            algorithm: Algo::Ed25519,
            signature: proof,
            signed_at: std::time::SystemTime::now(),
            canonical_hash: Sha256Hash::from_bytes(hash),
        };
        use crate::verifier::SignatureVerifier;
        let v = Ed25519Verifier::new();
        v.verify(&sig_struct, new_pem.as_bytes(), &cur_pem)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn install_then_sign_uses_provided_signer() {
        let ks = InMemoryKeyStore::new();
        let signer = Ed25519Signer::from_seed([42u8; 32]);
        let pem_before = signer.public_key_pem();
        let id = ks.install(signer);
        let pem_after = ks.public_key_pem(id).await.unwrap();
        assert_eq!(pem_before, pem_after);
    }

    #[tokio::test]
    async fn len_and_is_empty() {
        let ks = InMemoryKeyStore::new();
        assert!(ks.is_empty());
        assert_eq!(ks.len(), 0);
        let _ = ks.generate(SignatureAlgorithm::Ed25519).await.unwrap();
        assert_eq!(ks.len(), 1);
        assert!(!ks.is_empty());
    }

    #[test]
    fn debug_format_no_secret_leak() {
        let ks = InMemoryKeyStore::new();
        let s = format!("{ks:?}");
        assert!(s.contains("InMemoryKeyStore"));
        assert!(s.contains("keys"));
        // Must NOT leak any private bytes (we only print count).
        assert!(!s.contains("0x"));
    }
}
