//! **Sprint 7 — N9** : Merkle tree algorithm (RFC 6962 style).
//!
//! # Why RFC 6962 ?
//!
//! Certificate Transparency (CT) introduced this exact construction for
//! tamper-evident logs.  It is the most-deployed audit-grade Merkle
//! variant in production (Google CT logs, sigstore Rekor, Trillian).
//! Its key property is **domain separation** : leaf hashes and internal
//! node hashes are computed with different prefix bytes, so a node
//! cannot be confused with a leaf — this defeats the classic Merkle
//! "second pre-image" attack where an attacker presents a node hash as
//! if it were a leaf to forge an inclusion proof.
//!
//! ## Hash function
//!
//! SHA-256 throughout, matching the rest of the EFE audit chain
//! (cf [`crate::audit::payload`] and [`crate::audit::hmac_chain`]).
//!
//! ## Leaf hash
//!
//! ```text
//! leaf_hash = SHA-256( 0x00 ‖ payload_bytes )
//! ```
//!
//! For EFE traces, `payload_bytes` is the `payload_hash` field of the
//! existing [`crate::audit::chain_store::ChainEntry`] (already SHA-256
//! of the canonical row body).  So Merkle leaves are SHA-256 hashes of
//! SHA-256 hashes — that's intentional, the domain byte breaks the
//! collision domain.
//!
//! ## Internal node hash
//!
//! ```text
//! node_hash = SHA-256( 0x01 ‖ left_child ‖ right_child )
//! ```
//!
//! ## Odd-leaf-count handling
//!
//! When a level has an odd number of nodes, RFC 6962 specifies that
//! the **last node is promoted unchanged to the next level** (i.e. NOT
//! duplicated and hashed with itself).  This is what `build_root`
//! implements.  This contrasts with Bitcoin's Merkle convention
//! (duplicate-and-hash) which is vulnerable to the CVE-2012-2459
//! malleability bug.
//!
//! ## Inclusion proof
//!
//! Given a leaf at index `i` in a tree of `n` leaves, the inclusion
//! proof is the **sibling path** : for each level (bottom-up), the
//! sibling hash on the OTHER side of the path.  Length = `ceil(log2(n))`.
//! Verifier rebuilds the root from `(leaf_hash, siblings, leaf_index)`
//! and compares to the expected root in O(log N).
//!
//! ## Why not blake3 / blake2 ?
//!
//! BLAKE3 is faster (parallelizable + SIMD) but the EFE audit chain
//! is already SHA-256-locked at the HMAC payload layer (Sprint 6).
//! Mixing hash functions across layers would make the auditor's job
//! harder and require dual-implementation in every replay.  SHA-256
//! throughout is the correct choice for audit-grade interoperability.

use sha2::{Digest, Sha256};
use thiserror::Error;

// ─────────────────────────────────────────────────────────────────────
//  Domain separation constants
// ─────────────────────────────────────────────────────────────────────

/// Leaf domain prefix.  Prevents leaf-vs-node confusion.
pub const LEAF_DOMAIN: u8 = 0x00;

/// Internal-node domain prefix.
pub const NODE_DOMAIN: u8 = 0x01;

// ─────────────────────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MerkleError {
    /// `build_root` called with zero leaves.
    #[error("merkle tree must have at least one leaf")]
    EmptyTree,

    /// `inclusion_proof` called with `leaf_index >= leaves.len()`.
    #[error("leaf index {index} out of bounds (have {len} leaves)")]
    IndexOutOfBounds { index: usize, len: usize },

    /// `verify_inclusion` received a proof whose computed root doesn't
    /// match the expected root.
    #[error("inclusion proof verification failed : computed root differs from expected")]
    ProofMismatch,
}

// ─────────────────────────────────────────────────────────────────────
//  Hashing helpers
// ─────────────────────────────────────────────────────────────────────

