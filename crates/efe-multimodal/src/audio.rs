// SPDX-License-Identifier: UNLICENSED
//! M30 — Audio encoder pipeline (Whisper-tiny).
//!
//! Flow :
//!
//! ```text
//! audio bytes ─► input_guard ─► resample-to-16k ─► triton_client ─► (transcript, [Vec<f32>; 384])
//! ```
//!
//! The Whisper-tiny encoder is hosted on Triton as a 2-output ensemble :
//!
//! - `OUTPUT_TRANSCRIPT` (BYTES) — UTF-8 string. **Always passes
//!   through the M29 PII redactor downstream** ; this crate must never
//!   log the raw transcript.
//! - `OUTPUT_EMBEDDING` (FP32, 384) — encoder hidden-state pooled
//!   across time. L2-normalized client-side.
//!
//! ## Resampling
//!
//! M30 ships a **byte-rate-only** resampling check : if the WAV
//! header advertises a sample rate other than 16 kHz, we reject with
//! `EncodeFailed` and let the caller convert (typically via ffmpeg
//! in their proxy). A full software resampler is deferred to M30.1
//! (cf. ADR-M30-004).

use std::time::Instant;

use tracing::debug;

use crate::error::{MultimodalError, Result};
use crate::input_guard::{input_hash_prefix, GuardModality, GuardPolicy, InputGuard, InputMetadata};
use crate::metrics::{
    InflightGuard, MULTIMODAL_E2E_DURATION_MS, MULTIMODAL_INPUT_SIZE_BYTES,
    MULTIMODAL_REQUESTS_TOTAL,
};
use crate::triton_client::{InferRequest, TensorInput, TritonClient};
use crate::vision::l2_normalize;

/// Whisper-tiny pooled-encoder embedding dimension.
pub const WHISPER_EMBED_DIM: usize = 384;

/// Default Triton model name for the Whisper-tiny audio encoder.
pub const WHISPER_MODEL_NAME: &str = "whisper_tiny_encoder";

/// Required sample rate for the Whisper Triton ensemble.
pub const WHISPER_TARGET_SAMPLE_RATE: u32 = 16_000;

/// Result of an audio encode call.
#[derive(Debug, Clone)]
pub struct AudioOutput {
    /// UTF-8 transcript returned by Whisper. **PII-bearing** — caller
    /// must redact before logging or persisting.
    pub transcript: String,
    /// 384-dimensional L2-normalized embedding.
    pub embedding: Vec<f32>,
    /// Audit-safe metadata about the input (includes the SHA-256 of
    /// the raw bytes ; the transcript SHA-256 is computed separately).
    pub input: InputMetadata,
    /// Wall-clock latency in milliseconds.
    pub e2e_ms: u64,
}

/// Audio encoder.
#[derive(Debug, Clone)]
pub struct AudioEncoder {
    triton: TritonClient,
    model_name: String,
}

impl AudioEncoder {
    /// Build a new audio encoder from an already-constructed Triton
    /// client.
    pub fn new(triton: TritonClient) -> Self {
        Self {
            triton,
            model_name: WHISPER_MODEL_NAME.to_string(),
        }
    }

    /// Override the Triton model name (useful for canary versions).
    pub fn with_model_name(mut self, name: impl Into<String>) -> Self {
        self.model_name = name.into();
        self
    }

