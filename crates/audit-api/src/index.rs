// SPDX-License-Identifier: UNLICENSED
//! **L'index de requête** — un **cache reconstructible**, jamais un registre (cf. l'architecture §5).
//!
//! Les requêtes de la console (par période, par statut, file d'exceptions) supposent des index que S1
//! n'offre pas. S2 les bâtit — mais en **caches dérivés** : entièrement **reconstructibles** depuis les
//! records scellés seuls (invariant testé `I-S2-INDEX=CACHE`), donc **sans autorité propre**. Si l'index
//! et le record divergeaient, le **record gagne**. La pagination est un cas heureux : sur un log
//! append-only, la **position de chaîne** est un curseur stable et monotone (pas de page qui glisse).

use crate::project::{project, DecisionStatus, ProjectionConfig};
use provenance_dag::Record;
use std::collections::BTreeMap;

/// Le résumé indexé d'un cas (dérivé — ré-vérifiable en re-projetant le cas).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseEntry {
    pub case_id: String,
    pub status: DecisionStatus,
    pub n_records: usize,
    /// Position du **premier** record du cas dans la chaîne (le curseur stable de pagination).
    pub first_pos: usize,
    /// Horodatage du **dernier** nœud du cas (pour le filtre par période).
    pub last_ts: u64,
}

/// L'index des cas d'un tenant — reconstruit depuis les records.
#[derive(Debug, Default, Clone)]
pub struct CaseIndex {
    /// Trié par `case_id` ; on expose un ordre stable par `first_pos` via [`Self::ordered`].
    by_case: BTreeMap<String, CaseEntry>,
}

impl CaseIndex {
    /// **Reconstruit** l'index depuis les records scellés seuls. C'est l'unique constructeur : il n'y a
    /// pas de mutation incrémentale qui pourrait diverger — on rebâtit.
    pub fn rebuild(records: &[Record], config: &ProjectionConfig) -> Self {
        // Regroupe les records par cas, en gardant la position de chaîne (l'ordre est causal).
        let mut groups: BTreeMap<String, (usize, Vec<&Record>)> = BTreeMap::new();
        for (pos, r) in records.iter().enumerate() {
            // Un id conforme est « {cas}::{étape} ». Un id SANS « :: » (qui ne devrait pas exister — S1
            // namespace et valide tout) est routé sous son id COMPLET plutôt que **silencieusement
            // abandonné** : l'index ne doit jamais perdre un record (I-S2-INDEX=CACHE — un FAIL ne peut
            // pas devenir invisible).
            let case = r.node.id().split_once("::").map(|(c, _)| c).unwrap_or_else(|| r.node.id());
            let entry = groups.entry(case.to_string()).or_insert((pos, Vec::new()));
            entry.1.push(r);
        }
        let mut by_case = BTreeMap::new();
        for (case, (first_pos, recs)) in groups {
            let view = project(&recs, &case, config);
            let last_ts = recs.iter().map(|r| node_ts(&r.node)).max().unwrap_or(0);
            by_case.insert(
                case.clone(),
                CaseEntry { case_id: case, status: view.status, n_records: recs.len(), first_pos, last_ts },
            );
        }
        CaseIndex { by_case }
    }

    /// Tous les cas, **ordonnés par position de chaîne** (stable, monotone — l'ordre de pagination).
    pub fn ordered(&self) -> Vec<&CaseEntry> {
        let mut v: Vec<&CaseEntry> = self.by_case.values().collect();
        v.sort_by_key(|e| e.first_pos);
        v
    }

    /// Les cas dans un statut donné (file d'exceptions = `Exception`/`Fail`).
    pub fn by_status(&self, status: DecisionStatus) -> Vec<&CaseEntry> {
        self.ordered().into_iter().filter(|e| e.status == status).collect()
    }

    /// Les cas dont le **dernier** nœud tombe dans la fenêtre `[from, to]` (inclus). Sémantique assumée :
    /// on filtre par l'horodatage de **clôture** du cas (son dernier nœud) — un cas commencé avant `from`
    /// mais clos dans la fenêtre est inclus ; un cas clos après `to` est exclu. (Choix documenté, pas un
    /// filtre par chevauchement d'intervalle.)
    pub fn by_period(&self, from: u64, to: u64) -> Vec<&CaseEntry> {
        self.ordered().into_iter().filter(|e| e.last_ts >= from && e.last_ts <= to).collect()
    }

