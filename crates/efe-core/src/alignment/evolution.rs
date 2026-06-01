//! Opérations d'évolution d'un cône.
//!
//! Chaque opération transforme un `AlignmentCone` existant en un nouveau
//! cône. Les helpers [`apply_append`], [`apply_remove`],
//! [`apply_replace`], [`apply_update_radius`], [`apply_scale`] sont
//! **purs** et ne dépendent pas du registre.
//!
//! L'opération [`ConeEvolutionOp::UnionWith`] nécessite un lookup dans
//! le registre (pour récupérer le cône de l'autre clé) — elle est donc
//! appliquée à l'intérieur de [`crate::alignment::AlignmentRegistry::evolve`]
//! via [`crate::alignment::compose::union_cones`], pas par
//! [`ConeEvolutionOp::apply`].

use serde::{Deserialize, Serialize};

use crate::alignment::key::ConeKey;
use crate::errors::{EfeError, Result};
use crate::types::AlignmentCone;

/// Opération atomique de modification.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum ConeEvolutionOp {
    /// Ajoute un nouvel anchor à la fin de la liste.
    AppendAnchor {
        /// Anchor à ajouter. Sa dimension doit correspondre à celle du cône.
        anchor: Vec<f32>,
    },
    /// Retire l'anchor à l'index donné. Refuse si K == 1.
    RemoveAnchor {
        /// Index à retirer. Doit être `< K`.
        idx: usize,
    },
    /// Remplace l'anchor à l'index donné par un nouveau.
    ReplaceAnchor {
        /// Index à remplacer.
        idx: usize,
        /// Nouveau vecteur.
        anchor: Vec<f32>,
    },
    /// Change le rayon de projection.
    UpdateRadius {
        /// Nouveau rayon (doit être > 0 et fini).
        new_radius: f32,
    },
    /// Multiplie tous les anchors par un facteur fini non nul.
    ScaleAnchors {
        /// Facteur (ex. `2.0`, `0.5`).
        factor: f32,
    },
    /// Union avec un autre cône référencé par sa clé. Le lookup est
    /// effectué par le registre ; cette variante n'est pas applicable
    /// via [`ConeEvolutionOp::apply`] (voir la doc du module).
    UnionWith {
        /// Clé du cône à unir.
        other: ConeKey,
    },
}

impl ConeEvolutionOp {
    /// Étiquette courte pour télémétrie (stable à travers les versions
    /// mineures de la crate).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AppendAnchor { .. } => "append_anchor",
            Self::RemoveAnchor { .. } => "remove_anchor",
            Self::ReplaceAnchor { .. } => "replace_anchor",
            Self::UpdateRadius { .. } => "update_radius",
            Self::ScaleAnchors { .. } => "scale_anchors",
            Self::UnionWith { .. } => "union_with",
        }
    }

    /// Applique l'opération **locale** (toutes sauf `UnionWith`) à un
    /// cône, produisant un nouveau cône.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] si l'opération est invalide
    ///   (dim mismatch, idx hors bornes, ratio zéro, etc.).
    /// - [`EfeError::Internal`] si `UnionWith` est passé à `apply` —
    ///   cette variante doit être gérée par le registre.
    pub fn apply(&self, base: &AlignmentCone) -> Result<AlignmentCone> {
        match self {
            Self::AppendAnchor { anchor } => apply_append(base, anchor),
            Self::RemoveAnchor { idx } => apply_remove(base, *idx),
            Self::ReplaceAnchor { idx, anchor } => apply_replace(base, *idx, anchor),
            Self::UpdateRadius { new_radius } => apply_update_radius(base, *new_radius),
            Self::ScaleAnchors { factor } => apply_scale(base, *factor),
            Self::UnionWith { .. } => Err(EfeError::Internal(
                "ConeEvolutionOp::UnionWith must be applied by the registry, not apply()",
            )),
        }
    }
}

// =====================================================================
// Helpers (purs, testables en isolation)
// =====================================================================

/// Ajoute un anchor. La dim doit coïncider.
///
/// # Errors
///
/// [`EfeError::InvalidConfig`] si dim mismatch, ou si le nouveau cône
/// est rejeté par [`AlignmentCone::new`].
pub fn apply_append(base: &AlignmentCone, anchor: &[f32]) -> Result<AlignmentCone> {
    let d = base_dim(base)?;
    if anchor.len() != d {
        return Err(EfeError::InvalidConfig(format!(
            "append: anchor dim {} != cone dim {d}",
            anchor.len()
        )));
    }
    let mut anchors = base.anchors.clone();
    anchors.push(anchor.to_vec());
    AlignmentCone::new(anchors, base.max_projection_distance)
}

