// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `VoteEngine`.

use std::collections::HashMap;

use efe_coordinator::{Ballot, CoordError, VoteEngine};

fn b(provider: &str, text: &str, q: f32) -> Ballot {
    Ballot {
        provider: provider.into(),
        answer_text: text.into(),
        self_quality: q,
        latency_ms: 100,
    }
}

#[test]
fn unanimous_passes_quorum() {
    let v = VoteEngine::uniform();
    let ballots = vec![
        b("a", "yes", 1.0),
        b("b", "yes", 1.0),
        b("c", "yes", 1.0),
    ];
    let out = v.tally(&ballots, 3).unwrap();
    assert_eq!(out.winner_text, "yes");
    assert_eq!(out.ballots_received, 3);
}

#[test]
fn split_vote_fallback_invariant_i5() {
    let mut rates = HashMap::new();
    rates.insert("claude".to_string(), 0.99);
    rates.insert("gpt4".to_string(), 0.7);
    let v = VoteEngine::new(HashMap::new(), rates);
    // Pas de cluster majoritaire (chacun dit autre chose).
    let ballots = vec![
        b("claude", "A", 1.0),
        b("gpt4", "B", 1.0),
    ];
    let out = v.tally(&ballots, 2).unwrap();
    // success_rate plus haut = claude → fallback I5.
    assert_eq!(out.winner, "claude");
}

#[test]
fn invariant_quorum_zero_errors() {
    let v = VoteEngine::uniform();
    let r = v.tally(&[b("a", "x", 1.0)], 0);
    assert!(matches!(r, Err(CoordError::Config(_))));
}

#[test]
fn invariant_no_ballots_errors() {
    let v = VoteEngine::uniform();
    let r = v.tally(&[], 1);
    assert!(matches!(r, Err(CoordError::VoteNoQuorum { .. })));
}

#[test]
fn whitespace_insensitive_clustering() {
    let v = VoteEngine::uniform();
    let ballots = vec![
        b("a", "the answer", 1.0),
        b("b", "The Answer", 1.0),
        b("c", "  the   ANSWER  ", 1.0),
    ];
    let out = v.tally(&ballots, 3).unwrap();
    assert!(out.winner_text.to_lowercase().contains("answer"));
}

#[test]
fn weighted_overrides_majority() {
    let mut w = HashMap::new();
    w.insert("claude".to_string(), 100.0);
    let v = VoteEngine::new(w, HashMap::new());
    let ballots = vec![
        b("claude", "minority view", 1.0),
        b("gpt4", "majority view", 1.0),
        b("gemini", "majority view", 1.0),
    ];
    let out = v.tally(&ballots, 1).unwrap();
    assert_eq!(out.winner_text, "minority view");
}

#[test]
fn per_provider_scores_complete() {
    let v = VoteEngine::uniform();
    let ballots = vec![
        b("a", "x", 1.0),
        b("b", "y", 0.5),
    ];
    let out = v.tally(&ballots, 1).unwrap();
    assert!((out.per_provider_scores.get("a").copied().unwrap() - 1.0).abs() < 1e-6);
    assert!((out.per_provider_scores.get("b").copied().unwrap() - 0.5).abs() < 1e-6);
}
