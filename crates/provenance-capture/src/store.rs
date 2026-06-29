// SPDX-License-Identifier: UNLICENSED
//! **Le store de capture** — un [`provenance_dag::SealedDag`] alimenté par les taps, persistant **local**
//! (souverain), re-prouvable, et **requêtable** par cas et par rôle.
//!
//! L'append impose l'intégrité structurelle de S0 (références causales, ids uniques) : un message ou une
//! évidence doivent référencer des nœuds **déjà** enregistrés → l'appelant enregistre dans l'**ordre
//! causal** du workflow. Le store ne fait PAS sortir les records (egress refusé) : la persistance est un
//! **chemin local** que le client détient.

use std::collections::BTreeSet;
use std::path::Path;

use provenance_dag::{reprove_dag, DagError, Node, Record, SealedDag, TenantKey, Verdict};

use crate::tap::{HumanReview, LlmCall, MessageEvent, ToolCall};

/// Le store : le DAG scellé + la clé tenant (que le client détient).
pub struct CaptureStore {
    dag: SealedDag,
    key: TenantKey,
}

impl CaptureStore {
    /// Un store neuf pour un tenant (sa clé).
    pub fn new(key: TenantKey) -> Self {
        CaptureStore { dag: SealedDag::new(), key }
    }

    /// **Recharge** un store depuis des records **déjà scellés** (p. ex. via [`Self::load_records_local`])
    /// — le chemin de **redémarrage** d'un log persisté. Ne re-scelle ni ne valide : [`Self::reprove`]
    /// tranchera (fail-loud si le fichier a été altéré au repos).
    pub fn from_records(key: TenantKey, records: Vec<Record>) -> Self {
        CaptureStore { dag: SealedDag::from_records(records), key }
    }

    // ── enregistrement via les taps (scelle au fil de l'eau) ──

    /// Scelle un **acte** d'agent. Valide d'abord les segments d'id (cas, étape, étapes d'évidence).
    pub fn record_llm(&mut self, call: &LlmCall) -> Result<&Record, CaptureError> {
        check_segment(call.case_id, "case_id")?;
        check_segment(call.step, "step")?;
        for ev in call.evidence_steps {
            check_segment(ev, "evidence_step")?;
        }
        self.dag.append(&self.key, call.to_node()).map_err(CaptureError::Dag)
    }
    /// Scelle un **message** (arête). Les nœuds `from`/`to` doivent déjà exister (ordre causal).
    pub fn record_message(&mut self, ev: &MessageEvent) -> Result<&Record, CaptureError> {
        check_segment(ev.case_id, "case_id")?;
        check_segment(ev.step, "step")?;
        check_segment(ev.from_step, "from_step")?;
        check_segment(ev.to_step, "to_step")?;
        self.dag.append(&self.key, ev.to_node()).map_err(CaptureError::Dag)
    }
    /// Scelle une **évidence** (source). L'étape consommatrice est annotée (non-causale).
    pub fn record_evidence(&mut self, ev: &ToolCall) -> Result<&Record, CaptureError> {
        check_segment(ev.case_id, "case_id")?;
        check_segment(ev.step, "step")?;
        check_segment(ev.consumed_by_step, "consumed_by_step")?;
        self.dag.append(&self.key, ev.to_node()).map_err(CaptureError::Dag)
    }
    /// Scelle un acte de **supervision humaine**. Le nœud visé doit déjà exister.
    pub fn record_human(&mut self, h: &HumanReview) -> Result<&Record, CaptureError> {
        check_segment(h.case_id, "case_id")?;
        check_segment(h.step, "step")?;
        check_segment(h.target_step, "target_step")?;
        self.dag.append(&self.key, h.to_node()).map_err(CaptureError::Dag)
    }

    // ── intégrité & accès ──

    /// Le manifest-root courant (l'engagement public sur tout le DAG).
    pub fn manifest_root(&self) -> String {
        self.dag.manifest_root()
    }
    /// Re-prouve l'intégrité de tout le store (atteste ce qui s'est passé, pas la correction).
    pub fn reprove(&self) -> Verdict {
        reprove_dag(self.dag.records(), &self.key)
    }
    /// Les records scellés (ce qui s'exporte dans le bundle forensic).
    pub fn records(&self) -> &[Record] {
        self.dag.records()
    }
    /// Sérialise en JSONL (forme de stockage/inspection).
    pub fn to_jsonl(&self) -> String {
        self.dag.to_jsonl()
    }

    // ── requête (l'index, par convention `{case}::{étape}` dans le node_id) ──

