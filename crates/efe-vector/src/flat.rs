// SPDX-License-Identifier: UNLICENSED
//! M32 — Flat (brute-force) k-NN search.
//!
//! `O(N × dim)` per query — slow for large collections but :
//!
//! 1. Trivially correct → serves as the golden reference for HNSW
//!    recall benchmarks.
//! 2. Faster than HNSW on small collections (< 10 k docs).
//! 3. No build time ; inserts are O(1) (push into a Vec).
//!
//! The flat index is the **default** under
//! `BRUTE_FORCE_MAX_VECTORS`. Above that, callers should swap to
//! HNSW via `hnsw::HnswIndex`.

use std::cmp::Ordering;

use crate::distance::Distance;
use crate::error::{Result, VectorError};
use crate::store::{DocId, VectorDoc};

/// Default upper bound for flat search ; above this the build cost
/// of HNSW is amortized.
pub const BRUTE_FORCE_MAX_VECTORS: usize = 10_000;

/// Default upper bound on `k` (top-k cap). Prevents O(N) memory
/// blow-up on adversarial requests.
pub const MAX_K: usize = 10_000;

/// One search hit : doc id + similarity score.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    /// Document identifier.
    pub id: DocId,
    /// Similarity score (higher = closer).
    pub score: f32,
}

impl Eq for SearchHit {}

/// Flat in-memory index.
#[derive(Debug, Clone, Default)]
pub struct FlatIndex {
    /// Configured dim (set on first insert).
    dim: usize,
    /// Stored docs in insertion order.
    docs: Vec<VectorDoc>,
    /// Distance metric.
    metric: Distance,
}

impl FlatIndex {
    /// Construct a new empty flat index for the given dim + metric.
    #[must_use]
    pub fn new(dim: usize, metric: Distance) -> Self {
        Self {
            dim,
            docs: Vec::new(),
            metric,
        }
    }

    /// Returns the configured distance metric.
    #[must_use]
    pub fn metric(&self) -> Distance {
        self.metric
    }

