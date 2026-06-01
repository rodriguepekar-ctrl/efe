//! **Sprint 8 — N14** — end-to-end Swarm Envelope tests.
//!
//! Validates the full N14 envelope mechanism at the
//! `Dispatcher → AuditWriter → ChainStore → ReplayEngine` boundary.
//!
//! See `docs/architecture/EFE_M50_N14_SWARM_ENVELOPE_TRACE_ARCHITECTURE.md`.

use std::sync::Arc;
use uuid::Uuid;

use efe_nano::audit::chain_store::{ChainStore, InMemoryChainStore};
use efe_nano::audit::hkdf::HmacKeySource;
use efe_nano::audit::hmac_chain::InMemoryHmacChainAuditWriter;
use efe_nano::audit::stub::InMemoryAuditWriter;
use efe_nano::audit::{AuditWriter, ErasureReason, TombstoneMetadata};
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::envelope::{
    compute_request_hash, EnvelopeBuilder, SWARM_ENVELOPE_AGENT_ID,
    SWARM_ENVELOPE_EXECUTION_MODE, TRACE_KIND_SWARM_ENVELOPE,
};
use efe_nano::dispatcher::Dispatcher;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::replay::ReplayEngine;
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::starter_pack::register_all_starter_pack;
use efe_nano_sdk::{
    Catalog, EnvelopeKind, NanoAgentId, NanoAgentRef, OpStatus, Operation, OperationInput,
    RetryPolicy, SwarmRequest, SwarmStatus, TenantId,
};

// ─────────────────────────────────────────────────────────────────────
//  Harness
// ─────────────────────────────────────────────────────────────────────

#[allow(dead_code)]
struct Harness {
    dispatcher: Arc<Dispatcher>,
    audit: Arc<InMemoryAuditWriter>,
    chain_store: Arc<InMemoryChainStore>,
    key_source: HmacKeySource,
    catalog: Arc<InMemoryCatalog>,
    runtime: Arc<dyn efe_nano::runtime::NanoAgentExecutor>,
}

async fn make_harness() -> Harness {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry).await.unwrap();
    let runtime: Arc<dyn efe_nano::runtime::NanoAgentExecutor> =
        Arc::new(PureRustRuntime::new(registry));
    let audit = Arc::new(InMemoryAuditWriter::new());
    let idempotency = Arc::new(IdempotencyCache::new());
    let dispatcher = Arc::new(Dispatcher::new(
        Arc::clone(&catalog) as Arc<dyn Catalog>,
        Arc::clone(&runtime),
        Arc::clone(&audit) as Arc<dyn AuditWriter>,
        idempotency,
    ));
    let chain_store = Arc::new(InMemoryChainStore::new());
    let key_source = HmacKeySource::test_key();
    Harness {
        dispatcher,
        audit,
        chain_store,
        key_source,
        catalog,
        runtime,
    }
}

/// Harness with the HMAC chain audit writer wired (replay-ready).
struct ChainHarness {
    dispatcher: Arc<Dispatcher>,
    chain_store: Arc<InMemoryChainStore>,
    key_source: HmacKeySource,
    catalog: Arc<InMemoryCatalog>,
}

async fn make_chain_harness() -> ChainHarness {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry).await.unwrap();
    let runtime = Arc::new(PureRustRuntime::new(registry));
    let chain_store = Arc::new(InMemoryChainStore::new());
    let key_source = HmacKeySource::test_key();
    let writer = Arc::new(InMemoryHmacChainAuditWriter::new(
        Arc::clone(&chain_store),
        key_source.clone(),
    ));
    let idempotency = Arc::new(IdempotencyCache::new());
    let dispatcher = Arc::new(Dispatcher::with_pure_rust_runtime(
        Arc::clone(&catalog) as Arc<dyn Catalog>,
        runtime,
        writer as Arc<dyn AuditWriter>,
        idempotency,
    ));
    ChainHarness {
        dispatcher,
        chain_store,
        key_source,
        catalog,
    }
}

fn lit(v: serde_json::Value) -> OperationInput {
    OperationInput::Literal { value: v }
}

fn op(id: &str, agent_id: &str, deps: &[&str], input: OperationInput) -> Operation {
    Operation {
        op_id: id.to_string(),
        agent: NanoAgentRef::new(NanoAgentId::new(agent_id).unwrap()),
        input,
        depends_on: deps.iter().map(|s| s.to_string()).collect(),
        timeout_ms: None,
        retry: RetryPolicy::default(),
    }
}

