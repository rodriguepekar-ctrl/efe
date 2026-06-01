// SPDX-License-Identifier: UNLICENSED
//! M33 — Score fusion for GraphRAG.
//!
//! Combines dense vector scores with graph-distance scores via :
//!
//! ```text
//!     final = α × dense_norm + (1-α) × graph_score
//!     where graph_score = exp(-rate × hop_distance)
//! ```
//!
//! Default `α = 0.7`, `rate = 0.5`. The fusion produces a flat
//! ranked list deduplicated by entity id.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{GraphRagError, Result};
use crate::scoring::{graph_decay, normalize_dense_scores, DEFAULT_DECAY_RATE};

/// Default α (weight on dense vs graph).
pub const DEFAULT_ALPHA: f32 = 0.7;

/// Default decay rate.
pub const DEFAULT_DECAY: f32 = DEFAULT_DECAY_RATE;

/// One fused hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphRagHit {
    /// Entity / document identifier.
    pub id: String,
    /// Final fused score (higher = better).
    pub score: f32,
    /// Hop distance from the closest dense seed.
    pub hop_distance: usize,
    /// Whether this hit was produced by the dense run.
    pub dense_origin: bool,
    /// Whether this hit was discovered via graph expansion.
    pub graph_origin: bool,
}

/// Fuse dense + graph results.
///
/// - `dense_pairs` : `(id, score)` from the vector search.
/// - `graph_pairs` : `(id, hop_distance)` from BFS.
/// - `alpha`       : weight on dense (0 = graph only, 1 = dense only).
/// - `decay_rate`  : exponential decay applied to hop distance.
/// - `top_k`       : truncation cap.
pub fn fuse_scores(
    dense_pairs: &[(String, f32)],
    graph_pairs: &[(String, usize)],
    alpha: f32,
    decay_rate: f32,
    top_k: usize,
) -> Result<Vec<GraphRagHit>> {
    if !(0.0..=1.0).contains(&alpha) {
        return Err(GraphRagError::InvalidAlpha(alpha));
    }
    if top_k == 0 {
        return Ok(Vec::new());
    }

    // 1. Normalize dense scores.
    let mut dense_norm: HashMap<String, f32> = HashMap::new();
    if !dense_pairs.is_empty() {
        let mut scores: Vec<f32> = dense_pairs.iter().map(|(_, s)| *s).collect();
        normalize_dense_scores(&mut scores);
        for (i, (id, _)) in dense_pairs.iter().enumerate() {
            dense_norm.insert(id.clone(), scores[i]);
        }
    }

    // 2. Build graph scores.
    let mut graph_score: HashMap<String, f32> = HashMap::new();
    let mut hop_of: HashMap<String, usize> = HashMap::new();
    for (id, hops) in graph_pairs {
        let s = graph_decay(*hops, decay_rate);
        // Keep the BEST (smallest hop) score if duplicate.
        let prev = graph_score.get(id).copied().unwrap_or(f32::NEG_INFINITY);
        if s > prev {
            graph_score.insert(id.clone(), s);
            hop_of.insert(id.clone(), *hops);
        }
    }

    // 3. Union the keys + compute final score.
    let mut keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    keys.extend(dense_norm.keys().cloned());
    keys.extend(graph_score.keys().cloned());

    let mut hits: Vec<GraphRagHit> = keys
        .into_iter()
        .map(|id| {
            let d = dense_norm.remove(&id).unwrap_or(0.0);
            let g = graph_score.remove(&id).unwrap_or(0.0);
            let hop = hop_of.get(&id).copied().unwrap_or(0);
            let dense_origin = d > 0.0;
            let graph_origin = g > 0.0;
            GraphRagHit {
                id,
                score: alpha * d + (1.0 - alpha) * g,
                hop_distance: hop,
                dense_origin,
                graph_origin,
            }
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(top_k);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuse_rejects_invalid_alpha() {
        let e = fuse_scores(&[], &[], 1.5, 0.5, 10).unwrap_err();
        assert!(matches!(e, GraphRagError::InvalidAlpha(_)));
    }

    #[test]
    fn fuse_zero_top_k_returns_empty() {
        let out = fuse_scores(
            &[("a".into(), 1.0)],
            &[("a".into(), 0)],
            0.7,
            0.5,
            0,
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn fuse_alpha_one_returns_dense_only() {
        // alpha = 1 ⇒ graph contribution = 0.
        let out = fuse_scores(
            &[("a".into(), 10.0), ("b".into(), 5.0)],
            &[("z".into(), 0)],
            1.0,
            0.5,
            10,
        )
        .unwrap();
        // First hit should be `a` (highest dense), `z` should still
        // appear because the graph run found it.
        assert_eq!(out[0].id, "a");
        // `z` has score 0 at alpha=1.
        let z = out.iter().find(|h| h.id == "z").unwrap();
        assert!(z.score < 1e-6);
    }

    #[test]
    fn fuse_alpha_zero_returns_graph_only() {
        let out = fuse_scores(
            &[("a".into(), 10.0)],
            &[("a".into(), 0), ("b".into(), 1), ("c".into(), 2)],
            0.0,
            0.5,
            10,
        )
        .unwrap();
        // `a` at hop 0 ⇒ decay = 1.0 ⇒ highest graph score.
        assert_eq!(out[0].id, "a");
        // `c` at hop 2 has smaller score.
        let a = out.iter().find(|h| h.id == "a").unwrap();
        let c = out.iter().find(|h| h.id == "c").unwrap();
        assert!(a.score > c.score);
    }

    #[test]
    fn fuse_combines_dense_and_graph() {
        let out = fuse_scores(
            &[("a".into(), 1.0), ("b".into(), 0.0)],
            &[("a".into(), 0), ("c".into(), 1)],
            0.7,
            0.5,
            10,
        )
        .unwrap();
        // `a` is in both → highest score.
        assert_eq!(out[0].id, "a");
        let a = out.iter().find(|h| h.id == "a").unwrap();
        assert!(a.dense_origin);
        assert!(a.graph_origin);
    }

    #[test]
    fn fuse_truncates_at_top_k() {
        let mut dense = vec![];
        for i in 0..20 {
            dense.push((format!("d{i}"), i as f32));
        }
        let out = fuse_scores(&dense, &[], 0.7, 0.5, 5).unwrap();
        assert_eq!(out.len(), 5);
    }

    #[test]
    fn fuse_deduplicates_graph_hits_by_min_hop() {
        // `a` reachable at hop 1 AND hop 2 ⇒ keep hop 1.
        let out = fuse_scores(
            &[],
            &[("a".into(), 2), ("a".into(), 1)],
            0.0,
            0.5,
            10,
        )
        .unwrap();
        assert_eq!(out[0].id, "a");
        assert_eq!(out[0].hop_distance, 1);
    }
}
