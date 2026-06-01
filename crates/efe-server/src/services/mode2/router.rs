//! Sprint **D34** (M4 § 13.3-13.4) — `LlmRouter` trait + built-in
//! implementations.
//!
//! ## Trait surface
//!
//! [`LlmRouter`] generates **dynamic anchors** from a free-form
//! `query_text` at request time. Mode-2 wraps the router output into
//! a `SwarmSpec` and runs the swarm against those anchors.
//!
//! ## Built-in routers (D34)
//!
//! * [`MockRouter`] — deterministic SHA-256-seeded anchors. Used by
//!   tests + the bench harness. Produces a fixed embedding of the
//!   requested dimension by hashing `(query_text, top_k, salt_idx)`.
//! * [`OpenAIRouter`] — gpt-4o-mini stub (D35 follow-up). Currently
//!   short-circuits to `MockRouter::generate_anchors` and tags the
//!   result `from_cache = false, llm_call_id = "openai-stub-..."` so
//!   the wire-level contract is observable end-to-end.
//! * [`AnthropicRouter`] — claude-haiku stub (D36 follow-up). Same
//!   short-circuit pattern as `OpenAIRouter`.
//!
//! ## Cache (mode2/cache.rs in M4 § 13.4)
//!
//! The `Mode2ServiceImpl` wraps **any** `Arc<dyn LlmRouter>` in a
//! `DashMap<u64, RouterOutput>` keyed on
//! `xxhash64(query_text || top_k || router_id)`. The default TTL is
//! 5 minutes (5 * 60 * 1000 ms).
//!
//! ## SLO contracts touched
//!
//! * **S-3** (latency p99 ≤ 1500 ms) — dominated by the LLM call ;
//!   the mock router is microsecond-fast.
//! * **S-11** (cache hit rate ≥ 60 %) — depends on workload, not on
//!   the router itself.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::time::Instant;
use thiserror::Error;

/// One dynamic anchor returned by the router.
#[derive(Debug, Clone)]
pub struct DynamicAnchor {
    /// Human-readable label for this anchor (used by the audit log
    /// and the verdict response).
    pub label: String,

    /// Centroid embedding. Dimension MUST match the active embedder
    /// bundle's dim — the handler validates and rejects mismatches
    /// with `tonic::Status::FailedPrecondition`.
    pub centroid: Vec<f32>,

    /// Confidence assigned by the router (0.0 .. 1.0). Used by the
    /// "weighted" Mode-3 aggregator (D37).
    pub confidence: f32,
}

/// Aggregate output of one router call.
#[derive(Debug, Clone)]
pub struct RouterOutput {
    /// One anchor per class generated. `len() ∈ [1, top_k]` (the
    /// router MAY return fewer when its uncertainty is high).
    pub anchors: Vec<DynamicAnchor>,

    /// Provider correlation id (or synthesised by mock routers).
    pub llm_call_id: String,

    /// True when the result came from the in-process cache. The
    /// router itself sets this to `false` ; the cache layer in
    /// `services/mode2/cache.rs` flips it to `true` on hit.
    pub from_cache: bool,

    /// LLM call latency in ms (router-internal ; excludes the
    /// downstream swarm time).
    pub latency_ms: u32,
}

/// Typed errors. Mapped onto `tonic::Status` by the handler.
#[derive(Debug, Error)]
pub enum LlmRouterError {
    /// Provider returned a 5xx or a non-recoverable error.
    #[error("router unavailable: {0}")]
    Unavailable(String),

    /// Provider returned a malformed response (JSON parse, missing
    /// fields, …).
    #[error("invalid router response: {0}")]
    InvalidResponse(String),

    /// `top_k` exceeded the router's hard cap.
    #[error("top_k = {requested} exceeds router cap {cap}")]
    TopKTooLarge {
        /// `top_k` value supplied by the caller.
        requested: u32,
        /// Router's hard upper bound (typically 20).
        cap: u32,
    },

    /// The configured embedder dim doesn't match what this router
    /// emits. (E.g. mock router fixed at 384, server bundle = 512.)
    #[error("dim mismatch: router emits {router_dim}, bundle expects {bundle_dim}")]
    DimMismatch {
        /// Dimension the router would have produced.
        router_dim: usize,
        /// Dimension the active bundle expects.
        bundle_dim: usize,
    },

    /// Generic catch-all (HTTP I/O, timeout, …).
    #[error("router I/O error: {0}")]
    Io(String),
}

/// The trait every router implements.
#[async_trait]
pub trait LlmRouter: Send + Sync + std::fmt::Debug {
    /// Stable identifier (`"mock"`, `"openai"`, `"anthropic"`, …).
    /// Used as the registry key in `Mode2ServiceImpl::routers`.
    fn id(&self) -> &'static str;

    /// Generate `top_k` dynamic anchors for `query_text`.
    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError>;

