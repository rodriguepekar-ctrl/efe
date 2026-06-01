//! Opérateurs de composition sur les cônes d'alignement.
//!
//! Trois fonctions publiques, toutes pures et non-mutantes :
//!
//! - [`union_cones`] — concatène les anchors, rayon = max.
//! - [`blend_cones`] — interpolation linéaire `(1-t)·a + t·b`.
//! - [`intersect_cones`] — garde les anchors de `a` avec un pair proche
//!   dans `b` (cosine ≥ seuil), rayon = min.
//!
//! Chaque fonction termine par [`AlignmentCone::new`] qui valide
//! `anchors non vides + rayon strictement positif fini`. Les invariants
//! de policy (K ≤ max_anchors, cosine duplication, etc.) ne sont pas
//! vérifiés ici — c'est la responsabilité du caller (typiquement via
//! `PolicyEngine::check` ou `AlignmentRegistry::register`).

use crate::compute::{global_backend, utils::pairwise_cosine};
use crate::errors::{EfeError, Result};
use crate::types::AlignmentCone;

// =====================================================================
// Union
// =====================================================================

/// Union de deux cônes : anchors concaténés, rayon = max.
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] si les dimensions latentes diffèrent.
/// - [`EfeError::InvalidConfig`] propagée si le résultat est rejeté par
///   [`AlignmentCone::new`] (anchors vides / rayon invalide).
pub fn union_cones(a: &AlignmentCone, b: &AlignmentCone) -> Result<AlignmentCone> {
    let da = infer_dim(a)?;
    let db = infer_dim(b)?;
    if da != db {
        return Err(EfeError::InvalidConfig(format!(
            "union_cones: dim mismatch {da} != {db}"
        )));
    }
    let mut anchors: Vec<Vec<f32>> = Vec::with_capacity(a.anchors.len() + b.anchors.len());
    anchors.extend(a.anchors.iter().cloned());
    anchors.extend(b.anchors.iter().cloned());
    let radius = a.max_projection_distance.max(b.max_projection_distance);
    AlignmentCone::new(anchors, radius)
}

// =====================================================================
// Blend
// =====================================================================

/// Interpolation linéaire positionnelle entre deux cônes de même
/// cardinalité et même dimension.
///
/// `blend_cones(a, b, 0) == a` (valeurs), `blend_cones(a, b, 1) == b`.
///
/// # Errors
///
/// - `t` hors `[0, 1]` ou non fini.
/// - `|a.anchors| != |b.anchors|`.
/// - Dimensions différentes.
pub fn blend_cones(a: &AlignmentCone, b: &AlignmentCone, t: f32) -> Result<AlignmentCone> {
    if !t.is_finite() || !(0.0..=1.0).contains(&t) {
        return Err(EfeError::InvalidConfig(format!(
            "blend_cones: t {t} must be finite in [0, 1]"
        )));
    }
    if a.anchors.len() != b.anchors.len() {
        return Err(EfeError::InvalidConfig(format!(
            "blend_cones: anchor count mismatch ({} vs {})",
            a.anchors.len(),
            b.anchors.len()
        )));
    }
    let da = infer_dim(a)?;
    let db = infer_dim(b)?;
    if da != db {
        return Err(EfeError::InvalidConfig(format!(
            "blend_cones: dim mismatch {da} != {db}"
        )));
    }
    let one_minus_t = 1.0 - t;
    let anchors: Vec<Vec<f32>> = a
        .anchors
        .iter()
        .zip(&b.anchors)
        .map(|(ai, bi)| {
            ai.iter()
                .zip(bi)
                .map(|(x, y)| one_minus_t * x + t * y)
                .collect::<Vec<f32>>()
        })
        .collect();
    let radius = one_minus_t * a.max_projection_distance + t * b.max_projection_distance;
    AlignmentCone::new(anchors, radius)
}

// =====================================================================
// Intersect
// =====================================================================

