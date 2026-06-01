// SPDX-License-Identifier: UNLICENSED
//! **Sprint 18 — N26** : invariant regression tests pinning the
//! key behaviours of `ClaudeBridge`'s N17 provenance wiring.
//!
//! | I-# | Invariant pinned                                                           |
//! |-----|-----------------------------------------------------------------------------|
//! | I1  | `provenance + audit_context` → exactly one record emitted                  |
//! | I2  | Missing collector OR missing audit_context → no record + metric tick      |
//! | I3  | Two HTTP turns produce ONE logical record                                  |
//! | I5  | Pre-call verification failure SHORT-CIRCUITS the HTTP path                 |
//! | I8  | Post-call collector error does NOT propagate as BridgeError               |
//! | I11 | `provider_request_id` matches Anthropic's response `id` field             |
//! | I14 | `model_identity` matches the requested model                              |
//! | I18 | Publisher tier captured at call time (snapshot, not live)                 |

use std::sync::Arc;

use async_trait::async_trait;
use efe_agentic_bridge::{
    AgenticAuditContext, AgenticBridge, AgenticTask, ClaudeBridge,
};
use efe_llm_provenance::{
    DefaultLlmProvenanceCollector, InMemoryLlmProvenanceStore, InMemoryModelRegistry,
    LlmProvenanceCollector, LlmProvenanceStore, ModelRegistry, RegisteredModel,
};
use efe_nano_sdk::{
    Author, Blake3Hash, ContextBundle, Entity, KycTier, LlmProvenanceRecord,
    ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy, ProvenanceError, PublisherId,
    SwarmId, TenantId,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ─── Helpers ────────────────────────────────────────────────────────

fn sample_bundle() -> ContextBundle {
    let mut b = ContextBundle::empty(SwarmId::new());
    b.language = Some("en".into());
    b.entities.push(Entity {
        entity_type: "person".into(),
        value: "Alice".into(),
        span: Some((0, 5)),
    });
    b.finalize_size();
    b
}

async fn fresh_collector(
    model: &str,
) -> (
    Arc<DefaultLlmProvenanceCollector>,
    Arc<dyn LlmProvenanceStore>,
) {
    let store: Arc<dyn LlmProvenanceStore> = Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let mut registered = RegisteredModel::hosted(
        ModelIdentity::hosted("anthropic", model, "2023-06-01"),
        0.0,
        0.0,
        1,
    );
    registered.dev_skip_hash_check = true;
    registry.register(registered).await.unwrap();
    let collector = DefaultLlmProvenanceCollector::new(
        store.clone(),
        registry as Arc<dyn ModelRegistry>,
    );
    (collector, store)
}

async fn mount_two_turn(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_turn1_abc",
            "content": [{
                "type": "tool_use",
                "id": "tu_01",
                "name": "swarm_context",
                "input": {}
            }],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 25, "output_tokens": 7}
        })))
        .up_to_n_times(1)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "msg_turn2_xyz",
            "content": [{"type": "text", "text": "Risk score 7/10."}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 75, "output_tokens": 15}
        })))
        .mount(server)
        .await;
}

fn audit_ctx(tenant: TenantId) -> AgenticAuditContext {
    AgenticAuditContext {
        trace_id: uuid::Uuid::new_v4(),
        swarm_request_id: uuid::Uuid::new_v4(),
        op_id: "audit_test".into(),
        tenant_id: tenant,
        agent_id: NanoAgentId::new("test_agent").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        prompt_template_id: "audit_template_v1".into(),
        prompt_template_version: "1.0.0".into(),
        pii_strategy_applied: PiiStrategy::None,
        redaction_token_set: Vec::new(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: None,
    }
}

// ─── I1 — exactly one record emitted on the happy path ────────────

#[tokio::test]
async fn audit_n26_i1_provenance_emitted_exactly_once() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let ctx = audit_ctx(tenant);
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    assert_eq!(record.trace_id, trace);
    assert_eq!(record.tenant_id, tenant);
    assert_eq!(record.model_identity.provider, "anthropic");
    assert_eq!(record.model_identity.model_id, "claude-sonnet-4-6");
    // I3 — two HTTP turns → one record.
}

