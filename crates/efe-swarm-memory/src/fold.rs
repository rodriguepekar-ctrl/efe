// SPDX-License-Identifier: UNLICENSED
//! # `fold` — la projection d'un run en mémoire (la fonction de repli pure)
//!
//! Le repli est une **fonction pure** `fold(cellules, résumé_de_run) → cellules`. Elle
//! consomme un [`RunSummary`] — la **projection minimale** d'un run que la mémoire a besoin de
//! connaître (combien de candidats, quel pattern a gagné, quels exemplaires retenir, quels
//! agents ont performé). En MEM-1, `RunSummary` est un type nu (rempli par le SDK en MEM-2 à
//! partir d'un `SwarmRun`) ; ici, c'est l'**entrée testable** de la logique d'apprentissage.
//!
//! ## Deux propriétés cardinales
//! - **Idempotence par `run_id`** : un run ne replie qu'une fois. Garantie au niveau [`store`]
//!   (dédup), pas ici — [`materialize`] suppose des `run_id` uniques dans le journal.
//! - **Déterminisme par ordre canonique** : [`materialize`] replie le journal dans l'**ordre
//!   canonique** (par `run_id`, ADR-007 — jamais l'horloge), donc les cellules sont une
//!   fonction de l'**ensemble** des replis, indépendante de l'ordre d'arrivée. Les espèces
//!   associatives (histogrammes, compteurs) commutent ; l'espèce non-commutative (exemplaires,
//!   à cause de l'éviction) est rendue déterministe par cet ordre fixe.
//!
//! [`store`]: crate::store

use serde::{Deserialize, Serialize};

use crate::cells::{Exemplar, MemoryCells};
use crate::chain::{canonical_order, FoldRecord};

/// La performance d'un agent sur un run (pour la réputation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStat {
    pub agent_id: String,
    /// L'agent a produit un candidat **vérifié**.
    pub verified: bool,
    /// L'agent a produit le candidat **gagnant**.
    pub won: bool,
}

/// **La projection minimale d'un run que la mémoire consomme.** Rempli par le SDK (MEM-2) à
/// partir d'un `SwarmRun` ; ici, l'entrée pure de la logique d'apprentissage. **Entiers
/// uniquement** (aucun `f64`) pour la stabilité octet du leaf de chaîne.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSummary {
    /// **La porte (I-MEM3)** : le run est-il `Completed ∧ verified` (`SwarmRun::is_clean()`,
    /// ADR-018) ? Le [`store`](crate::store) ne replie QUE si `clean` — un run tronqué
    /// (budget/deadline) ou non vérifié ne touche jamais la mémoire.
    pub clean: bool,
    /// `k_used` du run (taille d'essaim effective) → prior de taille.
    pub k_used: u32,
    /// Le pattern d'agrégation gagnant (`"vote"`/`"borda"`/`"condorcet"`), si applicable.
    pub winning_pattern: Option<String>,
    /// Le run a-t-il escaladé (vote peu confiant → juges) ?
    pub escalated: bool,
    /// Les candidats **vérifiés/gagnants** à retenir comme exemplaires (few-shot).
    pub exemplars: Vec<Exemplar>,
    /// La performance par agent → réputation.
    pub agents: Vec<AgentStat>,
    /// Le run a-t-il échoué (aucun candidat vérifié) ? → mémoire d'échec.
    pub failed: bool,
    /// La clé d'échec à comptabiliser (famille ou agent). `None` ⇒ pas de comptage d'échec.
    pub failure_key: Option<String>,
}

impl RunSummary {
    /// Un résumé « propre » minimal (helper de test / construction).
    pub fn clean(k_used: u32) -> Self {
        Self {
            clean: true,
            k_used,
            winning_pattern: None,
            escalated: false,
            exemplars: Vec::new(),
            agents: Vec::new(),
            failed: false,
            failure_key: None,
        }
    }
}

/// **Replie UN résumé dans les cellules, en place.** Pure : aucune horloge, aucun aléa. Chaque
/// espèce est mise à jour indépendamment (les associatives par incrément, les exemplaires par
/// insertion + normalisation bornée). N'inspecte PAS `clean` — la porte est au niveau du store
/// (un `RunSummary` non-clean n'arrive jamais ici via le store).
pub fn fold_into(cells: &mut MemoryCells, s: &RunSummary) {
    // ① taille : un échantillon de plus dans l'histogramme de K.
    *cells.size.histogram.entry(s.k_used).or_insert(0) += 1;

    // ② pattern : un run de plus, le gagnant compté, l'escalade notée.
    if let Some(p) = &s.winning_pattern {
        *cells.pattern.wins.entry(p.clone()).or_insert(0) += 1;
        cells.pattern.total += 1;
        if s.escalated {
            cells.pattern.escalations += 1;
        }
    }

    // ③ exemplaires : insertion + ré-normalisation déterministe bornée (éviction quality-weighted).
    if !s.exemplars.is_empty() {
        cells.exemplars.items.extend(s.exemplars.iter().cloned());
        cells.exemplars.normalize();
    }

    // ④ réputation : compteurs par agent.
    for a in &s.agents {
        let rep = cells.reputation.agents.entry(a.agent_id.clone()).or_default();
        rep.total += 1;
        if a.verified {
            rep.verified += 1;
        }
        if a.won {
            rep.won += 1;
        }
    }

    // ⑤ échec : runs (toujours) + fails (si échec) sur la clé fournie.
    if let Some(key) = &s.failure_key {
        let rec = cells.failure.keys.entry(key.clone()).or_default();
        rec.runs += 1;
        if s.failed {
            rec.fails += 1;
        }
    }
}

