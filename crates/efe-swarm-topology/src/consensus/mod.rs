// SPDX-License-Identifier: UNLICENSED
//! # `consensus` — Phase 3 incrément 1 : qualité émergente (math PURE, sans LLM).
//!
//! Quand on ne peut PAS vérifier déterministiquement (F3 de Lot S2), on agrège les
//! K candidats par un **pattern de consensus** qui exploite la vue JOINTE :
//! - [`MajorityVote`] : mode des valeurs (le PLANCHER S1) ;
//! - [`BordaConsensus`] : score de Borda sur les classements des juges ;
//! - [`CondorcetConsensus`] : gagnant de Condorcet + détection de cycle (fallback Copeland).
//!
//! Un [`Judge`] (Mode A déterministe ici ; Mode C plus tard) produit un score par
//! candidat → un [`Ballot`]. Les agrégateurs fusionnent candidats + ballots en un
//! [`AggOutcome`]. Tout est déterministe et **prouvé par oracles** (paradoxe de
//! Condorcet, exemples Borda, axiomes) — **0 appel modèle**.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

pub mod borda;
pub mod condorcet;
pub mod judges;
pub mod rank;
pub mod selector;
pub mod vote;

pub use borda::BordaConsensus;
pub use condorcet::CondorcetConsensus;
pub use judges::{
    ConsistencyJudge, GroundednessJudge, KeywordOverlapJudge, LengthBandJudge, VerifierJudge,
};
pub use selector::{aggregator_for, agreement, Pattern, PatternSelector};
pub use vote::MajorityVote;

/// Un « votant » : évalue l'ensemble des candidats → un score par candidat (≥ = meilleur).
/// Mode A (critère déterministe, ~gratuit) ou Mode C (juge LLM, incrément ultérieur).
#[async_trait]
pub trait Judge: Send + Sync {
    /// `scores[i]` = qualité du candidat i selon ce juge.
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64>;
    fn name(&self) -> &str;
}

/// L'avis d'UN juge : un score par candidat.
#[derive(Debug, Clone, PartialEq)]
pub struct Ballot {
    pub judge: String,
    pub scores: Vec<f64>,
}
impl Ballot {
    pub fn new(judge: impl Into<String>, scores: Vec<f64>) -> Self {
        Self { judge: judge.into(), scores }
    }
}

/// Collecte les ballots de tous les juges sur le MÊME ensemble de candidats.
pub async fn collect_ballots(
    task: &Value,
    candidates: &[Value],
    judges: &[Arc<dyn Judge>],
) -> Vec<Ballot> {
    let mut out = Vec::with_capacity(judges.len());
    for j in judges {
        out.push(Ballot::new(j.name(), j.assess(task, candidates).await));
    }
    out
}

/// Décision consensuelle + signaux (pour la calibration / sélection adaptative à venir).
#[derive(Debug, Clone)]
pub struct AggOutcome {
    pub selected: Value,
    pub selected_index: usize,
    /// Score consensuel par candidat (counts / Borda / Copeland selon la méthode).
    pub scores: Vec<f64>,
    /// Marge normalisée gagnant vs 2ᵉ ∈ [0,1] (faible = désaccord).
    pub confidence: f64,
    pub method: &'static str,
    /// Condorcet : paradoxe (top-cycle) rencontré ?
    pub cycle_detected: bool,
}

pub(crate) fn empty_outcome(method: &'static str) -> AggOutcome {
    AggOutcome {
        selected: Value::Null,
        selected_index: 0,
        scores: Vec::new(),
        confidence: 0.0,
        method,
        cycle_detected: false,
    }
}

/// Fusionne candidats (+ ballots des juges) en UNE décision consensuelle.
/// Object-safe → `Box<dyn Aggregator>` (comparaison uniforme des patterns).
pub trait Aggregator: Send + Sync {
    fn aggregate(&self, task: &Value, candidates: &[Value], ballots: &[Ballot]) -> AggOutcome;
    fn method(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct ConstJudge(&'static str, Vec<f64>);
    #[async_trait]
    impl Judge for ConstJudge {
        async fn assess(&self, _t: &Value, _c: &[Value]) -> Vec<f64> {
            self.1.clone()
        }
        fn name(&self) -> &str {
            self.0
        }
    }

    #[tokio::test]
    async fn collect_ballots_gathers_each_judge() {
        let judges: Vec<Arc<dyn Judge>> = vec![
            Arc::new(ConstJudge("a", vec![1.0, 0.0])),
            Arc::new(ConstJudge("b", vec![0.0, 1.0])),
        ];
        let ballots = collect_ballots(&json!({}), &[json!("x"), json!("y")], &judges).await;
        assert_eq!(ballots.len(), 2);
        assert_eq!(ballots[0].judge, "a");
        assert_eq!(ballots[0].scores, vec![1.0, 0.0]);
        assert_eq!(ballots[1].scores, vec![0.0, 1.0]);
    }

    #[test]
    fn aggregator_is_object_safe() {
        let aggs: Vec<Box<dyn Aggregator>> = vec![
            Box::new(MajorityVote),
            Box::new(BordaConsensus),
            Box::new(CondorcetConsensus),
        ];
        let names: Vec<&str> = aggs.iter().map(|a| a.method()).collect();
        assert_eq!(names, vec!["vote", "borda", "condorcet"]);
    }
}
