// SPDX-License-Identifier: UNLICENSED
//! E2E Debate : pro vs contra sur N rounds + jury.

use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::{run_debate, DebateRequest, MultiLlmError};

fn make_coord() -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response(
        "<thesis>Rust is safer due to ownership.</thesis>\n- Memory safety guaranteed at compile time.",
    )));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response(
        "<thesis>Go is simpler to learn.</thesis>\n- Less syntax overhead.",
    )));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response(
        "After reviewing both arguments, the PRO position wins on safety; CONTRA wins on simplicity.",
    )));
    Coordinator::builder().registry(reg).build().unwrap()
}

fn req(rounds: u8) -> DebateRequest {
    DebateRequest {
        user_text: "Should we use Rust or Go?".into(),
        pro_llm: "claude".into(),
        contra_llm: "gpt".into(),
        jury_llm: "gemini".into(),
        rounds,
        max_budget_cents: 1000,
        max_total_latency: Duration::from_secs(15),
    }
}

#[tokio::test]
async fn debate_2_rounds_plus_jury() {
    let coord = make_coord();
    let resp = run_debate(&coord, req(2)).await.unwrap();
    assert_eq!(resp.arguments_per_round.len(), 2);
    assert_eq!(resp.jury_llm, "gemini");
    assert!(!resp.verdict.is_empty());
}

#[tokio::test]
async fn debate_1_round_minimum() {
    let coord = make_coord();
    let resp = run_debate(&coord, req(1)).await.unwrap();
    assert_eq!(resp.arguments_per_round.len(), 1);
}

#[tokio::test]
async fn debate_3_distinct_llms_required() {
    let coord = make_coord();
    let bad_req = DebateRequest {
        user_text: "x".into(),
        pro_llm: "claude".into(),
        contra_llm: "claude".into(), // same
        jury_llm: "gemini".into(),
        rounds: 1,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let r = run_debate(&coord, bad_req).await;
    assert!(matches!(r, Err(MultiLlmError::DebateTooFewLlms { .. })));
}

#[tokio::test]
async fn debate_zero_rounds_errors() {
    let coord = make_coord();
    let r = run_debate(&coord, req(0)).await;
    assert!(matches!(r, Err(MultiLlmError::Config(_))));
}

#[tokio::test]
async fn debate_excess_rounds_errors() {
    let coord = make_coord();
    let r = run_debate(&coord, req(100)).await;
    assert!(matches!(r, Err(MultiLlmError::Config(_))));
}

#[tokio::test]
async fn debate_arguments_have_correct_stances() {
    let coord = make_coord();
    let resp = run_debate(&coord, req(2)).await.unwrap();
    // Round 1 : opening (PRO + CONTRA).
    assert_eq!(resp.arguments_per_round[0][0].stance, "PRO");
    assert_eq!(resp.arguments_per_round[0][1].stance, "CONTRA");
    // Round 2 : rebuttals.
    assert_eq!(resp.arguments_per_round[1][0].stance, "REBUTTAL");
    assert_eq!(resp.arguments_per_round[1][1].stance, "REBUTTAL");
}

#[tokio::test]
async fn debate_records_total_cost() {
    let coord = make_coord();
    let resp = run_debate(&coord, req(1)).await.unwrap();
    // 2 LLMs round 1 + 1 jury = 3 appels.
    assert!(resp.total_cost_cents >= 3); // mock = 1 cent per call
}
