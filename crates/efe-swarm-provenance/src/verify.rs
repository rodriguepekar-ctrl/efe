// SPDX-License-Identifier: UNLICENSED
//! # `verify` — le **vérificateur indépendant** `verify_swarm(records, key)` (le *moat*).
//!
//! > *Un tiers, avec seulement les enregistrements et la clé du tenant, re-prouve qui a
//! > dit quoi, quand, à partir de quoi — **sans coopération du provider**.*
//!
//! [`verify_swarm`] reçoit des enregistrements **dans un ordre quelconque** (un tiers ne
//! contrôle pas l'ordre de transmission) et vérifie **quatre invariants** :
//!
//! 1. **Chaîne intacte** (tamper-evidence) — tous les HMAC se recalculent. On **réordonne
//!    d'abord déterministiquement** (ADR-SWARM-007) pour retrouver l'ordre de scellement,
//!    puis on rejoue la chaîne ([`crate::chain::verify_chain`]). Muter/supprimer/réordonner
//!    une entrée casse un HMAC → détecté.
//! 2. **Contenu intact** — chaque `payload_hash` se recalcule depuis le contenu (commet
//!    `input_hash`, `output_hash`, `reads`, `writes`…). Substituer une sortie → détecté.
//! 3. **DAG cohérent** — pas de parent causal pendant, et pas de lecture d'une **version
//!    fantôme** (lire `pointer@v` alors que `pointer` n'a jamais été écrit à `v`). Contrôle
//!    **causal**, indépendant de la position en chaîne (qui n'est pas causale — ADR-007).
//! 4. **Couverture** (`traced_pct`) — fraction d'enregistrements ancrés à une cellule
//!    durable (proxy de « 100 % des appels tracés »).
//!
//! Sortie : [`SwarmProvenanceReport`] + le `provenance_root` (l'engagement unique du run).
//! **Aucune dépendance au provider** : seuls les records + la clé tenant (serveur) suffisent.

use std::collections::HashSet;

use uuid::Uuid;

use crate::chain::verify_chain;
use crate::record::{hex32, AgentCallRecord};
use crate::replay::deterministic_order;

/// Bilan de vérification indépendante d'un run d'essaim.
#[derive(Debug, Clone, PartialEq)]
pub struct SwarmProvenanceReport {
    /// Nombre d'enregistrements vérifiés.
    pub records: usize,
    /// Fraction d'enregistrements ancrés à une cellule durable (cible 100 %).
    pub traced_pct: f64,
    /// Tous les HMAC se recalculent (chaîne non altérée).
    pub chain_intact: bool,
    /// Tous les `payload_hash` se recalculent (contenu non altéré).
    pub content_intact: bool,
    /// DAG causal cohérent (parents existants, pas de lecture de version fantôme).
    pub dag_consistent: bool,
    /// Premier enregistrement en faute (forensique), par `record_id`.
    pub first_break: Option<Uuid>,
    /// `provenance_root` reconstruit (engagement du run), si la chaîne est intacte.
    pub root: Option<[u8; 32]>,
}

impl SwarmProvenanceReport {
    /// Le run est-il **intégralement re-prouvé** ? (chaîne + contenu + DAG).
    pub fn ok(&self) -> bool {
        self.chain_intact && self.content_intact && self.dag_consistent
    }
    pub fn root_hex(&self) -> Option<String> {
        self.root.as_ref().map(hex32)
    }
}

/// **Le vérificateur indépendant.** Reçoit des enregistrements (ordre quelconque) + la clé
/// tenant ; renvoie le bilan + le `provenance_root`. Ne consulte **aucun** provider.
pub fn verify_swarm(records: &[AgentCallRecord], key: &[u8]) -> SwarmProvenanceReport {
    if records.is_empty() {
        return SwarmProvenanceReport {
            records: 0,
            traced_pct: 1.0,
            chain_intact: true,
            content_intact: true,
            dag_consistent: true,
            first_break: None,
            root: None,
        };
    }

    // (0) ordre canonique déterministe (ADR-007) — robuste à l'ordre de transmission.
    let ordered = deterministic_order(records);

    // (2) contenu : chaque payload_hash se recalcule.
    let mut content_intact = true;
    let mut content_break: Option<Uuid> = None;
    for r in &ordered {
        if !r.verify_payload_hash() {
            content_intact = false;
            content_break = Some(r.record_id);
            break;
        }
    }

    // (1) chaîne : rejoue les HMAC dans l'ordre canonique.
    let verdict = verify_chain(&ordered, key);
    let chain_intact = verdict.ok;
    let chain_break = verdict.first_break.map(|b| ordered[b.index].record_id);

    // (3) DAG causal.
    let (dag_consistent, dag_break) = check_dag(&ordered);

    // (4) couverture.
    let anchored = ordered.iter().filter(|r| !r.writes.is_empty()).count();
    let traced_pct = anchored as f64 / ordered.len() as f64;

    // 1ʳᵉ faute, par priorité : contenu → chaîne → DAG.
    let first_break = content_break.or(chain_break).or(dag_break);

    SwarmProvenanceReport {
        records: ordered.len(),
        traced_pct,
        chain_intact,
        content_intact,
        dag_consistent,
        first_break,
        root: verdict.root,
    }
}