/// Supprime l'anchor à l'index. Refus si K == 1.
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] si `idx >= K`.
/// - [`EfeError::InvalidConfig`] si le cône ne contient qu'un anchor
///   (suppression interdite).
pub fn apply_remove(base: &AlignmentCone, idx: usize) -> Result<AlignmentCone> {
    let k = base.anchors.len();
    if idx >= k {
        return Err(EfeError::InvalidConfig(format!(
            "remove: idx {idx} out of bounds for K={k}"
        )));
    }
    if k <= 1 {
        return Err(EfeError::InvalidConfig(
            "remove: refusing to empty the cone (K == 1)".into(),
        ));
    }
    let mut anchors = base.anchors.clone();
    anchors.remove(idx);
    AlignmentCone::new(anchors, base.max_projection_distance)
}

/// Remplace l'anchor à l'index donné par un nouveau vecteur.
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] si `idx >= K`.
/// - [`EfeError::InvalidConfig`] si dim mismatch.
pub fn apply_replace(base: &AlignmentCone, idx: usize, anchor: &[f32]) -> Result<AlignmentCone> {
    let k = base.anchors.len();
    if idx >= k {
        return Err(EfeError::InvalidConfig(format!(
            "replace: idx {idx} out of bounds for K={k}"
        )));
    }
    let d = base_dim(base)?;
    if anchor.len() != d {
        return Err(EfeError::InvalidConfig(format!(
            "replace: anchor dim {} != cone dim {d}",
            anchor.len()
        )));
    }
    let mut anchors = base.anchors.clone();
    anchors[idx] = anchor.to_vec();
    AlignmentCone::new(anchors, base.max_projection_distance)
}

/// Change le rayon. `AlignmentCone::new` re-vérifie `> 0 && finite`.
///
/// # Errors
///
/// [`EfeError::InvalidConfig`] si le rayon est invalide.
pub fn apply_update_radius(base: &AlignmentCone, new_radius: f32) -> Result<AlignmentCone> {
    AlignmentCone::new(base.anchors.clone(), new_radius)
}

/// Multiplie tous les anchors par un facteur.
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] si `factor` non fini ou nul.
pub fn apply_scale(base: &AlignmentCone, factor: f32) -> Result<AlignmentCone> {
    if !factor.is_finite() || factor == 0.0 {
        return Err(EfeError::InvalidConfig(format!(
            "scale: factor {factor} must be finite and non-zero"
        )));
    }
    let anchors: Vec<Vec<f32>> = base
        .anchors
        .iter()
        .map(|a| a.iter().map(|v| v * factor).collect())
        .collect();
    AlignmentCone::new(anchors, base.max_projection_distance)
}

