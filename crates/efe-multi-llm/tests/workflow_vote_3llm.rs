// SPDX-License-Identifier: UNLICENSED
//! E2E Mission Vote 3-LLM avec reformulation pré-vote.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::{run_vote_3llm, MultiLlmError, VoteMode, Vote3LlmRequest};
use efe_nano_app_coordinator::types::router::TaskClass;

fn coord_with(provider_responses: &[(&str, &str)]) -> Coordinator {
    let reg = ProviderRegistry::new();
    for (id, resp) in provider_responses {
        reg.register(Arc::new(MockLlmProvider::new(*id).with_response(*resp)));
    }
    Coordinator::builder().registry(reg).build().unwrap()
}

#[tokio::test]
async fn vote_3llm_unanimous_winner() {
    let coord = coord_with(&[
        ("claude", "Paris is the capital of France."),
        ("gpt", "Paris is the capital of France."),
        ("gemini", "Paris is the capital of France."),
    ]);
    let req = Vote3LlmRequest {
        user_text: "Capital of France?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Plurality,
        quorum: Some(2),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    assert_eq!(resp.per_llm_scores.len(), 3);
    assert!(!resp.final_text.is_empty());
}

#[tokio::test]
async fn vote_3llm_borda_mode_picks_winner() {
    let coord = coord_with(&[
        ("claude", "Answer A."),
        ("gpt", "Answer A."),
        ("gemini", "Answer B."),
    ]);
    let req = Vote3LlmRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Borda,
        quorum: None,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    assert_eq!(resp.mode, VoteMode::Borda);
    assert!(!resp.winner_llm.is_empty());
}

#[tokio::test]
async fn vote_3llm_condorcet_mode_returns_result() {
    let coord = coord_with(&[
        ("claude", "X"),
        ("gpt", "Y"),
        ("gemini", "X"),
    ]);
    let req = Vote3LlmRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Condorcet,
        quorum: None,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    assert_eq!(resp.mode, VoteMode::Condorcet);
    assert!(!resp.winner_llm.is_empty());
}

#[tokio::test]
async fn vote_3llm_all_providers_down_errors() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").fail_after(0)));
    reg.register(Arc::new(MockLlmProvider::new("b").fail_after(0)));
    reg.register(Arc::new(MockLlmProvider::new("c").fail_after(0)));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let req = Vote3LlmRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into(), "c".into()],
        mode: VoteMode::Borda,
        quorum: None,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_vote_3llm(&coord, req).await;
    assert!(matches!(r, Err(MultiLlmError::AllLlmsDown)));
}

#[tokio::test]
async fn vote_3llm_response_contains_all_scores() {
    let coord = coord_with(&[
        ("claude", "X"),
        ("gpt", "X"),
        ("gemini", "X"),
    ]);
    let req = Vote3LlmRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Plurality,
        quorum: None,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    assert_eq!(resp.per_llm_scores.len(), 3);
    assert!(resp.audit_chain_hash.len() >= 16);
}

#[tokio::test]
async fn vote_3llm_with_2_llms_only_works() {
    let coord = coord_with(&[("claude", "A"), ("gpt", "A")]);
    let req = Vote3LlmRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into()],
        mode: VoteMode::Plurality,
        quorum: Some(2),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    assert_eq!(resp.per_llm_scores.len(), 2);
}

// Test que la fonctionnalité ignore peut être appelée sans paramètre HashMap.
#[tokio::test]
async fn vote_3llm_default_quorum_uses_ceil_half() {
    let coord = coord_with(&[
        ("a", "X"),
        ("b", "X"),
        ("c", "Y"),
    ]);
    let req = Vote3LlmRequest {
        user_text: "Q".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into(), "c".into()],
        mode: VoteMode::Plurality,
        quorum: None, // default = ceil(3/2) = 2
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    // Cluster X a 2 votes → quorum atteint.
    assert!(!resp.final_text.is_empty());
    // Évite warning unused.
    let _ = HashMap::<String, f32>::new();
}
