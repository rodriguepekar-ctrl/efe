//! `IdentityDecoder` — MVP pass-through decoder.
//!
//! Returns the projected centroid verbatim as `DecodedOutput::Embedding`.
//! Useful as the default decoder when the client performs its own
//! decoding downstream (e.g. a sequence model, a classifier, or a
//! vector-DB lookup).

use async_trait::async_trait;

use crate::dissolution::bundle::DecodedOutput;
use crate::dissolution::decoders::OutputDecoder;
use crate::errors::Result;

/// Passthrough decoder: wraps the projected centroid in
/// `DecodedOutput::Embedding`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IdentityDecoder;

#[async_trait]
impl OutputDecoder for IdentityDecoder {
    async fn decode(&self, projected: &[f32], _weights: &[f32]) -> Result<DecodedOutput> {
        Ok(DecodedOutput::Embedding(projected.to_vec()))
    }

    fn name(&self) -> &'static str {
        "identity"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preserves_embedding() {
        let input = vec![0.1, 0.2, 0.3];
        let out = IdentityDecoder.decode(&input, &[]).await.unwrap();
        match out {
            DecodedOutput::Embedding(v) => {
                assert_eq!(v, input);
            }
            other => panic!("expected Embedding, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ignores_anchor_weights() {
        let input = vec![1.0, 2.0];
        let out1 = IdentityDecoder.decode(&input, &[]).await.unwrap();
        let out2 = IdentityDecoder.decode(&input, &[0.3, 0.7]).await.unwrap();
        assert_eq!(out1, out2);
    }

    #[tokio::test]
    async fn empty_input_returns_empty_embedding() {
        let out = IdentityDecoder.decode(&[], &[]).await.unwrap();
        match out {
            DecodedOutput::Embedding(v) => assert!(v.is_empty()),
            other => panic!("expected Embedding, got {other:?}"),
        }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(IdentityDecoder.name(), "identity");
    }
}
