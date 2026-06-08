// SPDX-License-Identifier: UNLICENSED
//! [`CondorcetConsensus`] — préférence pairwise majoritaire. Sélectionne le gagnant
//! de Condorcet (qui bat tous les autres en duel) s'il existe ; sinon (paradoxe /
//! top-cycle) bascule sur **Copeland** (bilan de victoires), méthode Condorcet-
//! consistante, et signale `cycle_detected`.

use std::cmp::Ordering;

use serde_json::Value;

use super::rank::EPS;
use super::{AggOutcome, Aggregator, Ballot, BordaConsensus, MajorityVote};

/// Consensus de Condorcet (+ fallback Copeland sur cycle).
pub struct CondorcetConsensus;

impl Aggregator for CondorcetConsensus {
    fn method(&self) -> &'static str {
        "condorcet"
    }

    fn aggregate(&self, task: &Value, candidates: &[Value], ballots: &[Ballot]) -> AggOutcome {
        let k = candidates.len();
        if k == 0 {
            return super::empty_outcome("condorcet");
        }
        let valid: Vec<&Ballot> = ballots.iter().filter(|b| b.scores.len() == k).collect();
        if valid.is_empty() {
            let v = MajorityVote.aggregate(task, candidates, ballots);
            return AggOutcome { method: "condorcet", ..v };
        }

        // matrice de préférences P[i][j] = # juges avec score_i > score_j.
        let mut p = vec![vec![0usize; k]; k];
        for b in &valid {
            for i in 0..k {
                for j in 0..k {
                    if i != j && b.scores[i] > b.scores[j] + EPS {
                        p[i][j] += 1;
                    }
                }
            }
        }

        // Copeland(i) = #victoires + 0,5·#égalités pairwise.
        let mut copeland = vec![0.0f64; k];
        for i in 0..k {
            for j in 0..k {
                if i == j {
                    continue;
                }
                match p[i][j].cmp(&p[j][i]) {
                    Ordering::Greater => copeland[i] += 1.0,
                    Ordering::Equal => copeland[i] += 0.5,
                    Ordering::Less => {}
                }
            }
        }

        // gagnant de Condorcet = bat TOUS les autres en duel.
        let condorcet_winner = (0..k).find(|&i| (0..k).filter(|&j| j != i).all(|j| p[i][j] > p[j][i]));

        // marge Copeland (confidence) — commune aux deux cas.
        let confidence = {
            let mut s = copeland.clone();
            s.sort_by(|a, b| b.partial_cmp(a).unwrap_or(Ordering::Equal));
            let m = if s.len() >= 2 { s[0] - s[1] } else { 0.0 };
            m / (k.max(2) - 1) as f64
        };

        match condorcet_winner {
            Some(i) => AggOutcome {
                selected: candidates[i].clone(),
                selected_index: i,
                scores: copeland,
                confidence,
                method: "condorcet",
                cycle_detected: false,
            },
            None => {
                // cycle → Copeland, tiebreak par Borda puis index.
                let borda = BordaConsensus.aggregate(task, candidates, ballots).scores;
                let sel = (0..k)
                    .max_by(|&a, &b| {
                        copeland[a]
                            .partial_cmp(&copeland[b])
                            .unwrap_or(Ordering::Equal)
                            .then(
                                borda
                                    .get(a)
                                    .unwrap_or(&0.0)
                                    .partial_cmp(borda.get(b).unwrap_or(&0.0))
                                    .unwrap_or(Ordering::Equal),
                            )
                            .then(b.cmp(&a))
                    })
                    .unwrap();
                AggOutcome {
                    selected: candidates[sel].clone(),
                    selected_index: sel,
                    scores: copeland,
                    confidence,
                    method: "copeland-fallback",
                    cycle_detected: true,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn abc() -> Vec<Value> {
        vec![json!("A"), json!("B"), json!("C")]
    }

    #[test]
    fn condorcet_winner_when_one_exists() {
        // tous préfèrent A en tête → A bat B et C en duel.
        let ballots = vec![
            Ballot::new("j1", vec![3.0, 2.0, 1.0]),
            Ballot::new("j2", vec![3.0, 1.0, 2.0]),
            Ballot::new("j3", vec![5.0, 2.0, 1.0]),
        ];
        let out = CondorcetConsensus.aggregate(&json!({}), &abc(), &ballots);
        assert_eq!(out.selected, json!("A"));
        assert_eq!(out.method, "condorcet");
        assert!(!out.cycle_detected);
    }

    #[test]
    fn condorcet_paradox_detected_and_resolved() {
        // cycle : A≻B≻C / B≻C≻A / C≻A≻B → aucun gagnant de Condorcet.
        let ballots = vec![
            Ballot::new("j1", vec![3.0, 2.0, 1.0]), // A≻B≻C
            Ballot::new("j2", vec![1.0, 3.0, 2.0]), // B≻C≻A
            Ballot::new("j3", vec![2.0, 1.0, 3.0]), // C≻A≻B
        ];
        let out = CondorcetConsensus.aggregate(&json!({}), &abc(), &ballots);
        assert!(out.cycle_detected, "paradoxe de Condorcet détecté");
        assert_eq!(out.method, "copeland-fallback");
        // Copeland tous à 1.0, Borda tous à 3 → tiebreak index → A (déterministe).
        assert_eq!(out.selected, json!("A"));
    }

    #[test]
    fn condorcet_differs_from_borda() {
        // 3 juges A≻B≻C, 2 juges B≻C≻A.
        // Condorcet : A bat B (3>2) et C (3>2) → gagnant A.
        // Borda : A=3·2=6, B=3·1+2·2=7, C=2·1=2 → gagnant B. Les deux DIFFÈRENT.
        let ballots = vec![
            Ballot::new("j1", vec![3.0, 2.0, 1.0]),
            Ballot::new("j2", vec![3.0, 2.0, 1.0]),
            Ballot::new("j3", vec![3.0, 2.0, 1.0]),
            Ballot::new("j4", vec![1.0, 3.0, 2.0]),
            Ballot::new("j5", vec![1.0, 3.0, 2.0]),
        ];
        let cond = CondorcetConsensus.aggregate(&json!({}), &abc(), &ballots);
        assert_eq!(cond.selected, json!("A"), "Condorcet choisit A");
        assert!(!cond.cycle_detected);
        let borda = BordaConsensus.aggregate(&json!({}), &abc(), &ballots);
        assert_eq!(borda.selected, json!("B"), "Borda choisit B (≠ Condorcet)");
    }

    #[test]
    fn copeland_consistent_with_condorcet_winner() {
        // quand un gagnant de Condorcet existe, son Copeland est maximal (K-1 victoires).
        let ballots = vec![
            Ballot::new("j1", vec![3.0, 2.0, 1.0]),
            Ballot::new("j2", vec![3.0, 1.0, 2.0]),
        ];
        let out = CondorcetConsensus.aggregate(&json!({}), &abc(), &ballots);
        assert_eq!(out.selected_index, 0);
        assert_eq!(out.scores[0], 2.0, "A a 2 victoires pairwise (K-1)");
    }

    #[test]
    fn empty_ballots_falls_back_to_vote() {
        let out = CondorcetConsensus.aggregate(&json!({}), &[json!("A"), json!("A"), json!("B")], &[]);
        assert_eq!(out.selected, json!("A"));
        assert_eq!(out.method, "condorcet");
    }
}
