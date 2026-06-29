// SPDX-License-Identifier: UNLICENSED
//! **Le record & son encodage canonique** — ré-implémentation **fidèle** de `provenance-dag::node`, sans
//! serde. Le sceau est calculé sur l'**encodage canonique BINAIRE** (octet de type, champs en ordre figé,
//! préfixés en longueur) — **pas** sur le JSON. verify-mini parse le JSON (cf. [`crate::json`]) pour en
//! extraire les champs, puis **recompose** cet encodage exact. Le cross-check contre S0 (`tests/`) prouve
//! que les octets concordent (sinon le manifest-root différerait).

use crate::json::{parse_object, Obj};

/// Un nœud typé (mêmes variantes/champs que S0). L'action humaine est stockée par son **tag** (1..4),
/// l'octet exact qu'attend l'encodage canonique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiniNode {
    AgentAct { node_id: String, agent_role: String, inputs_hash: String, model: String, version: String, output: String, evidence_refs: Vec<String>, timestamp: u64 },
    Edge { edge_id: String, from_node: String, to_node: String, artifact_hash: String, artifact_type: String, timestamp: u64 },
    Evidence { evidence_id: String, tool: String, query_hash: String, retrieved_ref: String, content_hash: String, consumed_by_node: String, timestamp: u64 },
    Human { node_id: String, human_id: String, action_tag: u8, target_node: String, justification: String, timestamp: u64 },
}

/// Un record scellé : le nœud + le sceau (hex).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MiniRecord {
    pub node: MiniNode,
    pub seal: String,
}

fn put_str(o: &mut Vec<u8>, s: &str) {
    o.extend_from_slice(&(s.len() as u32).to_be_bytes());
    o.extend_from_slice(s.as_bytes());
}
fn put_u64(o: &mut Vec<u8>, v: u64) {
    o.extend_from_slice(&v.to_be_bytes());
}

impl MiniNode {
    fn from_obj(o: &Obj) -> Result<MiniNode, String> {
        let s = |k: &str| o.get_str(k).map(str::to_string);
        match o.get_str("kind")? {
            "agent_act" => Ok(MiniNode::AgentAct {
                node_id: s("node_id")?,
                agent_role: s("agent_role")?,
                inputs_hash: s("inputs_hash")?,
                model: s("model")?,
                version: s("version")?,
                output: s("output")?,
                evidence_refs: o.get_arr("evidence_refs")?.to_vec(),
                timestamp: o.get_u64("timestamp")?,
            }),
            "edge" => Ok(MiniNode::Edge {
                edge_id: s("edge_id")?,
                from_node: s("from_node")?,
                to_node: s("to_node")?,
                artifact_hash: s("artifact_hash")?,
                artifact_type: s("artifact_type")?,
                timestamp: o.get_u64("timestamp")?,
            }),
            "evidence" => Ok(MiniNode::Evidence {
                evidence_id: s("evidence_id")?,
                tool: s("tool")?,
                query_hash: s("query_hash")?,
                retrieved_ref: s("retrieved_ref")?,
                content_hash: s("content_hash")?,
                consumed_by_node: s("consumed_by_node")?,
                timestamp: o.get_u64("timestamp")?,
            }),
            "human" => {
                let action_tag = match o.get_str("action")? {
                    "approve" => 1,
                    "override" => 2,
                    "escalate" => 3,
                    "comment" => 4,
                    other => return Err(format!("action humaine inconnue : « {other} »")),
                };
                Ok(MiniNode::Human {
                    node_id: s("node_id")?,
                    human_id: s("human_id")?,
                    action_tag,
                    target_node: s("target_node")?,
                    justification: s("justification")?,
                    timestamp: o.get_u64("timestamp")?,
                })
            }
            other => Err(format!("type de nœud inconnu (kind) : « {other} »")),
        }
    }