/// Intersection : conserve les anchors de `a` qui ont un partenaire dans
/// `b` à cosine ≥ `threshold`. Rayon = min.
///
/// # Errors
///
/// - Dim mismatch.
/// - `threshold` hors `(-1, 1]` ou non fini.
/// - Aucune ancre retenue (intersection vide) ⇒
///   [`EfeError::InvalidConfig`].
pub fn intersect_cones(
    a: &AlignmentCone,
    b: &AlignmentCone,
    threshold: f32,
) -> Result<AlignmentCone> {
    if !threshold.is_finite() || threshold <= -1.0 || threshold > 1.0 {
        return Err(EfeError::InvalidConfig(format!(
            "intersect_cones: threshold {threshold} must be finite in (-1, 1]"
        )));
    }
    let da = infer_dim(a)?;
    let db = infer_dim(b)?;
    if da != db {
        return Err(EfeError::InvalidConfig(format!(
            "intersect_cones: dim mismatch {da} != {db}"
        )));
    }

    // Sprint 5 — full pairwise cosine matrix via the canonical backend.
    // The previous implementation rolled its own dot/norm arithmetic,
    // duplicating the now-shared `compute::ComputeBackend::cosine_similarity_pair`.
    // The matrix has dimensions `(|a.anchors|, |b.anchors|)`. For
    // typical sizes (`K ≤ 32`) the cost is negligible; for hot paths
    // we'll add a batched op in a future sprint.
    let backend = global_backend();
    let lhs_refs: Vec<&[f32]> = a.anchors.iter().map(Vec::as_slice).collect();
    let rhs_refs: Vec<&[f32]> = b.anchors.iter().map(Vec::as_slice).collect();
    let cos_matrix = pairwise_cosine(&*backend, &lhs_refs, &rhs_refs)
        .map_err(|e| EfeError::InvalidConfig(format!(
            "intersect_cones: pairwise cosine failed: {e}"
        )))?;

    let mut kept: Vec<Vec<f32>> = Vec::new();
    for (i, ai) in a.anchors.iter().enumerate() {
        // Geometric limit: zero-norm anchors return cos == 0 from the
        // backend, which won't satisfy any positive threshold ⇒ skipped
        // implicitly via the inner loop.
        for &cos in &cos_matrix[i] {
            if cos.is_finite() && cos >= threshold {
                kept.push(ai.clone());
                break;
            }
        }
    }

    if kept.is_empty() {
        return Err(EfeError::InvalidConfig(
            "intersect_cones: empty intersection (no matching anchor pairs)".into(),
        ));
    }
    let radius = a.max_projection_distance.min(b.max_projection_distance);
    AlignmentCone::new(kept, radius)
}

// =====================================================================
// Helpers
// =====================================================================

