// SPDX-License-Identifier: UNLICENSED
//! **Le registre de mapping** — une **revendication versionnée et hachée**, jamais une vérité.
//!
//! L'énoncé « le nœud humain `APPROVE` évidencie l'art. 14 » n'est **pas un fait** : c'est une
//! **interprétation juridique déclarée** de l'éditeur, et elle lui appartient tant qu'un régulateur ne
//! l'a pas validée. S6 la traite donc comme ce qu'elle est : une revendication **datée**, **versionnée**,
//! et **hachée** ([`MappingSet::set_hash`]) qui **voyage avec l'artefact** — pour que la dérivation soit
//! reproductible et la lecture, datable (le réglementaire évolue : actes délégués, normes harmonisées).
//!
//! Deux gardes d'honnêteté sont **imposées à la construction** ([`MappingSet::new`]) :
//! - **`I-S6-COUVERTURE-HONNÊTE`** : tout cadre présent **doit** déclarer au moins un **trou**
//!   ([`Coverage::OutOfDagScope`]). Un cadre sans trou serait un « vert malhonnête » (la classe de bug
//!   attrapée en S2) — il est **rejeté**.
//! - **forme** : un hors-périmètre n'a **pas** de sélecteur (rien à sélectionner) et **doit nommer son
//!   trou** ; une exigence couverte/partielle **doit** porter un sélecteur.

use crate::framework::{Coverage, Framework};
use crate::selector::Selector;
use provenance_dag::sha256;
use serde::{Deserialize, Serialize};

/// Une **revendication déclarée** : telle exigence d'un cadre est évidencée par tels nœuds, avec telle
/// honnêteté de couverture. PAS une vérité — une lecture, lisible et contestable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementMapping {
    pub framework: Framework,
    /// La citation (« art. 12 », « art. 5(1)(c) », « §10 LkSG »).
    pub citation: String,
    /// Le libellé de l'exigence, cité.
    pub requirement_text: String,
    pub coverage: Coverage,
    /// Le sélecteur sur le DAG. `None` **ssi** [`Coverage::OutOfDagScope`] (un trou n'a rien à sélectionner).
    pub selector: Option<Selector>,
    /// Ce que le DAG **fournit**, en clair (pour l'artefact lisible).
    pub furnishes: String,
    /// Le **trou explicite** : ce qui reste organisationnel / hors-DAG. **Jamais vide** pour un
    /// hors-périmètre ; pour un couvert/partiel, énonce la **limite honnête** (la nuance « pas conforme »).
    pub does_not_furnish: String,
}

impl RequirementMapping {
    /// Invariant de **forme** (imposé par [`MappingSet::new`]).
    fn check(&self) -> Result<(), MappingError> {
        let where_ = format!("{} {}", self.framework.label(), self.citation);
        match self.coverage {
            Coverage::OutOfDagScope => {
                if self.selector.is_some() {
                    return Err(MappingError::ScopeMismatch(format!("{where_} : un hors-périmètre ne sélectionne rien (selector doit être None)")));
                }
                if self.does_not_furnish.trim().is_empty() {
                    return Err(MappingError::SilentHole(format!("{where_} : un hors-périmètre DOIT nommer son trou (does_not_furnish vide)")));
                }
            }
            Coverage::Covered | Coverage::Partial => {
                if self.selector.is_none() {
                    return Err(MappingError::ScopeMismatch(format!("{where_} : une exigence couverte/partielle DOIT porter un sélecteur")));
                }
            }
        }
        Ok(())
    }

    /// Encodage **canonique** (longueur-préfixé, ordre figé) pour le hash du registre — déterministe et
    /// indépendant de l'ordre des clés JSON (cohérent avec la discipline binaire de S0).
    fn canonical(&self, out: &mut Vec<u8>) {
        put_str(out, self.framework.slug());
        put_str(out, &self.citation);
        put_str(out, &self.requirement_text);
        put_str(out, self.coverage.slug());
        match &self.selector {
            // Le sélecteur est sérialisé en JSON (déterministe sur nos enums fermés), puis préfixé en longueur.
            Some(s) => {
                out.push(1);
                put_str(out, &serde_json::to_string(s).expect("Selector sérialisable"));
            }
            None => out.push(0),
        }
        put_str(out, &self.furnishes);
        put_str(out, &self.does_not_furnish);
    }
}