fn base_dim(cone: &AlignmentCone) -> Result<usize> {
    cone.anchors.first().map(Vec::len).ok_or(EfeError::Internal(
        "evolution::base_dim: cone has no anchors",
    ))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> AlignmentCone {
        AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap()
    }

    // --- kind() ---

    #[test]
    fn kind_strings() {
        assert_eq!(
            ConeEvolutionOp::AppendAnchor {
                anchor: vec![1.0, 0.0]
            }
            .kind(),
            "append_anchor"
        );
        assert_eq!(
            ConeEvolutionOp::RemoveAnchor { idx: 0 }.kind(),
            "remove_anchor"
        );
        assert_eq!(
            ConeEvolutionOp::UpdateRadius { new_radius: 1.0 }.kind(),
            "update_radius"
        );
    }

    // --- append ---

    #[test]
    fn append_adds_anchor() {
        let new = apply_append(&base(), &[0.5, 0.5]).unwrap();
        assert_eq!(new.anchors.len(), 3);
        assert_eq!(new.anchors[2], vec![0.5, 0.5]);
    }

    #[test]
    fn append_rejects_dim_mismatch() {
        assert!(apply_append(&base(), &[0.5, 0.5, 0.5]).is_err());
    }

    #[test]
    fn append_preserves_radius() {
        let new = apply_append(&base(), &[0.5, 0.5]).unwrap();
        assert!((new.max_projection_distance - 0.5).abs() < f32::EPSILON);
    }

    // --- remove ---

    #[test]
    fn remove_decreases_k() {
        let new = apply_remove(&base(), 0).unwrap();
        assert_eq!(new.anchors.len(), 1);
        assert_eq!(new.anchors[0], vec![0.0, 1.0]);
    }

    #[test]
    fn remove_out_of_bounds_rejected() {
        assert!(apply_remove(&base(), 5).is_err());
    }

    #[test]
    fn remove_last_anchor_rejected() {
        let cone = AlignmentCone::new(vec![vec![1.0, 0.0]], 0.5).unwrap();
        assert!(apply_remove(&cone, 0).is_err());
    }

    // --- replace ---

    #[test]
    fn replace_changes_anchor() {
        let new = apply_replace(&base(), 1, &[0.3, 0.7]).unwrap();
        assert_eq!(new.anchors.len(), 2);
        assert_eq!(new.anchors[1], vec![0.3, 0.7]);
    }

    #[test]
    fn replace_rejects_dim_mismatch() {
        assert!(apply_replace(&base(), 0, &[1.0]).is_err());
    }

    #[test]
    fn replace_rejects_out_of_bounds() {
        assert!(apply_replace(&base(), 5, &[1.0, 0.0]).is_err());
    }

    // --- update_radius ---

    #[test]
    fn update_radius_valid() {
        let new = apply_update_radius(&base(), 1.2).unwrap();
        assert!((new.max_projection_distance - 1.2).abs() < f32::EPSILON);
        assert_eq!(new.anchors.len(), 2);
    }

    #[test]
    fn update_radius_rejects_zero() {
        assert!(apply_update_radius(&base(), 0.0).is_err());
    }

    #[test]
    fn update_radius_rejects_nan() {
        assert!(apply_update_radius(&base(), f32::NAN).is_err());
    }

    // --- scale ---

    #[test]
    fn scale_multiplies_anchors() {
        let new = apply_scale(&base(), 2.0).unwrap();
        assert_eq!(new.anchors[0], vec![2.0, 0.0]);
        assert_eq!(new.anchors[1], vec![0.0, 2.0]);
    }

    #[test]
    fn scale_rejects_zero() {
        assert!(apply_scale(&base(), 0.0).is_err());
    }

    #[test]
    fn scale_rejects_nan() {
        assert!(apply_scale(&base(), f32::NAN).is_err());
    }

    #[test]
    fn scale_preserves_radius() {
        let new = apply_scale(&base(), 3.0).unwrap();
        assert!((new.max_projection_distance - 0.5).abs() < f32::EPSILON);
    }

    // Identité algébrique : scale par 1.0 ne modifie rien.
    #[test]
    fn scale_factor_one_is_identity() {
        let b = base();
        let new = apply_scale(&b, 1.0).unwrap();
        for (a, b) in new.anchors.iter().zip(b.anchors.iter()) {
            assert_eq!(a, b);
        }
        assert!((new.max_projection_distance - b.max_projection_distance).abs() < f32::EPSILON);
    }

    // Scale par facteur négatif : anchors inversés. Preserve la norme.
    #[test]
    fn scale_negative_factor_allowed() {
        let new = apply_scale(&base(), -1.0).unwrap();
        assert_eq!(new.anchors[0], vec![-1.0, -0.0]);
        assert_eq!(new.anchors[1], vec![-0.0, -1.0]);
    }

    // --- apply() dispatcher ---

    #[test]
    fn apply_dispatches_to_helpers() {
        let b = base();
        let op = ConeEvolutionOp::AppendAnchor {
            anchor: vec![0.5, 0.5],
        };
        let new = op.apply(&b).unwrap();
        assert_eq!(new.anchors.len(), 3);
    }

    #[test]
    fn apply_union_with_errors_internal() {
        let b = base();
        let op = ConeEvolutionOp::UnionWith {
            other: ConeKey::new("t", "k", 1).unwrap(),
        };
        let err = op.apply(&b).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn apply_rejects_dim_mismatch_via_append() {
        let b = base();
        let op = ConeEvolutionOp::AppendAnchor { anchor: vec![1.0] };
        assert!(op.apply(&b).is_err());
    }

    #[test]
    fn ops_serde_roundtrip() {
        let op = ConeEvolutionOp::ReplaceAnchor {
            idx: 0,
            anchor: vec![1.0, 0.0, 0.0],
        };
        let s = serde_json::to_string(&op).unwrap();
        let back: ConeEvolutionOp = serde_json::from_str(&s).unwrap();
        assert_eq!(op, back);
    }
}
