// SPDX-License-Identifier: UNLICENSED
//! Sélection ADAPTATIVE du pattern de consensus selon la tâche.
//!
//! Principe : ne pas payer les juges quand le vote suffit. Si les candidats
//! s'accordent fortement, le vote (gratuit) est consensuel → on l'utilise. Sinon
//! (dispersion), on agrège les ballots des juges (Borda/Condorcet). (Debate, plus
//! cher et itératif, reste GATÉ — cf. ADR-SWARM-003.)

use serde_json::Value;

use super::{Aggregator, BordaConsensus, CondorcetConsensus, MajorityVote};
use crate::topology::answer_is_present;

/// Accord entre candidats = fraction de paires de candidats **présents ET identiques**.
///
/// **B0.2 (I-SENT)** : un candidat ABSENT — un appel LLM échoué que la couche transport replie en
/// sentinel vide (`""`), ou une réponse `null`/vide légitime — n'AGRÉE **jamais**, même apparié à un
/// autre absent. Sans cette règle, l'égalité exacte lit plusieurs vides comme une unanimité (`1.0`) :
/// sur GPU chargé (OOM, timeouts), une fausse-unanimité d'échecs arrêterait le sizer au plancher
/// (« économie » mensongère), court-circuiterait les juges (Vote), et ferait converger l'itératif.
/// Deux silences ne sont pas un consensus. Le dénominateur reste **toutes** les paires : les absents
/// **tirent l'accord vers le bas** (un échec partiel n'est pas une convergence), ils ne sont pas
/// ignorés. Le chemin sain (accord réel sur une valeur présente) est byte-inchangé.
pub fn agreement(candidates: &[Value]) -> f64 {
    let k = candidates.len();
    if k < 2 {
        // **B0.2 (I-SENT) — le veto de présence vaut AUSSI au cas dégénéré.** Un candidat unique
        // ABSENT (un sentinel d'échec à k=1) ne doit PAS renvoyer 1.0 : ce serait la même fausse
        // unanimité, juste décalée à k=1 (le sizer s'arrêterait « target » au plancher sur un essaim
        // brisé — chemin réel, car rien n'impose k_min ≥ 2 pour StopRule::Agreement). Singleton
        // PRÉSENT → 1.0 (byte-inchangé) ; singleton/aucun candidat ABSENT → 0.0.
        return candidates.first().map_or(1.0, |c| answer_is_present(c) as u8 as f64);
    }
    let mut same = 0usize;
    let mut total = 0usize;
    for i in 0..k {
        for j in (i + 1)..k {
            total += 1;
            if answer_is_present(&candidates[i]) && answer_is_present(&candidates[j]) && candidates[i] == candidates[j] {
                same += 1;
            }
        }
    }
    same as f64 / total as f64
}

/// Pattern de consensus choisi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    Vote,
    Borda,
    Condorcet,
}

/// Sélecteur adaptatif.
#[derive(Debug, Clone)]
pub struct PatternSelector {
    /// Accord ≥ ce seuil → Vote (gratuit, candidats concordants).
    pub vote_threshold: f64,
    /// En cas de dispersion : Condorcet (préférence pairwise) si true, sinon Borda.
    pub use_condorcet: bool,
}

impl Default for PatternSelector {
    fn default() -> Self {
        // **Quasi-unanimité requise** pour le Vote : un accord PARTIEL peut être un
        // CLUSTER DE LEURRES (piège S1) → on n'économise les juges qu'en cas d'accord
        // quasi-total (sûr) ; sinon Borda/Condorcet tranchent via les juges.
        Self { vote_threshold: 0.9, use_condorcet: false }
    }
}

impl PatternSelector {
    pub fn new(vote_threshold: f64, use_condorcet: bool) -> Self {
        Self { vote_threshold, use_condorcet }
    }

    /// Choisit le pattern selon l'accord des candidats.
    pub fn select(&self, candidates: &[Value]) -> Pattern {
        if agreement(candidates) >= self.vote_threshold {
            Pattern::Vote
        } else if self.use_condorcet {
            Pattern::Condorcet
        } else {
            Pattern::Borda
        }
    }
}

