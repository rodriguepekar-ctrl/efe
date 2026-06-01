// SPDX-License-Identifier: UNLICENSED
//! Pairwise Condorcet — élimination face-à-face.

use std::collections::{BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::error::{MultiLlmError, MultiLlmResult};

use super::borda::BordaBallot;

/// Résultat Condorcet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CondorcetOutcome {
    /// Gagnant Condorcet — bat tous les autres en face-à-face.
    /// `None` si paradoxe Condorcet (cycle) → fallback Borda.
    pub winner: Option<String>,
    /// Matrice des victoires : `wins["A"]["B"] = nombre de votants
    /// qui préfèrent A à B`.
    pub wins: HashMap<String, HashMap<String, u32>>,
    /// Total ballots.
    pub ballots_count: usize,
    /// Si winner is None : on indique pourquoi.
    pub paradox_detected: bool,
}

/// Algo Condorcet :
/// 1. Pour chaque paire (A, B), compte combien de votants préfèrent A à B.
/// 2. Le gagnant Condorcet est le candidat qui bat tous les autres
///    (strict majority dans chaque face-à-face).
/// 3. Si aucun candidat ne satisfait (paradoxe Condorcet — cycle), on
///    retourne `winner: None` avec `paradox_detected: true`.
pub fn condorcet_tally(ballots: &[BordaBallot]) -> MultiLlmResult<CondorcetOutcome> {
    if ballots.is_empty() {
        return Err(MultiLlmError::Config(
            "condorcet requires at least 1 ballot".into(),
        ));
    }
    // Candidats.
    let mut candidates: BTreeSet<String> = Default::default();
    for b in ballots {
        for c in &b.ranking {
            candidates.insert(c.clone());
        }
    }
    let cands: Vec<String> = candidates.into_iter().collect();
    if cands.is_empty() {
        return Err(MultiLlmError::Config("no candidates".into()));
    }

    // Matrice wins[A][B] = nb de votants qui préfèrent A à B.
    let mut wins: HashMap<String, HashMap<String, u32>> = HashMap::new();
    for a in &cands {
        let inner: HashMap<String, u32> = cands
            .iter()
            .filter(|b| *b != a)
            .map(|b| (b.clone(), 0u32))
            .collect();
        wins.insert(a.clone(), inner);
    }
    for ballot in ballots {
        let rank: HashMap<&str, usize> = ballot
            .ranking
            .iter()
            .enumerate()
            .map(|(i, c)| (c.as_str(), i))
            .collect();
        for a in &cands {
            for b in &cands {
                if a == b {
                    continue;
                }
                let ra = rank.get(a.as_str()).copied().unwrap_or(usize::MAX);
                let rb = rank.get(b.as_str()).copied().unwrap_or(usize::MAX);
                if ra < rb {
                    if let Some(inner) = wins.get_mut(a) {
                        if let Some(v) = inner.get_mut(b) {
                            *v += 1;
                        }
                    }
                }
            }
        }
    }

    // Détermine le winner Condorcet : un candidat qui bat tous les autres
    // (wins[A][B] > wins[B][A] pour tout B).
    let mut winner: Option<String> = None;
    for a in &cands {
        let mut beats_all = true;
        for b in &cands {
            if a == b {
                continue;
            }
            let aw = wins.get(a).and_then(|m| m.get(b)).copied().unwrap_or(0);
            let bw = wins.get(b).and_then(|m| m.get(a)).copied().unwrap_or(0);
            if aw <= bw {
                beats_all = false;
                break;
            }
        }
        if beats_all {
            winner = Some(a.clone());
            break;
        }
    }

    let paradox_detected = winner.is_none() && cands.len() >= 3;

    Ok(CondorcetOutcome {
        winner,
        wins,
        ballots_count: ballots.len(),
        paradox_detected,
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
    fn unanimous_winner_condorcet() {
        let ballots = vec![
            b("v1", &["A", "B", "C"]),
            b("v2", &["A", "B", "C"]),
            b("v3", &["A", "B", "C"]),
        ];
        let out = condorcet_tally(&ballots).unwrap();
        assert_eq!(out.winner, Some("A".to_string()));
        assert!(!out.paradox_detected);
    }

    #[test]
    fn condorcet_paradox_3_way_cycle() {
        // Cycle parfait : A>B, B>C, C>A.  Aucun gagnant Condorcet.
        let ballots = vec![
            b("v1", &["A", "B", "C"]),
            b("v2", &["B", "C", "A"]),
            b("v3", &["C", "A", "B"]),
        ];
        let out = condorcet_tally(&ballots).unwrap();
        assert!(out.winner.is_none());
        assert!(out.paradox_detected);
    }

    #[test]
    fn condorcet_strict_majority() {
        let ballots = vec![
            b("v1", &["A", "B", "C"]),
            b("v2", &["A", "C", "B"]),
            b("v3", &["B", "A", "C"]),
        ];
        let out = condorcet_tally(&ballots).unwrap();
        // A bat B (2 vs 1), A bat C (2 vs 1) → A est Condorcet winner.
        assert_eq!(out.winner, Some("A".to_string()));
    }

    #[test]
    fn empty_ballots_errors() {
        let r = condorcet_tally(&[]);
        assert!(matches!(r, Err(MultiLlmError::Config(_))));
    }

    #[test]
    fn wins_matrix_complete() {
        let ballots = vec![b("v1", &["A", "B"])];
        let out = condorcet_tally(&ballots).unwrap();
        assert_eq!(out.wins["A"]["B"], 1);
        assert_eq!(out.wins["B"]["A"], 0);
    }

    #[test]
    fn outcome_serializable() {
        let ballots = vec![b("v", &["A", "B"])];
        let out = condorcet_tally(&ballots).unwrap();
        let s = serde_json::to_string(&out).unwrap();
        let _back: CondorcetOutcome = serde_json::from_str(&s).unwrap();
    }
}
