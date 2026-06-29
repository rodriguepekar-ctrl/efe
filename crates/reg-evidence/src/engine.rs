// SPDX-License-Identifier: UNLICENSED
//! **La dérivation** — `derive(records, clé, mapping, cadre, portée) → artefact`. Pure et déterministe.
//!
//! Le cœur de S6, et la raison de son caractère **trust-light** : l'artefact est une **fonction pure** de
//! deux entrées — les records scellés (le fait re-prouvé) et le [`MappingSet`] versionné (la revendication
//! déclarée). Un tiers détenant `(records, clé, mapping)` **ré-exécute** `derive` et obtient le **même**
//! artefact (octet-à-octet après rendu) → **aucune surface de confiance neuve** (`I-S6-VUE-REDÉRIVABLE`).
//!
//! Les gardes, dans l'ordre :
//! 1. **RE-PROUVER** ([`reprove_dag`]) et refuser le non-prouvé (`I-S6-SUR-PROUVÉ`) — jamais d'évidence
//!    réglementaire sur un DAG dont l'intégrité n'est pas vérifiée. S6 **re-prouve lui-même** (il ne fait
//!    pas confiance à un drapeau amont) ; c'est aussi la `clé` que le tiers détient pour re-dériver.
//! 2. **Cadre présent** dans le registre, sinon [`RegError::FrameworkAbsent`].
//! 3. **Filtrer la portée** (déterministe, ordre de chaîne) ; refuser une portée vide.
//! 4. **Évaluer** chaque sélecteur → `node_id` évidents ; **classer** la couverture.
//! 5. **Garde anti-citation-fantôme** (`I-S6-TRAÇABLE`) : tout id cité doit être présent — sinon échec dur.

use crate::artifact::{CaseScope, CoverageSummary, HonestyEnvelope, RegulatoryArtifact, RequirementFinding};
use crate::framework::{Coverage, Framework};
use crate::mapping::MappingSet;
use crate::selector::select_ids;
use provenance_dag::{reprove_dag, Node, Record, TenantKey};
use std::collections::HashSet;

/// Erreur de dérivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegError {
    /// Les records **ne re-prouvent pas** (intégrité cassée) — jamais d'évidence sur du non-prouvé.
    NotProven { broken_at: Option<usize>, reason: String },
    /// Le **cadre** demandé n'est pas dans le registre de mapping.
    FrameworkAbsent(String),
    /// La **portée** (cas) ne contient aucun record — rien à attester.
    EmptyScope(String),
    /// Garde défensive : une évidence citée n'est **pas** dans les records de la portée. Ne devrait jamais
    /// arriver (le sélecteur ne rend que des ids existants) — mais on échoue **dur** plutôt que d'émettre
    /// une citation fantôme.
    PhantomCitation(String),
}
impl std::fmt::Display for RegError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegError::NotProven { broken_at, reason } => write!(f, "DAG non re-prouvé (record {broken_at:?}) — on ne dérive JAMAIS d'évidence sur du non-prouvé : {reason}"),
            RegError::FrameworkAbsent(fw) => write!(f, "cadre « {fw} » absent du registre de mapping"),
            RegError::EmptyScope(s) => write!(f, "portée vide ({s}) — aucun record à attester"),
            RegError::PhantomCitation(id) => write!(f, "citation fantôme : le nœud « {id} » n'est pas dans la portée re-prouvée"),
        }
    }
}
impl std::error::Error for RegError {}

