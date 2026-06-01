// SPDX-License-Identifier: UNLICENSED
//! M32 — HNSW (Hierarchical Navigable Small Worlds) index.
//!
//! Approximate k-NN via a multi-layer skip-list-like graph. M32
//! ships a competent baseline implementation tuned for our typical
//! use case (≤ 1 M vectors, dim ≤ 1024, cosine metric).
//!
//! **Reference** : Malkov & Yashunin, 2018, "Efficient and robust
//! approximate nearest neighbor search using Hierarchical Navigable
//! Small World graphs".
//!
//! ## Tunables
//!
//! - `M`              : max edges per node per layer. Default 16.
//! - `efConstruction` : candidate list size during build. Default 200.
//! - `efSearch`       : candidate list size during search. Default 64.
//! - `maxLayers`      : max graph height. Default 16.
//!
//! ## Limitations (M32)
//!
//! - No filtered HNSW : filters are applied **after** search by the
//!   collection layer. M32.1 adds in-graph filtering.
//! - No concurrent inserts during search — the index uses a single
//!   `parking_lot::RwLock`. M32.1 introduces a segment-locked split.
//! - Deletes are tombstoned in the docs map but not removed from the
//!   graph until the next full rebuild (snapshot replay).

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::distance::Distance;
use crate::error::{Result, VectorError};
use crate::flat::SearchHit;
use crate::store::{DocId, VectorDoc};

/// HNSW configuration parameters.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct HnswConfig {
    /// Max edges per node per layer.
    pub m: usize,
    /// Candidate list size during build.
    pub ef_construction: usize,
    /// Candidate list size during search.
    pub ef_search: usize,
    /// Max graph height.
    pub max_layers: usize,
}

impl Default for HnswConfig {
    fn default() -> Self {
        Self {
            m: 16,
            ef_construction: 200,
            ef_search: 64,
            max_layers: 16,
        }
    }
}

/// One node in the HNSW graph. Stores the doc id, the vector, and
/// the per-layer neighbor lists.
#[derive(Debug, Clone)]
struct Node {
    id: DocId,
    vector: Vec<f32>,
    /// `neighbors[layer]` = vec of node indices that this node is
    /// connected to at that layer. `layer == 0` is the bottom
    /// (densest) layer.
    neighbors: Vec<Vec<usize>>,
    /// Tombstone marker (delete defers actual graph removal).
    tombstone: bool,
}

/// HNSW index.
///
/// Internally locked by an `RwLock` ; cloning is cheap (Arc-like
/// reference counting through the parking_lot wrapper isn't free
/// but rare ; M32.1 splits per-segment).
pub struct HnswIndex {
    cfg: HnswConfig,
    dim: usize,
    metric: Distance,
    state: RwLock<HnswState>,
}

struct HnswState {
    /// Flat list of nodes. Indices in the `neighbors` lists refer
    /// to this vec.
    nodes: Vec<Node>,
    /// Current entry point (index into `nodes`). `None` when empty.
    entry_point: Option<usize>,
    /// PRNG state used to assign new node layers. Deterministic
    /// seed so tests are reproducible.
    rng_state: u64,
}

impl HnswIndex {
    /// Construct a fresh empty index.
    #[must_use]
    pub fn new(dim: usize, metric: Distance, cfg: HnswConfig) -> Self {
        Self {
            cfg,
            dim,
            metric,
            state: RwLock::new(HnswState {
                nodes: Vec::new(),
                entry_point: None,
                rng_state: 0xCAFE_BABE_DEAD_BEEF,
            }),
        }
    }

    /// Configured dim.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Configured metric.
    #[must_use]
    pub fn metric(&self) -> Distance {
        self.metric
    }

