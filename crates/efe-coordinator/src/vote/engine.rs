// SPDX-License-Identifier: UNLICENSED
//! Implémentation du `VoteEngine`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::{CoordError, CoordResult};

/// Un bulletin = la réponse textuelle d'un LLM + son provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ballot {
    /// Provider qui a voté (= soumis cette réponse).
    pub provider: String,
    /// Texte de la réponse (sera hashé pour clustering).
    pub answer_text: String,
    /// Score qualité auto-déclaré (0..1, fallback 1.0).
    pub self_quality: f32,
    /// Latence ms (utile pour les tie-breaks).
    pub latency_ms: u64,
}

/// Résultat d'un vote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteOutcome {
    /// Provider gagnant.
    pub winner: String,
    /// Texte de la réponse gagnante.
    pub winner_text: String,
    /// Score du gagnant.
    pub winner_score: f32,
    /// Scores par provider (toujours présent, même les perdants).
    pub per_provider_scores: HashMap<String, f32>,
    /// Quorum requis.
    pub quorum_required: usize,
    /// Bulletins reçus.
    pub ballots_received: usize,
}

/// Moteur de vote pondéré.
///
/// Construit avec des poids `w_l` per-provider.  Si un provider présente
/// un bulletin sans poids explicite, le poids 1.0 est utilisé.
#[derive(Clone, Debug)]
pub struct VoteEngine {
    weights: HashMap<String, f32>,
    success_rates: HashMap<String, f32>,
}

impl VoteEngine {
    /// Construit un VoteEngine.
    #[must_use]
    pub fn new(
        weights: HashMap<String, f32>,
        success_rates: HashMap<String, f32>,
    ) -> Self {
        Self {
            weights,
            success_rates,
        }
    }

    /// Construit avec poids uniformes (1.0 partout).
    #[must_use]
    pub fn uniform() -> Self {
        Self {
            weights: HashMap::new(),
            success_rates: HashMap::new(),
        }
    }

