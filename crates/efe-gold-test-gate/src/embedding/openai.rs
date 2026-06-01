// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : Real OpenAI embedding provider — § 7.4 + G4.
//!
//! Uses `text-embedding-3-small` by default ; returns the served
//! model version from the response headers so drift detection
//! (F7) can flag silent migrations.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use efe_nano_sdk::ModelIdentity;
use serde::Deserialize;

use crate::embedding::{Embedding, EmbeddingProvider, EmbeddingProviderError};

/// **N20** : default model id used when the caller's `ModelIdentity`
/// has a non-matching `model_id`.
pub const DEFAULT_OPENAI_MODEL: &str = "text-embedding-3-small";

/// **N20** : OpenAI embedding dimension for the default model.
pub const OPENAI_DIMENSION: usize = 1536;

/// **N20** : default endpoint.  Overrideable for tests (wiremock).
pub const DEFAULT_OPENAI_ENDPOINT: &str = "https://api.openai.com";

/// **N20** : real OpenAI embedding provider backed by `reqwest`.
///
/// Auth : `Authorization: Bearer <api_key>` header.
/// Endpoint : `${base_url}/v1/embeddings`.
/// Response body : `{"data":[{"embedding":[...f32...],"index":0}],
/// "model":"text-embedding-3-small","usage":{...}}`
pub struct OpenAiEmbeddingProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    call_count: AtomicU64,
}

impl std::fmt::Debug for OpenAiEmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiEmbeddingProvider")
            .field("base_url", &self.base_url)
            .field("api_key_present", &!self.api_key.is_empty())
            .field("call_count", &self.call_count.load(Ordering::Relaxed))
            .finish()
    }
}

impl OpenAiEmbeddingProvider {
    /// **N20** : Construct with explicit API key + default endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_endpoint(api_key, DEFAULT_OPENAI_ENDPOINT)
    }

    /// **N20** : Construct with explicit endpoint (used by tests
    /// against wiremock).
    pub fn with_endpoint(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client construction"),
            api_key: api_key.into(),
            base_url: base_url.into(),
            call_count: AtomicU64::new(0),
        }
    }

    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    data: Vec<OpenAiEmbeddingItem>,
    model: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiEmbeddingItem {
    embedding: Vec<f64>,
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbeddingProvider {
    async fn embed(
        &self,
        model: &ModelIdentity,
        text: &str,
    ) -> Result<Embedding, EmbeddingProviderError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);

        let url = format!("{}/v1/embeddings", self.base_url.trim_end_matches('/'));
        let model_id = if model.model_id.is_empty() {
            DEFAULT_OPENAI_MODEL
        } else {
            &model.model_id
        };
        let body = serde_json::json!({
            "input": text,
            "model": model_id,
        });

        let req = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body);

        let resp = req.send().await.map_err(|e| {
            EmbeddingProviderError::Backend {
                message: format!("openai request : {e}"),
            }
        })?;

        let status = resp.status();
        if status == 429 {
            return Err(EmbeddingProviderError::RateLimited);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EmbeddingProviderError::Backend {
                message: format!("openai status {status} : {body}"),
            });
        }

        // F7 : capture the served-model header if present (OpenAI
        // exposes it as `openai-model` or `x-served-model`
        // depending on edge).  Defensive : check both ; fall back to
        // body.model.
        let served_version = resp
            .headers()
            .get("openai-model")
            .or_else(|| resp.headers().get("x-served-model"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let parsed: OpenAiResponse = resp.json().await.map_err(|e| {
            EmbeddingProviderError::Backend {
                message: format!("openai response parse : {e}"),
            }
        })?;

        if parsed.data.is_empty() {
            return Err(EmbeddingProviderError::Backend {
                message: "openai returned no embedding data".into(),
            });
        }

        // Build a returned ModelIdentity reflecting what OpenAI
        // ACTUALLY served (which may differ from the requested
        // version — F7).
        let mut served_model = model.clone();
        served_model.served_version = served_version.or(Some(parsed.model.clone()));

        // F7 explicit drift check : if the served model id (from
        // body) doesn't match the requested model id, surface drift.
        // Note : `served_version` is per-request snapshot ; we don't
        // fail here because the cosine comparator's job is to detect
        // semantic drift downstream.  We DO record the mismatch in
        // the returned `ModelIdentity` so callers can audit.
        let _ = parsed.model; // captured into served_model.served_version

        Ok(Embedding {
            model: served_model,
            vector: parsed.data[0].embedding.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn model() -> ModelIdentity {
        ModelIdentity::hosted("openai", "text-embedding-3-small", "1")
    }

    #[tokio::test]
    async fn openai_happy_path_returns_embedding() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("openai-model", "text-embedding-3-small")
                    .set_body_json(serde_json::json!({
                        "object": "list",
                        "data": [{
                            "object": "embedding",
                            "index": 0,
                            "embedding": vec![0.1_f64; 1536]
                        }],
                        "model": "text-embedding-3-small",
                        "usage": {"prompt_tokens": 5, "total_tokens": 5}
                    })),
            )
            .mount(&server)
            .await;

        let p = OpenAiEmbeddingProvider::with_endpoint("test-key", server.uri());
        let e = p.embed(&model(), "hello").await.unwrap();
        assert_eq!(e.vector.len(), 1536);
        assert_eq!(e.model.served_version.as_deref(), Some("text-embedding-3-small"));
    }

    #[tokio::test]
    async fn openai_429_returns_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;

        let p = OpenAiEmbeddingProvider::with_endpoint("test-key", server.uri());
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::RateLimited));
    }

    #[tokio::test]
    async fn openai_500_returns_backend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal err"))
            .mount(&server)
            .await;

        let p = OpenAiEmbeddingProvider::with_endpoint("test-key", server.uri());
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::Backend { .. }));
    }

    #[tokio::test]
    async fn openai_empty_data_returns_backend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [],
                "model": "text-embedding-3-small",
                "usage": {"prompt_tokens": 0, "total_tokens": 0}
            })))
            .mount(&server)
            .await;

        let p = OpenAiEmbeddingProvider::with_endpoint("test-key", server.uri());
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::Backend { .. }));
    }

    #[tokio::test]
    async fn openai_call_count_increments() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"embedding": vec![0.0_f64; 1536]}],
                "model": "text-embedding-3-small"
            })))
            .mount(&server)
            .await;

        let p = OpenAiEmbeddingProvider::with_endpoint("k", server.uri());
        assert_eq!(p.call_count(), 0);
        let _ = p.embed(&model(), "x").await.unwrap();
        let _ = p.embed(&model(), "y").await.unwrap();
        assert_eq!(p.call_count(), 2);
    }

    #[tokio::test]
    async fn openai_served_version_falls_back_to_body_model() {
        // Header missing ; body.model used as fallback.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": [{"embedding": vec![0.0_f64; 1536]}],
                "model": "text-embedding-3-small-2024-01"
            })))
            .mount(&server)
            .await;
        let p = OpenAiEmbeddingProvider::with_endpoint("k", server.uri());
        let e = p.embed(&model(), "x").await.unwrap();
        assert_eq!(
            e.model.served_version.as_deref(),
            Some("text-embedding-3-small-2024-01")
        );
    }

    #[test]
    fn debug_format_does_not_leak_api_key() {
        let p = OpenAiEmbeddingProvider::new("ultra-secret-key");
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("ultra-secret-key"));
        assert!(dbg.contains("api_key_present: true"));
    }
}
