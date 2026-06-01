// SPDX-License-Identifier: UNLICENSED
//! M32 — Hybrid dense + sparse search orchestrator.
//!
//! Combines a dense vector search ([`efe_vector::Collection`]) with
//! a sparse BM25 lookup ([`Bm25Index`]) and fuses results via RRF or
//! weighted merge.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use efe_vector::{Collection, SearchHit};

use crate::bm25::{Bm25Hit, Bm25Index};
use crate::error::{Result, RetrievalError};
use crate::rrf::{rrf_fuse, weighted_fuse, FusedHit, RrfRun, DEFAULT_RRF_K};

/// Hybrid retrieval mode discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HybridMode {
    /// Dense vector only.
    Dense,
    /// Sparse BM25 only.
    Sparse,
    /// Hybrid via RRF.
    HybridRrf,
    /// Hybrid via weighted mean.
    HybridWeighted,
}

/// Hybrid retrieval configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridConfig {
    /// Retrieval mode.
    pub mode: HybridMode,
    /// Candidates retrieved from each side before fusion. Defaults
    /// to `top_k * 4` ; bounded by 1000.
    #[serde(default)]
    pub candidates_per_side: Option<usize>,
    /// RRF k parameter (when mode = HybridRrf).
    #[serde(default = "default_rrf_k")]
    pub rrf_k: usize,
    /// Alpha for weighted merge (when mode = HybridWeighted).
    /// `alpha` is the dense weight ; `1 - alpha` is the sparse
    /// weight. Default 0.5.
    #[serde(default = "default_alpha")]
    pub alpha: f32,
}

fn default_rrf_k() -> usize {
    DEFAULT_RRF_K
}

fn default_alpha() -> f32 {
    0.5
}

impl Default for HybridConfig {
    fn default() -> Self {
        Self {
            mode: HybridMode::HybridRrf,
            candidates_per_side: None,
            rrf_k: DEFAULT_RRF_K,
            alpha: 0.5,
        }
    }
}

/// Hybrid searcher. Cheap to clone (Arcs internally).
#[derive(Clone)]
pub struct HybridSearcher {
    dense: Arc<Collection>,
    sparse: Arc<Bm25Index>,
}

impl HybridSearcher {
    /// Construct from a dense collection + sparse index.
    pub fn new(dense: Arc<Collection>, sparse: Arc<Bm25Index>) -> Self {
        Self { dense, sparse }
    }

    /// Run the search.
    ///
    /// - `query_vector` is required for [`HybridMode::Dense`],
    ///   [`HybridMode::HybridRrf`], and [`HybridMode::HybridWeighted`].
    /// - `query_text` is required for [`HybridMode::Sparse`],
    ///   [`HybridMode::HybridRrf`], and [`HybridMode::HybridWeighted`].
    /// - The function validates the appropriate inputs are
    ///   non-empty.
    pub async fn search(
        &self,
        query_vector: Option<&[f32]>,
        query_text: Option<&str>,
        top_k: usize,
        cfg: &HybridConfig,
    ) -> Result<Vec<FusedHit>> {
        let candidates = cfg
            .candidates_per_side
            .unwrap_or(top_k.saturating_mul(4))
            .max(top_k)
            .min(1000);

        match cfg.mode {
            HybridMode::Dense => {
                let qv = query_vector.ok_or(RetrievalError::Internal {
                    reason: "dense mode requires query_vector".into(),
                })?;
                let dense = self.dense.search(qv, top_k)?;
                Ok(dense.into_iter().map(hit_to_fused).collect())
            }
            HybridMode::Sparse => {
                let qt = query_text.ok_or(RetrievalError::Internal {
                    reason: "sparse mode requires query_text".into(),
                })?;
                let sparse = self.sparse.search(qt, top_k)?;
                Ok(sparse.into_iter().map(bm25_to_fused).collect())
            }
            HybridMode::HybridRrf => {
                let qv = query_vector.ok_or(RetrievalError::Internal {
                    reason: "hybrid_rrf requires query_vector".into(),
                })?;
                let qt = query_text.ok_or(RetrievalError::Internal {
                    reason: "hybrid_rrf requires query_text".into(),
                })?;
                let dense_hits = self.dense.search(qv, candidates)?;
                let sparse_hits = self.sparse.search(qt, candidates)?;
                let runs = vec![
                    RrfRun::from_pairs(
                        dense_hits.into_iter().map(|h| (h.id, h.score)),
                    ),
                    RrfRun::from_pairs(
                        sparse_hits.into_iter().map(|h| (h.id, h.score)),
                    ),
                ];
                rrf_fuse(&runs, Some(cfg.rrf_k), top_k)
            }
            HybridMode::HybridWeighted => {
                let qv = query_vector.ok_or(RetrievalError::Internal {
                    reason: "hybrid_weighted requires query_vector".into(),
                })?;
                let qt = query_text.ok_or(RetrievalError::Internal {
                    reason: "hybrid_weighted requires query_text".into(),
                })?;
                let dense_hits = self.dense.search(qv, candidates)?;
                let sparse_hits = self.sparse.search(qt, candidates)?;
                let dense_pairs: Vec<(String, f32)> = dense_hits
                    .into_iter()
                    .map(|h| (h.id, h.score))
                    .collect();
                let sparse_pairs: Vec<(String, f32)> = sparse_hits
                    .into_iter()
                    .map(|h| (h.id, h.score))
                    .collect();
                weighted_fuse(&dense_pairs, &sparse_pairs, cfg.alpha, top_k)
            }
        }
    }
}