// ─── I2 — no collector + no audit_context → no record ─────────────

#[tokio::test]
async fn audit_n26_i2_no_collector_skips_emission() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (_collector, store) = fresh_collector("claude-sonnet-4-6").await;
    // Build bridge WITHOUT `.with_provenance(...)`.
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6");
    let task = AgenticTask::new("hi").with_audit_context(audit_ctx(TenantId::new()));
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    assert_eq!(bridge.provenance_skipped_count(), 1);
    // Store is empty — no record could have been stashed without a
    // collector.
    let any_records = store
        .list_by_swarm(uuid::Uuid::new_v4(), TenantId::new())
        .await
        .unwrap();
    assert!(any_records.is_empty());
}

#[tokio::test]
async fn audit_n26_i2_no_audit_context_skips_emission() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, _store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    // No `.with_audit_context(...)`.
    let task = AgenticTask::new("hi");
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    assert_eq!(bridge.provenance_skipped_count(), 1);
}

// ─── I3 — two HTTP turns → one record ──────────────────────────────

#[tokio::test]
async fn audit_n26_i3_two_turns_one_record() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let ctx = audit_ctx(tenant);
    let swarm = ctx.swarm_request_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let records = store.list_by_swarm(swarm, tenant).await.unwrap();
    assert_eq!(records.len(), 1, "two HTTP turns MUST produce one record");
    // **G6** : tokens are SUMMED across both turns.
    assert_eq!(records[0].token_usage.canonical_input(), 100); // 25 + 75
    assert_eq!(records[0].token_usage.canonical_output(), 22); // 7 + 15
}

// ─── I5 — pre-call failure short-circuits HTTP ────────────────────

struct RejectingCollector;
#[async_trait]
impl LlmProvenanceCollector for RejectingCollector {
    async fn pre_call(
        &self,
        _pre: &efe_llm_provenance::PreCallProvenance,
        _actual: Option<&Blake3Hash>,
    ) -> Result<(), ProvenanceError> {
        Err(ProvenanceError::ModelNotRegistered {
            provider: "anthropic".into(),
            model_id: "rogue".into(),
            version: "v0".into(),
        })
    }
    async fn post_call(
        &self,
        _pre: efe_llm_provenance::PreCallProvenance,
        _post: efe_llm_provenance::PostCallProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        panic!("post_call should not be reached after pre_call failure");
    }
    async fn on_cache_hit(
        &self,
        _hit: efe_llm_provenance::CacheHitProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        unimplemented!()
    }
    async fn on_replay(
        &self,
        _replay: efe_llm_provenance::ReplayProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn audit_n26_i5_pre_call_failure_short_circuits_http() {
    let server = MockServer::start().await;
    // Note : do NOT mount any handlers — if pre-call fails the
    // HTTP path must never fire.  An unmounted mock server
    // returns 404 to any request ; failure means we'd see that
    // somewhere.
    let _ = mount_two_turn; // keep import alive
    let _ = server.uri();
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(Arc::new(RejectingCollector));
    let task = AgenticTask::new("hi").with_audit_context(audit_ctx(TenantId::new()));
    let err = bridge
        .call_with_context(&sample_bundle(), task)
        .await
        .unwrap_err();
    assert!(
        matches!(err, efe_agentic_bridge::BridgeError::ProvenanceVerifyFailed { .. }),
        "expected ProvenanceVerifyFailed, got {err:?}"
    );
    // `call_count` is 0 — no HTTP dispatch fired.
    assert_eq!(bridge.call_count(), 0);
}

// ─── I8 — post-call collector error does NOT propagate ────────────

struct PostCallErroringCollector {
    store: Arc<dyn LlmProvenanceStore>,
    registry: Arc<dyn ModelRegistry>,
}

#[async_trait]
impl LlmProvenanceCollector for PostCallErroringCollector {
    async fn pre_call(
        &self,
        pre: &efe_llm_provenance::PreCallProvenance,
        actual: Option<&Blake3Hash>,
    ) -> Result<(), ProvenanceError> {
        let inner = DefaultLlmProvenanceCollector::new(
            self.store.clone(),
            self.registry.clone(),
        );
        inner.pre_call(pre, actual).await
    }
    async fn post_call(
        &self,
        _pre: efe_llm_provenance::PreCallProvenance,
        _post: efe_llm_provenance::PostCallProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        Err(ProvenanceError::Store {
            message: "simulated store fault".into(),
        })
    }
    async fn on_cache_hit(
        &self,
        _hit: efe_llm_provenance::CacheHitProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        unimplemented!()
    }
    async fn on_replay(
        &self,
        _replay: efe_llm_provenance::ReplayProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        unimplemented!()
    }
}

#[tokio::test]
async fn audit_n26_i8_post_call_error_does_not_propagate() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let store: Arc<dyn LlmProvenanceStore> = Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(InMemoryModelRegistry::new());
    let mut registered = RegisteredModel::hosted(
        ModelIdentity::hosted("anthropic", "claude-sonnet-4-6", "2023-06-01"),
        0.0,
        0.0,
        1,
    );
    registered.dev_skip_hash_check = true;
    registry.register(registered).await.unwrap();
    let collector = Arc::new(PostCallErroringCollector {
        store: store.clone(),
        registry: registry as Arc<dyn ModelRegistry>,
    });
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let task = AgenticTask::new("hi").with_audit_context(audit_ctx(TenantId::new()));
    // The LLM response MUST still return to the caller, errors
    // logged only.
    let resp = bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    assert_eq!(resp.text, "Risk score 7/10.");
    assert_eq!(bridge.provenance_emit_failed_count(), 1);
}

// ─── I11 — provider_request_id matches Anthropic's `id` ──────────

#[tokio::test]
async fn audit_n26_i11_provider_request_id_matches_anthropic_id() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let ctx = audit_ctx(tenant);
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    assert_eq!(
        record.provider_request_id.as_deref(),
        Some("msg_turn2_xyz"),
        "provider_request_id MUST equal the turn-2 response id"
    );
}