    /// Health probe (no LLM call). Default impl returns Ok ; routers
    /// with a real upstream override to ping it.
    async fn health(&self) -> Result<(), LlmRouterError> {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
// MockRouter — D34 default
// ─────────────────────────────────────────────────────────────────────

/// Deterministic SHA-256-seeded mock router.
///
/// Produces anchors whose centroid is derived from
/// `SHA-256(query_text || idx || salt)` chunked into the requested
/// dimension. Same `query_text` + same `top_k` ⇒ same output.
///
/// **Why this exists** :
///   * tests : deterministic verdict regardless of network state.
///   * bench harness : isolated from LLM provider latency.
///   * fallback : if all real routers fail, the operator can flip
///     to `mock` to keep traffic flowing while the upstream is
///     repaired.
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
    /// Build with a specific dim (e.g. 512 for some BERT variants).
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
        // Synthetic but stable per-call id : a 16-char hex
        // truncation of SHA-256(query_text).
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

/// Deterministic per-anchor centroid : SHA-256(query || idx || salt)
/// expanded into `dim` floats normalised to L2 = 1.
fn synth_centroid(query_text: &str, idx: u32, dim: usize) -> Vec<f32> {
    let mut hasher = Sha256::new();
    hasher.update(query_text.as_bytes());
    hasher.update(idx.to_le_bytes());
    let mut seed = hasher.finalize().to_vec();
    // Expand the 32-byte seed into dim*4 bytes by re-hashing. We
    // chain `seed = SHA256(seed)` until we have enough material.
    while seed.len() < dim * 4 {
        let next = Sha256::digest(&seed);
        seed.extend_from_slice(&next);
    }
    let mut v: Vec<f32> = seed
        .chunks_exact(4)
        .take(dim)
        .map(|c| {
            // Map 32 bits → centred float in [-1, 1] via i32 cast.
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

// ─────────────────────────────────────────────────────────────────────
// OpenAIRouter — D35 stub
// ─────────────────────────────────────────────────────────────────────

/// Stub router for the OpenAI Chat Completions API.
///
/// **D34 status** : the network call is NOT wired ; we delegate to
/// [`MockRouter`] internally. The router id stays `"openai"` so the
/// per-router metric labels and the auth path are exercised by D34
/// tests. D35 swaps the inner mock for a real `reqwest::Client` call.
#[derive(Debug, Clone)]
pub struct OpenAIRouter {
    inner: MockRouter,
}

impl Default for OpenAIRouter {
    fn default() -> Self {
        Self {
            inner: MockRouter::default(),
        }
    }
}

#[async_trait]
impl LlmRouter for OpenAIRouter {
    fn id(&self) -> &'static str {
        "openai"
    }

    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError> {
        let mut out = self.inner.generate_anchors(query_text, top_k).await?;
        // Re-tag the call_id so consumers can tell mock-vs-openai apart.
        out.llm_call_id = format!("openai-stub-{}", out.llm_call_id);
        Ok(out)
    }
}

// ─────────────────────────────────────────────────────────────────────
// AnthropicRouter — D36 stub
// ─────────────────────────────────────────────────────────────────────

/// Stub router for the Anthropic Messages API.
///
/// **D34 status** : same stub pattern as [`OpenAIRouter`].
#[derive(Debug, Clone)]
pub struct AnthropicRouter {
    inner: MockRouter,
}

impl Default for AnthropicRouter {
    fn default() -> Self {
        Self {
            inner: MockRouter::default(),
        }
    }
}

#[async_trait]
impl LlmRouter for AnthropicRouter {
    fn id(&self) -> &'static str {
        "anthropic"
    }

    async fn generate_anchors(
        &self,
        query_text: &str,
        top_k: u32,
    ) -> Result<RouterOutput, LlmRouterError> {
        let mut out = self.inner.generate_anchors(query_text, top_k).await?;
        out.llm_call_id = format!("anthropic-stub-{}", out.llm_call_id);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_router_is_deterministic() {
        let r = MockRouter::default();
        let a = r.generate_anchors("hello", 3).await.unwrap();
        let b = r.generate_anchors("hello", 3).await.unwrap();
        // Same query → same call_id (deterministic).
        assert_eq!(a.llm_call_id, b.llm_call_id);
        assert_eq!(a.anchors.len(), 3);
        assert_eq!(b.anchors.len(), 3);
        // Same centroids per index.
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
    async fn openai_stub_tags_call_id() {
        let r = OpenAIRouter::default();
        let out = r.generate_anchors("q", 1).await.unwrap();
        assert!(out.llm_call_id.starts_with("openai-stub-"));
        assert_eq!(r.id(), "openai");
    }

    #[tokio::test]
    async fn anthropic_stub_tags_call_id() {
        let r = AnthropicRouter::default();
        let out = r.generate_anchors("q", 1).await.unwrap();
        assert!(out.llm_call_id.starts_with("anthropic-stub-"));
        assert_eq!(r.id(), "anthropic");
    }

    #[tokio::test]
    async fn three_routers_have_distinct_ids() {
        let m: &dyn LlmRouter = &MockRouter::default();
        let o: &dyn LlmRouter = &OpenAIRouter::default();
        let a: &dyn LlmRouter = &AnthropicRouter::default();
        let ids = [m.id(), o.id(), a.id()];
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), 3);
    }
}
