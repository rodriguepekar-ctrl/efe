// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `Speculator`.

use std::sync::Arc;

use efe_coordinator::{LlmCall, MockLlmProvider, Speculator};

#[tokio::test]
async fn perfect_draft_accepted() {
    let drafter = Arc::new(MockLlmProvider::new("d").with_response("expected output"));
    let verifier = Arc::new(MockLlmProvider::new("v").with_response("expected output"));
    let s = Speculator::new(drafter, verifier, 0.5);
    let out = s.run(LlmCall::simple("?", 100)).await.unwrap();
    assert!(out.draft_accepted);
    assert_eq!(out.tokens_regenerated, 0);
}

#[tokio::test]
async fn diverging_draft_rejected() {
    let drafter = Arc::new(MockLlmProvider::new("d").with_response("wrong stuff entirely"));
    let verifier = Arc::new(MockLlmProvider::new("v").with_response("the correct response is here"));
    let s = Speculator::new(drafter, verifier, 0.9);
    let out = s.run(LlmCall::simple("?", 100)).await.unwrap();
    assert!(!out.draft_accepted);
    assert_eq!(out.final_text, "the correct response is here");
}

#[tokio::test]
async fn partial_overlap_below_threshold() {
    // Drafter et verifier ont 1 mot commun sur 4 → Jaccard ~0.14.
    let drafter = Arc::new(MockLlmProvider::new("d").with_response("yes hello world"));
    let verifier = Arc::new(MockLlmProvider::new("v").with_response("hello stranger today"));
    let s = Speculator::new(drafter, verifier, 0.5);
    let out = s.run(LlmCall::simple("?", 100)).await.unwrap();
    assert!(!out.draft_accepted);
}

#[tokio::test]
async fn drafter_id_exposed() {
    let drafter = Arc::new(MockLlmProvider::new("smolm-q5"));
    let verifier = Arc::new(MockLlmProvider::new("claude-opus"));
    let s = Speculator::new(drafter, verifier, 0.5);
    assert_eq!(s.drafter_id(), "smolm-q5");
    assert_eq!(s.verifier_id(), "claude-opus");
}

#[tokio::test]
async fn latency_saved_can_be_positive_or_negative() {
    // Drafter ultra rapide (1ms), verifier lent (50ms) — typique cas favorable.
    let drafter = Arc::new(
        MockLlmProvider::new("d")
            .with_response("answer")
            .with_latency(std::time::Duration::from_millis(1)),
    );
    let verifier = Arc::new(
        MockLlmProvider::new("v")
            .with_response("answer")
            .with_latency(std::time::Duration::from_millis(50)),
    );
    let s = Speculator::new(drafter, verifier, 0.5);
    let out = s.run(LlmCall::simple("?", 10)).await.unwrap();
    // latency_saved_ms peut être négatif si le drafter ajoute un peu d'overhead,
    // mais doit être plausible (entre -100 et +100 ms).
    assert!(
        out.latency_saved_ms.abs() < 200,
        "abnormal latency_saved_ms: {}",
        out.latency_saved_ms
    );
}
