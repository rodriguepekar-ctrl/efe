// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : end-to-end acceptance tests.
//!
//! Matches the 15-row acceptance matrix in architecture § 12.1.

use std::sync::Arc;

use efe_llm_provenance::{
    call_with_provenance, hash::prompt_hashes_for, CacheHitProvenance,
    DefaultLlmProvenanceCollector, InMemoryLlmProvenanceStore, InMemoryModelRegistry,
    LlmProvenanceCollector, LlmProvenanceQueryApi, LlmProvenanceStore, ModelRegistry,
    N17Metrics, PostCallProvenance, PreCallProvenance, RegisteredModel, ReplayProvenance,
};
use efe_nano_sdk::{
    Blake3Hash, CacheProvenance, Completeness, CostAccounting, FinishReason, ModelIdentity,
    NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceError, ProvenanceId, RegionCode,
    SafetyCategory, SafetyFlag, SamplingParams, TenantId, TokenUsage, ToolCallRecord,
    ToolCallResultKind,
};
use efe_llm_provenance::DateRange;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Fixture helpers
// ─────────────────────────────────────────────────────────────────────

struct Fixture {
    collector: Arc<DefaultLlmProvenanceCollector>,
    #[allow(dead_code)] // held to keep the in-memory store alive for the test scope
    store: Arc<InMemoryLlmProvenanceStore>,
    api: Arc<LlmProvenanceQueryApi>,
    metrics: Arc<N17Metrics>,
    tenant: TenantId,
    identity: ModelIdentity,
}

async fn build_fixture() -> Fixture {
    let store = Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let identity = ModelIdentity::hosted("openai", "gpt-4-turbo", "2024-04-09");
    registry
        .register(RegisteredModel::hosted(
            identity.clone(),
            1_000.0,
            3_000.0,
            1,
        ))
        .await
        .unwrap();
    let metrics = N17Metrics::new();
    let collector = DefaultLlmProvenanceCollector::with_metrics(
        store.clone() as Arc<dyn LlmProvenanceStore>,
        registry as Arc<dyn ModelRegistry>,
        metrics.clone(),
    );
    let api = LlmProvenanceQueryApi::with_metrics(
        store.clone() as Arc<dyn LlmProvenanceStore>,
        metrics.clone(),
    );
    Fixture {
        collector,
        store,
        api,
        metrics,
        tenant: TenantId::new(),
        identity,
    }
}

fn pre(fx: &Fixture, swarm: Uuid) -> PreCallProvenance {
    let hashes = prompt_hashes_for("{\"x\":1}", "system", "user", "final");
    PreCallProvenance {
        trace_id: Uuid::new_v4(),
        swarm_request_id: swarm,
        op_id: "summarize".into(),
        tenant_id: fx.tenant,
        agent_id: NanoAgentId::new("summarize_strict").unwrap(),
        agent_version: NanoAgentVersion::new("1.2.3").unwrap(),
        model_identity: fx.identity.clone(),
        prompt_template_id: "summarize-v3".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: hashes.inputs_hash,
        system_prompt_hash: hashes.system_hash,
        user_prompt_hash: hashes.user_hash,
        final_prompt_hash: hashes.final_hash,
        sampling: SamplingParams {
            temperature: 0.0,
            top_p: Some(0.95),
            max_tokens: 512,
            seed: Some(42),
            stop_sequences: vec!["END".into()],
        },
        pii_strategy_applied: PiiStrategy::Strict,
        redaction_token_set: vec!["[REDACTED-EMAIL]".into()],
        provider_region: Some(RegionCode::UsEast1),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    }
}