fn request(tenant: TenantId, ops: Vec<Operation>) -> SwarmRequest {
    SwarmRequest {
        request_id: Uuid::new_v4(),
        tenant_id: tenant,
        caller_agent_id: Some("n14-test".into()),
        operations: ops,
        max_parallelism: 4,
        timeout_ms: 30_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["n14".into()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

fn validate_input() -> serde_json::Value {
    serde_json::json!({"schema": {"type": "object"}, "value": {"x": 1}})
}

// ─────────────────────────────────────────────────────────────────────
//  C1 — Envelope round-trip
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c1_envelope_written_for_successful_swarm() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let traces = h.audit.traces().await;
    let envelopes: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .collect();
    assert_eq!(envelopes.len(), 1, "exactly one envelope per swarm");
    let env = envelopes[0];
    assert_eq!(env.nano_agent_id.as_str(), SWARM_ENVELOPE_AGENT_ID);
    assert_eq!(env.execution_mode, SWARM_ENVELOPE_EXECUTION_MODE);
    assert!(env.full_input.is_some());
    assert!(env.full_output.is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  C2 — Op traces carry envelope id
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c2_op_traces_carry_envelope_id() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let resp = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![
                op("a", "json_validate", &[], lit(validate_input())),
                op("b", "json_validate", &[], lit(validate_input())),
                op("c", "json_validate", &[], lit(validate_input())),
            ],
        ))
        .await
        .unwrap();
    let envelope_id = resp.swarm_envelope_trace_id.expect("response has envelope id");
    let traces = h.audit.traces().await;
    let op_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    assert_eq!(op_traces.len(), 3);
    for t in &op_traces {
        assert_eq!(
            t.swarm_envelope_trace_id,
            Some(envelope_id),
            "op trace {} must reference envelope",
            t.operation_id
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
//  C3 — Envelope captures the request signature
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c3_envelope_captures_request_signature() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let req = SwarmRequest {
        request_id: Uuid::new_v4(),
        tenant_id: tenant,
        caller_agent_id: Some("specific-caller".into()),
        operations: vec![op("a", "json_validate", &[], lit(validate_input()))],
        max_parallelism: 7,
        timeout_ms: 12_345,
        idempotency_key: Some("idemp-key-x".into()),
        streaming: true,
        audit_tags: vec!["tag-1".into(), "tag-2".into()],
        cost_cap_cents: Some(0.5),
        parent_envelope_trace_id: None,
    };
    let _ = h.dispatcher.dispatch(req.clone()).await.unwrap();
    let traces = h.audit.traces().await;
    let env = traces
        .iter()
        .find(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .unwrap();
    let payload: efe_nano_sdk::SwarmEnvelopePayload =
        serde_json::from_value(env.full_input.clone().unwrap()).unwrap();
    assert_eq!(payload.swarm_request_id, req.request_id);
    assert_eq!(payload.caller_agent_id.as_deref(), Some("specific-caller"));
    assert_eq!(payload.idempotency_key.as_deref(), Some("idemp-key-x"));
    assert_eq!(payload.max_parallelism, 7);
    assert_eq!(payload.timeout_ms, 12_345);
    assert_eq!(payload.cost_cap_cents, Some(0.5));
    assert!(payload.streaming);
    assert_eq!(payload.audit_tags, vec!["tag-1", "tag-2"]);
    assert_eq!(payload.op_ids, vec!["a"]);
    assert_eq!(payload.op_count, 1);
}

// ─────────────────────────────────────────────────────────────────────
//  C4 — Envelope captures the outcome
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c4_envelope_captures_outcome() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![
                op("a", "json_validate", &[], lit(validate_input())),
                op("b", "json_validate", &[], lit(validate_input())),
            ],
        ))
        .await
        .unwrap();
    let traces = h.audit.traces().await;
    let env = traces
        .iter()
        .find(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .unwrap();
    let payload: efe_nano_sdk::SwarmEnvelopePayload =
        serde_json::from_value(env.full_input.clone().unwrap()).unwrap();
    assert_eq!(payload.overall_status, SwarmStatus::Success);
    assert_eq!(payload.op_status_counts.success, 2);
    assert_eq!(payload.op_status_counts.failed, 0);
    assert_eq!(payload.envelope_format_version, 1);
    assert_eq!(payload.envelope_kind, EnvelopeKind::Terminal);
}

// ─────────────────────────────────────────────────────────────────────
//  C5 — Pre-N14 HMAC chain still verifies
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c5_chain_with_envelope_still_verifies() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let report = efe_nano::audit::verify::verify_chain(
        &*h.chain_store,
        &h.key_source,
        tenant,
        None,
    )
    .await
    .unwrap();
    // 1 op + 1 envelope = 2 entries.
    assert_eq!(report.entries_verified, 2);
}

// ─────────────────────────────────────────────────────────────────────
//  C6 — Replay filter excludes SwarmEnvelope
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c6_replay_filters_swarm_envelope() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let mut req = request(
        tenant,
        vec![op("v", "json_validate", &[], lit(validate_input()))],
    );
    req.request_id = request_id;
    let _ = h.dispatcher.dispatch(req).await.unwrap();

    let engine = ReplayEngine::new(
        Arc::clone(&h.chain_store),
        h.key_source.clone(),
        Arc::clone(&h.dispatcher),
        Arc::clone(&h.catalog) as Arc<dyn Catalog>,
    );
    let report = engine.replay(tenant, request_id).await.unwrap();
    // Only the op is replayed ; envelope is filtered out.
    assert_eq!(report.total_ops, 1);
    assert_eq!(report.matched_ops, 1);
}