    /// Tally pondéré.
    ///
    /// Algorithme :
    /// 1. Pour chaque bulletin, calcule `score = weight × self_quality`.
    /// 2. Cluster les réponses identiques (BLAKE3 du texte canonicalisé) :
    ///    additionne les scores des bulletins du même cluster.
    /// 3. Sélectionne le cluster avec le score max.
    /// 4. Si le score max < `quorum`, fallback sur le provider de plus haut
    ///    `success_rate` (invariant I5).
    /// 5. Si tie, tie-break déterministe par ordre alphabétique du provider.
    pub fn tally(&self, ballots: &[Ballot], quorum: usize) -> CoordResult<VoteOutcome> {
        if ballots.is_empty() {
            return Err(CoordError::VoteNoQuorum {
                ballots_received: 0,
                quorum_required: quorum,
            });
        }
        if quorum == 0 {
            return Err(CoordError::Config(
                "quorum must be >= 1 for VoteEngine::tally".into(),
            ));
        }

        // Phase 1 : score par provider.
        let mut per_provider: HashMap<String, f32> = HashMap::new();
        for b in ballots {
            let w = self.weights.get(&b.provider).copied().unwrap_or(1.0);
            let s = w * b.self_quality.clamp(0.0, 1.0);
            *per_provider.entry(b.provider.clone()).or_insert(0.0) += s;
        }

        // Phase 2 : cluster par texte (hash BLAKE3 du texte normalisé).
        let mut by_cluster: HashMap<String, (f32, String, String)> = HashMap::new();
        // (cluster_score, leading_provider, leading_text)
        for b in ballots {
            let w = self.weights.get(&b.provider).copied().unwrap_or(1.0);
            let s = w * b.self_quality.clamp(0.0, 1.0);
            let canon = canonicalize(&b.answer_text);
            let key = blake3::hash(canon.as_bytes()).to_hex().to_string();
            let entry = by_cluster
                .entry(key)
                .or_insert_with(|| (0.0, b.provider.clone(), b.answer_text.clone()));
            entry.0 += s;
            // Tie-break déterministe au sein du cluster :
            // la provenance du leader = provider lexicographiquement minimum.
            if b.provider < entry.1 {
                entry.1 = b.provider.clone();
                entry.2 = b.answer_text.clone();
            }
        }

        // Phase 3 : sélectionne cluster max.
        let (winner_score, winner, winner_text) = by_cluster
            .into_values()
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.1.cmp(&a.1)) // tie-break : provider plus petit gagne
            })
            .ok_or(CoordError::VoteNoQuorum {
                ballots_received: ballots.len(),
                quorum_required: quorum,
            })?;

        // Phase 4 : check quorum.  Le quorum s'exprime en NOMBRE de
        // bulletins du cluster gagnant (pas en score).
        // On compte combien de bulletins ont effectivement voté pour ce cluster.
        let canon_winner = canonicalize(&winner_text);
        let key_winner = blake3::hash(canon_winner.as_bytes()).to_hex().to_string();
        let votes_for_winner = ballots
            .iter()
            .filter(|b| {
                let k = blake3::hash(canonicalize(&b.answer_text).as_bytes())
                    .to_hex()
                    .to_string();
                k == key_winner
            })
            .count();

        if votes_for_winner < quorum {
            // Fallback I5 : provider de plus haut success_rate parmi
            // ceux qui ont voté.
            if let Some(fallback) = ballots
                .iter()
                .max_by(|a, b| {
                    let sa = self.success_rates.get(&a.provider).copied().unwrap_or(0.0);
                    let sb = self.success_rates.get(&b.provider).copied().unwrap_or(0.0);
                    sa.partial_cmp(&sb)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.provider.cmp(&a.provider))
                })
            {
                return Ok(VoteOutcome {
                    winner: fallback.provider.clone(),
                    winner_text: fallback.answer_text.clone(),
                    winner_score: per_provider
                        .get(&fallback.provider)
                        .copied()
                        .unwrap_or(0.0),
                    per_provider_scores: per_provider,
                    quorum_required: quorum,
                    ballots_received: ballots.len(),
                });
            }
            return Err(CoordError::VoteNoQuorum {
                ballots_received: ballots.len(),
                quorum_required: quorum,
            });
        }

        Ok(VoteOutcome {
            winner,
            winner_text,
            winner_score,
            per_provider_scores: per_provider,
            quorum_required: quorum,
            ballots_received: ballots.len(),
        })
    }
}

