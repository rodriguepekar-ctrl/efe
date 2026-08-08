// SPDX-License-Identifier: UNLICENSED
//! E2E Relay : Plan (Claude) → Code (GPT) → Synthesis (Gemini).

use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::{run_relay, MultiLlmError, RelayRequest};

fn req() -> RelayRequest {
    RelayRequest {
        user_text: "Build a Rust web server".into(),
        planner_llm: "claude".into(),
        implementer_llm: "gpt".into(),
        synthesizer_llm: "gemini".into(),
        max_budget_cents: 1000,
        max_total_latency: Duration::from_secs(10),
    }
}

#[tokio::test]
async fn relay_happy_path_all_3_phases() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response(
        "1. Setup tokio\n2. Build axum app\n3. Add auth",
    )));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response(
        "```rust\nfn main() { println!(\"server\"); }\n```",
    )));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response(
        "## Tutorial\nHere's how to build the server step by step.",
    )));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let resp = run_relay(&coord, req()).await.unwrap();
    assert_eq!(resp.llms_used.len(), 3);
    assert!(!resp.plan_json.is_empty());
    assert!(!resp.code_json.is_empty());
    assert!(!resp.synthesis_json.is_empty());
    assert!(resp.final_text.contains("Tutorial") || !resp.final_text.is_empty());
}

#[tokio::test]
async fn relay_planner_down_fails_phase_alpha() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").fail_after(0)));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("code")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("synth")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let r = run_relay(&coord, req()).await;
    assert!(matches!(r, Err(MultiLlmError::RelayPhaseFailed { phase, .. }) if phase.contains("α")));
}

#[tokio::test]
async fn relay_implementer_down_fails_phase_beta() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("1. Step")));
    reg.register(Arc::new(MockLlmProvider::new("gpt").fail_after(0)));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("synth")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let r = run_relay(&coord, req()).await;
    assert!(matches!(r, Err(MultiLlmError::RelayPhaseFailed { phase, .. }) if phase.contains("β")));
}

#[tokio::test]
async fn relay_synthesizer_down_fails_phase_gamma() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("1. Step")));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("```rust\nfn x(){}\n```")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").fail_after(0)));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let r = run_relay(&coord, req()).await;
    assert!(matches!(r, Err(MultiLlmError::RelayPhaseFailed { phase, .. }) if phase.contains("γ")));
}

#[tokio::test]
async fn relay_swarm_id_hex_present() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("1. Plan")));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("```rust\nfn x(){}\n```")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("## Result\nDone.")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let resp = run_relay(&coord, req()).await.unwrap();
    assert!(resp.swarm_id_hex.len() >= 16);
}

#[tokio::test]
async fn relay_records_total_cost() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("1. P").with_cost_cents(5)));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("```rust\nfn x(){}\n```").with_cost_cents(3)));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("## R\nbody.").with_cost_cents(2)));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let resp = run_relay(&coord, req()).await.unwrap();
    assert_eq!(resp.total_cost_cents, 5 + 3 + 2);
}
