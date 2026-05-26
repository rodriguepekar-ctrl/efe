// SPDX-License-Identifier: UNLICENSED
//! Workflow Speculative end-to-end.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{
    run_speculative, CoordError, Coordinator, MockLlmProvider, ProviderRegistry,
    SpeculativeRequest,
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
async fn speculative_accepts_draft_when_match() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("drafter").with_response("the answer is 42")));
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("the answer is 42")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_speculative(&coord, req).await.unwrap();
    assert!(resp.draft_accepted);
    assert_eq!(resp.final_text, "the answer is 42");
}

#[tokio::test]
async fn speculative_rejects_draft_when_diverge() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("drafter").with_response("totally wrong")));
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("the correct response")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_speculative(&coord, req).await.unwrap();
    assert!(!resp.draft_accepted);
    assert_eq!(resp.final_text, "the correct response");
}

#[tokio::test]
async fn speculative_requires_drafter_registered() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("X")));
    // PAS de drafter
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_speculative(&coord, req).await;
    assert!(matches!(r, Err(CoordError::LlmProvider { .. })));
}

#[tokio::test]
async fn speculative_requires_verifier_registered() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("drafter").with_response("X")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "ghost-llm".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_speculative(&coord, req).await;
    assert!(matches!(r, Err(CoordError::LlmProvider { .. })));
}

#[tokio::test]
async fn speculative_emits_speculation_resolved_event() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("drafter").with_response("ok")));
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("ok")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("ui"));

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(3),
    };
    let _ = run_speculative(&coord, req).await.unwrap();

    let mut tags = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        tags.push(ev.tag());
    }
    assert!(tags.contains(&"SPECULATION_RESOLVED"), "tags={:?}", tags);
}
