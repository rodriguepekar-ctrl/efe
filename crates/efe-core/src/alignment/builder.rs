//! Extension `SwarmSpecBuilder::from_registry`.
//!
//! Point d'intégration **opt-in** et **non-breaking** entre M9 et le
//! builder de spec de M1. Le client peut soit fournir un cône inline via
//! [`crate::types::SwarmSpecBuilder::alignment_cone`] (comportement
//! historique), soit récupérer un cône gouverné via ce nouveau helper.
//!
//! # Exemple
//!
//! ```ignore
//! use efe_core::alignment::{AlignmentRegistry, ConeKey};
//! use efe_core::types::SwarmSpec;
//!
//! async fn make_spec(
//!     registry: &dyn AlignmentRegistry,
//!     key: &ConeKey,
//! ) -> efe_core::errors::Result<SwarmSpec> {
//!     SwarmSpec::builder()
//!         .problem_embedding(vec![0.0; 128])
//!         .dim_latent(efe_core::types::LatentDim::new(128)?)
//!         .population(efe_core::types::PopulationMix::new(
//!             vec![(efe_core::types::TemplateCode::NaSem, 4)])?)
//!         .from_registry(registry, key).await?
//!         .build()
//! }
//! ```

use crate::alignment::key::ConeKey;
use crate::alignment::registry::AlignmentRegistry;
use crate::errors::Result;
use crate::types::SwarmSpecBuilder;

/// Extension à `SwarmSpecBuilder` qui résout un cône depuis un registre.
///
/// Implémenté comme un `impl` inhérent séparé pour **ne pas toucher**
/// `types.rs` (zéro risque de régression sur l'existant).
impl SwarmSpecBuilder {
    /// Résout un cône à `key` depuis `registry` et l'injecte.
    ///
    /// # Errors
    ///
    /// Propage l'erreur du registre si `key` est inconnue, en conservant
    /// la variante [`crate::errors::EfeError::InvalidConfig`].
    pub async fn from_registry(
        self,
        registry: &(dyn AlignmentRegistry + '_),
        key: &ConeKey,
    ) -> Result<Self> {
        let handle = registry.get(key).await?;
        Ok(self.alignment_cone(handle.as_cone().clone()))
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alignment::canonical::CanonicalCone;
    use crate::alignment::registry::InMemoryRegistry;
    use crate::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};

    fn cone() -> AlignmentCone {
        AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap()
    }

    fn canonical() -> CanonicalCone {
        CanonicalCone::new(cone(), "test", "fixture").unwrap()
    }

    fn registry() -> InMemoryRegistry {
        InMemoryRegistry::with_default_config().unwrap()
    }

    #[tokio::test]
    async fn from_registry_happy_path() {
        let reg = registry();
        let key = ConeKey::new("acme", "task", 1).unwrap();
        reg.register(key.clone(), canonical()).await.unwrap();

        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 2])
            .dim_latent(LatentDim::new(2).unwrap())
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).unwrap())
            .max_rounds(4)
            .unwrap()
            .from_registry(&reg, &key)
            .await
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(spec.alignment_cone.anchors.len(), 2);
    }

    #[tokio::test]
    async fn from_registry_unknown_key_errors() {
        let reg = registry();
        let key = ConeKey::new("no", "such", 1).unwrap();
        let outcome = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 2])
            .dim_latent(LatentDim::new(2).unwrap())
            .from_registry(&reg, &key)
            .await;
        assert!(outcome.is_err());
    }

    #[tokio::test]
    async fn from_registry_identical_to_inline() {
        let reg = registry();
        let key = ConeKey::new("acme", "task", 1).unwrap();
        reg.register(key.clone(), canonical()).await.unwrap();
        let spec_reg = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 2])
            .dim_latent(LatentDim::new(2).unwrap())
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).unwrap())
            .max_rounds(4)
            .unwrap()
            .from_registry(&reg, &key)
            .await
            .unwrap()
            .build()
            .unwrap();
        let spec_inline = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 2])
            .dim_latent(LatentDim::new(2).unwrap())
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).unwrap())
            .max_rounds(4)
            .unwrap()
            .alignment_cone(cone())
            .build()
            .unwrap();
        assert_eq!(
            spec_reg.alignment_cone.anchors.len(),
            spec_inline.alignment_cone.anchors.len()
        );
        for (a, b) in spec_reg
            .alignment_cone
            .anchors
            .iter()
            .zip(spec_inline.alignment_cone.anchors.iter())
        {
            assert_eq!(a, b);
        }
    }

    #[tokio::test]
    async fn from_registry_can_be_overridden_by_inline() {
        // Si on appelle .alignment_cone(...) après from_registry, l'inline gagne.
        let reg = registry();
        let key = ConeKey::new("acme", "task", 1).unwrap();
        reg.register(key.clone(), canonical()).await.unwrap();
        let other_cone = AlignmentCone::new(vec![vec![0.5, 0.5]], 0.3).unwrap();
        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 2])
            .dim_latent(LatentDim::new(2).unwrap())
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).unwrap())
            .max_rounds(4)
            .unwrap()
            .from_registry(&reg, &key)
            .await
            .unwrap()
            .alignment_cone(other_cone)
            .build()
            .unwrap();
        assert_eq!(spec.alignment_cone.anchors.len(), 1);
        assert!((spec.alignment_cone.max_projection_distance - 0.3).abs() < 1e-6);
    }

    #[tokio::test]
    async fn from_registry_with_trait_object() {
        let reg: std::sync::Arc<dyn AlignmentRegistry> = std::sync::Arc::new(registry());
        let key = ConeKey::new("acme", "task", 1).unwrap();
        reg.register(key.clone(), canonical()).await.unwrap();
        let spec = SwarmSpec::builder()
            .problem_embedding(vec![0.0; 2])
            .dim_latent(LatentDim::new(2).unwrap())
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).unwrap())
            .max_rounds(4)
            .unwrap()
            .from_registry(&*reg, &key)
            .await
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(spec.alignment_cone.anchors.len(), 2);
    }
}
