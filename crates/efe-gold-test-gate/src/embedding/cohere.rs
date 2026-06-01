// SPDX-License-Identifier: UNLICENSED
//! **Sprint 12 — N20** : Real Cohere embedding provider — § 7.4 + G5.
//!
//! Uses `embed-multilingual-v3.0` by default ; 1024-dim vectors.
//!
//! API surface : `POST https://api.cohere.com/v1/embed` with body
//! `{"texts": ["..."], "model": "embed-multilingual-v3.0",
//! "input_type": "search_document"}` ; auth via `Authorization:
//! Bearer <api_key>`.

use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use efe_nano_sdk::ModelIdentity;
use serde::Deserialize;

use crate::embedding::{Embedding, EmbeddingProvider, EmbeddingProviderError};

pub const DEFAULT_COHERE_MODEL: &str = "embed-multilingual-v3.0";
pub const COHERE_DIMENSION: usize = 1024;
pub const DEFAULT_COHERE_ENDPOINT: &str = "https://api.cohere.com";

/// **N20** : default input-type tag.  Cohere distinguishes
/// `search_document` (the corpus side) from `search_query` (the
/// short query side).  Gold cases compare two outputs of similar
/// shape so `search_document` is the natural default.
pub const DEFAULT_INPUT_TYPE: &str = "search_document";

pub struct CohereEmbeddingProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    input_type: String,
    call_count: AtomicU64,
}

impl std::fmt::Debug for CohereEmbeddingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CohereEmbeddingProvider")
            .field("base_url", &self.base_url)
            .field("input_type", &self.input_type)
            .field("api_key_present", &!self.api_key.is_empty())
            .field("call_count", &self.call_count.load(Ordering::Relaxed))
            .finish()
    }
}

impl CohereEmbeddingProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_endpoint(api_key, DEFAULT_COHERE_ENDPOINT)
    }

    pub fn with_endpoint(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client construction"),
            api_key: api_key.into(),
            base_url: base_url.into(),
            input_type: DEFAULT_INPUT_TYPE.into(),
            call_count: AtomicU64::new(0),
        }
    }

    pub fn with_input_type(mut self, input_type: impl Into<String>) -> Self {
        self.input_type = input_type.into();
        self
    }

    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Deserialize)]
struct CohereResponse {
    embeddings: Vec<Vec<f64>>,
    #[serde(default)]
    response_type: Option<String>,
    #[serde(default)]
    meta: Option<CohereMeta>,
}

#[derive(Debug, Deserialize)]
struct CohereMeta {
    #[serde(default)]
    api_version: Option<CohereApiVersion>,
}

#[derive(Debug, Deserialize)]
struct CohereApiVersion {
    #[serde(default)]
    version: Option<String>,
}

#[async_trait]
impl EmbeddingProvider for CohereEmbeddingProvider {
    async fn embed(
        &self,
        model: &ModelIdentity,
        text: &str,
    ) -> Result<Embedding, EmbeddingProviderError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);

        let url = format!("{}/v1/embed", self.base_url.trim_end_matches('/'));
        let model_id = if model.model_id.is_empty() {
            DEFAULT_COHERE_MODEL
        } else {
            &model.model_id
        };
        let body = serde_json::json!({
            "texts": [text],
            "model": model_id,
            "input_type": self.input_type,
        });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbeddingProviderError::Backend {
                message: format!("cohere request : {e}"),
            })?;

        let status = resp.status();
        if status == 429 {
            return Err(EmbeddingProviderError::RateLimited);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(EmbeddingProviderError::Backend {
                message: format!("cohere status {status} : {body}"),
            });
        }

        // Cohere exposes the API version via `x-cohere-api-version`
        // or `meta.api_version.version` in the body.
        let header_version = resp
            .headers()
            .get("x-cohere-api-version")
            .or_else(|| resp.headers().get("x-cohere-model-version"))
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let parsed: CohereResponse = resp.json().await.map_err(|e| {
            EmbeddingProviderError::Backend {
                message: format!("cohere response parse : {e}"),
            }
        })?;

        if parsed.embeddings.is_empty() {
            return Err(EmbeddingProviderError::Backend {
                message: "cohere returned no embeddings".into(),
            });
        }

        let body_version = parsed
            .meta
            .as_ref()
            .and_then(|m| m.api_version.as_ref())
            .and_then(|v| v.version.clone());

        let served_version = header_version.or(body_version);

        let mut served_model = model.clone();
        if served_version.is_some() {
            served_model.served_version = served_version;
        }
        let _ = parsed.response_type;

        Ok(Embedding {
            model: served_model,
            vector: parsed.embeddings[0].clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn model() -> ModelIdentity {
        ModelIdentity::hosted("cohere", "embed-multilingual-v3.0", "1")
    }

    #[tokio::test]
    async fn cohere_happy_path_returns_embedding() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embed"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-cohere-api-version", "v3.0.1")
                    .set_body_json(serde_json::json!({
                        "id": "abc-123",
                        "texts": ["hello"],
                        "embeddings": [vec![0.2_f64; 1024]],
                        "meta": {"api_version": {"version": "v3.0.1"}}
                    })),
            )
            .mount(&server)
            .await;

        let p = CohereEmbeddingProvider::with_endpoint("test-key", server.uri());
        let e = p.embed(&model(), "hello").await.unwrap();
        assert_eq!(e.vector.len(), 1024);
        assert_eq!(e.model.served_version.as_deref(), Some("v3.0.1"));
    }

    #[tokio::test]
    async fn cohere_429_returns_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embed"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let p = CohereEmbeddingProvider::with_endpoint("k", server.uri());
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::RateLimited));
    }

    #[tokio::test]
    async fn cohere_500_returns_backend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embed"))
            .respond_with(ResponseTemplate::new(500).set_body_string("oops"))
            .mount(&server)
            .await;
        let p = CohereEmbeddingProvider::with_endpoint("k", server.uri());
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::Backend { .. }));
    }

    #[tokio::test]
    async fn cohere_empty_embeddings_returns_backend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "x",
                "texts": ["x"],
                "embeddings": []
            })))
            .mount(&server)
            .await;
        let p = CohereEmbeddingProvider::with_endpoint("k", server.uri());
        let err = p.embed(&model(), "x").await.unwrap_err();
        assert!(matches!(err, EmbeddingProviderError::Backend { .. }));
    }

    #[tokio::test]
    async fn cohere_call_count_increments() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embed"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "embeddings": [vec![0.0_f64; 1024]]
            })))
            .mount(&server)
            .await;
        let p = CohereEmbeddingProvider::with_endpoint("k", server.uri());
        let _ = p.embed(&model(), "x").await.unwrap();
        let _ = p.embed(&model(), "y").await.unwrap();
        assert_eq!(p.call_count(), 2);
    }

    #[tokio::test]
    async fn cohere_with_input_type_query() {
        // Verify we send the configured input_type ; wiremock matches
        // the request body.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embed"))
            .and(wiremock::matchers::body_partial_json(serde_json::json!({
                "input_type": "search_query"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "embeddings": [vec![0.0_f64; 1024]]
            })))
            .mount(&server)
            .await;
        let p = CohereEmbeddingProvider::with_endpoint("k", server.uri())
            .with_input_type("search_query");
        let _ = p.embed(&model(), "x").await.unwrap();
    }

    #[test]
    fn debug_format_does_not_leak_api_key() {
        let p = CohereEmbeddingProvider::new("secret-cohere-key");
        let dbg = format!("{p:?}");
        assert!(!dbg.contains("secret-cohere-key"));
        assert!(dbg.contains("api_key_present: true"));
    }
}
