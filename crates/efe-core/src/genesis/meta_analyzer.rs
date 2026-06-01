//! Meta-analyzer: classifies a problem embedding into a [`TaskDag`].
//!
//! MVP uses a k-NN-against-exemplars heuristic: for each [`TaskClass`]
//! we keep one "centroid" embedding derived from a calibration set.
//! At inference time we compute cosine similarity of the input against
//! each centroid and return a softmax distribution.
//!
//! ## Sprint 5 — façade over `ComputeBackend`
//!
//! Pre-Sprint-5, this module owned a private `cosine_similarity`
//! function that was a *strict duplicate* of `els::metrics::cosine_similarity`.
//! It also rolled its own softmax inline. Sprint 5 routes both
//! computations through [`crate::compute::ComputeBackend`] via the
//! shared `els::metrics::cosine_similarity` façade and
//! `backend.softmax`. The duplicate was deleted.

use std::sync::Arc;

use async_trait::async_trait;

use crate::compute::{global_backend, Tensor};
use crate::els::metrics::cosine_similarity;
use crate::errors::{EfeError, Result};
use crate::genesis::context::GenesisContext;
use crate::genesis::dag::{TaskClass, TaskDag, TaskMix};

/// Analyzes a problem embedding to produce a [`TaskDag`].
///
/// Implementations must be deterministic (same input → same output) to
/// enable caching and reproducibility in tests.
#[async_trait]
pub trait MetaAnalyzer: Send + Sync {
    /// Decompose a problem into a task-class distribution.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if the embedding has wrong
    ///   length or contains non-finite values.
    async fn decompose(
        &self,
        problem_embedding: Arc<[f32]>,
        ctx: &GenesisContext,
    ) -> Result<TaskDag>;

    /// Expected embedding dimensionality. The engine verifies this
    /// matches `spec.dim_latent` at startup.
    fn expected_dim(&self) -> u32;
}

/// Heuristic meta-analyzer using k-NN against class centroid embeddings.
///
/// # Determinism
///
/// Cosine similarity and softmax are pure functions of the input. The
/// same `(embedding, exemplars, temperature)` tuple always yields the
/// same [`TaskDag`].
pub struct HeuristicMetaAnalyzer {
    exemplars: [Arc<[f32]>; TaskClass::COUNT],
    dim: u32,
    temperature: f32,
}

impl HeuristicMetaAnalyzer {
    /// Minimum allowed temperature (prevents divide-by-zero in softmax).
    pub const MIN_TEMPERATURE: f32 = 1e-4;

    /// Default MVP exemplars, derived from deterministic patterns.
    ///
    /// Not calibrated against real data; suitable for tests and
    /// pipeline verification. Production M3 replaces these with
    /// exemplars loaded from a signed safetensors file.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `dim == 0`.
    pub fn load_default(dim: u32) -> Result<Self> {
        if dim == 0 {
            return Err(EfeError::InvalidConfig(
                "HeuristicMetaAnalyzer: dim must be > 0".into(),
            ));
        }
        let exemplars = Self::synth_default_exemplars(dim);
        Self::new(exemplars, /* temperature */ 0.5)
    }

    /// Construct with explicit exemplars and softmax temperature.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if any exemplar length mismatches
    ///   the others, or if `temperature` is non-finite / too small.
    pub fn new(exemplars: [Arc<[f32]>; TaskClass::COUNT], temperature: f32) -> Result<Self> {
        if !temperature.is_finite() || temperature < Self::MIN_TEMPERATURE {
            return Err(EfeError::InvalidConfig(format!(
                "HeuristicMetaAnalyzer: temperature {temperature} must be finite and >= {}",
                Self::MIN_TEMPERATURE
            )));
        }
        let dim = exemplars[0].len() as u32;
        if dim == 0 {
            return Err(EfeError::InvalidConfig(
                "HeuristicMetaAnalyzer: exemplar dim must be > 0".into(),
            ));
        }
        for (i, ex) in exemplars.iter().enumerate() {
            if ex.len() as u32 != dim {
                return Err(EfeError::InvalidConfig(format!(
                    "HeuristicMetaAnalyzer: exemplar {i} has dim {} != {dim}",
                    ex.len()
                )));
            }
            if !ex.iter().all(|v| v.is_finite()) {
                return Err(EfeError::InvalidConfig(format!(
                    "HeuristicMetaAnalyzer: exemplar {i} contains NaN or Inf"
                )));
            }
        }
        Ok(Self {
            exemplars,
            dim,
            temperature,
        })
    }

    fn synth_default_exemplars(dim: u32) -> [Arc<[f32]>; TaskClass::COUNT] {
        // Each class gets a distinct deterministic pattern. The
        // patterns are designed so cosine similarity varies smoothly
        // across the unit sphere.
        let mk = |phase: f32, scale: f32| -> Arc<[f32]> {
            let v: Vec<f32> = (0..dim)
                .map(|i| {
                    let x = i as f32 / dim as f32;
                    scale * ((x * std::f32::consts::PI * 2.0 + phase).sin())
                })
                .collect();
            Arc::from(v)
        };
        [
            mk(0.00, 1.0),                        // FactoidQa
            mk(std::f32::consts::PI * 0.40, 1.2), // MathWord
            mk(std::f32::consts::PI * 0.80, 1.1), // LogicalInference
            mk(std::f32::consts::PI * 1.20, 0.9), // Classification
            mk(std::f32::consts::PI * 1.60, 1.0), // Summarization
        ]
    }

}

