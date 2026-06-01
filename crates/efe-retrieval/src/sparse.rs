// SPDX-License-Identifier: UNLICENSED
//! M32 — Sparse vector representation.
//!
//! A sparse vector is a sorted list of (token_id, weight) pairs.
//! Used by BM25 for scoring queries against the inverted index.

use serde::{Deserialize, Serialize};

/// Sparse vector. `entries` MUST be sorted by `token_id` ascending
/// and contain no duplicates ; constructor enforces both invariants.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SparseVector {
    /// Sorted (token_id, weight) pairs.
    pub entries: Vec<(u32, f32)>,
}

impl SparseVector {
    /// Construct from an unsorted iterator. Drops duplicates by
    /// summing their weights.
    pub fn from_pairs<I: IntoIterator<Item = (u32, f32)>>(pairs: I) -> Self {
        let mut v: Vec<(u32, f32)> = pairs.into_iter().collect();
        v.sort_by_key(|(t, _)| *t);
        let mut merged: Vec<(u32, f32)> = Vec::with_capacity(v.len());
        for (t, w) in v {
            if let Some(last) = merged.last_mut() {
                if last.0 == t {
                    last.1 += w;
                    continue;
                }
            }
            merged.push((t, w));
        }
        Self { entries: merged }
    }

    /// Empty sparse vector.
    #[must_use]
    pub fn empty() -> Self {
        Self { entries: Vec::new() }
    }

    /// Number of non-zero entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// `true` if zero non-zero entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Dot product against another sparse vector. `O(min(a, b))`
    /// via merge-walk.
    #[must_use]
    pub fn dot(&self, other: &Self) -> f32 {
        let mut i = 0;
        let mut j = 0;
        let mut acc = 0.0_f32;
        while i < self.entries.len() && j < other.entries.len() {
            let (ti, wi) = self.entries[i];
            let (tj, wj) = other.entries[j];
            match ti.cmp(&tj) {
                std::cmp::Ordering::Equal => {
                    acc += wi * wj;
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }
        acc
    }

    /// L2 norm `√Σ w²`.
    #[must_use]
    pub fn norm(&self) -> f32 {
        self.entries.iter().map(|(_, w)| w * w).sum::<f32>().sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_pairs_sorts_and_dedups() {
        let v = SparseVector::from_pairs(vec![(3, 1.0), (1, 0.5), (3, 2.0), (2, 0.1)]);
        assert_eq!(
            v.entries,
            vec![(1, 0.5), (2, 0.1), (3, 3.0)] // 1.0 + 2.0 = 3.0
        );
    }

    #[test]
    fn empty_constructor() {
        let v = SparseVector::empty();
        assert!(v.is_empty());
        assert_eq!(v.len(), 0);
    }

    #[test]
    fn dot_matches_manual_compute() {
        let a = SparseVector::from_pairs(vec![(1, 1.0), (3, 2.0)]);
        let b = SparseVector::from_pairs(vec![(1, 0.5), (2, 1.0), (3, 0.5)]);
        // (1, 1.0) × (1, 0.5) + (3, 2.0) × (3, 0.5) = 0.5 + 1.0
        assert_eq!(a.dot(&b), 1.5);
    }

    #[test]
    fn norm_zero_for_empty() {
        let v = SparseVector::empty();
        assert_eq!(v.norm(), 0.0);
    }

    #[test]
    fn norm_basic() {
        let v = SparseVector::from_pairs(vec![(1, 3.0), (2, 4.0)]);
        assert_eq!(v.norm(), 5.0);
    }
}