/// **LA DÉRIVATION.** Cf. la doc du module. Pure (aucune horloge, aucun état caché, aucun ordre non
/// déterministe) → re-jouable à l'identique par un tiers depuis le bundle.
pub fn derive(
    records: &[Record],
    key: &TenantKey,
    mapping: &MappingSet,
    framework: Framework,
    scope: &CaseScope,
) -> Result<RegulatoryArtifact, RegError> {
    // 1. RE-PROUVER — jamais d'évidence sur du non-prouvé (I-S6-SUR-PROUVÉ).
    let verdict = reprove_dag(records, key);
    if !verdict.ok {
        return Err(RegError::NotProven {
            broken_at: verdict.broken_at,
            reason: verdict.reason.unwrap_or_else(|| "intégrité non vérifiée".into()),
        });
    }
    let manifest_root = verdict.manifest_root.expect("ok ⇒ manifest-root présent");

    // 2. Le cadre doit exister dans le registre.
    let reqs = mapping.for_framework(framework);
    if reqs.is_empty() {
        return Err(RegError::FrameworkAbsent(framework.label().to_string()));
    }

    // 3. Filtrer la PORTÉE (déterministe, ordre de chaîne).
    let scoped: Vec<&Record> = scope_records(records, scope);
    if scoped.is_empty() {
        return Err(RegError::EmptyScope(scope.label()));
    }
    let present: HashSet<&str> = scoped.iter().map(|r| r.node.id()).collect();

    // 4 & 5. Pour chaque exigence : sélectionner les node_id évidents ; classer ; garde anti-fantôme.
    let mut findings = Vec::with_capacity(reqs.len());
    let mut covered = 0usize;
    let mut partial = 0usize;
    let mut out_of_dag_scope = 0usize;
    for m in &reqs {
        let evidence = match &m.selector {
            Some(sel) => select_ids(&scoped, sel),
            None => Vec::new(), // un hors-périmètre n'a rien à sélectionner (forme validée par MappingSet::new)
        };
        // Garde anti-citation-fantôme (défensive, I-S6-TRAÇABLE).
        for id in &evidence {
            if !present.contains(id.as_str()) {
                return Err(RegError::PhantomCitation(id.clone()));
            }
        }
        match m.coverage {
            Coverage::Covered => covered += 1,
            Coverage::Partial => partial += 1,
            Coverage::OutOfDagScope => out_of_dag_scope += 1,
        }
        findings.push(RequirementFinding {
            citation: m.citation.clone(),
            requirement_text: m.requirement_text.clone(),
            coverage: m.coverage,
            evidence,
            furnishes: m.furnishes.clone(),
            does_not_furnish: m.does_not_furnish.clone(),
        });
    }

    Ok(RegulatoryArtifact {
        framework,
        case_scope: scope.clone(),
        mapping_version: mapping.version.clone(),
        mapping_set_hash: mapping.set_hash(),
        manifest_root,
        n_records_in_scope: scoped.len(),
        findings,
        coverage_summary: CoverageSummary { covered, partial, out_of_dag_scope },
        honesty: HonestyEnvelope::default(),
    })
}

