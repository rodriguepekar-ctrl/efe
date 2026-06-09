// SPDX-License-Identifier: UNLICENSED
//! # `hmac` — HMAC-SHA256 chaîné (tamper-evidence).
//!
//! Deux primitives, **même construction** que `efe_nano::audit::payload` :
//!
//! ```text
//! HMAC = HMAC-SHA256( parent_bytes ‖ payload_hash , clé_tenant )
//! parent_bytes = b"genesis"  si pas de parent (1ʳᵉ entrée)
//! ```
//!
//! - [`compute_hmac`] : parent identifié par un **UUID** (16 o) — **byte-identique**
//!   à `efe-nano` (chaîne de traces). Présente pour faire d'`efe-audit-core` le
//!   socle crypto partagé (un futur refactor pourra y rebrancher `efe-nano`).
//! - [`chain_hmac`] : parent identifié par le **HMAC précédent** (32 o) — la chaîne
//!   *hash-linkée* de la provenance par-swarm : muter une entrée antérieure change
//!   son HMAC, donc **toutes** les entrées suivantes (tip inclus). Le tip est le
//!   `provenance_root` : **un seul hash qui prouve tout le run**.
//!
//! La genèse `b"genesis"` est partagée par les deux (convention efe-nano / M23).

use uuid::Uuid;

/// Cœur partagé : `HMAC-SHA256(concat(parts), key)`.
fn hmac_sha256(parts: &[&[u8]], key: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = <Hmac<Sha256>>::new_from_slice(key).expect("HMAC-SHA256 accepte toute longueur de clé");
    for p in parts {
        mac.update(p);
    }
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

/// Octets de genèse, utilisés quand l'entrée n'a pas de parent.
pub const GENESIS: &[u8] = b"genesis";

/// HMAC chaîné par **UUID** parent (identique `efe_nano::audit::payload::compute_hmac`).
pub fn compute_hmac(chain_parent_id: Option<Uuid>, payload_hash: &[u8; 32], key: &[u8]) -> [u8; 32] {
    match chain_parent_id {
        Some(p) => hmac_sha256(&[p.as_bytes(), payload_hash], key),
        None => hmac_sha256(&[GENESIS, payload_hash], key),
    }
}

/// HMAC chaîné par **HMAC précédent** (chaîne hash-linkée de la provenance par-swarm).
/// `parent = None` ⇒ genèse du swarm. Le résultat devient le nouveau tip.
pub fn chain_hmac(chain_parent_hmac: Option<&[u8; 32]>, payload_hash: &[u8; 32], key: &[u8]) -> [u8; 32] {
    match chain_parent_hmac {
        Some(p) => hmac_sha256(&[p.as_slice(), payload_hash], key),
        None => hmac_sha256(&[GENESIS, payload_hash], key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8; 32]) -> String {
        let mut s = String::with_capacity(64);
        for x in b {
            s.push_str(&format!("{x:02x}"));
        }
        s
    }

    // ── compute_hmac (UUID parent, compat efe-nano) ─────────────────────────
    #[test]
    fn compute_hmac_genesis_distinct_from_parent() {
        let payload = [0u8; 32];
        let key = b"some_test_key_32_bytes_length_xx";
        assert_ne!(compute_hmac(None, &payload, key), compute_hmac(Some(Uuid::nil()), &payload, key));
    }

    #[test]
    fn compute_hmac_is_deterministic() {
        let p = Uuid::from_u128(0x9999);
        assert_eq!(compute_hmac(Some(p), &[42u8; 32], b"k"), compute_hmac(Some(p), &[42u8; 32], b"k"));
    }

    #[test]
    fn compute_hmac_sensitive_to_key_and_payload() {
        let p = Some(Uuid::nil());
        assert_ne!(compute_hmac(p, &[0u8; 32], b"key_a"), compute_hmac(p, &[0u8; 32], b"key_b"));
        assert_ne!(compute_hmac(p, &[0u8; 32], b"k"), compute_hmac(p, &[1u8; 32], b"k"));
    }

    // ── chain_hmac (HMAC parent, chaîne par-swarm) ──────────────────────────
    #[test]
    fn chain_hmac_genesis_distinct_from_parent() {
        let payload = [7u8; 32];
        let key = b"k";
        assert_ne!(chain_hmac(None, &payload, key), chain_hmac(Some(&[0u8; 32]), &payload, key));
    }

    #[test]
    fn chain_hmac_sensitive_to_parent() {
        let key = b"k";
        let payload = [1u8; 32];
        assert_ne!(chain_hmac(Some(&[1u8; 32]), &payload, key), chain_hmac(Some(&[2u8; 32]), &payload, key));
    }

    #[test]
    fn chain_hmac_is_deterministic() {
        let parent = [9u8; 32];
        assert_eq!(
            chain_hmac(Some(&parent), &[3u8; 32], b"key"),
            chain_hmac(Some(&parent), &[3u8; 32], b"key")
        );
    }

    // ── GOLDENS : figent la construction byte-à-byte (toute dérive casse). ───
    #[test]
    fn genesis_paths_agree_across_primitives() {
        // compute_hmac(None) et chain_hmac(None) hashent tous deux b"genesis" ‖ payload
        // avec la même clé ⇒ résultat IDENTIQUE (genèse partagée).
        let key = b"test-master-32-bytes-fixed-aaaaaaaaaaaa";
        let payload = [0xABu8; 32];
        assert_eq!(compute_hmac(None, &payload, key), chain_hmac(None, &payload, key));
    }

    #[test]
    fn golden_vectors_lock_the_construction() {
        // Vecteurs FIGÉS : toute dérive de l'algorithme HMAC-SHA256 / de la genèse
        // casse ce test (et romprait la cross-vérifiabilité avec efe-nano).
        let key = b"test-master-32-bytes-fixed-aaaaaaaaaaaa";
        let payload = [0xABu8; 32];
        assert_eq!(
            hex(&chain_hmac(None, &payload, key)),
            "583a95c2cb8af0231bc58e4bea09891bc8f5f2f595891f642e5c315fa87ae907",
            "genèse"
        );
        assert_eq!(
            hex(&chain_hmac(Some(&[0x11u8; 32]), &payload, key)),
            "ccebc3af4684e39038f0cd79c52b24b03cffb7a59d1f1c4b4e75b7ba11692c89",
            "parent 0x11..11"
        );
    }
}
