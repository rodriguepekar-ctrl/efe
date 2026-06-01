//! Output decoders — step 3 of the dissolution pipeline.
//!
//! Decoders transform the projected centroid into a [`crate::dissolution::DecodedOutput`].
//! The MVP ships two implementations; Sprint 9+ adds external
//! (HuggingFace, ONNX) decoders behind feature flags.

pub mod identity;
pub mod nearest_anchor;

pub use identity::IdentityDecoder;
pub use nearest_anchor::NearestAnchorDecoder;

use async_trait::async_trait;

use crate::dissolution::bundle::DecodedOutput;
use crate::errors::Result;

/// Transform a projected centroid + anchor weights into a final output form.
#[async_trait]
pub trait OutputDecoder: Send + Sync + std::fmt::Debug {
    /// Run the decoder.
    ///
    /// # Errors
    /// - [`crate::EfeError::Internal`] on decoder-specific failures
    ///   (e.g. missing model weights in external decoders).
    async fn decode(&self, projected: &[f32], anchor_weights: &[f32]) -> Result<DecodedOutput>;

    /// Stable short name for telemetry.
    fn name(&self) -> &'static str;
}
