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

    /// **VERDICT DE CORRECTION (ADR-019 v2)** : si ce juge enveloppe un **oracle de correction**
    /// (cf. [`crate::traits::Verifier::is_correctness_oracle`]), renvoie `Some(passes[i])` — le
    /// verdict ABSOLU pass/fail par candidat, par rapport à la vérité-terrain de la tâche. Sinon
    /// `None` : un juge heuristique (accord/forme) n'atteste PAS la justesse et ne gate donc jamais
    /// l'acceptation. Déterministe et synchrone (pas d'appel réseau) : le consensus l'évalue
    /// TOUJOURS, indépendamment de la machinerie de vote/escalade. Défaut `None`.
    fn correctness_verdict(&self, _task: &Value, _candidates: &[Value]) -> Option<Vec<bool>> {
        None
    }

    /// **ADR-019 v2** : ce juge est-il un ORACLE de correction DÉTERMINISTE (il enveloppe un
    /// [`crate::traits::Verifier`] avec `is_correctness_oracle() = true`) ? Détection sans candidats,
    /// pour que le **resolver** garde un tel juge DÉTERMINISTE même en Mode C (sinon un juge-LLM
    /// l'écraserait et le gate « vérification = acceptation » deviendrait inerte là où il importe le
    /// plus — petits modèles, accord-faux corrélé). Défaut `false`.
    fn is_correctness_oracle(&self) -> bool {
        false
    }
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

    /// **ADR-019 (read-back B, R-a)** : agrège puis pondère par la réputation des variantes,
    /// SOUS VETO (cf. [`apply_reputation_weights`]). Défaut **uniforme** pour TOUTE méthode — un
    /// post-traitement des scores + ré-argmax, jamais un refactor de la math d'agrégation. Poids
    /// vides/uniformes ⇒ strictement `aggregate` (byte-identique, I-REP2).
    fn aggregate_weighted(&self, task: &Value, candidates: &[Value], ballots: &[Ballot], weights: &[f64]) -> AggOutcome {
        let mut out = self.aggregate(task, candidates, ballots);
        apply_reputation_weights(&mut out, candidates, weights);
        out
    }
}

/// Confiance en deçà de laquelle la décision consensuelle est jugée **ambiguë** : c'est la SEULE
/// zone où la réputation a voix (§1 temps ③). Au-dessus, le gagnant est FRANC et immune.
const REPUTATION_VETO_FLOOR: f64 = 0.34;

/// **ADR-019 (read-back B, R-a)** : pondère une décision d'agrégation par la réputation des
/// variantes, **sous veto**. Multiplie chaque score consensuel par le poids de sa variante et ne
/// **ré-élit** le gagnant QUE si la décision d'origine était **peu confiante**
/// (`confidence < REPUTATION_VETO_FLOOR`) — un gagnant FRANC (forte marge) reste **immune** : le
/// signal du run garde son droit de veto. Poids **vides / uniformes / de longueur incohérente** ⇒
/// **NO-OP** → décision **byte-identique** (I-REP2). Pur, déterministe.
pub fn apply_reputation_weights(out: &mut AggOutcome, candidates: &[Value], weights: &[f64]) {
    use std::cmp::Ordering;
    let k = candidates.len();
    // Garde de neutralité : longueur incohérente, ou tous poids ≈ 1.0 → aucune influence.
    if k == 0 || weights.len() != k || out.scores.len() != k || weights.iter().all(|&w| (w - 1.0).abs() < 1e-9) {
        return;
    }
    let weighted: Vec<f64> = (0..k).map(|i| out.scores[i] * weights[i]).collect();
    // ré-argmax stable : score pondéré max, égalité → index le plus petit (comme les agrégateurs).
    let new_winner = (0..k)
        .max_by(|&a, &b| weighted[a].partial_cmp(&weighted[b]).unwrap_or(Ordering::Equal).then(b.cmp(&a)))
        .unwrap_or(out.selected_index);
    // VETO : n'adopter le re-choix que si l'agrégation d'origine était ambiguë.
    if out.confidence < REPUTATION_VETO_FLOOR && new_winner != out.selected_index {
        out.selected_index = new_winner;
        out.selected = candidates[new_winner].clone();
        // marge normalisée recomposée sur les scores pondérés (transparence).
        let mut s = weighted.clone();
        s.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
        let top = s.first().copied().unwrap_or(0.0);
        let second = s.get(1).copied().unwrap_or(0.0);
        out.confidence = if top > 0.0 { ((top - second) / top).clamp(0.0, 1.0) } else { 0.0 };
        out.scores = weighted;
    }
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

    #[test]
    fn reputation_weights_respect_veto_and_uniform_is_noop() {
        // read-back B (R-a) : la réputation ne re-décide que dans la zone AMBIGUË ; un gagnant
        // FRANC est immune ; poids uniformes ⇒ NO-OP (byte-identique, I-REP2).
        let cands = vec![json!("A"), json!("B")];
        let base = || AggOutcome {
            selected: json!("A"), selected_index: 0, scores: vec![3.0, 2.0],
            confidence: 0.5, method: "test", cycle_detected: false,
        };

        // ① poids uniformes → NO-OP.
        let mut o = base();
        apply_reputation_weights(&mut o, &cands, &[1.0, 1.0]);
        assert_eq!((o.selected_index, o.scores.clone()), (0, vec![3.0, 2.0]), "uniforme → byte-identique");

        // ② poids favorisant B mais décision FRANCHE (0.5 ≥ floor) → VETO : A reste gagnant.
        let mut o = base();
        apply_reputation_weights(&mut o, &cands, &[0.5, 1.5]); // pondéré [1.5, 3.0] → B... mais franc
        assert_eq!(o.selected_index, 0, "gagnant franc IMMUNE à la réputation (veto)");

        // ③ mêmes poids, décision AMBIGUË (0.1 < floor) → B ré-élu.
        let mut o = AggOutcome { confidence: 0.1, ..base() };
        apply_reputation_weights(&mut o, &cands, &[0.5, 1.5]);
        assert_eq!((o.selected_index, o.selected.clone()), (1, json!("B")), "ambiguë → la réputation re-décide");

        // ④ longueur de poids incohérente → NO-OP (sécurité).
        let mut o = base();
        apply_reputation_weights(&mut o, &cands, &[0.5]); // len != k
        assert_eq!(o.selected_index, 0, "poids incohérents → ignorés");
    }
}
