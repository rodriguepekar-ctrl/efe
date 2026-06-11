// SPDX-License-Identifier: UNLICENSED
//! [`BordaConsensus`] — score de Borda sur les classements des juges. Exploite la
//! position ordinale COMPLÈTE (pas juste le top-1 du vote) → peut battre le vote
//! quand les juges ajoutent un signal indépendant.

use serde_json::Value;

use super::rank;
use super::{AggOutcome, Aggregator, Ballot, MajorityVote};

/// Consensus de Borda : points = Σ_juges ((K−1) − rang) ; gagnant = argmax.
pub struct BordaConsensus;

impl Aggregator for BordaConsensus {
    fn method(&self) -> &'static str {
        "borda"
    }

    fn aggregate(&self, task: &Value, candidates: &[Value], ballots: &[Ballot]) -> AggOutcome {
        let k = candidates.len();
        if k == 0 {
            return super::empty_outcome("borda");
        }
        // ballots valides (longueur == K).
        let valid: Vec<&Ballot> = ballots.iter().filter(|b| b.scores.len() == k).collect();
        if valid.is_empty() {
            // sans juge, Borda n'a pas de sens → repli sur le vote (méthode relabelée).
            let v = MajorityVote.aggregate(task, candidates, ballots);
            return AggOutcome { method: "borda", ..v };
        }

        let mut borda = vec![0.0f64; k];
        let mut firsts = vec![0usize; k];
        for b in &valid {
            let ranks = rank::scores_to_ranks(&b.scores);
            let minr = ranks.iter().cloned().fold(f64::INFINITY, f64::min);
            for i in 0..k {
                borda[i] += (k as f64 - 1.0) - ranks[i];
                if (ranks[i] - minr).abs() < rank::EPS {
                    firsts[i] += 1; // 1ʳᵉ place (ou ex-æquo en tête)
                }
            }
        }

        // gagnant = argmax Borda ; tiebreak = # premières places, puis plus petit index.
        let selected_index = (0..k)
            .max_by(|&a, &b| {
                borda[a]
                    .partial_cmp(&borda[b])
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(firsts[a].cmp(&firsts[b]))
                    .then(b.cmp(&a)) // index plus petit = préféré
            })
            .expect("0..k non vide : k>0 garanti par l'early-return plus haut");

        let max_gain = (valid.len() * (k.max(2) - 1)) as f64;
        let confidence = rank::margin(&borda) / max_gain;

        AggOutcome {
            selected: candidates[selected_index].clone(),
            selected_index,
            scores: borda,
            confidence,
            method: "borda",
            cycle_detected: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cands() -> Vec<Value> {
        vec![json!("A"), json!("B"), json!("C")]
    }

    #[test]
    fn textbook_borda_winner() {
        // j1: A≻B≻C [3,2,1] ; j2: A≻C≻B [3,1,2].
        // Borda: A=2+2=4, B=1+0=1, C=0+1=1 → A.
        let ballots = vec![
            Ballot::new("j1", vec![3.0, 2.0, 1.0]),
            Ballot::new("j2", vec![3.0, 1.0, 2.0]),
        ];
        let out = BordaConsensus.aggregate(&json!({}), &cands(), &ballots);
        assert_eq!(out.selected, json!("A"));
        assert_eq!(out.selected_index, 0);
        assert_eq!(out.scores, vec![4.0, 1.0, 1.0]);
    }

    #[test]
    fn unanimous_first_place_wins() {
        // tous classent B 1er.
        let ballots = vec![
            Ballot::new("j1", vec![1.0, 3.0, 2.0]),
            Ballot::new("j2", vec![2.0, 3.0, 1.0]),
            Ballot::new("j3", vec![0.0, 5.0, 1.0]),
        ];
        let out = BordaConsensus.aggregate(&json!({}), &cands(), &ballots);
        assert_eq!(out.selected, json!("B"));
    }

    #[test]
    fn neutral_judge_does_not_change_order() {
        // un juge à scores tous égaux donne (K-1)/2 à chacun → ne change pas le classement.
        let mut ballots = vec![
            Ballot::new("j1", vec![3.0, 2.0, 1.0]),
            Ballot::new("j2", vec![3.0, 1.0, 2.0]),
        ];
        let a = BordaConsensus.aggregate(&json!({}), &cands(), &ballots);
        ballots.push(Ballot::new("neutral", vec![1.0, 1.0, 1.0]));
        let b = BordaConsensus.aggregate(&json!({}), &cands(), &ballots);
        assert_eq!(a.selected, b.selected, "juge neutre = neutralité");
    }

    #[test]
    fn empty_ballots_falls_back_to_vote() {
        // sans juge : repli vote (mode des valeurs), méthode relabelée "borda".
        let out = BordaConsensus.aggregate(&json!({}), &[json!("A"), json!("A"), json!("B")], &[]);
        assert_eq!(out.selected, json!("A"));
        assert_eq!(out.method, "borda");
    }

    #[test]
    fn single_candidate_trivial() {
        let out = BordaConsensus.aggregate(&json!({}), &[json!("X")], &[Ballot::new("j", vec![1.0])]);
        assert_eq!(out.selected, json!("X"));
    }
}
