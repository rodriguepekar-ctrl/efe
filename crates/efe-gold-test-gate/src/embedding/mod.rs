// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : Embedding provider abstraction.
//!
//! ADR-3 : LLM semantic similarity via embedding cosine + fixed
//! model.  This module pins the trait + ships a `Stub` that returns
//! deterministic hashed vectors so the wire format is frozen and the
//! Sprint 11 acceptance suite is self-contained.
//!
//! **Sprint 12 — N20** : real providers added.

/// **N20** : per-provider circuit breaker (3-fail → Open → 30s → HalfOpen).
pub mod circuit_breaker;
/// **N20** : Cohere embed-multilingual-v3.0 provider (reqwest-backed).
pub mod cohere;
/// **N20** : Priority-ordered multi-provider client with failover.
pub mod multi_provider;
/// **N20** : OpenAI text-embedding-3-small provider (reqwest-backed).
pub mod openai;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use efe_nano_sdk::ModelIdentity;
use sha2::{Digest, Sha256};
use thiserror::Error;

/// **N19** : closed taxonomy of embedding errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum EmbeddingProviderError {
    #[error("embedding provider rate limited")]
    RateLimited,

    #[error("embedding provider migrated mid-run : expected {expected}, got {got}")]
    ModelDrift { expected: String, got: String },

    #[error("embedding provider : {message}")]
    Backend { message: String },
}

/// **N19** : a single embedded vector (fixed dimension per model).
#[derive(Debug, Clone, PartialEq)]
pub struct Embedding {
    pub model: ModelIdentity,
    pub vector: Vec<f64>,
}

impl Embedding {
    /// **N19** : Compute cosine similarity in `[-1.0, 1.0]`.  Returns
    /// `None` if either vector has zero magnitude (degenerate).
    pub fn cosine(&self, other: &Self) -> Option<f64> {
        if self.vector.len() != other.vector.len() {
            return None;
        }
        let mut dot = 0.0_f64;
        let mut na = 0.0_f64;
        let mut nb = 0.0_f64;
        for (a, b) in self.vector.iter().zip(other.vector.iter()) {
            dot += a * b;
            na += a * a;
            nb += b * b;
        }
        if na == 0.0 || nb == 0.0 {
            return None;
        }
        Some(dot / (na.sqrt() * nb.sqrt()))
    }
}

/// **N19** : trait for embedding providers.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync + 'static {
    /// **N19** : Embed `text` using `model` ; returns the vector +
    /// the model identity actually used (compared by caller against
    /// `model` to detect mid-run drift — F10).
    async fn embed(
        &self,
        model: &ModelIdentity,
        text: &str,
    ) -> Result<Embedding, EmbeddingProviderError>;
}

/// **N19** : deterministic stub.  Given the same `(model, text)`
/// pair, returns the same vector.  Different texts produce
/// different vectors (with high probability).  The vector is built
/// from SHA-256(model || text) repeated to 256 dimensions to give a
/// realistic-shape vector.
///
/// Sprint 11 acceptance tests rely on this determinism : it lets
/// `cosine(x, x) == 1.0` exactly (modulo float ε), which validates
/// the `SemanticSimilarity` strategy plumbing.
pub struct StubEmbeddingProvider {
    /// If true, every call returns RateLimited (used to simulate
    /// F5).
    force_rate_limit: AtomicBool,
    /// Optional drift simulator : when set, the provider returns
    /// a different `model.served_version` than the requested one
    /// (used to simulate F10).
    drift_simulated: AtomicBool,
    /// Counts calls — handy for cost / cache tests.
    call_count: AtomicU64,
}

impl Default for StubEmbeddingProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for StubEmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StubEmbeddingProvider")
            .field("call_count", &self.call_count.load(Ordering::Relaxed))
            .finish()
    }
}

impl StubEmbeddingProvider {
    pub fn new() -> Self {
        Self {
            force_rate_limit: AtomicBool::new(false),
            drift_simulated: AtomicBool::new(false),
            call_count: AtomicU64::new(0),
        }
    }

    pub fn set_force_rate_limit(&self, on: bool) {
        self.force_rate_limit.store(on, Ordering::SeqCst);
    }

    pub fn set_drift_simulated(&self, on: bool) {
        self.drift_simulated.store(on, Ordering::SeqCst);
    }

    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// **N19** : Stub-specific embedding dimension (256).  Real
    /// providers will have model-specific dimensions (1536 for
    /// OpenAI text-embedding-3-small, etc.).
    pub const DIMENSION: usize = 256;
}

