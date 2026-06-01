//! `CanonicalCone`, `ConeMetadata`, `ConeHandle` — structure canonique
//! d'un cône stocké dans le registre.
//!
//! Un [`CanonicalCone`] est l'association d'un [`AlignmentCone`] (défini
//! dans M1) et de ses [`ConeMetadata`] (timestamp, parenté, description,
//! labels). Le [`ConeHandle`] est la référence comptée retournée par les
//! opérations du registre — il partage la charge mémoire entre lecteurs
//! via [`std::sync::Arc`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::alignment::evolution::ConeEvolutionOp;
use crate::alignment::key::ConeKey;
use crate::types::AlignmentCone;

/// Longueur max d'un `created_by`.
pub const CREATED_BY_MAX_LEN: usize = 128;
/// Longueur max d'une description.
pub const DESCRIPTION_MAX_LEN: usize = 512;
/// Nombre max d'étiquettes.
pub const LABELS_MAX_COUNT: usize = 16;
/// Longueur max d'une clé de label.
pub const LABEL_KEY_MAX_LEN: usize = 64;
/// Longueur max d'une valeur de label.
pub const LABEL_VALUE_MAX_LEN: usize = 256;

/// Métadonnées descriptives attachées à un `CanonicalCone`.
///
/// Les contraintes de longueur (`CREATED_BY_MAX_LEN`,
/// `DESCRIPTION_MAX_LEN`, `LABEL_*_MAX_LEN`, `LABELS_MAX_COUNT`) ne
/// sont vérifiées **que** par les constructeurs / builders ([`Self::new`],
/// [`Self::with_label`], [`Self::with_parent`]). Les champs publics
/// permettent la mutation ad-hoc — dans ce cas, l'appelant est
/// responsable de respecter les bornes. Le registre ne re-valide pas
/// la metadata au `register` : toute metadata hors-bornes passée à
/// travers un champ public sera stockée telle quelle.
///
/// **Règle interne** : toujours construire via les builders en crate
/// métier ; la mutation directe est réservée aux usages techniques
/// (fixtures de tests, désérialisation, migrations).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ConeMetadata {
    /// Nanos Unix epoch au moment de la création.
    pub created_at_nanos: u64,
    /// Identité qui a créé le cône (user, service, bot). Max 128 chars.
    pub created_by: String,
    /// Description humaine. Max 512 chars.
    pub description: String,
    /// Clé du parent si cette version résulte d'un `evolve`. Sinon `None`.
    pub parent: Option<ConeKey>,
    /// Opération qui a produit cette version (depuis `parent`). `None` si racine.
    pub evolution_op: Option<ConeEvolutionOp>,
    /// Étiquettes libres. Ordre stable (BTreeMap → serde déterministe).
    pub labels: BTreeMap<String, String>,
}

impl ConeMetadata {
    /// Construit une instance, validant les bornes de taille.
    ///
    /// # Errors
    ///
    /// [`crate::errors::EfeError::InvalidConfig`] si un champ dépasse sa
    /// longueur maximale.
    pub fn new(
        created_by: impl Into<String>,
        description: impl Into<String>,
    ) -> crate::errors::Result<Self> {
        let created_by = created_by.into();
        let description = description.into();
        check_len("created_by", &created_by, CREATED_BY_MAX_LEN)?;
        check_len("description", &description, DESCRIPTION_MAX_LEN)?;
        Ok(Self {
            created_at_nanos: now_nanos(),
            created_by,
            description,
            parent: None,
            evolution_op: None,
            labels: BTreeMap::new(),
        })
    }

