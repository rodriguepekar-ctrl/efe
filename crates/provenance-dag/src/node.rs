// SPDX-License-Identifier: UNLICENSED
//! **Les nœuds typés du DAG** — acte d'agent, arête (message), évidence (consultation), humain
//! (supervision) — et leur **encodage canonique**.
//!
//! Point d'architecture décisif : le **sceau est calculé sur l'encodage CANONIQUE BINAIRE** (préfixé
//! en longueur, ordre de champ figé), **PAS** sur le JSON (dont l'ordre des clés et les espaces sont
//! ambigus). Le JSON ne sert qu'au stockage/inspection humaine ; la re-preuve recompose l'encodage
//! canonique depuis les champs et recalcule la chaîne. Déterministe et non-ambigu par construction.

use serde::{Deserialize, Serialize};

/// L'action d'un acte de **supervision humaine** (EU AI Act art. 14 — l'oversight devient provenance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanAction {
    Approve,
    Override,
    Escalate,
    Comment,
}
impl HumanAction {
    /// Étiquette d'un octet pour l'encodage canonique (stable — ne jamais renuméroter).
    fn tag(self) -> u8 {
        match self {
            HumanAction::Approve => 1,
            HumanAction::Override => 2,
            HumanAction::Escalate => 3,
            HumanAction::Comment => 4,
        }
    }
}

/// Un **nœud** du DAG (le contenu d'un record ; le sceau est ajouté par le [`crate::dag`]).
/// Sérialisé en JSON **interne-taggé** par `kind` (une ligne JSONL plate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    /// L'**acte** d'un agent : ce qu'il a reçu (haché — minimisation GDPR), produit (gardé — la
    /// décision attestée), avec quel modèle, et quelles évidences il a citées.
    AgentAct {
        node_id: String,
        agent_role: String,
        /// Hachage des entrées (minimisation : on scelle l'empreinte, pas forcément les données brutes).
        inputs_hash: String,
        model: String,
        version: String,
        /// La sortie attestée (la décision/artefact — gardée pour l'audit).
        output: String,
        /// Réfs vers les nœuds d'évidence cités (`evidence_id`).
        evidence_refs: Vec<String>,
        timestamp: u64,
    },
    /// Une **arête** : un artefact typé passé d'un nœud-agent à un autre (la communication).
    Edge {
        edge_id: String,
        from_node: String,
        to_node: String,
        artifact_hash: String,
        /// Le **type** de l'artefact (la grammaire « artefacts typés », pas du consensus libre).
        artifact_type: String,
        timestamp: u64,
    },
    /// Une **évidence** : une consultation (outil/retrieval) figée au moment de la décision. C'est un
    /// nœud **source** (récupéré AVANT d'être consommé — voir [`Node::refs`]).
    Evidence {
        evidence_id: String,
        tool: String,
        query_hash: String,
        retrieved_ref: String,
        /// Empreinte du contenu **tel qu'il était au moment T** (si la source change après, l'audit
        /// montre toujours la version réellement utilisée).
        content_hash: String,
        /// L'étape qui consomme cette évidence — **annotation scellée** (infalsifiable), pas un
        /// prérequis causal : l'arête causale validée est `AgentAct.evidence_refs` (acte → évidence).
        consumed_by_node: String,
        timestamp: u64,
    },
    /// Un acte de **supervision humaine** (oversight) — scellé au même titre qu'un acte d'agent.
    ///
    /// `node_id` est l'**identité du nœud** (unique dans le DAG) ; `human_id` est l'**acteur** (le
    /// réviseur). Les deux sont distincts à dessein : un même officier de conformité révise BEAUCOUP
    /// de cas — confondre les deux ferait collisionner ses revues (`DuplicateId`) et les rendrait
    /// non-requêtables par cas. `node_id` porte donc la convention `{cas}::{étape}` ; `human_id`
    /// reste l'identité (auditée) de la personne.
    Human {
        node_id: String,
        human_id: String,
        action: HumanAction,
        target_node: String,
        justification: String,
        timestamp: u64,
    },
}

/// Ajoute une chaîne préfixée par sa longueur (u32 big-endian) — non-ambigu.
fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}
/// Ajoute un u64 big-endian.
fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}

