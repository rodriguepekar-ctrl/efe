// SPDX-License-Identifier: UNLICENSED
//! **L'enveloppe d'intégrité** — « intègre ≠ correct » imposé comme **TYPE**, pas comme convention.
//!
//! Toute réponse de l'audit-api porte une [`IntegrityEnvelope`]. Le choix décisif : il n'existe **aucun
//! champ** où écrire « correct ». Le verdict est un verdict d'**intégrité** (record authentique &
//! non-altéré), jamais de **justesse** (autre couche : la mesure, P2). Un développeur qui voudrait, par
//! mégarde ou par pression, faire dire à l'API « décision correcte » n'a structurellement nulle part où
//! l'écrire. Et la phrase de sémantique évite **littéralement** le token « correct » (elle dit « justesse »)
//! pour que le test-grep d'honnêteté (cf. `lib.rs`) soit trivialement vert et signifiant.

use provenance_dag::Verdict;
use serde::Serialize;

/// La sémantique d'un verdict — une énumération à **une seule valeur**. Pas une chaîne libre qu'on
/// remplirait un jour de « décision validée » : le type **interdit** le glissement vers la justesse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum IntegritySemantics {
    /// Le seul niveau : record AUTHENTIQUE & NON-ALTÉRÉ de ce qui s'est passé.
    #[serde(rename = "INTEGRITY_ONLY")]
    IntegrityOnly,
}

/// La phrase humaine de la sémantique, pour la console. **Constante** (pas du texte libre), et elle
/// n'emploie PAS le token « correct » (elle dit « justesse ») — l'invariant-grep reste littéral.
pub const SEMANTICS_NOTE: &str =
    "atteste ce qui s'est passé (record authentique et non-altéré) — ni la justesse de la décision, ni une reproductibilité bit-à-bit";

/// Le verdict d'intégrité exposé : le [`Verdict`] de S0, augmenté de l'horodatage `as_of` de la
/// dernière re-preuve **FROIDE** (la fraîcheur est une donnée, pas une hypothèse — cf. [`crate::monitor`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrityVerdict {
    /// Le record est-il authentique & non-altéré ?
    pub ok: bool,
    /// Horodatage de la dernière re-preuve **froide** (autoritaire) qui étaye ce verdict.
    pub as_of: u64,
    pub n_records: usize,
    /// Rang du premier record cassé (présent ssi `!ok`).
    pub broken_at: Option<usize>,
    /// Motif lisible de l'échec (présent ssi `!ok`).
    pub reason: Option<String>,
}

/// L'enveloppe portée par **chaque** réponse de l'API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrityEnvelope {
    /// L'engagement public sur le DAG (présent ssi intègre).
    pub manifest_root: Option<String>,
    pub verdict: IntegrityVerdict,
    pub semantics: IntegritySemantics,
    pub semantics_note: &'static str,
}

impl IntegrityEnvelope {
    /// Construit l'enveloppe depuis un [`Verdict`] S0 et l'horodatage de la re-preuve froide qui l'étaye.
    pub fn from_verdict(v: &Verdict, as_of: u64) -> Self {
        IntegrityEnvelope {
            manifest_root: v.manifest_root.clone(),
            verdict: IntegrityVerdict {
                ok: v.ok,
                as_of,
                n_records: v.n_records,
                broken_at: v.broken_at,
                reason: v.reason.clone(),
            },
            semantics: IntegritySemantics::IntegrityOnly,
            semantics_note: SEMANTICS_NOTE,
        }
    }
    /// L'intégrité est-elle vérifiée ?
    pub fn ok(&self) -> bool {
        self.verdict.ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_verdict() -> Verdict {
        // Reconstruit un Verdict::ok via une re-preuve réelle d'un DAG minuscule (les constructeurs de
        // Verdict sont privés à S0 ; on passe par l'API publique).
        use provenance_dag::{reprove_dag, Node, SealedDag, TenantKey};
        let k = TenantKey::from_bytes([3u8; 32]);
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "c::a".into(), agent_role: "r".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        reprove_dag(d.records(), &k)
    }

    #[test]
    fn envelope_carries_integrity_only_and_no_correct_token() {
        let env = IntegrityEnvelope::from_verdict(&ok_verdict(), 1234);
        assert!(env.ok());
        assert_eq!(env.verdict.as_of, 1234);
        assert_eq!(env.semantics, IntegritySemantics::IntegrityOnly);
        // L'invariant central : la sortie sérialisée n'emploie JAMAIS le token « correct ».
        let json = serde_json::to_string(&env).unwrap().to_lowercase();
        assert!(json.contains("integrity_only"), "la sémantique est portée");
        assert!(!json.contains("correct"), "aucun token « correct » dans l'enveloppe : {json}");
    }

    #[test]
    fn broken_verdict_surfaces_broken_at() {
        use provenance_dag::{reprove_dag, Node, Record, SealedDag, TenantKey};
        let k = TenantKey::from_bytes([3u8; 32]);
        let mut d = SealedDag::new();
        d.append(&k, Node::AgentAct { node_id: "c::a".into(), agent_role: "r".into(), inputs_hash: "h".into(), model: "m".into(), version: "1".into(), output: "PASS".into(), evidence_refs: vec![], timestamp: 1 }).unwrap();
        // Altère le JSONL puis re-prouve → verdict cassé.
        let tampered = d.to_jsonl().replace("PASS", "FAIL");
        let recs: Vec<Record> = SealedDag::records_from_jsonl(&tampered).unwrap();
        let v = reprove_dag(&recs, &k);
        let env = IntegrityEnvelope::from_verdict(&v, 9);
        assert!(!env.ok());
        assert_eq!(env.verdict.broken_at, Some(0));
        assert!(env.manifest_root.is_none());
    }
}
