// SPDX-License-Identifier: UNLICENSED
//! M32 — Reciprocal Rank Fusion + weighted merge.
//!
//! Given two or more **ranked** hit lists (e.g. dense + sparse),
//! produce a single fused ranking.
//!
//! ## RRF formula (Cormack et al., 2009)
//!
//! For each document `x` and each input run `r` :
//!
//! ```text
//!     score_rrf(x) = Σ over runs containing x of  1 / (k + rank_r(x))
//! ```
//!
//! where `rank_r(x)` is the 1-based rank of `x` in run `r`. `k`
//! defaults to 60.
//!
//! ## Weighted merge
//!
//! ```text
//!     score_weighted(x) = α × normalize(score_dense(x)) + (1 - α) × normalize(score_sparse(x))
//! ```

use std::collections::HashMap;

use crate::error::{Result, RetrievalError};

/// Default RRF `k` parameter.
pub const DEFAULT_RRF_K: usize = 60;

/// Fused hit.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FusedHit {
    /// Document identifier.
    pub id: String,
    /// Fused score (RRF or weighted).
    pub score: f32,
}

/// One input run for RRF.
#[derive(Debug, Clone)]
pub struct RrfRun {
    /// Hits in ranked order (best first).
    pub hits: Vec<(String, f32)>,
}

impl RrfRun {
    /// Build from an iterator of `(id, score)`.
    pub fn from_pairs<I: IntoIterator<Item = (String, f32)>>(pairs: I) -> Self {
        Self {
            hits: pairs.into_iter().collect(),
        }
    }
}

/// Reciprocal Rank Fusion across N runs. `k` defaults to
/// [`DEFAULT_RRF_K`] when None.
pub fn rrf_fuse(runs: &[RrfRun], k: Option<usize>, top_k: usize) -> Result<Vec<FusedHit>> {
    if runs.is_empty() {
        return Err(RetrievalError::RrfNoRuns);
    }
    if top_k == 0 {
        return Ok(Vec::new());
    }
    let k = k.unwrap_or(DEFAULT_RRF_K);
    let kf = k as f32;
    // **M32 perf (V6)** : pre-allocate with an upper-bound capacity =
    // Σ hits across runs. The HashMap won't realloc as we insert.
    let total: usize = runs.iter().map(|r| r.hits.len()).sum();
    let mut scores: HashMap<String, f32> = HashMap::with_capacity(total);
    for run in runs {
        for (rank, (id, _score)) in run.hits.iter().enumerate() {
            // 1-based rank.
            let r = (rank + 1) as f32;
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (kf + r);
        }
    }
    let mut out: Vec<FusedHit> = scores
        .into_iter()
        .map(|(id, score)| FusedHit { id, score })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(top_k);
    Ok(out)
}

