// SPDX-License-Identifier: UNLICENSED
//! **Les taps** — convertissent les événements observés du runtime en **nœuds scellables** (S0).
//!
//! Quatre adaptateurs purs : un appel modèle → **acte** ; un message → **arête** ; une consultation
//! (outil/retrieval) → **évidence** ; une revue → **supervision humaine**. Les entrées sensibles sont
//! **hachées** (minimisation : on scelle l'empreinte, pas forcément les données brutes — GDPR). Le **cas**
//! est porté DANS l'identifiant de nœud (`{case}::{étape}`) : il devient ainsi **scellé** (donc auditable)
//! et **requêtable**, **sans** modifier le contrat S0 (les types de nœuds restent intacts).

use provenance_dag::{sha256, HumanAction, Node};

/// `sha256` → hex (pour les empreintes d'entrées/contenus).
pub fn hash_hex(data: &[u8]) -> String {
    let d = sha256(data);
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// L'identifiant d'une étape dans un cas : `{case}::{step}`. Le séparateur `::` est **réservé** ; le
/// store l'**impose** (`CaptureStore::record_*` refuse un cas/étape qui le contient ou qui est vide,
/// via `check_segment`) — sans quoi deux paires (cas, étape) distinctes pourraient produire le même id
/// et la requête par cas mal-attribuerait. Cette fonction reste pure (le contrôle vit au point d'entrée).
pub fn node_id(case_id: &str, step: &str) -> String {
    format!("{case_id}::{step}")
}

/// Un **appel modèle** observé par la passerelle → un nœud ACTE. `inputs` est haché (minimisation) ;
/// `output` est gardé (la décision attestée). `evidence_steps` réfère des étapes d'évidence du même cas.
pub struct LlmCall<'a> {
    pub case_id: &'a str,
    pub step: &'a str,
    pub agent_role: &'a str,
    pub inputs: &'a [u8],
    pub model: &'a str,
    pub version: &'a str,
    pub output: &'a str,
    pub evidence_steps: &'a [&'a str],
    pub timestamp: u64,
}
impl LlmCall<'_> {
    pub fn to_node(&self) -> Node {
        Node::AgentAct {
            node_id: node_id(self.case_id, self.step),
            agent_role: self.agent_role.to_string(),
            inputs_hash: hash_hex(self.inputs),
            model: self.model.to_string(),
            version: self.version.to_string(),
            output: self.output.to_string(),
            evidence_refs: self.evidence_steps.iter().map(|s| node_id(self.case_id, s)).collect(),
            timestamp: self.timestamp,
        }
    }
}

/// Un **message** observé entre deux agents → un nœud ARÊTE (la communication). `artifact` est haché.
pub struct MessageEvent<'a> {
    pub case_id: &'a str,
    pub step: &'a str,
    pub from_step: &'a str,
    pub to_step: &'a str,
    pub artifact: &'a [u8],
    pub artifact_type: &'a str,
    pub timestamp: u64,
}
impl MessageEvent<'_> {
    pub fn to_node(&self) -> Node {
        Node::Edge {
            edge_id: node_id(self.case_id, self.step),
            from_node: node_id(self.case_id, self.from_step),
            to_node: node_id(self.case_id, self.to_step),
            artifact_hash: hash_hex(self.artifact),
            artifact_type: self.artifact_type.to_string(),
            timestamp: self.timestamp,
        }
    }
}

/// Une **consultation** (outil/retrieval) observée → un nœud ÉVIDENCE (une **source** : on l'enregistre
/// quand elle est récupérée, AVANT que l'acte qui la cite — via `evidence_steps` — ne soit enregistré).
/// `query` et `content` sont hachés ; `content_hash` fige l'évidence **telle qu'au moment T** (si la
/// source change après, l'audit montre la version réellement utilisée). `consumed_by_step` annote
/// l'étape consommatrice (scellé, mais non-causal — cf. [`provenance_dag::Node::refs`]).
pub struct ToolCall<'a> {
    pub case_id: &'a str,
    pub step: &'a str,
    pub tool: &'a str,
    pub query: &'a [u8],
    pub retrieved_ref: &'a str,
    pub content: &'a [u8],
    pub consumed_by_step: &'a str,
    pub timestamp: u64,
}
impl ToolCall<'_> {
    pub fn to_node(&self) -> Node {
        Node::Evidence {
            evidence_id: node_id(self.case_id, self.step),
            tool: self.tool.to_string(),
            query_hash: hash_hex(self.query),
            retrieved_ref: self.retrieved_ref.to_string(),
            content_hash: hash_hex(self.content),
            consumed_by_node: node_id(self.case_id, self.consumed_by_step),
            timestamp: self.timestamp,
        }
    }
}

