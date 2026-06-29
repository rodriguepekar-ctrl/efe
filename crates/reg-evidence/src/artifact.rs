// SPDX-License-Identifier: UNLICENSED
//! **L'artefact réglementaire** — une **VUE**, jamais un verdict. « outillé ≠ conforme » imposé par le TYPE.
//!
//! Le choix décisif, sœur de l'`IntegrityEnvelope` de S2 (qui n'a aucun champ « correct ») : il n'existe
//! **aucun champ** où écrire « conforme ». L'artefact porte `coverage` (couvert / partiel / hors-périmètre)
//! et `evidence: [node_id]` (les nœuds qui **évidencient** l'exigence), plus un [`HonestyEnvelope`] qui
//! **énonce** outillé ≠ conforme. Un développeur qui voudrait, par pression commerciale, faire dire à
//! l'artefact « système conforme » n'a **structurellement nulle part** où l'écrire. La conformité est une
//! conclusion **juridique** (classification + conseil + obligations organisationnelles) — pas la nôtre.

use crate::framework::{Coverage, Framework};
use serde::Serialize;

/// La **portée** d'un artefact : tout le tenant, ou un **cas (dossier)** isolé (ex. un fournisseur).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "case_id", rename_all = "snake_case")]
pub enum CaseScope {
    /// Tout le DAG du tenant.
    FullTenant,
    /// Un seul cas — par convention d'id `{cas}::{étape}` (+ ses évidences/arêtes/oversight incidents).
    Case(String),
}
impl CaseScope {
    /// L'étiquette humaine (pour le récit).
    pub fn label(&self) -> String {
        match self {
            CaseScope::FullTenant => "tout le tenant".to_string(),
            CaseScope::Case(c) => format!("cas « {c} »"),
        }
    }
}

/// Une **constatation par exigence** — l'évidence **mappée**, jamais un verdict. Chaque `node_id` cité
/// est présent dans les records re-prouvés (traçable, `I-S6-TRAÇABLE`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RequirementFinding {
    pub citation: String,
    pub requirement_text: String,
    pub coverage: Coverage,
    /// Les `node_id` qui **évidencient** l'exigence (vide pour un hors-périmètre déclaré, ou si aucun
    /// nœud de la portée ne satisfait le sélecteur). **Des ids seulement** — jamais d'output décisionnel,
    /// pour ne pas blanchir une décision scellée-mais-fausse en « évidence de conformité ».
    pub evidence: Vec<String>,
    pub furnishes: String,
    pub does_not_furnish: String,
}

/// Le **résumé de couverture** — montré côte à côte (la carte montre ses **trois** couleurs, §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CoverageSummary {
    pub covered: usize,
    pub partial: usize,
    pub out_of_dag_scope: usize,
}

/// La **sémantique** d'un artefact — une énumération à **une seule valeur**. Pas une chaîne libre qu'on
/// remplirait un jour de « conforme » : le type **interdit** le glissement vers le verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EvidenceSemantics {
    /// Le seul niveau : évidence **fournie et mappée** à l'exigence — pas un prononcé de conformité.
    #[serde(rename = "EVIDENCE_MAPPING_ONLY")]
    EvidenceMappingOnly,
}

/// La phrase humaine de la sémantique — **constante** (pas du texte libre). Énonce les **deux** limites :
/// outillé ≠ conforme (la conclusion juridique) ET intègre ≠ correct (la justesse des décisions citées).
pub const SEMANTICS_NOTE: &str = "fournit et mappe l'évidence que l'exigence réclame — ni un prononcé de conformité (qui dépend de la classification du système et du conseil du client), ni la justesse des décisions citées (intègre ≠ correct)";

/// L'**enveloppe d'honnêteté** portée par **chaque** artefact. À DESSEIN : aucun champ « conforme ».
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HonestyEnvelope {
    pub semantics: EvidenceSemantics,
    pub semantics_note: &'static str,
}
impl Default for HonestyEnvelope {
    fn default() -> Self {
        HonestyEnvelope { semantics: EvidenceSemantics::EvidenceMappingOnly, semantics_note: SEMANTICS_NOTE }
    }
}

