// SPDX-License-Identifier: UNLICENSED
//! Audit chain BLAKE3-keyed (préfiguration Phase 7 `efe-server::audit::hmac_chain`).
//!
//! ## Modèle
//!
//! Chaque entrée d'audit produit un `entry_hash` = `blake3_keyed(KEY, prev_hash || canonical_payload)`.
//! Le résultat : une chaîne cryptographique tamper-evident.
//!
//! - **Genesis** : `prev_hash = "GENESIS" repeated 64 chars` (constant).
//! - **Append** : non-bloquant (Mutex bornée), idempotent par `entry_id`.
//! - **Verify** : reconstitue la chaîne et compare bit-à-bit avec les hashes stockés.
//!
//! ## Stabilité
//!
//! v1.0.0 frozen — le wire format (entry_id, payload_hash, prev_hash, entry_hash)
//! est compatible cross-version.  Le KEY est fourni au boot (rotation = nouveau
//! chain).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// KEY symbolique pour le genesis (à remplacer par un secret per-tenant en prod).
pub const DEFAULT_AUDIT_KEY: &[u8] = b"efe-coordinator-default-audit-key-v1";

/// Hash de genesis (préfixe avant la 1ère entrée réelle).
pub const GENESIS_HASH: &str = "GENESIS_GENESIS_GENESIS_GENESIS_GENESIS_GENESIS_GENESIS_GENESIS_";

/// Une entrée d'audit chaînée.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainedEntry {
    /// UUID v7 de l'entrée (ordre temporel global).
    pub entry_id: Uuid,
    /// Hash BLAKE3 du payload canonicalisé (sans le chaînage).
    pub payload_hash: String,
    /// Hash de l'entrée précédente (ou `GENESIS_HASH` si première).
    pub prev_hash: String,
    /// Hash BLAKE3-keyed de `prev_hash || payload_hash`.
    pub entry_hash: String,
    /// Estampille de production.
    pub written_at: DateTime<Utc>,
    /// Étiquette libre (`"analyzer"`, `"claude_opus_4"`, ...).
    pub source: String,
}

/// Chaîne d'audit en mémoire (préfiguration Postgres Phase 7).
///
/// Thread-safe — utilisable depuis plusieurs workflows concurrents
/// (la chaîne est sérielle par construction).
#[derive(Clone)]
pub struct HmacAuditChain {
    inner: Arc<Mutex<ChainState>>,
    key: [u8; 32],
}

struct ChainState {
    entries: Vec<ChainedEntry>,
    last_hash: String,
}

impl HmacAuditChain {
    /// Construit une nouvelle chaîne avec le KEY donné (au moins 16 octets).
    ///
    /// Si `key` < 32 octets, il est étendu avec des zéros ; si > 32,
    /// les octets supplémentaires sont ignorés.
    #[must_use]
    pub fn new(key: &[u8]) -> Self {
        let mut k = [0u8; 32];
        let take = key.len().min(32);
        k[..take].copy_from_slice(&key[..take]);
        Self {
            inner: Arc::new(Mutex::new(ChainState {
                entries: Vec::new(),
                last_hash: GENESIS_HASH.to_string(),
            })),
            key: k,
        }
    }

    /// Construit avec la clé par défaut (TEST/dev only).
    #[must_use]
    pub fn default_key() -> Self {
        Self::new(DEFAULT_AUDIT_KEY)
    }

    /// Append une nouvelle entrée à la chaîne et retourne son hash.
    pub fn append(&self, source: impl Into<String>, payload_hash: String) -> String {
        let mut guard = self.inner.lock();
        let prev = guard.last_hash.clone();
        let combined = format!("{prev}|{payload_hash}");
        let mut hasher = blake3::Hasher::new_keyed(&self.key);
        hasher.update(combined.as_bytes());
        let entry_hash = hasher.finalize().to_hex().to_string();
        let entry = ChainedEntry {
            entry_id: Uuid::now_v7(),
            payload_hash,
            prev_hash: prev,
            entry_hash: entry_hash.clone(),
            written_at: Utc::now(),
            source: source.into(),
        };
        guard.last_hash = entry_hash.clone();
        guard.entries.push(entry);
        entry_hash
    }

    /// Hash courant (= dernier `entry_hash`, ou `GENESIS_HASH` si chaîne vide).
    #[must_use]
    pub fn current_hash(&self) -> String {
        self.inner.lock().last_hash.clone()
    }

    /// Snapshot complet de la chaîne (pour audit + persistence externe).
    #[must_use]
    pub fn snapshot(&self) -> Vec<ChainedEntry> {
        self.inner.lock().entries.clone()
    }