/// Erreur de construction du registre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingError {
    /// Un cadre **sans aucun trou déclaré** — le « vert malhonnête » interdit (`I-S6-COUVERTURE-HONNÊTE`).
    NoDeclaredHoles(String),
    /// Forme invalide (sélecteur présent sur un hors-périmètre, ou absent sur une couverture).
    ScopeMismatch(String),
    /// Un **trou non nommé** (`does_not_furnish` vide sur un hors-périmètre).
    SilentHole(String),
    /// Registre vide.
    Empty,
}
impl std::fmt::Display for MappingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MappingError::NoDeclaredHoles(fw) => write!(f, "couverture malhonnête : le cadre « {fw} » ne déclare AUCUN trou (hors-périmètre) — un journal d'événements ne couvre jamais un cadre en entier"),
            MappingError::ScopeMismatch(m) => write!(f, "forme de mapping invalide : {m}"),
            MappingError::SilentHole(m) => write!(f, "trou silencieux : {m}"),
            MappingError::Empty => write!(f, "registre de mapping vide"),
        }
    }
}
impl std::error::Error for MappingError {}

/// Le **registre** — versionné, haché, et qui **voyage** avec l'artefact (cf. [`crate::bundle`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappingSet {
    /// La **lecture**, datée (ex. « 2026-06 »). On n'écrase **jamais** une version : une v2 = un nouveau set.
    pub version: String,
    /// Quand la revendication a été figée (ex. « 2026-06-29 »).
    pub published_at: String,
    pub mappings: Vec<RequirementMapping>,
}

impl MappingSet {
    /// Construit **et VALIDE** le registre. Rejette (anti-vert-malhonnête, `I-S6-COUVERTURE-HONNÊTE`) tout
    /// cadre sans trou déclaré, et toute maille mal formée. C'est l'imposition **par le type** de
    /// l'honnêteté de couverture : un registre malhonnête n'existe pas (il ne se construit pas).
    pub fn new(version: impl Into<String>, published_at: impl Into<String>, mappings: Vec<RequirementMapping>) -> Result<Self, MappingError> {
        if mappings.is_empty() {
            return Err(MappingError::Empty);
        }
        for m in &mappings {
            m.check()?;
        }
        // Tout cadre présent DOIT déclarer au moins un trou.
        for fw in distinct_frameworks(&mappings) {
            let has_hole = mappings.iter().any(|m| m.framework == fw && m.coverage == Coverage::OutOfDagScope);
            if !has_hole {
                return Err(MappingError::NoDeclaredHoles(fw.label().to_string()));
            }
        }
        Ok(MappingSet { version: version.into(), published_at: published_at.into(), mappings })
    }

    /// Le **hash du registre** (hex SHA-256 de l'encodage canonique). L'artefact l'enregistre → on sait
    /// **laquelle** lecture l'a produit. Déterministe ; deux registres au contenu distinct → hash distinct
    /// (`I-S6-MAPPING-VERSIONNÉ`). Hand-rolled via le `sha256` zéro-dépendance de S0 (aucune crypto neuve).
    pub fn set_hash(&self) -> String {
        let mut buf = Vec::new();
        put_str(&mut buf, &self.version);
        put_str(&mut buf, &self.published_at);
        buf.extend_from_slice(&(self.mappings.len() as u32).to_be_bytes());
        for m in &self.mappings {
            m.canonical(&mut buf);
        }
        to_hex(&sha256(&buf))
    }

    /// Les mailles d'un cadre donné (dans l'ordre déclaré).
    pub fn for_framework(&self, fw: Framework) -> Vec<&RequirementMapping> {
        self.mappings.iter().filter(|m| m.framework == fw).collect()
    }

    /// Les cadres présents (ordre de première apparition).
    pub fn frameworks(&self) -> Vec<Framework> {
        distinct_frameworks(&self.mappings)
    }
}

/// Les cadres distincts présents, dans l'ordre de première apparition (déterministe, sans `HashSet`).
fn distinct_frameworks(mappings: &[RequirementMapping]) -> Vec<Framework> {
    let mut v = Vec::new();
    for m in mappings {
        if !v.contains(&m.framework) {
            v.push(m.framework);
        }
    }
    v
}