/// DAG causal : (a) tout `causal_parent` réfère un `record_id` présent ; (b) toute lecture
/// `pointer@v` dont le `pointer` est écrit par un record doit l'être **exactement à `v`**
/// (pas de version fantôme). Contrôle indépendant de l'ordre de chaîne.
fn check_dag(ordered: &[AgentCallRecord]) -> (bool, Option<Uuid>) {
    let ids: HashSet<Uuid> = ordered.iter().map(|r| r.record_id).collect();

    let mut written_cells: HashSet<(&str, u64)> = HashSet::new();
    let mut written_pointers: HashSet<&str> = HashSet::new();
    for r in ordered {
        for w in &r.writes {
            written_cells.insert((w.pointer.as_str(), w.version));
            written_pointers.insert(w.pointer.as_str());
        }
    }

    for r in ordered {
        // (a) parents causaux existants.
        for p in &r.causal_parents {
            if !ids.contains(p) {
                return (false, Some(r.record_id));
            }
        }
        // (b) lectures non fantômes.
        for c in &r.reads {
            if written_pointers.contains(c.pointer.as_str())
                && !written_cells.contains(&(c.pointer.as_str(), c.version))
            {
                return (false, Some(r.record_id));
            }
        }
    }
    (true, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{AgentKind, AgentRole, CellRef};
    use crate::recorder::{CallCapture, ProvenanceRecorder};
    use crate::HmacKeySource;
    use efe_blackboard::BlackboardWrite;
    use efe_nano_sdk::{AgentId, SwarmId, TenantId};
    use serde_json::json;
    use std::time::SystemTime;
    use uuid::Uuid;

    fn key() -> Vec<u8> {
        HmacKeySource::test_key().derive(TenantId::system()).unwrap()
    }
    fn swarm() -> SwarmId {
        SwarmId::from_uuid(Uuid::from_u128(0x2222))
    }
    fn wr(pointer: &str, version: u64, agent: &str) -> BlackboardWrite {
        BlackboardWrite {
            pointer: pointer.into(),
            version,
            agent_id: AgentId::new(agent).unwrap(),
            at: SystemTime::UNIX_EPOCH,
            bytes: 8,
        }
    }

    /// Run scellé via le recorder (ordre déterministe garanti) : 3 générateurs + 2 juges
    /// (les juges déclarent lire /cand/* + leur parent causal — DAG renseigné).
    fn sealed_run() -> Vec<AgentCallRecord> {
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        // 3 générateurs → /cand/0../cand/2
        for v in 0..3u64 {
            r.capture(CallCapture::new(
                AgentId::new("generator").unwrap(),
                AgentRole::Generator,
                AgentKind::ModeA,
                &json!({ "v": v }),
                &json!({ "value": v }),
            ));
            r.anchor(&wr(&format!("/cand/{v}"), 1, "generator"));
        }
        // 2 juges → /ballot/0../ballot/1, lisant /cand/0 (déclaré)
        for j in 0..2u64 {
            r.capture(
                CallCapture::new(
                    AgentId::new(format!("judge_{j:03}")).unwrap(),
                    AgentRole::Judge,
                    AgentKind::ModeA,
                    &json!({ "candidates": 3 }),
                    &json!({ "scores": [1, 0, 0] }),
                )
                .reading("/cand/0", 1),
            );
            r.anchor(&wr(&format!("/ballot/{j}"), 1, &format!("judge_{j:03}")));
        }
        r.seal_with_key(key()).records
    }

    #[test]
    fn honest_run_is_fully_reproven() {
        let recs = sealed_run();
        let rep = verify_swarm(&recs, &key());
        assert!(rep.ok(), "run sain re-prouvé : {rep:?}");
        assert!(rep.chain_intact && rep.content_intact && rep.dag_consistent);
        assert_eq!(rep.records, 5);
        assert!((rep.traced_pct - 1.0).abs() < 1e-9, "100 % ancrés");
        assert!(rep.root.is_some());
        assert!(rep.first_break.is_none());
    }

    #[test]
    fn verification_is_independent_of_transmission_order() {
        let recs = sealed_run();
        let mut shuffled = recs.clone();
        shuffled.reverse();
        shuffled.rotate_left(2);
        let a = verify_swarm(&recs, &key());
        let b = verify_swarm(&shuffled, &key());
        assert!(a.ok() && b.ok());
        assert_eq!(a.root, b.root, "même root quel que soit l'ordre reçu");
    }

    #[test]
    fn content_tampering_is_detected() {
        let mut recs = sealed_run();
        // substitue une sortie sur l'enregistrement d'index 1 (ordre quelconque).
        recs[1].output_hash = efe_nano_sdk::Blake3Hash::from_bytes([0xff; 32]);
        let rep = verify_swarm(&recs, &key());
        assert!(!rep.content_intact);
        assert!(!rep.ok());
        assert_eq!(rep.first_break, Some(recs[1].record_id));
    }

    #[test]
    fn wrong_key_breaks_the_chain() {
        let recs = sealed_run();
        let rep = verify_swarm(&recs, &[0u8; 32]);
        assert!(!rep.chain_intact, "mauvaise clé → HMAC ne concorde pas");
        assert!(!rep.ok());
    }

    #[test]
    fn dangling_causal_parent_breaks_dag() {
        // un parent causal SCELLÉ dans le contenu mais inexistant dans le set → DAG rompu
        // (le contenu, lui, reste intègre : on isole bien le DAG).
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        r.capture(
            CallCapture::new(AgentId::new("judge_000").unwrap(), AgentRole::Judge, AgentKind::ModeA, &json!({}), &json!({"scores":[1]}))
                .caused_by(Uuid::from_u128(0xDEAD)), // parent inexistant
        );
        r.anchor(&wr("/ballot/0", 1, "judge_000"));
        let recs = r.seal_with_key(key()).records;
        let rep = verify_swarm(&recs, &key());
        assert!(rep.content_intact, "le parent est scellé dans le contenu");
        assert!(!rep.dag_consistent, "parent causal pendant détecté");
        assert!(!rep.ok());
    }

    #[test]
    fn phantom_read_version_breaks_dag() {
        // run où un juge lit /cand/0@5 alors que /cand/0 n'existe qu'à v1.
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        r.capture(CallCapture::new(AgentId::new("generator").unwrap(), AgentRole::Generator, AgentKind::ModeA, &json!({}), &json!({"value":0})));
        r.anchor(&wr("/cand/0", 1, "generator"));
        r.capture(
            CallCapture::new(AgentId::new("judge_000").unwrap(), AgentRole::Judge, AgentKind::ModeA, &json!({}), &json!({"scores":[1]}))
                .reading("/cand/0", 5), // version fantôme
        );
        r.anchor(&wr("/ballot/0", 1, "judge_000"));
        let recs = r.seal_with_key(key()).records;
        let rep = verify_swarm(&recs, &key());
        assert!(!rep.dag_consistent, "lecture d'une version inexistante détectée");
        assert!(!rep.ok());
    }

    #[test]
    fn external_read_is_allowed() {
        // lire un pointeur JAMAIS écrit par l'essaim (entrée externe) ne casse pas le DAG.
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        r.capture(
            CallCapture::new(AgentId::new("generator").unwrap(), AgentRole::Generator, AgentKind::ModeA, &json!({}), &json!({"value":0}))
                .reading("/external/task", 1),
        );
        r.anchor(&wr("/cand/0", 1, "generator"));
        let recs = r.seal_with_key(key()).records;
        let rep = verify_swarm(&recs, &key());
        assert!(rep.dag_consistent, "lecture externe autorisée");
        assert!(rep.ok());
    }

    #[test]
    fn traced_pct_reflects_anchoring() {
        // un record capté SANS écriture ancrée → couverture < 100 %.
        let r = ProvenanceRecorder::new(swarm(), TenantId::system());
        r.capture(CallCapture::new(AgentId::new("generator").unwrap(), AgentRole::Generator, AgentKind::ModeA, &json!({}), &json!({"value":0})));
        r.anchor(&wr("/cand/0", 1, "generator"));
        r.capture(CallCapture::new(AgentId::new("merger").unwrap(), AgentRole::Merger, AgentKind::ModeA, &json!({}), &json!({"merged":true}))); // pas d'ancre
        let recs = r.seal_with_key(key()).records;
        let rep = verify_swarm(&recs, &key());
        assert!((rep.traced_pct - 0.5).abs() < 1e-9, "1/2 ancrés");
        assert!(rep.ok(), "couverture partielle n'est pas une rupture d'intégrité");
    }

    #[test]
    fn empty_run_verifies_vacuously() {
        let rep = verify_swarm(&[], &key());
        assert!(rep.ok());
        assert_eq!(rep.records, 0);
        assert_eq!(rep.root, None);
        assert_eq!(rep.traced_pct, 1.0);
    }

    #[test]
    fn report_exposes_root_hex() {
        let recs = sealed_run();
        let rep = verify_swarm(&recs, &key());
        assert_eq!(rep.root_hex().unwrap().len(), 64);
        // garde anti-warning : CellRef importé utilisé.
        let _ = CellRef::new("/x", 1);
    }
}
