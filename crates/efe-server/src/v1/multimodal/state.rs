//! M30 — Trait abstraction over the multimodal plane.
//!
//! Allows handlers to be tested without spinning up a Triton or
//! Postgres : the test harness provides an in-memory mock that
//! returns deterministic embeddings.

use async_trait::async_trait;
use efe_multimodal::{
    AudioOutput, ChunkConfig, FusionStrategy, GuardPolicy, ModalityWeights, TenantModalityPolicy,
    VisionOutput,
};

use super::errors::MultimodalApiError;

/// Handler state trait : provides the dependency surface for
/// `/v1/multimodal/*` handlers.
#[async_trait]
pub trait MultimodalApiState: Clone + Send + Sync + 'static {
    /// Authenticate a request. Returns `Some(tenant_id)` on success,
    /// `None` on missing or invalid bearer.
    fn auth_tenant_from_request(&self, parts: &axum::http::request::Parts) -> Option<String>;

    /// Returns `true` iff the caller has the `multimodal:write` scope.
    fn caller_has_multimodal_scope(&self, parts: &axum::http::request::Parts) -> bool;

    /// Load the per-tenant modality policy from PG.
    async fn load_policy(
        &self,
        tenant_id: &str,
    ) -> Result<TenantModalityPolicy, MultimodalApiError>;

    /// Persist an updated modality policy.
    async fn save_policy(
        &self,
        tenant_id: &str,
        policy: &TenantModalityPolicy,
    ) -> Result<(), MultimodalApiError>;

    /// Encode a vision input via the SigLIP encoder.
    async fn encode_vision(
        &self,
        bytes: &[u8],
        guard: &GuardPolicy,
    ) -> Result<VisionOutput, MultimodalApiError>;

    /// Encode an audio input via the Whisper encoder.
    async fn encode_audio(
        &self,
        bytes: &[u8],
        guard: &GuardPolicy,
    ) -> Result<AudioOutput, MultimodalApiError>;

    /// Run the long-text chunker.
    async fn chunk_text(
        &self,
        text: &str,
        cfg: &ChunkConfig,
    ) -> Result<Vec<efe_multimodal::Chunk>, MultimodalApiError>;

    /// Run the late-fusion stage.
    async fn fuse(
        &self,
        vision: Option<&[f32]>,
        audio: Option<&[f32]>,
        text: Option<&[f32]>,
        strategy: FusionStrategy,
        weights: ModalityWeights,
        target_dim: usize,
    ) -> Result<Vec<f32>, MultimodalApiError>;
}
