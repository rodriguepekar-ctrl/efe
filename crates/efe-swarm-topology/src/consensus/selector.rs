// SPDX-License-Identifier: UNLICENSED
//! Sélection ADAPTATIVE du pattern de consensus selon la tâche.
//!
//! Principe : ne pas payer les juges quand le vote suffit. Si les candidats
//! s'accordent fortement, le vote (gratuit) est consensuel → on l'utilise. Sinon
//! (dispersion), on agrège les ballots des juges (Borda/Condorcet). (Debate, plus
//! cher et itératif, reste GATÉ — cf. ADR-SWARM-003.)

use serde_json::Value;

use super::{Aggregator, BordaConsensus, CondorcetConsensus, MajorityVote};

/// Accord entre candidats = fraction de paires de candidats à valeur IDENTIQUE.
pub fn agreement(candidates: &[Value]) -> f64 {
    let k = candidates.len();
    if k < 2 {
        return 1.0;
    }
    let mut same = 0usize;
    let mut total = 0usize;
    for i in 0..k {
        for j in (i + 1)..k {
            total += 1;
            if candidates[i] == candidates[j] {
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
