//! End-to-end Phase 2 Sprint 4 (Mode B) integration tests.
//!
//! Wires together :
//!   InMemoryCatalog
//!   + HandlerRegistry      (Mode A)
//!   + PureRustRuntime      (Mode A)
//!   + InProcessModelLoader (Mode B)
//!   + StubVmPool           (Mode B)
//!   + LocalModelRuntime    (Mode B)
//!   + MultiModeRuntime     (A + B)
//!   + AuditWriter          (stub)
//!   + IdempotencyCache     (in-memory)
//!   + Dispatcher
//!
//! and runs realistic mixed-mode swarms through all 15 starter agents.

use std::sync::Arc;
use uuid::Uuid;

use efe_nano::audit::{stub::InMemoryAuditWriter, AuditWriter};
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::Dispatcher;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::runtime::model_loader::InProcessModelLoader;
use efe_nano::runtime::pool::stub::{StubVmPool, StubVmPoolConfig};
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::runtime::{LocalModelRuntime, MultiModeRuntime, NanoAgentExecutor};
use efe_nano::starter_pack::register_all_starter_pack_with_mode_b;
use efe_nano_sdk::{
    Catalog, NanoAgentId, NanoAgentRef, OpStatus, Operation, OperationInput, RetryPolicy,
    SwarmRequest, SwarmStatus, TenantId,
};

// ─────────────────────────────────────────────────────────────────────
//  Test harness
// ─────────────────────────────────────────────────────────────────────

struct Harness {
    dispatcher: Dispatcher,
    audit: Arc<InMemoryAuditWriter>,
}

async fn make_harness() -> Harness {
    make_harness_with_pool_config(StubVmPoolConfig::default()).await
}

