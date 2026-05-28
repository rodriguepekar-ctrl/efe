// SPDX-License-Identifier: UNLICENSED
//! Workflows haut-niveau du Coordinator.
//!
//! - [`run_mission_a`] : pré-mâcher pour 1 LLM.
//! - [`run_mission_b`] : fan-out N LLMs + vote.
//! - [`run_speculative`] : drafter + verifier.

mod mission_a;
mod mission_b;
mod speculative;
pub(crate) mod slm_guard;

pub use mission_a::run_mission_a;
pub use mission_b::run_mission_b;
pub use speculative::run_speculative;

/// Helper : calcule un hash BLAKE3 stable d'une chaîne d'audit
/// (concatène les entry_id en ordre temporel).
pub(crate) fn audit_chain_hash(entries: &[uuid::Uuid]) -> String {
    let mut hasher = blake3::Hasher::new();
    for e in entries {
        hasher.update(e.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_empty_is_blake3_default() {
        let h = audit_chain_hash(&[]);
        assert_eq!(h.len(), 64);
        assert_eq!(h, blake3::Hasher::new().finalize().to_hex().to_string());
    }

    #[test]
    fn hash_is_deterministic() {
        let e1 = uuid::Uuid::now_v7();
        let e2 = uuid::Uuid::now_v7();
        let h1 = audit_chain_hash(&[e1, e2]);
        let h2 = audit_chain_hash(&[e1, e2]);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_changes_when_order_changes() {
        let e1 = uuid::Uuid::now_v7();
        let e2 = uuid::Uuid::now_v7();
        let h1 = audit_chain_hash(&[e1, e2]);
        let h2 = audit_chain_hash(&[e2, e1]);
        assert_ne!(h1, h2);
    }
}
