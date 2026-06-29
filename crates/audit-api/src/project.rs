// SPDX-License-Identifier: UNLICENSED
//! **La projection « décision lisible »** — une fonction **PURE** des records scellés.
//!
//! [`project`] transforme les nœuds scellés d'un cas en un [`DecisionView`] qu'un agent de conformité
//! lit sans effort. Le principe cardinal de S2 (cf. l'architecture §2) : la projection **n'invente rien**
//! — chaque champ projeté nomme le `node_id` scellé dont il dérive (`backed_by_node`), et la fonction est
//! **déterministe** (mêmes records → même vue, bit-à-bit, recomputable par quiconque détient les records).
//! Le seul champ *dérivé* est le [`DecisionStatus`], et sa règle est **pure, publiée et ré-vérifiable**
//! (§3.2) — il décrit l'**état du record**, jamais la **justesse** de la décision.

use provenance_dag::{HumanAction, Node, Record};
use serde::Serialize;

/// L'état du record d'une décision — **dérivé** par une règle pure (cf. [`derive_status`]). Décrit ce
/// que le record CONTIENT (un verdict a été émis, un humain est intervenu), **jamais** la justesse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DecisionStatus {
    /// Un verdict d'agrégation **a conclu** sans token d'échec, aucun contrôle n'est en échec, et aucun
    /// humain n'a outrepassé. (Exige la PRÉSENCE d'un nœud de verdict — sinon [`DecisionStatus::Pending`].)
    Compliant,
    /// Signal d'échec : le **verdict** porte un token d'échec, **ou** un contrôle est en échec, **ou** le
    /// cas porte des verdicts **conflictuels**.
    Fail,
    /// Un humain est intervenu par OVERRIDE ou ESCALATE (oversight scellé).
    Exception,
    /// **Aucun verdict d'agrégation** n'a encore conclu le cas (en cours, ou seulement des arêtes/évidences)
    /// — on ne montre JAMAIS « conforme » vert sans conclusion scellée.
    Pending,
}

/// La configuration de projection — les **conventions** du workflow, explicites et documentées (pas de
/// magie). Le rôle d'agrégation porte le verdict ; les tokens d'échec marquent un contrôle FAIL.
#[derive(Debug, Clone)]
pub struct ProjectionConfig {
    /// Les rôles d'agent dont l'output EST le verdict de la décision (ex. « verdict_aggregate »).
    pub verdict_roles: Vec<String>,
    /// Les tokens (insensibles à la casse) d'un output qui signifie un échec de contrôle.
    pub fail_tokens: Vec<String>,
}
impl Default for ProjectionConfig {
    fn default() -> Self {
        ProjectionConfig {
            verdict_roles: vec!["verdict_aggregate".into()],
            fail_tokens: vec!["FAIL".into(), "NON_COMPLIANT".into(), "REJECT".into(), "BLOCK".into()],
        }
    }
}
impl ProjectionConfig {
    /// Reconnaît un rôle de verdict — normalisé (trim + insensible à la casse), **symétrique** avec
    /// [`Self::is_failure`] : « Verdict_Aggregate » et « verdict_aggregate » ne doivent pas se classer
    /// différemment (un rôle de verdict mal-cassé serait sinon traité comme un simple contrôle).
    fn is_verdict_role(&self, role: &str) -> bool {
        self.verdict_roles.iter().any(|r| r.trim().eq_ignore_ascii_case(role.trim()))
    }
    fn is_failure(&self, output: &str) -> bool {
        let up = output.trim().to_ascii_uppercase();
        self.fail_tokens.iter().any(|t| t.trim().to_ascii_uppercase() == up)
    }
}

/// Une **évidence citée** par un contrôle (un nœud `Evidence` résolu depuis `evidence_refs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceCitation {
    pub retrieved_ref: String,
    pub tool: String,
    /// L'empreinte du contenu **tel qu'au moment T** (figé — si la source change après, l'audit montre
    /// toujours la version réellement utilisée).
    pub content_hash: String,
    pub at: u64,
    pub backed_by_node: String,
}

/// Un **contrôle** (un acte d'agent qui n'est pas l'agrégation) : son exigence, sa conclusion, et les
/// évidences qu'il a citées.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// L'exigence contrôlée (= le rôle de l'agent, ex. « sanction_check »).
    pub requirement: String,
    /// La conclusion attestée (l'output du nœud — gardé tel quel, c'est la décision).
    pub outcome: String,
    pub agent_role: String,
    pub model: String,
    pub version: String,
    /// Toujours vrai : S1 scelle un **hachage** des entrées (minimisation GDPR), jamais le brut.
    pub inputs_minimized: bool,
    pub evidence: Vec<EvidenceCitation>,
    pub at: u64,
    pub backed_by_node: String,
}