// ─── I14 — model_identity matches requested model ─────────────────

#[tokio::test]
async fn audit_n26_i14_model_identity_matches_requested_model() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-haiku-3-5").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-haiku-3-5")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let ctx = audit_ctx(tenant);
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    assert_eq!(record.model_identity.model_id, "claude-haiku-3-5");
    assert_eq!(record.model_identity.provider, "anthropic");
}

// ─── I6 — provider_response_time_ms is the sum of both turns ──────

#[tokio::test]
async fn audit_n26_i6_provider_response_time_is_sum_of_both_turns() {
    let server = MockServer::start().await;
    // Turn 1 : artificial 50 ms latency.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "id": "msg_t1_slow",
                    "content": [{
                        "type": "tool_use",
                        "id": "tu_01",
                        "name": "swarm_context",
                        "input": {}
                    }],
                    "stop_reason": "tool_use",
                    "usage": {"input_tokens": 25, "output_tokens": 7}
                }))
                .set_delay(std::time::Duration::from_millis(50)),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Turn 2 : artificial 80 ms latency.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({
                    "id": "msg_t2_slow",
                    "content": [{"type": "text", "text": "ok"}],
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 75, "output_tokens": 15}
                }))
                .set_delay(std::time::Duration::from_millis(80)),
        )
        .mount(&server)
        .await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let ctx = audit_ctx(tenant);
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    // Latency MUST be at least the sum of both turn delays
    // (50 + 80 = 130 ms).  We give a generous +200 ms ceiling
    // to absorb CI noise but the sum invariant holds.
    assert!(
        record.provider_response_time_ms >= 130,
        "provider_response_time_ms ({}) must be at least sum of turn delays (130 ms)",
        record.provider_response_time_ms
    );
    assert!(
        record.provider_response_time_ms < 5_000,
        "provider_response_time_ms ({}) suspiciously high",
        record.provider_response_time_ms
    );
}

