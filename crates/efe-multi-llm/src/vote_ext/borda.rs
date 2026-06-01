// SPDX-License-Identifier: UNLICENSED
//! Borda count — chaque LLM ranke les outputs ; score = somme inverse-rank.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{MultiLlmError, MultiLlmResult};

/// Une ballot pour Borda : un votant `voter` ranke les candidats.
///
/// `ranking[0]` = candidat préféré, `ranking[n-1]` = moins préféré.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BordaBallot {
    /// Identifiant du votant.
    pub voter: String,
    /// Liste ordonnée des candidats (du plus préféré au moins).
    pub ranking: Vec<String>,
}

/// Résultat du Borda count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BordaOutcome {
    /// Candidat gagnant.
    pub winner: String,
    /// Scores par candidat.
    pub scores: HashMap<String, u32>,
    /// Total des ballots.
    pub ballots_count: usize,
}

/// Algo Borda :
/// 1. Pour chaque ballot, le candidat en position `i` (0-based)
///    reçoit `n - i` points (où `n` est le nombre total de candidats).
/// 2. Somme par candidat.
/// 3. Le candidat avec le plus haut total gagne.
/// 4. Tie-break déterministe : ordre alphabétique du candidat (plus
///    petit lex gagne).
///
/// Erreurs :
/// - `MultiLlmError::Config` si pas de ballots.
/// - `MultiLlmError::Config` si ballots ont des rankings vides.
pub fn borda_tally(ballots: &[BordaBallot]) -> MultiLlmResult<BordaOutcome> {
    if ballots.is_empty() {
        return Err(MultiLlmError::Config(
            "borda requires at least 1 ballot".into(),
        ));
    }
    // Calcule l'ensemble des candidats (union sur tous les ballots).
    let mut all_candidates: std::collections::BTreeSet<String> = Default::default();
    for b in ballots {
        if b.ranking.is_empty() {
            return Err(MultiLlmError::Config(format!(
                "ballot from '{}' has empty ranking",
                b.voter
            )));
        }
        for c in &b.ranking {
            all_candidates.insert(c.clone());
        }
    }
    let n = all_candidates.len();
    let mut scores: HashMap<String, u32> = all_candidates
        .iter()
        .map(|c| (c.clone(), 0u32))
        .collect();
    for b in ballots {
        for (i, c) in b.ranking.iter().enumerate() {
            let points = n.saturating_sub(i) as u32;
            *scores.entry(c.clone()).or_insert(0) += points;
        }
    }
    // Trouve le winner : max score, tie-break alphabétique min.
    let winner = scores
        .iter()
        .max_by(|a, b| {
            a.1.cmp(b.1)
                .then_with(|| b.0.cmp(a.0)) // tie-break : plus petit lex gagne
        })
        .map(|(k, _)| k.clone())
        .ok_or_else(|| MultiLlmError::Config("no candidates".into()))?;
    Ok(BordaOutcome {
        winner,
        scores,
        ballots_count: ballots.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(voter: &str, ranking: &[&str]) -> BordaBallot {
        BordaBallot {
            voter: voter.into(),
            ranking: ranking.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn unanimous_winner() {
        let ballots = vec![
            b("v1", &["A", "B", "C"]),
            b("v2", &["A", "B", "C"]),
            b("v3", &["A", "B", "C"]),
        ];
        let out = borda_tally(&ballots).unwrap();
        assert_eq!(out.winner, "A");
        // A obtient 3*3=9, B obtient 3*2=6, C obtient 3*1=3.
        assert_eq!(out.scores["A"], 9);
        assert_eq!(out.scores["B"], 6);
        assert_eq!(out.scores["C"], 3);
    }

    #[test]
    fn split_decision_picks_best_aggregate() {
        // V1 préfère A, V2 préfère B, V3 préfère A.
        let ballots = vec![
            b("v1", &["A", "B", "C"]),
            b("v2", &["B", "A", "C"]),
            b("v3", &["A", "B", "C"]),
        ];
        let out = borda_tally(&ballots).unwrap();
        assert_eq!(out.winner, "A");
        // A : (3+2+3) = 8, B : (2+3+2) = 7, C : (1+1+1) = 3.
        assert_eq!(out.scores["A"], 8);
        assert_eq!(out.scores["B"], 7);
    }

    #[test]
    fn tie_break_alphabetical() {
        // A et B obtiennent le même score.
        let ballots = vec![b("v1", &["A", "B"]), b("v2", &["B", "A"])];
        let out = borda_tally(&ballots).unwrap();
        // A: 2+1=3, B: 1+2=3. Tie → "A" (lex min) gagne.
        assert_eq!(out.winner, "A");
    }

    #[test]
    fn empty_ballots_errors() {
        let r = borda_tally(&[]);
        assert!(matches!(r, Err(MultiLlmError::Config(_))));
    }

    #[test]
    fn empty_ranking_in_ballot_errors() {
        let bs = vec![BordaBallot {
            voter: "x".into(),
            ranking: vec![],
        }];
        assert!(matches!(borda_tally(&bs), Err(MultiLlmError::Config(_))));
    }

    #[test]
    fn partial_rankings_handled() {
        // V1 ranke seulement A et B (pas C).
        let ballots = vec![
            b("v1", &["A", "B"]),
            b("v2", &["A", "B", "C"]),
        ];
        let out = borda_tally(&ballots).unwrap();
        // 3 candidats au total (A,B,C).  n=3.
        // V1 : A=3, B=2 (C n'est pas rangé → 0).
        // V2 : A=3, B=2, C=1.
        // Total : A=6, B=4, C=1.
        assert_eq!(out.winner, "A");
        assert_eq!(out.scores["A"], 6);
        assert_eq!(out.scores["B"], 4);
        assert_eq!(out.scores["C"], 1);
    }

    #[test]
    fn single_ballot_winner() {
        let ballots = vec![b("only", &["X", "Y", "Z"])];
        let out = borda_tally(&ballots).unwrap();
        assert_eq!(out.winner, "X");
    }

    #[test]
    fn outcome_serializable() {
        let ballots = vec![b("v", &["A", "B"])];
        let out = borda_tally(&ballots).unwrap();
        let s = serde_json::to_string(&out).unwrap();
        let _back: BordaOutcome = serde_json::from_str(&s).unwrap();
    }
}
