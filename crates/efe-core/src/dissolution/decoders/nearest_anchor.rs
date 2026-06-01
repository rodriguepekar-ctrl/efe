//! `NearestAnchorDecoder` — picks the anchor with highest cosine with
//! the projected centroid, returns its index + similarity score.
//!
//! Useful for classification-style outputs where each anchor represents
//! a class / category and the swarm's job is to pick one.

use std::sync::Arc;

use async_trait::async_trait;

use crate::dissolution::bundle::DecodedOutput;
use crate::dissolution::decoders::OutputDecoder;
use crate::els::metrics::cosine_similarity;
use crate::errors::{EfeError, Result};

/// Decoder that returns the index of the anchor closest (in cosine
/// similarity) to the projected centroid.
///
/// Requires a snapshot of the anchors — the decoder keeps its own
/// `Arc<Vec<Vec<f32>>>` so callers can swap anchor sets without
/// mutating the decoder.
#[derive(Debug, Clone)]
pub struct NearestAnchorDecoder {
    anchors: Arc<Vec<Vec<f32>>>,
}

impl NearestAnchorDecoder {
    /// Construct with a reference set of anchors.
    ///
    /// # Errors
    /// - [`EfeError::InvalidConfig`] if `anchors` is empty.
    pub fn new(anchors: Arc<Vec<Vec<f32>>>) -> Result<Self> {
        if anchors.is_empty() {
            return Err(EfeError::InvalidConfig(
                "NearestAnchorDecoder: anchors must be non-empty".into(),
            ));
        }
        Ok(Self { anchors })
    }

    /// Number of anchors.
    #[must_use]
    pub fn n_anchors(&self) -> usize {
        self.anchors.len()
    }
}

#[async_trait]
impl OutputDecoder for NearestAnchorDecoder {
    async fn decode(&self, projected: &[f32], _weights: &[f32]) -> Result<DecodedOutput> {
        if self.anchors.is_empty() {
            return Err(EfeError::Internal(
                "NearestAnchorDecoder: anchors vector went empty unexpectedly",
            ));
        }
        let mut best_idx = 0usize;
        let mut best_score = f32::NEG_INFINITY;
        for (idx, anchor) in self.anchors.iter().enumerate() {
            if anchor.len() != projected.len() {
                return Err(EfeError::InvalidConfig(format!(
                    "NearestAnchorDecoder: anchor[{idx}].len() = {} != projected dim {}",
                    anchor.len(),
                    projected.len()
                )));
            }
            let score = cosine_similarity(projected, anchor);
            if score.is_finite() && score > best_score {
                best_score = score;
                best_idx = idx;
            }
        }
        if !best_score.is_finite() {
            return Err(EfeError::Internal(
                "NearestAnchorDecoder: no finite cosine computed",
            ));
        }
        Ok(DecodedOutput::AnchorLabel {
            index: u32::try_from(best_idx).unwrap_or(u32::MAX),
            score: best_score,
        })
    }

    fn name(&self) -> &'static str {
        "nearest-anchor"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn picks_exact_match_anchor() {
        let anchors = Arc::new(vec![
            vec![1.0f32, 0.0, 0.0],
            vec![0.0f32, 1.0, 0.0],
            vec![0.0f32, 0.0, 1.0],
        ]);
        let d = NearestAnchorDecoder::new(anchors).unwrap();
        let out = d.decode(&[0.0f32, 1.0, 0.0], &[]).await.unwrap();
        match out {
            DecodedOutput::AnchorLabel { index, score } => {
                assert_eq!(index, 1);
                assert!((score - 1.0).abs() < 1e-5);
            }
            other => panic!("expected AnchorLabel, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn picks_max_cosine_anchor_with_mixed_directions() {
        let anchors = Arc::new(vec![vec![1.0f32, 0.0], vec![0.7, 0.7], vec![0.0, 1.0]]);
        let d = NearestAnchorDecoder::new(anchors).unwrap();
        let projected = vec![0.8f32, 0.6]; // close to anchor index 1
        let out = d.decode(&projected, &[]).await.unwrap();
        match out {
            DecodedOutput::AnchorLabel { index, score } => {
                assert_eq!(index, 1);
                assert!(score > 0.9);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_dim_mismatch() {
        let anchors = Arc::new(vec![vec![1.0f32; 5]]);
        let d = NearestAnchorDecoder::new(anchors).unwrap();
        let err = d.decode(&[0.0f32; 3], &[]).await.unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn constructor_rejects_empty_anchors() {
        let err = NearestAnchorDecoder::new(Arc::new(vec![])).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn n_anchors_matches_size() {
        let anchors = Arc::new(vec![vec![1.0f32; 2]; 3]);
        let d = NearestAnchorDecoder::new(anchors).unwrap();
        assert_eq!(d.n_anchors(), 3);
    }

    #[test]
    fn name_is_stable() {
        let anchors = Arc::new(vec![vec![1.0f32]]);
        let d = NearestAnchorDecoder::new(anchors).unwrap();
        assert_eq!(d.name(), "nearest-anchor");
    }
}