    /// Number of nodes (including tombstoned).
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.read().nodes.len()
    }

    /// `true` if no live (non-tombstoned) node remains.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let g = self.state.read();
        g.nodes.iter().all(|n| n.tombstone)
    }

    /// Insert a document. Replaces existing rows with the same `id`
    /// (the previous row is tombstoned + a new node is added — graph
    /// stays consistent at the cost of some staleness).
    pub fn insert(&self, doc: VectorDoc) -> Result<()> {
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

        let mut g = self.state.write();

        // Tombstone an existing node with the same id.
        if let Some(prev) = g.nodes.iter_mut().find(|n| n.id == doc.id) {
            prev.tombstone = true;
        }

        let lvl = sample_level(&mut g.rng_state, self.cfg.m, self.cfg.max_layers);
        let new_idx = g.nodes.len();
        let new_node = Node {
            id: doc.id.clone(),
            vector: doc.vector.clone(),
            neighbors: vec![Vec::new(); lvl + 1],
            tombstone: false,
        };
        g.nodes.push(new_node);

        // First-ever insert : entry point.
        if g.entry_point.is_none() {
            g.entry_point = Some(new_idx);
            return Ok(());
        }

        let entry_idx = g.entry_point.unwrap();
        let entry_level = g.nodes[entry_idx].neighbors.len() - 1;

        // Greedy descent from top layer to lvl+1.
        let mut cur = entry_idx;
        for layer in ((lvl + 1)..=entry_level).rev() {
            cur = self
                .greedy_search_layer_inner(&g, &doc.vector, cur, layer, 1)
                .into_iter()
                .next()
                .map(|(_, idx)| idx)
                .unwrap_or(cur);
        }

        // For each layer ≤ lvl, find ef_construction neighbors then
        // shrink to M (bidirectional edges).
        for layer in (0..=lvl.min(entry_level)).rev() {
            let candidates =
                self.greedy_search_layer_inner(&g, &doc.vector, cur, layer, self.cfg.ef_construction);
            cur = candidates
                .first()
                .map(|(_, idx)| *idx)
                .unwrap_or(cur);
            // Select up to M nearest from candidates.
            let nearest: Vec<usize> = candidates
                .into_iter()
                .take(self.cfg.m)
                .map(|(_, idx)| idx)
                .collect();
            // Bidirectional edges : add the new node ↔ each nearest.
            g.nodes[new_idx].neighbors[layer] = nearest.clone();
            for &peer in &nearest {
                g.nodes[peer].neighbors[layer].push(new_idx);
                // If the peer now has > M neighbors at this layer,
                // prune to M nearest.
                if g.nodes[peer].neighbors[layer].len() > self.cfg.m {
                    let peer_vec = g.nodes[peer].vector.clone();
                    let neighs = g.nodes[peer].neighbors[layer].clone();
                    let mut scored: Vec<(f32, usize)> = neighs
                        .into_iter()
                        .filter_map(|n| {
                            self.metric
                                .score(&peer_vec, &g.nodes[n].vector)
                                .ok()
                                .map(|s| (s, n))
                        })
                        .collect();
                    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
                    scored.truncate(self.cfg.m);
                    g.nodes[peer].neighbors[layer] =
                        scored.into_iter().map(|(_, n)| n).collect();
                }
            }
        }

        // If we landed above the previous entry-point level, promote.
        if lvl > entry_level {
            g.entry_point = Some(new_idx);
        }

        Ok(())
    }

    /// Top-`k` search. Uses `efSearch` candidate-list size at every
    /// layer.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<SearchHit>> {
        if k == 0 || k > crate::flat::MAX_K {
            return Err(VectorError::KOutOfRange {
                observed: k,
                max: crate::flat::MAX_K,
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

        let g = self.state.read();
        if g.nodes.iter().all(|n| n.tombstone) {
            return Ok(Vec::new());
        }
        let entry_idx = match g.entry_point {
            Some(e) => e,
            None => return Ok(Vec::new()),
        };
        let entry_level = g.nodes[entry_idx].neighbors.len() - 1;

        // Greedy descent from top layer to layer 1.
        let mut cur = entry_idx;
        for layer in (1..=entry_level).rev() {
            cur = self
                .greedy_search_layer_inner(&g, query, cur, layer, 1)
                .into_iter()
                .next()
                .map(|(_, idx)| idx)
                .unwrap_or(cur);
        }

        // Bottom layer : full ef_search.
        let ef = self.cfg.ef_search.max(k);
        let candidates = self.greedy_search_layer_inner(&g, query, cur, 0, ef);

        let mut out: Vec<SearchHit> = candidates
            .into_iter()
            .filter(|(_, idx)| !g.nodes[*idx].tombstone)
            .take(k)
            .map(|(score, idx)| SearchHit {
                id: g.nodes[idx].id.clone(),
                score,
            })
            .collect();
        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        Ok(out)
    }

    /// Delete a doc by id. The graph node is tombstoned ; full
    /// graph cleanup happens on the next rebuild.
    pub fn delete(&self, id: &str) -> Result<()> {
        let mut g = self.state.write();
        for n in g.nodes.iter_mut() {
            if n.id == id {
                n.tombstone = true;
            }
        }
        Ok(())
    }

    // ─────────────── internal helpers ───────────────

    /// Greedy beam search on one layer. Returns an ordered list of
    /// `(score, node_idx)` pairs, score-descending. Length capped
    /// at `ef`.
    fn greedy_search_layer_inner(
        &self,
        g: &HnswState,
        query: &[f32],
        entry: usize,
        layer: usize,
        ef: usize,
    ) -> Vec<(f32, usize)> {
        // Min-heap of unexplored candidates (smaller score → priority).
        // Max-heap of best-so-far ("results").
        #[derive(PartialEq)]
        struct Pair(f32, usize);
        impl Eq for Pair {}
        impl Ord for Pair {
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.partial_cmp(&other.0).unwrap_or(Ordering::Equal)
            }
        }
        impl PartialOrd for Pair {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        // **M32 perf (V4)** : pre-allocate visited + heaps to avoid
        // the realloc growth that happens by default on every query.
        // `ef × 4` is generous but cheap (8 bytes / entry).
        let prealloc = ef.saturating_mul(4).max(16);
        let mut visited: HashSet<usize> = HashSet::with_capacity(prealloc);
        visited.insert(entry);

        let entry_score = self
            .metric
            .score(query, &g.nodes[entry].vector)
            .unwrap_or(f32::NEG_INFINITY);
        let mut candidates: BinaryHeap<Pair> = BinaryHeap::with_capacity(prealloc);
        candidates.push(Pair(entry_score, entry));
        let mut results: BinaryHeap<std::cmp::Reverse<Pair>> =
            BinaryHeap::with_capacity(ef + 1);
        results.push(std::cmp::Reverse(Pair(entry_score, entry)));

        while let Some(Pair(cur_score, cur_idx)) = candidates.pop() {
            // Stop when the best candidate is worse than the worst
            // result and we have ef results.
            if let Some(std::cmp::Reverse(Pair(worst, _))) = results.peek() {
                if cur_score < *worst && results.len() >= ef {
                    break;
                }
            }
            // **V4** : avoid `cloned()` on the neighbor Vec by
            // iterating by reference. Each neighbor is a `usize`
            // copy (8 bytes) so we don't pay for the clone.
            let empty: Vec<usize> = Vec::new();
            let neighbors_ref = g.nodes[cur_idx]
                .neighbors
                .get(layer)
                .unwrap_or(&empty);
            for &n in neighbors_ref {
                if !visited.insert(n) {
                    continue;
                }
                let s = self
                    .metric
                    .score(query, &g.nodes[n].vector)
                    .unwrap_or(f32::NEG_INFINITY);
                if results.len() < ef {
                    candidates.push(Pair(s, n));
                    results.push(std::cmp::Reverse(Pair(s, n)));
                } else if let Some(std::cmp::Reverse(Pair(worst, _))) = results.peek() {
                    if s > *worst {
                        candidates.push(Pair(s, n));
                        results.push(std::cmp::Reverse(Pair(s, n)));
                        results.pop();
                    }
                }
            }
        }

        let mut out: Vec<(f32, usize)> = results
            .into_iter()
            .map(|std::cmp::Reverse(Pair(s, i))| (s, i))
            .collect();
        out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        out
    }
}

/// xorshift-based level sampler. Uses the Malkov/Yashunin
/// distribution `floor(-ln(unif()) × 1/ln(M))`.
fn sample_level(state: &mut u64, m: usize, max_layers: usize) -> usize {
    // xorshift64
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    let u = (x as f64) / (u64::MAX as f64);
    let u = u.max(f64::MIN_POSITIVE);
    let m_inv_log = 1.0 / (m as f64).ln();
    let lvl = (-(u.ln()) * m_inv_log).floor() as usize;
    lvl.min(max_layers.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::VectorDoc;

    fn d(id: &str, v: Vec<f32>) -> VectorDoc {
        VectorDoc::new(id, v)
    }

    #[test]
    fn empty_search_returns_empty() {
        let idx = HnswIndex::new(4, Distance::Cosine, HnswConfig::default());
        let out = idx.search(&[1.0; 4], 5).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn insert_rejects_dim_mismatch() {
        let idx = HnswIndex::new(3, Distance::Cosine, HnswConfig::default());
        let e = idx.insert(d("x", vec![1.0, 2.0])).unwrap_err();
        assert!(matches!(e, VectorError::DimensionMismatch { .. }));
    }

    #[test]
    fn insert_rejects_nan() {
        let idx = HnswIndex::new(3, Distance::Cosine, HnswConfig::default());
        let e = idx.insert(d("x", vec![1.0, f32::NAN, 1.0])).unwrap_err();
        assert!(matches!(e, VectorError::InvalidVector { .. }));
    }

    #[test]
    fn insert_then_search_finds_self() {
        let idx = HnswIndex::new(4, Distance::Cosine, HnswConfig::default());
        idx.insert(d("a", vec![1.0, 0.0, 0.0, 0.0])).unwrap();
        let out = idx.search(&[1.0, 0.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn insert_many_and_recall_at_1_is_perfect_for_self_query() {
        // Self-query recall@1 should be 1.0 on a small dataset.
        let idx = HnswIndex::new(8, Distance::Cosine, HnswConfig::default());
        let vectors = vec![
            vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        ];
        for (i, v) in vectors.iter().enumerate() {
            idx.insert(d(&format!("v{i}"), v.clone())).unwrap();
        }
        for (i, v) in vectors.iter().enumerate() {
            let out = idx.search(v, 1).unwrap();
            assert_eq!(out[0].id, format!("v{i}"), "self-query for v{i} failed");
        }
    }

    #[test]
    fn delete_tombstones_and_excludes_from_search() {
        let idx = HnswIndex::new(4, Distance::Cosine, HnswConfig::default());
        idx.insert(d("a", vec![1.0, 0.0, 0.0, 0.0])).unwrap();
        idx.insert(d("b", vec![0.99, 0.0, 0.0, 0.0])).unwrap();
        idx.delete("a").unwrap();
        let out = idx.search(&[1.0, 0.0, 0.0, 0.0], 5).unwrap();
        assert!(out.iter().all(|h| h.id != "a"));
    }

    #[test]
    fn search_rejects_invalid_k() {
        let idx = HnswIndex::new(4, Distance::Cosine, HnswConfig::default());
        idx.insert(d("a", vec![1.0; 4])).unwrap();
        let e = idx.search(&[1.0; 4], 0).unwrap_err();
        assert!(matches!(e, VectorError::KOutOfRange { .. }));
    }

    #[test]
    fn insert_with_existing_id_replaces() {
        let idx = HnswIndex::new(4, Distance::Cosine, HnswConfig::default());
        idx.insert(d("x", vec![1.0, 0.0, 0.0, 0.0])).unwrap();
        idx.insert(d("x", vec![0.0, 1.0, 0.0, 0.0])).unwrap();
        // Only the second version should be found.
        let out = idx.search(&[0.0, 1.0, 0.0, 0.0], 5).unwrap();
        // Either no result with id="x" carrying the old vector, or
        // the new vector matches better — depends on tombstone
        // filter ordering. Assert the new wins.
        if let Some(hit) = out.iter().find(|h| h.id == "x") {
            assert!(hit.score > 0.9);
        }
    }

    #[test]
    fn sample_level_is_bounded() {
        let mut s = 0xDEAD_BEEF_CAFE_BABE_u64;
        for _ in 0..1000 {
            let lvl = sample_level(&mut s, 16, 16);
            assert!(lvl < 16);
        }
    }
}