/// Une **revue humaine** observée → un nœud SUPERVISION (oversight EU AI Act art. 14). Réfère l'étape
/// visée. `step` nomme l'**acte de revue** (son identité dans le cas, ex. « review-1 ») : distinct de
/// `human_id` (l'**acteur**, l'officier) — un même officier révise beaucoup de cas sans collisionner.
pub struct HumanReview<'a> {
    pub case_id: &'a str,
    pub step: &'a str,
    pub human_id: &'a str,
    pub action: HumanAction,
    pub target_step: &'a str,
    pub justification: &'a str,
    pub timestamp: u64,
}
impl HumanReview<'_> {
    pub fn to_node(&self) -> Node {
        Node::Human {
            node_id: node_id(self.case_id, self.step),
            human_id: self.human_id.to_string(),
            action: self.action,
            target_node: node_id(self.case_id, self.target_step),
            justification: self.justification.to_string(),
            timestamp: self.timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_minimizes() {
        let h1 = hash_hex(b"donnee-sensible-PII");
        let h2 = hash_hex(b"donnee-sensible-PII");
        assert_eq!(h1, h2, "déterministe");
        assert_eq!(h1.len(), 64, "empreinte hex de 32 octets");
        assert_ne!(h1, hash_hex(b"autre"), "empreintes distinctes");
        // SHA-256("") connu (le hachage est bien celui de provenance-dag).
        assert_eq!(hash_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn llm_call_embeds_case_and_hashes_inputs() {
        let call = LlmCall {
            case_id: "SUP-2024",
            step: "sanction",
            agent_role: "sanction_check",
            inputs: b"nom: ACME; pays: FR",
            model: "phi4-mini",
            version: "1",
            output: "PASS",
            evidence_steps: &["ev-policy"],
            timestamp: 3,
        };
        match call.to_node() {
            Node::AgentAct { node_id, inputs_hash, output, evidence_refs, .. } => {
                assert_eq!(node_id, "SUP-2024::sanction", "le cas est dans l'id (scellé, requêtable)");
                assert_eq!(inputs_hash, hash_hex(b"nom: ACME; pays: FR"), "entrée hachée (minimisation)");
                assert_eq!(output, "PASS", "sortie gardée (décision attestée)");
                assert_eq!(evidence_refs, vec!["SUP-2024::ev-policy".to_string()], "réfs d'évidence namespacées");
            }
            _ => panic!("doit être un AgentAct"),
        }
    }

    #[test]
    fn message_and_evidence_and_human_namespacing() {
        let m = MessageEvent { case_id: "c1", step: "e1", from_step: "extract", to_step: "sanction", artifact: b"rec", artifact_type: "supplier_record", timestamp: 4 };
        if let Node::Edge { from_node, to_node, artifact_hash, .. } = m.to_node() {
            assert_eq!((from_node.as_str(), to_node.as_str()), ("c1::extract", "c1::sanction"));
            assert_eq!(artifact_hash, hash_hex(b"rec"));
        } else {
            panic!()
        }
        let t = ToolCall { case_id: "c1", step: "ev", tool: "policy_db", query: b"q", retrieved_ref: "policy://4.2", content: b"clause", consumed_by_step: "sanction", timestamp: 2 };
        if let Node::Evidence { evidence_id, content_hash, consumed_by_node, .. } = t.to_node() {
            assert_eq!(evidence_id, "c1::ev");
            assert_eq!(content_hash, hash_hex(b"clause"), "content_hash fige l'évidence");
            assert_eq!(consumed_by_node, "c1::sanction");
        } else {
            panic!()
        }
        let h = HumanReview { case_id: "c1", step: "review-1", human_id: "officer:42", action: HumanAction::Override, target_step: "aggregate", justification: "raison", timestamp: 7 };
        if let Node::Human { node_id, human_id, target_node, action, .. } = h.to_node() {
            assert_eq!(node_id, "c1::review-1", "l'acte de revue est namespacé par cas (requêtable, sans collision)");
            assert_eq!(human_id, "officer:42", "l'acteur reste distinct de l'identité du nœud");
            assert_eq!(target_node, "c1::aggregate");
            assert_eq!(action, HumanAction::Override);
        } else {
            panic!()
        }
    }
}
