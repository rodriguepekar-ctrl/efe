// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration : Borda + Condorcet.

use efe_multi_llm::{borda_tally, condorcet_tally};
use efe_multi_llm::vote_ext::borda::BordaBallot;

fn b(voter: &str, ranking: &[&str]) -> BordaBallot {
    BordaBallot {
        voter: voter.into(),
        ranking: ranking.iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn borda_3_llms_ranks_consistently() {
    let ballots = vec![
        b("claude", &["claude", "gpt", "gemini"]),
        b("gpt", &["claude", "gpt", "gemini"]),
        b("gemini", &["claude", "gpt", "gemini"]),
    ];
    let out = borda_tally(&ballots).unwrap();
    assert_eq!(out.winner, "claude");
    assert_eq!(out.scores["claude"], 9);
    assert_eq!(out.scores["gpt"], 6);
    assert_eq!(out.scores["gemini"], 3);
}

#[test]
fn borda_handles_split_minds() {
    let ballots = vec![
        b("claude", &["claude", "gpt", "gemini"]),
        b("gpt", &["gpt", "gemini", "claude"]),
        b("gemini", &["gemini", "claude", "gpt"]),
    ];
    let out = borda_tally(&ballots).unwrap();
    // claude: 3+1+2=6, gpt: 2+3+1=6, gemini: 1+2+3=6 → tie → lex min "claude" gagne.
    assert_eq!(out.winner, "claude");
}

#[test]
fn condorcet_picks_strict_winner() {
    let ballots = vec![
        b("v1", &["A", "B", "C"]),
        b("v2", &["A", "C", "B"]),
        b("v3", &["B", "A", "C"]),
    ];
    let out = condorcet_tally(&ballots).unwrap();
    assert_eq!(out.winner, Some("A".to_string()));
}

#[test]
fn condorcet_paradox_returns_none() {
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
fn condorcet_unanimous() {
    let ballots = vec![
        b("v1", &["A", "B"]),
        b("v2", &["A", "B"]),
        b("v3", &["A", "B"]),
    ];
    let out = condorcet_tally(&ballots).unwrap();
    assert_eq!(out.winner, Some("A".to_string()));
}

#[test]
fn borda_outcome_includes_all_scores() {
    let ballots = vec![b("v", &["X", "Y", "Z"])];
    let out = borda_tally(&ballots).unwrap();
    assert!(out.scores.contains_key("X"));
    assert!(out.scores.contains_key("Y"));
    assert!(out.scores.contains_key("Z"));
}

#[test]
fn borda_tie_break_alphabetical() {
    let ballots = vec![b("v1", &["A", "B"]), b("v2", &["B", "A"])];
    let out = borda_tally(&ballots).unwrap();
    assert_eq!(out.winner, "A"); // tie → "A" (lex min).
}
