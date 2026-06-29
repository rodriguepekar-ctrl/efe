// SPDX-License-Identifier: UNLICENSED
//! **Le langage de sélecteurs** — un prédicat **DÉCLARATIF et FERMÉ** sur les nœuds du DAG.
//!
//! Un mapping (cf. [`crate::mapping`]) doit dire « tels nœuds évidencient telle exigence ». La tentation
//! serait d'y mettre du **code arbitraire** ; le choix d'architecture de S6 est l'inverse : un **petit
//! langage fermé** (type de nœud, présence/égalité de champ, action humaine, conjonction/disjonction).
//! Cette fermeture donne trois propriétés décisives — toutes nécessaires au caractère **trust-light** :
//!
//! 1. **Lisible à l'œil** : un juriste peut lire le sélecteur et contester la revendication.
//! 2. **Sérialisable** : le sélecteur voyage dans `mapping.<version>.json` du bundle (le tiers re-dérive).
//! 3. **Déterministe** : un prédicat évalué sur des records ordonnés → mêmes ids, dans l'ordre de chaîne.
//!
//! Et surtout : un sélecteur ne peut, par construction, que **SÉLECTIONNER** des nœuds qui existent ; il
//! ne peut pas en **INVENTER**. C'est l'anti-citation-fantôme au niveau du mapping (sœur de la garde de S4).

use provenance_dag::{HumanAction, Node, Record};
use serde::{Deserialize, Serialize};

/// Le **type** d'un nœud — la dimension de sélection la plus grossière.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    AgentAct,
    Edge,
    Evidence,
    Human,
}

impl NodeKind {
    /// Le type d'un nœud concret.
    fn of(node: &Node) -> NodeKind {
        match node {
            Node::AgentAct { .. } => NodeKind::AgentAct,
            Node::Edge { .. } => NodeKind::Edge,
            Node::Evidence { .. } => NodeKind::Evidence,
            Node::Human { .. } => NodeKind::Human,
        }
    }
}

/// Un **champ sélectionnable** — un ensemble **FERMÉ** (pas de réflexion arbitraire). C'est ce qui garde
/// le sélecteur lisible, sérialisable, et déterministe. Un champ absent pour un type de nœud donné rend
/// simplement `None` (le prédicat est alors faux pour ce nœud — jamais une erreur).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// `AgentAct.agent_role` — l'exigence contrôlée (ex. « sanction_check »).
    AgentRole,
    /// `AgentAct.model` — le modèle qui a conclu.
    Model,
    /// `AgentAct.output` — la décision attestée (gardée telle quelle).
    Output,
    /// `AgentAct.inputs_hash` — l'empreinte d'entrée (minimisation native : présent ⇒ haché).
    InputsHash,
    /// `Evidence.tool` — la source consultée (ex. « policy_db »).
    Tool,
    /// `Evidence.retrieved_ref` — la référence citée (ex. « policy://4.2 »).
    RetrievedRef,
    /// `Evidence.content_hash` — l'empreinte du contenu **au moment T** (figé).
    ContentHash,
    /// `Edge.artifact_type` — le type d'artefact passé (la grammaire de communication).
    ArtifactType,
    /// `Human.justification` — le motif saisi par l'humain.
    Justification,
    /// `Human.human_id` — l'**acteur** (l'officier), distinct de l'identité du nœud.
    HumanId,
}

impl Field {
    /// Lit le champ d'un nœud — `None` si le champ n'existe pas pour ce type de nœud.
    fn read(self, node: &Node) -> Option<&str> {
        match (self, node) {
            (Field::AgentRole, Node::AgentAct { agent_role, .. }) => Some(agent_role),
            (Field::Model, Node::AgentAct { model, .. }) => Some(model),
            (Field::Output, Node::AgentAct { output, .. }) => Some(output),
            (Field::InputsHash, Node::AgentAct { inputs_hash, .. }) => Some(inputs_hash),
            (Field::Tool, Node::Evidence { tool, .. }) => Some(tool),
            (Field::RetrievedRef, Node::Evidence { retrieved_ref, .. }) => Some(retrieved_ref),
            (Field::ContentHash, Node::Evidence { content_hash, .. }) => Some(content_hash),
            (Field::ArtifactType, Node::Edge { artifact_type, .. }) => Some(artifact_type),
            (Field::Justification, Node::Human { justification, .. }) => Some(justification),
            (Field::HumanId, Node::Human { human_id, .. }) => Some(human_id),
            _ => None,
        }
    }
}

/// Un **prédicat déclaratif** sur un nœud — l'unité de sélecteur. Fermé, sérialisable, déterministe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Selector {
    /// Tous les nœuds d'un **type** donné.
    Kind(NodeKind),
    /// Le **champ est présent ET non vide** (après trim).
    FieldPresent(Field),
    /// Le champ **égale** une valeur (trim + insensible à la casse — symétrique avec la projection S2).
    FieldEquals { field: Field, value: String },
    /// L'**action humaine** est dans l'ensemble (faux pour un nœud non-humain).
    HumanActionIn(Vec<HumanAction>),
    /// **Conjonction** — tous vrais. `All([])` = vrai (élément neutre).
    All(Vec<Selector>),
    /// **Disjonction** — au moins un vrai. `Any([])` = faux.
    Any(Vec<Selector>),
}