/// Filtre les records de la portée. `FullTenant` = tous. `Case(c)` = les nœuds du cas (id préfixé `c::`),
/// **plus** une **expansion à un niveau** : les évidences citées par les actes du cas (`evidence_refs`),
/// les arêtes incidentes (`from`/`to` ∈ cas), l'oversight visant le cas (`target` ∈ cas), et les évidences
/// annotées `consumed_by` ∈ cas. Tout dans l'**ordre de chaîne** (déterministe).
///
/// (L'**isolation cryptographique** d'un cas sans révéler les autres — remettre UN dossier fournisseur à
/// un régulateur sans montrer le reste — est la **forêt de Merkle par-cas**, raffinement nommé hérité de
/// S4 §10 ; non livré ici. Aujourd'hui, l'artefact par-cas se re-dérive et se re-prouve contre le
/// `tenant-root` complet.)
fn scope_records<'a>(records: &'a [Record], scope: &CaseScope) -> Vec<&'a Record> {
    match scope {
        CaseScope::FullTenant => records.iter().collect(),
        CaseScope::Case(case) => {
            let prefix = format!("{case}::");
            // Seed : les ids portant le préfixe du cas.
            let seed: HashSet<&str> = records.iter().map(|r| r.node.id()).filter(|id| id.starts_with(&prefix)).collect();
            // Expansion à UN niveau (nœuds incidents).
            let mut also: HashSet<&str> = HashSet::new();
            for r in records {
                match &r.node {
                    Node::AgentAct { node_id, evidence_refs, .. } if seed.contains(node_id.as_str()) => {
                        for e in evidence_refs {
                            also.insert(e.as_str());
                        }
                    }
                    Node::Edge { edge_id, from_node, to_node, .. } if seed.contains(from_node.as_str()) || seed.contains(to_node.as_str()) => {
                        also.insert(edge_id.as_str());
                    }
                    Node::Human { node_id, target_node, .. } if seed.contains(target_node.as_str()) => {
                        also.insert(node_id.as_str());
                    }
                    Node::Evidence { evidence_id, consumed_by_node, .. } if seed.contains(consumed_by_node.as_str()) => {
                        also.insert(evidence_id.as_str());
                    }
                    _ => {}
                }
            }
            records
                .iter()
                .filter(|r| {
                    let id = r.node.id();
                    seed.contains(id) || also.contains(id)
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::Coverage;
    use crate::mapping::{MappingSet, RequirementMapping};
    use crate::selector::{NodeKind, Selector};
    use provenance_dag::{HumanAction, SealedDag};

    fn key() -> TenantKey {
        TenantKey::from_bytes([11u8; 32])
    }

    /// Un cas réaliste « conformité fournisseur » + un SECOND cas (pour tester la portée par-cas).
    fn build() -> SealedDag {
        let k = key();
        let mut d = SealedDag::new();
        // cas SUP-2024
        d.append(&k, Node::AgentAct { node_id: "SUP-2024::extract".into(), agent_role: "supplier_extract".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "{supplier}".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        d.append(&k, Node::Evidence { evidence_id: "SUP-2024::ev".into(), tool: "policy_db".into(), query_hash: "q".into(), retrieved_ref: "policy://4.2".into(), content_hash: "cc".into(), consumed_by_node: "SUP-2024::sanction".into(), timestamp: 2 }).unwrap();
        d.append(&k, Node::AgentAct { node_id: "SUP-2024::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec!["SUP-2024::ev".into()], timestamp: 3 }).unwrap();
        d.append(&k, Node::Edge { edge_id: "SUP-2024::e1".into(), from_node: "SUP-2024::extract".into(), to_node: "SUP-2024::sanction".into(), artifact_hash: "a".into(), artifact_type: "supplier_record".into(), timestamp: 4 }).unwrap();
        d.append(&k, Node::AgentAct { node_id: "SUP-2024::aggregate".into(), agent_role: "verdict_aggregate".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "COMPLIANT".into(), evidence_refs: vec![], timestamp: 5 }).unwrap();
        d.append(&k, Node::Human { node_id: "SUP-2024::rev".into(), human_id: "officer:1".into(), action: HumanAction::Approve, target_node: "SUP-2024::aggregate".into(), justification: "revue OK".into(), timestamp: 6 }).unwrap();
        // cas SUP-OTHER (ne doit PAS apparaître dans la portée SUP-2024)
        d.append(&k, Node::AgentAct { node_id: "SUP-OTHER::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 7 }).unwrap();
        d
    }

    /// Un registre minuscule mais HONNÊTE (un trou déclaré) pour l'EU AI Act.
    fn mapping() -> MappingSet {
        MappingSet::new(
            "2026-06",
            "2026-06-29",
            vec![
                RequirementMapping {
                    framework: Framework::EuAiAct,
                    citation: "art. 12".into(),
                    requirement_text: "tenue de registres".into(),
                    coverage: Coverage::Covered,
                    selector: Some(Selector::Any(vec![Selector::Kind(NodeKind::AgentAct), Selector::Kind(NodeKind::Edge), Selector::Kind(NodeKind::Evidence), Selector::Kind(NodeKind::Human)])),
                    furnishes: "le DAG scellé EST le registre".into(),
                    does_not_furnish: "la rétention reste au déployeur".into(),
                },
                RequirementMapping {
                    framework: Framework::EuAiAct,
                    citation: "art. 14".into(),
                    requirement_text: "contrôle humain".into(),
                    coverage: Coverage::Partial,
                    selector: Some(Selector::All(vec![Selector::Kind(NodeKind::Human), Selector::HumanActionIn(vec![HumanAction::Approve, HumanAction::Override, HumanAction::Escalate])])),
                    furnishes: "les nœuds humains prouvent que l'oversight a eu lieu".into(),
                    does_not_furnish: "pas que le dispositif est « effectif »".into(),
                },
                RequirementMapping {
                    framework: Framework::EuAiAct,
                    citation: "art. 9".into(),
                    requirement_text: "système de gestion des risques".into(),
                    coverage: Coverage::OutOfDagScope,
                    selector: None,
                    furnishes: String::new(),
                    does_not_furnish: "artefact organisationnel hors DAG".into(),
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn derive_is_deterministic_and_traceable() {
        // I-S6-VUE-REDÉRIVABLE + I-S6-TRAÇABLE.
        let d = build();
        let m = mapping();
        let a = derive(d.records(), &key(), &m, Framework::EuAiAct, &CaseScope::Case("SUP-2024".into())).unwrap();
        let b = derive(d.records(), &key(), &m, Framework::EuAiAct, &CaseScope::Case("SUP-2024".into())).unwrap();
        assert_eq!(a, b, "deux dérivations → artefact identique (pur, déterministe)");
        // L'artefact enregistre la version + le hash du mapping + le root re-dérivé.
        assert_eq!(a.mapping_version, "2026-06");
        assert_eq!(a.mapping_set_hash, m.set_hash());
        assert_eq!(a.manifest_root, d.manifest_root());
        // Chaque id cité est présent dans les records (traçable).
        let present: HashSet<&str> = d.records().iter().map(|r| r.node.id()).collect();
        for f in &a.findings {
            for id in &f.evidence {
                assert!(present.contains(id.as_str()), "id cité présent : {id}");
            }
        }
    }

    #[test]
    fn scope_isolates_the_case_with_one_level_expansion() {
        let d = build();
        let m = mapping();
        let a = derive(d.records(), &key(), &m, Framework::EuAiAct, &CaseScope::Case("SUP-2024".into())).unwrap();
        // L'art. 12 sélectionne TOUT le log DANS la portée — donc les 6 records de SUP-2024, et JAMAIS
        // le record de SUP-OTHER.
        let art12 = a.findings.iter().find(|f| f.citation == "art. 12").unwrap();
        assert_eq!(art12.evidence.len(), 6, "les 6 records du cas (extract, ev, sanction, e1, aggregate, rev)");
        assert!(art12.evidence.iter().all(|id| id.starts_with("SUP-2024::")), "aucune fuite d'un autre cas");
        assert!(!art12.evidence.iter().any(|id| id.starts_with("SUP-OTHER::")), "SUP-OTHER exclu");
        assert_eq!(a.n_records_in_scope, 6);
        // L'art. 14 ne sélectionne QUE l'oversight (le nœud humain).
        let art14 = a.findings.iter().find(|f| f.citation == "art. 14").unwrap();
        assert_eq!(art14.evidence, vec!["SUP-2024::rev"]);
        // Le trou (art. 9) n'a aucune évidence, et la couverture montre ses trois classes.
        let art9 = a.findings.iter().find(|f| f.citation == "art. 9").unwrap();
        assert!(art9.evidence.is_empty(), "un hors-périmètre n'a pas d'évidence");
        assert_eq!(a.coverage_summary.covered, 1);
        assert_eq!(a.coverage_summary.partial, 1);
        assert_eq!(a.coverage_summary.out_of_dag_scope, 1, "le trou est compté et montré");
    }

    #[test]
    fn full_tenant_scope_sees_all_cases() {
        let d = build();
        let m = mapping();
        let a = derive(d.records(), &key(), &m, Framework::EuAiAct, &CaseScope::FullTenant).unwrap();
        assert_eq!(a.n_records_in_scope, 7, "les 7 records du tenant");
        let art12 = a.findings.iter().find(|f| f.citation == "art. 12").unwrap();
        assert!(art12.evidence.iter().any(|id| id.starts_with("SUP-OTHER::")), "le tenant complet voit SUP-OTHER");
    }

    #[test]
    fn refuses_tampered_records() {
        // I-S6-SUR-PROUVÉ : un DAG altéré → NotProven, jamais d'évidence.
        let d = build();
        let m = mapping();
        let tampered = d.to_jsonl().replacen("PASS", "FAIL", 1);
        let recs = SealedDag::records_from_jsonl(&tampered).unwrap();
        let err = derive(&recs, &key(), &m, Framework::EuAiAct, &CaseScope::FullTenant);
        assert!(matches!(err, Err(RegError::NotProven { .. })), "altération → NotProven : {err:?}");
    }

    #[test]
    fn refuses_wrong_key() {
        // Une mauvaise clé tenant → NotProven dès le 1er record (la dérivation re-prouve elle-même).
        let d = build();
        let m = mapping();
        let other = TenantKey::from_bytes([99u8; 32]);
        assert!(matches!(derive(d.records(), &other, &m, Framework::EuAiAct, &CaseScope::FullTenant), Err(RegError::NotProven { .. })));
    }

    #[test]
    fn empty_scope_and_absent_framework_are_errors() {
        let d = build();
        let m = mapping();
        // Cas inconnu → portée vide.
        assert!(matches!(derive(d.records(), &key(), &m, Framework::EuAiAct, &CaseScope::Case("NOPE".into())), Err(RegError::EmptyScope(_))));
        // Cadre absent du registre (le registre ne contient que l'EU AI Act).
        assert!(matches!(derive(d.records(), &key(), &m, Framework::Gdpr, &CaseScope::FullTenant), Err(RegError::FrameworkAbsent(_))));
    }
}
