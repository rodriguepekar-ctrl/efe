// SPDX-License-Identifier: UNLICENSED
//! M30 — Multimodal error taxonomy.
//!
//! All public functions of the `efe-multimodal` crate return
//! `Result<T, MultimodalError>` where `MultimodalError` is the single
//! canonical error type. Per M30 ADR-M30-002, the variants form a flat
//! enum with a stable string identifier (the `error_code()` method) so
//! that REST handlers in `efe-server` can deterministically map errors
//! to HTTP status codes without inspecting `Debug` strings.
//!
//! Mapping table (consumed by `v1/multimodal/error.rs`) :
//!
//! | Variant                | error_code()                  | HTTP |
//! |------------------------|-------------------------------|------|
//! | `InputTooLarge`        | `multimodal.input_too_large`  | 413  |
//! | `InvalidFormat`        | `multimodal.invalid_format`   | 415  |
//! | `InvalidDuration`      | `multimodal.invalid_duration` | 422  |
//! | `ModalityDisabled`     | `multimodal.modality_disabled`| 403  |
//! | `TritonUnavailable`    | `multimodal.triton_unavail`   | 503  |
//! | `TritonTimeout`        | `multimodal.triton_timeout`   | 504  |
//! | `TritonBadResponse`    | `multimodal.triton_bad_resp`  | 502  |
//! | `EncodeFailed`         | `multimodal.encode_failed`    | 500  |
//! | `FusionDimMismatch`    | `multimodal.fusion_dim_mismatch`| 422 |
//! | `ChunkOverflow`        | `multimodal.chunk_overflow`   | 422  |
//! | `Internal`             | `multimodal.internal`         | 500  |
//!
//! **Logging rule** : raw multimodal input (image bytes, audio bytes,
//! transcripts) MUST NEVER appear in error messages. Use SHA-256
//! prefixes (first 16 hex chars) via the `input_hash` helper in
//! `crate::input_guard`. This is enforced by the QA guardrail in
//! `crates/efe-pii::redactor`.

use thiserror::Error;

/// Canonical multimodal error type.
///
/// Variants are designed to map 1:1 to a stable string identifier
/// returned by [`MultimodalError::error_code`]. Callers must NEVER
/// match on the `Display` representation — only on the variant or on
/// `error_code()`.
#[derive(Debug, Error)]
pub enum MultimodalError {
    /// Input bytes exceed the per-modality `max_bytes` configured for
    /// the tenant. Carries the observed size and the configured cap.
    #[error("input too large: {observed_bytes} bytes > cap {cap_bytes} bytes")]
    InputTooLarge {
        /// Observed byte count of the input.
        observed_bytes: u64,
        /// Configured cap for the tenant + modality.
        cap_bytes: u64,
    },

    /// Magic-byte sniffing failed or yielded a MIME type that is not on
    /// the per-modality allow-list (e.g. `image/jpeg`, `image/png` for
    /// vision ; `audio/wav`, `audio/mpeg` for audio).
    #[error("invalid format: detected {detected_mime}, allowed {allowed_mimes:?}")]
    InvalidFormat {
        /// MIME type detected from magic bytes.
        detected_mime: String,
        /// Allowed MIME types for the modality.
        allowed_mimes: Vec<&'static str>,
    },

    /// Audio duration violates the per-tenant cap. Carries the observed
    /// duration in milliseconds and the configured cap.
    #[error("invalid duration: {observed_ms} ms > cap {cap_ms} ms")]
    InvalidDuration {
        /// Observed audio duration in milliseconds.
        observed_ms: u64,
        /// Configured cap in milliseconds.
        cap_ms: u64,
    },

    /// The tenant's modality policy disables this modality (e.g. vision
    /// is off, or all multimodal endpoints are off because the tenant
    /// has not signed the AI Act Art. 9 DPIA addendum).
    #[error("modality disabled for tenant: {modality}")]
    ModalityDisabled {
        /// Name of the modality (`vision`, `audio`, `long_text`).
        modality: &'static str,
    },

    /// Triton Inference Server returned a transport-level error (TCP
    /// reset, DNS failure, refusal). Maps to HTTP 503.
    #[error("triton unavailable: {source}")]
    TritonUnavailable {
        /// Underlying transport error.
        #[source]
        source: reqwest::Error,
    },

    /// Triton call exceeded the per-request deadline. Maps to HTTP 504.
    #[error("triton timeout after {elapsed_ms} ms")]
    TritonTimeout {
        /// Elapsed wall-clock time in milliseconds.
        elapsed_ms: u64,
    },

    /// Triton returned a non-2xx response or the response body could not
    /// be decoded into the expected output tensor shape. Carries the
    /// HTTP status code and a sanitized error message (no PII).
    #[error("triton bad response: status {status}, msg {sanitized_msg}")]
    TritonBadResponse {
        /// HTTP status code returned by Triton.
        status: u16,
        /// Sanitized error message (no raw input bytes).
        sanitized_msg: String,
    },