impl Selector {
    /// Évalue le prédicat sur un nœud — **pur et déterministe** (aucune horloge, aucun état caché).
    pub fn matches(&self, node: &Node) -> bool {
        match self {
            Selector::Kind(k) => NodeKind::of(node) == *k,
            Selector::FieldPresent(f) => f.read(node).map(|s| !s.trim().is_empty()).unwrap_or(false),
            Selector::FieldEquals { field, value } => {
                field.read(node).map(|s| s.trim().eq_ignore_ascii_case(value.trim())).unwrap_or(false)
            }
            Selector::HumanActionIn(set) => match node {
                Node::Human { action, .. } => set.contains(action),
                _ => false,
            },
            Selector::All(ss) => ss.iter().all(|s| s.matches(node)),
            Selector::Any(ss) => ss.iter().any(|s| s.matches(node)),
        }
    }
}

/// Sélectionne, dans l'**ordre de chaîne** (déterministe), les `node_id` des records qui satisfont le
/// sélecteur. Ne rend que des ids **présents** (les records passés) — il ne peut rien inventer.
pub fn select_ids(records: &[&Record], selector: &Selector) -> Vec<String> {
    records
        .iter()
        .filter(|r| selector.matches(&r.node))
        .map(|r| r.node.id().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_dag::{SealedDag, TenantKey};

    fn key() -> TenantKey {
        TenantKey::from_bytes([9u8; 32])
    }

    /// Un petit DAG : acte (sanction PASS, cite une évidence) ; évidence ; humain (override).
    fn dag() -> SealedDag {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::Evidence { evidence_id: "C::ev".into(), tool: "policy_db".into(), query_hash: "q".into(), retrieved_ref: "policy://4.2".into(), content_hash: "cc".into(), consumed_by_node: "C::sanction".into(), timestamp: 1 }).unwrap();
        d.append(&k, Node::AgentAct { node_id: "C::sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h".into(), model: "phi4".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec!["C::ev".into()], timestamp: 2 }).unwrap();
        d.append(&k, Node::Human { node_id: "C::rev".into(), human_id: "officer:1".into(), action: HumanAction::Override, target_node: "C::sanction".into(), justification: "motif".into(), timestamp: 3 }).unwrap();
        d
    }

    #[test]
    fn kind_selector_filters_by_type() {
        let d = dag();
        let recs: Vec<&Record> = d.records().iter().collect();
        assert_eq!(select_ids(&recs, &Selector::Kind(NodeKind::AgentAct)), vec!["C::sanction"]);
        assert_eq!(select_ids(&recs, &Selector::Kind(NodeKind::Evidence)), vec!["C::ev"]);
        assert_eq!(select_ids(&recs, &Selector::Kind(NodeKind::Human)), vec!["C::rev"]);
        assert!(select_ids(&recs, &Selector::Kind(NodeKind::Edge)).is_empty());
    }

    #[test]
    fn field_present_and_equals() {
        let d = dag();
        let recs: Vec<&Record> = d.records().iter().collect();
        // content_hash présent ⇒ l'évidence figée.
        assert_eq!(select_ids(&recs, &Selector::FieldPresent(Field::ContentHash)), vec!["C::ev"]);
        // inputs_hash présent ⇒ minimisation native (l'acte).
        assert_eq!(select_ids(&recs, &Selector::FieldPresent(Field::InputsHash)), vec!["C::sanction"]);
        // égalité de rôle, insensible à la casse.
        assert_eq!(select_ids(&recs, &Selector::FieldEquals { field: Field::AgentRole, value: "Sanction_Check".into() }), vec!["C::sanction"]);
    }

    #[test]
    fn human_action_in_set() {
        let d = dag();
        let recs: Vec<&Record> = d.records().iter().collect();
        let oversight = Selector::All(vec![
            Selector::Kind(NodeKind::Human),
            Selector::HumanActionIn(vec![HumanAction::Approve, HumanAction::Override, HumanAction::Escalate]),
        ]);
        assert_eq!(select_ids(&recs, &oversight), vec!["C::rev"]);
        // Un ensemble qui ne contient pas Override → vide.
        let only_approve = Selector::HumanActionIn(vec![HumanAction::Approve]);
        assert!(select_ids(&recs, &only_approve).is_empty());
    }

    #[test]
    fn any_and_all_compose() {
        let d = dag();
        let recs: Vec<&Record> = d.records().iter().collect();
        let any_log = Selector::Any(vec![
            Selector::Kind(NodeKind::AgentAct),
            Selector::Kind(NodeKind::Edge),
            Selector::Kind(NodeKind::Evidence),
            Selector::Kind(NodeKind::Human),
        ]);
        // Tous les records, dans l'ordre de chaîne (déterministe).
        assert_eq!(select_ids(&recs, &any_log), vec!["C::ev", "C::sanction", "C::rev"]);
        // All([]) est neutre (vrai) → sélectionne tout ; Any([]) est faux → rien.
        assert_eq!(select_ids(&recs, &Selector::All(vec![])).len(), 3);
        assert!(select_ids(&recs, &Selector::Any(vec![])).is_empty());
    }

    #[test]
    fn selector_round_trips_through_json() {
        // Le sélecteur voyage dans le bundle → il doit re-désérialiser à l'identique.
        let sel = Selector::All(vec![
            Selector::Kind(NodeKind::Human),
            Selector::HumanActionIn(vec![HumanAction::Override]),
            Selector::FieldPresent(Field::Justification),
        ]);
        let json = serde_json::to_string(&sel).unwrap();
        let back: Selector = serde_json::from_str(&json).unwrap();
        assert_eq!(sel, back, "le sélecteur re-désérialise à l'identique (il voyage)");
    }

    #[test]
    fn select_is_deterministic() {
        let d = dag();
        let recs: Vec<&Record> = d.records().iter().collect();
        let sel = Selector::Kind(NodeKind::AgentAct);
        assert_eq!(select_ids(&recs, &sel), select_ids(&recs, &sel), "même entrée → même sortie");
    }
}