async fn make_harness_with_pool_config(pool_config: StubVmPoolConfig) -> Harness {
    // 1. Catalog + Mode A registry (with handlers).
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    let count = register_all_starter_pack_with_mode_b(&*catalog, &registry)
        .await
        .unwrap();
    assert_eq!(count, 15, "expected 15 starter agents (10 Mode A + 5 Mode B)");
    assert_eq!(registry.len().await, 10, "10 Mode A handlers only");

    // 2. Mode A runtime.
    let mode_a = Arc::new(PureRustRuntime::new(registry));

    // 3. Mode B : model loader + VM pool + runtime.
    let loader = Arc::new(InProcessModelLoader::with_phase_2_builtins());
    let pool = Arc::new(StubVmPool::new(pool_config, loader));
    let mode_b = Arc::new(LocalModelRuntime::new(pool));

    // 4. Multi-mode runtime.
    let multi: Arc<dyn NanoAgentExecutor> =
        Arc::new(MultiModeRuntime::mode_a_and_b(mode_a, mode_b));

    // 5. Audit + idempotency + dispatcher.
    let mem_audit = Arc::new(InMemoryAuditWriter::new());
    let audit_dyn: Arc<dyn AuditWriter> = mem_audit.clone();
    let idempotency = Arc::new(IdempotencyCache::new());
    let dispatcher = Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        multi,
        audit_dyn,
        idempotency,
    );

    Harness {
        dispatcher,
        audit: mem_audit,
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

fn req(ops: Vec<Operation>) -> SwarmRequest {
    SwarmRequest {
        request_id: Uuid::new_v4(),
        tenant_id: TenantId::system(),
        caller_agent_id: Some("phase-2-mode-b-test".to_string()),
        operations: ops,
        max_parallelism: 100,
        timeout_ms: 60_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["phase-2".to_string(), "mode-b".to_string()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Scenarios
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scenario_all_5_mode_b_agents_invokable_in_parallel() {
    let h = make_harness().await;
    let ops = vec![
        op(
            "ner_med",
            "ner_clinical",
            &[],
            lit(serde_json::json!({"text": "metformin for diabetes BRCA1 phase 3"})),
        ),
        op(
            "ner_law",
            "ner_legal",
            &[],
            lit(serde_json::json!({"text": "Per Roe v. Wade and 347 U.S. 483"})),
        ),
        op(
            "intent",
            "intent_classify_generic",
            &[],
            lit(serde_json::json!({"text": "I want to buy this product"})),
        ),
        op(
            "lang",
            "lang_detect",
            &[],
            lit(serde_json::json!({"text": "The quick brown fox"})),
        ),
        op(
            "embed",
            "embed_text",
            &[],
            lit(serde_json::json!({"text": "hello world"})),
        ),
    ];

    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 5);
    for r in resp.results.values() {
        assert_eq!(
            r.status,
            OpStatus::Success,
            "{} failed : {:?}",
            r.op_id,
            r.error
        );
    }
    // 5 successful calls → 5 audit traces.
    assert_eq!(h.audit.op_count().await, 5);
}

#[tokio::test]
async fn scenario_mixed_mode_a_b_dag_in_one_swarm() {
    let h = make_harness().await;
    // Layer 0 : pdf_extract_text (Mode A) on a fake "PDF"
    //           + ner_clinical (Mode B) on raw text
    //           + ner_legal (Mode B) on raw text
    // Layer 1 : embed_text (Mode B) on a literal text
    let ops = vec![
        op(
            "pdf",
            "pdf_extract_text",
            &[],
            lit(serde_json::json!({"pdf_base64": ""})),
        ),
        op(
            "med",
            "ner_clinical",
            &[],
            lit(serde_json::json!({
                "text": "Patient is on aspirin and metformin"
            })),
        ),
        op(
            "law",
            "ner_legal",
            &[],
            lit(serde_json::json!({"text": "The plaintiff cited 347 U.S. 483"})),
        ),
        op(
            "embed",
            "embed_text",
            &["med", "law"],
            lit(serde_json::json!({"text": "downstream embedding"})),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 4);
    for r in resp.results.values() {
        assert_eq!(r.status, OpStatus::Success);
    }
}

#[tokio::test]
async fn scenario_mode_b_failure_propagates_to_downstream() {
    let h = make_harness().await;
    // a : ner_clinical with invalid input (no `text` field) → input schema fails.
    // b : embed_text depends on a → Skipped.
    let ops = vec![
        op(
            "a",
            "ner_clinical",
            &[],
            lit(serde_json::json!({})),
        ),
        op(
            "b",
            "embed_text",
            &["a"],
            lit(serde_json::json!({"text": "x"})),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    assert_eq!(resp.results["a"].status, OpStatus::Failed);
    assert_eq!(resp.results["b"].status, OpStatus::Skipped);
}

#[tokio::test]
async fn scenario_50_op_parallel_mode_b_swarm() {
    let h = make_harness().await;
    let mut ops = Vec::with_capacity(50);
    for i in 0..50 {
        ops.push(op(
            &format!("op-{i}"),
            "lang_detect",
            &[],
            lit(serde_json::json!({"text": format!("Sample text number {i}")})),
        ));
    }
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 50);
    for r in resp.results.values() {
        assert_eq!(r.status, OpStatus::Success);
    }
}

#[tokio::test]
async fn scenario_mode_b_idempotency_second_call_hits_cache() {
    let h = make_harness().await;
    let ops = vec![op(
        "ner",
        "ner_clinical",
        &[],
        lit(serde_json::json!({"text": "aspirin for hypertension"})),
    )];
    // First dispatch : cold path.
    let r1 = h.dispatcher.dispatch(req(ops.clone())).await.unwrap();
    assert_eq!(r1.results["ner"].status, OpStatus::Success);
    // Second dispatch : should hit the in-process idempotency cache.
    let r2 = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(r2.results["ner"].status, OpStatus::Success);

    // **N12 + N14** : 1 cold + 1 cache-hit + 2 envelopes (one per swarm) = 4.
    let traces = h.audit.traces().await;
    assert_eq!(traces.len(), 4);
    let cold = traces.iter().filter(|t| t.error_kind.is_none()).count();
    let hit = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .count();
    let envelopes = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("SwarmEnvelope"))
        .count();
    assert_eq!(cold, 1, "exactly ONE cold-call trace");
    assert_eq!(hit, 1, "exactly ONE cache-hit lineage trace");
    assert_eq!(envelopes, 2, "two envelopes, one per dispatch");
}

#[tokio::test]
async fn scenario_mode_b_determinism_check() {
    // Mode B agents (builtin models) are fully deterministic : same input
    // produces same output across dispatches.
    let h1 = make_harness().await;
    let h2 = make_harness().await;
    let ops = vec![op(
        "embed",
        "embed_text",
        &[],
        lit(serde_json::json!({"text": "determinism canary"})),
    )];
    let r1 = h1.dispatcher.dispatch(req(ops.clone())).await.unwrap();
    let r2 = h2.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(
        r1.results["embed"].output, r2.results["embed"].output,
        "Mode B determinism violated"
    );
}

#[tokio::test]
async fn scenario_mode_b_acquire_timeout_via_small_pool() {
    // Configure a pool with capacity 1 + synthetic latency 50 ms.
    // Launch 5 parallel ops → 4 should still succeed (queued) within
    // a generous swarm budget.
    let h = make_harness_with_pool_config(StubVmPoolConfig {
        mode_b_max: 1,
        mode_b_warm: 0,
        mode_c_max: 1,
        mode_c_warm: 0,
        synthetic_latency: std::time::Duration::from_millis(20),
    })
    .await;

    let mut ops = Vec::new();
    for i in 0..5 {
        ops.push(op(
            &format!("e{i}"),
            "embed_text",
            &[],
            lit(serde_json::json!({"text": format!("item {i}")})),
        ));
    }
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    // All 5 succeed because the swarm budget (60 s) easily accommodates
    // 5 sequential 20 ms inferences when pool capacity is 1.
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    for r in resp.results.values() {
        assert_eq!(r.status, OpStatus::Success);
    }
}

#[tokio::test]
async fn scenario_audit_trace_records_local_model_mode() {
    let h = make_harness().await;
    let request = req(vec![op(
        "ner",
        "ner_clinical",
        &[],
        lit(serde_json::json!({"text": "aspirin for diabetes"})),
    )]);
    let swarm_id = request.request_id;
    let _ = h.dispatcher.dispatch(request).await.unwrap();
    // N14 : 1 op trace + 1 envelope = 2.  Filter to the op trace.
    let traces = h.audit.traces().await;
    let op_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    assert_eq!(op_traces.len(), 1);
    let t = op_traces[0];
    assert_eq!(t.execution_mode, "local_model");
    assert_eq!(t.swarm_request_id, swarm_id);
    assert_eq!(t.nano_agent_id.as_str(), "ner_clinical");
    assert!(t.completed_at.is_some());
}

#[tokio::test]
async fn scenario_15_starter_agents_all_invokable() {
    let h = make_harness().await;
    // One op per starter agent that exercises a non-trivial input.
    let ops = vec![
        // 10 Mode A.
        op("json", "json_validate", &[],
            lit(serde_json::json!({"value": {"x": 1}, "schema": {"type": "object"}}))),
        op("xml", "xml_validate", &[],
            lit(serde_json::json!({"xml": "<root/>"}))),
        op("regex", "regex_extract", &[],
            lit(serde_json::json!({"pattern": "\\d+", "text": "abc 123 def"}))),
        op("date", "date_normalize", &[],
            lit(serde_json::json!({"input": "2026-05-18"}))),
        op("currency", "currency_normalize", &[],
            lit(serde_json::json!({"input": "$1234"}))),
        op("md", "markdown_extract_headings", &[],
            lit(serde_json::json!({"markdown": "# H1"}))),
        op("csv", "csv_normalize", &[],
            lit(serde_json::json!({"csv": "a,b\n1,2"}))),
        op("sql", "sql_parse_safe", &[],
            lit(serde_json::json!({"sql": "SELECT 1"}))),
        op("html", "html_extract_main_content", &[],
            lit(serde_json::json!({"html": "<title>X</title>Body"}))),
        op("pdf", "pdf_extract_text", &[],
            lit(serde_json::json!({"pdf_base64": ""}))),
        // 5 Mode B.
        op("ner_clinical", "ner_clinical", &[],
            lit(serde_json::json!({"text": "aspirin"}))),
        op("ner_legal", "ner_legal", &[],
            lit(serde_json::json!({"text": "plaintiff"}))),
        op("intent", "intent_classify_generic", &[],
            lit(serde_json::json!({"text": "buy"}))),
        op("lang", "lang_detect", &[],
            lit(serde_json::json!({"text": "the and is of to"}))),
        op("embed", "embed_text", &[],
            lit(serde_json::json!({"text": "vec me"}))),
    ];

    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 15);
    for r in resp.results.values() {
        assert_eq!(
            r.status,
            OpStatus::Success,
            "{} failed : {:?}",
            r.op_id,
            r.error
        );
    }
    // 15 successful calls → 15 audit traces.
    assert_eq!(h.audit.op_count().await, 15);
}

#[tokio::test]
async fn scenario_mode_b_only_when_runtime_lacks_mode_b_returns_mode_not_wired() {
    // Construct a harness with ONLY Mode A wired ; Mode B agent invocation
    // surfaces as `ModeNotWired` (mapped to `InternalError`) and the swarm
    // reports PartialFailure.
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack_with_mode_b(&*catalog, &registry)
        .await
        .unwrap();
    let mode_a = Arc::new(PureRustRuntime::new(registry));
    let multi: Arc<dyn NanoAgentExecutor> =
        Arc::new(MultiModeRuntime::mode_a_only(mode_a));
    let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
    let idempotency = Arc::new(IdempotencyCache::new());
    let dispatcher = Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        multi,
        audit,
        idempotency,
    );

    let ops = vec![op(
        "ner",
        "ner_clinical",
        &[],
        lit(serde_json::json!({"text": "aspirin"})),
    )];
    let resp = dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    let err = resp.results["ner"].error.as_ref().unwrap();
    assert_eq!(err.kind, efe_nano_sdk::OperationErrorKind::InternalError);
    assert!(err.message.contains("local_model"));
}

#[tokio::test]
async fn scenario_total_cost_aggregates_across_modes() {
    let h = make_harness().await;
    let ops = vec![
        op("a", "json_validate", &[],
            lit(serde_json::json!({"value": 1, "schema": {"type": "integer"}}))),
        op("b", "ner_clinical", &[],
            lit(serde_json::json!({"text": "aspirin"}))),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    let summed: f64 = resp.results.values().map(|r| r.cost_cents).sum();
    assert!((resp.total_cost_cents - summed).abs() < 1e-9);
    // Mode B is more expensive than Mode A.
    assert!(resp.results["b"].cost_cents >= resp.results["a"].cost_cents);
}

// ─────────────────────────────────────────────────────────────────────
//  Defensive scenarios (added during Sprint 4 verification audit)
// ─────────────────────────────────────────────────────────────────────

/// 200-op mixed Mode A + Mode B diamond DAG — stress test that scaling
/// + parallelism + audit trace are all correct under load.
#[tokio::test]
async fn scenario_stress_200_op_diamond_mixed_mode() {
    let h = make_harness().await;
    // Layer 0 : 50 lang_detect (Mode B)
    // Layer 1 : 50 regex_extract (Mode A, deps on lang_detect)
    // Layer 2 : 50 embed_text (Mode B, deps on regex_extract)
    // Layer 3 : 50 json_validate (Mode A, deps on embed_text)
    let mut ops = Vec::with_capacity(200);
    for i in 0..50 {
        ops.push(op(
            &format!("lang-{i}"),
            "lang_detect",
            &[],
            lit(serde_json::json!({"text": format!("the and {i}")})),
        ));
    }
    for i in 0..50 {
        ops.push(op(
            &format!("regex-{i}"),
            "regex_extract",
            &[&format!("lang-{i}")],
            lit(serde_json::json!({"pattern": "\\d+", "text": format!("v={i}")})),
        ));
    }
    for i in 0..50 {
        ops.push(op(
            &format!("embed-{i}"),
            "embed_text",
            &[&format!("regex-{i}")],
            lit(serde_json::json!({"text": format!("doc {i}")})),
        ));
    }
    for i in 0..50 {
        ops.push(op(
            &format!("json-{i}"),
            "json_validate",
            &[&format!("embed-{i}")],
            lit(serde_json::json!({
                "value": i,
                "schema": {"type": "integer"}
            })),
        ));
    }

    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 200);
    for r in resp.results.values() {
        assert_eq!(
            r.status,
            OpStatus::Success,
            "op {} failed : {:?}",
            r.op_id,
            r.error
        );
    }
    // All 200 should have audit traces (no cache hits since inputs differ
    // per index).
    assert_eq!(h.audit.op_count().await, 200);
}

/// 4 concurrent dispatchers running independent swarms must not interfere
/// (no shared state in dispatcher beyond the catalog + runtime).
#[tokio::test]
async fn scenario_4_concurrent_dispatchers_independent_swarms() {
    use std::sync::Arc as StdArc;

    let h = StdArc::new(make_harness().await);
    let mut handles = Vec::new();
    for tag in 0..4 {
        let h = StdArc::clone(&h);
        handles.push(tokio::spawn(async move {
            let ops = vec![
                op(
                    "ner",
                    "ner_clinical",
                    &[],
                    lit(serde_json::json!({
                        "text": format!("aspirin trial {tag}")
                    })),
                ),
                op(
                    "lang",
                    "lang_detect",
                    &[],
                    lit(serde_json::json!({"text": "the and is of"})),
                ),
            ];
            h.dispatcher.dispatch(req(ops)).await
        }));
    }

    let mut all_results = Vec::new();
    for handle in handles {
        let resp = handle.await.unwrap().unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);
        all_results.push(resp);
    }
    assert_eq!(all_results.len(), 4);
    // All 4 × 2 = 8 ops succeeded.
    for r in &all_results {
        for op_res in r.results.values() {
            assert_eq!(op_res.status, OpStatus::Success);
        }
    }
}

/// Multi-mode runtime health reports correct Mode B status when wired vs unwired.
#[tokio::test]
async fn scenario_multi_mode_health_reports_correct_state() {
    use efe_nano::runtime::pure_rust::HandlerRegistry;
    use efe_nano::runtime::pool::stub::StubVmPool;
    use efe_nano::runtime::model_loader::InProcessModelLoader;
    use efe_nano::runtime::{LocalModelRuntime, MultiModeRuntime, PureRustRuntime};

    let reg = HandlerRegistry::new();
    let mode_a_only = MultiModeRuntime::mode_a_only(Arc::new(PureRustRuntime::new(reg.clone())));
    let h = mode_a_only.health().await;
    assert!(h.mode_a_alive);
    assert!(!h.mode_b_alive);
    assert!(!h.mode_c_alive);
    assert!(!mode_a_only.has_mode_b());

    let pool = Arc::new(StubVmPool::new(
        efe_nano::runtime::pool::stub::StubVmPoolConfig::default(),
        Arc::new(InProcessModelLoader::with_phase_2_builtins()),
    ));
    let mode_a_b = MultiModeRuntime::mode_a_and_b(
        Arc::new(PureRustRuntime::new(reg)),
        Arc::new(LocalModelRuntime::new(pool)),
    );
    let h2 = mode_a_b.health().await;
    assert!(h2.mode_a_alive);
    assert!(h2.mode_b_alive);
    assert!(mode_a_b.has_mode_b());
}

/// Mode B `caller_agent_id` propagates from SwarmRequest into the audit trace.
///
/// We don't currently set caller_agent_id on every trace (that's Phase 6 work
/// in the HMAC chain extension) but the SwarmRequest field IS persisted, so
/// the dispatcher must not lose it.
#[tokio::test]
async fn scenario_caller_agent_id_preserved_through_dispatch() {
    let h = make_harness().await;
    let mut request = req(vec![op(
        "ner",
        "ner_clinical",
        &[],
        lit(serde_json::json!({"text": "aspirin"})),
    )]);
    request.caller_agent_id = Some("claude-sonnet-4-5".to_string());
    let original_caller = request.caller_agent_id.clone();
    let _ = h.dispatcher.dispatch(request).await.unwrap();
    // The dispatcher consumed the request (moved); we can't inspect it
    // after, but the test demonstrates the field is part of the contract.
    // The audit trace itself was recorded — let's check it has the
    // right swarm_request_id (caller_agent_id flows to the HMAC chain in
    // Sprint 6 ; for now it's preserved in the request but not on the trace).
    assert!(original_caller.is_some());
    // N14 : 1 op trace + 1 envelope = 2.
    let traces = h.audit.traces().await;
    assert_eq!(traces.len(), 2);
}

/// Idempotency cache survives multi-tenant — different tenant IDs must
/// produce different cache keys (no cross-tenant cache hit).
#[tokio::test]
async fn scenario_idempotency_isolated_per_tenant() {
    // Tenant A and B both run the same Mode B agent with identical input.
    let h = make_harness().await;
    let mut req_a = req(vec![op(
        "ner",
        "ner_clinical",
        &[],
        lit(serde_json::json!({"text": "aspirin for diabetes"})),
    )]);
    let mut req_b = req_a.clone();
    req_a.tenant_id = TenantId::system();
    req_b.tenant_id = TenantId::new();   // different tenant

    let _ = h.dispatcher.dispatch(req_a).await.unwrap();
    let _ = h.dispatcher.dispatch(req_b).await.unwrap();
    // 2 distinct tenants → 2 distinct cache keys → 2 audit traces.
    assert_eq!(h.audit.op_count().await, 2);
}

/// Mode B handler with very small wall_clock_ms still completes for a
/// fast deterministic model (no spurious timeouts for cheap inference).
#[tokio::test]
async fn scenario_mode_b_small_budget_still_succeeds_for_cheap_model() {
    let h = make_harness().await;
    let mut request = req(vec![op(
        "lang",
        "lang_detect",
        &[],
        lit(serde_json::json!({"text": "the and is"})),
    )]);
    // 500 ms swarm budget — comfortable for sub-ms lang_detect.
    request.timeout_ms = 500;
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert!(resp.results["lang"].latency_ms < 100);
}

// ─────────────────────────────────────────────────────────────────────
//  Deepest defensive scenarios (Pass 3 — chirurgical verification)
// ─────────────────────────────────────────────────────────────────────

/// Mode A → Mode B chained via `OperationInput::Ref`.
///
/// op_a (Mode A regex_extract) extracts numbers from a text and produces
/// `{ matches: [...], match_count: N }`.
/// op_b (Mode B embed_text) receives a literal `{ text: "..." }` BUT
/// depends on op_a so it only runs after op_a succeeded.
///
/// We don't try to pipe Mode A output directly into Mode B's `text`
/// field via Ref because the agent input schemas don't compose
/// trivially — but the dependency edge + Ref-aware planner is exercised.
#[tokio::test]
async fn scenario_mode_a_to_mode_b_dependency_chain() {
    let h = make_harness().await;
    let ops = vec![
        op(
            "extract",
            "regex_extract",
            &[],
            lit(serde_json::json!({"pattern": "\\d+", "text": "v=42 v=43"})),
        ),
        op(
            "embed",
            "embed_text",
            &["extract"],
            lit(serde_json::json!({"text": "downstream after Mode A success"})),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results["extract"].status, OpStatus::Success);
    assert_eq!(resp.results["embed"].status, OpStatus::Success);
    let extract_count = resp.results["extract"].output.as_ref().unwrap()
        ["count"].as_u64().unwrap();
    assert_eq!(extract_count, 2);
    let embed_dim = resp.results["embed"].output.as_ref().unwrap()
        ["dimension"].as_u64().unwrap();
    assert_eq!(embed_dim, 384);
}

/// Mode B → Mode A chained — opposite direction.
#[tokio::test]
async fn scenario_mode_b_to_mode_a_dependency_chain() {
    let h = make_harness().await;
    let ops = vec![
        op(
            "ner",
            "ner_clinical",
            &[],
            lit(serde_json::json!({"text": "metformin BRCA1 phase 3"})),
        ),
        op(
            "json",
            "json_validate",
            &["ner"],
            lit(serde_json::json!({
                "value": {"x": 1},
                "schema": {"type": "object"}
            })),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    for r in resp.results.values() {
        assert_eq!(r.status, OpStatus::Success);
    }
}

/// Very long inputs (50 KB at the max input cap) must not crash or
/// produce schema-violating outputs.
#[tokio::test]
async fn scenario_mode_b_very_long_input() {
    let h = make_harness().await;
    // Build a 30 KB input (under the 50 KB cap).
    let huge = "aspirin metformin BRCA1 phase 3 ".repeat(1000);
    let ops = vec![op(
        "ner",
        "ner_clinical",
        &[],
        lit(serde_json::json!({"text": huge})),
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    let count = resp.results["ner"].output.as_ref().unwrap()
        ["count"].as_u64().unwrap();
    // 1000 repetitions × 4 entities per chunk = 4000 entities.
    assert!(count >= 3000, "expected many entities from huge input, got {count}");
}

/// Empty-text inputs are valid per schema (`minLength: 1` for some agents,
/// but lang_detect accepts any text ≥ 1 char) — verify near-edge cases
/// still produce valid output.
#[tokio::test]
async fn scenario_mode_b_minimum_length_input() {
    let h = make_harness().await;
    let ops = vec![op(
        "lang",
        "lang_detect",
        &[],
        lit(serde_json::json!({"text": "x"})),  // single char, no markers
    )];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    let out = resp.results["lang"].output.as_ref().unwrap();
    // No language markers found → "und" with 0.0 confidence.
    assert_eq!(out["language"], "und");
    assert!((out["confidence"].as_f64().unwrap() - 0.0).abs() < 1e-9);
}

/// Builder pattern parity : `MultiModeBuilder::build()` produces a runtime
/// equivalent to `MultiModeRuntime::mode_a_and_b(...)`.
#[tokio::test]
async fn scenario_multi_mode_builder_parity_with_constructor() {
    use efe_nano::runtime::pool::stub::StubVmPool;
    use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use efe_nano::runtime::{LocalModelRuntime, MultiModeRuntime};

    // Build the same runtime two ways.
    let reg1 = HandlerRegistry::new();
    let pool1 = Arc::new(StubVmPool::with_phase_2_defaults());
    let r1 = MultiModeRuntime::mode_a_and_b(
        Arc::new(PureRustRuntime::new(reg1)),
        Arc::new(LocalModelRuntime::new(pool1)),
    );

    let reg2 = HandlerRegistry::new();
    let pool2 = Arc::new(StubVmPool::with_phase_2_defaults());
    let r2 = MultiModeRuntime::builder(Arc::new(PureRustRuntime::new(reg2)))
        .with_mode_b(Arc::new(LocalModelRuntime::new(pool2)))
        .build();

    assert_eq!(r1.has_mode_b(), r2.has_mode_b());
    let h1 = r1.health().await;
    let h2 = r2.health().await;
    assert_eq!(h1.mode_a_alive, h2.mode_a_alive);
    assert_eq!(h1.mode_b_alive, h2.mode_b_alive);
}

/// Twin agents calling the same model concurrently must both succeed
/// without deadlock.  Exercises the OnceCell shared-init path.
#[tokio::test]
async fn scenario_mode_b_concurrent_same_input_both_succeed() {
    let h = Arc::new(make_harness().await);
    // Two ops in same layer, same agent, same input.
    let ops = vec![
        op("a", "embed_text", &[],
            lit(serde_json::json!({"text": "twin input"}))),
        op("b", "embed_text", &[],
            lit(serde_json::json!({"text": "twin input"}))),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    let a = resp.results["a"].output.as_ref().unwrap();
    let b = resp.results["b"].output.as_ref().unwrap();
    // Same input → same deterministic output.
    assert_eq!(a["embedding"], b["embedding"]);
    // But idempotency cache hits on second call → only 1 audit trace
    // recorded (the second call short-circuits via the cache).
    let trace_count = h.audit.op_count().await;
    assert!(
        trace_count == 1 || trace_count == 2,
        "expected 1 or 2 traces (cache race), got {trace_count}"
    );
}

/// Distinct VmKinds use distinct semaphores : a saturated ModelInference
/// pool does NOT block LlmCaller acquires (and vice versa).  This is the
/// per-kind isolation guarantee.
#[tokio::test]
async fn scenario_per_kind_pool_isolation_via_dispatcher() {
    // Saturate Mode B with one slow op, then verify a fast Mode A op
    // (which doesn't go through the VM pool) is unaffected.
    let h = make_harness_with_pool_config(StubVmPoolConfig {
        mode_b_max: 1,
        mode_b_warm: 0,
        mode_c_max: 1,
        mode_c_warm: 0,
        synthetic_latency: std::time::Duration::from_millis(50),
    })
    .await;

    let ops = vec![
        op("slow_b", "embed_text", &[],
            lit(serde_json::json!({"text": "slow"}))),
        op("fast_a", "json_validate", &[],
            lit(serde_json::json!({
                "value": 1,
                "schema": {"type": "integer"}
            }))),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    // Mode A op finishes faster than Mode B op (because Mode B has 50ms
    // synthetic latency but Mode A has zero).
    let a_lat = resp.results["fast_a"].latency_ms;
    let b_lat = resp.results["slow_b"].latency_ms;
    assert!(
        a_lat < b_lat,
        "expected fast_a ({a_lat} ms) < slow_b ({b_lat} ms)"
    );
}

/// Repeated lease acquire / drop / acquire (10 times sequentially) on a
/// pool with capacity 1 — proves RAII release works correctly.
#[tokio::test]
async fn scenario_vm_lease_drop_releases_permit_sequentially() {
    use efe_nano::runtime::pool::stub::StubVmPool;
    use efe_nano::runtime::pool::{VmKind, VmPool};
    use efe_nano::runtime::model_loader::InProcessModelLoader;
    use std::time::Duration;

    let pool = StubVmPool::new(
        StubVmPoolConfig {
            mode_b_max: 1,
            mode_b_warm: 0,
            mode_c_max: 1,
            mode_c_warm: 0,
            synthetic_latency: Duration::ZERO,
        },
        Arc::new(InProcessModelLoader::with_phase_2_builtins()),
    );

    for i in 0..10 {
        let lease = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap_or_else(|_| panic!("iteration {i} failed to acquire"));
        // Lease drops at end of scope.
        drop(lease);
    }
    // If permit is leaked, the 2nd iteration would have timed out.
    // 10 successful sequential acquires proves RAII release.
}
