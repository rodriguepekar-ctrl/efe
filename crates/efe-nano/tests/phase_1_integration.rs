//! End-to-end Phase 1 integration tests.
//!
//! Wires together :
//!   InMemoryCatalog  +  HandlerRegistry  +  PureRustRuntime
//!   +  AuditWriter   +  IdempotencyCache  +  Dispatcher
//!
//! and runs realistic [`SwarmRequest`] flows through all 10 starter-pack
//! nano agents.

use std::sync::Arc;
use uuid::Uuid;

use efe_nano::audit::{stub::InMemoryAuditWriter, AuditWriter};
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::Dispatcher;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::starter_pack::register_all_starter_pack;
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
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    let count = register_all_starter_pack(&*catalog, &registry).await.unwrap();
    assert_eq!(count, 10, "expected 10 starter agents to register");

    let runtime = Arc::new(PureRustRuntime::new(registry));
    let mem_audit = Arc::new(InMemoryAuditWriter::new());
    let audit_dyn: Arc<dyn AuditWriter> = mem_audit.clone();
    let idempotency = Arc::new(IdempotencyCache::new());

    Harness {
        dispatcher: Dispatcher::with_pure_rust_runtime(
            catalog as Arc<dyn Catalog>,
            runtime,
            audit_dyn,
            idempotency,
        ),
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
        caller_agent_id: Some("integration-test".to_string()),
        operations: ops,
        max_parallelism: 100,
        timeout_ms: 30_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["phase-1".to_string()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Scenarios
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scenario_all_10_starter_agents_invokable_in_parallel() {
    let h = make_harness().await;

    let ops = vec![
        op(
            "json",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": {"name": "Alice", "age": 30},
                "schema": {"type": "object", "required": ["name"]}
            })),
        ),
        op(
            "xml",
            "xml_validate",
            &[],
            lit(serde_json::json!({"xml": "<root><child>v</child></root>"})),
        ),
        op(
            "regex",
            "regex_extract",
            &[],
            lit(serde_json::json!({
                "pattern": r"\d+",
                "text": "Bob is 42 years old"
            })),
        ),
        op(
            "date",
            "date_normalize",
            &[],
            lit(serde_json::json!({"input": "2026-05-18"})),
        ),
        op(
            "currency",
            "currency_normalize",
            &[],
            lit(serde_json::json!({"input": "$1,234.56"})),
        ),
        op(
            "md",
            "markdown_extract_headings",
            &[],
            lit(serde_json::json!({"markdown": "# H1\n\n## H2"})),
        ),
        op(
            "csv",
            "csv_normalize",
            &[],
            lit(serde_json::json!({"csv": "a,b\n1,2\n3,4"})),
        ),
        op(
            "sql",
            "sql_parse_safe",
            &[],
            lit(serde_json::json!({"sql": "SELECT * FROM users"})),
        ),
        op(
            "html",
            "html_extract_main_content",
            &[],
            lit(serde_json::json!({
                "html": "<html><title>T</title><body><p>Body</p></body></html>"
            })),
        ),
        op(
            "pdf",
            "pdf_extract_text",
            &[],
            lit(serde_json::json!({"pdf_base64": ""})),
        ),
    ];

    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 10);
    for r in resp.results.values() {
        assert_eq!(
            r.status,
            OpStatus::Success,
            "{} failed : {:?}",
            r.op_id,
            r.error
        );
    }

    // Audit trace : every successful op records exactly one trace.
    assert_eq!(h.audit.op_count().await, 10);
}

#[tokio::test]
async fn scenario_three_layer_dag_with_ref_input() {
    let h = make_harness().await;

    // Layer 0 : `extract` runs regex on raw text.
    // Layer 1 : `validate` checks the extraction has ≥ 1 match.
    // Layer 2 : `normalize` formats the count as currency.
    let ops = vec![
        op(
            "extract",
            "regex_extract",
            &[],
            lit(serde_json::json!({
                "pattern": r"\d+",
                "text": "I paid 42 dollars and 13 cents"
            })),
        ),
        op(
            "validate",
            "json_validate",
            &["extract"],
            OperationInput::Merge {
                sources: vec![
                    lit(serde_json::json!({
                        "schema": {"type": "object", "required": ["match_count"]}
                    })),
                    OperationInput::Ref {
                        op_id: "extract".to_string(),
                        json_path: "$".to_string(),
                    },
                    // Wrap the upstream output into the {"value": ...} envelope.
                    lit(serde_json::json!({"value": {"match_count": 2}})),
                ],
            },
        ),
        op(
            "normalize",
            "currency_normalize",
            &["validate"],
            lit(serde_json::json!({"input": "$2.00"})),
        ),
    ];

    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results["extract"].status, OpStatus::Success);
    assert_eq!(resp.results["validate"].status, OpStatus::Success);
    assert_eq!(resp.results["normalize"].status, OpStatus::Success);
}

#[tokio::test]
async fn scenario_failure_propagates_to_downstream_only() {
    let h = make_harness().await;

    // a fails, b is independent and succeeds, c depends on a → Skipped.
    let ops = vec![
        op(
            "a",
            "json_validate",
            &[],
            lit(serde_json::json!({"value": {}})),  // missing schema → input schema fail
        ),
        op(
            "b",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": "x",
                "schema": {"type": "string"}
            })),
        ),
        op(
            "c",
            "json_validate",
            &["a"],
            lit(serde_json::json!({
                "value": "x",
                "schema": {"type": "string"}
            })),
        ),
        op(
            "d",
            "json_validate",
            &["b"],
            lit(serde_json::json!({
                "value": "y",
                "schema": {"type": "string"}
            })),
        ),
    ];

    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::PartialFailure);
    assert_eq!(resp.results["a"].status, OpStatus::Failed);
    assert_eq!(resp.results["b"].status, OpStatus::Success);
    assert_eq!(resp.results["c"].status, OpStatus::Skipped);
    assert_eq!(resp.results["d"].status, OpStatus::Success);
}