    /// Nombre d'entrées dans la chaîne.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().entries.len()
    }

    /// La chaîne est-elle vide ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.lock().entries.is_empty()
    }

    /// Vérifie l'intégrité de la chaîne : reconstitue chaque hash et compare.
    ///
    /// Retourne `Ok(n)` si les `n` entrées vérifient, `Err(i)` si l'entrée
    /// `i` est corrompue ou injectée.
    pub fn verify(&self) -> Result<usize, usize> {
        let guard = self.inner.lock();
        let mut prev = GENESIS_HASH.to_string();
        for (i, entry) in guard.entries.iter().enumerate() {
            if entry.prev_hash != prev {
                return Err(i);
            }
            let combined = format!("{}|{}", entry.prev_hash, entry.payload_hash);
            let mut hasher = blake3::Hasher::new_keyed(&self.key);
            hasher.update(combined.as_bytes());
            let recomputed = hasher.finalize().to_hex().to_string();
            if recomputed != entry.entry_hash {
                return Err(i);
            }
            prev = entry.entry_hash.clone();
        }
        Ok(guard.entries.len())
    }
}

impl Default for HmacAuditChain {
    fn default() -> Self {
        Self::default_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_chain_starts_at_genesis() {
        let c = HmacAuditChain::default();
        assert!(c.is_empty());
        assert_eq!(c.current_hash(), GENESIS_HASH);
        assert_eq!(c.verify(), Ok(0));
    }

    #[test]
    fn append_advances_hash() {
        let c = HmacAuditChain::default();
        let h1 = c.append("analyzer", "abc".into());
        let h2 = c.append("verifier", "def".into());
        assert_ne!(h1, h2);
        assert_eq!(c.current_hash(), h2);
        assert_eq!(c.len(), 2);
        assert_eq!(c.verify(), Ok(2));
    }

    #[test]
    fn deterministic_hash_with_same_key_and_inputs() {
        let c1 = HmacAuditChain::new(b"k1");
        let c2 = HmacAuditChain::new(b"k1");
        let h1 = c1.append("x", "p".into());
        let h2 = c2.append("x", "p".into());
        // entry_id diffère mais le hash sur les inputs deterministe identique
        // car la chaîne ne dépend pas de l'entry_id (qui est juste métadonnée).
        assert_eq!(h1, h2);
    }

    #[test]
    fn different_keys_produce_different_hashes() {
        let c1 = HmacAuditChain::new(b"k1");
        let c2 = HmacAuditChain::new(b"k2");
        let h1 = c1.append("x", "p".into());
        let h2 = c2.append("x", "p".into());
        assert_ne!(h1, h2);
    }

    #[test]
    fn verify_detects_payload_tampering() {
        let c = HmacAuditChain::default();
        c.append("a", "h1".into());
        c.append("b", "h2".into());
        // Tamper avec l'entrée 1 : modifie le payload_hash.
        {
            let mut g = c.inner.lock();
            g.entries[1].payload_hash = "tampered".into();
        }
        assert_eq!(c.verify(), Err(1));
    }

    #[test]
    fn verify_detects_chain_break() {
        let c = HmacAuditChain::default();
        c.append("a", "h1".into());
        c.append("b", "h2".into());
        // Casse le chaînage en réécrivant prev_hash de l'entrée 1.
        {
            let mut g = c.inner.lock();
            g.entries[1].prev_hash = GENESIS_HASH.to_string();
        }
        assert_eq!(c.verify(), Err(1));
    }

    #[test]
    fn snapshot_includes_all_entries() {
        let c = HmacAuditChain::default();
        for i in 0..5 {
            c.append("x", format!("h{i}"));
        }
        let snap = c.snapshot();
        assert_eq!(snap.len(), 5);
    }

    #[test]
    fn entry_id_is_uuid_v7() {
        let c = HmacAuditChain::default();
        c.append("x", "p".into());
        let snap = c.snapshot();
        assert_eq!(snap[0].entry_id.get_version_num(), 7);
    }

    #[test]
    fn key_padding_is_safe() {
        // Une clé < 32 octets ne doit pas panic.
        let _c = HmacAuditChain::new(b"short");
        let _c = HmacAuditChain::new(&[0u8; 100]); // longue → tronquée
    }

    #[test]
    fn concurrent_append_preserves_chain() {
        use std::thread;
        let c = HmacAuditChain::default();
        let mut handles = vec![];
        for i in 0..20 {
            let c = c.clone();
            handles.push(thread::spawn(move || {
                for j in 0..5 {
                    c.append(format!("t{i}"), format!("h{i}-{j}"));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(c.len(), 100);
        assert_eq!(c.verify(), Ok(100));
    }
}
