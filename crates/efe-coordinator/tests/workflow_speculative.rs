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
        // S7 R2 : préserve comportement pré-S7 — drafter ID fixe + slow path
        // (max_words_for_quick_accept = 0 désactive le hot path).
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0,
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
        // S7 R2 : préserve comportement pré-S7 — drafter ID fixe + slow path
        // (max_words_for_quick_accept = 0 désactive le hot path).
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0,
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
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0,
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
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0,
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
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0,
    };
    let _ = run_speculative(&coord, req).await.unwrap();

    let mut tags = Vec::new();
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
        tags.push(ev.tag());
    }
    assert!(tags.contains(&"SPECULATION_RESOLVED"), "tags={:?}", tags);
}

// ─── Sprint S7 R2 — Tests offline du hot path + chewing_metrics ──────────
//
// Ces 6 tests valident en OFFLINE :
// 1. Le hot path (`max_words_for_quick_accept > 0`) skip le verifier quand
//    le drafter répond court.
// 2. Le slow path est forcé quand le draft est long.
// 3. `chewing_metrics` est rempli avec les nouveaux champs S7.
// 4. `verifier_skipped` est propagé dans la response ET dans
//    `chewing_metrics.verifier_skipped`.
// 5. `spec_savings_cents` > 0 quand verifier skipped.
// 6. Le helper `with_drafter_and_verifier` produit une requête valide.

