//! Post-extraction invariant checks, applicable independently of an
//! `OutputProjector` impl.
//!
//! These functions are defensive: an engine that has already called
//! [`crate::dissolution::OutputBundle::validate`] doesn't need them,
//! but consumers that deserialize bundles from untrusted sources do.

use crate::dissolution::bundle::OutputBundle;
use crate::errors::{EfeError, Result};

/// Categorical outcome of the validator, for telemetry.
#[derive(Clone, Debug, PartialEq)]
#[must_use]
pub enum ExtractionValidation {
    /// Bundle satisfies all invariants.
    Ok,
    /// `centroid_raw` is empty (dim = 0). Invariant of M4 / M8 : a
    /// populated ELS always has a non-zero latent dim.
    EmptyCentroid,
    /// `centroid_raw` and `centroid_projected` have different lengths.
    DimMismatch {
        /// Raw dim.
        raw: usize,
        /// Projected dim.
        projected: usize,
    },
    /// One of the centroid / decoded payloads has NaN or Inf.
    NonFinite {
        /// Component that was non-finite.
        component: &'static str,
    },
    /// Anchor weights violate their contract.
    BadWeights {
        /// Sum observed (expected ≈ 1 or 0 on empty).
        sum: f32,
    },
    /// Confidence outside `[0, 1]`.
    ConfidenceOutOfRange(f32),
    /// Alignment distance negative or non-finite.
    InvalidAlignmentDistance(f32),
}

impl ExtractionValidation {
    /// `true` iff variant is `Ok`.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }

    /// Stable label for telemetry.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::EmptyCentroid => "empty_centroid",
            Self::DimMismatch { .. } => "dim_mismatch",
            Self::NonFinite { .. } => "non_finite",
            Self::BadWeights { .. } => "bad_weights",
            Self::ConfidenceOutOfRange(_) => "confidence_out_of_range",
            Self::InvalidAlignmentDistance(_) => "invalid_alignment_distance",
        }
    }
}

/// Check all bundle invariants and return a categorical result.
///
/// Equivalent to [`OutputBundle::validate`] but returns structured
/// diagnostics instead of an error.
pub fn classify_bundle(bundle: &OutputBundle) -> ExtractionValidation {
    // 0. Non-empty centroid — aligned with `OutputBundle::validate()`.
    if bundle.centroid_raw.is_empty() && bundle.centroid_projected.is_empty() {
        return ExtractionValidation::EmptyCentroid;
    }
    if bundle.centroid_raw.len() != bundle.centroid_projected.len() {
        return ExtractionValidation::DimMismatch {
            raw: bundle.centroid_raw.len(),
            projected: bundle.centroid_projected.len(),
        };
    }
    if !bundle.centroid_raw.iter().all(|x| x.is_finite()) {
        return ExtractionValidation::NonFinite {
            component: "centroid_raw",
        };
    }
    if !bundle.centroid_projected.iter().all(|x| x.is_finite()) {
        return ExtractionValidation::NonFinite {
            component: "centroid_projected",
        };
    }
    if !bundle.decoded.is_finite() {
        return ExtractionValidation::NonFinite {
            component: "decoded",
        };
    }
    if !bundle
        .anchor_weights
        .iter()
        .all(|w| w.is_finite() && *w >= 0.0)
    {
        return ExtractionValidation::BadWeights { sum: f32::NAN };
    }
    if !bundle.anchor_weights.is_empty() {
        let sum: f32 = bundle.anchor_weights.iter().sum();
        if (sum - 1.0).abs() > 1e-3 {
            return ExtractionValidation::BadWeights { sum };
        }
    }
    if !bundle.confidence.is_finite() || !(0.0..=1.0).contains(&bundle.confidence) {
        return ExtractionValidation::ConfidenceOutOfRange(bundle.confidence);
    }
    if !bundle.alignment_distance.is_finite() || bundle.alignment_distance < 0.0 {
        return ExtractionValidation::InvalidAlignmentDistance(bundle.alignment_distance);
    }
    ExtractionValidation::Ok
}

