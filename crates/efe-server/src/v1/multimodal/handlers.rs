//! M30 — `/v1/multimodal/*` HTTP handlers.

use axum::{
    extract::State,
    routing::{get, post, put},
    Json, Router,
};
use base64::Engine;
use efe_multimodal::{
    ChunkConfig, FusionStrategy, GuardModality, ModalityWeights, TenantModalityPolicy,
};
use serde::{Deserialize, Serialize};

use super::{errors::MultimodalApiError, state::MultimodalApiState};

// ─────────────────────────── REQUEST BODIES ───────────────────────────

#[derive(Debug, Deserialize)]
pub struct VisionEncodeBody {
    /// Base64-encoded image bytes. The handler validates magic bytes.
    pub image_b64: String,
}

#[derive(Debug, Serialize)]
pub struct VisionEncodeResponse {
    pub model: &'static str,
    pub dim: usize,
    pub embedding: Vec<f32>,
    /// First 16 hex chars of the input SHA-256 — audit-safe.
    pub input_hash_prefix: String,
    pub e2e_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct AudioEncodeBody {
    /// Base64-encoded audio bytes.
    pub audio_b64: String,
    /// When `true`, the transcript is included in the response (the
    /// embedding is always returned). Defaults to `true`. PII bearing
    /// — caller is responsible for downstream redaction.
    #[serde(default = "default_true")]
    pub return_transcript: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct AudioEncodeResponse {
    pub model: &'static str,
    pub dim: usize,
    pub embedding: Vec<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    pub input_hash_prefix: String,
    pub duration_ms_estimate: u64,
    pub e2e_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct ChunkBody {
    pub text: String,
    #[serde(default)]
    pub tokens_per_chunk: Option<usize>,
    #[serde(default)]
    pub overlap_tokens: Option<usize>,
    #[serde(default)]
    pub max_chunks: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ChunkResponse {
    pub chunks: Vec<efe_multimodal::Chunk>,
}

#[derive(Debug, Deserialize)]
pub struct FuseBody {
    #[serde(default)]
    pub vision: Option<Vec<f32>>,
    #[serde(default)]
    pub audio: Option<Vec<f32>>,
    #[serde(default)]
    pub text: Option<Vec<f32>>,
    #[serde(default = "default_strategy")]
    pub strategy: String,
    #[serde(default = "default_weights")]
    pub weights: FuseWeights,
    #[serde(default = "default_target_dim")]
    pub target_dim: usize,
}

#[derive(Debug, Deserialize)]
pub struct FuseWeights {
    pub vision: f32,
    pub audio: f32,
    pub text: f32,
}

fn default_strategy() -> String {
    "concat_linear".to_string()
}
fn default_weights() -> FuseWeights {
    FuseWeights {
        vision: 1.0,
        audio: 1.0,
        text: 1.0,
    }
}
fn default_target_dim() -> usize {
    efe_multimodal::FUSION_TARGET_DIM
}

#[derive(Debug, Serialize)]
pub struct FuseResponse {
    pub dim: usize,
    pub embedding: Vec<f32>,
    pub strategy: String,
}

// ─────────────────────────── HANDLERS ───────────────────────────

/// `POST /v1/multimodal/vision/encode`
pub async fn vision_encode<S: MultimodalApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<VisionEncodeResponse>, MultimodalApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(MultimodalApiError::Unauthenticated)?;
    if !state.caller_has_multimodal_scope(&parts) {
        return Err(MultimodalApiError::Forbidden);
    }
    let body: VisionEncodeBody = decode_json_body(body).await?;
    let bytes = decode_b64(&body.image_b64)?;

    let policy = state.load_policy(&tenant).await?;
    let guard = policy
        .guard_for(GuardModality::Vision)
        .map_err(MultimodalApiError::Multimodal)?;
    let out = state.encode_vision(&bytes, &guard).await?;
    Ok(Json(VisionEncodeResponse {
        model: efe_multimodal::SIGLIP_MODEL_NAME,
        dim: out.embedding.len(),
        embedding: out.embedding,
        input_hash_prefix: efe_multimodal::input_hash_prefix(&out.input.sha256_hex, 16).to_string(),
        e2e_ms: out.e2e_ms,
    }))
}

/// `POST /v1/multimodal/audio/encode`
pub async fn audio_encode<S: MultimodalApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<AudioEncodeResponse>, MultimodalApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(MultimodalApiError::Unauthenticated)?;
    if !state.caller_has_multimodal_scope(&parts) {
        return Err(MultimodalApiError::Forbidden);
    }
    let body: AudioEncodeBody = decode_json_body(body).await?;
    let bytes = decode_b64(&body.audio_b64)?;

    let policy = state.load_policy(&tenant).await?;
    let guard = policy
        .guard_for(GuardModality::Audio)
        .map_err(MultimodalApiError::Multimodal)?;
    let out = state.encode_audio(&bytes, &guard).await?;
    Ok(Json(AudioEncodeResponse {
        model: efe_multimodal::WHISPER_MODEL_NAME,
        dim: out.embedding.len(),
        embedding: out.embedding,
        transcript: if body.return_transcript {
            Some(out.transcript)
        } else {
            None
        },
        input_hash_prefix: efe_multimodal::input_hash_prefix(&out.input.sha256_hex, 16).to_string(),
        duration_ms_estimate: out.input.duration_ms_estimate,
        e2e_ms: out.e2e_ms,
    }))
}

/// `POST /v1/multimodal/text/chunk`
pub async fn text_chunk<S: MultimodalApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<ChunkResponse>, MultimodalApiError> {
    let (parts, body) = req.into_parts();
    let _tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(MultimodalApiError::Unauthenticated)?;
    if !state.caller_has_multimodal_scope(&parts) {
        return Err(MultimodalApiError::Forbidden);
    }
    let body: ChunkBody = decode_json_body(body).await?;
    let cfg = ChunkConfig {
        tokens_per_chunk: body
            .tokens_per_chunk
            .unwrap_or(efe_multimodal::DEFAULT_TOKENS_PER_CHUNK),
        overlap_tokens: body.overlap_tokens.unwrap_or(64),
        max_chunks: body.max_chunks.unwrap_or(64),
    };
    let chunks = state.chunk_text(&body.text, &cfg).await?;
    Ok(Json(ChunkResponse { chunks }))
}

/// `POST /v1/multimodal/fuse`
pub async fn fuse<S: MultimodalApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<FuseResponse>, MultimodalApiError> {
    let (parts, body) = req.into_parts();
    let _tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(MultimodalApiError::Unauthenticated)?;
    if !state.caller_has_multimodal_scope(&parts) {
        return Err(MultimodalApiError::Forbidden);
    }
    let body: FuseBody = decode_json_body(body).await?;
    let strategy = parse_strategy(&body.strategy)?;
    let weights = ModalityWeights {
        vision: body.weights.vision,
        audio: body.weights.audio,
        text: body.weights.text,
    };
    let embedding = state
        .fuse(
            body.vision.as_deref(),
            body.audio.as_deref(),
            body.text.as_deref(),
            strategy,
            weights,
            body.target_dim,
        )
        .await?;
    Ok(Json(FuseResponse {
        dim: embedding.len(),
        embedding,
        strategy: body.strategy,
    }))
}

/// `GET /v1/multimodal/policy`
pub async fn get_policy<S: MultimodalApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<TenantModalityPolicyView>, MultimodalApiError> {
    let (parts, _body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(MultimodalApiError::Unauthenticated)?;
    let p = state.load_policy(&tenant).await?;
    Ok(Json(p.into()))
}

/// `PUT /v1/multimodal/policy`
pub async fn put_policy<S: MultimodalApiState>(
    State(state): State<S>,
    req: axum::extract::Request,
) -> Result<Json<TenantModalityPolicyView>, MultimodalApiError> {
    let (parts, body) = req.into_parts();
    let tenant = state
        .auth_tenant_from_request(&parts)
        .ok_or(MultimodalApiError::Unauthenticated)?;
    if !state.caller_has_multimodal_scope(&parts) {
        return Err(MultimodalApiError::Forbidden);
    }
    let view: TenantModalityPolicyView = decode_json_body(body).await?;
    let policy: TenantModalityPolicy = view.into_with_tenant(tenant.clone());
    state.save_policy(&tenant, &policy).await?;
    Ok(Json(policy.into()))
}

// ─────────────────────────── ROUTER ───────────────────────────

/// Build the `/v1/multimodal/*` sub-router.
pub fn router<S: MultimodalApiState>(state: S) -> Router {
    Router::new()
        .route("/vision/encode", post(vision_encode::<S>))
        .route("/audio/encode", post(audio_encode::<S>))
        .route("/text/chunk", post(text_chunk::<S>))
        .route("/fuse", post(fuse::<S>))
        .route("/policy", get(get_policy::<S>).put(put_policy::<S>))
        .with_state(state)
}

// ─────────────────────────── HELPERS ───────────────────────────

fn parse_strategy(s: &str) -> Result<FusionStrategy, MultimodalApiError> {
    match s {
        "concat_linear" => Ok(FusionStrategy::ConcatLinear),
        "weighted_mean" => Ok(FusionStrategy::WeightedMean),
        "clip_joint" => Ok(FusionStrategy::ClipJoint),
        other => Err(MultimodalApiError::InvalidBody(format!(
            "unknown fusion strategy: {other}"
        ))),
    }
}

async fn decode_json_body<T: for<'de> Deserialize<'de>>(
    body: axum::body::Body,
) -> Result<T, MultimodalApiError> {
    let bytes = axum::body::to_bytes(body, 64 * 1024 * 1024)
        .await
        .map_err(|e| MultimodalApiError::InvalidBody(format!("read body: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| MultimodalApiError::InvalidBody(format!("decode json: {e}")))
}

fn decode_b64(s: &str) -> Result<Vec<u8>, MultimodalApiError> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| MultimodalApiError::InvalidBody(format!("b64 decode: {e}")))
}

// ─────────────────────────── POLICY VIEW ───────────────────────────

/// Public view of a `TenantModalityPolicy` (no internal-only fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantModalityPolicyView {
    #[serde(default)]
    pub tenant_id: String,
    pub vision_enabled: bool,
    pub audio_enabled: bool,
    pub long_text_enabled: bool,
    pub max_vision_bytes: u64,
    pub max_audio_bytes: u64,
    pub max_long_text_bytes: u64,
    pub max_audio_ms: u64,
    pub allowed_vision_mimes: Vec<String>,
    pub allowed_audio_mimes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dpia_signed_at: Option<String>,
    /// Identifier of the operator / DPO who signed the DPIA. Surfaced
    /// in the public view for audit attribution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dpia_signed_by: Option<String>,
}

impl From<TenantModalityPolicy> for TenantModalityPolicyView {
    fn from(p: TenantModalityPolicy) -> Self {
        Self {
            tenant_id: p.tenant_id,
            vision_enabled: p.vision_enabled,
            audio_enabled: p.audio_enabled,
            long_text_enabled: p.long_text_enabled,
            max_vision_bytes: p.max_vision_bytes,
            max_audio_bytes: p.max_audio_bytes,
            max_long_text_bytes: p.max_long_text_bytes,
            max_audio_ms: p.max_audio_ms,
            allowed_vision_mimes: p
                .allowed_vision_mimes
                .iter()
                .map(|s| s.to_string())
                .collect(),
            allowed_audio_mimes: p
                .allowed_audio_mimes
                .iter()
                .map(|s| s.to_string())
                .collect(),
            dpia_signed_at: p.dpia_signed_at.map(|t| t.0),
            dpia_signed_by: p.dpia_signed_by,
        }
    }
}

impl TenantModalityPolicyView {
    /// Convert the view back into a `TenantModalityPolicy`, attributing
    /// the row to the given tenant.
    pub fn into_with_tenant(self, tenant_id: String) -> TenantModalityPolicy {
        TenantModalityPolicy {
            tenant_id,
            vision_enabled: self.vision_enabled,
            audio_enabled: self.audio_enabled,
            long_text_enabled: self.long_text_enabled,
            max_vision_bytes: self.max_vision_bytes,
            max_audio_bytes: self.max_audio_bytes,
            max_long_text_bytes: self.max_long_text_bytes,
            max_audio_ms: self.max_audio_ms,
            allowed_vision_mimes: stringify_allow_list(&self.allowed_vision_mimes, "vision"),
            allowed_audio_mimes: stringify_allow_list(&self.allowed_audio_mimes, "audio"),
            dpia_signed_at: self
                .dpia_signed_at
                .map(efe_multimodal::policy::chrono_like_iso8601::Iso8601::from_string),
            dpia_signed_by: self.dpia_signed_by,
        }
    }
}

/// Convert a JSON-supplied dynamic MIME list to a stable static-str
/// allow-list. Unknown MIME types are dropped (the caller observes
/// the rejection via subsequent encode-time `InvalidFormat` errors).
fn stringify_allow_list(input: &[String], kind: &str) -> Vec<&'static str> {
    let known_vision: &[&'static str] = &["image/jpeg", "image/png", "image/webp"];
    let known_audio: &[&'static str] = &["audio/wav", "audio/mpeg", "audio/flac"];
    let pool: &[&'static str] = match kind {
        "vision" => known_vision,
        "audio" => known_audio,
        _ => &[],
    };
    pool.iter()
        .copied()
        .filter(|s| input.iter().any(|v| v == *s))
        .collect()
}

#[cfg(test)]
mod tests {
    //! M30 audit fix : smoke tests that exercise the router wiring
    //! itself. They confirm the trait bounds + axum extractors
    //! compile correctly against a hand-rolled mock state, catching
    //! refactor regressions that wouldn't trip the per-module unit
    //! tests in `efe-multimodal`.

    use super::*;
    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use efe_multimodal::{
        AudioOutput, ChunkConfig, FusionStrategy, GuardPolicy, ModalityWeights,
        TenantModalityPolicy, VisionOutput,
    };
    use std::sync::Arc;
    use tower::ServiceExt; // for `.oneshot()`

    #[derive(Clone)]
    struct MockState {
        authed_tenant: Option<String>,
        scope_ok: bool,
        policy: Arc<TenantModalityPolicy>,
    }

    impl MockState {
        fn unauthed() -> Self {
            Self {
                authed_tenant: None,
                scope_ok: true,
                policy: Arc::new(TenantModalityPolicy::default()),
            }
        }
        fn authed_no_dpia() -> Self {
            Self {
                authed_tenant: Some("acme".into()),
                scope_ok: true,
                policy: Arc::new(TenantModalityPolicy {
                    vision_enabled: true,
                    audio_enabled: true,
                    dpia_signed_at: None,
                    dpia_signed_by: None,
                    ..TenantModalityPolicy::default()
                }),
            }
        }
    }

    #[async_trait]
    impl MultimodalApiState for MockState {
        fn auth_tenant_from_request(&self, _parts: &axum::http::request::Parts) -> Option<String> {
            self.authed_tenant.clone()
        }
        fn caller_has_multimodal_scope(&self, _parts: &axum::http::request::Parts) -> bool {
            self.scope_ok
        }
        async fn load_policy(
            &self,
            _tenant_id: &str,
        ) -> Result<TenantModalityPolicy, MultimodalApiError> {
            Ok((*self.policy).clone())
        }
        async fn save_policy(
            &self,
            _tenant_id: &str,
            _p: &TenantModalityPolicy,
        ) -> Result<(), MultimodalApiError> {
            Ok(())
        }
        async fn encode_vision(
            &self,
            _bytes: &[u8],
            _guard: &GuardPolicy,
        ) -> Result<VisionOutput, MultimodalApiError> {
            Err(MultimodalApiError::Internal("mock: not implemented".into()))
        }
        async fn encode_audio(
            &self,
            _bytes: &[u8],
            _guard: &GuardPolicy,
        ) -> Result<AudioOutput, MultimodalApiError> {
            Err(MultimodalApiError::Internal("mock: not implemented".into()))
        }
        async fn chunk_text(
            &self,
            _text: &str,
            _cfg: &ChunkConfig,
        ) -> Result<Vec<efe_multimodal::Chunk>, MultimodalApiError> {
            Ok(vec![])
        }
        async fn fuse(
            &self,
            _v: Option<&[f32]>,
            _a: Option<&[f32]>,
            _t: Option<&[f32]>,
            _s: FusionStrategy,
            _w: ModalityWeights,
            _target_dim: usize,
        ) -> Result<Vec<f32>, MultimodalApiError> {
            Ok(vec![0.0; 32])
        }
    }

    #[tokio::test]
    async fn router_builds_and_unauthed_returns_401() {
        let app = router(MockState::unauthed());
        let req = Request::builder()
            .method("POST")
            .uri("/vision/encode")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"image_b64":""}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn dpia_unsigned_vision_returns_403() {
        let app = router(MockState::authed_no_dpia());
        let body = serde_json::json!({"image_b64": "/9j/4AAQ"}); // tiny JPEG b64
        let req = Request::builder()
            .method("POST")
            .uri("/vision/encode")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn chunk_endpoint_authed_returns_200() {
        let app = router(MockState::authed_no_dpia());
        let body = serde_json::json!({"text": "hello world"});
        let req = Request::builder()
            .method("POST")
            .uri("/text/chunk")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
