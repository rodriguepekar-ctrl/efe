// SPDX-License-Identifier: UNLICENSED
//! # `hash` — primitives de hash de la provenance par-agent.
//!
//! Deux familles, **délibérément distinctes** (alignées sur l'existant) :
//! - **BLAKE3-256** pour le **contenu** (`input_hash`, `output_hash`) — content-
//!   addressing, aligné `efe-llm-provenance` (N17) qui hashe ses prompts/sorties
//!   en BLAKE3. Type partagé : [`efe_nano_sdk::Blake3Hash`].
//! - **SHA-256** pour le **`payload_hash`** d'un enregistrement — aligné la chaîne
//!   d'audit HMAC d'`efe-nano` (`HMAC-SHA256(parent ‖ payload_hash, clé)`), pour que
//!   les deux chaînes restent mutuellement vérifiables (§3.2).
//!
//! Toutes les fonctions sont **pures et déterministes** (aucun aléa, aucune horloge).

use efe_nano_sdk::Blake3Hash;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::canon::canonical_bytes;

/// BLAKE3-256 d'octets bruts → [`Blake3Hash`].
pub fn blake3_bytes(bytes: &[u8]) -> Blake3Hash {
    Blake3Hash::from_bytes(*blake3::hash(bytes).as_bytes())
}

/// BLAKE3-256 d'une valeur JSON, hashée sous sa forme **canonique** : deux valeurs
/// logiquement égales (ordre de clés différent) produisent le **même** hash de
/// contenu. C'est le hash utilisé pour `input_hash` / `output_hash`.
pub fn blake3_value(v: &Value) -> Blake3Hash {
    blake3_bytes(&canonical_bytes(v))
}

/// SHA-256 d'octets bruts → digest 32 octets (le bloc HMAC-SHA256).
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// SHA-256 de la forme canonique d'une valeur JSON — le `payload_hash` d'un corps
/// d'enregistrement (construit en clés triées par [`crate::record`]).
pub fn payload_hash_of(body: &Value) -> [u8; 32] {
    sha256(&canonical_bytes(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blake3_is_deterministic() {
        assert_eq!(blake3_bytes(b"hello"), blake3_bytes(b"hello"));
    }

    #[test]
    fn blake3_empty_matches_known_vector() {
        // Vecteur connu BLAKE3 de l'entrée vide (identique au test N17 de
        // `efe-llm-provenance`). Verrouille l'algorithme.
        assert_eq!(
            blake3_bytes(b"").to_hex(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn blake3_distinct_inputs_distinct_hashes() {
        assert_ne!(blake3_bytes(b"a"), blake3_bytes(b"b"));
    }

    #[test]
    fn blake3_value_is_canonical_insensitive_to_key_order() {
        let a = json!({ "x": 1, "y": 2 });
        let b = json!({ "y": 2, "x": 1 });
        assert_eq!(blake3_value(&a), blake3_value(&b), "hash de contenu canonique");
    }

    #[test]
    fn blake3_value_changes_with_content() {
        assert_ne!(blake3_value(&json!({ "v": 1 })), blake3_value(&json!({ "v": 2 })));
    }

    #[test]
    fn sha256_is_32_bytes_and_deterministic() {
        let a = sha256(b"payload");
        let b = sha256(b"payload");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn payload_hash_of_is_key_order_invariant() {
        let a = json!({ "b": 2, "a": 1 });
        let b = json!({ "a": 1, "b": 2 });
        assert_eq!(payload_hash_of(&a), payload_hash_of(&b));
    }
}
