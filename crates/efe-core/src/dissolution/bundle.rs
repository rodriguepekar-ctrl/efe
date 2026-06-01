//! `OutputBundle` and `DecodedOutput` — the final types emitted by M8.
//!
//! The bundle aggregates every piece of information produced by the
//! dissolution pipeline. It is fully serializable and free of
//! lifetimes / arena references — consumable by M11 (telemetry) and
//! M12 (server / FFI) without further processing.

use serde::{Deserialize, Serialize};

use crate::dissolution::telemetry::ExtractionTelemetry;
use crate::errors::{EfeError, Result};
use crate::types::SwarmId;

/// Discriminated final output form, produced by an [`crate::dissolution::OutputDecoder`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DecodedOutput {
    /// Identity pass-through — the raw f32 embedding.
    Embedding(Vec<f32>),
    /// Anchor chosen by nearest cosine, with its score ∈ `[-1, 1]`.
    AnchorLabel {
        /// Index of the selected anchor.
        index: u32,
        /// Cosine similarity with the projected centroid.
        score: f32,
    },
    /// Decoded tokens (Sprint 9+).
    Text(String),
    /// Raw bytes from an opaque decoder (Sprint 9+).
    Bytes(Vec<u8>),
}

impl DecodedOutput {
    /// Short label for metrics / telemetry.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Embedding(_) => "embedding",
            Self::AnchorLabel { .. } => "anchor_label",
            Self::Text(_) => "text",
            Self::Bytes(_) => "bytes",
        }
    }

    /// Approximate byte size of the decoded payload (used by M11).
    #[must_use]
    pub fn byte_size(&self) -> usize {
        match self {
            Self::Embedding(v) => v.len() * 4,
            Self::AnchorLabel { .. } => 8, // u32 + f32
            Self::Text(s) => s.len(),
            Self::Bytes(b) => b.len(),
        }
    }

    /// True iff every scalar in the payload is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Embedding(v) => v.iter().all(|x| x.is_finite()),
            Self::AnchorLabel { score, .. } => score.is_finite(),
            Self::Text(_) | Self::Bytes(_) => true,
        }
    }
}

impl std::fmt::Display for DecodedOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Embedding(v) => write!(f, "embedding(dim={})", v.len()),
            Self::AnchorLabel { index, score } => {
                write!(f, "anchor_label(index={index}, score={score:.4})")
            }
            Self::Text(s) => write!(f, "text(len={})", s.len()),
            Self::Bytes(b) => write!(f, "bytes(len={})", b.len()),
        }
    }
}

/// The authoritative output of one dissolution run.
///
/// Returned by [`crate::dissolution::OutputProjector::extract`] on
/// success. Consumers (M11, M12) should call [`Self::validate`] before
/// trusting the contents when deserializing from untrusted sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutputBundle {
    /// Swarm identifier for traceability.
    pub swarm_id: SwarmId,
    /// Raw centroid extracted from the ELS (pre-projection).
    pub centroid_raw: Vec<f32>,
    /// Projected centroid (post-alignment, within cone).
    pub centroid_projected: Vec<f32>,
    /// Confidence propagated from M6 (unchanged).
    pub confidence: f32,
    /// `||centroid_raw - centroid_projected||_2`.
    pub alignment_distance: f32,
    /// Normalized non-negative weights per anchor (Σ ≈ 1 unless empty).
    pub anchor_weights: Vec<f32>,
    /// Final decoded form.
    pub decoded: DecodedOutput,
    /// Telemetry for M11.
    pub telemetry: ExtractionTelemetry,
}

impl OutputBundle {
    /// Verify post-condition invariants.
    ///
    /// Called automatically by the engine when `ExtractionConfig::validate_post`
    /// is true. Consumers can call it defensively after deserialization.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::Internal`] on any invariant violation.
    pub fn validate(&self) -> Result<()> {
        // 1. Centroid dims agree.
        if self.centroid_raw.len() != self.centroid_projected.len() {
            return Err(EfeError::Internal(
                "OutputBundle: centroid_raw and centroid_projected have different dims",
            ));
        }
        if self.centroid_raw.is_empty() {
            return Err(EfeError::Internal("OutputBundle: centroid_raw is empty"));
        }
        // 2. All centroid elements finite.
        if !self.centroid_raw.iter().all(|x| x.is_finite()) {
            return Err(EfeError::Internal(
                "OutputBundle: centroid_raw contains non-finite values",
            ));
        }
        if !self.centroid_projected.iter().all(|x| x.is_finite()) {
            return Err(EfeError::Internal(
                "OutputBundle: centroid_projected contains non-finite values",
            ));
        }
        // 3. Weights: non-negative, finite, and sum ≈ 1 (if non-empty).
        if !self
            .anchor_weights
            .iter()
            .all(|w| w.is_finite() && *w >= 0.0)
        {
            return Err(EfeError::Internal(
                "OutputBundle: anchor_weights contains non-finite or negative values",
            ));
        }
        if !self.anchor_weights.is_empty() {
            let sum: f32 = self.anchor_weights.iter().sum();
            if (sum - 1.0).abs() > 1e-3 {
                return Err(EfeError::Internal(
                    "OutputBundle: anchor_weights do not sum to 1",
                ));
            }
        }
        // 4. Confidence in [0, 1] and finite.
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(EfeError::Internal(
                "OutputBundle: confidence outside [0, 1]",
            ));
        }
        // 5. Alignment distance finite and non-negative.
        if !self.alignment_distance.is_finite() || self.alignment_distance < 0.0 {
            return Err(EfeError::Internal(
                "OutputBundle: alignment_distance is non-finite or negative",
            ));
        }
        // 6. Decoded payload is finite.
        if !self.decoded.is_finite() {
            return Err(EfeError::Internal(
                "OutputBundle: decoded payload has non-finite values",
            ));
        }
        Ok(())
    }

    /// Quality score: `1.0 - min(1, distance / cone_radius)` given the
    /// radius from the originating ELS. Returns `1.0` when
    /// `cone_radius ≤ 0` (no cone constraint).
    #[must_use]
    pub fn alignment_quality(&self, cone_radius: f32) -> f32 {
        if cone_radius <= 0.0 || !cone_radius.is_finite() {
            return 1.0;
        }
        (1.0 - (self.alignment_distance / cone_radius).clamp(0.0, 1.0)).max(0.0)
    }

    /// Dimension of the centroid vectors. Zero on a malformed bundle.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.centroid_projected.len()
    }
}

