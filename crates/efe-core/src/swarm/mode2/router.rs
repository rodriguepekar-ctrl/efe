//! **Sprint M1 extension D34** — `LlmRouter` trait + `MockRouter`.
//!
//! ## Backport
//!
//! This file is a backport of the `LlmRouter` trait from
//! `crates/efe-server/src/services/mode2/router.rs` (delivered in
//! M4 phase 1) into `efe-core`. The trait moves down ; the network
//! impls (`OpenAIRouter`, `AnthropicRouter`) **stay in `efe-server`**
//! because they depend on `reqwest` (HTTP client) which is a
//! gRPC-server-side concern, not a core-engine concern.
//!
//! Cf. `EFE_CORE_EXTENSION_ARCHITECTURE.md` § 10.4.
//!
//! ## What lives here (efe-core)
//!
//!   * The [`LlmRouter`] trait — public, implementable from any layer.
//!   * The [`MockRouter`] impl — pure-Rust, deterministic via
//!     SHA-256, no network. Used by tests + benches + production
//!     fallback.
//!   * Typed errors via [`LlmRouterError`].
//!
//! ## What stays in efe-server
//!
//!   * `OpenAIRouter` (gpt-4o-mini stub D35).
//!   * `AnthropicRouter` (claude-haiku stub D36).
//!
//! Both implement `LlmRouter` (via the public trait re-exported
//! by efe-core) so they integrate cleanly without architectural
//! contortions.

use std::time::Instant;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::errors::EfeError;

/// One dynamic anchor returned by the router.
#[derive(Debug, Clone)]
pub struct DynamicAnchor {
    /// Human-readable label for this anchor.
    pub label: String,
    /// Centroid embedding ; dim must match the active embedder.
    pub centroid: Vec<f32>,
    /// Confidence in [0.0, 1.0] assigned by the router.
    pub confidence: f32,
}

/// Aggregate output of one router call.
#[derive(Debug, Clone)]
pub struct RouterOutput {
    /// Per-class anchors. `len() ∈ [1, top_k]`.
    pub anchors: Vec<DynamicAnchor>,
    /// Provider correlation id (or synthesised by mock router).
    pub llm_call_id: String,
    /// True when the result came from the in-process cache.
    /// The router itself sets `false` ; the cache layer flips it.
    pub from_cache: bool,
    /// LLM call latency in ms (router-internal, excludes swarm).
    pub latency_ms: u32,
}

/// Typed errors emitted by `LlmRouter` impls.
#[derive(Debug, Error)]
pub enum LlmRouterError {
    /// Provider returned a 5xx or non-recoverable error.
    #[error("router unavailable: {0}")]
    Unavailable(String),
    /// Provider returned a malformed response.
    #[error("invalid router response: {0}")]
    InvalidResponse(String),
    /// `top_k` exceeded the router's hard cap.
    #[error("top_k = {requested} exceeds router cap {cap}")]
    TopKTooLarge {
        /// Caller-supplied top_k.
        requested: u32,
        /// Router's hard cap (typically 20).
        cap: u32,
    },
    /// Embedder dim doesn't match what this router emits.
    #[error("dim mismatch: router={router_dim}, bundle={bundle_dim}")]
    DimMismatch {
        /// Dim the router would emit.
        router_dim: usize,
        /// Dim the active bundle expects.
        bundle_dim: usize,
    },
    /// Generic catch-all (HTTP I/O, timeout, …).
    #[error("router I/O error: {0}")]
    Io(String),
}

impl From<LlmRouterError> for EfeError {
    fn from(e: LlmRouterError) -> Self {
        EfeError::InvalidConfig(format!("LlmRouter: {e}"))
    }
}

/// **Sprint D34** — Trait every router (mock, OpenAI, Anthropic, …)
/// implements.
///
/// Trait bounds : `Send + Sync + Debug` so trait objects can move
/// across tokio tasks and surface usefully in tracing logs.
#[async_trait]
pub trait LlmRouter: Send + Sync + std::fmt::Debug {
    /// Stable identifier (`"mock"`, `"openai"`, `"anthropic"`, …).
    /// Used as the registry key in `Mode2Controller::routers`.
    fn id(&self) -> &'static str;

    /// Generate `top_k` dynamic anchors for `query_text`.
    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError>;

    /// Health probe (no LLM call). Default impl returns Ok.
    async fn health(&self) -> Result<(), LlmRouterError> {
        Ok(())
    }
}

// =====================================================================
// MockRouter — D34 default
// =====================================================================

/// **Sprint D34** — Deterministic SHA-256-seeded mock router.
///
/// Pure-Rust, no network deps. Produces anchors whose centroid is
/// derived from `SHA-256(query_text || idx || salt)` chunked into
/// the requested dimension. Same `(query, top_k)` ⇒ same output
/// across runs (institutional determinism guarantee).
///
/// ## Use cases
///
///   * **Tests** : deterministic verdict regardless of network.
///   * **Bench harness** : isolated from LLM provider latency.
///   * **Production fallback** : if real routers fail, operators
///     can flip to `mock` to keep traffic flowing while the
///     upstream is repaired.
#[derive(Debug, Clone)]
pub struct MockRouter {
    /// Embedding dimension. Must match the active embedder bundle.
    pub dim: usize,
    /// Hard cap on `top_k`. Default 20.
    pub max_top_k: u32,
}