fn hit_to_fused(h: SearchHit) -> FusedHit {
    FusedHit {
        id: h.id,
        score: h.score,
    }
}

fn bm25_to_fused(h: Bm25Hit) -> FusedHit {
    FusedHit {
        id: h.id,
        score: h.score,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_vector::{CollectionConfig, Distance, HnswConfig, InMemoryStore, VectorDoc};

    fn coll() -> Arc<Collection> {
        let cfg = CollectionConfig {
            tenant_id: "acme".into(),
            name: "test".into(),
            embedding_model_name: "efe-classifier-768".into(),
            dim: 3,
            metric: Distance::Cosine,
            hnsw_cfg: HnswConfig::default(),
            max_docs: 1000,
            force_hnsw: false,
        };
        let c = Collection::new(cfg, Arc::new(InMemoryStore::new())).unwrap();
        Arc::new(c)
    }

    #[tokio::test]
    async fn dense_only_uses_vector_index() {
        let dense = coll();
        dense.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        let sparse = Arc::new(Bm25Index::default());
        let s = HybridSearcher::new(dense, sparse);
        let out = s
            .search(
                Some(&[1.0, 0.0, 0.0]),
                None,
                5,
                &HybridConfig {
                    mode: HybridMode::Dense,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(out[0].id, "a");
    }

    #[tokio::test]
    async fn sparse_only_uses_bm25() {
        let dense = coll();
        let sparse = Arc::new(Bm25Index::default());
        sparse.upsert("a", "the quick brown fox").unwrap();
        sparse.upsert("b", "the lazy dog").unwrap();
        let s = HybridSearcher::new(dense, sparse);
        let out = s
            .search(
                None,
                Some("fox"),
                5,
                &HybridConfig {
                    mode: HybridMode::Sparse,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(out[0].id, "a");
    }

    #[tokio::test]
    async fn dense_mode_requires_vector() {
        let dense = coll();
        let sparse = Arc::new(Bm25Index::default());
        let s = HybridSearcher::new(dense, sparse);
        let e = s
            .search(
                None,
                None,
                5,
                &HybridConfig {
                    mode: HybridMode::Dense,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(e, RetrievalError::Internal { .. }));
    }

    #[tokio::test]
    async fn hybrid_rrf_uses_both_sides() {
        let dense = coll();
        dense.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        dense.upsert(VectorDoc::new("b", vec![0.0, 1.0, 0.0])).await.unwrap();
        dense.upsert(VectorDoc::new("c", vec![0.0, 0.0, 1.0])).await.unwrap();
        let sparse = Arc::new(Bm25Index::default());
        // M32 audit fix : `a` has more "login" tokens than `b`
        // so the BM25 ranking is deterministically a > b.
        sparse.upsert("a", "login login login").unwrap();
        sparse.upsert("b", "login").unwrap();
        sparse.upsert("c", "billing question").unwrap();
        let s = HybridSearcher::new(dense, sparse);
        let out = s
            .search(
                Some(&[1.0, 0.0, 0.0]),
                Some("login"),
                3,
                &HybridConfig {
                    mode: HybridMode::HybridRrf,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // `a` ranks rank-1 in BOTH runs ⇒ highest RRF score.
        assert_eq!(out[0].id, "a");
    }

    #[tokio::test]
    async fn hybrid_weighted_alpha_one_is_dense_only() {
        let dense = coll();
        dense.upsert(VectorDoc::new("a", vec![1.0, 0.0, 0.0])).await.unwrap();
        let sparse = Arc::new(Bm25Index::default());
        sparse.upsert("a", "irrelevant").unwrap();
        sparse.upsert("z", "very relevant text").unwrap();
        let s = HybridSearcher::new(dense, sparse);
        let out = s
            .search(
                Some(&[1.0, 0.0, 0.0]),
                Some("relevant"),
                5,
                &HybridConfig {
                    mode: HybridMode::HybridWeighted,
                    alpha: 1.0,
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // alpha=1.0 → only the dense run matters ; `a` is the only
        // dense hit so it ranks first.
        assert_eq!(out[0].id, "a");
    }

    #[tokio::test]
    async fn candidates_capped_at_1000() {
        let cfg = HybridConfig {
            mode: HybridMode::Dense,
            candidates_per_side: Some(usize::MAX),
            ..Default::default()
        };
        let dense = coll();
        dense.upsert(VectorDoc::new("a", vec![1.0; 3])).await.unwrap();
        let s = HybridSearcher::new(dense, Arc::new(Bm25Index::default()));
        // No panic / OOM despite huge `candidates_per_side`.
        let out = s.search(Some(&[1.0; 3]), None, 1, &cfg).await.unwrap();
        assert_eq!(out.len(), 1);
    }
}
