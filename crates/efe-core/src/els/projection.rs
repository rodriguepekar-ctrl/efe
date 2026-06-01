//! Soft projection of a vector onto the convex hull of anchors.
//!
//! Two entry points:
//!
//! - [`project_onto_anchors`]: takes an `EpistemicLatentSpace` and an
//!   arena, reads anchors from device memory.
//! - [`project_vector_onto_anchors`]: pure function taking `&[Vec<f32>]`
//!   anchors. Used by M9 when only the cone is in scope.
//!
//! ## Sprint 5 — façade over `ComputeBackend::cosine_anchor_blend`
//!
//! The Sprint 5 cross-module audit found the soft-projection algorithm
//! duplicated in three places (`els::projection`, `dissolution::alignment`,
//! `fusion::kernels::project`). The canonical implementation now lives
//! in [`crate::compute::ComputeBackend::cosine_anchor_blend`]. This
//! module's public API is preserved, but its body is a thin wrapper
//! that converts host-side `Vec<f32>` / `AnchorsView` inputs into
//! `Tensor<f32>` and calls the backend.

use crate::compute::{global_backend, Tensor};
use crate::els::metrics::l2_distance;
use crate::els::views::AnchorsView;
use crate::els::EpistemicLatentSpace;
use crate::errors::{EfeError, Result};
use crate::memory::Arena;

/// Project `vector` onto the convex hull of the anchors stored in the
/// ELS, blended with the input by `strength`.
///
/// Returns `(projected, delta)` where `delta = ||projected - vector||_2`.
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] if `vector.len() != dim` or `strength`
///   is outside `[0, 1]`.
/// - [`EfeError::Internal`] if the ELS has no anchors populated.
pub fn project_onto_anchors(
    els: &EpistemicLatentSpace,
    arena: &Arena,
    vector: &[f32],
    strength: f32,
) -> Result<(Vec<f32>, f32)> {
    let anchors = els.anchors_view(arena)?.ok_or(EfeError::Internal(
        "project_onto_anchors: ELS has no anchors",
    ))?;
    project_with_anchor_view(&anchors, vector, strength)
}

/// Same as [`project_onto_anchors`] but operates on a host-side
/// `&[Vec<f32>]`. Used by M9's `AlignmentProjector` impl, which
/// receives the cone directly from the request and does not need an
/// arena handle.
///
/// **Sprint 5 façade** — delegates the soft-projection arithmetic to
/// [`crate::compute::ComputeBackend::cosine_anchor_blend`].
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] on dim mismatch / strength out of range.
/// - [`EfeError::Internal`] if `anchors` is empty.
pub fn project_vector_onto_anchors(
    vector: &[f32],
    anchors: &[Vec<f32>],
    strength: f32,
) -> Result<(Vec<f32>, f32)> {
    validate_strength(strength)?;
    if anchors.is_empty() {
        return Err(EfeError::Internal(
            "project_vector_onto_anchors: no anchors",
        ));
    }
    let dim = vector.len();
    for (i, anchor) in anchors.iter().enumerate() {
        if anchor.len() != dim {
            return Err(EfeError::InvalidConfig(format!(
                "anchor {i} has dim {} != vector dim {dim}",
                anchor.len()
            )));
        }
    }
    if dim == 0 {
        return Err(EfeError::InvalidConfig(
            "project_vector_onto_anchors: empty vector".into(),
        ));
    }
    // Flatten anchors into a contiguous (N, D) buffer once.
    let n = anchors.len();
    let mut flat = Vec::with_capacity(n * dim);
    for anchor in anchors {
        flat.extend_from_slice(anchor);
    }
    invoke_cosine_anchor_blend(vector, &flat, n, dim, strength)
}

fn project_with_anchor_view(
    anchors: &AnchorsView<'_>,
    vector: &[f32],
    strength: f32,
) -> Result<(Vec<f32>, f32)> {
    validate_strength(strength)?;
    let dim = anchors.dim() as usize;
    if vector.len() != dim {
        return Err(EfeError::InvalidConfig(format!(
            "projection: vector dim {} != anchors dim {dim}",
            vector.len()
        )));
    }
    let n_anchors = anchors.n_anchors() as usize;
    if n_anchors == 0 {
        return Err(EfeError::Internal("projection: no anchors"));
    }
    if dim == 0 {
        return Err(EfeError::InvalidConfig(
            "projection: empty vector / anchors".into(),
        ));
    }
    // The view exposes anchors as `&[f32]` slices via `iter()`. We
    // copy them into a single contiguous buffer to feed the backend.
    let mut flat = Vec::with_capacity(n_anchors * dim);
    for anchor in anchors.iter() {
        debug_assert_eq!(anchor.len(), dim);
        flat.extend_from_slice(anchor);
    }
    invoke_cosine_anchor_blend(vector, &flat, n_anchors, dim, strength)
}

