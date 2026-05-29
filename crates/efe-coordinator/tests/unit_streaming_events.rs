// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration : streaming SSE des workflows (LlmTokenStreamed, DraftToken).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_mission_a, run_mission_b, run_speculative, Coordinator, MissionARequest,
    MissionBRequest, MockLlmProvider, ProviderRegistry, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

#[tokio::test]
async fn mission_a_emits_llm_token_streamed_events() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("claude").with_response("alpha beta gamma delta epsilon"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("test"));

    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta());
    let req = MissionARequest {
        tenant_id: None,
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 100,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
        task_class_scoring_table: None,
    };
    let _resp = run_mission_a(&coord, req).await.unwrap();

    let mut token_count = 0;
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
    {
        if ev.tag() == "LLM_TOKEN_STREAMED" {
            token_count += 1;
        }
    }
    // 5 mots → 5 tokens streamés.
    assert_eq!(token_count, 5);
}

#[tokio::test]
async fn mission_b_emits_token_streamed_for_each_provider() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("a").with_response("aa bb"),
    )); // 2 tokens
    reg.register(Arc::new(
        MockLlmProvider::new("b").with_response("cc dd ee"),
    )); // 3 tokens
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("test"));

    let req = MissionBRequest {
        tenant_id: None,
        user_text: "x".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into()],
        llm_metadata: HashMap::new(),
        vote_weights: HashMap::new(),
        quorum: Some(1),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let _ = run_mission_b(&coord, req).await.unwrap();

    let mut by_provider = std::collections::HashMap::<String, u32>::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
    {
        if let efe_coordinator::StreamEvent::LlmTokenStreamed { provider, .. } = &ev {
            *by_provider.entry(provider.clone()).or_insert(0) += 1;
        }
    }
    assert_eq!(by_provider.get("a").copied(), Some(2));
    assert_eq!(by_provider.get("b").copied(), Some(3));
}

#[tokio::test]
async fn speculative_emits_draft_token_events() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("drafter").with_response("alpha beta gamma"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("claude").with_response("alpha beta gamma"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("test"));

    let req = SpeculativeRequest {
        tenant_id: None,
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 64,
        max_total_latency: Duration::from_secs(2),
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0,
        // R3.S3 : gate de bruit + min words.
        reject_draft_noise: true,
        min_words_for_quick_accept: 1,
    };
    let _ = run_speculative(&coord, req).await.unwrap();

    let mut draft_token_count = 0;
    let mut tags_seen = std::collections::HashSet::new();
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(50), rx.recv()).await
    {
        tags_seen.insert(ev.tag().to_string());
        if ev.tag() == "DRAFT_TOKEN" {
            draft_token_count += 1;
        }
    }
    // "alpha beta gamma" → 3 mots draftés.
    assert_eq!(draft_token_count, 3);
    assert!(tags_seen.contains("SPECULATION_RESOLVED"));
    assert!(tags_seen.contains("SWARM_STARTED"));
    assert!(tags_seen.contains("SWARM_FINISHED"));
}