fn infer_dim(cone: &AlignmentCone) -> Result<usize> {
    let first = cone.anchors.first().ok_or(EfeError::Internal(
        "compose: AlignmentCone with no anchors reached compose layer",
    ))?;
    let d = first.len();
    for a in &cone.anchors {
        if a.len() != d {
            return Err(EfeError::InvalidConfig(format!(
                "compose: anchor dim mismatch ({d} vs {})",
                a.len()
            )));
        }
    }
    Ok(d)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn cone(anchors: Vec<Vec<f32>>, r: f32) -> AlignmentCone {
        AlignmentCone::new(anchors, r).unwrap()
    }

    // --- Union ---

    #[test]
    fn union_concatenates_anchors() {
        let a = cone(vec![vec![1.0, 0.0]], 0.3);
        let b = cone(vec![vec![0.0, 1.0]], 0.5);
        let u = union_cones(&a, &b).unwrap();
        assert_eq!(u.anchors.len(), 2);
        assert!((u.max_projection_distance - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn union_radius_is_max() {
        let a = cone(vec![vec![1.0, 0.0]], 0.7);
        let b = cone(vec![vec![0.0, 1.0]], 0.2);
        let u = union_cones(&a, &b).unwrap();
        assert!((u.max_projection_distance - 0.7).abs() < f32::EPSILON);
    }

    #[test]
    fn union_rejects_dim_mismatch() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![1.0, 0.0, 0.0]], 0.5);
        assert!(union_cones(&a, &b).is_err());
    }

    #[test]
    fn union_does_not_mutate_inputs() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![0.0, 1.0]], 0.5);
        let snap_a = format!("{a:?}");
        let snap_b = format!("{b:?}");
        let _ = union_cones(&a, &b).unwrap();
        assert_eq!(format!("{a:?}"), snap_a);
        assert_eq!(format!("{b:?}"), snap_b);
    }

    #[test]
    fn union_is_commutative_up_to_anchor_order() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![0.0, 1.0]], 0.5);
        let u1 = union_cones(&a, &b).unwrap();
        let u2 = union_cones(&b, &a).unwrap();
        assert_eq!(u1.anchors.len(), u2.anchors.len());
        // Same radius.
        assert!((u1.max_projection_distance - u2.max_projection_distance).abs() < f32::EPSILON);
    }

    #[test]
    fn union_idempotent_on_identical() {
        let a = cone(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5);
        let u = union_cones(&a, &a).unwrap();
        // K doubles.
        assert_eq!(u.anchors.len(), 4);
        assert!((u.max_projection_distance - 0.5).abs() < f32::EPSILON);
    }

    // --- Blend ---

    #[test]
    fn blend_t0_returns_a_values() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![0.0, 1.0]], 0.9);
        let r = blend_cones(&a, &b, 0.0).unwrap();
        assert!((r.anchors[0][0] - 1.0).abs() < 1e-6);
        assert!((r.anchors[0][1] - 0.0).abs() < 1e-6);
        assert!((r.max_projection_distance - 0.5).abs() < 1e-6);
    }

    #[test]
    fn blend_t1_returns_b_values() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![0.0, 1.0]], 0.9);
        let r = blend_cones(&a, &b, 1.0).unwrap();
        assert!((r.anchors[0][0] - 0.0).abs() < 1e-6);
        assert!((r.anchors[0][1] - 1.0).abs() < 1e-6);
        assert!((r.max_projection_distance - 0.9).abs() < 1e-6);
    }

    #[test]
    fn blend_t05_is_midpoint() {
        let a = cone(vec![vec![1.0, 0.0]], 0.4);
        let b = cone(vec![vec![0.0, 1.0]], 0.6);
        let r = blend_cones(&a, &b, 0.5).unwrap();
        assert!((r.anchors[0][0] - 0.5).abs() < 1e-6);
        assert!((r.anchors[0][1] - 0.5).abs() < 1e-6);
        assert!((r.max_projection_distance - 0.5).abs() < 1e-6);
    }

    #[test]
    fn blend_rejects_t_out_of_range() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![0.0, 1.0]], 0.5);
        assert!(blend_cones(&a, &b, -0.1).is_err());
        assert!(blend_cones(&a, &b, 1.1).is_err());
        assert!(blend_cones(&a, &b, f32::NAN).is_err());
    }

    #[test]
    fn blend_rejects_count_mismatch() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5);
        assert!(blend_cones(&a, &b, 0.5).is_err());
    }

    #[test]
    fn blend_rejects_dim_mismatch() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![1.0, 0.0, 0.0]], 0.5);
        assert!(blend_cones(&a, &b, 0.5).is_err());
    }

    // --- Intersect ---

    #[test]
    fn intersect_self_with_self_keeps_all() {
        let a = cone(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5);
        let r = intersect_cones(&a, &a, 0.5).unwrap();
        assert_eq!(r.anchors.len(), 2);
    }

    #[test]
    fn intersect_orthogonal_is_empty() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![0.0, 1.0]], 0.5);
        // cosine = 0 < 0.5 threshold.
        let err = intersect_cones(&a, &b, 0.5).unwrap_err();
        assert!(err.to_string().contains("empty intersection"));
    }

    #[test]
    fn intersect_radius_is_min() {
        let a = cone(vec![vec![1.0, 0.0]], 0.3);
        let b = cone(vec![vec![1.0, 0.0]], 0.9);
        let r = intersect_cones(&a, &b, 0.5).unwrap();
        assert!((r.max_projection_distance - 0.3).abs() < 1e-6);
    }

    #[test]
    fn intersect_rejects_bad_threshold() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![1.0, 0.0]], 0.5);
        assert!(intersect_cones(&a, &b, 2.0).is_err());
        assert!(intersect_cones(&a, &b, -2.0).is_err());
        assert!(intersect_cones(&a, &b, f32::NAN).is_err());
    }

    #[test]
    fn intersect_keeps_only_matching() {
        let a = cone(vec![vec![1.0, 0.0], vec![0.0, 1.0], vec![0.7, 0.7]], 0.5);
        let b = cone(vec![vec![1.0, 0.0]], 0.5);
        // threshold = 0.6 : only vec[1,0] (cos=1) and vec[0.7,0.7] (cos≈0.707) match.
        let r = intersect_cones(&a, &b, 0.6).unwrap();
        assert_eq!(r.anchors.len(), 2);
    }

    #[test]
    fn intersect_does_not_mutate_inputs() {
        let a = cone(vec![vec![1.0, 0.0]], 0.5);
        let b = cone(vec![vec![1.0, 0.0]], 0.5);
        let snap_a = format!("{a:?}");
        let snap_b = format!("{b:?}");
        let _ = intersect_cones(&a, &b, 0.5);
        assert_eq!(format!("{a:?}"), snap_a);
        assert_eq!(format!("{b:?}"), snap_b);
    }

    // Intersection avec 3+ anchors qui matchent chacun : on les garde tous.
    #[test]
    fn intersect_keeps_all_three_matching_pairs() {
        let a = cone(
            vec![
                vec![1.0, 0.0, 0.0],
                vec![0.0, 1.0, 0.0],
                vec![0.0, 0.0, 1.0],
            ],
            0.5,
        );
        let b = cone(
            vec![
                vec![0.9, 0.1, 0.0],
                vec![0.1, 0.9, 0.0],
                vec![0.0, 0.1, 0.9],
            ],
            0.4,
        );
        let r = intersect_cones(&a, &b, 0.85).unwrap();
        assert_eq!(r.anchors.len(), 3);
        assert!((r.max_projection_distance - 0.4).abs() < 1e-6);
    }

    // Intersection avec `threshold = 1.0` : seuls les anchors
    // parfaitement parallèles matchent. Utile en debug.
    #[test]
    fn intersect_threshold_one_accepts_only_parallel() {
        let a = cone(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5);
        let b = cone(vec![vec![1.0, 0.0]], 0.5);
        let r = intersect_cones(&a, &b, 1.0).unwrap();
        // cos(a[0], b[0]) = 1 exactement ; cos(a[1], b[0]) = 0.
        assert_eq!(r.anchors.len(), 1);
        assert_eq!(r.anchors[0], vec![1.0, 0.0]);
    }
}