// ─────────────────────────────────────────────────────────────────────
//  C7 — Tombstone envelope cascades to op traces
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c7_tombstone_envelope_cascades_to_op_traces() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let mut req = request(
        tenant,
        vec![
            op("a", "json_validate", &[], lit(validate_input())),
            op("b", "json_validate", &[], lit(validate_input())),
        ],
    );
    req.request_id = request_id;
    let _ = h.dispatcher.dispatch(req).await.unwrap();

    // Find the envelope trace_id.
    let traces = h.chain_store.traces_for_swarm(tenant, request_id).await.unwrap();
    let envelope = traces
        .iter()
        .find(|e| e.trace.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .unwrap();
    let envelope_trace_id = envelope.trace.trace_id.0;

    // Cascade tombstone the envelope.
    let meta = TombstoneMetadata::new(
        chrono::Utc::now(),
        ErasureReason::GdprArticle17,
        None,
        "test",
        [0u8; 32],
    );
    let outcome = h
        .chain_store
        .tombstone_swarm_envelope(tenant, envelope_trace_id, meta)
        .await
        .unwrap();
    // 2 ops + 1 envelope = 3 traces all tombstoned.
    assert_eq!(outcome.traces_tombstoned, 3);
}

// ─────────────────────────────────────────────────────────────────────
//  C8 — Cache-hit traces carry envelope id
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn c8_cache_hit_traces_carry_envelope_id() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let ops = vec![op("v", "json_validate", &[], lit(validate_input()))];
    let r1 = h.dispatcher.dispatch(request(tenant, ops.clone())).await.unwrap();
    let r2 = h.dispatcher.dispatch(request(tenant, ops)).await.unwrap();
    let env1 = r1.swarm_envelope_trace_id.unwrap();
    let env2 = r2.swarm_envelope_trace_id.unwrap();
    assert_ne!(env1, env2);
    let traces = h.audit.traces().await;
    let cache_hit = traces
        .iter()
        .find(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .unwrap();
    // The cache-hit's envelope id is the SECOND swarm's envelope id
    // (the one in the current swarm context).
    assert_eq!(cache_hit.swarm_envelope_trace_id, Some(env2));
}

// ─────────────────────────────────────────────────────────────────────
//  Replay envelope_for_swarm
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn envelope_for_swarm_round_trip() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let mut req = request(
        tenant,
        vec![op("v", "json_validate", &[], lit(validate_input()))],
    );
    req.request_id = request_id;
    req.caller_agent_id = Some("specific-caller".into());
    let _ = h.dispatcher.dispatch(req).await.unwrap();

    let engine = ReplayEngine::new(
        Arc::clone(&h.chain_store),
        h.key_source.clone(),
        Arc::clone(&h.dispatcher),
        Arc::clone(&h.catalog) as Arc<dyn Catalog>,
    );
    let payload = engine.envelope_for_swarm(tenant, request_id).await.unwrap();
    assert_eq!(payload.swarm_request_id, request_id);
    assert_eq!(payload.caller_agent_id.as_deref(), Some("specific-caller"));
}

#[tokio::test]
async fn envelope_for_swarm_not_found_for_unknown_id() {
    let h = make_chain_harness().await;
    let engine = ReplayEngine::new(
        Arc::clone(&h.chain_store),
        h.key_source.clone(),
        Arc::clone(&h.dispatcher),
        Arc::clone(&h.catalog) as Arc<dyn Catalog>,
    );
    let r = engine
        .envelope_for_swarm(TenantId::system(), Uuid::new_v4())
        .await;
    assert!(matches!(
        r,
        Err(efe_nano::error::ReplayError::NotFound(_))
            | Err(efe_nano::error::ReplayError::EnvelopeNotFound(_))
    ));
}

