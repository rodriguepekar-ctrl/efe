// SPDX-License-Identifier: UNLICENSED
//! # `efe-multimodal` — M30 multimodal inference plane.
//!
//! This crate is the **client-side** half of the EFE multimodal stack
//! (vision + audio + long-context text). It is consumed by
//! `efe-server` (REST + gRPC handlers under `v1/multimodal/`) and by
//! `efe-agent-runner` (tool invocations that include images or audio).
//!
//! ## Scope
//!
//! - **Vision encoder** : SigLIP-base-patch16-224 served by Triton.
//! - **Audio encoder** : Whisper-tiny served by Triton (transcript +
//!   pooled-encoder embedding).
//! - **Long-context chunker** : 512-token hierarchical chunker with
//!   markdown-heading-aware parent tree.
//! - **Late fusion** : concat-linear, weighted-mean (clip-joint is
//!   stubbed and deferred to M30.1).
//! - **Input guard** : magic-byte sniffing + size cap + audio
//!   duration estimate, with SHA-256 audit-safe hashing.
//! - **Per-tenant policy** : modality on/off + DPIA gating + per-MIME
//!   allow-list.
//! - **Triton HTTP client** : JSON `/v2/models/{m}/infer` ; gRPC
//!   deferred to M30.1.
//!
//! ## Out of scope (deferred)
//!
//! - Multi-image batching (M30.1)
//! - Streaming ASR (M31)
//! - Real-time video (M31)
//! - Software audio resampler (M30.1)
//! - Binary tensor attachments (M30.1)
//!
//! ## Architectural invariants
//!
//! 1. **Raw input never leaves the crate.** Every public method that
//!    accepts bytes either consumes them (encoder) or fingerprints
//!    them with SHA-256 (audit). Raw bytes are forbidden in tracing
//!    spans, logs, and metrics.
//! 2. **Errors are typed.** Every method returns `Result<T,
//!    MultimodalError>` where `MultimodalError` carries a stable
//!    `error_code()` for downstream HTTP mapping.
//! 3. **All Triton calls have a deadline.** No call may exceed the
//!    per-request `request_timeout_ms` (default 5 s, configurable).
//! 4. **All histograms have ≤ 2 labels.** Cardinality budgeted at
//!    ≤ 16 active series per histogram. (cf. metrics.rs)
//!
//! ## Re-exports
//!
//! For ergonomic use downstream, the most common types are
//! re-exported at the crate root.

#![warn(missing_docs)]
#![warn(rust_2018_idioms)]
#![deny(unsafe_code)]

pub mod audio;
pub mod chunking;
pub mod error;
pub mod fusion;
pub mod input_guard;
pub mod metrics;
pub mod policy;
pub mod triton_client;
pub mod vision;

pub use audio::{AudioEncoder, AudioOutput, WHISPER_EMBED_DIM, WHISPER_MODEL_NAME};
pub use chunking::{Chunk, ChunkConfig, HierarchicalChunker, DEFAULT_TOKENS_PER_CHUNK};
pub use error::{MultimodalError, Result};
pub use fusion::{
    fuse, FusionInputs, FusionStrategy, ModalityWeights, FUSION_MAX_CONCAT_DIM, FUSION_TARGET_DIM,
};
pub use input_guard::{
    input_hash_prefix, sha256_hex, GuardModality, GuardPolicy, InputGuard, InputMetadata,
};
pub use metrics::InflightGuard;
pub use policy::TenantModalityPolicy;
pub use triton_client::{InferRequest, InferResponse, TensorInput, TensorOutput, TritonClient, TritonClientConfig};
pub use vision::{l2_normalize, VisionEncoder, VisionOutput, SIGLIP_EMBED_DIM, SIGLIP_MODEL_NAME};

/// Static crate identifier emitted in user-agent strings and audit
/// trails.
pub const CRATE_NAME_VERSION: &str = concat!("efe-multimodal/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_name_version_includes_pkg_version() {
        assert!(CRATE_NAME_VERSION.starts_with("efe-multimodal/"));
    }

    #[test]
    fn reexports_are_constructible() {
        let _err: MultimodalError = MultimodalError::Internal { reason: "test".into() };
        let _policy: TenantModalityPolicy = TenantModalityPolicy::default();
        let _cfg: ChunkConfig = ChunkConfig::default();
        let _mw: ModalityWeights = ModalityWeights::default();
    }
}
