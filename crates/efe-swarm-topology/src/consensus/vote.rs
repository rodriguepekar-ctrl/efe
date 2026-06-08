// SPDX-License-Identifier: UNLICENSED
//! [`MajorityVote`] — mode des VALEURS des candidats. Le PLANCHER de référence
//! (S1 a montré qu'il échoue sur erreurs corrélées) ; on le construit pour le
//! mesurer, pas pour le vendre.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{empty_outcome, AggOutcome, Aggregator, Ballot};

/// Vote majoritaire : la valeur la plus fréquente parmi les candidats.
pub struct MajorityVote;

impl Aggregator for MajorityVote {
    fn method(&self) -> &'static str {
        "vote"
    }

    fn aggregate(&self, _task: &Value, candidates: &[Value], _ballots: &[Ballot]) -> AggOutcome {
        let k = candidates.len();
        if k == 0 {
            return empty_outcome("vote");
        }
        // clé = forme JSON canonique ; valeur = (compte, 1er index d'apparition).
        let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for (i, c) in candidates.iter().enumerate() {
            let key = c.to_string();
            let e = counts.entry(key).or_insert((0, i));
            e.0 += 1;
        }
        // scores[i] = nombre de candidats identiques à i.
        let scores: Vec<f64> = candidates
            .iter()
            .map(|c| counts.get(&c.to_string()).map(|(n, _)| *n as f64).unwrap_or(0.0))
            .collect();
        // gagnant = compte max ; tiebreak = plus petit 1er index (stable).
        let (best_count, best_first) = counts.values().fold((0usize, usize::MAX), |(bc, bf), &(n, fi)| {
            if n > bc || (n == bc && fi < bf) {
                (n, fi)
            } else {
                (bc, bf)
            }
        });
        // confidence = (max − 2ᵉ) / K.
        let mut cv: Vec<usize> = counts.values().map(|(n, _)| *n).collect();
        cv.sort_unstable_by(|a, b| b.cmp(a));
        let second = cv.get(1).copied().unwrap_or(0);
        let confidence = (best_count as f64 - second as f64) / k as f64;

        AggOutcome {
            selected: candidates[best_first].clone(),
            selected_index: best_first,
            scores,
            confidence,
            method: "vote",
            cycle_detected: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agg(cands: &[Value]) -> AggOutcome {
        MajorityVote.aggregate(&json!({}), cands, &[])
    }

    #[test]
    fn picks_majority_value() {
        let out = agg(&[json!("A"), json!("A"), json!("B")]);
        assert_eq!(out.selected, json!("A"));
        assert_eq!(out.selected_index, 0);
        assert!(out.confidence > 0.0);
    }

    #[test]
    fn tie_breaks_on_first_index_zero_confidence() {
        let out = agg(&[json!("A"), json!("B")]);
        assert_eq!(out.selected_index, 0, "tiebreak = plus petit index");
        assert_eq!(out.confidence, 0.0);
    }

    #[test]
    fn plurality_without_absolute_majority() {
        // A=2, B=1, C=1 → pluralité A, confidence (2-1)/4.
        let out = agg(&[json!("A"), json!("A"), json!("B"), json!("C")]);
        assert_eq!(out.selected, json!("A"));
        assert!((out.confidence - 0.25).abs() < 1e-9);
    }

    #[test]
    fn single_candidate() {
        let out = agg(&[json!(42)]);
        assert_eq!(out.selected, json!(42));
        assert_eq!(out.confidence, 1.0);
    }

    #[test]
    fn empty_is_null() {
        let out = agg(&[]);
        assert_eq!(out.selected, Value::Null);
        assert_eq!(out.method, "vote");
    }
}