/// Construit l'agrégateur correspondant à un pattern.
pub fn aggregator_for(p: Pattern) -> Box<dyn Aggregator> {
    match p {
        Pattern::Vote => Box::new(MajorityVote),
        Pattern::Borda => Box::new(BordaConsensus),
        Pattern::Condorcet => Box::new(CondorcetConsensus),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn agreement_measures_identical_pairs() {
        assert_eq!(agreement(&[json!("A"), json!("A"), json!("A")]), 1.0);
        assert_eq!(agreement(&[json!("A"), json!("B"), json!("C")]), 0.0);
        // [A,A,B] : paires (0,1)=same, (0,2)=diff, (1,2)=diff → 1/3.
        assert!((agreement(&[json!("A"), json!("A"), json!("B")]) - 1.0 / 3.0).abs() < 1e-9);
        assert_eq!(agreement(&[json!("X")]), 1.0); // < 2 candidats
    }

    #[test]
    fn b0_2_absent_candidates_never_agree() {
        // I-SENT : un sentinel d'échec (vide) ne compte JAMAIS comme accord — même tous identiques.
        // C'est le défaut « GPU saturé » : plusieurs appels échoués → "" identiques → fausse unanimité.
        assert_eq!(agreement(&[json!(""), json!(""), json!("")]), 0.0, "tous vides → 0, PAS 1.0 (anti fausse-économie)");
        assert_eq!(agreement(&[json!({ "value": "" }), json!({ "value": "" })]), 0.0, "vide enveloppé {{value:''}} non plus");
        assert_eq!(agreement(&[json!(null), json!(null)]), 0.0, "null = absent");
        // Mix : 3 réels « A » présents + 1 vide (k=4) → paires identiques = (A,A)×3 sur 6 = 0.5.
        // Un échec partiel TIRE l'accord vers le bas (conservateur) — pas une convergence.
        let mixed = [json!("A"), json!("A"), json!("A"), json!("")];
        assert!((agreement(&mixed) - 0.5).abs() < 1e-9, "3 réels agréants + 1 vide → 0.5, pas 1.0 : got {}", agreement(&mixed));
        // Chemin sain INCHANGÉ : un accord réel sur une valeur présente reste 1.0.
        assert_eq!(agreement(&[json!("A"), json!("A"), json!("A")]), 1.0);
        // Cas dégénéré k=1 (le trou que la revue a trouvé) : un SINGLETON absent ne renvoie pas 1.0,
        // sinon arrêt « target » au plancher k_min=1 sur un essaim brisé. Singleton présent → 1.0.
        assert_eq!(agreement(&[json!("")]), 0.0, "singleton vide → 0 (ferme le trou k<2)");
        assert_eq!(agreement(&[json!(null)]), 0.0, "singleton null → 0");
        assert_eq!(agreement(&[json!("X")]), 1.0, "singleton PRÉSENT → 1.0 (chemin sain inchangé)");
    }

    #[test]
    fn high_agreement_picks_vote_low_picks_borda() {
        let s = PatternSelector::default(); // seuil 0.6
        assert_eq!(s.select(&[json!("A"), json!("A"), json!("A")]), Pattern::Vote);
        assert_eq!(s.select(&[json!("A"), json!("B"), json!("C")]), Pattern::Borda);
    }

    #[test]
    fn condorcet_option_on_dispersion() {
        let s = PatternSelector::new(0.6, true);
        assert_eq!(s.select(&[json!("A"), json!("B"), json!("C")]), Pattern::Condorcet);
    }

    #[test]
    fn aggregator_for_maps_patterns() {
        assert_eq!(aggregator_for(Pattern::Vote).method(), "vote");
        assert_eq!(aggregator_for(Pattern::Borda).method(), "borda");
        assert_eq!(aggregator_for(Pattern::Condorcet).method(), "condorcet");
    }
}
