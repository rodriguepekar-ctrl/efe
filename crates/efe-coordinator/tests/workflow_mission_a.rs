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

// ─── Sprint S4 R2 : couverture offline de la nouvelle instrumentation ───────
//
// Ces tests valident en offline (avec MockLlmProvider, sans nano-robots) que
// `run_mission_a` remplit correctement le champ `MissionAResponse.chewing_metrics`
// et émet l'event `StreamEvent::PreChewingMetrics`. Sans ces tests, les
// modifications source de S4 ne sont validées qu'en mode live (qui requiert
// EFE_LIVE_LLM=1 + EFE_LIVE_SLM=1 + 3 daemons SLM UP).

#[tokio::test]
async fn mission_a_chewing_metrics_filled_with_mocks() {
    // ── Setup minimal : 1 mock LLM, 0 nano-robot ────────────────────────
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude_mock").with_response("answer")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let mut metas = HashMap::new();
    metas.insert("claude_mock".to_string(), meta());

    // user_text court (< 500 chars) → compressor skip → ratio = 1.0
    let user_text = "Tell me X";
    let req = MissionARequest {
        user_text: user_text.into(),
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

    // ── Le champ chewing_metrics doit être Some(...) systématiquement ───
    let m = resp
        .chewing_metrics
        .as_ref()
        .expect("S4: MissionAResponse.chewing_metrics doit être Some(...) après run_mission_a");

    // ── input_tokens_raw : ceil(9/4) = 3 (pour "Tell me X") ─────────────
    assert_eq!(
        m.input_tokens_raw,
        3,
        "estimate_tokens('Tell me X') = ceil(9/4) = 3, got {}",
        m.input_tokens_raw
    );

    // ── Sans compressor : compressed == raw, ratio = 1.0, savings = 0 ───
    assert_eq!(
        m.input_tokens_compressed, m.input_tokens_raw,
        "Sans compressor (clients.compressor = None), compressed == raw"
    );
    assert_eq!(m.compression_ratio, 1.0, "ratio = 1.0 quand pas de compression");
    assert_eq!(m.savings_tokens, 0);
    assert_eq!(m.savings_ratio, 0.0);

    // ── selected_llm = "claude_mock" (forcé via available_llms.first()) ─
    assert_eq!(m.selected_llm, "claude_mock");

    // ── Sans router : pas de reason, latency = 0 ────────────────────────
    assert!(
        m.router_reason.is_empty(),
        "Sans router nano-robot, router_reason reste vide"
    );
    assert_eq!(
        m.router_latency_ms, 0,
        "Sans router, router_latency_ms reste 0"
    );

    // ── Sans analyzer / compressor / verifier : leurs latences = 0 ──────
    assert_eq!(m.analyzer_latency_ms, 0);
    assert_eq!(m.compressor_latency_ms, 0);
    assert_eq!(m.verifier_latency_ms, 0);

    // ── total_slm_latency_ms = 0 (somme des 3 ci-dessus) ────────────────
    assert_eq!(m.total_slm_latency_ms, 0);

    // ── llm_latency_ms : non zéro (le mock retourne un peu de latency) ──
    // MockLlmProvider retourne par défaut latency_ms = 1.
    assert!(
        m.llm_latency_ms <= 1000,
        "llm_latency_ms ≤ 1000 ms (mock rapide), got {}",
        m.llm_latency_ms
    );

    // ── total_pipeline_latency_ms > 0 (le mission_a a tourné) ───────────
    assert!(
        m.total_pipeline_latency_ms > 0,
        "total_pipeline_latency_ms > 0 après une mission complète"
    );

    // ── Cohérence : pipeline >= slm (qui est 0 ici) ─────────────────────
    assert!(m.total_pipeline_latency_ms >= m.total_slm_latency_ms);

    // ── verdicts vide (pas de verifier) ─────────────────────────────────
    assert!(resp.verdicts.is_empty());
}

#[tokio::test]
async fn mission_a_emits_pre_chewing_metrics_event() {
    // Vérifie que l'event PRE_CHEWING_METRICS est bien émis sur le bus
    // par run_mission_a, en mode offline (mock LLM).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("x")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();
    let (_t, mut rx) = coord.subscribe(Some("s4-offline-event-test"));

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

    // Drain events.
    let mut tags = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        tags.push(ev.tag());
    }

    // L'event PRE_CHEWING_METRICS doit être présent.
    assert!(
        tags.contains(&"PRE_CHEWING_METRICS"),
        "S4: PRE_CHEWING_METRICS doit être émis par run_mission_a, got tags={tags:?}"
    );

    // Et il doit arriver AVANT SWARM_FINISHED (ordre du workflow).
    let pre_chew_pos = tags.iter().position(|t| *t == "PRE_CHEWING_METRICS").unwrap();
    let finished_pos = tags
        .iter()
        .position(|t| *t == "SWARM_FINISHED")
        .expect("SWARM_FINISHED doit être présent");
    assert!(
        pre_chew_pos < finished_pos,
        "S4: PRE_CHEWING_METRICS (pos {pre_chew_pos}) doit précéder SWARM_FINISHED (pos {finished_pos})"
    );
}

#[tokio::test]
async fn mission_a_chewing_metrics_serializable_through_response() {
    // Vérifie que MissionAResponse.chewing_metrics fait correctement le
    // roundtrip JSON via serde (utilisé pour exposer la response via API).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("x")));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

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
    let resp = run_mission_a(&coord, req).await.unwrap();

    // Sérialise → désérialise → compare.
    let json = serde_json::to_string(&resp).unwrap();
    assert!(
        json.contains("\"chewing_metrics\""),
        "Le JSON sérialisé doit contenir le champ chewing_metrics"
    );
    let back: efe_coordinator::MissionAResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(back.final_text, resp.final_text);
    assert!(back.chewing_metrics.is_some());
}

#[tokio::test]
async fn mission_a_backward_compat_old_response_without_chewing_metrics() {
    // Garantit que la backward compat #[serde(default)] fonctionne :
    // un MissionAResponse sérialisé SANS le champ chewing_metrics
    // (par exemple par un ancien serveur ou un test offline antérieur)
    // doit toujours désérialiser correctement avec chewing_metrics = None.
    let old_json = r#"{
        "final_text": "Paris est la capitale",
        "llm_used": "claude_old",
        "cost_cents": 1,
        "latency_ms": 1500,
        "verdicts": [],
        "audit_chain_hash": "abc123"
    }"#;
    let parsed: efe_coordinator::MissionAResponse =
        serde_json::from_str(old_json).expect("backward compat #[serde(default)] doit fonctionner");
    assert!(
        parsed.chewing_metrics.is_none(),
        "chewing_metrics doit être None pour un JSON pré-S4 (#[serde(default)])"
    );
    assert_eq!(parsed.final_text, "Paris est la capitale");
}

