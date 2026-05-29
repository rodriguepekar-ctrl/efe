// SPDX-License-Identifier: UNLICENSED
//! **R3.S6 — Tests d'intégration chaos OFFLINE (mock + chaos).**
//!
//! Prouvent que chaque mission dégrade correctement quand un provider faute,
//! en branchant un [`ChaosLlmProvider`] sur un `MockLlmProvider` (déterministe,
//! sans réseau). Les tests live (réseau réel) sont dans `tests/live_chaos.rs`.
//!
//! Gate de compilation : `--features chaos` (le module `chaos` est
//! feature-gated). Aucun provider cloud requis (le chaos wrappe des mocks).

#![cfg(feature = "chaos")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::failure::FailureMode;
use efe_coordinator::{
    run_mission_a, run_mission_b, run_speculative, ChaosFaultKind, ChaosInjector, Coordinator,
    MissionARequest, MissionBRequest, MockLlmProvider, ProviderRegistry, SpeculativeRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 100,
        p99_latency_ms: 500,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

fn budget() -> Budget {
    Budget {
        max_cost_cents: 1000,
        max_cost_microcents: 0,
        max_latency_ms: 10_000,
        min_quality_score: 0.0,
    }
}

// =====================================================================
// #9 — Mission A : 429 sur le selected → fallback chain
// =====================================================================

/// Le LLM `selected` (claude) faute systématiquement (429) ; le fallback
/// (gemini sain) prend le relais. Asserte la réponse + `failure_mode`.
#[tokio::test]
async fn chaos_mission_a_429_triggers_fallback() {
    // claude faute toujours (429), gemini répond.
    let claude = Arc::new(MockLlmProvider::new("claude").with_response("CLAUDE"));
    let injector = Arc::new(
        ChaosInjector::seeded(ChaosFaultKind::HttpStatus(429), 1).with_fault_rate("claude", 1.0),
    );
    let reg = ProviderRegistry::new();
    reg.register(injector.wrap(claude)); // id = "claude", faute 100 %
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("GEMINI-PARIS")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta());
    metas.insert("gemini".to_string(), meta());

    // Sans table de scoring → selected = available_llms.first() = "claude".
    let req = MissionARequest {
        user_text: "Capitale de la France ?".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude".into(), "gemini".into()],
        llm_metadata: metas,
        budget: budget(),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        task_class_scoring_table: None,
        tenant_id: None,
    };
    let resp = run_mission_a(&coord, req).await.expect("mission A survit via fallback");

    assert_eq!(resp.llm_used, "gemini", "fallback vers gemini");
    assert_eq!(resp.final_text, "GEMINI-PARIS");
    assert_eq!(
        resp.failure_mode,
        FailureMode::LlmFallbackTriggered {
            from: "claude".into(),
            to: "gemini".into(),
        },
        "failure_mode doit tracer le fallback"
    );
    assert_eq!(resp.fallback_chain_used.as_deref(), Some("gemini"));
}

// =====================================================================
// #10 — Mission B : 1/3 LLM faute → vote dégradé
// =====================================================================

#[tokio::test]
async fn chaos_mission_b_one_llm_fault_degrades_vote() {
    // "a" faute toujours (503) ; "b" et "c" votent "Paris".
    let a = Arc::new(MockLlmProvider::new("a").with_response("Paris"));
    let injector = Arc::new(
        ChaosInjector::seeded(ChaosFaultKind::HttpStatus(503), 7).with_fault_rate("a", 1.0),
    );
    let reg = ProviderRegistry::new();
    reg.register(injector.wrap(a));
    reg.register(Arc::new(MockLlmProvider::new("b").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("Paris")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    for id in ["a", "b", "c"] {
        metas.insert(id.to_string(), meta());
    }
    let req = MissionBRequest {
        user_text: "Capitale ?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into(), "c".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
        tenant_id: None,
    };
    let resp = run_mission_b(&coord, req).await.expect("mission B survit (vote sur 2/3)");

    assert!(resp.vote_degraded, "1 LLM fauté → vote dégradé");
    assert_eq!(resp.failed_llms.len(), 1, "exactement 1 LLM échoué");
    assert_eq!(resp.failed_llms[0].0, "a", "c'est 'a' qui a fauté");
    assert_ne!(resp.winner_llm, "a", "le winner n'est pas le LLM fauté");
}

// =====================================================================
// #11 — Speculative : drafter EmptyResponse → noise gate → slow path
// =====================================================================

#[tokio::test]
async fn chaos_speculative_drafter_empty_response_forces_slow_path() {
    // Le drafter renvoie un faux succès vide → R3.S3 noise gate rejette →
    // slow path → verifier produit la vraie réponse.
    let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("ignored"));
    let injector = Arc::new(
        ChaosInjector::seeded(ChaosFaultKind::EmptyResponse, 4).with_fault_rate("drafter", 1.0),
    );
    let reg = ProviderRegistry::new();
    reg.register(injector.wrap(drafter));
    reg.register(Arc::new(MockLlmProvider::new("verifier").with_response("Paris")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        "Capitale de la France ?".into(),
        TaskClass::FactualQa,
        "drafter".into(),
        "verifier".into(),
    );
    req.max_words_for_quick_accept = 10; // hot path candidat → noise gate évalué
    req.reject_draft_noise = true;

    let resp = run_speculative(&coord, req).await.expect("spec survit via slow path");
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");

    assert_eq!(
        cm.spec_noise_rejected,
        Some(true),
        "draft vide → rejeté comme bruit"
    );
    assert!(
        !resp.verifier_skipped,
        "bruit rejeté → slow path → verifier appelé"
    );
    assert_eq!(resp.final_text, "Paris", "le verifier produit la réponse");
}

// =====================================================================
// #12 — Speculative : drafter 503 → drafter bypass → verifier
// =====================================================================

#[tokio::test]
async fn chaos_speculative_drafter_503_bypasses_to_verifier() {
    // Le drafter faute (503) → bypass → verifier produit la réponse.
    let drafter = Arc::new(MockLlmProvider::new("drafter").with_response("ignored"));
    let injector = Arc::new(
        ChaosInjector::seeded(ChaosFaultKind::HttpStatus(503), 9).with_fault_rate("drafter", 1.0),
    );
    let reg = ProviderRegistry::new();
    reg.register(injector.wrap(drafter));
    reg.register(Arc::new(MockLlmProvider::new("verifier").with_response("Paris")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut req = SpeculativeRequest::with_drafter_and_verifier(
        "Capitale de la France ?".into(),
        TaskClass::FactualQa,
        "drafter".into(),
        "verifier".into(),
    );
    req.max_words_for_quick_accept = 10;

    let resp = run_speculative(&coord, req).await.expect("spec survit via bypass");

    assert!(!resp.draft_accepted, "drafter fauté → rien à accepter");
    assert!(
        !resp.verifier_skipped,
        "drafter bypassé → verifier appelé (slow path)"
    );
    assert_eq!(resp.final_text, "Paris", "le verifier produit la réponse");
    assert!(
        resp.draft_text.is_empty(),
        "drafter bypassé → draft_text vide, got {:?}",
        resp.draft_text
    );
}