#[tokio::test]
async fn scenario_idempotency_second_invocation_hits_cache() {
    let h = make_harness().await;

    let ops = vec![op(
        "v",
        "json_validate",
        &[],
        lit(serde_json::json!({
            "value": {"x": 1},
            "schema": {"type": "object"}
        })),
    )];
    let r1 = h.dispatcher.dispatch(req(ops.clone())).await.unwrap();
    let r2 = h.dispatcher.dispatch(req(ops)).await.unwrap();

    assert_eq!(r1.overall_status, SwarmStatus::Success);
    assert_eq!(r2.overall_status, SwarmStatus::Success);

    // **Sprint 8 — N12** : cache hits now emit a dedicated trace.
    // **Sprint 8 — N14** : each swarm also emits a SwarmEnvelope trace.
    // We expect 4 traces total : 1 cold + 1 cache-hit + 2 envelopes
    // (one per swarm dispatch).
    let traces = h.audit.traces().await;
    assert_eq!(traces.len(), 4, "1 cold + 1 cache-hit + 2 envelopes = 4");

    let cold_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    let hit_traces: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .collect();
    assert_eq!(
        cold_traces.len(),
        1,
        "exactly ONE handler invocation across 2 dispatches"
    );
    assert_eq!(
        hit_traces.len(),
        1,
        "exactly ONE cache-hit lineage trace from the warm call"
    );
    // The cache-hit trace's parent is the cold trace (N1 + N12 lineage).
    assert_eq!(hit_traces[0].parent_trace_ids, vec![cold_traces[0].trace_id]);
}

#[tokio::test]
async fn scenario_cycle_rejected_pre_execution() {
    let h = make_harness().await;
    let ops = vec![
        op(
            "a",
            "json_validate",
            &["c"],
            lit(serde_json::json!({"value": {}, "schema": {}})),
        ),
        op(
            "b",
            "json_validate",
            &["a"],
            lit(serde_json::json!({"value": {}, "schema": {}})),
        ),
        op(
            "c",
            "json_validate",
            &["b"],
            lit(serde_json::json!({"value": {}, "schema": {}})),
        ),
    ];
    let err = h.dispatcher.dispatch(req(ops)).await.unwrap_err();
    let s = format!("{err}");
    assert!(s.contains("cycle"), "expected cycle in error, got {s}");
    // Nothing should have been audited.
    assert_eq!(h.audit.op_count().await, 0);
}

#[tokio::test]
async fn scenario_unknown_agent_yields_invalid_dag() {
    let h = make_harness().await;
    let ops = vec![op(
        "x",
        "ghost_agent",
        &[],
        lit(serde_json::json!({})),
    )];
    let err = h.dispatcher.dispatch(req(ops)).await.unwrap_err();
    let s = format!("{err}");
    assert!(
        s.contains("invalid DAG") || s.contains("catalog"),
        "got {s}"
    );
}

#[tokio::test]
async fn scenario_large_parallel_fanout_50_ops() {
    let h = make_harness().await;

    // 50 independent json_validate ops in one layer.
    let mut ops = Vec::with_capacity(50);
    for i in 0..50 {
        ops.push(op(
            &format!("op-{i}"),
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": i,
                "schema": {"type": "integer"}
            })),
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
async fn scenario_total_cost_is_sum_of_per_op_costs() {
    let h = make_harness().await;
    let ops = vec![
        op(
            "a",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": 1,
                "schema": {"type": "integer"}
            })),
        ),
        op(
            "b",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": 2,
                "schema": {"type": "integer"}
            })),
        ),
        op(
            "c",
            "json_validate",
            &[],
            lit(serde_json::json!({
                "value": 3,
                "schema": {"type": "integer"}
            })),
        ),
    ];
    let resp = h.dispatcher.dispatch(req(ops)).await.unwrap();
    let summed: f64 = resp.results.values().map(|r| r.cost_cents).sum();
    assert!((resp.total_cost_cents - summed).abs() < 1e-9);
}

#[tokio::test]
async fn scenario_audit_trace_carries_swarm_metadata() {
    let h = make_harness().await;
    let request = req(vec![op(
        "v",
        "json_validate",
        &[],
        lit(serde_json::json!({
            "value": 1,
            "schema": {"type": "integer"}
        })),
    )]);
    let swarm_id = request.request_id;
    let _ = h.dispatcher.dispatch(request).await.unwrap();

    // **Sprint 8 — N14** : each swarm emits a SwarmEnvelope trace +
    // K op traces.  Filter for the actual op trace.
    let traces = h.audit.traces().await;
    let op_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    assert_eq!(op_traces.len(), 1);
    let t = op_traces[0];
    assert_eq!(t.swarm_request_id, swarm_id);
    assert_eq!(t.operation_id, "v");
    assert_eq!(t.nano_agent_id.as_str(), "json_validate");
    assert!(t.error_kind.is_none());
    assert!(t.completed_at.is_some());
    assert!(t.latency_ms.is_some());
    // N14 : the op trace carries the envelope reference.
    assert!(t.swarm_envelope_trace_id.is_some(), "N14: op trace must reference its envelope");
}
