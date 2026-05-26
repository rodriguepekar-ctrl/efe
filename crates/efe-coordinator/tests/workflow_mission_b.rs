// SPDX-License-Identifier: UNLICENSED
//! Workflow Mission B end-to-end.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_b, CoordError, Coordinator, MissionBRequest, MockLlmProvider, ProviderRegistry,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::LlmMetadata;

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

#[tokio::test]
async fn mission_b_with_unanimous_llms_picks_consensus() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("Paris is the capital")));
    reg.register(Arc::new(MockLlmProvider::new("gpt4").with_response("Paris is the capital")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("Paris is the capital")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    for id in ["claude", "gpt4", "gemini"] {
        metas.insert(id.to_string(), meta());
    }

    let req = MissionBRequest {
        user_text: "Capital of France?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt4".into(), "gemini".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: Some(2),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_mission_b(&coord, req).await.unwrap();
    assert_eq!(resp.final_text, "Paris is the capital");
    assert_eq!(resp.per_llm_scores.len(), 3);
    assert!(resp.audit_chain_hash.len() == 64);
}

#[tokio::test]
async fn mission_b_too_few_llms_errors() {
    let coord = Coordinator::for_tests();
    let req = MissionBRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["only-one".into()],
        llm_metadata: HashMap::new(),
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_mission_b(&coord, req).await;
    assert!(matches!(r, Err(CoordError::MissionBTooFewLlms { n: 1 })));
}

#[tokio::test]
async fn mission_b_all_providers_fail_errors() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").fail_after(0)));
    reg.register(Arc::new(MockLlmProvider::new("b").fail_after(0)));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = MissionBRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into()],
        llm_metadata: HashMap::new(),
        vote_weights: HashMap::new(),
        quorum: Some(1),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_mission_b(&coord, req).await;
    assert!(matches!(r, Err(CoordError::LlmProvider { .. })));
}

#[tokio::test]
async fn mission_b_publishes_vote_resolved_event() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").with_response("YES")));
    reg.register(Arc::new(MockLlmProvider::new("b").with_response("YES")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("ui"));

    let req = MissionBRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into()],
        llm_metadata: HashMap::new(),
        vote_weights: HashMap::new(),
        quorum: Some(2),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let _ = run_mission_b(&coord, req).await.unwrap();

    let mut tags = Vec::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
    {
        tags.push(ev.tag());
    }
    assert!(tags.contains(&"VOTE_RESOLVED"), "tags={:?}", tags);
}

#[tokio::test]
async fn mission_b_weighted_vote_overrides_majority() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("minority view")));
    reg.register(Arc::new(MockLlmProvider::new("gpt4").with_response("majority view")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("majority view")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut weights = HashMap::new();
    weights.insert("claude".to_string(), 100.0);
    weights.insert("gpt4".to_string(), 1.0);
    weights.insert("gemini".to_string(), 1.0);
    let req = MissionBRequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt4".into(), "gemini".into()],
        llm_metadata: HashMap::new(),
        vote_weights: weights,
        quorum: Some(1),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let resp = run_mission_b(&coord, req).await.unwrap();
    assert_eq!(resp.final_text, "minority view");
    assert_eq!(resp.winner_llm, "claude");
}