// ─────────────────────────────────────────────────────────────────────
//  Response carries envelope id
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn response_carries_swarm_envelope_trace_id() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let resp = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    assert!(resp.swarm_envelope_trace_id.is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  Empty swarm still emits envelope
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn empty_swarm_still_emits_envelope() {
    // The planner rejects empty swarms with InvalidDag, but **the
    // envelope is still emitted** (N14 architecture § 6.3) so the
    // attempt is auditable.  Test : dispatch errors out, envelope row
    // appears in the audit log.
    let h = make_harness().await;
    let tenant = TenantId::system();
    let r = h.dispatcher.dispatch(request(tenant, vec![])).await;
    assert!(r.is_err(), "planner rejects empty swarms");
    let traces = h.audit.traces().await;
    let envs: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .collect();
    assert_eq!(envs.len(), 1, "planner-failure envelope must be emitted");
}

// ─────────────────────────────────────────────────────────────────────
//  Cycle / planner failure still emits envelope
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn planner_failure_still_emits_envelope() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    // Build a cycle a → b → a.
    let req = request(
        tenant,
        vec![
            op("a", "json_validate", &["b"], lit(validate_input())),
            op("b", "json_validate", &["a"], lit(validate_input())),
        ],
    );
    let err = h.dispatcher.dispatch(req).await.unwrap_err();
    let _ = err;
    // Envelope was still emitted for forensics.
    let traces = h.audit.traces().await;
    let envs: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .collect();
    assert_eq!(envs.len(), 1, "planner-failure envelope must be emitted");
}

// ─────────────────────────────────────────────────────────────────────
//  Request hash is deterministic
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn request_hash_deterministic_across_dispatches() {
    let req = request(
        TenantId::system(),
        vec![op("v", "json_validate", &[], lit(validate_input()))],
    );
    let h1 = compute_request_hash(&req);
    let h2 = compute_request_hash(&req);
    assert_eq!(h1, h2);
}

