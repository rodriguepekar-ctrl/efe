//! Sprint **D16** — embedder runtime adapter.
//!
//! Mirror of `docs/bench/d16_embedder_bundling_architecture.md` § 8.4.
//!
//! D16 ships **two** runtime implementations :
//!
//!   * [`mock::MockSession`] — deterministic SHA-256-seeded PRNG ;
//!     used for tests, demos, and the Tier 1 dev path that doesn't
//!     want to install a 200 MiB ONNX runtime. Identical behaviour
//!     to the existing `efe_ft.embedder.backends.mock.MockEmbedder`
//!     so an integration test that round-trips through the FFI can
//!     compare bit-exact vectors.
//!
//!   * `onnx::OnnxSession` (deferred to D16-bis) — real ONNX
//!     Runtime adapter. Pulls in `ort = "2.0"` which has
//!     platform-specific shared-library dependencies (libonnxruntime).
//!     We keep the trait stable here so the D16-bis switch is
//!     a pure addition, not a refactor.
//!
//! The runtime trait is async to allow GPU offload paths in
//! D16-bis ; current `MockSession` is synchronous CPU work that
//! returns immediately.

use thiserror::Error;

use super::manifest::{Framework, Manifest};

pub mod mock;

// Stub for the ONNX path. D16-bis will provide the real impl ;
// for now the mock path covers the loader / verify / signature
// surface we ship in D16.
pub mod onnx_stub;

/// Per-bundle inference session. Implementations are concrete types
/// (e.g. [`mock::MockSession`]) wrapped in `Box<dyn EmbedderSession>`
/// inside [`super::bundle::Bundle`].
pub trait EmbedderSession: Send + Sync {
    /// Encode one text → vector of length `dim()`.
    fn encode_one(&self, text: &str) -> Result<Vec<f32>, EmbedderError>;
    /// Encode a batch of texts → vectors of length `dim()`.
    fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError>;
    /// Output dimension (matches `manifest.dim`).
    fn dim(&self) -> usize;
    /// Framework label for telemetry / logs.
    fn framework_label(&self) -> &'static str;
}

/// Failure modes of the encoder hot path.
#[derive(Debug, Error)]
pub enum EmbedderError {
    /// Tokenization failed (e.g. malformed input bytes).
    #[error("tokenizer error: {0}")]
    Tokenizer(String),
    /// Forward pass failed (NaN / runtime crash / etc.).
    #[error("forward pass error: {0}")]
    Forward(String),
    /// Empty input — the caller must filter these out, but we
    /// surface a typed error rather than silently returning
    /// `[0; dim]`.
    #[error("empty input not allowed")]
    EmptyInput,
    /// Caller passed a slice of refs containing a mismatched-length
    /// item — defensive.
    #[error("invalid input shape: {0}")]
    InvalidShape(String),
}

/// Build an [`EmbedderSession`] from a verified manifest + bundle root.
/// Architecture § 8.3 step 4.
///
/// In D16 we always return a [`mock::MockSession`] regardless of
/// `framework`. D16-bis adds the ONNX path :
///
/// ```rust,ignore
/// match manifest.framework {
///     Framework::Onnx => Box::new(onnx::OnnxSession::new(manifest, root)?),
///     Framework::Safetensors => Box::new(safetensors::SafetensorsSession::new(manifest, root)?),
/// }
/// ```
pub fn instantiate(
    manifest: &Manifest,
    bundle_root: &std::path::Path,
) -> Result<Box<dyn EmbedderSession>, EmbedderError> {
    let _ = bundle_root;
    match manifest.framework {
        Framework::Onnx | Framework::Safetensors => {
            // **D16 placeholder** : both frameworks return MockSession
            // in v0.1.16. Real ONNX Runtime + safetensors loaders ship
            // in D16-bis. The trait is stable, so swapping the impl
            // is purely additive (no caller change).
            tracing::info!(
                target: "efe.embedder.runtime",
                framework = manifest.framework.as_str(),
                bundle_id = %manifest.bundle_id,
                "D16 mock runtime — D16-bis will swap in real ORT/safetensors loaders"
            );
            Ok(Box::new(mock::MockSession::for_dim(manifest.dim)))
        }
    }
}
