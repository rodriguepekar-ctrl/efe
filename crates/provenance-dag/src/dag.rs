// SPDX-License-Identifier: UNLICENSED
//! **Le DAG scellé re-prouvable** : une chaîne append-only de records `(nœud, sceau)`, à **manifest-root**,
//! que [`reprove_dag`] re-dérive et vérifie depuis les records bruts et la clé tenant.
//!
//! Chaque maillon : `sceau_i = HMAC(K_tenant, chaîne_{i-1} ‖ canonical(nœud_i))`, `chaîne_0 = SHA256(GENESIS)`.
//! Le **manifest-root** = `chaîne_n` (engage tout le DAG dans l'ordre). Toute altération d'un champ, ou
//! tout réordonnancement, casse la chaîne à partir du point touché — **détectablement**.
//!
//! **Sémantique (à ne jamais sur-vendre) :** la re-preuve atteste l'**INTÉGRITÉ** — « ce DAG est le
//! compte authentique, non-altéré, de ce qui s'est passé ». Elle n'atteste **ni** la **correction** de
//! la décision (autre couche : la mesure), **ni** une **reproductibilité bit-à-bit** (les LLM sont
//! non-déterministes). Cette distinction est imposée en doc ici, et au niveau API/UX en aval (P1 §8).

use crate::hmac::{ct_eq, hmac_sha256};
use crate::key::TenantKey;
use crate::node::Node;
use crate::sha256::sha256;
use serde::{Deserialize, Serialize};

/// Constante de séparation de domaine — l'ancre de la chaîne (versionnée).
pub const GENESIS: &[u8] = b"efe-prov-dag:v1:genesis";

/// Un **record** scellé : un nœud + le maillon de chaîne HMAC à sa position (hex).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(flatten)]
    pub node: Node,
    /// Le sceau = la valeur de chaîne HMAC à ce record (hex). Avec le DAG, c'est ce qui est exporté.
    pub seal: String,
}

/// Le **DAG scellé** — append-only, déterministe, re-prouvable.
#[derive(Debug, Clone)]
pub struct SealedDag {
    records: Vec<Record>,
    chain: [u8; 32], // valeur de chaîne courante (= dernier sceau) ; aussi le manifest-root
}

/// Erreur d'append/parse du DAG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DagError {
    /// Le nœud référence un id absent (ou postérieur) — viole la causalité du DAG.
    DanglingRef(String),
    /// Id de nœud déjà présent.
    DuplicateId(String),
    /// Échec de parsing JSONL.
    Parse(String),
}
impl std::fmt::Display for DagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DagError::DanglingRef(r) => write!(f, "référence pendante vers « {r} » (nœud absent ou postérieur)"),
            DagError::DuplicateId(i) => write!(f, "id de nœud dupliqué « {i} »"),
            DagError::Parse(e) => write!(f, "JSONL invalide : {e}"),
        }
    }
}
impl std::error::Error for DagError {}

impl SealedDag {
    /// Un DAG vide, chaîne ancrée au genesis.
    pub fn new() -> Self {
        SealedDag { records: Vec::new(), chain: sha256(GENESIS) }
    }

    /// **Scelle et ajoute** un nœud. Vérifie d'abord l'intégrité structurelle (références ⊆ ids
    /// antérieurs ; id unique), puis chaîne le sceau. Déterministe.
    pub fn append(&mut self, key: &TenantKey, node: Node) -> Result<&Record, DagError> {
        let ids: std::collections::HashSet<&str> = self.records.iter().map(|r| r.node.id()).collect();
        for r in node.refs() {
            if !ids.contains(r) {
                return Err(DagError::DanglingRef(r.to_string()));
            }
        }
        if ids.contains(node.id()) {
            return Err(DagError::DuplicateId(node.id().to_string()));
        }
        let mut buf = self.chain.to_vec();
        buf.extend_from_slice(&node.canonical());
        let seal = hmac_sha256(key.as_bytes(), &buf);
        self.chain = seal;
        self.records.push(Record { node, seal: to_hex(&seal) });
        Ok(self.records.last().expect("vient d'être poussé"))
    }