    /// Ajoute une étiquette, renvoie `self` pour chaînage.
    ///
    /// # Errors
    ///
    /// [`crate::errors::EfeError::InvalidConfig`] si la clé ou la valeur
    /// dépasse sa longueur, ou si le nombre d'étiquettes excède
    /// [`LABELS_MAX_COUNT`].
    pub fn with_label(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> crate::errors::Result<Self> {
        let key = key.into();
        let value = value.into();
        check_len("label key", &key, LABEL_KEY_MAX_LEN)?;
        check_len("label value", &value, LABEL_VALUE_MAX_LEN)?;
        if self.labels.len() >= LABELS_MAX_COUNT && !self.labels.contains_key(&key) {
            return Err(crate::errors::EfeError::InvalidConfig(format!(
                "ConeMetadata: labels count would exceed max {LABELS_MAX_COUNT}"
            )));
        }
        self.labels.insert(key, value);
        Ok(self)
    }

    /// Attache une information de parenté. Utilisée par le flux `evolve`.
    #[must_use]
    pub fn with_parent(mut self, parent: ConeKey, op: ConeEvolutionOp) -> Self {
        self.parent = Some(parent);
        self.evolution_op = Some(op);
        self
    }
}

/// Cône canonical : données (`AlignmentCone`) + métadonnées.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CanonicalCone {
    /// Le cône d'alignement lui-même.
    pub cone: AlignmentCone,
    /// Métadonnées associées.
    pub metadata: ConeMetadata,
}

impl CanonicalCone {
    /// Constructeur ergonomique avec timestamp automatique.
    ///
    /// # Errors
    ///
    /// Propage [`crate::errors::EfeError::InvalidConfig`] depuis
    /// [`ConeMetadata::new`].
    pub fn new(
        cone: AlignmentCone,
        created_by: impl Into<String>,
        description: impl Into<String>,
    ) -> crate::errors::Result<Self> {
        Ok(Self {
            cone,
            metadata: ConeMetadata::new(created_by, description)?,
        })
    }

    /// Constructeur à partir d'une métadonnée préconstruite.
    #[must_use]
    pub fn with_metadata(cone: AlignmentCone, metadata: ConeMetadata) -> Self {
        Self { cone, metadata }
    }

    /// Nombre d'anchors (K).
    #[must_use]
    pub fn num_anchors(&self) -> usize {
        self.cone.anchors.len()
    }

    /// Dimension latente (D). Retourne 0 si aucun anchor (ne devrait pas
    /// se produire après [`AlignmentCone::new`]).
    #[must_use]
    pub fn dim(&self) -> usize {
        self.cone.anchors.first().map_or(0, Vec::len)
    }

    /// Rayon max de projection.
    #[must_use]
    pub fn radius(&self) -> f32 {
        self.cone.max_projection_distance
    }

    /// Accès direct aux anchors.
    #[must_use]
    pub fn anchors(&self) -> &[Vec<f32>] {
        &self.cone.anchors
    }
}

/// Référence comptée vers un cône canonical + sa clé.
///
/// Retourné par `AlignmentRegistry::register` et `get`. Le `Arc` évite
/// toute copie coûteuse du payload des anchors ; les lecteurs paient
/// uniquement le coût atomique d'incrément du refcount.
#[derive(Clone, Debug)]
pub struct ConeHandle {
    /// Clé stable du cône.
    pub key: ConeKey,
    /// Contenu canonical partagé.
    pub cone: Arc<CanonicalCone>,
}

impl ConeHandle {
    /// Construit un handle depuis la clé et le canonical.
    #[must_use]
    pub fn new(key: ConeKey, cone: Arc<CanonicalCone>) -> Self {
        Self { key, cone }
    }

    /// Accès au cône d'alignement nu (type M1).
    #[must_use]
    pub fn as_cone(&self) -> &AlignmentCone {
        &self.cone.cone
    }
}

// =====================================================================
// Helpers
// =====================================================================