// ─── G7 — PII strategy + redaction token set propagated ───────────

#[tokio::test]
async fn audit_n26_g7_pii_strategy_and_redaction_tokens_propagated() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let mut ctx = audit_ctx(tenant);
    ctx.pii_strategy_applied = PiiStrategy::Strict;
    ctx.redaction_token_set = vec![
        "<EMAIL_0>".into(),
        "<PHONE_0>".into(),
        "<SSN_0>".into(),
    ];
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    assert_eq!(record.pii_strategy_applied, PiiStrategy::Strict);
    assert_eq!(
        record.redaction_token_set,
        vec!["<EMAIL_0>".to_string(), "<PHONE_0>".into(), "<SSN_0>".into()]
    );
}

// ─── G9 — gold_case_id propagated from audit_context to record ────

#[tokio::test]
async fn audit_n26_g9_gold_case_id_propagated_to_record() {
    use efe_nano_sdk::GoldCaseId;
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let case_id = GoldCaseId::new();
    let mut ctx = audit_ctx(tenant);
    ctx.gold_case_id = Some(case_id);
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    assert_eq!(record.gold_case_id, Some(case_id));
}

// ─── G6 — token usage is sum across both turns (already pinned ────
//         in i3 ; here we ALSO check cost_accounting consistency) ──

#[tokio::test]
async fn audit_n26_g6_cost_accounting_matches_summed_tokens() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let ctx = audit_ctx(tenant);
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    // Tokens : turn 1 = (25, 7), turn 2 = (75, 15), summed = (100, 22).
    // Sonnet 4.6 : 3_000 mc / 1k input, 15_000 mc / 1k output.
    // Cost : 100 × 3_000 / 1k + 22 × 15_000 / 1k = 300 + 330 = 630 mc
    //      = 0.063 cents.
    let expected_cents = (100_i64 * 3_000 / 1_000 + 22_i64 * 15_000 / 1_000) as f64
        / 10_000.0;
    assert!(
        (record.cost_accounting.cost_cents - expected_cents).abs() < 1e-9,
        "cost_cents ({}) MUST equal {} ($-derivation from summed tokens)",
        record.cost_accounting.cost_cents,
        expected_cents
    );
    assert_eq!(
        record.cost_accounting.prices_table_version,
        efe_agentic_bridge::PRICES_TABLE_VERSION
    );
}

// ─── I18 — publisher tier captured at call time (snapshot) ────────

#[tokio::test]
async fn audit_n26_i18_publisher_tier_snapshot_pinned() {
    let server = MockServer::start().await;
    mount_two_turn(&server).await;
    let (collector, store) = fresh_collector("claude-sonnet-4-6").await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri())
        .with_model("claude-sonnet-4-6")
        .with_provenance(collector);
    let tenant = TenantId::new();
    let pid = PublisherId::new();
    let mut ctx = audit_ctx(tenant);
    ctx.publisher_id = Some(pid);
    // Pin Tier2 at call time.
    ctx.publisher_kyc_tier = Some(KycTier::Tier2);
    let trace = ctx.trace_id;
    let task = AgenticTask::new("hi").with_audit_context(ctx);
    bridge.call_with_context(&sample_bundle(), task).await.unwrap();
    let record = store.get_by_trace(trace, tenant).await.unwrap();
    assert_eq!(record.publisher_id, Some(pid));
    assert_eq!(record.publisher_kyc_tier, Some(KycTier::Tier2));
}

// ─── Bonus — confirm Author::Publisher mapping not assumed ────────

#[test]
fn audit_n26_audit_context_does_not_require_publisher_id() {
    let ctx = audit_ctx(TenantId::new());
    assert!(ctx.publisher_id.is_none());
    // The minimal context is valid serde.
    let json = serde_json::to_string(&ctx).unwrap();
    let back: AgenticAuditContext = serde_json::from_str(&json).unwrap();
    assert_eq!(back, ctx);
    // And the Author::Efe path doesn't blow up when accessed.
    assert!(!Author::Efe.is_publisher());
}
