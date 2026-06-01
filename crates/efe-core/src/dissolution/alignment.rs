//! Strict alignment projection — step 2 of the dissolution pipeline.
//!
//! Unlike `els::projection::project_vector_onto_anchors` (used by M5 at
//! each fusion round in a soft manner), the M8 projector is **strict**:
//! it emits `EfeError::AlignmentBreach` when the projection would place
//! the final centroid too far from the cone. This is the last barrier
//! before handing the output to the client.

use crate::dissolution::config::ExtractionConfig;
use crate::els::metrics::{cosine_similarity, l2_distance};
use crate::errors::{EfeError, Result};

/// Outcome of a successful projection call.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionOutcome {
    /// Blended `(1-α)·raw + α·anchor_centroid`.
    pub projected_centroid: Vec<f32>,
    /// `||centroid_raw - projected_centroid||_2`.
    pub distance_from_raw: f32,
    /// Normalized non-negative weights per anchor (Σ ≈ 1).
    pub anchor_weights: Vec<f32>,
}

/// Project a centroid onto the alignment cone's convex hull.
///
/// Implementations:
/// - [`StrictAlignmentProjector`] — enforces `cone_radius` with
///   `breach_tolerance`.
pub trait AlignmentProjector: Send + Sync + std::fmt::Debug {
    /// Perform the projection.
    ///
    /// # Errors
    ///
    /// - [`EfeError::AlignmentBreach`] if the resulting distance exceeds
    ///   `cone_radius * (1 + config.breach_tolerance)`, or if no anchor
    ///   has positive cosine with the centroid.
    /// - [`EfeError::InvalidConfig`] on dimension mismatch between
    ///   centroid and any anchor.
    fn project(
        &self,
        centroid_raw: &[f32],
        anchors: &[Vec<f32>],
        cone_radius: f32,
        config: &ExtractionConfig,
    ) -> Result<ProjectionOutcome>;

    /// Stable short name for telemetry.
    fn name(&self) -> &'static str;
}

/// MVP projector with strict breach detection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StrictAlignmentProjector;

impl AlignmentProjector for StrictAlignmentProjector {
    fn project(
        &self,
        centroid_raw: &[f32],
        anchors: &[Vec<f32>],
        cone_radius: f32,
        config: &ExtractionConfig,
    ) -> Result<ProjectionOutcome> {
        if centroid_raw.is_empty() {
            return Err(EfeError::Internal(
                "StrictAlignmentProjector: centroid_raw is empty",
            ));
        }
        if anchors.is_empty() {
            return Err(EfeError::AlignmentBreach {
                delta: f32::INFINITY,
                max: cone_radius,
            });
        }

        // Compute positive cosine weights. Reject dim mismatches early.
        let mut weights = Vec::with_capacity(anchors.len());
        let mut total = 0.0f32;
        for (idx, anchor) in anchors.iter().enumerate() {
            if anchor.len() != centroid_raw.len() {
                return Err(EfeError::InvalidConfig(format!(
                    "StrictAlignmentProjector: anchor[{idx}].len() = {} != centroid dim {}",
                    anchor.len(),
                    centroid_raw.len()
                )));
            }
            let cos = cosine_similarity(centroid_raw, anchor);
            let w = if cos.is_finite() { cos.max(0.0) } else { 0.0 };
            weights.push(w);
            total += w;
        }
        if !total.is_finite() || total < f32::EPSILON {
            // Centroid is orthogonal or worse than orthogonal to every
            // anchor — no alignment direction available.
            return Err(EfeError::AlignmentBreach {
                delta: f32::INFINITY,
                max: cone_radius,
            });
        }
        for w in &mut weights {
            *w /= total;
        }

        // Anchor-centroid = convex combination of anchors.
        let mut anchor_centroid = vec![0.0f32; centroid_raw.len()];
        for (anchor, &w) in anchors.iter().zip(weights.iter()) {
            if w == 0.0 {
                continue;
            }
            for (ac, a) in anchor_centroid.iter_mut().zip(anchor.iter()) {
                *ac += w * a;
            }
        }

        // Blend: projected = (1-α)·raw + α·anchor_centroid.
        let alpha = config.alignment_strength;
        let mut projected = Vec::with_capacity(centroid_raw.len());
        for (r, ac) in centroid_raw.iter().zip(anchor_centroid.iter()) {
            projected.push((1.0 - alpha) * r + alpha * ac);
        }

        let delta = l2_distance(centroid_raw, &projected);

        // Strict breach check.
        let limit = cone_radius * (1.0 + config.breach_tolerance);
        if !delta.is_finite() || delta > limit {
            return Err(EfeError::AlignmentBreach {
                delta,
                max: cone_radius,
            });
        }

        Ok(ProjectionOutcome {
            projected_centroid: projected,
            distance_from_raw: delta,
            anchor_weights: weights,
        })
    }