// ── helpers d'encodage canonique (zéro dépendance, cohérents avec S0) ──

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}
fn to_hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::{NodeKind, Selector};

    fn covered(fw: Framework, cite: &str) -> RequirementMapping {
        RequirementMapping {
            framework: fw,
            citation: cite.into(),
            requirement_text: "exigence".into(),
            coverage: Coverage::Covered,
            selector: Some(Selector::Kind(NodeKind::AgentAct)),
            furnishes: "le DAG fournit X".into(),
            does_not_furnish: "limite honnête".into(),
        }
    }
    fn hole(fw: Framework, cite: &str) -> RequirementMapping {
        RequirementMapping {
            framework: fw,
            citation: cite.into(),
            requirement_text: "exigence organisationnelle".into(),
            coverage: Coverage::OutOfDagScope,
            selector: None,
            furnishes: String::new(),
            does_not_furnish: "artefact organisationnel hors DAG".into(),
        }
    }

    #[test]
    fn rejects_a_framework_with_no_declared_hole() {
        // I-S6-COUVERTURE-HONNÊTE : un cadre tout-couvert (sans trou) est REJETÉ.
        let err = MappingSet::new("v", "d", vec![covered(Framework::EuAiAct, "art. 12")]);
        assert!(matches!(err, Err(MappingError::NoDeclaredHoles(_))), "tout-vert sans trou refusé : {err:?}");
        // Avec un trou déclaré → accepté.
        let ok = MappingSet::new("v", "d", vec![covered(Framework::EuAiAct, "art. 12"), hole(Framework::EuAiAct, "art. 9")]);
        assert!(ok.is_ok());
    }

    #[test]
    fn rejects_malformed_shapes() {
        // hors-périmètre avec un sélecteur → ScopeMismatch.
        let mut bad = hole(Framework::Gdpr, "art. 6");
        bad.selector = Some(Selector::Kind(NodeKind::AgentAct));
        assert!(matches!(MappingSet::new("v", "d", vec![bad]), Err(MappingError::ScopeMismatch(_))));
        // couvert sans sélecteur → ScopeMismatch.
        let mut bad2 = covered(Framework::Gdpr, "art. 30");
        bad2.selector = None;
        assert!(matches!(MappingSet::new("v", "d", vec![bad2, hole(Framework::Gdpr, "art. 6")]), Err(MappingError::ScopeMismatch(_))));
        // trou non nommé → SilentHole.
        let mut bad3 = hole(Framework::Gdpr, "art. 35");
        bad3.does_not_furnish = "   ".into();
        assert!(matches!(MappingSet::new("v", "d", vec![bad3]), Err(MappingError::SilentHole(_))));
        // vide → Empty.
        assert!(matches!(MappingSet::new("v", "d", vec![]), Err(MappingError::Empty)));
    }

    #[test]
    fn each_present_framework_must_have_a_hole() {
        // Deux cadres ; l'un sans trou → rejet nommant CE cadre.
        let set = MappingSet::new(
            "v",
            "d",
            vec![
                covered(Framework::EuAiAct, "art. 12"),
                hole(Framework::EuAiAct, "art. 9"),
                covered(Framework::Gdpr, "art. 30"), // pas de trou GDPR
            ],
        );
        match set {
            Err(MappingError::NoDeclaredHoles(fw)) => assert_eq!(fw, "GDPR", "c'est le cadre GDPR qui manque de trou"),
            other => panic!("attendu NoDeclaredHoles(GDPR), obtenu {other:?}"),
        }
    }

    #[test]
    fn set_hash_is_deterministic_and_version_sensitive() {
        // I-S6-MAPPING-VERSIONNÉ : même contenu → même hash ; contenu/version distinct → hash distinct.
        let a = MappingSet::new("2026-06", "2026-06-29", vec![covered(Framework::EuAiAct, "art. 12"), hole(Framework::EuAiAct, "art. 9")]).unwrap();
        let a2 = MappingSet::new("2026-06", "2026-06-29", vec![covered(Framework::EuAiAct, "art. 12"), hole(Framework::EuAiAct, "art. 9")]).unwrap();
        assert_eq!(a.set_hash(), a2.set_hash(), "même registre → même hash");
        assert_eq!(a.set_hash().len(), 64, "hex de 32 octets");
        // Une version différente change le hash.
        let b = MappingSet::new("2026-07", "2026-07-01", vec![covered(Framework::EuAiAct, "art. 12"), hole(Framework::EuAiAct, "art. 9")]).unwrap();
        assert_ne!(a.set_hash(), b.set_hash(), "version distincte → hash distinct");
        // Un libellé d'exigence modifié change le hash.
        let mut diff = covered(Framework::EuAiAct, "art. 12");
        diff.requirement_text = "AUTRE libellé".into();
        let c = MappingSet::new("2026-06", "2026-06-29", vec![diff, hole(Framework::EuAiAct, "art. 9")]).unwrap();
        assert_ne!(a.set_hash(), c.set_hash(), "contenu distinct → hash distinct");
    }

    #[test]
    fn round_trips_through_json_and_for_framework() {
        let set = MappingSet::new("2026-06", "2026-06-29", vec![covered(Framework::EuAiAct, "art. 12"), hole(Framework::EuAiAct, "art. 9"), covered(Framework::Gdpr, "art. 30"), hole(Framework::Gdpr, "art. 6")]).unwrap();
        // Le registre voyage en JSON → re-désérialise à l'identique (même hash).
        let json = serde_json::to_string(&set).unwrap();
        let back: MappingSet = serde_json::from_str(&json).unwrap();
        assert_eq!(set, back);
        assert_eq!(set.set_hash(), back.set_hash(), "le registre rechargé produit le MÊME hash (re-dérivable)");
        // Filtrage par cadre.
        assert_eq!(set.for_framework(Framework::EuAiAct).len(), 2);
        assert_eq!(set.frameworks(), vec![Framework::EuAiAct, Framework::Gdpr]);
    }
}
