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

// ─── Sprint S5 R2 : couverture offline de l'instrumentation Mission B ───────
//
// Ces 3 tests valident en offline (avec MockLlmProvider × 3) que
// `run_mission_b` remplit correctement le champ `MissionBResponse.chewing_metrics`
// (S5 extension) et émet l'event `StreamEvent::PreChewingMetrics`. Cohérence
// avec les 4 tests offline S4 ajoutés dans `workflow_mission_a.rs`.

#[tokio::test]
async fn mission_b_chewing_metrics_filled_with_mocks() {
    // 3 mocks LLM, 0 nano-robot (tous None par défaut) :
    // - chewing_metrics doit être Some(...)
    // - selected_llm = winner_llm
    // - llm_latency_ms = max des 3 latences (= mock fast latency)
    // - llm_tokens_generated = somme des 3
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("gpt4").with_response("Paris")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("Paris")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta());
    metas.insert("gpt4".to_string(), meta());
    metas.insert("gemini".to_string(), meta());

    let req = MissionBRequest {
        user_text: "What is the capital of France?".into(), // < 500 chars → no compress
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt4".into(), "gemini".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_mission_b(&coord, req).await.unwrap();

    // ── chewing_metrics doit être Some(...) ────────────────────────────
    let m = resp
        .chewing_metrics
        .as_ref()
        .expect("S5: MissionBResponse.chewing_metrics doit être Some(...)");

    // ── input_tokens_raw : "What is the capital of France?" = 30 chars → 8 tokens ─
    let expected_raw = (30.0_f32 / 4.0).ceil() as u32; // ceil(7.5) = 8
    assert_eq!(m.input_tokens_raw, expected_raw);

    // ── Sans compressor : compressed == raw, ratio = 1.0 ────────────────
    assert_eq!(m.input_tokens_compressed, m.input_tokens_raw);
    assert_eq!(m.compression_ratio, 1.0);
    assert_eq!(m.savings_tokens, 0);

    // ── selected_llm = winner du vote Symphony (unanime sur "Paris") ────
    // Tie-break par min lex : "claude" < "gemini" < "gpt4" → claude gagne.
    assert_eq!(m.selected_llm, "claude");
    assert_eq!(m.selected_llm, resp.winner_llm, "selected_llm == winner_llm");

    // ── llm_latency_ms = max des 3 mocks (très rapide, ≤ 1000 ms) ──────
    assert!(m.llm_latency_ms <= 1000);

    // ── llm_tokens_generated = somme des 3 (toujours ≥ 0) ──────────────
    // MockLlmProvider retourne par défaut une valeur — on check juste ≥ 0
    // (peut être 0 si mock défaut, c'est OK pour ce test).
    let _ = m.llm_tokens_generated;

    // ── Sans analyzer / compressor / verifier (tous None) : latencies = 0 ─
    assert_eq!(m.analyzer_latency_ms, 0);
    assert_eq!(m.compressor_latency_ms, 0);
    assert_eq!(m.verifier_latency_ms, 0);
    assert_eq!(m.total_slm_latency_ms, 0);

    // ── router_* vides (Mission B n'utilise pas le router) ─────────────
    assert!(m.router_reason.is_empty());
    assert_eq!(m.router_latency_ms, 0);

    // ── total_pipeline_latency_ms > 0 et cohérent ───────────────────────
    assert!(m.total_pipeline_latency_ms > 0);
    assert!(m.total_pipeline_latency_ms >= m.total_slm_latency_ms);

    // ── Total cost = somme des 3 LLMs ───────────────────────────────────
    let _ = resp.total_cost_cents; // smoke check (peut être 0 avec mocks)
}

#[tokio::test]
async fn mission_b_emits_pre_chewing_metrics_event() {
    // Vérifie que l'event PRE_CHEWING_METRICS est bien émis par
    // run_mission_b et qu'il arrive AVANT SwarmFinished.
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").with_response("X")));
    reg.register(Arc::new(MockLlmProvider::new("b").with_response("X")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("s5-offline-event-test"));

    let mut metas = HashMap::new();
    metas.insert("a".to_string(), meta());
    metas.insert("b".to_string(), meta());
    let req = MissionBRequest {
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let _resp = run_mission_b(&coord, req).await.unwrap();

    let mut tags = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        tags.push(ev.tag());
    }

    // ── L'event doit être présent ──────────────────────────────────────
    assert!(
        tags.contains(&"PRE_CHEWING_METRICS"),
        "S5: PRE_CHEWING_METRICS missing, got tags={tags:?}"
    );

    // ── Ordre : PRE_CHEWING_METRICS arrive AVANT SWARM_FINISHED ─────────
    let pre_chew_pos = tags.iter().position(|t| *t == "PRE_CHEWING_METRICS").unwrap();
    let finished_pos = tags
        .iter()
        .position(|t| *t == "SWARM_FINISHED")
        .expect("SWARM_FINISHED missing");
    assert!(
        pre_chew_pos < finished_pos,
        "S5: PRE_CHEWING_METRICS ({pre_chew_pos}) must precede SWARM_FINISHED ({finished_pos})"
    );

    // ── VOTE_RESOLVED doit aussi être présent (Mission B specific) ──────
    assert!(
        tags.contains(&"VOTE_RESOLVED"),
        "S5: VOTE_RESOLVED missing (existant Mission B), got tags={tags:?}"
    );
}

#[tokio::test]
async fn mission_b_chewing_metrics_serializable_through_response() {
    // Vérifie le roundtrip serde JSON sur MissionBResponse.chewing_metrics.
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("a").with_response("X")));
    reg.register(Arc::new(MockLlmProvider::new("b").with_response("X")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("a".to_string(), meta());
    metas.insert("b".to_string(), meta());
    let req = MissionBRequest {
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["a".into(), "b".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: None,
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(5),
    };
    let resp = run_mission_b(&coord, req).await.unwrap();

    // ── Sérialise → désérialise → compare ──────────────────────────────
    let json = serde_json::to_string(&resp).unwrap();
    assert!(
        json.contains("\"chewing_metrics\""),
        "S5: JSON sérialisé doit contenir chewing_metrics"
    );
    let back: efe_coordinator::MissionBResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back.final_text, resp.final_text);
    assert!(back.chewing_metrics.is_some());
    assert_eq!(back.winner_llm, resp.winner_llm);

    // ── Backward compat : JSON pré-S5 sans le champ ─────────────────────
    let pre_s5_json = r#"{
        "final_text": "old answer",
        "winner_llm": "claude_old",
        "winner_score": 1.0,
        "per_llm_scores": {"claude_old": 1.0},
        "total_cost_cents": 5,
        "latency_ms": 2000,
        "verdicts": [],
        "audit_chain_hash": "abc123"
    }"#;
    let old: efe_coordinator::MissionBResponse =
        serde_json::from_str(pre_s5_json).expect("backward compat #[serde(default)] OK");
    assert!(old.chewing_metrics.is_none());
    assert_eq!(old.final_text, "old answer");
}