fn check_len(name: &'static str, value: &str, max: usize) -> crate::errors::Result<()> {
    if value.len() > max {
        return Err(crate::errors::EfeError::InvalidConfig(format!(
            "{name} length {} exceeds max {max}",
            value.len()
        )));
    }
    Ok(())
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cone() -> AlignmentCone {
        AlignmentCone::new(vec![vec![1.0, 0.0], vec![0.0, 1.0]], 0.5).unwrap()
    }

    #[test]
    fn metadata_new_defaults() {
        let md = ConeMetadata::new("alice", "test cone").unwrap();
        assert_eq!(md.created_by, "alice");
        assert_eq!(md.description, "test cone");
        assert!(md.parent.is_none());
        assert!(md.evolution_op.is_none());
        assert!(md.labels.is_empty());
        assert!(md.created_at_nanos > 0);
    }

    #[test]
    fn metadata_rejects_long_created_by() {
        let long = "a".repeat(CREATED_BY_MAX_LEN + 1);
        assert!(ConeMetadata::new(long, "x").is_err());
    }

    #[test]
    fn metadata_rejects_long_description() {
        let long = "a".repeat(DESCRIPTION_MAX_LEN + 1);
        assert!(ConeMetadata::new("x", long).is_err());
    }

    #[test]
    fn with_label_adds_entry() {
        let md = ConeMetadata::new("bot", "auto")
            .unwrap()
            .with_label("env", "prod")
            .unwrap();
        assert_eq!(md.labels.get("env").map(String::as_str), Some("prod"));
    }

    #[test]
    fn with_label_rejects_too_many() {
        let mut md = ConeMetadata::new("bot", "auto").unwrap();
        for i in 0..LABELS_MAX_COUNT {
            md = md.with_label(format!("k{i}"), "v").unwrap();
        }
        assert!(md.with_label("k_extra", "v").is_err());
    }

    #[test]
    fn with_label_replaces_existing_key_even_at_max() {
        let mut md = ConeMetadata::new("bot", "auto").unwrap();
        for i in 0..LABELS_MAX_COUNT {
            md = md.with_label(format!("k{i}"), "v").unwrap();
        }
        // Replacing should succeed: no new key.
        let md2 = md.with_label("k0", "new").unwrap();
        assert_eq!(md2.labels.get("k0").map(String::as_str), Some("new"));
    }

    #[test]
    fn with_parent_sets_fields() {
        let parent = ConeKey::new("t", "k", 1).unwrap();
        let op = ConeEvolutionOp::AppendAnchor {
            anchor: vec![1.0, 0.0],
        };
        let md = ConeMetadata::new("alice", "evolution")
            .unwrap()
            .with_parent(parent.clone(), op.clone());
        assert_eq!(md.parent, Some(parent));
        assert_eq!(md.evolution_op, Some(op));
    }

    #[test]
    fn canonical_accessors() {
        let cone = make_cone();
        let canon = CanonicalCone::new(cone, "bot", "desc").unwrap();
        assert_eq!(canon.num_anchors(), 2);
        assert_eq!(canon.dim(), 2);
        assert!((canon.radius() - 0.5).abs() < f32::EPSILON);
        assert_eq!(canon.anchors().len(), 2);
    }

    #[test]
    fn canonical_with_metadata_preserves_meta() {
        let md = ConeMetadata::new("a", "b").unwrap();
        let canon = CanonicalCone::with_metadata(make_cone(), md.clone());
        assert_eq!(canon.metadata.created_by, md.created_by);
    }

    #[test]
    fn handle_as_cone() {
        let canon = CanonicalCone::new(make_cone(), "b", "d").unwrap();
        let handle = ConeHandle::new(ConeKey::new("t", "k", 1).unwrap(), Arc::new(canon));
        assert_eq!(handle.as_cone().anchors.len(), 2);
    }

    #[test]
    fn canonical_json_roundtrip() {
        let canon = CanonicalCone::new(make_cone(), "bot", "desc").unwrap();
        let s = serde_json::to_string(&canon).unwrap();
        let back: CanonicalCone = serde_json::from_str(&s).unwrap();
        assert_eq!(back.num_anchors(), canon.num_anchors());
        assert_eq!(back.metadata.created_by, canon.metadata.created_by);
    }

    #[test]
    fn label_key_length_enforced() {
        let md = ConeMetadata::new("b", "d").unwrap();
        let long_key = "k".repeat(LABEL_KEY_MAX_LEN + 1);
        assert!(md.with_label(long_key, "v").is_err());
    }

    #[test]
    fn label_value_length_enforced() {
        let md = ConeMetadata::new("b", "d").unwrap();
        let long_val = "v".repeat(LABEL_VALUE_MAX_LEN + 1);
        assert!(md.with_label("k", long_val).is_err());
    }
}
