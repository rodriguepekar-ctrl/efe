// SPDX-License-Identifier: UNLICENSED
//! # `replay` — rejeu **déterministe**, clé sur la **version du Blackboard** (ADR-SWARM-007).
//!
//! ## Le problème
//! Les agents s'exécutent **en parallèle** (JoinSet). L'ordre d'achèvement — et donc
//! `SystemTime::now()` (`wall_at`) — est **non déterministe**. Si l'ordre de la chaîne
//! HMAC dépendait de l'horloge murale, le `provenance_root` ne serait **pas reproductible**
//! → pas de rejeu, pas de vérification tierce.
//!
//! ## La décision (ADR-SWARM-007)
//! On **cle l'ordre sur les coordonnées DÉTERMINISTES du substrat** : la cellule écrite
//! `(version, pointer)` (la `version` est atomique et par-cellule ; le `pointer` la
//! complète — l'ordre `(version, pointer)` reflète la convention I13 du Blackboard, dont
//! le `snapshot()` est lui-même « sorted by pointer for replay determinism »). En
//! départage final : le `payload_hash` (adresse de contenu). `wall_at` est **advisory** et
//! **exclu du `payload_hash`** (déjà garanti par [`crate::record`]).
//!
//! ## Conséquence
//! - Le même **ensemble** d'enregistrements → le **même** `provenance_root`, quel que soit
//!   l'ordre de capture/exécution. ([`replay`] / [`replay_root`].)
//! - Le rejeu d'**exécution** ([`replay_outputs`]) re-dérive les sorties d'agents PURS
//!   (fonction de `(input, variant)`) et compare les `output_hash` — déterministe pour Mode A ;
//!   Mode C exige une graine épinglée (`sampling.seed`).
//!
//! Aucune horloge logique n'est injectée dans le substrat : les coordonnées existantes
//! suffisent (choix anti-invasif).

use std::cmp::Ordering;

use serde_json::Value;

use crate::chain::SwarmChain;
use crate::hash::blake3_value;
use crate::record::{hex32, AgentCallRecord, CellRef};

// ─────────────────────────────────────────────────────────────────────────────
//  Clé d'ordre déterministe (ADR-007)
// ─────────────────────────────────────────────────────────────────────────────

/// Clé d'ordre d'un enregistrement : `(a-une-écriture, version, pointer, payload_hash)`.
/// Les enregistrements **avec** écriture passent avant ceux sans ; puis ordre par
/// `(version, pointer)` de la cellule représentante ; départage par `payload_hash`.
/// **Ne dépend JAMAIS de `wall_at`.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrderKey {
    no_write: bool,
    version: u64,
    pointer: String,
    payload_hash: [u8; 32],
}

