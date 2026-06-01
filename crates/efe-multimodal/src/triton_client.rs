// SPDX-License-Identifier: UNLICENSED
//! M30 — Triton Inference Server HTTP client.
//!
//! M30 ships the **HTTP** variant of the Triton client. gRPC is
//! deferred to M30.1 (cf. ADR-M30-003 — HTTP is sufficient at the
//! anticipated MVP request volumes ≤ 50 RPS / region, and avoids a
//! protobuf dependency for now).
//!
//! The client targets the Triton V2 inference REST API :
//!
//! - `POST /v2/models/{model}/infer` → run inference
//! - `GET  /v2/health/ready`         → readiness probe
//! - `GET  /v2/health/live`          → liveness probe
//!
//! Reference :
//! <https://github.com/triton-inference-server/server/blob/main/docs/protocol/extension_kfserving.md>
//!
//! ## Concurrency model
//!
//! A single `TritonClient` wraps a `reqwest::Client` with HTTP/2 +
//! connection pooling. Cloning is cheap (Arc-of-pool internally).
//!
//! ## Timeout strategy
//!
//! Every call has a hard deadline (`request_timeout_ms`). When the
//! deadline is hit we emit `MultimodalError::TritonTimeout` and the
//! request is dropped on the client side (Triton will eventually
//! cancel the inference once its scheduler notices the connection is
//! closed).
//!
//! ## Retry strategy
//!
//! The client itself does NOT retry. Retries are the caller's
//! responsibility because retry semantics depend on whether the
//! inference is idempotent — which depends on the model, not on the
//! HTTP layer. The caller (typically `vision.rs` or `audio.rs`)
//! decides.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{MultimodalError, Result};
use crate::metrics::{
    MULTIMODAL_TRITON_DURATION_MS, MULTIMODAL_TRITON_ERRORS_TOTAL,
};

/// Configuration for the Triton HTTP client.
#[derive(Debug, Clone)]
pub struct TritonClientConfig {
    /// Base URL of the Triton server, e.g.
    /// `http://triton.efe-svc.svc.cluster.local:8000`.
    pub base_url: String,
    /// Per-request deadline in milliseconds.
    pub request_timeout_ms: u64,
    /// Connect timeout in milliseconds.
    pub connect_timeout_ms: u64,
    /// HTTP keep-alive duration in seconds.
    pub keep_alive_secs: u64,
    /// User agent string (audit-trail).
    pub user_agent: String,
}

impl Default for TritonClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://triton.efe-svc.svc.cluster.local:8000".into(),
            request_timeout_ms: 5_000,
            connect_timeout_ms: 500,
            keep_alive_secs: 60,
            user_agent: "efe-multimodal/0.0.1".into(),
        }
    }
}

/// Single tensor descriptor in a Triton inference request.
///
/// Triton expects exact shape + datatype. The supported datatypes
/// for M30 are `FP32` (vision/audio embeddings) and `INT64` (token
/// IDs for long-context text). Add more variants in M30.1 as
/// new model types come online.
#[derive(Debug, Clone, Serialize)]
pub struct TensorInput {
    /// Tensor name as defined in the model's `config.pbtxt`.
    pub name: String,
    /// Tensor shape, e.g. `[1, 3, 224, 224]` for a 224×224 RGB image.
    pub shape: Vec<i64>,
    /// Triton datatype, e.g. `"FP32"`, `"INT64"`, `"UINT8"`.
    pub datatype: String,
    /// Tensor payload. Triton accepts either inline JSON `data` (slow
    /// for big tensors) or binary attached via HTTP multipart. For M30
    /// we use inline JSON ; M30.1 adds binary for vision.
    pub data: Vec<f32>,
}

/// Single tensor descriptor in a Triton inference response.
#[derive(Debug, Clone, Deserialize)]
pub struct TensorOutput {
    /// Output name as defined in the model's `config.pbtxt`.
    pub name: String,
    /// Tensor shape returned by Triton.
    pub shape: Vec<i64>,
    /// Triton datatype.
    pub datatype: String,
    /// Decoded payload (always FP32 in M30 ; INT64 outputs are
    /// coerced via `as f32`).
    pub data: Vec<f32>,
}

/// Triton V2 inference request envelope.
#[derive(Debug, Clone, Serialize)]
pub struct InferRequest {
    /// Inference identifier (echoed back ; useful for correlating
    /// logs across the boundary).
    pub id: String,
    /// One or more input tensors.
    pub inputs: Vec<TensorInput>,
}

/// Triton V2 inference response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct InferResponse {
    /// Model name that handled the request.
    pub model_name: String,
    /// Model version (string in the wire protocol).
    pub model_version: String,
    /// Inference identifier echoed from the request.
    #[serde(default)]
    pub id: String,
    /// One or more output tensors.
    pub outputs: Vec<TensorOutput>,
}

/// Triton HTTP client.
#[derive(Debug, Clone)]
pub struct TritonClient {
    inner: Client,
    config: TritonClientConfig,
}

impl TritonClient {
    /// Build a new Triton client from the given configuration.
    ///
    /// Returns `Err(MultimodalError::Internal)` if the underlying
    /// `reqwest::Client` cannot be constructed (typically a TLS
    /// configuration issue in CI).
    pub fn new(config: TritonClientConfig) -> Result<Self> {
        let inner = Client::builder()
            .user_agent(&config.user_agent)
            .connect_timeout(Duration::from_millis(config.connect_timeout_ms))
            .pool_idle_timeout(Duration::from_secs(config.keep_alive_secs))
            .tcp_keepalive(Duration::from_secs(config.keep_alive_secs))
            // Triton's HTTP/1.1 server is the default ; if H2 is
            // enabled server-side it will negotiate via ALPN over TLS.
            .build()
            .map_err(|e| MultimodalError::Internal {
                reason: format!("reqwest build failed: {e}"),
            })?;
        Ok(Self { inner, config })
    }