    /// Tous les records d'un **cas** : on découpe le node_id sur le PREMIER `::` et on compare le
    /// segment-cas à l'égalité (cohérent avec [`Self::cases`], et insensible aux préfixes — `SUP-A`
    /// ne capture pas `SUP-A2`). Comme `::` est interdit dans les segments, le découpage est non-ambigu.
    pub fn by_case(&self, case_id: &str) -> Vec<&Record> {
        self.records()
            .iter()
            .filter(|r| r.node.id().split_once("::").map(|(c, _)| c == case_id).unwrap_or(false))
            .collect()
    }
    /// La liste triée des **cas** distincts présents.
    pub fn cases(&self) -> Vec<String> {
        let mut s: BTreeSet<String> = BTreeSet::new();
        for r in self.records() {
            if let Some((case, _)) = r.node.id().split_once("::") {
                s.insert(case.to_string());
            }
        }
        s.into_iter().collect()
    }
    /// Tous les actes d'agent d'un **rôle** donné (ex. « sanction_check »).
    pub fn by_role(&self, role: &str) -> Vec<&Record> {
        self.records()
            .iter()
            .filter(|r| matches!(&r.node, Node::AgentAct { agent_role, .. } if agent_role == role))
            .collect()
    }

    // ── persistance LOCALE (souveraine — jamais d'egress) ──

    /// Écrit le store en JSONL sur un **chemin local** (le client détient ses records), de façon
    /// **atomique et durable** : crée le dossier parent au besoin, écrit dans un fichier temporaire
    /// voisin, le `fsync`, puis `rename` par-dessus la cible (remplacement atomique sur le même système
    /// de fichiers). Un crash en plein write ne laisse donc jamais un bundle précédent tronqué.
    pub fn save_local(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut data = self.to_jsonl();
        data.push('\n'); // JSONL terminé par une nouvelle ligne (convention).
        let mut tmp_os = path.as_os_str().to_owned();
        tmp_os.push(format!(".{}.tmp", std::process::id())); // voisin, propre au processus
        let tmp = std::path::PathBuf::from(tmp_os);
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(data.as_bytes())?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, path)
    }
    /// Recharge des records depuis un fichier JSONL local (pour la re-preuve hors-ligne).
    pub fn load_records_local(path: &Path) -> Result<Vec<Record>, LoadError> {
        let content = std::fs::read_to_string(path).map_err(LoadError::Io)?;
        SealedDag::records_from_jsonl(&content).map_err(LoadError::Parse)
    }
}

/// Valide un segment d'identifiant (cas, étape) : non vide et **sans** le séparateur réservé `::`.
/// Sans cette garde, deux paires (cas, étape) distinctes pourraient produire le même node_id, et
/// `cases()`/`by_case()` (qui découpent sur `::`) mal-attribueraient les records — fatal pour un
/// produit dont la valeur EST l'attribution correcte. La convention devient ainsi **imposée**, pas
/// seulement documentée.
fn check_segment(s: &str, field: &str) -> Result<(), CaptureError> {
    if s.is_empty() {
        return Err(CaptureError::Naming(format!("le champ « {field} » est vide")));
    }
    if s.contains("::") {
        return Err(CaptureError::Naming(format!(
            "le champ « {field} » contient le séparateur réservé « :: » : {s:?}"
        )));
    }
    Ok(())
}