#[async_trait]
impl EmbeddingProvider for StubEmbeddingProvider {
    async fn embed(
        &self,
        model: &ModelIdentity,
        text: &str,
    ) -> Result<Embedding, EmbeddingProviderError> {
        if self.force_rate_limit.load(Ordering::SeqCst) {
            return Err(EmbeddingProviderError::RateLimited);
        }
        self.call_count.fetch_add(1, Ordering::Relaxed);

        if self.drift_simulated.load(Ordering::SeqCst) {
            return Err(EmbeddingProviderError::ModelDrift {
                expected: format!(
                    "{}/{}/{}",
                    model.provider, model.model_id, model.requested_version
                ),
                got: format!(
                    "{}/{}/{}-drifted",
                    model.provider, model.model_id, model.requested_version
                ),
            });
        }

        // Deterministic vector : SHA-256(model identity || text)
        // repeated 8 times → 256 bytes → 256 floats normalised
        // around 0 (subtract 127.5 then divide by 127.5).
        let mut hasher = Sha256::new();
        hasher.update(model.provider.as_bytes());
        hasher.update(b"|");
        hasher.update(model.model_id.as_bytes());
        hasher.update(b"|");
        hasher.update(model.requested_version.as_bytes());
        hasher.update(b"|");
        hasher.update(text.as_bytes());
        let mut seed = hasher.finalize().to_vec();
        // Expand to 256 bytes by re-hashing.
        let mut expanded = Vec::with_capacity(Self::DIMENSION);
        let mut iter = 0u8;
        while expanded.len() < Self::DIMENSION {
            for b in seed.iter() {
                expanded.push(*b);
                if expanded.len() >= Self::DIMENSION {
                    break;
                }
            }
            let mut h2 = Sha256::new();
            h2.update(&seed);
            h2.update([iter]);
            seed = h2.finalize().to_vec();
            iter = iter.wrapping_add(1);
        }
        let vector: Vec<f64> = expanded
            .into_iter()
            .map(|b| (b as f64 - 127.5) / 127.5)
            .collect();
        Ok(Embedding {
            model: model.clone(),
            vector,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ModelIdentity {
        ModelIdentity::hosted("openai", "text-embedding-3-small", "1")
    }

    #[tokio::test]
    async fn stub_embed_deterministic_same_input() {
        let p = StubEmbeddingProvider::new();
        let e1 = p.embed(&model(), "hello").await.unwrap();
        let e2 = p.embed(&model(), "hello").await.unwrap();
        assert_eq!(e1.vector, e2.vector);
    }

    #[tokio::test]
    async fn stub_embed_different_input_different_vector() {
        let p = StubEmbeddingProvider::new();
        let e1 = p.embed(&model(), "hello").await.unwrap();
        let e2 = p.embed(&model(), "world").await.unwrap();
        assert_ne!(e1.vector, e2.vector);
    }

    #[tokio::test]
    async fn stub_embed_correct_dimension() {
        let p = StubEmbeddingProvider::new();
        let e = p.embed(&model(), "x").await.unwrap();
        assert_eq!(e.vector.len(), StubEmbeddingProvider::DIMENSION);
    }

    #[tokio::test]
    async fn stub_force_rate_limit_returns_error() {
        let p = StubEmbeddingProvider::new();
        p.set_force_rate_limit(true);
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::RateLimited));
    }

    #[tokio::test]
    async fn stub_drift_simulated_returns_drift_error() {
        let p = StubEmbeddingProvider::new();
        p.set_drift_simulated(true);
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::ModelDrift { .. }));
    }

    #[tokio::test]
    async fn cosine_self_equals_one() {
        let p = StubEmbeddingProvider::new();
        let e = p.embed(&model(), "hello").await.unwrap();
        let c = e.cosine(&e).unwrap();
        assert!((c - 1.0).abs() < 1e-10);
    }

    #[tokio::test]
    async fn cosine_returns_none_on_dim_mismatch() {
        let mut e1 = StubEmbeddingProvider::new().embed(&model(), "x").await.unwrap();
        let e2 = StubEmbeddingProvider::new().embed(&model(), "x").await.unwrap();
        e1.vector.truncate(10);
        assert!(e1.cosine(&e2).is_none());
    }

    #[tokio::test]
    async fn cosine_zero_vector_returns_none() {
        let e1 = Embedding {
            model: model(),
            vector: vec![0.0; 10],
        };
        let e2 = Embedding {
            model: model(),
            vector: vec![1.0; 10],
        };
        assert!(e1.cosine(&e2).is_none());
    }

    #[tokio::test]
    async fn stub_call_count_increments() {
        let p = StubEmbeddingProvider::new();
        assert_eq!(p.call_count(), 0);
        let _ = p.embed(&model(), "x").await.unwrap();
        let _ = p.embed(&model(), "y").await.unwrap();
        assert_eq!(p.call_count(), 2);
    }

    #[tokio::test]
    async fn cosine_different_text_below_one() {
        let p = StubEmbeddingProvider::new();
        let a = p.embed(&model(), "alice").await.unwrap();
        let b = p.embed(&model(), "bob").await.unwrap();
        let c = a.cosine(&b).unwrap();
        assert!(c.abs() <= 1.0);
        // Two unrelated hashed texts should have ~0 cosine ; not
        // strictly < 1 every time (random vectors), but very high
        // probability.
        assert!(c < 0.5);
    }

    #[test]
    fn debug_format_no_panic() {
        let p = StubEmbeddingProvider::new();
        let _ = format!("{p:?}");
    }
}