/// L'**artefact réglementaire** — la VUE re-dérivable. **Aucun champ « conforme » nulle part** : ce n'est
/// pas un verdict, c'est de l'évidence mappée, traçable, et re-calculable par un tiers (trust-light).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegulatoryArtifact {
    pub framework: Framework,
    pub case_scope: CaseScope,
    /// La **version** de la lecture (le mapping) qui a produit cet artefact.
    pub mapping_version: String,
    /// Le **hash** du registre de mapping (on sait LAQUELLE lecture l'a produit ; elle voyage, cf. bundle).
    pub mapping_set_hash: String,
    /// Le **manifest-root** re-dérivé pendant la dérivation (lie l'artefact à un état précis du DAG).
    pub manifest_root: String,
    pub n_records_in_scope: usize,
    pub findings: Vec<RequirementFinding>,
    pub coverage_summary: CoverageSummary,
    pub honesty: HonestyEnvelope,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact() -> RegulatoryArtifact {
        RegulatoryArtifact {
            framework: Framework::EuAiAct,
            case_scope: CaseScope::Case("SUP-2024".into()),
            mapping_version: "2026-06".into(),
            mapping_set_hash: "ab".repeat(32),
            manifest_root: "cd".repeat(32),
            n_records_in_scope: 4,
            findings: vec![RequirementFinding {
                citation: "art. 12".into(),
                requirement_text: "tenue de registres".into(),
                coverage: Coverage::Covered,
                evidence: vec!["SUP-2024::sanction".into()],
                furnishes: "le DAG scellé EST le registre".into(),
                does_not_furnish: "la rétention reste au déployeur".into(),
            }],
            coverage_summary: CoverageSummary { covered: 1, partial: 0, out_of_dag_scope: 0 },
            honesty: HonestyEnvelope::default(),
        }
    }

    #[test]
    fn artifact_carries_semantics_and_has_no_compliant_field() {
        // I-S6-PAS-DE-VERDICT : la sortie sérialisée porte la sémantique d'évidence et n'emploie JAMAIS
        // le token « compliant » (aucun champ de conformité n'existe ; les ids cités n'en contiennent pas).
        let json = serde_json::to_string(&artifact()).unwrap().to_lowercase();
        assert!(json.contains("evidence_mapping_only"), "la sémantique d'évidence est portée");
        assert!(!json.contains("compliant"), "aucun verdict de conformité dans l'artefact : {json}");
        // Et il n'y a aucune clé nommée « conforme »/« conformity »/« compliance ».
        for forbidden in ["\"compliant\"", "\"conforme\"", "\"compliance\"", "\"conformity\""] {
            assert!(!json.contains(forbidden), "aucun champ de verdict « {forbidden} »");
        }
    }

    #[test]
    fn case_scope_serialises_adjacently() {
        assert_eq!(serde_json::to_string(&CaseScope::FullTenant).unwrap(), "{\"kind\":\"full_tenant\"}");
        let j = serde_json::to_string(&CaseScope::Case("SUP-2024".into())).unwrap();
        assert!(j.contains("\"kind\":\"case\"") && j.contains("\"case_id\":\"SUP-2024\""), "portée par-cas sérialisée : {j}");
        assert_eq!(CaseScope::Case("X".into()).label(), "cas « X »");
    }

    #[test]
    fn honesty_envelope_default_is_evidence_only() {
        let h = HonestyEnvelope::default();
        assert_eq!(h.semantics, EvidenceSemantics::EvidenceMappingOnly);
        // La note énonce les deux limites (outillé≠conforme ET intègre≠correct).
        assert!(h.semantics_note.contains("conformité"));
        assert!(h.semantics_note.contains("intègre ≠ correct"));
    }
}