#[async_trait]
impl MetaAnalyzer for HeuristicMetaAnalyzer {
    async fn decompose(
        &self,
        problem_embedding: Arc<[f32]>,
        _ctx: &GenesisContext,
    ) -> Result<TaskDag> {
        if problem_embedding.len() as u32 != self.dim {
            return Err(EfeError::InvalidConfig(format!(
                "HeuristicMetaAnalyzer: embedding dim {} != expected {}",
                problem_embedding.len(),
                self.dim
            )));
        }
        if !problem_embedding.iter().all(|v| v.is_finite()) {
            return Err(EfeError::InvalidConfig(
                "HeuristicMetaAnalyzer: embedding contains NaN or Inf".into(),
            ));
        }

        // 1. Cosine similarity to each exemplar (façade ➜ backend).
        let mut scores = [0.0f32; TaskClass::COUNT];
        for (i, ex) in self.exemplars.iter().enumerate() {
            scores[i] = cosine_similarity(&problem_embedding, ex);
        }

        // 2. Temperature-scaled softmax via the backend.
        //    `backend.softmax` already applies LogSumExp internally for
        //    numerical stability, so we only need to pre-divide by T.
        let backend = global_backend();
        let scaled: Vec<f32> = scores.iter().map(|s| s / self.temperature).collect();
        let scaled_t = Tensor::<f32>::from_slice(
            &scaled,
            [scaled.len()].into(),
        )
        .map_err(|e| EfeError::InvalidConfig(format!(
            "meta_analyzer scores tensor: {e}"
        )))?;
        let probs_t = backend
            .softmax(&scaled_t, -1)
            .map_err(|e| EfeError::InvalidConfig(format!(
                "meta_analyzer softmax: {e}"
            )))?;
        let probs_slice = probs_t
            .as_slice()
            .map_err(|_| EfeError::Internal("meta_analyzer softmax slice"))?;

        // 3. Defensive renormalisation: backend.softmax already produces
        //    a row-stochastic vector, but if all scores collapse to a
        //    single mode we fall back to uniform — preserving the legacy
        //    behaviour.
        let total: f32 = probs_slice.iter().sum();
        let probs: Vec<f32> = if total > 0.0 {
            probs_slice.iter().map(|p| p / total).collect()
        } else {
            vec![1.0 / TaskClass::COUNT as f32; TaskClass::COUNT]
        };

        let task_mix = TaskMix::new(probs)?;
        Ok(TaskDag::from_mix(task_mix, problem_embedding))
    }

    fn expected_dim(&self) -> u32 {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SwarmId;

    fn mk_ctx() -> GenesisContext {
        GenesisContext::minimal(SwarmId::new())
    }

    #[tokio::test]
    async fn heuristic_decomposes_valid_embedding() {
        let ma = HeuristicMetaAnalyzer::load_default(32).unwrap();
        let emb: Arc<[f32]> = Arc::from(vec![0.1f32; 32]);
        let dag = ma.decompose(emb, &mk_ctx()).await.unwrap();
        assert!((dag.task_mix.as_slice().iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn heuristic_rejects_wrong_dim() {
        let ma = HeuristicMetaAnalyzer::load_default(32).unwrap();
        let emb: Arc<[f32]> = Arc::from(vec![0.1f32; 16]);
        let err = ma.decompose(emb, &mk_ctx()).await.unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn heuristic_rejects_nan_embedding() {
        let ma = HeuristicMetaAnalyzer::load_default(8).unwrap();
        let emb: Arc<[f32]> = Arc::from(vec![f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        let err = ma.decompose(emb, &mk_ctx()).await.unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn heuristic_is_deterministic() {
        let ma = HeuristicMetaAnalyzer::load_default(64).unwrap();
        let emb: Arc<[f32]> = Arc::from((0..64).map(|i| (i as f32) * 0.01).collect::<Vec<_>>());
        let dag1 = ma.decompose(emb.clone(), &mk_ctx()).await.unwrap();
        let dag2 = ma.decompose(emb, &mk_ctx()).await.unwrap();
        assert_eq!(dag1.primary_class, dag2.primary_class);
        assert_eq!(dag1.task_mix.as_slice(), dag2.task_mix.as_slice());
    }

    #[tokio::test]
    async fn heuristic_argmax_matches_closest_exemplar() {
        // An input identical to one exemplar should argmax to that class.
        let ma = HeuristicMetaAnalyzer::load_default(32).unwrap();
        let ex = ma.exemplars[TaskClass::MathWord.index()].clone();
        let dag = ma.decompose(ex, &mk_ctx()).await.unwrap();
        assert_eq!(dag.primary_class, TaskClass::MathWord);
    }

    #[test]
    fn rejects_invalid_temperature() {
        let ex = HeuristicMetaAnalyzer::synth_default_exemplars(16);
        assert!(HeuristicMetaAnalyzer::new(ex.clone(), 0.0).is_err());
        assert!(HeuristicMetaAnalyzer::new(ex.clone(), -1.0).is_err());
        assert!(HeuristicMetaAnalyzer::new(ex, f32::NAN).is_err());
    }

    #[test]
    fn rejects_zero_dim() {
        assert!(HeuristicMetaAnalyzer::load_default(0).is_err());
    }

    #[test]
    fn meta_analyzer_cosine_zero_vector_returns_zero() {
        // Sprint 5: now uses the shared `els::metrics::cosine_similarity`
        // façade, which delegates to `compute::ComputeBackend`. Geometric
        // limit on zero-norm input still returns 0.
        let a = vec![0.0f32; 4];
        let b = vec![1.0f32; 4];
        let s = cosine_similarity(&a, &b);
        assert!(s.abs() < 1e-6);
    }

    #[test]
    fn meta_analyzer_cosine_identical_returns_one() {
        let a = vec![1.0f32, 2.0, 3.0];
        let s = cosine_similarity(&a, &a);
        assert!((s - 1.0).abs() < 1e-6);
    }
}