    /// Encoder (vision or audio) failed in a deterministic way (e.g.
    /// image decoder rejected the bytes, audio resampler failed). This
    /// is distinct from `TritonBadResponse` because it indicates a bug
    /// in our preprocessor, not in Triton.
    #[error("encode failed: {reason}")]
    EncodeFailed {
        /// Sanitized reason — no raw input bytes.
        reason: String,
    },

    /// Fusion received vectors with incompatible dimensions for the
    /// chosen strategy (e.g. `WeightedMean` requires all inputs to have
    /// the same dim ; `ConcatLinear` is more permissive but still has
    /// a max joint dim of 4096).
    #[error("fusion dim mismatch: vision={vision_dim}, audio={audio_dim}, text={text_dim}, strategy={strategy}")]
    FusionDimMismatch {
        /// Vision embedding dimension (0 if absent).
        vision_dim: usize,
        /// Audio embedding dimension (0 if absent).
        audio_dim: usize,
        /// Text embedding dimension (0 if absent).
        text_dim: usize,
        /// Fusion strategy name.
        strategy: &'static str,
    },

    /// Long-context chunker produced more chunks than the per-request
    /// cap (default 64). This is typically a bug or an adversarial
    /// input — surfaced as 422 so the caller can split the request.
    #[error("chunk overflow: produced {produced}, cap {cap}")]
    ChunkOverflow {
        /// Number of chunks produced before the cap was hit.
        produced: usize,
        /// Configured per-request chunk cap.
        cap: usize,
    },

    /// Catch-all for unexpected internal errors. Should never be
    /// surfaced in steady state ; trigger a P3 incident if observed
    /// more than 0.01 % of requests.
    #[error("internal: {reason}")]
    Internal {
        /// Sanitized reason — no raw input bytes.
        reason: String,
    },
}

impl MultimodalError {
    /// Returns the stable string identifier for this variant. This is
    /// the canonical value to include in REST error envelopes and to
    /// route on in the HTTP-status mapping layer.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::InputTooLarge { .. } => "multimodal.input_too_large",
            Self::InvalidFormat { .. } => "multimodal.invalid_format",
            Self::InvalidDuration { .. } => "multimodal.invalid_duration",
            Self::ModalityDisabled { .. } => "multimodal.modality_disabled",
            Self::TritonUnavailable { .. } => "multimodal.triton_unavail",
            Self::TritonTimeout { .. } => "multimodal.triton_timeout",
            Self::TritonBadResponse { .. } => "multimodal.triton_bad_resp",
            Self::EncodeFailed { .. } => "multimodal.encode_failed",
            Self::FusionDimMismatch { .. } => "multimodal.fusion_dim_mismatch",
            Self::ChunkOverflow { .. } => "multimodal.chunk_overflow",
            Self::Internal { .. } => "multimodal.internal",
        }
    }

    /// Returns the recommended HTTP status code for this error, per
    /// the mapping documented at the top of this module.
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::InputTooLarge { .. } => 413,
            Self::InvalidFormat { .. } => 415,
            Self::InvalidDuration { .. } | Self::FusionDimMismatch { .. } | Self::ChunkOverflow { .. } => 422,
            Self::ModalityDisabled { .. } => 403,
            Self::TritonUnavailable { .. } => 503,
            Self::TritonTimeout { .. } => 504,
            Self::TritonBadResponse { .. } => 502,
            Self::EncodeFailed { .. } | Self::Internal { .. } => 500,
        }
    }

    /// Returns `true` if the error is retryable from the client's
    /// point of view. Currently only `TritonUnavailable` and
    /// `TritonTimeout` are retryable ; everything else is a client
    /// error or a permanent bug.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::TritonUnavailable { .. } | Self::TritonTimeout { .. })
    }
}

/// Type alias used pervasively in the crate.
pub type Result<T> = std::result::Result<T, MultimodalError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable() {
        // Lock the stable string identifiers so that a refactor that
        // accidentally renames one of them breaks this test.
        assert_eq!(
            MultimodalError::InputTooLarge { observed_bytes: 1, cap_bytes: 0 }.error_code(),
            "multimodal.input_too_large"
        );
        assert_eq!(
            MultimodalError::TritonTimeout { elapsed_ms: 1 }.error_code(),
            "multimodal.triton_timeout"
        );
    }

    #[test]
    fn http_status_mapping_is_well_defined() {
        assert_eq!(MultimodalError::InputTooLarge { observed_bytes: 1, cap_bytes: 0 }.http_status(), 413);
        assert_eq!(MultimodalError::ModalityDisabled { modality: "vision" }.http_status(), 403);
        assert_eq!(MultimodalError::TritonTimeout { elapsed_ms: 1 }.http_status(), 504);
        assert_eq!(MultimodalError::Internal { reason: "x".into() }.http_status(), 500);
    }

    #[test]
    fn retryable_set_is_minimal() {
        assert!(MultimodalError::TritonTimeout { elapsed_ms: 1 }.is_retryable());
        assert!(!MultimodalError::InvalidFormat {
            detected_mime: "x".into(),
            allowed_mimes: vec![],
        }
        .is_retryable());
        assert!(!MultimodalError::ModalityDisabled { modality: "vision" }.is_retryable());
    }
}
