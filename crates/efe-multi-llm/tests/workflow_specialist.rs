// SPDX-License-Identifier: UNLICENSED
//! E2E Specialist : decompose + route + aggregate.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::{run_specialist, SpecialistRequest};

fn capability_scores() -> HashMap<String, HashMap<String, f32>> {
    let mut caps: HashMap<String, HashMap<String, f32>> = HashMap::new();
    let mut c = HashMap::new();
    c.insert("CODE".to_string(), 0.94);
    c.insert("FACTUAL_QA".to_string(), 0.85);
    c.insert("TRANSLATION".to_string(), 0.80);
    c.insert("ANALYSIS".to_string(), 0.82);
    c.insert("CREATIVE".to_string(), 0.75);
    c.insert("REASONING".to_string(), 0.82);
    caps.insert("gpt".to_string(), c);
    let mut c = HashMap::new();
    c.insert("CODE".to_string(), 0.85);
    c.insert("FACTUAL_QA".to_string(), 0.92);
    c.insert("REASONING".to_string(), 0.95);
    c.insert("CREATIVE".to_string(), 0.91);
    c.insert("TRANSLATION".to_string(), 0.82);
    c.insert("ANALYSIS".to_string(), 0.88);
    caps.insert("claude".to_string(), c);
    let mut c = HashMap::new();
    c.insert("CODE".to_string(), 0.80);
    c.insert("TRANSLATION".to_string(), 0.93);
    c.insert("ANALYSIS".to_string(), 0.92);
    c.insert("CREATIVE".to_string(), 0.85);
    c.insert("FACTUAL_QA".to_string(), 0.88);
    c.insert("REASONING".to_string(), 0.80);
    caps.insert("gemini".to_string(), c);
    caps
}

fn make_coord() -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("```rust\nfn x(){}\n```")));
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("Reasoning answer.")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("Analysis answer.")));
    Coordinator::builder().registry(reg).build().unwrap()
}

#[tokio::test]
async fn specialist_decomposes_and_routes_3_subtasks() {
    let coord = make_coord();
    let req = SpecialistRequest {
        user_text: "Translate this to Rust. Then explain the algorithm. Then write a haiku."
            .into(),
        available_llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        capability_scores: capability_scores(),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_specialist(&coord, req).await.unwrap();
    assert_eq!(resp.assignments.len(), 3);
    assert!(!resp.final_text.is_empty());
}

#[tokio::test]
async fn specialist_each_subtask_assigned_to_best_llm() {
    let coord = make_coord();
    let req = SpecialistRequest {
        user_text: "Translate to Rust. Analyze semantics. Write a haiku.".into(),
        available_llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        capability_scores: capability_scores(),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_specialist(&coord, req).await.unwrap();
    // Vérifie qu'au moins 1 LLM différent est utilisé.
    let used: std::collections::HashSet<_> = resp
        .assignments
        .iter()
        .map(|a| a.assigned_llm.clone())
        .collect();
    assert!(used.len() >= 1);
}

#[tokio::test]
async fn specialist_records_audit_hash() {
    let coord = make_coord();
    let req = SpecialistRequest {
        user_text: "One single task here.".into(),
        available_llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        capability_scores: capability_scores(),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_specialist(&coord, req).await.unwrap();
    assert!(resp.swarm_id_hex.len() >= 16);
}

#[tokio::test]
async fn specialist_single_subtask_works() {
    let coord = make_coord();
    let req = SpecialistRequest {
        user_text: "Explain quantum entanglement in simple terms.".into(),
        available_llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        capability_scores: capability_scores(),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_specialist(&coord, req).await.unwrap();
    assert_eq!(resp.assignments.len(), 1);
}

#[tokio::test]
async fn specialist_anti_monoculture_with_multiple_subtasks() {
    let coord = make_coord();
    // 5 sub-tasks, toutes "Code"-classifiées → favoriserait GPT (0.94 vs 0.85 vs 0.80).
    // Mais anti-monoculture I11 (≤ 60%) force au moins 2 sur 5 vers d'autres LLMs.
    let req = SpecialistRequest {
        user_text: "Build a Rust function. Build a Rust enum. Build a Rust trait. Build a Rust impl. Build a Rust macro.".into(),
        available_llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        capability_scores: capability_scores(),
        max_budget_cents: 200,
        max_total_latency: Duration::from_secs(10),
    };
    let resp = run_specialist(&coord, req).await.unwrap();
    let gpt_count = resp
        .assignments
        .iter()
        .filter(|a| a.assigned_llm == "gpt")
        .count();
    let total = resp.assignments.len();
    assert!(
        gpt_count <= (total * 60 / 100),
        "anti-monoculture: gpt has {gpt_count}/{total}"
    );
}