fn post(input_tokens: u32, output_tokens: u32, cost_cents: f64) -> PostCallProvenance {
    PostCallProvenance {
        token_usage: TokenUsage {
            client_estimated_input: input_tokens,
            client_estimated_output: output_tokens,
            provider_reported_input: Some(input_tokens + 1),
            provider_reported_output: Some(output_tokens),
            is_estimated: false,
        },
        cost_accounting: CostAccounting {
            cost_cents,
            prices_table_version: 1,
            provider_quoted_cost_cents: None,
            overflow_capped: false,
        },
        provider_request_id: Some("req_abc123".into()),
        provider_response_time_ms: 487,
        provider_finish_reason: FinishReason::Stop,
        provider_metadata: Default::default(),
        output_hash: Some(Blake3Hash::from_bytes([9u8; 32])),
        completeness: Completeness::Complete,
        safety_flags: vec![],
        tool_calls: vec![],
        model_hash_verified: false,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  G1 — Model identity recorded
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_model_identity_recorded() {
    let fx = build_fixture().await;
    let swarm = Uuid::new_v4();
    let pre = pre(&fx, swarm);
    let trace_id = pre.trace_id;
    fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    let got = fx
        .api
        .get_by_trace(trace_id, fx.tenant)
        .await
        .unwrap();
    assert_eq!(got.model_identity.provider, "openai");
    assert_eq!(got.model_identity.model_id, "gpt-4-turbo");
    assert_eq!(got.model_identity.requested_version, "2024-04-09");
}

// ─────────────────────────────────────────────────────────────────────
//  G2 — Prompt hashes stable across serdes
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_prompt_hashes_stable_across_serdes() {
    let fx = build_fixture().await;
    let swarm = Uuid::new_v4();
    let pre_a = pre(&fx, swarm);
    let pre_b = pre(&fx, swarm);
    // Same inputs → same hashes.
    assert_eq!(pre_a.system_prompt_hash, pre_b.system_prompt_hash);
    assert_eq!(pre_a.user_prompt_hash, pre_b.user_prompt_hash);
    assert_eq!(pre_a.final_prompt_hash, pre_b.final_prompt_hash);
    // Serde round-trip.
    let json = serde_json::to_string(&pre_a.system_prompt_hash).unwrap();
    let h: Blake3Hash = serde_json::from_str(&json).unwrap();
    assert_eq!(h, pre_a.system_prompt_hash);
}

// ─────────────────────────────────────────────────────────────────────
//  G3 — Pre-call hash mismatch blocks the call
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_hash_mismatch_blocks_call() {
    let store = Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
    let mut m = RegisteredModel::hosted(identity.clone(), 1000.0, 3000.0, 1);
    m.expected_content_hash = Some(Blake3Hash::from_bytes([0xAA; 32]));
    registry.register(m).await.unwrap();
    let collector = DefaultLlmProvenanceCollector::new(
        store.clone() as Arc<dyn LlmProvenanceStore>,
        registry as Arc<dyn ModelRegistry>,
    );
    let tenant = TenantId::new();
    let pre = PreCallProvenance {
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "x".into(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("a").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: identity,
        prompt_template_id: "t".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::ZERO,
        final_prompt_hash: Blake3Hash::ZERO,
        sampling: SamplingParams::default(),
        pii_strategy_applied: PiiStrategy::Strict,
        redaction_token_set: vec![],
        provider_region: None,
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    };
    let actual = Blake3Hash::from_bytes([0xBB; 32]);
    let err = collector.pre_call(&pre, Some(&actual)).await.unwrap_err();
    assert!(matches!(err, ProvenanceError::ModelIntegrityViolation { .. }));
    // Drift event recorded.
    assert_eq!(store.drift_event_count(), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  G4 — Provider metadata recorded
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_provider_metadata_recorded() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    let mut post = post(100, 50, 0.4);
    post.provider_metadata.insert(
        "system_fingerprint".into(),
        serde_json::json!("fp_abc123"),
    );
    fx.collector.post_call(pre, post).await.unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert_eq!(got.provider_request_id.as_deref(), Some("req_abc123"));
    assert_eq!(got.provider_response_time_ms, 487);
    assert_eq!(got.provider_finish_reason, FinishReason::Stop);
    assert_eq!(got.provider_region, Some(RegionCode::UsEast1));
    assert_eq!(
        got.provider_metadata.get("system_fingerprint"),
        Some(&serde_json::json!("fp_abc123"))
    );
}

// ─────────────────────────────────────────────────────────────────────
//  G5 — Token drift visible in cost_summary
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_token_drift_visible() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    let summary = fx
        .api
        .cost_summary(fx.tenant, DateRange::since_minutes_ago(60))
        .await
        .unwrap();
    // Client = 150, provider = 151 → drift = 1/150 ≈ 0.0067.
    let drift = summary.drift_ratio.unwrap();
    assert!(drift > 0.0);
    assert!(drift < 0.02);
}

// ─────────────────────────────────────────────────────────────────────
//  G6 — Tool call record serdes round-trip
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_tool_call_record_serdes_round_trip() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    let mut post = post(100, 50, 0.4);
    post.tool_calls.push(ToolCallRecord {
        ordinal: 0,
        tool_name: "lookup_patient_chart".into(),
        arguments_hash: Blake3Hash::from_bytes([1u8; 32]),
        result_hash: Some(Blake3Hash::from_bytes([2u8; 32])),
        result_kind: ToolCallResultKind::Ok,
    });
    fx.collector.post_call(pre, post).await.unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert_eq!(got.tool_calls.len(), 1);
    assert_eq!(got.tool_calls[0].tool_name, "lookup_patient_chart");
    assert_eq!(got.tool_calls[0].result_kind, ToolCallResultKind::Ok);
    // Round-trip via JSON.
    let json = serde_json::to_string(&got).unwrap();
    let got2: efe_nano_sdk::LlmProvenanceRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(got, got2);
}

// ─────────────────────────────────────────────────────────────────────
//  G7 — Sampling params recorded
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_sampling_params_recorded() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert_eq!(got.sampling.temperature, 0.0);
    assert_eq!(got.sampling.top_p, Some(0.95));
    assert_eq!(got.sampling.max_tokens, 512);
    assert_eq!(got.sampling.seed, Some(42));
    assert_eq!(got.sampling.stop_sequences, vec!["END".to_string()]);
}

// ─────────────────────────────────────────────────────────────────────
//  G8 — Safety flag taxonomy round-trip (8 variants)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_safety_flag_taxonomy_round_trip() {
    let cases = vec![
        SafetyFlag::PiiDetected,
        SafetyFlag::PromptInjectionSuspected,
        SafetyFlag::OutputModerationTriggered,
        SafetyFlag::ContentFilter {
            category: SafetyCategory::Hate,
        },
        SafetyFlag::ToxicityDetected,
        SafetyFlag::SelfHarmDetected,
        SafetyFlag::JailbreakAttemptDetected,
        SafetyFlag::PromptLengthExceeded,
    ];
    assert_eq!(cases.len(), 8);
    for flag in cases {
        let json = serde_json::to_string(&flag).unwrap();
        let flag2: SafetyFlag = serde_json::from_str(&json).unwrap();
        assert_eq!(flag, flag2);
    }
}