// ─────────────────────────────────────────────────────────────────────
//  Catalog rejects publishes with the reserved envelope id
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn catalog_rejects_publish_with_reserved_envelope_id() {
    use efe_nano_sdk::{
        AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
        IdempotencyPolicy, NanoAgent, NanoAgentVersion, ResourceLimits, StabilityTier,
    };
    let catalog = InMemoryCatalog::new();
    let agent = NanoAgent {
        id: NanoAgentId::new(SWARM_ENVELOPE_AGENT_ID).unwrap(),
        version: NanoAgentVersion::new("0.0.0").unwrap(),
        description: "bogus".into(),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "envelope::run".into(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.0001,
        latency_p99_ms_target: 100,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: chrono::Utc::now(),
    };
    let r = catalog.publish(TenantId::system(), agent).await;
    assert!(
        r.is_err(),
        "reserved envelope agent id must be rejected by catalog"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Builder unit-level test for finalize aggregation accuracy
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn envelope_builder_finalize_correctly_aggregates() {
    use efe_nano::dispatcher::OperationOutcome;
    let req = request(
        TenantId::system(),
        vec![
            op("a", "json_validate", &[], lit(validate_input())),
            op("b", "json_validate", &[], lit(validate_input())),
        ],
    );
    let envelope_id = efe_nano_sdk::TraceId::new();
    let b = EnvelopeBuilder::new(envelope_id, &req);
    // **B1 fix** : op_status_counts is now computed from response.results.
    let mut results = std::collections::HashMap::new();
    for id in ["a", "b"] {
        results.insert(
            id.to_string(),
            efe_nano_sdk::OperationResult {
                op_id: id.to_string(),
                status: OpStatus::Success,
                output: None,
                error: None,
                latency_ms: 50,
                cost_cents: 0.25,
            },
        );
    }
    let resp = efe_nano_sdk::SwarmResponse {
        request_id: req.request_id,
        overall_status: SwarmStatus::Success,
        results,
        total_latency_ms: 100,
        total_cost_cents: 0.5,
        swarm_envelope_trace_id: None,
    };
    let mut outcomes = std::collections::HashMap::new();
    for id in ["a", "b"] {
        let mut o = OperationOutcome::pending(id);
        o.status = OpStatus::Success;
        outcomes.insert(id.to_string(), o);
    }
    let payload = b.finalize(&resp, &outcomes);
    assert_eq!(payload.op_status_counts.success, 2);
    assert!((payload.total_cost_cents - 0.5).abs() < 1e-9);
    assert_eq!(payload.overall_status, SwarmStatus::Success);
}

// ═════════════════════════════════════════════════════════════════════
//  Audit fix regression tests (B1–B9)
// ═════════════════════════════════════════════════════════════════════

// ─────────────────────────────────────────────────────────────────────
//  B1 regression — op_status_counts from response.results, not outcomes
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b1_planner_failure_envelope_op_status_counts_match_op_count() {
    // **I10 invariant** : op_status_counts.total() == op_count.
    // Pre-fix : op_status_counts was all zeros on planner failure (outcomes empty).
    // Post-fix : op_status_counts is computed from response.results.
    let h = make_harness().await;
    let tenant = TenantId::system();
    let req = request(
        tenant,
        vec![
            op("a", "json_validate", &["c"], lit(validate_input())),
            op("b", "json_validate", &["a"], lit(validate_input())),
            op("c", "json_validate", &["b"], lit(validate_input())),
        ],
    );
    let _ = h.dispatcher.dispatch(req).await.unwrap_err();
    let traces = h.audit.traces().await;
    let envelope = traces
        .iter()
        .find(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .expect("planner failure still emits envelope");
    let payload: efe_nano_sdk::SwarmEnvelopePayload =
        serde_json::from_value(envelope.full_input.clone().unwrap()).unwrap();
    // I10 : counts sum equals op_count.
    assert_eq!(payload.op_count, 3);
    assert_eq!(payload.op_status_counts.total(), 3);
    // All 3 ops were Skipped (synthesized by planner_failure_response).
    assert_eq!(payload.op_status_counts.skipped, 3);
}

// ─────────────────────────────────────────────────────────────────────
//  B2 regression — shadow_run_count + canary_routed_count wired
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b2_envelope_shadow_run_count_increments_with_shadow_policy() {
    use efe_nano::audit::drift::DriftWindow;
    use efe_nano::dispatcher::shadow::ShadowScheduler;
    use efe_nano::runtime::cost_attribution::CostRouter;
    use efe_nano_sdk::{
        AuditPolicy, Author, Category, ComparisonStrategy, ComplianceTags, CostAttribution,
        DeterminismLevel, ExecutionMode, IdempotencyPolicy, NanoAgent, NanoAgentVersion,
        ResourceLimits, RolloutPolicy, StabilityTier,
    };

    let h = make_harness().await;
    let tenant = TenantId::system();
    // Publish a v1.1.0 for shadow.
    let mut shadow_agent = NanoAgent {
        id: NanoAgentId::new("json_validate").unwrap(),
        version: NanoAgentVersion::new("1.1.0").unwrap(),
        description: "v1.1.0 shadow".into(),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust { entry: "json_validate::run".into() },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.001,
        latency_p99_ms_target: 100,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: chrono::Utc::now(),
    };
    h.catalog.publish(tenant, shadow_agent.clone()).await.unwrap();
    shadow_agent.version = NanoAgentVersion::new("1.0.0").unwrap();
    // (v1.0.0 already exists from starter pack.)

    h.catalog
        .set_rollout_policy(
            tenant,
            &NanoAgentId::new("json_validate").unwrap(),
            RolloutPolicy::Shadow {
                prod_version: NanoAgentVersion::new("1.0.0").unwrap(),
                shadow_version: NanoAgentVersion::new("1.1.0").unwrap(),
                started_at: chrono::Utc::now(),
                comparison: ComparisonStrategy::Exact,
                cost_attribution: CostAttribution::Platform,
                shadow_budget_cents: None,
                shadow_on_cache_hit: true,
            },
        )
        .await
        .unwrap();
    // Build a dispatcher with shadow scheduler, reusing the harness runtime.
    let audit_arc: Arc<dyn AuditWriter> = Arc::clone(&h.audit) as Arc<dyn AuditWriter>;
    let drift = Arc::new(DriftWindow::default_for_sprint_8());
    let cost_router = Arc::new(CostRouter::unlimited());
    let scheduler = Arc::new(ShadowScheduler::new(
        Arc::clone(&h.runtime),
        Arc::clone(&audit_arc),
        drift,
        cost_router,
    ));
    let dispatcher = Arc::new(
        Dispatcher::new(
            Arc::clone(&h.catalog) as Arc<dyn Catalog>,
            Arc::clone(&h.runtime),
            Arc::clone(&audit_arc),
            Arc::new(IdempotencyCache::new()),
        )
        .with_shadow_scheduler(scheduler),
    );
    let resp = dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    // Give shadow time to land.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let envelope_id = resp.swarm_envelope_trace_id.unwrap();
    let traces = h.audit.traces().await;
    let env = traces.iter().find(|t| t.trace_id == envelope_id).unwrap();
    let payload: efe_nano_sdk::SwarmEnvelopePayload =
        serde_json::from_value(env.full_input.clone().unwrap()).unwrap();
    // **B2** : the envelope's shadow_run_count must be 1 (one op + shadow policy).
    assert_eq!(payload.shadow_run_count, 1, "B2: shadow_run_count must be 1");
}

#[tokio::test]
async fn b2_envelope_canary_routed_count_increments() {
    use efe_nano_sdk::{
        AuditPolicy, Author, Category, ComplianceTags, CostAttribution, DeterminismLevel,
        ExecutionMode, IdempotencyPolicy, NanoAgent, NanoAgentVersion, PromotionGates,
        ResourceLimits, RolloutPolicy, StabilityTier,
    };
    let h = make_harness().await;
    let tenant = TenantId::system();
    let canary_agent = NanoAgent {
        id: NanoAgentId::new("json_validate").unwrap(),
        version: NanoAgentVersion::new("1.1.0").unwrap(),
        description: "v1.1.0 canary".into(),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust { entry: "json_validate::run".into() },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: serde_json::json!({"type": "object"}),
        output_schema: serde_json::json!({"type": "object"}),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.001,
        latency_p99_ms_target: 100,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: chrono::Utc::now(),
    };
    h.catalog.publish(tenant, canary_agent).await.unwrap();
    // 100 % canary so every input routes to v1.1.0.
    h.catalog
        .set_rollout_policy(
            tenant,
            &NanoAgentId::new("json_validate").unwrap(),
            RolloutPolicy::Canary {
                prod_version: NanoAgentVersion::new("1.0.0").unwrap(),
                canary_version: NanoAgentVersion::new("1.1.0").unwrap(),
                canary_bps: 10_000,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();
    let resp = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![
                op("a", "json_validate", &[], lit(validate_input())),
                op("b", "json_validate", &[], lit(validate_input())),
            ],
        ))
        .await
        .unwrap();
    let envelope_id = resp.swarm_envelope_trace_id.unwrap();
    let traces = h.audit.traces().await;
    let env = traces.iter().find(|t| t.trace_id == envelope_id).unwrap();
    let payload: efe_nano_sdk::SwarmEnvelopePayload =
        serde_json::from_value(env.full_input.clone().unwrap()).unwrap();
    // **B2** : both ops routed to canary (canary_bps = 10_000 = 100 %).
    assert_eq!(payload.canary_routed_count, 2);
}

// ─────────────────────────────────────────────────────────────────────
//  B3 regression — EnvelopeMetrics 8 surfaces tick
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b3_envelope_metrics_tick_on_write() {
    use efe_nano::dispatcher::envelope::EnvelopeMetrics;
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry).await.unwrap();
    let runtime = Arc::new(PureRustRuntime::new(registry));
    let audit = Arc::new(InMemoryAuditWriter::new());
    let metrics = Arc::new(EnvelopeMetrics::new());
    let dispatcher = Dispatcher::with_pure_rust_runtime(
        Arc::clone(&catalog) as Arc<dyn Catalog>,
        runtime,
        Arc::clone(&audit) as Arc<dyn AuditWriter>,
        Arc::new(IdempotencyCache::new()),
    )
    .with_envelope_metrics(Arc::clone(&metrics));
    let _ = dispatcher
        .dispatch(request(
            TenantId::system(),
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let snap = metrics.snapshot();
    assert_eq!(snap.envelopes_written_total, 1);
    assert!(snap.envelope_op_count_sum >= 1);
    assert!(snap.envelope_size_bytes_sum > 0);
    assert!(snap.envelope_size_bytes_count > 0);
}

#[tokio::test]
async fn b3_envelope_metrics_tick_on_query() {
    use efe_nano::dispatcher::envelope::EnvelopeMetrics;
    let h = make_chain_harness().await;
    let metrics = Arc::new(EnvelopeMetrics::new());
    // Reuse harness dispatcher's catalog ; build a fresh registry +
    // runtime so the dispatcher can be reconstructed with metrics.
    // The chain harness exposes its catalog ; we just need a runtime.
    let registry = HandlerRegistry::new();
    // Starter pack already registered by make_chain_harness for catalog ;
    // we just need the handlers in the registry, so re-register and
    // ignore the catalog-collision error.
    let _ = register_all_starter_pack(&*h.catalog, &registry).await;
    let runtime: Arc<dyn efe_nano::runtime::NanoAgentExecutor> =
        Arc::new(PureRustRuntime::new(registry));
    let audit_writer = Arc::new(efe_nano::audit::hmac_chain::InMemoryHmacChainAuditWriter::new(
        Arc::clone(&h.chain_store),
        h.key_source.clone(),
    ));
    let dispatcher = Arc::new(
        Dispatcher::new(
            Arc::clone(&h.catalog) as Arc<dyn Catalog>,
            runtime,
            audit_writer as Arc<dyn AuditWriter>,
            Arc::new(IdempotencyCache::new()),
        )
        .with_envelope_metrics(Arc::clone(&metrics)),
    );
    let request_id = Uuid::new_v4();
    let mut req = request(
        TenantId::system(),
        vec![op("v", "json_validate", &[], lit(validate_input()))],
    );
    req.request_id = request_id;
    let _ = dispatcher.dispatch(req).await.unwrap();
    let engine = ReplayEngine::new(
        Arc::clone(&h.chain_store),
        h.key_source.clone(),
        Arc::clone(&dispatcher),
        Arc::clone(&h.catalog) as Arc<dyn Catalog>,
    );
    let _ = engine
        .envelope_for_swarm(TenantId::system(), request_id)
        .await
        .unwrap();
    let snap = metrics.snapshot();
    assert_eq!(snap.envelopes_queried_total, 1);
}

// ─────────────────────────────────────────────────────────────────────
//  I10 invariant — op_status_counts.total() == op_count
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn i10_op_status_counts_sum_equals_op_count() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![
                op("a", "json_validate", &[], lit(validate_input())),
                op("b", "json_validate", &[], lit(validate_input())),
                op("c", "json_validate", &[], lit(validate_input())),
            ],
        ))
        .await
        .unwrap();
    let traces = h.audit.traces().await;
    let env = traces
        .iter()
        .find(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .unwrap();
    let p: efe_nano_sdk::SwarmEnvelopePayload =
        serde_json::from_value(env.full_input.clone().unwrap()).unwrap();
    assert_eq!(p.op_status_counts.total(), p.op_count);
}

// ─────────────────────────────────────────────────────────────────────
//  I14 invariant — tombstoned envelope's full_input == None
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn i14_tombstoned_envelope_full_input_is_none() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let mut req = request(
        tenant,
        vec![op("v", "json_validate", &[], lit(validate_input()))],
    );
    req.request_id = request_id;
    let _ = h.dispatcher.dispatch(req).await.unwrap();

    let traces = h.chain_store.traces_for_swarm(tenant, request_id).await.unwrap();
    let envelope = traces
        .iter()
        .find(|e| e.trace.error_kind.as_deref() == Some(TRACE_KIND_SWARM_ENVELOPE))
        .unwrap();
    let envelope_trace_id = envelope.trace.trace_id.0;
    let meta = TombstoneMetadata::new(
        chrono::Utc::now(),
        ErasureReason::GdprArticle17,
        None,
        "test",
        [0u8; 32],
    );
    h.chain_store
        .tombstone_swarm_envelope(tenant, envelope_trace_id, meta)
        .await
        .unwrap();
    // After tombstone : full_input MUST be None on the envelope.
    let post = h
        .chain_store
        .trace_by_id(tenant, envelope_trace_id)
        .await
        .unwrap();
    assert!(post.trace.full_input.is_none(), "I14 : tombstoned envelope full_input must be None");
    assert!(post.trace.is_tombstoned());
}

// ─────────────────────────────────────────────────────────────────────
//  I17 invariant — envelope is part of the Merkle epoch
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn i17_envelope_is_in_merkle_epoch() {
    use efe_nano::audit::{
        build_root, EpochSealer, EpochSealerConfig, InMemoryEpochRootStore,
    };
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    // 1 op + 1 envelope = 2 entries → epoch_size = 2 to seal one epoch.
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap();
    assert_eq!(chain.len(), 2);
    let leaves: Vec<Vec<u8>> = chain.iter().map(|e| e.payload_hash.to_vec()).collect();
    let expected_root = build_root(&leaves).unwrap();
    let epoch_store = Arc::new(InMemoryEpochRootStore::new());
    let sealer = EpochSealer::new(
        Arc::clone(&h.chain_store),
        Arc::clone(&epoch_store),
        Arc::new(h.key_source.clone()),
        EpochSealerConfig { epoch_size_traces: 2 },
    );
    let sealed = sealer.seal_pending_epochs(tenant).await.unwrap();
    assert_eq!(sealed.len(), 1);
    assert_eq!(
        sealed[0].root_hash, expected_root,
        "I17 : envelope MUST be in the epoch's Merkle root"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  I18 — RolloutPolicyChanged virtual rows have swarm_envelope_trace_id == None
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn i18_rollout_policy_changed_has_no_envelope_anchor() {
    use efe_nano::dispatcher::shadow::build_rollout_policy_changed_trace;
    use efe_nano_sdk::{NanoAgentVersion, PromotionGates, RolloutPolicy};
    let from = RolloutPolicy::Stable {
        version: NanoAgentVersion::new("1.0.0").unwrap(),
    };
    let to = RolloutPolicy::Canary {
        prod_version: NanoAgentVersion::new("1.0.0").unwrap(),
        canary_version: NanoAgentVersion::new("1.1.0").unwrap(),
        canary_bps: 500,
        started_at: chrono::Utc::now(),
        promotion_gates: PromotionGates::default(),
        cost_attribution: efe_nano_sdk::CostAttribution::Tenant,
    };
    let trace = build_rollout_policy_changed_trace(
        TenantId::system(),
        NanoAgentId::new("agent").unwrap(),
        &from,
        &to,
        "operator:test",
        None,
    );
    // I18 : virtual-swarm rows MUST have swarm_envelope_trace_id == None.
    assert!(trace.swarm_envelope_trace_id.is_none());
}

// ─────────────────────────────────────────────────────────────────────
//  Canonical body excludes swarm_envelope_trace_id (HMAC compat)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn canonical_row_body_excludes_swarm_envelope_trace_id() {
    use efe_nano::audit::{payload::canonical_row_body, NanoAgentTrace};
    use efe_nano_sdk::{NanoAgentVersion, TraceId};

    fn t(swarm_envelope: Option<TraceId>) -> NanoAgentTrace {
        NanoAgentTrace {
            trace_id: TraceId::new(),
            parent_trace_ids: vec![],
            swarm_request_id: Uuid::new_v4(),
            operation_id: "v".into(),
            tenant_id: TenantId::system(),
            caller_agent_id: None,
            nano_agent_id: NanoAgentId::new("test").unwrap(),
            nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            execution_mode: "pure_rust".into(),
            input_hash_source: [0u8; 32],
            input_hash_resolved: [0u8; 32],
            output_hash: None,
            error_kind: None,
            started_at: chrono::Utc::now(),
            completed_at: None,
            latency_ms: Some(1),
            cost_cents: 0.0,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: swarm_envelope,
        }
    }
    // Build two traces with everything identical EXCEPT the envelope id field.
    let trace_id = TraceId::new();
    let mut a = t(None);
    let mut b = t(Some(TraceId::new()));
    // Force identical trace_id + uuids so the canonical bodies match.
    a.trace_id = trace_id;
    b.trace_id = trace_id;
    a.swarm_request_id = b.swarm_request_id;
    a.started_at = b.started_at;
    let body_a = canonical_row_body(&a);
    let body_b = canonical_row_body(&b);
    assert_eq!(
        body_a, body_b,
        "canonical_row_body MUST exclude swarm_envelope_trace_id (HMAC compat)"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Meta-swarm — parent_envelope_trace_id propagation
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn meta_swarm_parent_envelope_propagated_to_child() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    // Parent swarm.
    let parent_resp = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("p", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let parent_envelope_id = parent_resp.swarm_envelope_trace_id.unwrap();

    // Child swarm with parent reference.
    let mut child_req = request(
        tenant,
        vec![op("c", "json_validate", &[], lit(validate_input()))],
    );
    child_req.parent_envelope_trace_id = Some(parent_envelope_id);
    let child_resp = h.dispatcher.dispatch(child_req).await.unwrap();
    let child_envelope_id = child_resp.swarm_envelope_trace_id.unwrap();
    assert_ne!(parent_envelope_id, child_envelope_id);

    let traces = h.audit.traces().await;
    let child_envelope = traces.iter().find(|t| t.trace_id == child_envelope_id).unwrap();
    // The child envelope's parent_trace_ids must contain the parent.
    assert_eq!(child_envelope.parent_trace_ids, vec![parent_envelope_id]);
    // And the payload's parent_envelope_trace_id must echo back.
    let payload: efe_nano_sdk::SwarmEnvelopePayload =
        serde_json::from_value(child_envelope.full_input.clone().unwrap()).unwrap();
    assert_eq!(payload.parent_envelope_trace_id, Some(parent_envelope_id));
}
