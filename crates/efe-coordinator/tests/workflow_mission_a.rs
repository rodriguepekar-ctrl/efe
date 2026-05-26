// SPDX-License-Identifier: UNLICENSED
//! Workflow Mission A end-to-end (sans nano-robots — provider mock).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, Coordinator, MissionARequest, MockLlmProvider, ProviderRegistry,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

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
async fn mission_a_runs_with_mock_llm_only() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude_mock").with_response("the answer is X")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude_mock".to_string(), meta());

    let req = MissionARequest {
        user_text: "Tell me X".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude_mock".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            max_latency_ms: 10_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_mission_a(&coord, req).await.unwrap();
    assert_eq!(resp.llm_used, "claude_mock");
    assert_eq!(resp.final_text, "the answer is X");
    assert!(resp.audit_chain_hash.len() == 64);
    assert!(resp.latency_ms < 5000);
}

#[tokio::test]
async fn mission_a_publishes_swarm_started_and_finished() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("done")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("test-ui"));

    let mut metas = HashMap::new();
    metas.insert("c".to_string(), meta());

    let req = MissionARequest {
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["c".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            max_latency_ms: 10_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let _resp = run_mission_a(&coord, req).await.unwrap();

    // Drain le canal en non-bloquant.
    let mut tags = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        tags.push(ev.tag());
    }
    assert!(tags.contains(&"SWARM_STARTED"), "tags={:?}", tags);
    assert!(tags.contains(&"SWARM_FINISHED"), "tags={:?}", tags);
    assert!(tags.contains(&"LLM_FINISHED"), "tags={:?}", tags);
}

#[tokio::test]
async fn mission_a_with_no_available_llm_errors() {
    let coord = Coordinator::for_tests();
    let req = MissionARequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec![],
        llm_metadata: HashMap::new(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 1000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_mission_a(&coord, req).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn mission_a_falls_back_to_first_llm_if_router_absent() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("first").with_response("FIRST")));
    reg.register(Arc::new(MockLlmProvider::new("second").with_response("SECOND")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = MissionARequest {
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["first".into(), "second".into()],
        llm_metadata: HashMap::new(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 1000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let resp = run_mission_a(&coord, req).await.unwrap();
    assert_eq!(resp.llm_used, "first");
}