impl Node {
    /// L'**encodage canonique** du nœud : un octet de type, puis chaque champ dans un ordre FIGÉ,
    /// préfixé en longueur. C'est ce sur quoi le sceau est calculé — déterministe et non-ambigu.
    pub(crate) fn canonical(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            Node::AgentAct { node_id, agent_role, inputs_hash, model, version, output, evidence_refs, timestamp } => {
                o.push(1);
                put_str(&mut o, node_id);
                put_str(&mut o, agent_role);
                put_str(&mut o, inputs_hash);
                put_str(&mut o, model);
                put_str(&mut o, version);
                put_str(&mut o, output);
                o.extend_from_slice(&(evidence_refs.len() as u32).to_be_bytes());
                for e in evidence_refs {
                    put_str(&mut o, e);
                }
                put_u64(&mut o, *timestamp);
            }
            Node::Edge { edge_id, from_node, to_node, artifact_hash, artifact_type, timestamp } => {
                o.push(2);
                put_str(&mut o, edge_id);
                put_str(&mut o, from_node);
                put_str(&mut o, to_node);
                put_str(&mut o, artifact_hash);
                put_str(&mut o, artifact_type);
                put_u64(&mut o, *timestamp);
            }
            Node::Evidence { evidence_id, tool, query_hash, retrieved_ref, content_hash, consumed_by_node, timestamp } => {
                o.push(3);
                put_str(&mut o, evidence_id);
                put_str(&mut o, tool);
                put_str(&mut o, query_hash);
                put_str(&mut o, retrieved_ref);
                put_str(&mut o, content_hash);
                put_str(&mut o, consumed_by_node);
                put_u64(&mut o, *timestamp);
            }
            Node::Human { node_id, human_id, action, target_node, justification, timestamp } => {
                o.push(4);
                put_str(&mut o, node_id);
                put_str(&mut o, human_id);
                o.push(action.tag());
                put_str(&mut o, target_node);
                put_str(&mut o, justification);
                put_u64(&mut o, *timestamp);
            }
        }
        o
    }

    /// L'**identité** du nœud (son id, unique dans le DAG).
    pub fn id(&self) -> &str {
        match self {
            Node::AgentAct { node_id, .. } => node_id,
            Node::Edge { edge_id, .. } => edge_id,
            Node::Evidence { evidence_id, .. } => evidence_id,
            Node::Human { node_id, .. } => node_id,
        }
    }

    /// Les ids que ce nœud **référence** (doivent exister AVANT lui — causalité du DAG append-only).
    ///
    /// - `AgentAct` référence les **évidences qu'il cite** (`evidence_refs`) : c'est l'arête causale
    ///   acte→évidence, et elle est **réellement imposée** (append/re-preuve rejettent une citation
    ///   pendante). Pas de cycle : une évidence est toujours enregistrée AVANT l'acte qui la cite.
    /// - `Evidence` est la seule **source pure** : *récupérée* avant d'être consommée — la forcer à
    ///   pointer vers son consommateur créerait un cycle. `consumed_by_node` reste un champ **scellé**
    ///   (infalsifiable) mais purement **annotatif** : un pointeur-retour, pas un prérequis causal.
    /// - Arête/humain pointent vers des nœuds qui doivent préexister.
    pub fn refs(&self) -> Vec<&str> {
        match self {
            Node::AgentAct { evidence_refs, .. } => evidence_refs.iter().map(String::as_str).collect(),
            Node::Evidence { .. } => Vec::new(),
            Node::Edge { from_node, to_node, .. } => vec![from_node.as_str(), to_node.as_str()],
            Node::Human { target_node, .. } => vec![target_node.as_str()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn act(id: &str) -> Node {
        Node::AgentAct {
            node_id: id.into(),
            agent_role: "sanction_check".into(),
            inputs_hash: "h-in".into(),
            model: "phi4-mini".into(),
            version: "1".into(),
            output: "PASS".into(),
            evidence_refs: vec!["ev-1".into()],
            timestamp: 1000,
        }
    }

    #[test]
    fn canonical_is_deterministic_and_field_sensitive() {
        let a = act("n1");
        assert_eq!(a.canonical(), a.canonical(), "déterministe");
        // Changer un seul champ change l'encodage.
        let mut b = a.clone();
        if let Node::AgentAct { output, .. } = &mut b {
            *output = "FAIL".into();
        }
        assert_ne!(a.canonical(), b.canonical(), "un champ modifié ⇒ encodage différent");
    }

    #[test]
    fn length_prefix_prevents_concatenation_ambiguity() {
        // ("ab","c") et ("a","bc") ne doivent PAS produire le même encodage (préfixe de longueur).
        let x = Node::Edge { edge_id: "ab".into(), from_node: "c".into(), to_node: "t".into(), artifact_hash: "h".into(), artifact_type: "verdict".into(), timestamp: 1 };
        let y = Node::Edge { edge_id: "a".into(), from_node: "bc".into(), to_node: "t".into(), artifact_hash: "h".into(), artifact_type: "verdict".into(), timestamp: 1 };
        assert_ne!(x.canonical(), y.canonical(), "pas d'ambiguïté de concaténation");
    }

    #[test]
    fn refs_and_id() {
        assert_eq!(act("n1").id(), "n1");
        assert_eq!(act("n1").refs(), vec!["ev-1"], "un acte cite (et valide) ses évidences");
        let e = Node::Edge { edge_id: "e1".into(), from_node: "n1".into(), to_node: "n2".into(), artifact_hash: "h".into(), artifact_type: "t".into(), timestamp: 1 };
        assert_eq!(e.refs(), vec!["n1", "n2"]);
        // Une évidence est une SOURCE : elle ne référence rien (récupérée avant d'être consommée ;
        // `consumed_by_node` est annotatif, pas un prérequis — sinon cycle causal avec evidence_refs).
        let ev = Node::Evidence { evidence_id: "ev1".into(), tool: "db".into(), query_hash: "h".into(), retrieved_ref: "r".into(), content_hash: "h".into(), consumed_by_node: "futur".into(), timestamp: 1 };
        assert_eq!(ev.id(), "ev1");
        assert!(ev.refs().is_empty(), "l'évidence est une source");
        let h = Node::Human { node_id: "c::rev1".into(), human_id: "hu1".into(), action: HumanAction::Override, target_node: "n2".into(), justification: "raison".into(), timestamp: 1 };
        assert_eq!(h.id(), "c::rev1", "l'identité du nœud est node_id, pas l'acteur");
        assert_eq!(h.refs(), vec!["n2"]);
    }

    #[test]
    fn json_roundtrip_is_flat() {
        let a = act("n1");
        let j = serde_json::to_string(&a).unwrap();
        assert!(j.contains("\"kind\":\"agent_act\""), "tag interne plat : {j}");
        let back: Node = serde_json::from_str(&j).unwrap();
        assert_eq!(a, back);
    }
}
