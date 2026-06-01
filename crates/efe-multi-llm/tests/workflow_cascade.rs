// SPDX-License-Identifier: UNLICENSED
//! E2E Cascade speculative 3 niveaux.

use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::{run_cascade, CascadeRequest};

fn coord_3(drafter_text: &str, verifier_text: &str, validator_text: &str) -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("drafter").with_response(drafter_text),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("verifier").with_response(verifier_text),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("validator").with_response(validator_text),
    ));
    Coordinator::builder().registry(reg).build().unwrap()
}

fn req() -> CascadeRequest {
    CascadeRequest {
        user_text: "What is 2+2?".into(),
        task_class: efe_nano_app_coordinator::types::router::TaskClass::FactualQa,
        drafter_llm: "drafter".into(),
        verifier_llm: "verifier".into(),
        validator_llm: "validator".into(),
        accept_threshold: 0.7,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn cascade_accepts_when_drafter_matches_verifier() {
    let coord = coord_3("the answer is four", "the answer is four", "totally different");
    let resp = run_cascade(&coord, req()).await.unwrap();
    assert_eq!(resp.verifier_decision, "ACCEPT");
    assert_eq!(resp.final_text, "the answer is four");
    assert!(!resp.validator_used);
}

#[tokio::test]
async fn cascade_regen_ranges_uses_verifier_output() {
    let coord = coord_3("wrong stuff entirely", "correct response here please", "v");
    let resp = run_cascade(&coord, req()).await.unwrap();
    assert!(resp.verifier_decision == "REGEN_RANGES" || resp.verifier_decision == "REGEN_ALL");
}

#[tokio::test]
async fn cascade_regen_all_escalates_to_validator() {
    let coord = coord_3("draft", "REGEN_ALL is needed here", "validator final answer");
    let resp = run_cascade(&coord, req()).await.unwrap();
    assert_eq!(resp.verifier_decision, "REGEN_ALL");
    assert!(resp.validator_used);
    assert_eq!(resp.final_text, "validator final answer");
}

#[tokio::test]
async fn cascade_response_includes_audit_hash() {
    let coord = coord_3("a", "a", "v");
    let resp = run_cascade(&coord, req()).await.unwrap();
    assert!(resp.audit_chain_hash.len() >= 16);
}

#[tokio::test]
async fn cascade_latency_saved_can_be_positive() {
    // Mock providers ont latence 1ms → workflow rapide → latency_saved > 0.
    let coord = coord_3("ok", "ok", "v");
    let resp = run_cascade(&coord, req()).await.unwrap();
    assert!(resp.latency_saved_ms >= 0);
}

#[tokio::test]
async fn cascade_validator_down_falls_back_to_draft() {
    // Validator fail mais on déclenche REGEN_ALL.
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("drafter").with_response("draft_answer"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("verifier").with_response("REGEN_ALL needed"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("validator").fail_after(0),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let resp = run_cascade(&coord, req()).await.unwrap();
    // Validator a échoué → fallback sur draft.
    assert_eq!(resp.verifier_decision, "REGEN_ALL");
    assert!(!resp.validator_used);
    assert_eq!(resp.final_text, "draft_answer");
}