impl Ord for OrderKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.no_write
            .cmp(&other.no_write) // false (a une écriture) avant true
            .then(self.version.cmp(&other.version))
            .then(self.pointer.cmp(&other.pointer))
            .then(self.payload_hash.cmp(&other.payload_hash))
    }
}
impl PartialOrd for OrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Construit la clé d'ordre depuis les cellules écrites + le `payload_hash`. La cellule
/// représentante = la `(version, pointer)` minimale parmi les écritures.
pub(crate) fn key_of(writes: &[CellRef], payload_hash: [u8; 32]) -> OrderKey {
    match writes.iter().map(|c| (c.version, c.pointer.clone())).min() {
        Some((version, pointer)) => OrderKey { no_write: false, version, pointer, payload_hash },
        None => OrderKey { no_write: true, version: 0, pointer: String::new(), payload_hash },
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Ordre déterministe + rejeu de chaîne
// ─────────────────────────────────────────────────────────────────────────────

/// Renvoie les enregistrements dans l'**ordre déterministe** (ADR-007). Pure fonction
/// de l'ENSEMBLE : insensible à l'ordre d'entrée (donc à l'ordre d'exécution parallèle).
pub fn deterministic_order(records: &[AgentCallRecord]) -> Vec<AgentCallRecord> {
    let mut v = records.to_vec();
    v.sort_by(|a, b| key_of(&a.writes, a.payload_hash).cmp(&key_of(&b.writes, b.payload_hash)));
    v
}

/// Re-scelle les enregistrements dans l'ordre déterministe avec `key` → `provenance_root`.
/// Le même ensemble d'enregistrements donne **toujours** la même racine.
pub fn replay_root(records: &[AgentCallRecord], key: &[u8]) -> Option<[u8; 32]> {
    if records.is_empty() {
        return None;
    }
    let ordered = deterministic_order(records);
    let mut chain = SwarmChain::new(ordered[0].tenant_id, ordered[0].swarm_id, key.to_vec());
    for r in &ordered {
        chain.append(r.to_draft());
    }
    chain.root()
}

/// Rapport de rejeu de chaîne.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    /// Nombre d'enregistrements rejoués.
    pub replayed: usize,
    /// Tous les `payload_hash` se recalculent (intégrité de contenu).
    pub content_intact: bool,
    /// Index (dans l'ordre déterministe) du 1er enregistrement au contenu altéré.
    pub first_content_break: Option<usize>,
    /// `provenance_root` reconstruit déterministiquement.
    pub root: Option<[u8; 32]>,
}

impl ReplayReport {
    pub fn root_hex(&self) -> Option<String> {
        self.root.as_ref().map(hex32)
    }
    /// Le root reconstruit correspond-il au root attendu (de la chaîne d'origine) ?
    pub fn matches(&self, expected_root: Option<[u8; 32]>) -> bool {
        self.root == expected_root
    }
}

/// **Rejeu déterministe** : ordonne (ADR-007), vérifie l'intégrité de contenu de chaque
/// enregistrement, re-scelle → `provenance_root`. Le résultat ne dépend QUE de l'ensemble
/// d'enregistrements + la clé tenant (et jamais de `wall_at`).
pub fn replay(records: &[AgentCallRecord], key: &[u8]) -> ReplayReport {
    let ordered = deterministic_order(records);

    let mut first_content_break = None;
    for (i, r) in ordered.iter().enumerate() {
        if !r.verify_payload_hash() {
            first_content_break = Some(i);
            break;
        }
    }

    let root = if ordered.is_empty() {
        None
    } else {
        let mut chain = SwarmChain::new(ordered[0].tenant_id, ordered[0].swarm_id, key.to_vec());
        for r in &ordered {
            chain.append(r.to_draft());
        }
        chain.root()
    };

    ReplayReport {
        replayed: ordered.len(),
        content_intact: first_content_break.is_none(),
        first_content_break,
        root,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Rejeu d'EXÉCUTION (re-dérive les sorties d'agents purs)
// ─────────────────────────────────────────────────────────────────────────────

/// Rapport de rejeu d'exécution (comparaison des `output_hash`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputReplayReport {
    pub checked: usize,
    pub matched: usize,
    pub mismatched: usize,
    /// Index du 1er enregistrement dont la sortie rejouée diffère.
    pub first_mismatch: Option<usize>,
}

impl OutputReplayReport {
    /// Rejeu d'exécution parfaitement déterministe (toutes les sorties reproduites).
    pub fn deterministic(&self) -> bool {
        self.mismatched == 0
    }
}

/// Rejoue les sorties : pour chaque enregistrement, `reproduce` re-produit la **valeur**
/// de sortie (ex. re-exécuter l'agent pur sur l'entrée reconstruite) ; on la re-hashe
/// (BLAKE3 canonique) et on compare à `output_hash`. **Déterministe** pour les agents
/// Mode A (fonction pure de l'entrée) ; Mode C requiert une graine épinglée.
pub fn replay_outputs<F>(records: &[AgentCallRecord], reproduce: F) -> OutputReplayReport
where
    F: Fn(&AgentCallRecord) -> Value,
{
    let mut matched = 0;
    let mut mismatched = 0;
    let mut first_mismatch = None;
    for (i, r) in records.iter().enumerate() {
        if blake3_value(&reproduce(r)) == r.output_hash {
            matched += 1;
        } else {
            mismatched += 1;
            if first_mismatch.is_none() {
                first_mismatch = Some(i);
            }
        }
    }
    OutputReplayReport { checked: records.len(), matched, mismatched, first_mismatch }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::{AgentCallDraft, AgentKind, AgentRole};
    use crate::{HmacKeySource, SwarmChain};
    use efe_nano_sdk::{AgentId, Blake3Hash, SwarmId, TenantId};
    use serde_json::json;
    use std::time::{Duration, UNIX_EPOCH};
    use uuid::Uuid;

    fn key() -> Vec<u8> {
        HmacKeySource::test_key().derive(TenantId::system()).unwrap()
    }
    fn swarm() -> SwarmId {
        SwarmId::from_uuid(Uuid::from_u128(0x2222))
    }

    /// Construit un jeu d'enregistrements scellés : K générateurs (/cand/i) + 2 juges.
    fn build_run() -> Vec<AgentCallRecord> {
        let mut chain = SwarmChain::new(TenantId::system(), swarm(), key());
        let mut recs = Vec::new();
        for i in 0..4u64 {
            let d = AgentCallDraft::new(
                swarm(),
                TenantId::system(),
                AgentId::new("generator").unwrap(),
                AgentRole::Generator,
                AgentKind::ModeA,
                Blake3Hash::from_bytes([i as u8; 32]),
                Blake3Hash::from_bytes([(i + 10) as u8; 32]),
            )
            .with_record_id(Uuid::from_u128(0x100 + i as u128))
            .with_variant(i)
            .wrote(format!("/cand/{i}"), 1);
            recs.push(chain.append(d));
        }
        recs
    }

    #[test]
    fn deterministic_order_is_input_order_independent() {
        let recs = build_run();
        let mut reversed = recs.clone();
        reversed.reverse();
        let a = deterministic_order(&recs);
        let b = deterministic_order(&reversed);
        assert_eq!(a, b, "ordre déterministe insensible à l'ordre d'entrée");
    }

    #[test]
    fn replay_root_is_reproducible_regardless_of_order() {
        let recs = build_run();
        let mut shuffled = recs.clone();
        shuffled.rotate_left(2);
        shuffled.reverse();
        let r1 = replay_root(&recs, &key());
        let r2 = replay_root(&shuffled, &key());
        assert_eq!(r1, r2, "même ensemble → même provenance_root");
        assert!(r1.is_some());
    }

    #[test]
    fn replay_root_ignores_wall_at_advisory() {
        // ADR-007 : deux jeux ne différant QUE par wall_at → même root.
        let recs = build_run();
        let mut shifted = recs.clone();
        for (i, r) in shifted.iter_mut().enumerate() {
            r.wall_at = UNIX_EPOCH + Duration::from_secs(1000 + i as u64);
        }
        assert_eq!(replay_root(&recs, &key()), replay_root(&shifted, &key()), "wall_at advisory");
    }

    #[test]
    fn replay_reports_content_intact_on_honest_run() {
        let recs = build_run();
        let rep = replay(&recs, &key());
        assert!(rep.content_intact);
        assert_eq!(rep.replayed, 4);
        assert!(rep.first_content_break.is_none());
        assert!(rep.root.is_some());
    }

    #[test]
    fn replay_detects_content_tampering() {
        let mut recs = build_run();
        recs[1].output_hash = Blake3Hash::from_bytes([0xFF; 32]); // sortie substituée
        let rep = replay(&recs, &key());
        assert!(!rep.content_intact);
        assert!(rep.first_content_break.is_some());
    }

    #[test]
    fn replay_matches_live_recorder_root() {
        use crate::{ProvenanceRecorder, CallCapture};
        // un run via le recorder (ordre de capture quelconque) ↔ rejeu depuis records.
        let rec = ProvenanceRecorder::new(swarm(), TenantId::system());
        rec.anchor(&efe_blackboard::BlackboardWrite {
            pointer: "/cand/1".into(), version: 1,
            agent_id: AgentId::new("generator").unwrap(), at: UNIX_EPOCH, bytes: 5,
        });
        rec.anchor(&efe_blackboard::BlackboardWrite {
            pointer: "/cand/0".into(), version: 1,
            agent_id: AgentId::new("generator").unwrap(), at: UNIX_EPOCH, bytes: 5,
        });
        rec.capture(CallCapture::new(AgentId::new("generator").unwrap(), AgentRole::Generator, AgentKind::ModeA, &json!({"a":1}), &json!({"o":1})).with_variant(0));
        rec.capture(CallCapture::new(AgentId::new("generator").unwrap(), AgentRole::Generator, AgentKind::ModeA, &json!({"a":2}), &json!({"o":2})).with_variant(1));
        let sealed = rec.seal_with_key(key());
        // rejeu depuis les records → MÊME root que le recorder (ordre déterministe partagé).
        let rep = replay(&sealed.records, &key());
        assert_eq!(rep.root, sealed.root, "rejeu reproduit le root vivant");
    }

    #[test]
    fn replay_outputs_matches_for_pure_reproduction() {
        let recs = build_run();
        // reproduit EXACTEMENT la sortie enregistrée (générateur pur : variant i → [i+10;32]).
        // ici on simule en renvoyant la valeur dont le hash == output_hash : impossible sans
        // l'inverse du hash ; on teste donc via une repro qui reconstruit la même VALEUR.
        // build_run a hashé Blake3Hash directement (pas une valeur), donc on teste le chemin
        // mismatch + un cas match dédié ci-dessous.
        let rep = replay_outputs(&recs, |_| json!({ "x": 1 }));
        assert_eq!(rep.checked, 4);
        assert!(rep.mismatched > 0, "repro arbitraire → mismatch attendu");
    }

    #[test]
    fn replay_outputs_is_deterministic_when_reproduction_is_exact() {
        // Jeu où output_hash = BLAKE3 d'une VALEUR connue → repro exacte → match.
        let mut chain = SwarmChain::new(TenantId::system(), swarm(), key());
        let value = json!({ "value": "tech" });
        let d = AgentCallDraft::from_io(
            swarm(), TenantId::system(), AgentId::new("generator").unwrap(),
            AgentRole::Generator, AgentKind::ModeA, &json!({ "task": 1 }), &value,
        )
        .wrote("/cand/0", 1);
        let recs = vec![chain.append(d)];
        let rep = replay_outputs(&recs, |_| value.clone());
        assert!(rep.deterministic(), "repro exacte → toutes sorties reproduites");
        assert_eq!(rep.matched, 1);
    }

    #[test]
    fn empty_replay_has_no_root() {
        let rep = replay(&[], &key());
        assert!(rep.content_intact);
        assert_eq!(rep.root, None);
        assert_eq!(rep.replayed, 0);
    }
}