/// Une **arête** de communication (un message inter-agents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowEdge {
    pub from: String,
    pub to: String,
    pub artifact_type: String,
    pub at: u64,
    pub backed_by_node: String,
}

/// Un **acte de supervision humaine** (oversight EU AI Act art. 14), scellé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OversightAction {
    /// L'acteur (l'officier) — distinct de l'identité du nœud.
    pub human_id: String,
    pub action: HumanAction,
    pub justification: String,
    pub at: u64,
    pub backed_by_node: String,
}

/// La **décision lisible** — la projection pure d'un cas. (L'enveloppe d'intégrité est ajoutée par le
/// service, pas par cette fonction, pour garder `project` pure de toute horloge.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecisionView {
    pub case_id: String,
    pub status: DecisionStatus,
    /// L'output du nœud d'agrégation (présent ssi un rôle de verdict est trouvé ; le PREMIER si plusieurs).
    pub verdict_label: Option<String>,
    /// Le nœud scellé qui PORTE le verdict (le premier rencontré).
    pub verdict_node: Option<String>,
    /// Vrai si le cas porte **plusieurs** nœuds de verdict (ambiguïté) — surfacé, jamais résolu en silence.
    pub verdict_conflict: bool,
    pub checks: Vec<Check>,
    pub flow: Vec<FlowEdge>,
    pub oversight: Vec<OversightAction>,
}

/// **LA PROJECTION** — pure, déterministe, recomputable. `case_records` sont les records du cas dans
/// l'**ordre de chaîne** (= ordre causal, gratuit : une référence précède toujours le nœud qui la cite).
pub fn project(case_records: &[&Record], case_id: &str, config: &ProjectionConfig) -> DecisionView {
    // Index local id → nœud Evidence, pour résoudre les citations (evidence_refs).
    let mut evidence_by_id: std::collections::HashMap<&str, &Node> = std::collections::HashMap::new();
    for r in case_records {
        if let Node::Evidence { evidence_id, .. } = &r.node {
            evidence_by_id.insert(evidence_id.as_str(), &r.node);
        }
    }

    let mut verdict_label = None;
    let mut verdict_node = None;
    let mut verdict_count = 0usize;
    let mut checks = Vec::new();
    let mut flow = Vec::new();
    let mut oversight = Vec::new();

    for r in case_records {
        match &r.node {
            Node::AgentAct { node_id, agent_role, model, version, output, evidence_refs, timestamp, .. } => {
                if config.is_verdict_role(agent_role) {
                    verdict_count += 1;
                    // On garde le PREMIER verdict (sans écraser en silence) ; un 2ᵉ → conflit surfacé.
                    if verdict_node.is_none() {
                        verdict_label = Some(output.clone());
                        verdict_node = Some(node_id.clone());
                    }
                } else {
                    let evidence = evidence_refs
                        .iter()
                        .filter_map(|ref_id| match evidence_by_id.get(ref_id.as_str()) {
                            Some(Node::Evidence { evidence_id, tool, retrieved_ref, content_hash, timestamp, .. }) => {
                                Some(EvidenceCitation {
                                    retrieved_ref: retrieved_ref.clone(),
                                    tool: tool.clone(),
                                    content_hash: content_hash.clone(),
                                    at: *timestamp,
                                    backed_by_node: evidence_id.clone(),
                                })
                            }
                            _ => None, // une réf pendante n'existe pas dans un DAG re-prouvable (S0 valide)
                        })
                        .collect();
                    checks.push(Check {
                        requirement: agent_role.clone(),
                        outcome: output.clone(),
                        agent_role: agent_role.clone(),
                        model: model.clone(),
                        version: version.clone(),
                        inputs_minimized: true,
                        evidence,
                        at: *timestamp,
                        backed_by_node: node_id.clone(),
                    });
                }
            }
            Node::Edge { edge_id, from_node, to_node, artifact_type, timestamp, .. } => {
                flow.push(FlowEdge {
                    from: from_node.clone(),
                    to: to_node.clone(),
                    artifact_type: artifact_type.clone(),
                    at: *timestamp,
                    backed_by_node: edge_id.clone(),
                });
            }
            Node::Human { node_id, human_id, action, justification, timestamp, .. } => {
                oversight.push(OversightAction {
                    human_id: human_id.clone(),
                    action: *action,
                    justification: justification.clone(),
                    at: *timestamp,
                    backed_by_node: node_id.clone(),
                });
            }
            Node::Evidence { .. } => { /* déjà indexée ; apparaît comme citation d'un contrôle */ }
        }
    }

    let verdict_conflict = verdict_count > 1;
    let status = derive_status(&checks, &oversight, verdict_label.as_deref(), verdict_node.is_some(), verdict_conflict, config);
    DecisionView { case_id: case_id.to_string(), status, verdict_label, verdict_node, verdict_conflict, checks, flow, oversight }
}