    /// L'**encodage canonique** — identique octet pour octet à `provenance-dag::node::canonical`.
    pub fn canonical(&self) -> Vec<u8> {
        let mut o = Vec::new();
        match self {
            MiniNode::AgentAct { node_id, agent_role, inputs_hash, model, version, output, evidence_refs, timestamp } => {
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
            MiniNode::Edge { edge_id, from_node, to_node, artifact_hash, artifact_type, timestamp } => {
                o.push(2);
                put_str(&mut o, edge_id);
                put_str(&mut o, from_node);
                put_str(&mut o, to_node);
                put_str(&mut o, artifact_hash);
                put_str(&mut o, artifact_type);
                put_u64(&mut o, *timestamp);
            }
            MiniNode::Evidence { evidence_id, tool, query_hash, retrieved_ref, content_hash, consumed_by_node, timestamp } => {
                o.push(3);
                put_str(&mut o, evidence_id);
                put_str(&mut o, tool);
                put_str(&mut o, query_hash);
                put_str(&mut o, retrieved_ref);
                put_str(&mut o, content_hash);
                put_str(&mut o, consumed_by_node);
                put_u64(&mut o, *timestamp);
            }
            MiniNode::Human { node_id, human_id, action_tag, target_node, justification, timestamp } => {
                o.push(4);
                put_str(&mut o, node_id);
                put_str(&mut o, human_id);
                o.push(*action_tag);
                put_str(&mut o, target_node);
                put_str(&mut o, justification);
                put_u64(&mut o, *timestamp);
            }
        }
        o
    }

    /// L'identité du nœud (id unique).
    pub fn id(&self) -> &str {
        match self {
            MiniNode::AgentAct { node_id, .. } => node_id,
            MiniNode::Edge { edge_id, .. } => edge_id,
            MiniNode::Evidence { evidence_id, .. } => evidence_id,
            MiniNode::Human { node_id, .. } => node_id,
        }
    }

    /// Les ids référencés (doivent préexister) — comme S0 : l'évidence est une **source pure** (rien).
    pub fn refs(&self) -> Vec<&str> {
        match self {
            MiniNode::AgentAct { evidence_refs, .. } => evidence_refs.iter().map(String::as_str).collect(),
            MiniNode::Evidence { .. } => Vec::new(),
            MiniNode::Edge { from_node, to_node, .. } => vec![from_node.as_str(), to_node.as_str()],
            MiniNode::Human { target_node, .. } => vec![target_node.as_str()],
        }
    }
}

impl MiniRecord {
    /// Parse une **ligne** JSONL en record.
    pub fn parse_line(line: &str) -> Result<MiniRecord, String> {
        let o = parse_object(line)?;
        let node = MiniNode::from_obj(&o)?;
        let seal = o.get_str("seal")?.to_string();
        Ok(MiniRecord { node, seal })
    }

    /// Parse un **JSONL** (lignes non-blanches), comme `SealedDag::records_from_jsonl`.
    pub fn parse_jsonl(s: &str) -> Result<Vec<MiniRecord>, String> {
        s.lines().filter(|l| !l.trim().is_empty()).map(MiniRecord::parse_line).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_node_kind_and_seal() {
        let act = MiniRecord::parse_line(r#"{"kind":"agent_act","node_id":"c::a","agent_role":"r","inputs_hash":"h","model":"m","version":"1","output":"PASS","evidence_refs":["c::e"],"timestamp":3,"seal":"ab"}"#).unwrap();
        assert_eq!(act.node.id(), "c::a");
        assert_eq!(act.node.refs(), vec!["c::e"]);
        assert_eq!(act.seal, "ab");

        let hum = MiniRecord::parse_line(r#"{"kind":"human","node_id":"c::r1","human_id":"o","action":"override","target_node":"c::a","justification":"x","timestamp":9,"seal":"cd"}"#).unwrap();
        match &hum.node {
            MiniNode::Human { action_tag, .. } => assert_eq!(*action_tag, 2, "override → tag 2"),
            _ => panic!(),
        }
        assert_eq!(hum.node.refs(), vec!["c::a"]);

        let ev = MiniRecord::parse_line(r#"{"kind":"evidence","evidence_id":"c::e","tool":"db","query_hash":"q","retrieved_ref":"r","content_hash":"cc","consumed_by_node":"c::a","timestamp":2,"seal":"ef"}"#).unwrap();
        assert_eq!(ev.node.id(), "c::e");
        assert!(ev.node.refs().is_empty(), "l'évidence est une source");
    }

    #[test]
    fn canonical_is_length_prefixed_and_type_tagged() {
        let act = MiniRecord::parse_line(r#"{"kind":"agent_act","node_id":"ab","agent_role":"r","inputs_hash":"h","model":"m","version":"1","output":"o","evidence_refs":[],"timestamp":1,"seal":"x"}"#).unwrap();
        let c = act.node.canonical();
        assert_eq!(c[0], 1, "octet de type AgentAct");
        // node_id « ab » : préfixe u32 BE = 0,0,0,2 puis « ab ».
        assert_eq!(&c[1..5], &[0, 0, 0, 2]);
        assert_eq!(&c[5..7], b"ab");
    }

    #[test]
    fn rejects_unknown_kind_and_action() {
        assert!(MiniRecord::parse_line(r#"{"kind":"mystère","seal":"x"}"#).is_err());
        assert!(MiniRecord::parse_line(r#"{"kind":"human","node_id":"a","human_id":"o","action":"DANSER","target_node":"b","justification":"j","timestamp":1,"seal":"x"}"#).is_err());
    }
}