    /// Encode a single audio clip.
    pub async fn encode(&self, audio_bytes: &[u8], policy: &GuardPolicy) -> Result<AudioOutput> {
        let _guard = InflightGuard::new("audio");
        let started = Instant::now();

        // 1) Input guard.
        let meta = InputGuard::inspect(GuardModality::Audio, audio_bytes, policy)?;
        MULTIMODAL_INPUT_SIZE_BYTES
            .with_label_values(&["audio"])
            .observe(meta.size_bytes as f64);

        // 2) Sample rate check (WAV-only ; lossy formats are decoded
        //    server-side inside the Triton ensemble).
        if meta.detected_mime == "audio/wav" {
            let sr = read_wav_sample_rate(audio_bytes).ok_or(MultimodalError::EncodeFailed {
                reason: "wav sample_rate parse".into(),
            })?;
            if sr != WHISPER_TARGET_SAMPLE_RATE {
                return Err(MultimodalError::EncodeFailed {
                    reason: format!(
                        "sample rate {} != target {} (M30.1 will add software resampler)",
                        sr, WHISPER_TARGET_SAMPLE_RATE
                    ),
                });
            }
        }
        debug!(
            sha256 = %input_hash_prefix(&meta.sha256_hex, 16),
            mime = %meta.detected_mime,
            duration_ms = meta.duration_ms_estimate,
            "audio encode start"
        );

        // 3) Build Triton request — widen bytes to f32 (same caveat as
        //    vision : M30.1 switches to binary attachments).
        let data: Vec<f32> = audio_bytes.iter().map(|&b| b as f32).collect();
        let inputs = vec![TensorInput {
            name: "INPUT_AUDIO_RAW".into(),
            shape: vec![1, audio_bytes.len() as i64],
            datatype: "FP32".into(),
            data,
        }];
        let req = InferRequest {
            id: format!("audio-{}", input_hash_prefix(&meta.sha256_hex, 16)),
            inputs,
        };

        // 4) Triton call.
        let outcome_label;
        let resp = match self.triton.infer(&self.model_name, "audio", &req).await {
            Ok(r) => {
                outcome_label = "ok";
                r
            }
            Err(e) => {
                outcome_label = "err";
                MULTIMODAL_REQUESTS_TOTAL
                    .with_label_values(&["audio", outcome_label])
                    .inc();
                MULTIMODAL_E2E_DURATION_MS
                    .with_label_values(&["audio", outcome_label])
                    .observe(started.elapsed().as_millis() as f64);
                return Err(e);
            }
        };

        // 5) Validate outputs.
        if resp.outputs.len() < 2 {
            MULTIMODAL_REQUESTS_TOTAL
                .with_label_values(&["audio", "err"])
                .inc();
            return Err(MultimodalError::EncodeFailed {
                reason: format!(
                    "whisper response expected 2 outputs, got {}",
                    resp.outputs.len()
                ),
            });
        }
        // Convention : embedding first, transcript second. Lookup by
        // name when present so the order is not load-bearing.
        let (mut embedding, transcript) = match (
            resp.outputs.iter().find(|t| t.name == "OUTPUT_EMBEDDING"),
            resp.outputs.iter().find(|t| t.name == "OUTPUT_TRANSCRIPT"),
        ) {
            (Some(emb), Some(tx)) => {
                let emb_data = emb.data.clone();
                let tx_str = decode_transcript_tensor(tx)?;
                (emb_data, tx_str)
            }
            _ => {
                // Fall back to positional decoding.
                let emb_data = resp.outputs[0].data.clone();
                let tx_str = decode_transcript_tensor(&resp.outputs[1])?;
                (emb_data, tx_str)
            }
        };
        if embedding.len() != WHISPER_EMBED_DIM {
            return Err(MultimodalError::EncodeFailed {
                reason: format!(
                    "whisper embedding dim mismatch: got {}, expected {}",
                    embedding.len(),
                    WHISPER_EMBED_DIM
                ),
            });
        }
        l2_normalize(&mut embedding);

        MULTIMODAL_REQUESTS_TOTAL
            .with_label_values(&["audio", outcome_label])
            .inc();
        let e2e_ms = started.elapsed().as_millis() as u64;
        MULTIMODAL_E2E_DURATION_MS
            .with_label_values(&["audio", outcome_label])
            .observe(e2e_ms as f64);

        Ok(AudioOutput {
            transcript,
            embedding,
            input: meta,
            e2e_ms,
        })
    }
}

/// Read the WAV header `sample_rate` field (little-endian u32 at
/// offset 24). Returns `None` if the header is too short.
fn read_wav_sample_rate(bytes: &[u8]) -> Option<u32> {
    if bytes.len() < 28 {
        return None;
    }
    Some(u32::from_le_bytes([
        bytes[24], bytes[25], bytes[26], bytes[27],
    ]))
}

/// Decode the transcript tensor. Whisper emits the transcript as a
/// BYTES tensor — in the HTTP variant Triton encodes this as a
/// JSON array of f32 representing the UTF-8 codepoints (M30 hack).
/// M30.1 will use a dedicated BYTES tensor.
fn decode_transcript_tensor(t: &crate::triton_client::TensorOutput) -> Result<String> {
    let raw: Vec<u8> = t.data.iter().map(|&x| x as u8).collect();
    String::from_utf8(raw).map_err(|e| MultimodalError::EncodeFailed {
        reason: format!("transcript utf-8 decode: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_wav_sample_rate_extracts_field() {
        let mut bytes = vec![0u8; 28];
        bytes[24..28].copy_from_slice(&16_000u32.to_le_bytes());
        assert_eq!(read_wav_sample_rate(&bytes), Some(16_000));
    }

    #[test]
    fn read_wav_sample_rate_short_header_returns_none() {
        assert_eq!(read_wav_sample_rate(&[0, 1, 2]), None);
    }

    #[test]
    fn decode_transcript_tensor_roundtrips_ascii() {
        let s = "hello world";
        let t = crate::triton_client::TensorOutput {
            name: "OUTPUT_TRANSCRIPT".into(),
            shape: vec![s.len() as i64],
            datatype: "FP32".into(),
            data: s.bytes().map(|b| b as f32).collect(),
        };
        let got = decode_transcript_tensor(&t).unwrap();
        assert_eq!(got, s);
    }
}
