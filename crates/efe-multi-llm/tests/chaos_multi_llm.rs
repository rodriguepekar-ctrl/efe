// SPDX-License-Identifier: UNLICENSED
//! Chaos suite Phase 3 : 1 LLM down / 2 down / all down.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::{
    run_cascade, run_debate, run_relay, run_vote_3llm, CascadeRequest, DebateRequest,
    MultiLlmError, RelayRequest, VoteMode, Vote3LlmRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;

fn build_coord(specs: &[(&str, Option<&str>)]) -> Coordinator {
    let reg = ProviderRegistry::new();
    for (id, resp) in specs {
        match resp {
            Some(text) => reg.register(Arc::new(MockLlmProvider::new(*id).with_response(*text))),
            None => reg.register(Arc::new(MockLlmProvider::new(*id).fail_after(0))),
        }
    }
    Coordinator::builder().registry(reg).build().unwrap()
}

#[tokio::test]
async fn vote_3llm_1_down_2_up_still_succeeds() {
    let coord = build_coord(&[
        ("claude", Some("X")),
        ("gpt", Some("X")),
        ("gemini", None), // down
    ]);
    let req = Vote3LlmRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Plurality,
        quorum: Some(2),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    assert_eq!(resp.per_llm_scores.len(), 2); // 1 LLM down → 2 votes seulement
}

#[tokio::test]
async fn vote_3llm_2_down_falls_back_to_solo() {
    let coord = build_coord(&[
        ("claude", Some("Lone answer")),
        ("gpt", None),
        ("gemini", None),
    ]);
    let req = Vote3LlmRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Plurality,
        quorum: Some(1), // quorum bas pour permettre 1 seul vote
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_vote_3llm(&coord, req).await.unwrap();
    assert_eq!(resp.per_llm_scores.len(), 1);
}

#[tokio::test]
async fn vote_3llm_all_down_errors() {
    let coord = build_coord(&[
        ("claude", None),
        ("gpt", None),
        ("gemini", None),
    ]);
    let req = Vote3LlmRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Plurality,
        quorum: None,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_vote_3llm(&coord, req).await;
    assert!(matches!(r, Err(MultiLlmError::AllLlmsDown)));
}

#[tokio::test]
async fn relay_phase_alpha_down_propagates_error() {
    let coord = build_coord(&[
        ("claude", None),
        ("gpt", Some("```rust\nfn x(){}\n```")),
        ("gemini", Some("done")),
    ]);
    let req = RelayRequest {
        user_text: "x".into(),
        planner_llm: "claude".into(),
        implementer_llm: "gpt".into(),
        synthesizer_llm: "gemini".into(),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_relay(&coord, req).await;
    assert!(matches!(r, Err(MultiLlmError::RelayPhaseFailed { .. })));
}

#[tokio::test]
async fn debate_jury_down_propagates_error() {
    let coord = build_coord(&[
        ("claude", Some("<thesis>A</thesis>")),
        ("gpt", Some("<thesis>B</thesis>")),
        ("gemini", None),
    ]);
    let req = DebateRequest {
        user_text: "Q?".into(),
        pro_llm: "claude".into(),
        contra_llm: "gpt".into(),
        jury_llm: "gemini".into(),
        rounds: 1,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let r = run_debate(&coord, req).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn cascade_drafter_down_propagates_error() {
    let coord = build_coord(&[
        ("drafter", None),
        ("verifier", Some("v")),
        ("validator", Some("V")),
    ]);
    let req = CascadeRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        drafter_llm: "drafter".into(),
        verifier_llm: "verifier".into(),
        validator_llm: "validator".into(),
        accept_threshold: 0.7,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_cascade(&coord, req).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn cascade_verifier_down_propagates_error() {
    let coord = build_coord(&[
        ("drafter", Some("d")),
        ("verifier", None),
        ("validator", Some("V")),
    ]);
    let req = CascadeRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        drafter_llm: "drafter".into(),
        verifier_llm: "verifier".into(),
        validator_llm: "validator".into(),
        accept_threshold: 0.7,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    let r = run_cascade(&coord, req).await;
    // Verifier mandatory pour la cascade.
    assert!(r.is_err());
}

#[tokio::test]
async fn cascade_validator_down_falls_back_to_draft() {
    let coord = build_coord(&[
        ("drafter", Some("draft_x")),
        ("verifier", Some("REGEN_ALL needed")),
        ("validator", None),
    ]);
    let req = CascadeRequest {
        user_text: "Q?".into(),
        task_class: TaskClass::FactualQa,
        drafter_llm: "drafter".into(),
        verifier_llm: "verifier".into(),
        validator_llm: "validator".into(),
        accept_threshold: 0.7,
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(2),
    };
    // Validator down + REGEN_ALL → fallback sur draft.
    let resp = run_cascade(&coord, req).await.unwrap();
    assert!(!resp.validator_used);
    assert_eq!(resp.final_text, "draft_x");
}

#[tokio::test]
async fn specialist_with_no_capability_match_uses_neutral_default() {
    let coord = build_coord(&[("a", Some("ok")), ("b", Some("ok")), ("c", Some("ok"))]);
    let req = efe_multi_llm::SpecialistRequest {
        user_text: "Do something.".into(),
        available_llms: vec!["a".into(), "b".into(), "c".into()],
        capability_scores: HashMap::new(), // pas de scores → tous neutres
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = efe_multi_llm::run_specialist(&coord, req).await.unwrap();
    assert!(!resp.assignments.is_empty());
}