    /// Une page de cas : ceux dont `first_pos > after_pos`, au plus `limit`. Le curseur retourné est la
    /// `first_pos` du dernier élément servi (à recombiner avec le manifest-root par l'appelant, §5.1).
    pub fn page(&self, after_pos: Option<usize>, limit: usize) -> (Vec<&CaseEntry>, Option<usize>) {
        let limit = limit.max(1); // garde contre un underflow `limit - 1` si l'appelant passe 0
        let start = after_pos.map(|p| p + 1).unwrap_or(0);
        let mut page: Vec<&CaseEntry> = self.ordered().into_iter().filter(|e| e.first_pos >= start).collect();
        let next = if page.len() > limit { Some(page[limit - 1].first_pos) } else { None };
        page.truncate(limit);
        (page, next)
    }

    pub fn len(&self) -> usize {
        self.by_case.len()
    }
    pub fn is_empty(&self) -> bool {
        self.by_case.is_empty()
    }
}

/// L'horodatage d'un nœud (tous les variants en portent un).
fn node_ts(node: &provenance_dag::Node) -> u64 {
    use provenance_dag::Node::*;
    match node {
        AgentAct { timestamp, .. } | Edge { timestamp, .. } | Evidence { timestamp, .. } | Human { timestamp, .. } => *timestamp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use provenance_capture::{CaptureStore, LlmCall};
    use provenance_dag::TenantKey;

    fn store() -> CaptureStore {
        let mut s = CaptureStore::new(TenantKey::from_bytes([1u8; 32]));
        // SUP-A : compliant ; SUP-B : fail ; positions de chaîne croissantes.
        s.record_llm(&LlmCall { case_id: "SUP-A", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "m", version: "1", output: "PASS", evidence_steps: &[], timestamp: 10 }).unwrap();
        s.record_llm(&LlmCall { case_id: "SUP-B", step: "sanction", agent_role: "sanction_check", inputs: b"i", model: "m", version: "1", output: "FAIL", evidence_steps: &[], timestamp: 20 }).unwrap();
        s
    }

    #[test]
    fn rebuild_indexes_status_and_period() {
        let s = store();
        let idx = CaseIndex::rebuild(s.records(), &ProjectionConfig::default());
        assert_eq!(idx.len(), 2);
        assert_eq!(idx.by_status(DecisionStatus::Fail).len(), 1, "SUP-B est FAIL");
        assert_eq!(idx.by_status(DecisionStatus::Fail)[0].case_id, "SUP-B");
        assert_eq!(idx.by_period(0, 15).len(), 1, "seul SUP-A (ts=10) dans [0,15]");
        // ordre = position de chaîne.
        let ord = idx.ordered();
        assert_eq!(ord[0].case_id, "SUP-A");
        assert_eq!(ord[1].case_id, "SUP-B");
    }

    #[test]
    fn index_is_a_reconstructible_cache() {
        // I-S2-INDEX=CACHE : reconstruire depuis les records seuls rend l'IDENTIQUE.
        let s = store();
        let cfg = ProjectionConfig::default();
        let a = CaseIndex::rebuild(s.records(), &cfg);
        let b = CaseIndex::rebuild(s.records(), &cfg);
        assert_eq!(a.ordered(), b.ordered(), "la reconstruction est déterministe et sans autorité propre");
    }

    #[test]
    fn pagination_is_chain_stable() {
        let s = store();
        let idx = CaseIndex::rebuild(s.records(), &ProjectionConfig::default());
        let (p1, next) = idx.page(None, 1);
        assert_eq!(p1.len(), 1);
        assert_eq!(p1[0].case_id, "SUP-A");
        assert!(next.is_some(), "il reste une page");
        let (p2, next2) = idx.page(next, 1);
        assert_eq!(p2[0].case_id, "SUP-B");
        assert!(next2.is_none(), "fin");
    }
}