/// Shortcut: convert a classification into a `Result<()>`.
///
/// Returns [`EfeError::Internal`] with a **static** label for any
/// non-Ok variant. Detailed diagnostics (dimensions, bad values, etc.)
/// remain available via [`classify_bundle`] when richer information is
/// needed.
///
/// # Errors
///
/// Returns [`EfeError::Internal`] when the classification is not `Ok`.
/// The error message is a `&'static str` — no allocation, no leak.
pub fn validate_bundle(bundle: &OutputBundle) -> Result<()> {
    match classify_bundle(bundle) {
        ExtractionValidation::Ok => Ok(()),
        ExtractionValidation::DimMismatch { .. } => Err(EfeError::Internal(
            "OutputBundle: centroid_raw and centroid_projected have different dims",
        )),
        ExtractionValidation::NonFinite { component } => Err(EfeError::Internal(match component {
            "centroid_raw" => "OutputBundle: centroid_raw contains non-finite values",
            "centroid_projected" => "OutputBundle: centroid_projected contains non-finite values",
            "decoded" => "OutputBundle: decoded payload has non-finite values",
            _ => "OutputBundle: non-finite value in an unknown component",
        })),
        ExtractionValidation::BadWeights { .. } => Err(EfeError::Internal(
            "OutputBundle: anchor_weights fail non-negativity / sum-to-one invariant",
        )),
        ExtractionValidation::ConfidenceOutOfRange(_) => Err(EfeError::Internal(
            "OutputBundle: confidence outside [0, 1]",
        )),
        ExtractionValidation::InvalidAlignmentDistance(_) => Err(EfeError::Internal(
            "OutputBundle: alignment_distance is non-finite or negative",
        )),
        ExtractionValidation::EmptyCentroid => {
            Err(EfeError::Internal("OutputBundle: centroid_raw is empty"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dissolution::bundle::{DecodedOutput, OutputBundle};
    use crate::dissolution::telemetry::ExtractionTelemetry;
    use crate::types::SwarmId;

    fn mk_bundle() -> OutputBundle {
        OutputBundle {
            swarm_id: SwarmId::new(),
            centroid_raw: vec![0.5, 0.5],
            centroid_projected: vec![0.4, 0.4],
            confidence: 0.9,
            alignment_distance: 0.14,
            anchor_weights: vec![1.0],
            decoded: DecodedOutput::Embedding(vec![0.4, 0.4]),
            telemetry: ExtractionTelemetry::default(),
        }
    }

    #[test]
    fn classify_ok_on_clean_bundle() {
        let b = mk_bundle();
        assert_eq!(classify_bundle(&b), ExtractionValidation::Ok);
    }

    #[test]
    fn classify_dim_mismatch() {
        let mut b = mk_bundle();
        b.centroid_projected.pop();
        match classify_bundle(&b) {
            ExtractionValidation::DimMismatch { .. } => {}
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classify_nan_centroid_raw() {
        let mut b = mk_bundle();
        b.centroid_raw[0] = f32::NAN;
        match classify_bundle(&b) {
            ExtractionValidation::NonFinite { component } => {
                assert_eq!(component, "centroid_raw");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classify_nan_projected() {
        let mut b = mk_bundle();
        b.centroid_projected[0] = f32::INFINITY;
        match classify_bundle(&b) {
            ExtractionValidation::NonFinite { component } => {
                assert_eq!(component, "centroid_projected");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classify_nan_decoded() {
        let mut b = mk_bundle();
        b.decoded = DecodedOutput::Embedding(vec![f32::NAN]);
        match classify_bundle(&b) {
            ExtractionValidation::NonFinite { component } => {
                assert_eq!(component, "decoded");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classify_bad_weights_sum() {
        let mut b = mk_bundle();
        b.anchor_weights = vec![0.3, 0.3];
        match classify_bundle(&b) {
            ExtractionValidation::BadWeights { sum } => {
                assert!((sum - 0.6).abs() < 1e-5);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classify_negative_weight() {
        let mut b = mk_bundle();
        b.anchor_weights = vec![1.5, -0.5];
        assert!(matches!(
            classify_bundle(&b),
            ExtractionValidation::BadWeights { .. }
        ));
    }

    #[test]
    fn classify_confidence_out_of_range() {
        let mut b = mk_bundle();
        b.confidence = 1.5;
        match classify_bundle(&b) {
            ExtractionValidation::ConfidenceOutOfRange(c) => {
                assert!((c - 1.5).abs() < 1e-5);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn classify_negative_alignment_distance() {
        let mut b = mk_bundle();
        b.alignment_distance = -0.1;
        match classify_bundle(&b) {
            ExtractionValidation::InvalidAlignmentDistance(d) => {
                assert!((d + 0.1).abs() < 1e-5);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn validate_bundle_ok_passes() {
        validate_bundle(&mk_bundle()).unwrap();
    }

    #[test]
    fn validate_bundle_fails_on_nan() {
        let mut b = mk_bundle();
        b.centroid_raw[0] = f32::NAN;
        assert!(validate_bundle(&b).is_err());
    }

    #[test]
    fn labels_are_stable_snake_case() {
        let cases = [
            ExtractionValidation::Ok,
            ExtractionValidation::DimMismatch {
                raw: 1,
                projected: 2,
            },
            ExtractionValidation::NonFinite {
                component: "centroid_raw",
            },
            ExtractionValidation::BadWeights { sum: 0.0 },
            ExtractionValidation::ConfidenceOutOfRange(2.0),
            ExtractionValidation::InvalidAlignmentDistance(-1.0),
        ];
        for c in cases {
            let l = c.label();
            assert!(l.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_'));
        }
    }

    #[test]
    fn is_ok_only_true_for_ok() {
        assert!(ExtractionValidation::Ok.is_ok());
        assert!(!ExtractionValidation::BadWeights { sum: 0.0 }.is_ok());
    }

    #[test]
    fn classify_empty_centroid_returns_empty_centroid_variant() {
        let mut b = mk_bundle();
        b.centroid_raw.clear();
        b.centroid_projected.clear();
        match classify_bundle(&b) {
            ExtractionValidation::EmptyCentroid => {}
            other => panic!("expected EmptyCentroid, got {other:?}"),
        }
    }

    #[test]
    fn validate_bundle_aligns_with_bundle_validate() {
        // Invariant: whenever OutputBundle::validate() succeeds,
        // validate_bundle() also succeeds, and vice-versa for failure.
        // We exercise both on a matrix of bundles.
        let cases = {
            let mut v = vec![mk_bundle()];
            {
                let mut b = mk_bundle();
                b.centroid_raw[0] = f32::NAN;
                v.push(b);
            }
            {
                let mut b = mk_bundle();
                b.centroid_projected.pop();
                v.push(b);
            }
            {
                let mut b = mk_bundle();
                b.confidence = 2.0;
                v.push(b);
            }
            {
                let mut b = mk_bundle();
                b.alignment_distance = -1.0;
                v.push(b);
            }
            {
                let mut b = mk_bundle();
                b.anchor_weights = vec![0.3, 0.3];
                v.push(b);
            }
            {
                let mut b = mk_bundle();
                b.centroid_raw.clear();
                b.centroid_projected.clear();
                v.push(b);
            }
            v
        };
        for b in &cases {
            let validate_result = b.validate();
            let vb_result = validate_bundle(b);
            assert_eq!(
                validate_result.is_ok(),
                vb_result.is_ok(),
                "validate() and validate_bundle() disagree on {b:?}"
            );
        }
    }

    #[test]
    fn validate_bundle_emits_static_error_messages() {
        // Ensure we're not regressing into Box::leak: a failed
        // validation must carry a `&'static str` message, not an
        // owned string allocated on each call.
        let mut b = mk_bundle();
        b.centroid_raw[0] = f32::NAN;
        let err1 = validate_bundle(&b).unwrap_err();
        let err2 = validate_bundle(&b).unwrap_err();
        // Two calls produce the same static pointer — proof of
        // static-literal usage (no fresh allocation).
        if let (EfeError::Internal(m1), EfeError::Internal(m2)) = (err1, err2) {
            assert_eq!(m1.as_ptr(), m2.as_ptr());
        } else {
            panic!("expected two Internal errors");
        }
    }
}