    /// Les records scellés (ce qui s'exporte dans le bundle forensic).
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Le **manifest-root** courant (hex) — l'engagement public sur tout le DAG.
    pub fn manifest_root(&self) -> String {
        to_hex(&self.chain)
    }

    /// Sérialise en **JSONL** (un record par ligne) — la forme de stockage/inspection humaine.
    pub fn to_jsonl(&self) -> String {
        self.records.iter().map(|r| serde_json::to_string(r).expect("Record sérialisable")).collect::<Vec<_>>().join("\n")
    }

    /// Recharge des records depuis du JSONL (pour la re-preuve d'un bundle).
    pub fn records_from_jsonl(s: &str) -> Result<Vec<Record>, DagError> {
        s.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Record>(l).map_err(|e| DagError::Parse(e.to_string())))
            .collect()
    }
}
impl Default for SealedDag {
    fn default() -> Self {
        Self::new()
    }
}

/// Le **verdict** d'une re-preuve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Verdict {
    /// Le DAG est-il authentique &amp; non-altéré ?
    pub ok: bool,
    pub n_records: usize,
    /// Le manifest-root re-dérivé (présent ssi `ok`).
    pub manifest_root: Option<String>,
    /// Index du premier record cassé (présent ssi `!ok`).
    pub broken_at: Option<usize>,
    /// Motif lisible de l'échec (présent ssi `!ok`).
    pub reason: Option<String>,
}
impl Verdict {
    fn ok(n: usize, root: String) -> Self {
        Verdict { ok: true, n_records: n, manifest_root: Some(root), broken_at: None, reason: None }
    }
    fn fail(at: usize, reason: impl Into<String>) -> Self {
        Verdict { ok: false, n_records: 0, manifest_root: None, broken_at: Some(at), reason: Some(reason.into()) }
    }
}

/// **LA RE-PREUVE** — le cœur trustless. Re-dérive la chaîne depuis le genesis et la clé, vérifie chaque
/// sceau (à temps constant) ET l'intégrité structurelle (références causales, ids uniques). Ne dépend
/// que des records bruts et de la clé — rien d'autre à qui faire confiance.
pub fn reprove_dag(records: &[Record], key: &TenantKey) -> Verdict {
    let mut chain = sha256(GENESIS);
    let mut ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (i, rec) in records.iter().enumerate() {
        // Intégrité STRUCTURELLE : les références doivent exister parmi les nœuds ANTÉRIEURS.
        for r in rec.node.refs() {
            if !ids.contains(r) {
                return Verdict::fail(i, format!("référence pendante vers « {r} » (nœud absent, postérieur, ou auto-référence)"));
            }
        }
        if !ids.insert(rec.node.id()) {
            return Verdict::fail(i, format!("id de nœud dupliqué « {} »", rec.node.id()));
        }
        // Intégrité du SCEAU : recalcul de la chaîne, comparaison à temps constant.
        let mut buf = chain.to_vec();
        buf.extend_from_slice(&rec.node.canonical());
        let expect = hmac_sha256(key.as_bytes(), &buf);
        let stored = match from_hex(&rec.seal) {
            Some(b) => b,
            None => return Verdict::fail(i, "sceau hex invalide"),
        };
        if !ct_eq(&expect, &stored) {
            return Verdict::fail(i, "sceau invalide : record altéré, réordonné, ou mauvaise clé tenant");
        }
        chain = expect;
    }
    Verdict::ok(records.len(), to_hex(&chain))
}