/// Normalisation pour clustering : lowercase + trim + collapse whitespace.
fn canonicalize(s: &str) -> String {
    s.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(provider: &str, text: &str, q: f32) -> Ballot {
        Ballot {
            provider: provider.into(),
            answer_text: text.into(),
            self_quality: q,
            latency_ms: 100,
        }
    }

    #[test]
    fn canonicalize_lowers_and_collapses() {
        assert_eq!(canonicalize("Hello   World"), "hello world");
        assert_eq!(canonicalize("  X\nY  "), "x y");
    }

    #[test]
    fn uniform_majority_wins() {
        let v = VoteEngine::uniform();
        let ballots = vec![
            b("claude", "yes", 1.0),
            b("gpt4", "yes", 1.0),
            b("gemini", "no", 1.0),
        ];
        let out = v.tally(&ballots, 2).unwrap();
        assert_eq!(out.winner_text, "yes");
        assert_eq!(out.ballots_received, 3);
    }

    #[test]
    fn weighted_minority_can_win() {
        let mut w = HashMap::new();
        w.insert("claude".to_string(), 10.0);
        w.insert("gpt4".to_string(), 1.0);
        w.insert("gemini".to_string(), 1.0);
        let v = VoteEngine::new(w, HashMap::new());
        let ballots = vec![
            b("claude", "yes", 1.0), // poids = 10
            b("gpt4", "no", 1.0),    // poids = 1
            b("gemini", "no", 1.0),  // poids = 1 (total no = 2)
        ];
        let out = v.tally(&ballots, 1).unwrap();
        assert_eq!(out.winner_text, "yes");
    }

    #[test]
    fn no_quorum_falls_back_to_success_rate() {
        // 3 réponses différentes → aucun cluster n'atteint quorum=2.
        // success_rate plus haut : gpt4 → gagne par fallback (I5).
        let mut rates = HashMap::new();
        rates.insert("claude".to_string(), 0.7);
        rates.insert("gpt4".to_string(), 0.99);
        rates.insert("gemini".to_string(), 0.5);
        let v = VoteEngine::new(HashMap::new(), rates);
        let ballots = vec![
            b("claude", "AAA", 1.0),
            b("gpt4", "BBB", 1.0),
            b("gemini", "CCC", 1.0),
        ];
        let out = v.tally(&ballots, 2).unwrap();
        assert_eq!(out.winner, "gpt4");
    }

    #[test]
    fn empty_ballots_errors() {
        let v = VoteEngine::uniform();
        let r = v.tally(&[], 1);
        assert!(matches!(r, Err(CoordError::VoteNoQuorum { .. })));
    }

    #[test]
    fn zero_quorum_errors() {
        let v = VoteEngine::uniform();
        let r = v.tally(&[b("c", "x", 1.0)], 0);
        assert!(matches!(r, Err(CoordError::Config(_))));
    }

    #[test]
    fn whitespace_variations_cluster_together() {
        let v = VoteEngine::uniform();
        let ballots = vec![
            b("claude", "Hello World", 1.0),
            b("gpt4", "hello   world", 1.0),
            b("gemini", "X", 1.0),
        ];
        let out = v.tally(&ballots, 2).unwrap();
        // Les deux premiers sont canonicalement identiques → cluster=2, quorum=2 OK.
        assert_eq!(canonicalize(&out.winner_text), "hello world");
    }

    #[test]
    fn tie_break_deterministic_by_alphabetical() {
        // 2 LLMs disent "A", 2 LLMs disent "B" — exact tie en poids.
        let v = VoteEngine::uniform();
        let ballots = vec![
            b("claude", "A", 1.0),
            b("gpt4", "B", 1.0),
            b("gemini", "A", 1.0),
            b("mistral", "B", 1.0),
        ];
        let out = v.tally(&ballots, 2).unwrap();
        // Les deux clusters ont score 2.0 ; tie-break : provider min lexico
        // dans le cluster gagnant.  "claude" et "gemini" en A, "gpt4" et
        // "mistral" en B.  Min "claude" < min "gpt4" → A gagne via
        // notre tie-break (then b.1.cmp(&a.1)).
        assert_eq!(out.winner_text, "A");
        assert_eq!(out.winner, "claude");
    }

    #[test]
    fn fallback_on_no_quorum_picks_min_lex_provider_when_tie() {
        // Cluster X score = 1.0 (claude seul) ; cluster Y score = 0.2 (gpt4+gemini, qualité 0.1).
        // X gagne en score MAIS n'a qu'1 vote ; quorum=2 → fallback I5.
        // Avec success_rates = {} (tous 0), tie-break déterministe :
        //   `b.provider.cmp(&a.provider)` dans max_by → min lex wins.
        // Min lex parmi {claude, gpt4, gemini} = "claude" → answer "X".
        let v = VoteEngine::uniform();
        let ballots = vec![
            b("claude", "X", 1.0),
            b("gpt4", "Y", 0.1),
            b("gemini", "Y", 0.1),
        ];
        let out = v.tally(&ballots, 2).unwrap();
        assert_eq!(out.winner_text, "X");
        assert_eq!(out.winner, "claude");
    }

    #[test]
    fn outcome_has_all_providers_in_scores() {
        let v = VoteEngine::uniform();
        let ballots = vec![
            b("claude", "yes", 1.0),
            b("gpt4", "yes", 1.0),
        ];
        let out = v.tally(&ballots, 1).unwrap();
        assert!(out.per_provider_scores.contains_key("claude"));
        assert!(out.per_provider_scores.contains_key("gpt4"));
    }
}