    /// Returns the configured dim.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Number of documents in the index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.docs.len()
    }

    /// `true` if no documents are indexed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Insert a doc. Replaces existing rows with the same `id`.
    /// Returns the new cardinality.
    pub fn insert(&mut self, doc: VectorDoc) -> Result<usize> {
        if doc.vector.len() != self.dim {
            return Err(VectorError::DimensionMismatch {
                observed: doc.vector.len(),
                expected: self.dim,
            });
        }
        if doc.vector.iter().any(|x| !x.is_finite()) {
            return Err(VectorError::InvalidVector {
                reason: "vector contains NaN or Infinity".into(),
            });
        }
        // Linear scan replace (acceptable below BRUTE_FORCE_MAX_VECTORS).
        if let Some(pos) = self.docs.iter().position(|d| d.id == doc.id) {
            self.docs[pos] = doc;
        } else {
            self.docs.push(doc);
        }
        Ok(self.docs.len())
    }

    /// Delete a doc by id. Idempotent.
    pub fn delete(&mut self, id: &str) -> Result<()> {
        if let Some(pos) = self.docs.iter().position(|d| d.id == id) {
            self.docs.swap_remove(pos);
        }
        Ok(())
    }

    /// Top-`k` search. Returns hits sorted by score (descending).
    ///
    /// Uses a partial-sort min-heap of size k for `O(N log k)`
    /// complexity. For very small k (< 100) a full sort would also
    /// work but the heap is cleaner.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchHit>> {
        if k == 0 || k > MAX_K {
            return Err(VectorError::KOutOfRange {
                observed: k,
                max: MAX_K,
            });
        }
        if query.len() != self.dim {
            return Err(VectorError::DimensionMismatch {
                observed: query.len(),
                expected: self.dim,
            });
        }
        if query.iter().any(|x| !x.is_finite()) {
            return Err(VectorError::InvalidVector {
                reason: "query contains NaN or Infinity".into(),
            });
        }
        if self.docs.is_empty() {
            return Ok(Vec::new());
        }

        // Min-heap by score so we keep the top-k largest scores.
        // `BinaryHeap` is a max-heap so we wrap in `Reverse` to
        // emulate min-heap behaviour.
        use std::collections::BinaryHeap;
        use std::cmp::Reverse;

        #[derive(PartialEq)]
        struct Entry(f32, usize);
        impl Eq for Entry {}
        impl Ord for Entry {
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.partial_cmp(&other.0).unwrap_or(Ordering::Equal)
            }
        }
        impl PartialOrd for Entry {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        let mut heap: BinaryHeap<Reverse<Entry>> = BinaryHeap::with_capacity(k + 1);
        for (idx, doc) in self.docs.iter().enumerate() {
            let s = self.metric.score(query, &doc.vector)?;
            if heap.len() < k {
                heap.push(Reverse(Entry(s, idx)));
            } else if let Some(Reverse(min)) = heap.peek() {
                if s > min.0 {
                    heap.pop();
                    heap.push(Reverse(Entry(s, idx)));
                }
            }
        }

        // Drain the heap and sort descending.
        let mut out: Vec<SearchHit> = heap
            .into_iter()
            .map(|Reverse(Entry(score, idx))| SearchHit {
                id: self.docs[idx].id.clone(),
                score,
            })
            .collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        Ok(out)
    }

    /// Borrow the underlying doc slice. Useful for HNSW bootstrap
    /// + tests.
    #[must_use]
    pub fn docs(&self) -> &[VectorDoc] {
        &self.docs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(id: &str, v: Vec<f32>) -> VectorDoc {
        VectorDoc::new(id, v)
    }

    #[test]
    fn empty_index_search_returns_empty() {
        let idx = FlatIndex::new(4, Distance::Cosine);
        let out = idx.search(&[1.0, 1.0, 1.0, 1.0], 5).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn insert_rejects_dim_mismatch() {
        let mut idx = FlatIndex::new(3, Distance::Cosine);
        let e = idx.insert(d("x", vec![1.0, 2.0])).unwrap_err();
        assert!(matches!(e, VectorError::DimensionMismatch { .. }));
    }

    #[test]
    fn insert_rejects_nan() {
        let mut idx = FlatIndex::new(3, Distance::Cosine);
        let e = idx.insert(d("x", vec![1.0, f32::NAN, 1.0])).unwrap_err();
        assert!(matches!(e, VectorError::InvalidVector { .. }));
    }

    #[test]
    fn insert_replaces_existing_id() {
        let mut idx = FlatIndex::new(3, Distance::Cosine);
        idx.insert(d("x", vec![1.0, 0.0, 0.0])).unwrap();
        idx.insert(d("x", vec![0.0, 1.0, 0.0])).unwrap();
        assert_eq!(idx.len(), 1);
    }

    #[test]
    fn delete_is_idempotent() {
        let mut idx = FlatIndex::new(3, Distance::Cosine);
        idx.delete("missing").unwrap();
        idx.insert(d("x", vec![1.0, 0.0, 0.0])).unwrap();
        idx.delete("x").unwrap();
        idx.delete("x").unwrap();
        assert_eq!(idx.len(), 0);
    }

    #[test]
    fn search_top_k_returns_descending_score() {
        let mut idx = FlatIndex::new(3, Distance::Cosine);
        // M32 audit fix : original test used `[1,0,0]` and `[0.99,0,0]`
        // which are **parallel** ⇒ cosine = 1.0 for both. We now
        // use vectors with distinct angles so the ordering is
        // deterministic.
        idx.insert(d("identical", vec![1.0, 0.0, 0.0])).unwrap();
        idx.insert(d("near",      vec![1.0, 0.2, 0.0])).unwrap(); // ~ 11° off
        idx.insert(d("orth",      vec![0.0, 1.0, 0.0])).unwrap(); // 90° off
        let q = vec![1.0, 0.0, 0.0];
        let out = idx.search(&q, 3).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].id, "identical");
        assert_eq!(out[1].id, "near");
        assert_eq!(out[2].id, "orth");
        assert!(out[0].score > out[1].score);
        assert!(out[1].score > out[2].score);
    }

    #[test]
    fn search_caps_at_k() {
        let mut idx = FlatIndex::new(2, Distance::Cosine);
        for i in 0..50 {
            idx.insert(d(&format!("d{i}"), vec![1.0, i as f32])).unwrap();
        }
        let q = vec![1.0, 0.5];
        let out = idx.search(&q, 5).unwrap();
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn search_rejects_k_zero() {
        let mut idx = FlatIndex::new(2, Distance::Cosine);
        idx.insert(d("x", vec![1.0, 0.0])).unwrap();
        let e = idx.search(&[1.0, 0.0], 0).unwrap_err();
        assert!(matches!(e, VectorError::KOutOfRange { .. }));
    }

    #[test]
    fn search_rejects_k_too_large() {
        let idx = FlatIndex::new(2, Distance::Cosine);
        let e = idx.search(&[1.0, 0.0], MAX_K + 1).unwrap_err();
        assert!(matches!(e, VectorError::KOutOfRange { .. }));
    }

    #[test]
    fn l2_metric_ranks_close_first() {
        let mut idx = FlatIndex::new(2, Distance::L2);
        idx.insert(d("close", vec![1.0, 0.0])).unwrap();
        idx.insert(d("far",  vec![10.0, 10.0])).unwrap();
        let out = idx.search(&[1.1, 0.0], 2).unwrap();
        assert_eq!(out[0].id, "close");
        // L2 score is negated, so close has higher score.
        assert!(out[0].score > out[1].score);
    }

    #[test]
    fn dot_metric_works() {
        let mut idx = FlatIndex::new(2, Distance::Dot);
        idx.insert(d("a", vec![1.0, 0.0])).unwrap();
        idx.insert(d("b", vec![0.5, 0.5])).unwrap();
        let q = vec![2.0, 0.0];
        let out = idx.search(&q, 2).unwrap();
        assert_eq!(out[0].id, "a"); // 2.0 dot product
        assert!(out[0].score > out[1].score);
    }
}