impl Default for MockRouter {
    fn default() -> Self {
        Self {
            dim: 384,
            max_top_k: 20,
        }
    }
}

impl MockRouter {
    /// Build with a specific dim (e.g. 512 for some BERT variants,
    /// 8 for the synthesis bench).
    #[must_use]
    pub fn with_dim(dim: usize) -> Self {
        Self {
            dim,
            ..Self::default()
        }
    }
}

#[async_trait]
impl LlmRouter for MockRouter {
    fn id(&self) -> &'static str {
        "mock"
    }

    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError> {
        if top_k > self.max_top_k {
            return Err(LlmRouterError::TopKTooLarge {
                requested: top_k,
                cap: self.max_top_k,
            });
        }
        let started = Instant::now();
        let dim = self.dim;
        let mut anchors = Vec::with_capacity(top_k as usize);
        for idx in 0..top_k {
            let centroid = synth_centroid(query_text, idx, dim);
            anchors.push(DynamicAnchor {
                label: format!("mock_anchor_{idx}"),
                centroid,
                confidence: 1.0 - (idx as f32) * 0.05,
            });
        }
        // Synthetic but stable per-call id : 16-char hex truncation
        // of SHA-256(query_text). Same query ⇒ same call_id.
        let llm_call_id = format!(
            "mock-{}",
            &hex::encode(Sha256::digest(query_text.as_bytes()))[..16]
        );
        Ok(RouterOutput {
            anchors,
            llm_call_id,
            from_cache: false,
            latency_ms: started.elapsed().as_millis() as u32,
        })
    }
}

/// Deterministic per-anchor centroid derivation.
///
/// `SHA-256(query || idx)` is chained until we have enough material
/// for `dim` f32 values. Each f32 lives in `[-1, 1]` and the
/// final vector is L2-normalised so it's a **unit vector**.
fn synth_centroid(query_text: &str, idx: u32, dim: usize) -> Vec<f32> {
    let mut hasher = Sha256::new();
    hasher.update(query_text.as_bytes());
    hasher.update(idx.to_le_bytes());
    let mut seed = hasher.finalize().to_vec();
    while seed.len() < dim * 4 {
        let next = Sha256::digest(&seed);
        seed.extend_from_slice(&next);
    }
    let mut v: Vec<f32> = seed
        .chunks_exact(4)
        .take(dim)
        .map(|c| {
            let raw = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            (raw as f32) / (i32::MAX as f32)
        })
        .collect();
    debug_assert_eq!(v.len(), dim);
    // L2-normalise — Mode-2 spec MUST emit unit vectors so the
    // downstream swarm geometry is well-defined.
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_router_id_is_mock() {
        let r = MockRouter::default();
        assert_eq!(r.id(), "mock");
    }

    #[tokio::test]
    async fn mock_router_is_deterministic() {
        let r = MockRouter::default();
        let a = r.generate_anchors("hello", 3).await.unwrap();
        let b = r.generate_anchors("hello", 3).await.unwrap();
        assert_eq!(a.llm_call_id, b.llm_call_id);
        assert_eq!(a.anchors.len(), 3);
        for (x, y) in a.anchors.iter().zip(b.anchors.iter()) {
            assert_eq!(x.centroid, y.centroid);
            assert_eq!(x.label, y.label);
        }
    }

    #[tokio::test]
    async fn mock_router_centroids_are_unit_norm() {
        let r = MockRouter::with_dim(64);
        let out = r.generate_anchors("normal test", 5).await.unwrap();
        for a in &out.anchors {
            let n: f32 = a.centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((n - 1.0).abs() < 1e-3, "centroid not unit-norm: {n}");
        }
    }

    #[tokio::test]
    async fn mock_router_top_k_cap() {
        let r = MockRouter {
            dim: 64,
            max_top_k: 5,
        };
        let err = r.generate_anchors("oops", 6).await.unwrap_err();
        assert!(matches!(err, LlmRouterError::TopKTooLarge { .. }));
    }

    #[tokio::test]
    async fn mock_router_health_ok() {
        let r = MockRouter::default();
        r.health().await.unwrap();
    }

    #[tokio::test]
    async fn distinct_queries_distinct_call_ids() {
        let r = MockRouter::default();
        let a = r.generate_anchors("query A", 1).await.unwrap();
        let b = r.generate_anchors("query B", 1).await.unwrap();
        assert_ne!(a.llm_call_id, b.llm_call_id);
    }

    #[test]
    fn llm_router_error_converts_to_efe_error() {
        let e: EfeError = LlmRouterError::Unavailable("test".into()).into();
        let s = format!("{e}");
        assert!(s.contains("LlmRouter"));
        assert!(s.contains("test"));
    }
}
