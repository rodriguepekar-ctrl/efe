//! Validator pur — wrapper autour de [`PolicyEngine::check`].
//!
//! Exposé pour les callers qui ne veulent pas instancier un moteur à
//! chaque appel (ex. tests, composition), ou pour valider un cône dérivé
//! avant de le soumettre à `register`/`evolve`.
//!
//! # Zéro état, zéro I/O
//!
//! La fonction est pure : mêmes entrées ⇒ même résultat. Pas d'allocation
//! sur le chemin succès hors structures déjà tenues par le cône.

use crate::alignment::policy::{PolicyConfig, PolicyEngine};
use crate::errors::Result;
use crate::types::AlignmentCone;

/// Valide un cône contre la policy donnée.
///
/// # Errors
///
/// Propage l'erreur [`crate::errors::EfeError::InvalidConfig`] renvoyée
/// par [`PolicyEngine::check`] si un invariant (I1..I7) est violé.
pub fn validate_cone(cone: &AlignmentCone, policy: &PolicyConfig) -> Result<()> {
    PolicyEngine::new(policy.clone()).check(cone)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_valid_cone_passes() {
        let cone = AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap();
        validate_cone(&cone, &PolicyConfig::default()).unwrap();
    }

    #[test]
    fn validate_forwards_policy_error() {
        let cfg = PolicyConfig {
            max_anchors: 1,
            ..PolicyConfig::default()
        };
        let cone = AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap();
        assert!(validate_cone(&cone, &cfg).is_err());
    }

    #[test]
    fn validate_empty_dim_fails() {
        let cone = AlignmentCone::new(vec![vec![]], 0.5);
        // AlignmentCone::new could accept empty inner vecs depending on
        // its validation ; let us check both branches.
        if let Ok(cone) = cone {
            let err = validate_cone(&cone, &PolicyConfig::default()).unwrap_err();
            assert!(err.to_string().contains("I2"));
        }
        // If rejected at cone construction, the test is vacuously true.
    }

    #[test]
    fn validate_nan_anchor_rejected() {
        let cone = AlignmentCone::new(vec![vec![f32::NAN, 0.0], vec![0.0, 1.0]], 0.5).unwrap();
        let err = validate_cone(&cone, &PolicyConfig::default()).unwrap_err();
        assert!(err.to_string().contains("I7"));
    }

    #[test]
    fn validate_is_pure_no_mutation() {
        let cone = AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap();
        let snapshot = format!("{cone:?}");
        let _ = validate_cone(&cone, &PolicyConfig::default());
        assert_eq!(format!("{cone:?}"), snapshot);
    }
}
