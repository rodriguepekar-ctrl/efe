// SPDX-License-Identifier: UNLICENSED
//! **Les cibles d'outillage** — le cadre réglementaire visé, et la **classe de couverture** honnête.
//!
//! Deux énumérations, petites mais décisives. [`Framework`] nomme le cadre (EU AI Act, GDPR, devoir de
//! vigilance) **comme une cible d'outillage, pas un avis juridique** : l'applicabilité dépend de la
//! classification du système et du conseil du client. [`Coverage`] porte la distinction la plus
//! importante de S6 au niveau de chaque exigence : **COUVERT** (le DAG fournit l'évidence), **PARTIEL**
//! (il évidencie le fait, pas l'adéquation du dispositif), ou **HORS-PÉRIMÈTRE-DAG** (un artefact
//! organisationnel que le DAG ne produit pas). La carte montre **toujours ses trois couleurs** — taire
//! la rouge serait un « vert malhonnête » (cf. l'architecture S6 §5, l'invariant `I-S6-COUVERTURE-HONNÊTE`).

use serde::{Deserialize, Serialize};

/// Le cadre réglementaire ciblé. **Cible d'OUTILLAGE** — le produit fournit l'évidence que le cadre
/// réclame ; il ne **prononce pas** la conformité (cf. [`crate::artifact::HonestyEnvelope`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framework {
    /// EU AI Act — Règlement (UE) 2024/1689 (régime haut risque).
    EuAiAct,
    /// GDPR — Règlement (UE) 2016/679.
    Gdpr,
    /// CSDDD — Directive (UE) 2024/1760 (devoir de vigilance « durabilité »).
    Csddd,
    /// LkSG — Lieferkettensorgfaltspflichtengesetz (devoir de vigilance, Allemagne).
    Lksg,
}

impl Framework {
    /// L'étiquette humaine (pour le récit lisible).
    pub fn label(self) -> &'static str {
        match self {
            Framework::EuAiAct => "EU AI Act",
            Framework::Gdpr => "GDPR",
            Framework::Csddd => "CSDDD",
            Framework::Lksg => "LkSG",
        }
    }

    /// Le slug stable (pour les noms de fichiers du bundle, ex. `reg/eu_ai_act.md`). Stable et
    /// **délibérément distinct** de `Debug` : un nom de fichier ne doit pas suivre un renommage de variante.
    pub fn slug(self) -> &'static str {
        match self {
            Framework::EuAiAct => "eu_ai_act",
            Framework::Gdpr => "gdpr",
            Framework::Csddd => "csddd",
            Framework::Lksg => "lksg",
        }
    }
}

/// La **classe de couverture** d'une exigence — la distinction d'honnêteté de S6, au niveau de la maille.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    /// Le DAG **fournit** l'évidence que l'exigence réclame — parfois **au-delà** de la lettre
    /// (ex. l'art. 12 : le DAG EST le registre d'événements, et il est tamper-evident).
    Covered,
    /// Le DAG évidencie le **fait**, pas l'**adéquation du dispositif** (ex. l'art. 14 : prouve que
    /// l'oversight a eu lieu — pas qu'il est « effectif » au sens de l'article).
    Partial,
    /// **Hors périmètre du DAG** : un artefact **organisationnel** ou de cycle de vie que le DAG ne
    /// produit pas (système de gestion des risques, documentation technique, base légale…). Déclaré,
    /// **jamais tu** — c'est ce qui distingue un moteur d'évidence honnête d'un faux plein.
    OutOfDagScope,
}

impl Coverage {
    /// L'étiquette humaine (pour le récit).
    pub fn label(self) -> &'static str {
        match self {
            Coverage::Covered => "COUVERT",
            Coverage::Partial => "PARTIEL",
            Coverage::OutOfDagScope => "HORS-PÉRIMÈTRE-DAG",
        }
    }

    /// Le slug stable (pour l'encodage canonique du hash de mapping).
    pub fn slug(self) -> &'static str {
        match self {
            Coverage::Covered => "covered",
            Coverage::Partial => "partial",
            Coverage::OutOfDagScope => "out_of_dag_scope",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framework_slug_is_stable_and_filename_safe() {
        for fw in [Framework::EuAiAct, Framework::Gdpr, Framework::Csddd, Framework::Lksg] {
            let s = fw.slug();
            assert!(!s.is_empty());
            assert!(s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'), "slug sûr pour un nom de fichier : {s}");
            assert!(!fw.label().is_empty());
        }
        // Slugs distincts (pas de collision de fichier).
        let slugs: Vec<&str> = [Framework::EuAiAct, Framework::Gdpr, Framework::Csddd, Framework::Lksg].iter().map(|f| f.slug()).collect();
        let mut uniq = slugs.clone();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), slugs.len(), "slugs uniques");
    }

    #[test]
    fn coverage_serialises_snake_case_and_has_three_classes() {
        assert_eq!(serde_json::to_string(&Coverage::OutOfDagScope).unwrap(), "\"out_of_dag_scope\"");
        assert_eq!(Coverage::Covered.slug(), "covered");
        // Les trois classes ont des étiquettes distinctes (la carte montre ses trois couleurs).
        let labels = [Coverage::Covered.label(), Coverage::Partial.label(), Coverage::OutOfDagScope.label()];
        assert_eq!(labels.iter().collect::<std::collections::HashSet<_>>().len(), 3);
    }
}