/// **Matérialise les cellules depuis le journal** — la SEULE source de vérité des cellules.
/// Replie les résumés dans l'**ordre canonique** (par `run_id`, ADR-007), depuis des cellules
/// vides. Résultat = fonction déterministe de l'ENSEMBLE des replis, insensible à l'arrivée.
pub fn materialize(journal: &[FoldRecord]) -> MemoryCells {
    let mut cells = MemoryCells::default();
    for rec in canonical_order(journal) {
        fold_into(&mut cells, &rec.summary);
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::FoldRecord;
    use crate::family::{family_key, FamilyMode};
    use serde_json::json;
    use uuid::Uuid;

    fn fam() -> crate::family::FamilyKey {
        family_key(&json!({ "text": "x" }), "consensus", &["g".into()], FamilyMode::Structural)
    }

    fn rec(run: u128, s: RunSummary) -> FoldRecord {
        FoldRecord { run_id: Uuid::from_u128(run), family: fam(), summary: s }
    }

    #[test]
    fn fold_accumulates_each_species() {
        let mut c = MemoryCells::default();
        let s = RunSummary {
            clean: true,
            k_used: 6,
            winning_pattern: Some("borda".into()),
            escalated: true,
            exemplars: vec![Exemplar { content_hash: "aa".into(), preview: "p".into(), quality_milli: 900 }],
            agents: vec![
                AgentStat { agent_id: "g".into(), verified: true, won: true },
                AgentStat { agent_id: "j".into(), verified: false, won: false },
            ],
            failed: false,
            failure_key: Some("fam".into()),
        };
        fold_into(&mut c, &s);
        assert_eq!(c.size.histogram.get(&6), Some(&1));
        assert_eq!(c.pattern.wins.get("borda"), Some(&1));
        assert_eq!(c.pattern.escalations, 1);
        assert_eq!(c.exemplars.items.len(), 1);
        assert_eq!(c.reputation.of("g").unwrap().won, 1);
        assert_eq!(c.reputation.of("j").unwrap().total, 1);
        assert_eq!(c.failure.of("fam").unwrap().runs, 1);
        assert_eq!(c.failure.of("fam").unwrap().fails, 0, "run réussi → 0 fail mais 1 run");
    }

    #[test]
    fn materialize_is_order_insensitive() {
        // I-MEM (déterminisme) : le MÊME ensemble de replis, en ordres d'entrée différents,
        // matérialise les MÊMES cellules (ordre canonique par run_id).
        let j1 = vec![
            rec(1, { let mut s = RunSummary::clean(4); s.winning_pattern = Some("vote".into()); s }),
            rec(2, { let mut s = RunSummary::clean(6); s.winning_pattern = Some("borda".into()); s }),
            rec(3, RunSummary::clean(6)),
        ];
        let mut j2 = j1.clone();
        j2.reverse();
        let mut j3 = j1.clone();
        j3.swap(0, 2);
        let c1 = materialize(&j1);
        assert_eq!(c1, materialize(&j2), "ordre d'entrée inversé → mêmes cellules");
        assert_eq!(c1, materialize(&j3), "ordre d'entrée permuté → mêmes cellules");
        // contenu attendu : K {4:1, 6:2} ; patterns {vote:1, borda:1}.
        assert_eq!(c1.size.histogram.get(&6), Some(&2));
        assert_eq!(c1.size.histogram.get(&4), Some(&1));
    }

    #[test]
    fn materialize_exemplar_eviction_is_deterministic_under_reorder() {
        // L'espèce NON-commutative (exemplaires + éviction) reste déterministe via l'ordre
        // canonique : 3 runs apportent 4 exemplaires distincts ; cap 2 → mêmes 2 gardés
        // quel que soit l'ordre d'entrée.
        let mk = |h: &str, q: u32| Exemplar { content_hash: h.into(), preview: h.into(), quality_milli: q };
        let mut s1 = RunSummary::clean(4); s1.exemplars = vec![mk("aa", 100), mk("bb", 900)];
        let mut s2 = RunSummary::clean(4); s2.exemplars = vec![mk("cc", 500)];
        let mut s3 = RunSummary::clean(4); s3.exemplars = vec![mk("dd", 950)];
        let j = vec![rec(1, s1), rec(2, s2), rec(3, s3)];
        let mut cells = materialize(&j);
        cells.exemplars.cap = 2; // simulate cap; re-normalize to assert top-2
        cells.exemplars.normalize();
        let kept: Vec<&str> = cells.exemplars.items.iter().map(|e| e.content_hash.as_str()).collect();
        assert_eq!(kept, vec!["dd", "bb"], "top-2 par qualité (950 dd, 900 bb), déterministe");
        // ordre d'entrée permuté → mêmes gardés.
        let mut jr = j.clone(); jr.reverse();
        let mut cells_r = materialize(&jr);
        cells_r.exemplars.cap = 2; cells_r.exemplars.normalize();
        assert_eq!(cells, cells_r, "éviction insensible à l'ordre d'entrée");
    }
}