/// **S7-T1** : Hot path — drafter court (≤ 10 mots) → verifier skip.
#[tokio::test]
async fn s7_hot_path_skips_verifier_when_drafter_short_offline() {
    let reg = ProviderRegistry::new();
    // Drafter répond en 3 mots "Paris is capital".
    reg.register(Arc::new(
        MockLlmProvider::new("gemini_drafter").with_response("Paris is capital"),
    ));
    // Verifier ne devrait JAMAIS être appelé.
    reg.register(Arc::new(
        MockLlmProvider::new("claude_verifier")
            .with_response("Paris is the capital of France long answer"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "Capital of France?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude_verifier".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "gemini_drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 10, // hot path activé
    };
    let resp = run_speculative(&coord, req).await.unwrap();

    assert!(resp.verifier_skipped, "S7 hot path : verifier_skipped=true");
    assert!(resp.draft_accepted, "draft accepté");
    assert_eq!(resp.final_text, "Paris is capital", "draft retourné tel quel");
    assert_eq!(resp.tokens_regenerated, 0);
}

/// **S7-T2** : Slow path — drafter long (> 10 mots) → verifier appelé.
#[tokio::test]
async fn s7_slow_path_calls_verifier_when_drafter_long_offline() {
    let reg = ProviderRegistry::new();
    let long_draft =
        "Paris is the beautiful capital of France with monuments and history exceeding many cities around the world today";
    reg.register(Arc::new(
        MockLlmProvider::new("gemini_drafter").with_response(long_draft),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("claude_verifier")
            .with_response("Claude validated Paris response"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude_verifier".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "gemini_drafter".to_string(),
        accept_threshold: 0.9, // strict → reject + verifier text retained
        max_words_for_quick_accept: 10,
    };
    let resp = run_speculative(&coord, req).await.unwrap();

    assert!(!resp.verifier_skipped, "S7 slow path : verifier_skipped=false");
    // Jaccard faible → on garde le verifier_text.
    assert_eq!(resp.final_text, "Claude validated Paris response");
}

/// **S7-T3** : `chewing_metrics` rempli avec les nouveaux champs S7.
#[tokio::test]
async fn s7_chewing_metrics_filled_for_spec_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("gemini_drafter").with_response("short"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("claude_verifier").with_response("short"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "ping".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "claude_verifier".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "gemini_drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 10,
    };
    let resp = run_speculative(&coord, req).await.unwrap();

    let metrics = resp.chewing_metrics.expect("chewing_metrics doit être Some(...)");
    assert_eq!(metrics.verifier_skipped, Some(true));
    assert!(metrics.input_tokens_raw >= 1);
    assert_eq!(metrics.spec_verifier_latency_ms, 0, "verifier skipped → 0");
    assert!(
        metrics.router_reason.contains("hot path"),
        "router_reason doit mentionner 'hot path', got: {}",
        metrics.router_reason
    );
}

/// **S7-T4** : `verifier_skipped` propagé dans la response + chewing_metrics.
#[tokio::test]
async fn s7_verifier_skipped_propagated_to_response_offline() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("d").with_response("ok"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("v").with_response("ok"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "v".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "d".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 5,
    };
    let resp = run_speculative(&coord, req).await.unwrap();

    assert!(resp.verifier_skipped);
    assert_eq!(
        resp.chewing_metrics.as_ref().and_then(|m| m.verifier_skipped),
        Some(true),
        "chewing_metrics doit refléter verifier_skipped"
    );
}

/// **S7-T5** : `spec_savings_cents` > 0 quand verifier skipped + verifier cher.
#[tokio::test]
async fn s7_spec_savings_cents_above_zero_when_skip_offline() {
    let reg = ProviderRegistry::new();
    // Drafter répond court → hot path.
    reg.register(Arc::new(
        MockLlmProvider::new("d").with_response("Paris"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("v").with_response("Paris"),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    // Verifier metadata avec coût significatif pour produire savings > 0.
    let expensive_verifier_meta = LlmMetadata {
        cost_input_per_million: 3000,  // Claude-style
        cost_output_per_million: 15000,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        ..meta()
    };
    let req = SpeculativeRequest {
        user_text: "A".repeat(2000), // ~500 tokens input estimés
        task_class: TaskClass::FactualQa,
        verifier_llm: "v".into(),
        verifier_metadata: expensive_verifier_meta,
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 256,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "d".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 5,
    };
    let resp = run_speculative(&coord, req).await.unwrap();
    assert!(resp.verifier_skipped);
    let savings = resp
        .chewing_metrics
        .as_ref()
        .map(|m| m.spec_savings_cents)
        .unwrap_or(0);
    assert!(
        savings >= 1,
        "savings_cents >= 1 attendu pour verifier cher + skipped, got {savings}"
    );
}

/// **S7-T6** : Helper `with_drafter_and_verifier` produit une request valide.
#[tokio::test]
async fn s7_with_drafter_and_verifier_helper_offline() {
    // Le helper utilise la table S6 → verifier_metadata pré-rempli.
    let req = SpeculativeRequest::with_drafter_and_verifier(
        "test".to_string(),
        TaskClass::Code,
        "gemini_gemini-2.5-flash".to_string(),
        "claude_claude-sonnet-4-6".to_string(),
    );
    assert_eq!(req.drafter_llm, "gemini_gemini-2.5-flash");
    assert_eq!(req.verifier_llm, "claude_claude-sonnet-4-6");
    assert_eq!(req.accept_threshold, 0.7);
    assert_eq!(req.max_words_for_quick_accept, 10);
    // Le verifier_metadata doit avoir une capability CODE remplie via S6.
    assert!(req.verifier_metadata.capability_scores.contains_key("CODE"));
}

// ─── Sprint S7 R2 — Patches chirurgicaux post-audit ─────────────────────
// Ces 3 tests comblent les gaps identifiés par l'audit chirurgical post-S7 :
// - S7-T7 : backward-compat SpeculativeResponse JSON sans cost_cents/chewing/verifier_skipped
// - S7-T8 : cost_cents observé hot path = drafter seul
// - S7-T9 : cost_cents observé slow path = drafter + verifier (preuve quantitative)

/// **S7-T7** : Backward-compat — un JSON `SpeculativeResponse` pré-S7
/// (sans `cost_cents`, `verifier_skipped`, `chewing_metrics`) doit
/// désérialiser correctement avec defaults serde.
#[test]
fn s7_speculative_response_backward_compat_pre_s7_json() {
    // JSON typique S2-S6 (sans les champs S7).
    let old_json = r#"{
        "final_text": "Paris",
        "draft_text": "Paris",
        "draft_accepted": true,
        "tokens_accepted": 1,
        "tokens_regenerated": 0,
        "latency_saved_ms": 0,
        "latency_ms": 500,
        "audit_chain_hash": "abc"
    }"#;
    let parsed: efe_coordinator::SpeculativeResponse = serde_json::from_str(old_json)
        .expect("backward-compat #[serde(default)] doit fonctionner");
    // Defaults S7 :
    assert!(!parsed.verifier_skipped, "default false");
    assert!(parsed.chewing_metrics.is_none(), "default None");
    assert_eq!(parsed.cost_cents, 0, "default 0");
    // Le reste préservé :
    assert_eq!(parsed.final_text, "Paris");
    assert_eq!(parsed.tokens_accepted, 1);
}

/// **S7-T8** : cost_cents observé hot path = `drafter.cost_cents` seul.
///
/// Drafter à 1¢/call, Verifier à 10¢/call → hot path doit produire
/// `resp.cost_cents == 1` (verifier non facturé).
#[tokio::test]
async fn s7_cost_cents_hot_path_equals_drafter_only() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("d_cheap")
            .with_response("Paris") // 1 mot → hot path
            .with_cost_cents(1),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("v_expensive")
            .with_response("Paris is capital")
            .with_cost_cents(10),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "v_expensive".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "d_cheap".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 10,
    };
    let resp = run_speculative(&coord, req).await.unwrap();

    assert!(resp.verifier_skipped, "hot path");
    assert_eq!(
        resp.cost_cents, 1,
        "hot path → cost = drafter (1¢), verifier NON facturé"
    );
}

/// **S7-T9** : cost_cents observé slow path = `drafter + verifier`.
///
/// Drafter à 1¢, Verifier à 10¢, hot path désactivé (`max_words=0`)
/// → `resp.cost_cents == 11`. Preuve QUANTITATIVE du savings = 10¢ par
/// query évitée.
#[tokio::test]
async fn s7_cost_cents_slow_path_equals_drafter_plus_verifier() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("d_cheap")
            .with_response("Paris is capital")
            .with_cost_cents(1),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("v_expensive")
            .with_response("Paris is the capital")
            .with_cost_cents(10),
    ));
    let coord = Coordinator::builder().registry(reg).build().unwrap();

    let req = SpeculativeRequest {
        user_text: "?".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "v_expensive".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 50,
        max_total_latency: Duration::from_secs(5),
        drafter_llm: "d_cheap".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 0, // hot path désactivé
    };
    let resp = run_speculative(&coord, req).await.unwrap();

    assert!(!resp.verifier_skipped, "slow path");
    assert_eq!(
        resp.cost_cents,
        11,
        "slow path → cost = drafter(1) + verifier(10) = 11¢. \
         Savings du hot path quantifié : 10¢ par query évitée."
    );
}