    /// Run an inference. The caller supplies the model name (e.g.
    /// `siglip_base_patch16_224`) and the request envelope.
    ///
    /// Records latency in `MULTIMODAL_TRITON_DURATION_MS` and bumps
    /// `MULTIMODAL_TRITON_ERRORS_TOTAL` on transport / parse failures.
    pub async fn infer(&self, model: &str, modality: &str, req: &InferRequest) -> Result<InferResponse> {
        let url = format!("{}/v2/models/{}/infer", self.config.base_url, model);
        let started = Instant::now();
        let outcome_label;
        let resp_result = tokio::time::timeout(
            Duration::from_millis(self.config.request_timeout_ms),
            self.inner.post(&url).json(req).send(),
        )
        .await;

        // Timeout branch.
        let resp = match resp_result {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                MULTIMODAL_TRITON_ERRORS_TOTAL
                    .with_label_values(&[modality, model, "transport"])
                    .inc();
                warn!(error = %e, model = %model, "triton transport error");
                outcome_label = "err";
                MULTIMODAL_TRITON_DURATION_MS
                    .with_label_values(&[modality, model, outcome_label])
                    .observe(started.elapsed().as_millis() as f64);
                return Err(MultimodalError::TritonUnavailable { source: e });
            }
            Err(_elapsed) => {
                MULTIMODAL_TRITON_ERRORS_TOTAL
                    .with_label_values(&[modality, model, "timeout"])
                    .inc();
                warn!(model = %model, "triton request timeout");
                outcome_label = "err";
                MULTIMODAL_TRITON_DURATION_MS
                    .with_label_values(&[modality, model, outcome_label])
                    .observe(self.config.request_timeout_ms as f64);
                return Err(MultimodalError::TritonTimeout {
                    elapsed_ms: self.config.request_timeout_ms,
                });
            }
        };

        // Non-2xx response.
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let sanitized_msg = resp
                .text()
                .await
                .unwrap_or_else(|_| "no_body".into())
                .chars()
                .take(256) // cap body length in error
                .collect::<String>();
            MULTIMODAL_TRITON_ERRORS_TOTAL
                .with_label_values(&[modality, model, "bad_response"])
                .inc();
            outcome_label = "err";
            MULTIMODAL_TRITON_DURATION_MS
                .with_label_values(&[modality, model, outcome_label])
                .observe(started.elapsed().as_millis() as f64);
            return Err(MultimodalError::TritonBadResponse {
                status,
                sanitized_msg,
            });
        }

        // Parse body.
        let body: InferResponse = match resp.json().await {
            Ok(b) => b,
            Err(e) => {
                MULTIMODAL_TRITON_ERRORS_TOTAL
                    .with_label_values(&[modality, model, "bad_response"])
                    .inc();
                outcome_label = "err";
                MULTIMODAL_TRITON_DURATION_MS
                    .with_label_values(&[modality, model, outcome_label])
                    .observe(started.elapsed().as_millis() as f64);
                return Err(MultimodalError::TritonBadResponse {
                    status: 200,
                    sanitized_msg: format!("body decode: {e}"),
                });
            }
        };

        outcome_label = "ok";
        MULTIMODAL_TRITON_DURATION_MS
            .with_label_values(&[modality, model, outcome_label])
            .observe(started.elapsed().as_millis() as f64);
        debug!(
            model = %model,
            modality = %modality,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "triton infer ok"
        );
        Ok(body)
    }

    /// Probe the readiness endpoint. Returns `Ok(())` if Triton
    /// answered 200 within `request_timeout_ms`.
    pub async fn health_ready(&self) -> Result<()> {
        let url = format!("{}/v2/health/ready", self.config.base_url);
        let resp = tokio::time::timeout(
            Duration::from_millis(self.config.request_timeout_ms),
            self.inner.get(&url).send(),
        )
        .await
        .map_err(|_| MultimodalError::TritonTimeout {
            elapsed_ms: self.config.request_timeout_ms,
        })?
        .map_err(|e| MultimodalError::TritonUnavailable { source: e })?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(MultimodalError::TritonBadResponse {
                status: resp.status().as_u16(),
                sanitized_msg: "health/ready non-200".into(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_sane() {
        let c = TritonClientConfig::default();
        assert!(c.request_timeout_ms >= 1_000);
        assert!(c.connect_timeout_ms <= c.request_timeout_ms);
        assert!(c.user_agent.starts_with("efe-multimodal/"));
    }

    #[test]
    fn client_builds_with_defaults() {
        let _client = TritonClient::new(TritonClientConfig::default()).unwrap();
    }

    #[tokio::test]
    async fn infer_against_unreachable_returns_unavailable() {
        // Use an unbound port so the connect fails fast.
        let cfg = TritonClientConfig {
            base_url: "http://127.0.0.1:1".into(),
            request_timeout_ms: 200,
            connect_timeout_ms: 100,
            ..Default::default()
        };
        let client = TritonClient::new(cfg).unwrap();
        let req = InferRequest {
            id: "test-1".into(),
            inputs: vec![],
        };
        let err = client.infer("foo", "vision", &req).await.unwrap_err();
        // Either Unavailable (connect refused) or Timeout depending
        // on OS-level behavior — both are acceptable for this test.
        assert!(matches!(
            err,
            MultimodalError::TritonUnavailable { .. } | MultimodalError::TritonTimeout { .. }
        ));
    }
}