/// La **règle de statut** — pure, publiée, ré-vérifiable. Décrit l'**état du record**, jamais la justesse.
/// Priorité (haut→bas) :
/// 1. **EXCEPTION** — un humain a outrepassé/escaladé (le signal humain prime).
/// 2. **FAIL** — le **verdict** porte un token d'échec, **ou** un contrôle est en échec, **ou** des
///    verdicts **conflictuels** (le verdict scellé NE peut PAS être ignoré : c'était le défaut « green »).
/// 3. **PENDING** — aucun nœud de verdict n'a conclu (on ne montre jamais « conforme » sans conclusion).
/// 4. **COMPLIANT** — un verdict a conclu, sans échec, sans override.
pub fn derive_status(
    checks: &[Check],
    oversight: &[OversightAction],
    verdict_label: Option<&str>,
    verdict_present: bool,
    verdict_conflict: bool,
    config: &ProjectionConfig,
) -> DecisionStatus {
    if oversight
        .iter()
        .any(|o| matches!(o.action, HumanAction::Override | HumanAction::Escalate))
    {
        return DecisionStatus::Exception;
    }
    let verdict_failed = verdict_label.map(|v| config.is_failure(v)).unwrap_or(false);
    if verdict_conflict || verdict_failed || checks.iter().any(|c| config.is_failure(&c.outcome)) {
        return DecisionStatus::Fail;
    }
    if !verdict_present {
        return DecisionStatus::Pending; // aucune conclusion scellée → jamais « conforme » vert
    }
    DecisionStatus::Compliant
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_dag::{Node, SealedDag, TenantKey};

    fn key() -> TenantKey {
        TenantKey::from_bytes([5u8; 32])
    }

    /// Construit un cas complet réaliste et rend (les records possédés, la clé).
    fn build_case() -> SealedDag {
        let k = key();
        let mut d = SealedDag::new();
        let case = "SUP-2024";
        d.append(&k, Node::AgentAct { node_id: format!("{case}::extract"), agent_role: "supplier_extract".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "{supplier}".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        d.append(&k, Node::Evidence { evidence_id: format!("{case}::ev-pol"), tool: "policy_db".into(), query_hash: "q".into(), retrieved_ref: "policy://4.2".into(), content_hash: "cc".into(), consumed_by_node: format!("{case}::sanction"), timestamp: 2 }).unwrap();
        d.append(&k, Node::AgentAct { node_id: format!("{case}::sanction"), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![format!("{case}::ev-pol")], timestamp: 3 }).unwrap();
        d.append(&k, Node::Edge { edge_id: format!("{case}::e1"), from_node: format!("{case}::extract"), to_node: format!("{case}::sanction"), artifact_hash: "a".into(), artifact_type: "supplier_record".into(), timestamp: 4 }).unwrap();
        d.append(&k, Node::AgentAct { node_id: format!("{case}::aggregate"), agent_role: "verdict_aggregate".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "COMPLIANT".into(), evidence_refs: vec![], timestamp: 5 }).unwrap();
        d
    }

    #[test]
    fn projects_a_full_case_with_backing_nodes() {
        let d = build_case();
        let recs: Vec<&Record> = d.records().iter().collect();
        let v = project(&recs, "SUP-2024", &ProjectionConfig::default());
        assert_eq!(v.status, DecisionStatus::Compliant);
        assert_eq!(v.verdict_label.as_deref(), Some("COMPLIANT"));
        assert_eq!(v.verdict_node.as_deref(), Some("SUP-2024::aggregate"));
        // un seul contrôle (extract + sanction sont des actes ; extract n'est pas un rôle de verdict →
        // c'est aussi un contrôle). Donc 2 contrôles (extract, sanction), 0 verdict parmi eux.
        assert_eq!(v.checks.len(), 2, "extract + sanction");
        let sanction = v.checks.iter().find(|c| c.requirement == "sanction_check").unwrap();
        assert_eq!(sanction.outcome, "PASS");
        assert_eq!(sanction.backed_by_node, "SUP-2024::sanction");
        assert!(sanction.inputs_minimized);
        assert_eq!(sanction.evidence.len(), 1, "la citation d'évidence est résolue");
        assert_eq!(sanction.evidence[0].retrieved_ref, "policy://4.2");
        assert_eq!(sanction.evidence[0].backed_by_node, "SUP-2024::ev-pol");
        assert_eq!(v.flow.len(), 1);
        assert_eq!(v.flow[0].from, "SUP-2024::extract");
    }

    #[test]
    fn projection_is_pure_and_deterministic() {
        let d = build_case();
        let recs: Vec<&Record> = d.records().iter().collect();
        let cfg = ProjectionConfig::default();
        let a = project(&recs, "SUP-2024", &cfg);
        let b = project(&recs, "SUP-2024", &cfg);
        assert_eq!(a, b, "mêmes records → même vue, bit-à-bit");
        // chaque champ projeté nomme un backed_by_node présent dans les records.
        let ids: std::collections::HashSet<&str> = recs.iter().map(|r| r.node.id()).collect();
        for c in &a.checks {
            assert!(ids.contains(c.backed_by_node.as_str()), "contrôle adossé à un nœud réel");
        }
        if let Some(vn) = &a.verdict_node {
            assert!(ids.contains(vn.as_str()));
        }
    }

    #[test]
    fn status_fail_when_a_check_fails() {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "C::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "FAIL".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        let recs: Vec<&Record> = d.records().iter().collect();
        let v = project(&recs, "C", &ProjectionConfig::default());
        assert_eq!(v.status, DecisionStatus::Fail);
    }

    #[test]
    fn status_exception_when_human_overrides() {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "C::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "FAIL".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        d.append(&k, Node::Human { node_id: "C::rev1".into(), human_id: "officer:1".into(), action: HumanAction::Override, target_node: "C::sanction".into(), justification: "raison".into(), timestamp: 2 }).unwrap();
        let recs: Vec<&Record> = d.records().iter().collect();
        let v = project(&recs, "C", &ProjectionConfig::default());
        assert_eq!(v.status, DecisionStatus::Exception, "l'override humain prime sur le FAIL");
        assert_eq!(v.oversight.len(), 1);
        assert_eq!(v.oversight[0].action, HumanAction::Override);
    }

    #[test]
    fn failing_verdict_node_is_not_green() {
        // RÉGRESSION (finding HIGH) : un verdict d'agrégation REJECT/NON_COMPLIANT NE doit PAS rendre
        // COMPLIANT vert — le verdict scellé est désormais consulté par derive_status.
        let k = key();
        for token in ["REJECT", "NON_COMPLIANT", "fail", "  Block  "] {
            let mut d = SealedDag::new();
            d.append(&k, Node::AgentAct { node_id: "C::aggregate".into(), agent_role: "verdict_aggregate".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: token.into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
            let recs: Vec<&Record> = d.records().iter().collect();
            let v = project(&recs, "C", &ProjectionConfig::default());
            assert_eq!(v.status, DecisionStatus::Fail, "verdict « {token} » → FAIL, jamais COMPLIANT");
            assert_eq!(v.verdict_label.as_deref(), Some(token), "le verdict reste affiché tel quel");
        }
    }

    #[test]
    fn no_verdict_node_is_pending_not_compliant() {
        // finding MEDIUM : un cas sans nœud de verdict (en cours / seulement évidences) → PENDING, pas vert.
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "C::extract".into(), agent_role: "supplier_extract".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "{supplier}".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        let recs: Vec<&Record> = d.records().iter().collect();
        let v = project(&recs, "C", &ProjectionConfig::default());
        assert_eq!(v.status, DecisionStatus::Pending, "aucune conclusion scellée → PENDING");
        assert!(v.verdict_node.is_none());
    }

    #[test]
    fn conflicting_verdicts_are_surfaced_and_fail() {
        // finding MEDIUM : deux nœuds de verdict → conflit surfacé (pas de last-wins silencieux) + FAIL.
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "C::agg1".into(), agent_role: "verdict_aggregate".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "COMPLIANT".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        d.append(&k, Node::AgentAct { node_id: "C::agg2".into(), agent_role: "verdict_aggregate".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "COMPLIANT".into(), evidence_refs: vec![], timestamp: 2 }).unwrap();
        let recs: Vec<&Record> = d.records().iter().collect();
        let v = project(&recs, "C", &ProjectionConfig::default());
        assert!(v.verdict_conflict, "deux verdicts → conflit surfacé");
        assert_eq!(v.status, DecisionStatus::Fail, "un conflit de verdict ne peut pas lire COMPLIANT");
        assert_eq!(v.verdict_node.as_deref(), Some("C::agg1"), "le premier verdict est gardé (pas écrasé en silence)");
    }

    #[test]
    fn verdict_role_match_is_case_insensitive() {
        // finding LOW : un rôle de verdict mal-cassé ne doit pas être pris pour un simple contrôle.
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "C::agg".into(), agent_role: "Verdict_Aggregate".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "COMPLIANT".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        let recs: Vec<&Record> = d.records().iter().collect();
        let v = project(&recs, "C", &ProjectionConfig::default());
        assert_eq!(v.verdict_node.as_deref(), Some("C::agg"), "rôle de verdict reconnu malgré la casse");
        assert!(v.checks.is_empty(), "le verdict n'est pas classé comme contrôle");
        assert_eq!(v.status, DecisionStatus::Compliant);
    }
}