/// Re-preuve **contre un manifest-root publié** : en plus de l'intégrité, exige que le root re-dérivé
/// égale celui que le client a publié/conservé (le tiers vérifie « c'est bien CE DAG-là »).
pub fn verify_against_root(records: &[Record], key: &TenantKey, expected_root: &str) -> Verdict {
    let v = reprove_dag(records, key);
    if !v.ok {
        return v;
    }
    match &v.manifest_root {
        Some(root) if root == expected_root => v,
        other => Verdict::fail(
            records.len().saturating_sub(1),
            format!("manifest-root attendu « {expected_root} », obtenu « {} »", other.clone().unwrap_or_default()),
        ),
    }
}

// ── hex (zéro dépendance) ──

fn to_hex(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}
fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::HumanAction;

    fn key() -> TenantKey {
        TenantKey::derive(b"root", "acme-corp")
    }

    /// Un DAG réaliste, en forme de « conformité fournisseur » : extraction → évidence(politique)
    /// consommée par un contrôle sanctions → arête → agrégation → arête → approbation humaine.
    fn build() -> SealedDag {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct {
            node_id: "extract".into(), agent_role: "supplier_extract".into(), inputs_hash: "h-doc".into(),
            model: "phi4-mini".into(), version: "1".into(), output: "{supplier:SUP-2024}".into(),
            evidence_refs: vec![], timestamp: 1,
        }).unwrap();
        d.append(&k, Node::Evidence {
            evidence_id: "ev-policy".into(), tool: "policy_db".into(), query_hash: "h-q".into(),
            retrieved_ref: "policy://4.2".into(), content_hash: "h-clause".into(),
            consumed_by_node: "extract".into(), timestamp: 2,
        }).unwrap();
        d.append(&k, Node::AgentAct {
            node_id: "sanction".into(), agent_role: "sanction_check".into(), inputs_hash: "h-sup".into(),
            model: "phi4-mini".into(), version: "1".into(), output: "PASS".into(),
            evidence_refs: vec!["ev-policy".into()], timestamp: 3,
        }).unwrap();
        d.append(&k, Node::Edge {
            edge_id: "e1".into(), from_node: "extract".into(), to_node: "sanction".into(),
            artifact_hash: "h-art".into(), artifact_type: "supplier_record".into(), timestamp: 4,
        }).unwrap();
        d.append(&k, Node::AgentAct {
            node_id: "aggregate".into(), agent_role: "verdict_aggregate".into(), inputs_hash: "h-agg".into(),
            model: "phi4-mini".into(), version: "1".into(), output: "COMPLIANT".into(),
            evidence_refs: vec![], timestamp: 5,
        }).unwrap();
        d.append(&k, Node::Edge {
            edge_id: "e2".into(), from_node: "sanction".into(), to_node: "aggregate".into(),
            artifact_hash: "h-art2".into(), artifact_type: "check_verdict".into(), timestamp: 6,
        }).unwrap();
        d.append(&k, Node::Human {
            human_id: "officer:42".into(), action: HumanAction::Approve, target_node: "aggregate".into(),
            justification: "revue conforme".into(), timestamp: 7,
        }).unwrap();
        d
    }

    #[test]
    fn seal_and_reprove_roundtrip() {
        let d = build();
        let v = reprove_dag(d.records(), &key());
        assert!(v.ok, "DAG authentique → re-preuve OK : {v:?}");
        assert_eq!(v.n_records, 7);
        assert_eq!(v.manifest_root.as_deref(), Some(d.manifest_root().as_str()), "root re-dérivé = root du builder");
    }

    #[test]
    fn tampering_any_field_breaks_reprove() {
        // LE test de falsification (la DoD de S0) : altérer la sortie d'un record casse la re-preuve À CE POINT.
        let d = build();
        let mut recs = d.records().to_vec();
        if let Node::AgentAct { output, .. } = &mut recs[2].node {
            *output = "FAIL".into(); // l'auditeur verrait PASS transformé en FAIL → doit être détecté
        }
        let v = reprove_dag(&recs, &key());
        assert!(!v.ok, "tamper de la décision détecté");
        assert_eq!(v.broken_at, Some(2), "cassé exactement au record altéré");
    }

    #[test]
    fn tampering_evidence_content_hash_breaks_reprove() {
        // L'évidence figée : changer le content_hash (la politique « telle qu'au moment T ») est détecté.
        let d = build();
        let mut recs = d.records().to_vec();
        if let Node::Evidence { content_hash, .. } = &mut recs[1].node {
            *content_hash = "h-clause-modifiée".into();
        }
        assert_eq!(reprove_dag(&recs, &key()).broken_at, Some(1));
    }

    #[test]
    fn tampering_human_justification_breaks_reprove() {
        // L'oversight scellé : altérer la justification d'une approbation humaine est détecté.
        let d = build();
        let mut recs = d.records().to_vec();
        if let Node::Human { justification, .. } = &mut recs[6].node {
            *justification = "raison réécrite après coup".into();
        }
        assert_eq!(reprove_dag(&recs, &key()).broken_at, Some(6));
    }

    #[test]
    fn reordering_breaks_reprove() {
        let d = build();
        let mut recs = d.records().to_vec();
        recs.swap(4, 5); // réordonner deux records
        assert!(!reprove_dag(&recs, &key()).ok, "réordonnancement détecté");
    }

    #[test]
    fn wrong_key_breaks_reprove() {
        let d = build();
        let other = TenantKey::derive(b"root", "globex"); // mauvais tenant
        let v = reprove_dag(d.records(), &other);
        assert!(!v.ok && v.broken_at == Some(0), "mauvaise clé → échec dès le 1er record");
    }

    #[test]
    fn dangling_reference_is_rejected_at_append_and_reprove() {
        let k = key();
        let mut d = SealedDag::new();
        // Une arête vers des nœuds inexistants → refusée à l'append.
        let err = d.append(&k, Node::Edge {
            edge_id: "e".into(), from_node: "ghost".into(), to_node: "x".into(),
            artifact_hash: "h".into(), artifact_type: "t".into(), timestamp: 1,
        });
        assert!(matches!(err, Err(DagError::DanglingRef(_))), "append refuse la référence pendante");
        // Et si quelqu'un forge un record pendant à la main, la re-preuve le rejette.
        let forged = vec![Record {
            node: Node::Edge { edge_id: "e".into(), from_node: "ghost".into(), to_node: "x".into(), artifact_hash: "h".into(), artifact_type: "t".into(), timestamp: 1 },
            seal: "00".repeat(32),
        }];
        assert!(!reprove_dag(&forged, &k).ok);
    }

    #[test]
    fn duplicate_id_is_rejected() {
        let k = key();
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "n".into(), agent_role: "r".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "o".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        let err = d.append(&k, Node::AgentAct { node_id: "n".into(), agent_role: "r".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "o2".into(), evidence_refs: vec![], timestamp: 2 });
        assert!(matches!(err, Err(DagError::DuplicateId(_))));
    }

    #[test]
    fn determinism_same_root_bit_for_bit() {
        // Re-prouvabilité : même contenu + même clé → même manifest-root, à l'octet.
        assert_eq!(build().manifest_root(), build().manifest_root());
    }

    #[test]
    fn jsonl_roundtrip_then_reprove() {
        let d = build();
        let jsonl = d.to_jsonl();
        let recs = SealedDag::records_from_jsonl(&jsonl).unwrap();
        let v = reprove_dag(&recs, &key());
        assert!(v.ok, "stocké en JSONL → rechargé → re-prouvé");
        assert_eq!(v.manifest_root, Some(d.manifest_root()));
    }

    #[test]
    fn verify_against_published_root() {
        let d = build();
        let root = d.manifest_root();
        // Le bon root passe.
        assert!(verify_against_root(d.records(), &key(), &root).ok);
        // Un root différent échoue, même si le DAG est par ailleurs intègre.
        assert!(!verify_against_root(d.records(), &key(), &"ff".repeat(32)).ok);
    }
}