/// Weighted fusion of two runs. Score normalization is min-max
/// per run before the weighted sum. Documents missing from one run
/// contribute 0 from that side.
pub fn weighted_fuse(
    dense: &[(String, f32)],
    sparse: &[(String, f32)],
    alpha: f32,
    top_k: usize,
) -> Result<Vec<FusedHit>> {
    if top_k == 0 {
        return Ok(Vec::new());
    }
    if !(0.0..=1.0).contains(&alpha) {
        return Err(RetrievalError::Internal {
            reason: format!("alpha must be in [0,1], got {alpha}"),
        });
    }

    // Normalize each side via min-max.
    let mut dense_norm: HashMap<String, f32> = norm_to_map(dense);
    let mut sparse_norm: HashMap<String, f32> = norm_to_map(sparse);

    // **M32 perf (V6)** : pre-allocate the key-union HashSet with
    // upper bound = sum of both runs.
    let mut keys: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(dense_norm.len() + sparse_norm.len());
    for k in dense_norm.keys() {
        keys.insert(k.clone());
    }
    for k in sparse_norm.keys() {
        keys.insert(k.clone());
    }

    let mut out: Vec<FusedHit> = keys
        .into_iter()
        .map(|id| {
            let d = dense_norm.remove(&id).unwrap_or(0.0);
            let s = sparse_norm.remove(&id).unwrap_or(0.0);
            FusedHit {
                id,
                score: alpha * d + (1.0 - alpha) * s,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(top_k);
    Ok(out)
}

fn norm_to_map(hits: &[(String, f32)]) -> HashMap<String, f32> {
    if hits.is_empty() {
        return HashMap::new();
    }
    let mut scores: Vec<f32> = hits.iter().map(|(_, s)| *s).collect();
    crate::scoring::minmax_normalize(&mut scores);
    // **M32 perf (V6)** : pre-allocate with hits.len() exactly.
    let mut out: HashMap<String, f32> = HashMap::with_capacity(hits.len());
    for ((id, _), s) in hits.iter().zip(scores.iter()) {
        out.insert(id.clone(), *s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ids: &[&str]) -> RrfRun {
        RrfRun::from_pairs(
            ids.iter()
                .enumerate()
                .map(|(i, id)| (id.to_string(), 1.0 / (i + 1) as f32)),
        )
    }

    #[test]
    fn rrf_rejects_empty_runs() {
        let e = rrf_fuse(&[], None, 10).unwrap_err();
        assert!(matches!(e, RetrievalError::RrfNoRuns));
    }

    #[test]
    fn rrf_single_run_preserves_order() {
        let runs = vec![run(&["a", "b", "c"])];
        let out = rrf_fuse(&runs, None, 3).unwrap();
        assert_eq!(out[0].id, "a");
        assert_eq!(out[1].id, "b");
        assert_eq!(out[2].id, "c");
    }

    #[test]
    fn rrf_two_runs_boosts_doc_present_in_both() {
        // `b` appears in BOTH runs (top of dense, middle of sparse).
        // `c` and `d` only appear in one run each. RRF should
        // rank `b` first because it has hits in two runs.
        let dense = run(&["b", "c"]);
        let sparse = run(&["b", "d"]);
        let out = rrf_fuse(&[dense, sparse], None, 5).unwrap();
        assert_eq!(out[0].id, "b");
        // c and d each appear in exactly one run at rank 2 ⇒
        // they have equal RRF score.
        let c = out.iter().find(|h| h.id == "c").unwrap().score;
        let d = out.iter().find(|h| h.id == "d").unwrap().score;
        assert!((c - d).abs() < 1e-6);
    }

    #[test]
    fn rrf_symmetric_runs_a_and_c_outrank_b() {
        // Mathematical property : for fixed `rank_a + rank_b`, RRF
        // is convex, so docs at the extremes outrank the middle.
        // dense  = a, b, c
        // sparse = c, b, a
        // a : 1/61 + 1/63 ≈ 0.032266
        // b : 1/62 + 1/62 ≈ 0.032258  ← strictly less
        // c : 1/63 + 1/61 ≈ 0.032266
        let dense = run(&["a", "b", "c"]);
        let sparse = run(&["c", "b", "a"]);
        let out = rrf_fuse(&[dense, sparse], None, 3).unwrap();
        let b = out.iter().find(|h| h.id == "b").unwrap().score;
        let a = out.iter().find(|h| h.id == "a").unwrap().score;
        let c = out.iter().find(|h| h.id == "c").unwrap().score;
        assert!(a > b);
        assert!(c > b);
    }

    #[test]
    fn rrf_top_k_truncates() {
        let dense = run(&["a", "b", "c", "d", "e"]);
        let out = rrf_fuse(&[dense], None, 2).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn rrf_zero_top_k_returns_empty() {
        let dense = run(&["a", "b"]);
        let out = rrf_fuse(&[dense], None, 0).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn weighted_alpha_one_returns_dense_only() {
        let dense = vec![("a".into(), 1.0), ("b".into(), 0.5)];
        let sparse = vec![("a".into(), 0.0), ("c".into(), 10.0)];
        let out = weighted_fuse(&dense, &sparse, 1.0, 10).unwrap();
        // alpha=1 ⇒ only dense matters ⇒ `a` first.
        assert_eq!(out[0].id, "a");
    }

    #[test]
    fn weighted_alpha_zero_returns_sparse_only() {
        let dense = vec![("a".into(), 1.0), ("b".into(), 0.5)];
        let sparse = vec![("c".into(), 1.0), ("a".into(), 0.1)];
        let out = weighted_fuse(&dense, &sparse, 0.0, 10).unwrap();
        assert_eq!(out[0].id, "c");
    }

    #[test]
    fn weighted_rejects_alpha_out_of_range() {
        let e = weighted_fuse(&[], &[], 1.5, 1).unwrap_err();
        assert!(matches!(e, RetrievalError::Internal { .. }));
    }

    #[test]
    fn weighted_zero_top_k_returns_empty() {
        let out = weighted_fuse(
            &[("a".into(), 1.0)],
            &[("a".into(), 1.0)],
            0.5,
            0,
        )
        .unwrap();
        assert!(out.is_empty());
    }
}
