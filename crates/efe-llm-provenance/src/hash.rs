// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : BLAKE3 hashing helpers.
//!
//! Wraps the `blake3` crate with the SDK [`Blake3Hash`] newtype so
//! every call site can content-address bytes without re-importing
//! the underlying crypto crate.

use efe_nano_sdk::Blake3Hash;

/// **N17** : Hash arbitrary bytes with BLAKE3-256.
pub fn hash_bytes(bytes: &[u8]) -> Blake3Hash {
    let digest = blake3::hash(bytes);
    Blake3Hash::from_bytes(*digest.as_bytes())
}

/// **N17** : Hash a UTF-8 string with BLAKE3-256.
pub fn hash_str(s: &str) -> Blake3Hash {
    hash_bytes(s.as_bytes())
}

/// **N17** : Bundle of the four canonical prompt hashes for an LLM
/// call.  Computed once at call time ; embedded in the provenance
/// record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptHashes {
    /// Hash of the JSON-serialised inputs that fed the template.
    pub inputs_hash: Blake3Hash,
    /// Hash of the assembled system prompt (post-template-render).
    pub system_hash: Blake3Hash,
    /// Hash of the assembled user prompt.
    pub user_hash: Blake3Hash,
    /// Hash of the full final prompt (system + user concatenated
    /// in provider-specific ways).
    pub final_hash: Blake3Hash,
}

/// **N17** : Compute the four canonical prompt hashes for a call.
///
/// `inputs_json` is the operator input JSON-encoded ; `system` is
/// the system prompt text ; `user` is the user prompt text ;
/// `final_prompt` is whatever the provider client ultimately sends
/// (e.g. JSON-encoded messages array for OpenAI).
pub fn prompt_hashes_for(
    inputs_json: &str,
    system: &str,
    user: &str,
    final_prompt: &str,
) -> PromptHashes {
    PromptHashes {
        inputs_hash: hash_str(inputs_json),
        system_hash: hash_str(system),
        user_hash: hash_str(user),
        final_hash: hash_str(final_prompt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_bytes_is_deterministic() {
        let a = hash_bytes(b"hello");
        let b = hash_bytes(b"hello");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_bytes_collision_resistant() {
        let a = hash_bytes(b"hello");
        let b = hash_bytes(b"helln");
        assert_ne!(a, b);
    }

    #[test]
    fn hash_str_matches_bytes() {
        assert_eq!(hash_str("abc"), hash_bytes(b"abc"));
    }

    #[test]
    fn empty_bytes_has_known_blake3_digest() {
        // BLAKE3 empty digest, hex : af1349b9f5f9a1a6a0404dea36dcc949
        //                            9bcb25c9adc112b7cc9a93cae41f3262
        let h = hash_bytes(b"");
        let hex = h.to_hex();
        assert_eq!(
            hex,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn prompt_hashes_for_distinct_fields() {
        let p = prompt_hashes_for(
            "{\"x\":1}",
            "You are helpful.",
            "Summarise this document.",
            "[system] You are helpful. [user] Summarise this document.",
        );
        // Each hash must be distinct (no collisions on these inputs).
        let v = [p.inputs_hash, p.system_hash, p.user_hash, p.final_hash];
        for i in 0..v.len() {
            for j in (i + 1)..v.len() {
                assert_ne!(v[i], v[j]);
            }
        }
    }

    #[test]
    fn prompt_hashes_stable_across_calls() {
        let a = prompt_hashes_for("i", "s", "u", "f");
        let b = prompt_hashes_for("i", "s", "u", "f");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_is_non_zero_for_any_input() {
        // BLAKE3 has a non-zero digest even for the empty string.
        assert!(!hash_bytes(b"").is_zero());
        assert!(!hash_bytes(b"x").is_zero());
        assert!(!hash_str("hello world").is_zero());
    }
}
