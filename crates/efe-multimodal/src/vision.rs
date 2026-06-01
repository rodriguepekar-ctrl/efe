// SPDX-License-Identifier: UNLICENSED
//! M30 — Vision encoder pipeline (SigLIP-base-patch16-224).
//!
//! Flow :
//!
//! ```text
//! image bytes ─► input_guard ─► preprocess ─► triton_client ─► [Vec<f32>; 768]
//! ```
//!
//! Preprocessing is **pure on-CPU** (no GPU dependency) :
//!
//! - Resize to 224×224 with bilinear interpolation (handled
//!   server-side at M30 for now ; client only forwards raw RGB bytes
//!   via the `siglip_preprocess` Triton ensemble step in M30.1).
//! - Mean/std normalization is performed by the SigLIP preprocessing
//!   ensemble step inside Triton ; the client passes raw `UINT8` RGB
//!   bytes when available, or a pre-normalized `FP32` tensor when the
//!   caller has already preprocessed (rare).
//!
//! For M30 (HTTP, no binary tensors) we send the **raw image bytes**
//! Base64-encoded in a single `INPUT_IMAGE_RAW` FP32 tensor where each
//! byte is widened to f32. M30.1 will move to binary attachments.

use std::time::Instant;

use tracing::debug;

use crate::error::{MultimodalError, Result};
use crate::input_guard::{input_hash_prefix, GuardModality, GuardPolicy, InputGuard, InputMetadata};
use crate::metrics::{
    InflightGuard, MULTIMODAL_E2E_DURATION_MS, MULTIMODAL_INPUT_SIZE_BYTES, MULTIMODAL_REQUESTS_TOTAL,
};
use crate::triton_client::{InferRequest, TensorInput, TritonClient};

/// SigLIP-base-patch16-224 produces 768-dimensional embeddings.
pub const SIGLIP_EMBED_DIM: usize = 768;

/// Default Triton model name for the SigLIP vision encoder.
/// Matches the model directory under `/models/siglip_base_patch16_224`.
pub const SIGLIP_MODEL_NAME: &str = "siglip_base_patch16_224";

/// Result of a vision encode call.
#[derive(Debug, Clone)]
pub struct VisionOutput {
    /// 768-dimensional L2-normalized embedding.
    pub embedding: Vec<f32>,
    /// Audit-safe metadata about the input.
    pub input: InputMetadata,
    /// Wall-clock latency in milliseconds (preprocessing + Triton +
    /// postprocessing — measured at the encoder boundary).
    pub e2e_ms: u64,
}

/// Vision encoder.
///
/// Thin wrapper around `TritonClient` that adds :
///
/// - input guard invocation
/// - tensor shape validation
/// - L2 normalization on the way back (idempotent — if SigLIP already
///   normalized server-side, this is a no-op)
/// - metric emission
#[derive(Debug, Clone)]
pub struct VisionEncoder {
    triton: TritonClient,
    model_name: String,
}

impl VisionEncoder {
    /// Build a new vision encoder from an already-constructed Triton
    /// client.
    pub fn new(triton: TritonClient) -> Self {
        Self {
            triton,
            model_name: SIGLIP_MODEL_NAME.to_string(),
        }
    }

    /// Override the Triton model name (useful for canary versions).
    pub fn with_model_name(mut self, name: impl Into<String>) -> Self {
        self.model_name = name.into();
        self
    }