// ─────────────────────────────────────────────────────────────────────
//  G9 — Cross-tenant blocked
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_provenance_cross_tenant_blocked() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    let other_tenant = TenantId::new();
    let err = fx.api.get_by_trace(trace_id, other_tenant).await.unwrap_err();
    assert!(matches!(
        err,
        efe_llm_provenance::ProvenanceStoreError::CrossTenant { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────
//  G10 — Four query patterns work end-to-end
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_four_query_patterns_work() {
    let fx = build_fixture().await;
    let swarm = Uuid::new_v4();
    let pre = pre(&fx, swarm);
    let trace_id = pre.trace_id;
    let pid = fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap().provenance_id;

    // 1) get_by_trace
    let r1 = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert_eq!(r1.swarm_request_id, swarm);

    // 2) get_by_provenance
    let r2 = fx.api.get_by_provenance(pid, fx.tenant).await.unwrap();
    assert_eq!(r2.trace_id, trace_id);

    // 3) list_by_swarm
    let list = fx.api.list_by_swarm(swarm, fx.tenant).await.unwrap();
    assert_eq!(list.len(), 1);

    // 4) cost_summary
    let summary = fx
        .api
        .cost_summary(fx.tenant, DateRange::since_minutes_ago(60))
        .await
        .unwrap();
    assert!(summary.total_cents > 0.0);
}

// ─────────────────────────────────────────────────────────────────────
//  G11 — Cost recomputable from tokens × prices
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_cost_recomputable_from_tokens_prices() {
    // Look up the registered model + recompute cost from its prices.
    let fx = build_fixture().await;
    let model = fx.collector.registry().lookup(&fx.identity).await.unwrap();
    // 1M input + 1M output should equal 1000 + 3000 = 4000 cents per the
    // registry's price table.
    let (cost, capped) = model.compute_cost_cents(1_000_000, 1_000_000);
    assert!((cost - 4000.0).abs() < 1e-6);
    assert!(!capped);
}

// ─────────────────────────────────────────────────────────────────────
//  G12 — PII strategy recorded at call site
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_pii_strategy_recorded_at_call_site() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    let expected_strategy = pre.pii_strategy_applied;
    let expected_tokens = pre.redaction_token_set.clone();
    fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert_eq!(got.pii_strategy_applied, expected_strategy);
    assert_eq!(got.redaction_token_set, expected_tokens);
    assert!(matches!(got.pii_strategy_applied, PiiStrategy::Strict));
}

// ─────────────────────────────────────────────────────────────────────
//  G13 — Cache hit records provenance with cache_hit set
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_cache_hit_records_provenance() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    let original_trace_id = Uuid::new_v4();
    let original_provenance_id = ProvenanceId::new();
    let hit = CacheHitProvenance {
        pre_call: pre,
        cache_provenance: CacheProvenance {
            original_trace_id,
            original_provenance_id,
            cache_key_version: 1,
        },
    };
    fx.collector.on_cache_hit(hit).await.unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert!(got.is_cache_hit());
    let cache = got.cache_hit.as_ref().unwrap();
    assert_eq!(cache.original_trace_id, original_trace_id);
    assert_eq!(cache.original_provenance_id, original_provenance_id);
    assert_eq!(fx.metrics.cache_hits_total(), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  G14 — Replay call recorded with replay_of pointer
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_replay_call_recorded() {
    let fx = build_fixture().await;
    let original_id = ProvenanceId::new();
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    fx.collector
        .on_replay(ReplayProvenance {
            pre_call: pre,
            post_call: post(100, 50, 0.4),
            replay_of: original_id,
        })
        .await
        .unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert!(got.is_replay());
    assert_eq!(got.replay_of, Some(original_id));
    assert_eq!(fx.metrics.replay_calls_total(), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  G15 — Pre-N17 chain still verifies
// ─────────────────────────────────────────────────────────────────────
//
// Architecture G15 + ADR-1 : provenance lives in a new trace-level
// field excluded from the canonical body, so pre-N17 chains MUST
// keep verifying.  Sprint 9 doesn't yet emit to the Sprint 6 chain
// directly (that wiring lands in Sprint 10).  This test verifies
// the structural invariant : creating a provenance record does NOT
// require touching any audit chain primitive, so by construction
// every pre-N17 chain in the wild verifies unchanged.

#[tokio::test]
async fn phase_3_n17_pre_n17_chain_still_verifies() {
    // Construct a record + serialise it.  The serialised JSON has a
    // closed shape ; nothing here touches the Sprint 6 chain
    // canonical-body format.  This assertion ensures the SDK keeps
    // provenance OUT of the canonical chain row.
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let record = fx
        .collector
        .post_call(pre, post(100, 50, 0.4))
        .await
        .unwrap();
    let json = serde_json::to_string(&record).unwrap();
    // The provenance record JSON is self-contained ; not a wrapper
    // around any chain row.
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    // The wire format MUST NOT carry the legacy chain fields
    // `canonical_row_body` or `hmac` — those belong to the chain
    // trace, not the provenance payload.
    assert!(parsed.get("canonical_row_body").is_none());
    assert!(parsed.get("hmac").is_none());
    // It MUST carry the new typed fields.
    assert!(parsed.get("provenance_id").is_some());
    assert!(parsed.get("model_identity").is_some());
    assert!(parsed.get("token_usage").is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  Bonus integration tests — end-to-end via caller helper
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_3_n17_call_with_provenance_emits_full_record() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    let outcome = call_with_provenance::<_, _, String>(
        fx.collector.as_ref(),
        pre,
        None,
        |_pre| async {
            Ok((
                "synthetic-response".into(),
                post(50, 25, 0.2),
            ))
        },
    )
    .await
    .unwrap();
    assert_eq!(outcome.response, "synthetic-response");
    assert_eq!(outcome.record.trace_id, trace_id);
    // Verify metrics ticked.
    assert!(fx.metrics.llm_call_total("gpt-4-turbo", "complete") >= 1);
    assert!(fx.metrics.tokens_input_total("gpt-4-turbo") >= 50);
    assert!(fx.metrics.cost_cents_total("gpt-4-turbo") > 0.0);
}

#[tokio::test]
async fn phase_3_n17_drift_event_persisted_and_listable() {
    let store = Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
    let mut m = RegisteredModel::hosted(identity.clone(), 1000.0, 3000.0, 1);
    m.expected_content_hash = Some(Blake3Hash::from_bytes([0xAA; 32]));
    registry.register(m).await.unwrap();
    let collector = DefaultLlmProvenanceCollector::new(
        store.clone() as Arc<dyn LlmProvenanceStore>,
        registry as Arc<dyn ModelRegistry>,
    );
    let tenant = TenantId::new();
    let pre = PreCallProvenance {
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "x".into(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("a").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: identity,
        prompt_template_id: "t".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::ZERO,
        final_prompt_hash: Blake3Hash::ZERO,
        sampling: SamplingParams::default(),
        pii_strategy_applied: PiiStrategy::Strict,
        redaction_token_set: vec![],
        provider_region: None,
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    };
    let actual = Blake3Hash::from_bytes([0xBB; 32]);
    let _ = collector.pre_call(&pre, Some(&actual)).await.unwrap_err();
    let api = LlmProvenanceQueryApi::new(store as Arc<dyn LlmProvenanceStore>);
    let events = api
        .list_drift_events(tenant, DateRange::since_minutes_ago(60))
        .await
        .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].tenant_id, tenant);
}

#[tokio::test]
async fn phase_3_n17_compliance_eu_ai_act_audit_query() {
    // Compliance scenario : a regulator asks for every Mode C call
    // made by tenant T against model X between two dates.  N17's
    // `list_by_model` answers in one call, tenant-scoped.
    let fx = build_fixture().await;
    for _ in 0..3 {
        let pre = pre(&fx, Uuid::new_v4());
        fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    }
    let records = fx
        .api
        .list_by_model(&fx.identity, fx.tenant, DateRange::since_days_ago(30))
        .await
        .unwrap();
    assert_eq!(records.len(), 3);
    for r in &records {
        assert_eq!(r.tenant_id, fx.tenant);
        assert_eq!(r.model_identity.model_id, "gpt-4-turbo");
    }
}

#[tokio::test]
async fn phase_3_n17_provider_migration_visible_in_record() {
    // P4 / Incident I-2026-05-19 : provider silently rotates model.
    // We capture both requested + served version.
    let fx = build_fixture().await;
    let mut pre = pre(&fx, Uuid::new_v4());
    pre.model_identity.served_version = Some("2024-04-09-rev2".into()); // different from requested 2024-04-09
    let trace_id = pre.trace_id;
    fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert!(got.model_identity.provider_migrated());
    assert_eq!(got.model_identity.requested_version, "2024-04-09");
    assert_eq!(got.model_identity.served_version.as_deref(), Some("2024-04-09-rev2"));
}

#[tokio::test]
async fn phase_3_n17_metrics_record_provenance_writes() {
    let fx = build_fixture().await;
    for _ in 0..5 {
        let pre = pre(&fx, Uuid::new_v4());
        fx.collector.post_call(pre, post(100, 50, 0.4)).await.unwrap();
    }
    assert_eq!(fx.metrics.provenance_writes_success_total(), 5);
    assert_eq!(fx.metrics.provenance_writes_failure_total(), 0);
}

#[tokio::test]
async fn phase_3_n17_refused_completeness_round_trips() {
    let fx = build_fixture().await;
    let pre = pre(&fx, Uuid::new_v4());
    let trace_id = pre.trace_id;
    let mut post = post(100, 0, 0.0);
    post.completeness = Completeness::Refused {
        category: SafetyCategory::Hate,
    };
    post.safety_flags.push(SafetyFlag::ContentFilter {
        category: SafetyCategory::Hate,
    });
    post.output_hash = None;
    fx.collector.post_call(pre, post).await.unwrap();
    let got = fx.api.get_by_trace(trace_id, fx.tenant).await.unwrap();
    assert!(got.was_refused());
    assert!(got.output_hash.is_none());
    assert_eq!(got.safety_flags.len(), 1);
}