    fn name(&self) -> &'static str {
        "strict-alignment"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ExtractionConfig {
        ExtractionConfig::default()
    }

    #[test]
    fn projection_on_aligned_centroid_succeeds() {
        let centroid = vec![1.0f32, 0.0, 0.0];
        let anchors = vec![vec![1.0, 0.0, 0.0]];
        let outcome = StrictAlignmentProjector
            .project(&centroid, &anchors, 1.0, &cfg())
            .unwrap();
        // Anchor == centroid direction → projected = centroid_raw.
        for (p, c) in outcome.projected_centroid.iter().zip(centroid.iter()) {
            assert!((p - c).abs() < 1e-5);
        }
        assert!(outcome.distance_from_raw.abs() < 1e-5);
        assert!((outcome.anchor_weights[0] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn projection_orthogonal_centroid_breaches() {
        let centroid = vec![1.0f32, 0.0, 0.0];
        let anchors = vec![vec![0.0f32, 1.0, 0.0]];
        let err = StrictAlignmentProjector
            .project(&centroid, &anchors, 0.5, &cfg())
            .unwrap_err();
        assert!(matches!(err, EfeError::AlignmentBreach { .. }));
    }

    #[test]
    fn projection_empty_anchors_breaches() {
        let centroid = vec![1.0f32];
        let err = StrictAlignmentProjector
            .project(&centroid, &[], 0.5, &cfg())
            .unwrap_err();
        assert!(matches!(err, EfeError::AlignmentBreach { .. }));
    }

    #[test]
    fn projection_empty_centroid_internal_error() {
        let err = StrictAlignmentProjector
            .project(&[], &[vec![1.0f32]], 0.5, &cfg())
            .unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn projection_dim_mismatch_invalid_config() {
        let centroid = vec![1.0f32; 3];
        let anchors = vec![vec![1.0f32; 5]];
        let err = StrictAlignmentProjector
            .project(&centroid, &anchors, 1.0, &cfg())
            .unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn projection_respects_breach_tolerance() {
        // Centroid far from anchor direction, but still within
        // cone_radius*(1+tolerance).
        let centroid = vec![1.0f32, 0.1, 0.0];
        let anchors = vec![vec![1.0f32, 1.0, 0.0]];
        let mut c = cfg();
        c.alignment_strength = 1.0; // full pull to anchor centroid
        c.breach_tolerance = 0.5; // 50% slack
        let outcome = StrictAlignmentProjector
            .project(&centroid, &anchors, 2.0, &c)
            .unwrap();
        assert!(outcome.distance_from_raw < 2.0 * 1.5);
    }

    #[test]
    fn projection_alpha_zero_returns_raw() {
        let centroid = vec![2.0f32, 1.0];
        let anchors = vec![vec![1.0f32, 1.0]];
        let mut c = cfg();
        c.alignment_strength = 0.0;
        let outcome = StrictAlignmentProjector
            .project(&centroid, &anchors, 1.0, &c)
            .unwrap();
        for (p, r) in outcome.projected_centroid.iter().zip(centroid.iter()) {
            assert!((p - r).abs() < 1e-6);
        }
        assert!(outcome.distance_from_raw.abs() < 1e-6);
    }

    #[test]
    fn projection_weights_sum_to_one() {
        let centroid = vec![1.0f32, 1.0, 1.0];
        let anchors = vec![
            vec![1.0f32, 0.0, 0.0],
            vec![0.0f32, 1.0, 0.0],
            vec![0.0f32, 0.0, 1.0],
        ];
        let outcome = StrictAlignmentProjector
            .project(&centroid, &anchors, 2.0, &cfg())
            .unwrap();
        let sum: f32 = outcome.anchor_weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn projection_negative_cosines_clamped_to_zero() {
        // Two anchors: one aligned, one anti-aligned. The anti-aligned
        // one gets weight 0.
        let centroid = vec![1.0f32, 0.0];
        let anchors = vec![vec![1.0f32, 0.0], vec![-1.0, 0.0]];
        let outcome = StrictAlignmentProjector
            .project(&centroid, &anchors, 1.0, &cfg())
            .unwrap();
        assert!((outcome.anchor_weights[0] - 1.0).abs() < 1e-5);
        assert!(outcome.anchor_weights[1].abs() < 1e-6);
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(StrictAlignmentProjector.name(), "strict-alignment");
    }
}
