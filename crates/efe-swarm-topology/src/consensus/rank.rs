// SPDX-License-Identifier: UNLICENSED
//! Conversion scores → rangs (0 = meilleur), avec **rang MOYEN** sur égalités, et
//! helpers de préférence/marge. C'est le point le plus piégeux du consensus : une
//! conversion naïve biaiserait Borda/Condorcet par l'ordre d'arrivée des ex-æquo.

pub(crate) const EPS: f64 = 1e-9;

/// `ranks[i]` = rang du candidat i (0 = meilleur score). Les ex-æquo reçoivent le
/// **rang moyen** de leur groupe (fractionnaire) → aucun biais d'ordre.
pub fn scores_to_ranks(scores: &[f64]) -> Vec<f64> {
    let k = scores.len();
    if k == 0 {
        return Vec::new();
    }
    // indices triés par score DÉCROISSANT (tri stable).
    let mut idx: Vec<usize> = (0..k).collect();
    idx.sort_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap_or(std::cmp::Ordering::Equal));

    let mut ranks = vec![0.0f64; k];
    let mut pos = 0;
    while pos < k {
        // groupe d'ex-æquo [pos, end) sur le même score.
        let mut end = pos + 1;
        while end < k && (scores[idx[end]] - scores[idx[pos]]).abs() < EPS {
            end += 1;
        }
        // rang moyen des positions pos..=end-1.
        let avg = (pos + end - 1) as f64 / 2.0;
        for p in idx.iter().take(end).skip(pos) {
            ranks[*p] = avg;
        }
        pos = end;
    }
    ranks
}

/// Préférence STRICTE : i ≻ j ssi `score_i > score_j` (+ tolérance flottante).
pub fn prefers(scores: &[f64], i: usize, j: usize) -> bool {
    scores[i] > scores[j] + EPS
}

/// Marge gagnant vs 2ᵉ d'un vecteur de scores (0 si < 2 éléments).
pub fn margin(scores: &[f64]) -> f64 {
    if scores.len() < 2 {
        return 0.0;
    }
    let mut s = scores.to_vec();
    s.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    s[0] - s[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_strict_order() {
        // scores [3,1,2] → A(3) rang 0, C(2) rang 1, B(1) rang 2.
        assert_eq!(scores_to_ranks(&[3.0, 1.0, 2.0]), vec![0.0, 2.0, 1.0]);
    }

    #[test]
    fn ranks_average_on_ties() {
        // [0.5, 0.5, 0.1] → les deux 0.5 partagent les positions 0,1 → rang moyen 0.5 ; 0.1 → rang 2.
        assert_eq!(scores_to_ranks(&[0.5, 0.5, 0.1]), vec![0.5, 0.5, 2.0]);
        // tous égaux → tous au rang (k-1)/2.
        assert_eq!(scores_to_ranks(&[1.0, 1.0, 1.0]), vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn prefers_is_strict() {
        let s = [2.0, 2.0, 1.0];
        assert!(prefers(&s, 0, 2));
        assert!(!prefers(&s, 0, 1)); // égalité → pas de préférence
        assert!(!prefers(&s, 2, 0));
    }

    #[test]
    fn margin_top_two() {
        assert_eq!(margin(&[5.0, 2.0, 1.0]), 3.0);
        assert_eq!(margin(&[4.0]), 0.0);
        assert_eq!(margin(&[]), 0.0);
    }
}