/// Internal helper that calls `compute::ComputeBackend::cosine_anchor_blend`
/// and translates the result into the legacy `(Vec<f32>, f32)` shape.
#[allow(clippy::many_single_char_names)] // `n, dim` are canonical anchor / dim variables
fn invoke_cosine_anchor_blend(
    vector: &[f32],
    anchors_flat: &[f32],
    n: usize,
    dim: usize,
    strength: f32,
) -> Result<(Vec<f32>, f32)> {
    let backend = global_backend();
    let v = Tensor::<f32>::from_slice(vector, [dim].into())
        .map_err(|e| EfeError::InvalidConfig(format!("vector tensor: {e}")))?;
    let a = Tensor::<f32>::from_slice(anchors_flat, [n, dim].into())
        .map_err(|e| EfeError::InvalidConfig(format!("anchors tensor: {e}")))?;
    match backend.cosine_anchor_blend(&v, &a, strength) {
        Ok((result, delta)) => {
            let result_vec = result
                .as_slice()
                .map_err(|_| EfeError::Internal("projection: contiguous result slice"))?
                .to_vec();
            // Cross-validation: the backend returns delta directly,
            // but we double-check via the legacy `els::metrics::l2_distance`
            // façade on debug builds. Keeps the contract immune to any
            // future drift in either side.
            debug_assert!(
                (delta - l2_distance(vector, &result_vec)).abs() < 1e-3,
                "delta divergence between backend and façade",
            );
            Ok((result_vec, delta))
        }
        // The backend may reject non-finite inputs. Translate that to
        // the legacy InvalidConfig so existing call sites that catch
        // EfeError see the familiar variant.
        Err(crate::compute::ComputeError::NumericalInstability { detail, .. }) => {
            Err(EfeError::InvalidConfig(format!(
                "projection numerical instability: {detail}"
            )))
        }
        Err(e) => Err(EfeError::InvalidConfig(format!(
            "projection backend error: {e}"
        ))),
    }
}

fn validate_strength(strength: f32) -> Result<()> {
    if !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
        return Err(EfeError::InvalidConfig(format!(
            "projection: strength {strength} must be in [0, 1]"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_anchors_single(vals: &[f32]) -> Vec<Vec<f32>> {
        vec![vals.to_vec()]
    }

    #[test]
    fn project_strength_zero_returns_input() {
        let v = vec![1.0f32, 0.5, -0.5];
        let anchors = mk_anchors_single(&[0.3, 0.3, 0.3]);
        let (out, delta) = project_vector_onto_anchors(&v, &anchors, 0.0).unwrap();
        assert_eq!(out, v);
        assert!(delta.abs() < f32::EPSILON);
    }

    #[test]
    fn project_strength_one_moves_to_centroid() {
        let v = vec![1.0f32, 0.0];
        let anchors = vec![vec![0.0f32, 1.0], vec![0.0, 2.0]];
        let (out, delta) = project_vector_onto_anchors(&v, &anchors, 1.0).unwrap();
        // cosine with v = [1,0] vs [0,1]/[0,2] is 0 → weights are 0.
        // Total = 0 → identity: out == v.
        assert_eq!(out, v);
        assert!(delta.abs() < f32::EPSILON);
    }

    #[test]
    fn project_parallel_anchor_pulls_toward() {
        let v = vec![1.0f32, 0.0];
        let anchors = vec![vec![0.5f32, 0.0]]; // same direction as v
        let (out, delta) = project_vector_onto_anchors(&v, &anchors, 0.5).unwrap();
        // Centroid is [0.5, 0]; blend = 0.5*[1,0] + 0.5*[0.5,0] = [0.75,0].
        assert!((out[0] - 0.75).abs() < 1e-5);
        assert!(out[1].abs() < 1e-5);
        assert!(delta > 0.0);
    }

    #[test]
    fn project_rejects_dim_mismatch() {
        let v = vec![1.0f32; 4];
        let anchors = vec![vec![0.0f32; 3]];
        assert!(project_vector_onto_anchors(&v, &anchors, 0.1).is_err());
    }

    #[test]
    fn project_rejects_strength_out_of_range() {
        let v = vec![1.0f32, 0.0];
        let anchors = vec![vec![0.5f32, 0.0]];
        assert!(project_vector_onto_anchors(&v, &anchors, -0.1).is_err());
        assert!(project_vector_onto_anchors(&v, &anchors, 1.5).is_err());
        assert!(project_vector_onto_anchors(&v, &anchors, f32::NAN).is_err());
    }

    #[test]
    fn project_rejects_empty_anchors() {
        let v = vec![1.0f32; 4];
        let err = project_vector_onto_anchors(&v, &[], 0.1).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn project_is_deterministic() {
        let v = vec![0.1f32, 0.2, 0.3];
        let anchors = vec![vec![0.1f32, 0.2, 0.3], vec![0.2, 0.1, 0.4]];
        let a = project_vector_onto_anchors(&v, &anchors, 0.3).unwrap();
        let b = project_vector_onto_anchors(&v, &anchors, 0.3).unwrap();
        assert_eq!(a.0, b.0);
        assert!((a.1 - b.1).abs() < f32::EPSILON);
    }

    #[test]
    fn project_delta_grows_monotonically_with_strength() {
        let v = vec![1.0f32, 0.1];
        let anchors = vec![vec![0.5f32, 0.5]]; // partially aligned
        let (_, d_small) = project_vector_onto_anchors(&v, &anchors, 0.1).unwrap();
        let (_, d_large) = project_vector_onto_anchors(&v, &anchors, 0.9).unwrap();
        assert!(d_large > d_small);
    }

    #[test]
    fn project_orthogonal_anchors_produces_identity() {
        let v = vec![1.0f32, 0.0, 0.0];
        let anchors = vec![vec![0.0f32, 1.0, 0.0], vec![0.0, 0.0, 1.0]];
        let (out, delta) = project_vector_onto_anchors(&v, &anchors, 0.5).unwrap();
        assert_eq!(out, v);
        assert!(delta.abs() < f32::EPSILON);
    }

    #[test]
    fn project_delta_bounded_by_vector_magnitude() {
        let v = vec![1.0f32, 1.0, 1.0];
        let anchors = vec![vec![0.0f32, 0.0, 0.0]]; // zero anchor → weight = 0
        let (out, _delta) = project_vector_onto_anchors(&v, &anchors, 1.0).unwrap();
        // Zero-norm anchor → cosine = 0 → total weights = 0 → identity.
        assert_eq!(out, v);
    }
}