/// Hash a leaf payload with the leaf domain prefix.
///
/// `payload` is typically the `payload_hash` field of a `ChainEntry`
/// (already a SHA-256 digest), but the function accepts any byte slice.
pub fn leaf_hash(payload: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([LEAF_DOMAIN]);
    h.update(payload);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// Hash two children into an internal node hash with the node domain prefix.
pub fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update([NODE_DOMAIN]);
    h.update(left);
    h.update(right);
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

// ─────────────────────────────────────────────────────────────────────
//  Tree construction
// ─────────────────────────────────────────────────────────────────────

/// Build the Merkle root over `leaves`.
///
/// `leaves[i]` is interpreted as the **raw payload bytes** for leaf `i`
/// (NOT yet leaf-hashed).  The function applies [`leaf_hash`] internally.
///
/// Returns the 32-byte SHA-256 root.
///
/// # Errors
///
/// - [`MerkleError::EmptyTree`] if `leaves` is empty.  An empty audit
///   epoch is a logic error — callers should check `chain.len() > 0`
///   before sealing.
///
/// # Algorithm
///
/// Iterative bottom-up pairing per RFC 6962 § 2.1 :
///
/// ```text
/// MTH({}) = SHA-256(0x00)    -- (we treat this as an error instead)
/// MTH({d1}) = leaf_hash(d1)
/// MTH(D) = SHA-256(0x01 ‖ MTH(D[0..k]) ‖ MTH(D[k..n]))
///   where k is the largest power of 2 < n
/// ```
///
/// Concretely, at each level :
/// - Pair adjacent nodes (i, i+1) → next level's node `H(0x01 ‖ left ‖ right)`.
/// - **If odd count, promote the last node unchanged.**
/// - Repeat until one node remains : the root.
///
/// # Complexity
///
/// Time : O(N).  Memory : O(N) (we hold all intermediate levels for
/// later inclusion-proof reconstruction).
pub fn build_root(leaves: &[Vec<u8>]) -> Result<[u8; 32], MerkleError> {
    if leaves.is_empty() {
        return Err(MerkleError::EmptyTree);
    }
    let tree = build_tree(leaves)?;
    Ok(*tree.root())
}

/// Build the **complete tree** (every level, including leaves and root)
/// over the given leaf payloads.  Returned [`MerkleTree`] supports
/// `root()` and `inclusion_proof(i)`.
pub fn build_tree(leaves: &[Vec<u8>]) -> Result<MerkleTree, MerkleError> {
    if leaves.is_empty() {
        return Err(MerkleError::EmptyTree);
    }
    // Level 0 : leaf hashes (with domain prefix).
    let leaf_level: Vec<[u8; 32]> = leaves.iter().map(|p| leaf_hash(p)).collect();

    let mut levels: Vec<Vec<[u8; 32]>> = vec![leaf_level];

    // Build up.
    while levels.last().unwrap().len() > 1 {
        let prev = levels.last().unwrap();
        let mut next: Vec<[u8; 32]> = Vec::with_capacity(prev.len().div_ceil(2));
        let mut i = 0;
        while i + 1 < prev.len() {
            next.push(node_hash(&prev[i], &prev[i + 1]));
            i += 2;
        }
        if i < prev.len() {
            // Odd one out at this level : promote unchanged.
            next.push(prev[i]);
        }
        levels.push(next);
    }

    Ok(MerkleTree { levels })
}

// ─────────────────────────────────────────────────────────────────────
//  MerkleTree — owns all levels, can serve inclusion proofs
// ─────────────────────────────────────────────────────────────────────

/// A fully-materialized Merkle tree.
///
/// `levels[0]` = leaf hashes ; `levels[h]` where `h = depth - 1` is the
/// single-element root level.
///
/// Memory : `2N - 1` × 32 bytes ≈ 64 N bytes.  For an epoch of 1 024
/// traces, that's 64 KiB — fine to keep in memory during sealing.
#[derive(Debug, Clone)]
pub struct MerkleTree {
    levels: Vec<Vec<[u8; 32]>>,
}

impl MerkleTree {
    /// Number of leaves in the tree.
    pub fn leaf_count(&self) -> usize {
        self.levels[0].len()
    }

    /// Tree depth (root level index + 1).  A single-leaf tree has depth 1.
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// The root hash.
    pub fn root(&self) -> &[u8; 32] {
        // Last level always has exactly one element (loop invariant of build_tree).
        &self.levels[self.levels.len() - 1][0]
    }

    /// Get the leaf hash at `index` (already domain-prefixed).
    pub fn leaf(&self, index: usize) -> Option<&[u8; 32]> {
        self.levels[0].get(index)
    }

    /// Build an inclusion proof for the leaf at `index`.
    ///
    /// The returned [`InclusionProof`] can be verified independently
    /// by [`verify_inclusion`] in O(log N) without needing the full tree.
    ///
    /// # Errors
    ///
    /// - [`MerkleError::IndexOutOfBounds`] if `index >= leaf_count()`.
    pub fn inclusion_proof(&self, index: usize) -> Result<InclusionProof, MerkleError> {
        let n = self.leaf_count();
        if index >= n {
            return Err(MerkleError::IndexOutOfBounds { index, len: n });
        }

        let leaf_hash = self.levels[0][index];
        let mut siblings: Vec<[u8; 32]> = Vec::with_capacity(self.depth() - 1);
        let mut idx = index;

        for level in &self.levels[..self.depth() - 1] {
            let sibling_idx = if idx % 2 == 0 { idx + 1 } else { idx - 1 };
            if sibling_idx < level.len() {
                siblings.push(level[sibling_idx]);
            }
            // else : we're the odd promoted node at this level —
            // no sibling, no entry in the proof at this level.
            idx /= 2;
        }

        Ok(InclusionProof {
            leaf_index: index,
            tree_size: n,
            leaf_hash,
            siblings,
            root: *self.root(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  InclusionProof
// ─────────────────────────────────────────────────────────────────────

/// Proof that a specific leaf is included in a Merkle tree.
///
/// Sized `O(log N)` × 32 bytes = ~ 320 bytes for N = 1 M leaves.
///
/// **Verification** : independent of the source tree — given the proof
/// alone, [`verify_inclusion`] can confirm the leaf was committed to
/// the root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InclusionProof {
    /// 0-based index of the leaf in the tree (left-to-right at level 0).
    pub leaf_index: usize,
    /// Total number of leaves at the time the tree was built.  Needed
    /// to detect odd-promotion at each level during verification.
    pub tree_size: usize,
    /// Domain-prefixed hash of the leaf (i.e. the `leaf_hash`, not
    /// the original payload bytes).  Verifier compares against this
    /// directly ; the original payload is checked at a higher layer
    /// (e.g. equals `ChainEntry::payload_hash`).
    pub leaf_hash: [u8; 32],
    /// Sibling hashes bottom-up.  Length ≤ ceil(log2(tree_size)).
    /// Missing entries (when we hit an odd-promotion level) are simply
    /// absent from the vec — the verifier mirrors the build_tree
    /// pairing logic to know when to skip.
    pub siblings: Vec<[u8; 32]>,
    /// Expected root.  Auditor compares against the externally-attested
    /// `EpochRoot::root_hash` to confirm authenticity.
    pub root: [u8; 32],
}

/// Verify that an [`InclusionProof`] correctly attests its leaf is in
/// the committed root.
///
/// Algorithm : starting from `leaf_hash`, mirror the build_tree pairing
/// at each level (consuming a sibling from `siblings` when paired,
/// promoting unchanged when odd).  At the end, compare to `proof.root`.
///
/// # Errors
///
/// - [`MerkleError::ProofMismatch`] if the recomputed root differs.
pub fn verify_inclusion(proof: &InclusionProof) -> Result<(), MerkleError> {
    let mut computed = proof.leaf_hash;
    let mut level_size = proof.tree_size;
    let mut idx = proof.leaf_index;
    let mut sib_cursor = 0;

    while level_size > 1 {
        let is_last_odd = idx == level_size - 1 && level_size % 2 == 1;
        if !is_last_odd {
            // Pair with sibling.
            let sibling = proof
                .siblings
                .get(sib_cursor)
                .ok_or(MerkleError::ProofMismatch)?;
            sib_cursor += 1;
            computed = if idx % 2 == 0 {
                node_hash(&computed, sibling)
            } else {
                node_hash(sibling, &computed)
            };
        }
        // Else : we're the odd one out at this level, promoted unchanged.
        idx /= 2;
        // Next level size : ceil(level_size / 2)
        level_size = level_size.div_ceil(2);
    }

    // We must have consumed exactly all siblings.
    if sib_cursor != proof.siblings.len() {
        return Err(MerkleError::ProofMismatch);
    }
    if computed != proof.root {
        return Err(MerkleError::ProofMismatch);
    }
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(i: u8) -> Vec<u8> {
        vec![i; 32]
    }

    // ─────────────────────────────────────────────────────────────────
    //  Hashing helpers
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn leaf_and_node_use_different_domain_bytes() {
        // Same input under different domains MUST hash differently —
        // this is the property RFC 6962 relies on.  We hash a 64-byte
        // payload as a single leaf AND as the concatenation of two
        // 32-byte halves treated as siblings.  Domain separation
        // guarantees the two results are uncorrelated.
        let p = [0xaa_u8; 64];
        let h_leaf = leaf_hash(&p);
        let mut l32 = [0u8; 32];
        let mut r32 = [0u8; 32];
        l32.copy_from_slice(&p[..32]);
        r32.copy_from_slice(&p[32..]);
        let h_node = node_hash(&l32, &r32);
        assert_ne!(h_leaf, h_node);
    }

    #[test]
    fn leaf_hash_deterministic() {
        let a = leaf_hash(b"x");
        let b = leaf_hash(b"x");
        assert_eq!(a, b);
    }

    #[test]
    fn node_hash_order_matters() {
        let l = [1u8; 32];
        let r = [2u8; 32];
        assert_ne!(node_hash(&l, &r), node_hash(&r, &l));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Build root / tree
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn empty_tree_is_error() {
        let err = build_root(&[]).unwrap_err();
        assert_eq!(err, MerkleError::EmptyTree);
    }

    #[test]
    fn single_leaf_root_equals_leaf_hash() {
        let leaves = vec![payload(1)];
        let root = build_root(&leaves).unwrap();
        assert_eq!(root, leaf_hash(&leaves[0]));
    }

    #[test]
    fn two_leaves_root_formula() {
        let leaves = vec![payload(1), payload(2)];
        let l0 = leaf_hash(&leaves[0]);
        let l1 = leaf_hash(&leaves[1]);
        let expected = node_hash(&l0, &l1);
        let root = build_root(&leaves).unwrap();
        assert_eq!(root, expected);
    }

    #[test]
    fn three_leaves_promotes_third_unchanged_at_first_level() {
        // Per RFC 6962 : for n=3, level 1 has [H(l0,l1), l2-leaf-hash-promoted]
        // level 2 = root = H(H(l0,l1), leaf_hash(l2))
        let leaves = vec![payload(1), payload(2), payload(3)];
        let l0 = leaf_hash(&leaves[0]);
        let l1 = leaf_hash(&leaves[1]);
        let l2 = leaf_hash(&leaves[2]);
        let lvl1_pair = node_hash(&l0, &l1);
        // l2 is promoted unchanged → root = H(lvl1_pair, l2)
        let expected = node_hash(&lvl1_pair, &l2);
        assert_eq!(build_root(&leaves).unwrap(), expected);
    }

    #[test]
    fn power_of_two_balanced_tree_depth() {
        // 1, 2, 4, 8 leaves → depth 1, 2, 3, 4.
        for (n, expected_depth) in [(1, 1), (2, 2), (4, 3), (8, 4)] {
            let leaves: Vec<Vec<u8>> = (0..n).map(|i| payload(i as u8)).collect();
            let tree = build_tree(&leaves).unwrap();
            assert_eq!(tree.depth(), expected_depth, "n={n}");
            assert_eq!(tree.leaf_count(), n);
        }
    }

    #[test]
    fn root_depends_on_leaf_order() {
        let a = build_root(&[payload(1), payload(2)]).unwrap();
        let b = build_root(&[payload(2), payload(1)]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn root_depends_on_leaf_content() {
        let a = build_root(&[payload(1)]).unwrap();
        let b = build_root(&[payload(2)]).unwrap();
        assert_ne!(a, b);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Inclusion proof — generation + verification round-trip
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn inclusion_proof_round_trip_for_every_leaf_power_of_two() {
        let leaves: Vec<Vec<u8>> = (0..8).map(|i| payload(i as u8)).collect();
        let tree = build_tree(&leaves).unwrap();
        for i in 0..8 {
            let proof = tree.inclusion_proof(i).unwrap();
            assert_eq!(proof.leaf_index, i);
            assert_eq!(proof.tree_size, 8);
            verify_inclusion(&proof).unwrap();
        }
    }

    #[test]
    fn inclusion_proof_round_trip_for_every_leaf_odd_size() {
        for n in [1, 3, 5, 7, 9, 11, 1023, 1025] {
            let leaves: Vec<Vec<u8>> = (0..n).map(|i| payload((i % 251) as u8)).collect();
            let tree = build_tree(&leaves).unwrap();
            for i in 0..n {
                let proof = tree.inclusion_proof(i).unwrap();
                verify_inclusion(&proof).unwrap_or_else(|e| {
                    panic!("n={n}, i={i}, err={e:?}");
                });
            }
        }
    }

    #[test]
    fn inclusion_proof_index_out_of_bounds() {
        let tree = build_tree(&[payload(1), payload(2)]).unwrap();
        let err = tree.inclusion_proof(99).unwrap_err();
        assert_eq!(err, MerkleError::IndexOutOfBounds { index: 99, len: 2 });
    }

    #[test]
    fn single_leaf_inclusion_proof_has_no_siblings() {
        let tree = build_tree(&[payload(1)]).unwrap();
        let proof = tree.inclusion_proof(0).unwrap();
        assert!(proof.siblings.is_empty());
        assert_eq!(proof.tree_size, 1);
        verify_inclusion(&proof).unwrap();
    }

    // ─────────────────────────────────────────────────────────────────
    //  Tampering detection
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn tampering_leaf_hash_fails_verification() {
        let leaves = vec![payload(1), payload(2), payload(3), payload(4)];
        let tree = build_tree(&leaves).unwrap();
        let mut proof = tree.inclusion_proof(2).unwrap();
        proof.leaf_hash[0] ^= 0xff;
        let err = verify_inclusion(&proof).unwrap_err();
        assert_eq!(err, MerkleError::ProofMismatch);
    }

    #[test]
    fn tampering_root_fails_verification() {
        let leaves = vec![payload(1), payload(2), payload(3), payload(4)];
        let tree = build_tree(&leaves).unwrap();
        let mut proof = tree.inclusion_proof(0).unwrap();
        proof.root[0] ^= 0xff;
        assert_eq!(
            verify_inclusion(&proof).unwrap_err(),
            MerkleError::ProofMismatch
        );
    }

    #[test]
    fn tampering_a_sibling_fails_verification() {
        let leaves = vec![payload(1), payload(2), payload(3), payload(4)];
        let tree = build_tree(&leaves).unwrap();
        let mut proof = tree.inclusion_proof(1).unwrap();
        // Modify any sibling.
        assert!(!proof.siblings.is_empty());
        proof.siblings[0][0] ^= 0xff;
        assert_eq!(
            verify_inclusion(&proof).unwrap_err(),
            MerkleError::ProofMismatch
        );
    }

    #[test]
    fn extra_sibling_in_proof_fails_verification() {
        let leaves = vec![payload(1), payload(2)];
        let tree = build_tree(&leaves).unwrap();
        let mut proof = tree.inclusion_proof(0).unwrap();
        proof.siblings.push([0u8; 32]);
        assert_eq!(
            verify_inclusion(&proof).unwrap_err(),
            MerkleError::ProofMismatch
        );
    }

    #[test]
    fn missing_sibling_in_proof_fails_verification() {
        let leaves = vec![payload(1), payload(2), payload(3), payload(4)];
        let tree = build_tree(&leaves).unwrap();
        let mut proof = tree.inclusion_proof(0).unwrap();
        proof.siblings.pop();
        assert_eq!(
            verify_inclusion(&proof).unwrap_err(),
            MerkleError::ProofMismatch
        );
    }

    #[test]
    fn swapping_leaves_changes_root() {
        let a = build_root(&[payload(1), payload(2), payload(3)]).unwrap();
        let b = build_root(&[payload(2), payload(1), payload(3)]).unwrap();
        assert_ne!(a, b);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Determinism / property-style
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn build_root_is_idempotent() {
        let leaves: Vec<Vec<u8>> = (0..50).map(|i| payload(i as u8)).collect();
        let a = build_root(&leaves).unwrap();
        let b = build_root(&leaves).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn large_tree_one_thousand_leaves_root_stable() {
        let leaves: Vec<Vec<u8>> = (0..1_000).map(|i| payload((i % 251) as u8)).collect();
        let tree = build_tree(&leaves).unwrap();
        assert_eq!(tree.leaf_count(), 1_000);
        // depth = ceil(log2(1000)) + 1 = 11 (10 doubling steps from 1024 → 2 → 1)
        assert!(tree.depth() >= 11 && tree.depth() <= 12);
        // Spot-check : proofs for first, middle, last leaves.
        for i in [0, 499, 999] {
            verify_inclusion(&tree.inclusion_proof(i).unwrap()).unwrap();
        }
    }

    #[test]
    fn proof_size_is_logarithmic() {
        // For n = 1024 leaves we expect ~ 10 siblings ; for n = 1 we expect 0.
        let small = build_tree(&[payload(1)]).unwrap();
        assert_eq!(small.inclusion_proof(0).unwrap().siblings.len(), 0);

        let large_leaves: Vec<Vec<u8>> = (0..1024).map(|i| payload((i % 251) as u8)).collect();
        let large = build_tree(&large_leaves).unwrap();
        let proof = large.inclusion_proof(512).unwrap();
        assert_eq!(proof.siblings.len(), 10);
    }

    #[test]
    fn proof_for_odd_tree_size_works_at_every_index() {
        // n=5 — at level 1 we'll have [H(l0,l1), H(l2,l3), l4-promoted].
        // At level 2 we'll have [H(...,...), l4-still-promoted-but-now-padded-up].
        // Leaf 4 has a 0-sibling proof segment somewhere.
        let leaves: Vec<Vec<u8>> = (0..5).map(|i| payload(i as u8)).collect();
        let tree = build_tree(&leaves).unwrap();
        for i in 0..5 {
            verify_inclusion(&tree.inclusion_proof(i).unwrap()).unwrap();
        }
    }
}