/// Erreur de **capture** : nommage invalide (segment vide ou contenant `::`), ou erreur structurelle
/// du DAG scellé sous-jacent (référence pendante, id dupliqué).
#[derive(Debug)]
pub enum CaptureError {
    Naming(String),
    Dag(DagError),
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Naming(m) => write!(f, "nommage invalide : {m}"),
            CaptureError::Dag(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for CaptureError {}

/// Erreur de chargement local.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Parse(DagError),
}
impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "E/S locale : {e}"),
            LoadError::Parse(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for LoadError {}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_dag::HumanAction;

    fn key() -> TenantKey {
        TenantKey::derive(b"root", "acme-corp")
    }

    /// Enregistre un cas complet de conformité fournisseur, dans l'ordre causal, via les taps.
    fn record_case(store: &mut CaptureStore, case: &str) {
        store.record_llm(&LlmCall { case_id: case, step: "extract", agent_role: "supplier_extract", inputs: b"doc", model: "phi4-mini", version: "1", output: "{supplier}", evidence_steps: &[], timestamp: 1 }).unwrap();
        store.record_evidence(&ToolCall { case_id: case, step: "ev-policy", tool: "policy_db", query: b"q", retrieved_ref: "policy://4.2", content: b"clause", consumed_by_step: "extract", timestamp: 2 }).unwrap();
        store.record_llm(&LlmCall { case_id: case, step: "sanction", agent_role: "sanction_check", inputs: b"sup", model: "phi4-mini", version: "1", output: "PASS", evidence_steps: &["ev-policy"], timestamp: 3 }).unwrap();
        store.record_message(&MessageEvent { case_id: case, step: "e1", from_step: "extract", to_step: "sanction", artifact: b"rec", artifact_type: "supplier_record", timestamp: 4 }).unwrap();
        store.record_llm(&LlmCall { case_id: case, step: "aggregate", agent_role: "verdict_aggregate", inputs: b"agg", model: "phi4-mini", version: "1", output: "COMPLIANT", evidence_steps: &[], timestamp: 5 }).unwrap();
        store.record_human(&HumanReview { case_id: case, step: "review-1", human_id: "officer:42", action: HumanAction::Approve, target_step: "aggregate", justification: "revue conforme", timestamp: 6 }).unwrap();
    }

    #[test]
    fn capture_a_case_then_reprove() {
        let mut s = CaptureStore::new(key());
        record_case(&mut s, "SUP-2024");
        let v = s.reprove();
        assert!(v.ok, "le DAG capté est re-prouvable : {v:?}");
        assert_eq!(v.n_records, 6);
    }

    #[test]
    fn query_by_case_and_role_and_list() {
        let mut s = CaptureStore::new(key());
        record_case(&mut s, "SUP-A");
        record_case(&mut s, "SUP-B");
        assert_eq!(s.cases(), vec!["SUP-A".to_string(), "SUP-B".to_string()], "cas distincts triés");
        assert_eq!(s.by_case("SUP-A").len(), 6, "6 records pour SUP-A");
        assert!(s.by_case("SUP-A").iter().all(|r| r.node.id().starts_with("SUP-A::")), "filtré au bon cas");
        // Régression : le MÊME officier (officer:42) révise les deux cas SANS collisionner — la revue
        // est namespacée par cas, donc requêtable et unique (jadis : DuplicateId + omission de by_case).
        assert!(
            s.by_case("SUP-A").iter().any(|r| matches!(&r.node, Node::Human { human_id, .. } if human_id == "officer:42")),
            "la revue humaine est bien rattachée à son cas",
        );
        // un rôle apparaît une fois par cas.
        assert_eq!(s.by_role("sanction_check").len(), 2, "un check sanctions par cas");
        assert!(s.by_case("INCONNU").is_empty());
    }

    #[test]
    fn causal_order_is_enforced() {
        // Une arête vers un nœud pas encore enregistré est refusée (intégrité structurelle S0).
        let mut s = CaptureStore::new(key());
        let r = s.record_message(&MessageEvent { case_id: "c", step: "e", from_step: "ghost", to_step: "x", artifact: b"a", artifact_type: "t", timestamp: 1 });
        assert!(matches!(r, Err(CaptureError::Dag(DagError::DanglingRef(_)))), "ordre causal imposé");
    }

    #[test]
    fn reserved_separator_is_rejected() {
        // La convention `::` est IMPOSÉE, pas seulement documentée : un cas/étape qui la contient (ou
        // un segment vide) est refusé au tap — sinon collision de node_id et mal-attribution par cas.
        let mut s = CaptureStore::new(key());
        let bad_case = s.record_llm(&LlmCall { case_id: "a::b", step: "x", agent_role: "r", inputs: b"i", model: "m", version: "1", output: "o", evidence_steps: &[], timestamp: 1 });
        assert!(matches!(bad_case, Err(CaptureError::Naming(_))), "« :: » dans le cas refusé");
        let bad_step = s.record_llm(&LlmCall { case_id: "a", step: "b::evil", agent_role: "r", inputs: b"i", model: "m", version: "1", output: "o", evidence_steps: &[], timestamp: 1 });
        assert!(matches!(bad_step, Err(CaptureError::Naming(_))), "« :: » dans l'étape refusé");
        let empty = s.record_llm(&LlmCall { case_id: "", step: "x", agent_role: "r", inputs: b"i", model: "m", version: "1", output: "o", evidence_steps: &[], timestamp: 1 });
        assert!(matches!(empty, Err(CaptureError::Naming(_))), "segment vide refusé");
        // Rien n'a été scellé (toutes les tentatives ont échoué avant l'append).
        assert_eq!(s.records().len(), 0);
    }

    #[test]
    fn save_and_load_local_roundtrip_reproves() {
        let mut s = CaptureStore::new(key());
        record_case(&mut s, "SUP-2024");
        let root = s.manifest_root();
        // Chemin local unique (souverain).
        let path = std::env::temp_dir().join("prov-capture-roundtrip-SUP-2024.jsonl");
        s.save_local(&path).unwrap();
        let loaded = CaptureStore::load_records_local(&path).unwrap();
        let v = reprove_dag(&loaded, &key());
        assert!(v.ok, "stocké local → rechargé → re-prouvé");
        assert_eq!(v.manifest_root, Some(root), "même manifest-root après aller-retour disque");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reload_a_persisted_store_then_query_and_reprove() {
        // Redémarrage : capter → persister → recharger via from_records → re-prouver + requêter.
        let mut s = CaptureStore::new(key());
        record_case(&mut s, "SUP-2024");
        let root = s.manifest_root();
        let recs = SealedDag::records_from_jsonl(&s.to_jsonl()).unwrap();
        let reloaded = CaptureStore::from_records(key(), recs);
        assert_eq!(reloaded.manifest_root(), root, "même engagement après reload");
        assert!(reloaded.reprove().ok, "store rechargé re-prouvé");
        assert_eq!(reloaded.by_case("SUP-2024").len(), 6, "requêtable après reload");
    }

    #[test]
    fn in_memory_jsonl_roundtrip() {
        let mut s = CaptureStore::new(key());
        record_case(&mut s, "c1");
        let jsonl = s.to_jsonl();
        let recs = SealedDag::records_from_jsonl(&jsonl).unwrap();
        assert!(reprove_dag(&recs, &key()).ok);
    }
}