impl std::fmt::Display for OutputBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "bundle[swarm={swarm:?} conf={conf:.4} dist={dist:.4} decoded={decoded}]",
            swarm = self.swarm_id,
            conf = self.confidence,
            dist = self.alignment_distance,
            decoded = self.decoded,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_bundle() -> OutputBundle {
        OutputBundle {
            swarm_id: SwarmId::new(),
            centroid_raw: vec![0.5, 0.5, 0.5],
            centroid_projected: vec![0.4, 0.4, 0.4],
            confidence: 0.9,
            alignment_distance: 0.17,
            anchor_weights: vec![1.0],
            decoded: DecodedOutput::Embedding(vec![0.4, 0.4, 0.4]),
            telemetry: ExtractionTelemetry::default(),
        }
    }

    #[test]
    fn validate_ok_on_clean_bundle() {
        let b = mk_bundle();
        b.validate().unwrap();
    }

    #[test]
    fn validate_rejects_dim_mismatch() {
        let mut b = mk_bundle();
        b.centroid_projected.pop();
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_empty_centroid() {
        let mut b = mk_bundle();
        b.centroid_raw.clear();
        b.centroid_projected.clear();
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_nan_centroid() {
        let mut b = mk_bundle();
        b.centroid_raw[0] = f32::NAN;
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_nan_projected() {
        let mut b = mk_bundle();
        b.centroid_projected[0] = f32::INFINITY;
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_weight() {
        let mut b = mk_bundle();
        b.anchor_weights = vec![0.5, -0.5, 1.0];
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_weights_not_summing_to_one() {
        let mut b = mk_bundle();
        b.anchor_weights = vec![0.3, 0.3];
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_accepts_empty_weights() {
        let mut b = mk_bundle();
        b.anchor_weights.clear();
        // Vide signifie "pas d'anchors" — validate accepte.
        b.validate().unwrap();
    }

    #[test]
    fn validate_rejects_confidence_out_of_range() {
        let mut b = mk_bundle();
        b.confidence = 1.5;
        assert!(b.validate().is_err());
        b.confidence = -0.1;
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_negative_distance() {
        let mut b = mk_bundle();
        b.alignment_distance = -0.1;
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_nan_distance() {
        let mut b = mk_bundle();
        b.alignment_distance = f32::NAN;
        assert!(b.validate().is_err());
    }

    #[test]
    fn validate_rejects_non_finite_decoded() {
        let mut b = mk_bundle();
        b.decoded = DecodedOutput::Embedding(vec![f32::NAN]);
        assert!(b.validate().is_err());
    }

    #[test]
    fn decoded_kind_is_stable() {
        assert_eq!(DecodedOutput::Embedding(vec![]).kind(), "embedding");
        assert_eq!(
            DecodedOutput::AnchorLabel {
                index: 0,
                score: 0.0
            }
            .kind(),
            "anchor_label"
        );
        assert_eq!(DecodedOutput::Text(String::new()).kind(), "text");
        assert_eq!(DecodedOutput::Bytes(vec![]).kind(), "bytes");
    }

    #[test]
    fn decoded_byte_size_proportional_to_payload() {
        assert_eq!(DecodedOutput::Embedding(vec![0.0; 4]).byte_size(), 16);
        assert_eq!(DecodedOutput::Text("abc".into()).byte_size(), 3);
        assert_eq!(DecodedOutput::Bytes(vec![0u8; 10]).byte_size(), 10);
    }

    #[test]
    fn decoded_is_finite_detects_nan() {
        assert!(DecodedOutput::Embedding(vec![1.0, 2.0]).is_finite());
        assert!(!DecodedOutput::Embedding(vec![f32::NAN]).is_finite());
        assert!(DecodedOutput::AnchorLabel {
            index: 0,
            score: 0.5
        }
        .is_finite());
        assert!(!DecodedOutput::AnchorLabel {
            index: 0,
            score: f32::INFINITY
        }
        .is_finite());
        assert!(DecodedOutput::Text("x".into()).is_finite());
        assert!(DecodedOutput::Bytes(vec![0u8]).is_finite());
    }

    #[test]
    fn alignment_quality_full_on_zero_distance() {
        let mut b = mk_bundle();
        b.alignment_distance = 0.0;
        assert!((b.alignment_quality(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn alignment_quality_zero_at_or_above_radius() {
        let mut b = mk_bundle();
        b.alignment_distance = 1.5;
        assert!(b.alignment_quality(1.0).abs() < 1e-6);
    }

    #[test]
    fn alignment_quality_returns_one_on_zero_radius() {
        assert!((mk_bundle().alignment_quality(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn display_impl_readable() {
        let b = mk_bundle();
        let s = format!("{b}");
        assert!(s.contains("bundle"));
        assert!(s.contains("embedding"));
    }

    #[test]
    fn serde_round_trip() {
        let b = mk_bundle();
        let s = serde_json::to_string(&b).unwrap();
        let back: OutputBundle = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }
}