    /// Encode a single image. Performs input-guard + preprocess +
    /// Triton call + L2 normalize.
    pub async fn encode(&self, image_bytes: &[u8], policy: &GuardPolicy) -> Result<VisionOutput> {
        let _guard = InflightGuard::new("vision");
        let started = Instant::now();

        // 1) Input guard.
        let meta = InputGuard::inspect(GuardModality::Vision, image_bytes, policy)?;
        MULTIMODAL_INPUT_SIZE_BYTES
            .with_label_values(&["vision"])
            .observe(meta.size_bytes as f64);
        debug!(
            sha256 = %input_hash_prefix(&meta.sha256_hex, 16),
            mime = %meta.detected_mime,
            size_bytes = meta.size_bytes,
            "vision encode start"
        );

        // 2) Preprocess : widen bytes to f32. M30.1 will swap this for
        //    a Triton binary attachment ; for now we use inline JSON
        //    since the request volume is modest.
        let data: Vec<f32> = image_bytes.iter().map(|&b| b as f32).collect();
        let inputs = vec![TensorInput {
            name: "INPUT_IMAGE_RAW".into(),
            shape: vec![1, image_bytes.len() as i64],
            datatype: "FP32".into(),
            data,
        }];
        let req = InferRequest {
            id: format!("vision-{}", input_hash_prefix(&meta.sha256_hex, 16)),
            inputs,
        };

        // 3) Triton call.
        let outcome_label;
        let resp = match self.triton.infer(&self.model_name, "vision", &req).await {
            Ok(r) => {
                outcome_label = "ok";
                r
            }
            Err(e) => {
                outcome_label = "err";
                MULTIMODAL_REQUESTS_TOTAL
                    .with_label_values(&["vision", outcome_label])
                    .inc();
                MULTIMODAL_E2E_DURATION_MS
                    .with_label_values(&["vision", outcome_label])
                    .observe(started.elapsed().as_millis() as f64);
                return Err(e);
            }
        };

        // 4) Validate + L2 normalize.
        let embedding = extract_and_normalize(&resp, SIGLIP_EMBED_DIM)?;

        MULTIMODAL_REQUESTS_TOTAL
            .with_label_values(&["vision", outcome_label])
            .inc();
        let e2e_ms = started.elapsed().as_millis() as u64;
        MULTIMODAL_E2E_DURATION_MS
            .with_label_values(&["vision", outcome_label])
            .observe(e2e_ms as f64);

        Ok(VisionOutput {
            embedding,
            input: meta,
            e2e_ms,
        })
    }
}

/// Extract the first output tensor of expected dimension and L2-normalize it.
fn extract_and_normalize(
    resp: &crate::triton_client::InferResponse,
    expected_dim: usize,
) -> Result<Vec<f32>> {
    let out = resp.outputs.first().ok_or(MultimodalError::EncodeFailed {
        reason: "triton response had no outputs".into(),
    })?;
    if out.data.len() != expected_dim {
        return Err(MultimodalError::EncodeFailed {
            reason: format!(
                "embedding dim mismatch: got {}, expected {}",
                out.data.len(),
                expected_dim
            ),
        });
    }
    let mut v = out.data.clone();
    l2_normalize(&mut v);
    Ok(v)
}

/// In-place L2 normalization. If the L2 norm is < 1e-12 the vector is
/// left untouched (avoids NaN explosion on adversarial zero-vectors).
pub fn l2_normalize(v: &mut [f32]) {
    let norm_sq: f32 = v.iter().map(|x| x * x).sum();
    let norm = norm_sq.sqrt();
    if norm < 1e-12 {
        return;
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_unit_vector() {
        let mut v = vec![3.0_f32, 4.0, 0.0];
        l2_normalize(&mut v);
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
        assert!((v[0] - 0.6).abs() < 1e-5);
        assert!((v[1] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn l2_normalize_zero_vector_stays_zero() {
        let mut v = vec![0.0_f32; 4];
        l2_normalize(&mut v);
        assert_eq!(v, vec![0.0; 4]);
    }

    #[test]
    fn extract_and_normalize_rejects_dim_mismatch() {
        let resp = crate::triton_client::InferResponse {
            model_name: "x".into(),
            model_version: "1".into(),
            id: "y".into(),
            outputs: vec![crate::triton_client::TensorOutput {
                name: "o".into(),
                shape: vec![1, 10],
                datatype: "FP32".into(),
                data: vec![0.0; 10],
            }],
        };
        let err = extract_and_normalize(&resp, 768).unwrap_err();
        assert!(matches!(err, MultimodalError::EncodeFailed { .. }));
    }

    #[test]
    fn extract_and_normalize_succeeds() {
        let mut data = vec![0.0_f32; SIGLIP_EMBED_DIM];
        data[0] = 1.0;
        data[1] = 1.0;
        let resp = crate::triton_client::InferResponse {
            model_name: "x".into(),
            model_version: "1".into(),
            id: "y".into(),
            outputs: vec![crate::triton_client::TensorOutput {
                name: "o".into(),
                shape: vec![1, SIGLIP_EMBED_DIM as i64],
                datatype: "FP32".into(),
                data,
            }],
        };
        let out = extract_and_normalize(&resp, SIGLIP_EMBED_DIM).unwrap();
        assert_eq!(out.len(), SIGLIP_EMBED_DIM);
        let n: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
    }
}
