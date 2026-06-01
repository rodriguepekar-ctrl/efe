//! End-to-end Phase 2 Sprint 6 — Audit HMAC chain integration tests.
//!
//! Validates the **complete pipeline** from a live `Dispatcher.dispatch`
//! call down to the per-tenant HMAC chain :
//!
//!   Dispatcher
//!     → PureRustRuntime          (Mode A, Sprint 1)
//!     → InMemoryHmacChainAuditWriter  (Sprint 6)
//!       → InMemoryChainStore           (Sprint 6)
//!         → tenant-scoped chain
//!         → side-store for full input / output bodies
//!     ↑   reads ↑
//!     verify_chain / verify_subchain  (Sprint 6)
//!
//! Each scenario exercises one architectural invariant of Sprint 6 :
//!
//! | # | Test                                              | Property under test                                 |
//! |---|---------------------------------------------------|-----------------------------------------------------|
//! | 1 | `audit_chain_records_one_trace_per_op`            | one op  →  one chain entry                          |
//! | 2 | `audit_chain_links_traces_in_order`               | chain_parent_id forms a linear order                |
//! | 3 | `audit_chain_first_entry_has_no_parent`           | genesis row has `chain_parent_id == None`           |
//! | 4 | `audit_chain_cross_tenant_isolated`               | per-tenant chains are independent                   |
//! | 5 | `audit_chain_verify_passes_on_clean_data`         | verify_chain returns Ok on honest writer            |
//! | 6 | `audit_chain_verify_detects_tampered_cost`        | post-hoc mutation → HmacMismatch                    |
//! | 7 | `audit_chain_verify_detects_swapped_entries`      | reorder → ChainBroken                               |
//! | 8 | `audit_chain_verify_wrong_key_rejected`           | wrong key  →  HmacMismatch                          |
//! | 9 | `audit_chain_subchain_verify_swarm_scoped`        | verify_subchain works on a single swarm's slice     |
//! |10 | `audit_chain_body_storage_full_input_present`     | full_input lands in side-store on default policy    |
//! |11 | `audit_chain_payload_hashes_distinct_per_trace`   | every payload_hash is unique                        |
//! |12 | `audit_chain_concurrent_tenants_no_contention`    | concurrent multi-tenant appends preserve isolation  |
//! |13 | `audit_chain_post_restart_warm_tip_no_fork`       | new writer + warm_tip continues the chain           |
//! |14 | `audit_chain_full_swarm_dag_recorded_in_order`    | diamond DAG produces N chained entries              |
//! |15 | `audit_chain_thousand_trace_verify_under_5_sec`   | 1 000 traces verify in well under 5 seconds         |
//!
//! These are deliberately written in the "phase_2_mode_*" integration
//! test style :  one binary file, one shared harness, one `#[tokio::test]`
//! per scenario.

use std::sync::Arc;
use uuid::Uuid;

use efe_nano::audit::chain_store::{ChainStore, InMemoryChainStore};
use efe_nano::audit::hkdf::HmacKeySource;
use efe_nano::audit::hmac_chain::{HmacChainAuditWriter, InMemoryHmacChainAuditWriter};
use efe_nano::audit::verify::{verify_chain, verify_subchain};
use efe_nano::audit::AuditWriter;
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::Dispatcher;
use efe_nano::error::AuditChainError;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::runtime::NanoAgentExecutor;
use efe_nano::starter_pack::register_all_starter_pack;
use efe_nano_sdk::{
    Catalog, NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy, SwarmRequest,
    SwarmStatus, TenantId,
};

// ─────────────────────────────────────────────────────────────────────
//  Harness
// ─────────────────────────────────────────────────────────────────────

/// Wire `catalog + Mode A runtime + InMemoryHmacChainAuditWriter +
/// dispatcher` together.  Exposes the chain store + key source so
/// tests can verify or tamper.
struct ChainHarness {
    dispatcher: Arc<Dispatcher>,
    chain_store: Arc<InMemoryChainStore>,
    key_source: HmacKeySource,
}

async fn make_chain_harness() -> ChainHarness {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry)
        .await
        .unwrap();
    let runtime: Arc<dyn NanoAgentExecutor> = Arc::new(PureRustRuntime::new(registry));

    let chain_store = Arc::new(InMemoryChainStore::new());
    let key_source = HmacKeySource::test_key();
    let writer = Arc::new(InMemoryHmacChainAuditWriter::new(
        Arc::clone(&chain_store),
        key_source.clone(),
    ));
    let dispatcher = Arc::new(Dispatcher::new(
        catalog as Arc<dyn Catalog>,
        runtime,
        Arc::clone(&writer) as Arc<dyn AuditWriter>,
        Arc::new(IdempotencyCache::new()),
    ));
    ChainHarness {
        dispatcher,
        chain_store,
        key_source,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────

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

fn req_tagged(ops: Vec<Operation>, tenant: TenantId, request_id: Uuid) -> SwarmRequest {
    SwarmRequest {
        request_id,
        tenant_id: tenant,
        caller_agent_id: Some("phase-2-audit-chain".to_string()),
        operations: ops,
        max_parallelism: 64,
        timeout_ms: 60_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["phase-2".to_string(), "audit-chain".to_string()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

/// A single `json_validate` op with a trivially-valid input.
///
/// The op_id is embedded in the input value so that no two calls share
/// the same `(agent, input)` pair — otherwise the dispatcher's
/// idempotency cache would short-circuit the second call and skip the
/// audit-writer call (cf `dispatcher::executor::run_one_op` line 326-333).
/// Sprint 6 audit-chain tests REQUIRE every op to produce a fresh trace.
fn op_validate_obj(op_id: &str) -> Operation {
    op(
        op_id,
        "json_validate",
        &[],
        lit(serde_json::json!({
            "value": {"x": 1, "_uniq": op_id},
            "schema": {"type": "object"}
        })),
    )
}

/// Dispatch a single-op swarm and return (request_id, response).
async fn dispatch_one_op(
    harness: &ChainHarness,
    tenant: TenantId,
    op_id: &str,
) -> (Uuid, efe_nano_sdk::SwarmResponse) {
    let request_id = Uuid::new_v4();
    let request = req_tagged(vec![op_validate_obj(op_id)], tenant, request_id);
    let resp = harness.dispatcher.dispatch(request).await.unwrap();
    (request_id, resp)
}

// ─────────────────────────────────────────────────────────────────────
//  1. One op → one chain entry
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_records_one_trace_per_op() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let (_, resp) = dispatch_one_op(&h, tenant, "v").await;
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(
        chain.len(),
        1,
        "single op should produce exactly one chain entry, got {}",
        chain.len()
    );
    assert_eq!(chain[0].trace.operation_id, "v");
    assert_eq!(chain[0].trace.execution_mode, "pure_rust");
}

// ─────────────────────────────────────────────────────────────────────
//  2. N ops → N entries chained in append order
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_links_traces_in_order() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![
            op_validate_obj("a"),
            op_validate_obj("b"),
            op_validate_obj("c"),
        ],
        tenant,
        request_id,
    );
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 3);

    // First entry : genesis (no parent).
    assert!(
        chain[0].chain_parent_id.is_none(),
        "first entry should have no chain_parent_id"
    );

    // Subsequent entries link to the previous trace's trace_id.
    for i in 1..chain.len() {
        assert_eq!(
            chain[i].chain_parent_id,
            Some(chain[i - 1].trace_id()),
            "entry {i} should chain-link to entry {}",
            i - 1
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
//  3. First (genesis) entry has chain_parent_id = None
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_first_entry_has_no_parent() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let (_, _resp) = dispatch_one_op(&h, tenant, "v").await;

    // N14 : the chain has [op, envelope].  The envelope is the tip ;
    // the GENESIS (first) row is the op trace.  Verify the genesis
    // has no parent.
    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap();
    let genesis = chain
        .first()
        .expect("tenant has at least one entry");
    assert!(
        genesis.chain_parent_id.is_none(),
        "genesis row must have no parent (got {:?})",
        genesis.chain_parent_id
    );
    assert_ne!(genesis.hmac, [0u8; 32], "HMAC must be populated even on genesis");
    assert_ne!(
        genesis.payload_hash, [0u8; 32],
        "payload_hash must be populated"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  4. Cross-tenant isolation : two tenants → two independent chains
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_cross_tenant_isolated() {
    let h = make_chain_harness().await;
    let t1 = TenantId::new();
    let t2 = TenantId::new();

    // Dispatch one swarm per tenant.
    let (_, _) = dispatch_one_op(&h, t1, "op-t1").await;
    let (_, _) = dispatch_one_op(&h, t2, "op-t2").await;

    let c1 = h.chain_store.chain_for_tenant(t1, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    let c2 = h.chain_store.chain_for_tenant(t2, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();

    assert_eq!(c1.len(), 1, "tenant 1 chain should have 1 entry");
    assert_eq!(c2.len(), 1, "tenant 2 chain should have 1 entry");
    assert_eq!(c1[0].trace.tenant_id, t1);
    assert_eq!(c2[0].trace.tenant_id, t2);

    // Both are genesis rows for their respective tenants.
    assert!(c1[0].chain_parent_id.is_none());
    assert!(c2[0].chain_parent_id.is_none());

    // Different HMACs (different derived keys + different payloads).
    assert_ne!(c1[0].hmac, c2[0].hmac);
}

// ─────────────────────────────────────────────────────────────────────
//  5. verify_chain succeeds on honest data
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_verify_passes_on_clean_data() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();

    // 5 ops in one swarm.
    let request_id = Uuid::new_v4();
    let mut ops = Vec::new();
    for i in 0..5 {
        ops.push(op_validate_obj(&format!("op-{i}")));
    }
    let request = req_tagged(ops, tenant, request_id);
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    let report = verify_chain(&*h.chain_store, &h.key_source, tenant, None)
        .await
        .unwrap();
    assert_eq!(report.tenant, tenant);
    // N14 : 5 op traces + 1 envelope = 6 entries.
    assert_eq!(report.entries_verified, 6);
    assert!(report.first_trace_id.is_some());
    assert!(report.last_trace_id.is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  6. Detect tampered payload (HmacMismatch)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_verify_detects_tampered_cost() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![op_validate_obj("a"), op_validate_obj("b")],
        tenant,
        request_id,
    );
    let _ = h.dispatcher.dispatch(request).await.unwrap();

    // Snapshot, tamper one entry, re-verify via verify_subchain.
    let mut chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 2);
    chain[0].trace.cost_cents = 9_999.99;        // alter the body

    let err = verify_subchain(&chain, &h.key_source, tenant)
        .await
        .unwrap_err();
    match err {
        AuditChainError::HmacMismatch(id) => {
            assert_eq!(id, chain[0].trace_id(), "must point to the tampered row");
        }
        other => panic!("expected HmacMismatch, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  7. Detect swapped entries (ChainBroken)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_verify_detects_swapped_entries() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![
            op_validate_obj("a"),
            op_validate_obj("b"),
            op_validate_obj("c"),
        ],
        tenant,
        request_id,
    );
    let _ = h.dispatcher.dispatch(request).await.unwrap();

    let mut chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 3);
    chain.swap(1, 2);  // reorder rows 1 and 2

    let err = verify_subchain(&chain, &h.key_source, tenant)
        .await
        .unwrap_err();
    // After swapping, the new chain[1] has a chain_parent_id pointing
    // somewhere that's no longer the previous tip → ChainBroken.
    // (HmacMismatch is also acceptable because we recompute HMAC against
    // the expected_parent passed in ; both are valid integrity signals.)
    match err {
        AuditChainError::ChainBroken(_) | AuditChainError::HmacMismatch(_) => {}
        other => panic!("expected ChainBroken or HmacMismatch, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  8. Wrong verification key → HmacMismatch
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_verify_wrong_key_rejected() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(vec![op_validate_obj("a")], tenant, request_id);
    let _ = h.dispatcher.dispatch(request).await.unwrap();

    // Verify with a wrong static master.
    let wrong_key = HmacKeySource::StaticMaster(
        b"another-master-32-bytes-zzzzzzzzzzzz".to_vec(),
    );
    let err = verify_chain(&*h.chain_store, &wrong_key, tenant, None)
        .await
        .unwrap_err();
    match err {
        AuditChainError::HmacMismatch(_) => {}
        other => panic!("expected HmacMismatch, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  9. verify_subchain works on a single swarm's slice
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_subchain_verify_swarm_scoped() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();

    // Two different swarms back-to-back.
    let swarm_a = Uuid::new_v4();
    let swarm_b = Uuid::new_v4();
    let _ = h
        .dispatcher
        .dispatch(req_tagged(
            vec![op_validate_obj("a1"), op_validate_obj("a2")],
            tenant,
            swarm_a,
        ))
        .await
        .unwrap();
    let _ = h
        .dispatcher
        .dispatch(req_tagged(
            vec![op_validate_obj("b1"), op_validate_obj("b2")],
            tenant,
            swarm_b,
        ))
        .await
        .unwrap();

    let chain_a = h
        .chain_store
        .traces_for_swarm(tenant, swarm_a)
        .await
        .unwrap();
    let chain_b = h
        .chain_store
        .traces_for_swarm(tenant, swarm_b)
        .await
        .unwrap();
    // N14 : 2 ops + 1 envelope per swarm = 3 traces.
    assert_eq!(chain_a.len(), 3);
    assert_eq!(chain_b.len(), 3);

    // verify_subchain for swarm_a slice : the first entry of swarm_a is
    // the genesis row of the whole tenant chain, so chain_parent_id == None.
    let report_a = verify_subchain(&chain_a, &h.key_source, tenant)
        .await
        .unwrap();
    assert_eq!(report_a.entries_verified, 3);

    // verify_subchain for swarm_b slice : the first entry of swarm_b
    // chain-links to the LAST entry of swarm_a (continuous chain).
    let report_b = verify_subchain(&chain_b, &h.key_source, tenant)
        .await
        .unwrap();
    assert_eq!(report_b.entries_verified, 3);
}

// ─────────────────────────────────────────────────────────────────────
//  10. Full input present in the body side-store
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_body_storage_full_input_present() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();

    // All starter-pack agents use AuditPolicy::default() = include_input + include_output.
    let request_id = Uuid::new_v4();
    let request = req_tagged(vec![op_validate_obj("v")], tenant, request_id);
    let _ = h.dispatcher.dispatch(request).await.unwrap();

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 1);
    let entry = &chain[0];

    // The body store should have the input + output bodies stashed by
    // input_hash / output_hash respectively.
    let input_body = h.chain_store.get_input(&entry.trace.input_hash_resolved);
    assert!(
        input_body.is_some(),
        "full_input should be stashed by the InMemoryHmacChainAuditWriter (agent.audit.include_input=true)"
    );
    let stored = input_body.unwrap();
    assert_eq!(stored["value"]["x"], 1);

    if let Some(output_hash) = entry.trace.output_hash {
        let output_body = h.chain_store.get_output(&output_hash);
        assert!(output_body.is_some(), "full_output should also be stashed");
    }

    // Body store counts : at least 1 input + 1 output.
    assert!(h.chain_store.input_body_count() >= 1);
    assert!(h.chain_store.output_body_count() >= 1);
}

// ─────────────────────────────────────────────────────────────────────
//  11. Every payload_hash is unique (per-trace fingerprint)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_payload_hashes_distinct_per_trace() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();

    // Same agent, same input, different op_ids → traces should still
    // differ (trace_id + started_at change, so payload_hash differs).
    let request_id = Uuid::new_v4();
    let mut ops = Vec::new();
    for i in 0..6 {
        ops.push(op_validate_obj(&format!("op-{i}")));
    }
    let request = req_tagged(ops, tenant, request_id);
    let _ = h.dispatcher.dispatch(request).await.unwrap();

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 6);

    let mut seen_payload = std::collections::HashSet::new();
    let mut seen_hmac = std::collections::HashSet::new();
    for entry in &chain {
        assert!(
            seen_payload.insert(entry.payload_hash),
            "duplicate payload_hash found"
        );
        assert!(seen_hmac.insert(entry.hmac), "duplicate hmac found");
    }
}

// ─────────────────────────────────────────────────────────────────────
//  12. Concurrent multi-tenant appends preserve isolation
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_concurrent_tenants_no_contention() {
    let h = Arc::new(make_chain_harness().await);
    // Pre-allocate tenant ids and ops outside the join (the dispatcher
    // doesn't expose `Arc<Dispatcher>` directly, but we have it).
    let tenant_ids: Vec<TenantId> = (0..8).map(|_| TenantId::new()).collect();

    let mut handles = Vec::new();
    for (i, &tenant) in tenant_ids.iter().enumerate() {
        let harness = Arc::clone(&h);
        handles.push(tokio::spawn(async move {
            let request_id = Uuid::new_v4();
            let request = req_tagged(
                vec![op_validate_obj(&format!("t{i}-op"))],
                tenant,
                request_id,
            );
            harness.dispatcher.dispatch(request).await
        }));
    }
    for handle in handles {
        let resp = handle.await.unwrap().unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);
    }

    // Each tenant should have exactly 1 entry.
    for &tenant in &tenant_ids {
        let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
        assert_eq!(
            chain.len(),
            1,
            "tenant {tenant} should have exactly 1 entry"
        );
        assert!(chain[0].chain_parent_id.is_none());
    }

    // N14 : total entries == 2 × tenants (1 op + 1 envelope per tenant).
    assert_eq!(
        h.chain_store.total_entries().await.unwrap(),
        (tenant_ids.len() * 2) as u64
    );
}

// ─────────────────────────────────────────────────────────────────────
//  13. Post-restart : new writer + warm_tip → no fork
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_post_restart_warm_tip_no_fork() {
    // Build with the standard harness, dispatch a swarm.
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let (_, _) = dispatch_one_op(&h, tenant, "before-restart").await;

    let initial_chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(initial_chain.len(), 1);
    let original_tip = initial_chain[0].trace_id();

    // Simulate a service restart : construct a fresh HmacChainAuditWriter
    // against the SAME underlying chain store + key source.  The "post-
    // restart" writer must NOT create a forked genesis row ; it must warm
    // its tip to the existing chain's tip and link new entries to it.
    let post_restart_writer = HmacChainAuditWriter::new(
        Arc::clone(&h.chain_store) as Arc<dyn ChainStore>,
        h.key_source.clone(),
    );
    post_restart_writer.warm_tip(tenant).await.unwrap();

    // Build a synthetic trace and record it via the post-restart writer.
    use chrono::Utc;
    use efe_nano::audit::NanoAgentTrace;
    use efe_nano_sdk::{NanoAgentVersion, TraceId};
    let new_trace = NanoAgentTrace {
        trace_id: TraceId(Uuid::new_v4()),
        parent_trace_ids: vec![],
        swarm_request_id: Uuid::new_v4(),
        operation_id: "after-restart".to_string(),
        tenant_id: tenant,
        caller_agent_id: Some("warm-tip-test".to_string()),
        nano_agent_id: NanoAgentId::new("json_validate").unwrap(),
        nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        execution_mode: "pure_rust".to_string(),
        input_hash_source: [42u8; 32],
        input_hash_resolved: [42u8; 32],
        output_hash: Some([43u8; 32]),
        error_kind: None,
        started_at: Utc::now(),
        completed_at: Some(Utc::now()),
        latency_ms: Some(5),
        cost_cents: 0.001,
        llm_tokens_used: None,
        full_input: None,
        full_output: None,
        tombstone: None,
        swarm_envelope_trace_id: None,
    };
    post_restart_writer.record(new_trace).await;

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 2, "post-restart write must not fork the chain");
    // N14 : with the envelope in the chain too, the post-restart entry's
    // chain_parent_id is the envelope id of the first swarm, not the op.
    // Check the GLOBAL chain order (including envelope) for the link
    // assertion.
    let full_chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap();
    let last = full_chain.last().expect("at least one entry");
    let prev = full_chain.get(full_chain.len() - 2).expect("at least two entries");
    assert_eq!(
        last.chain_parent_id,
        Some(prev.trace_id()),
        "post-restart entry must chain to the pre-restart tip"
    );
    let _ = original_tip; // unused in N14 since the linkage logic differs

    // And the full chain still verifies.  N14 : 1 pre-restart op + 1
    // envelope + 1 post-restart op = 3 entries.
    let report = verify_chain(&*h.chain_store, &h.key_source, tenant, None)
        .await
        .unwrap();
    assert_eq!(report.entries_verified, 3);
}

// ─────────────────────────────────────────────────────────────────────
//  14. Diamond DAG → N chained entries in execution order
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_full_swarm_dag_recorded_in_order() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();

    // Diamond DAG :
    //         src
    //         /  \
    //      leaf1  leaf2
    //         \  /
    //         join
    let request = req_tagged(
        vec![
            op(
                "src",
                "json_validate",
                &[],
                lit(serde_json::json!({
                    "value": "data",
                    "schema": {"type": "string"}
                })),
            ),
            op(
                "leaf1",
                "regex_extract",
                &["src"],
                lit(serde_json::json!({
                    "text": "hello world",
                    "pattern": r"\w+"
                })),
            ),
            op(
                "leaf2",
                "csv_normalize",
                &["src"],
                lit(serde_json::json!({"csv": "a,b\n1,2"})),
            ),
            op(
                "join",
                "json_validate",
                &["leaf1", "leaf2"],
                lit(serde_json::json!({
                    "value": "done",
                    "schema": {"type": "string"}
                })),
            ),
        ],
        tenant,
        request_id,
    );
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 4, "diamond DAG should yield 4 chain entries");

    // Every entry must chain-link forward (regardless of which layer ran first).
    for i in 1..chain.len() {
        assert_eq!(
            chain[i].chain_parent_id,
            Some(chain[i - 1].trace_id()),
            "entry {i} must chain-link to entry {}",
            i - 1
        );
    }

    // Verify the whole chain.  N14 : 4 ops + 1 envelope = 5 entries.
    let report = verify_chain(&*h.chain_store, &h.key_source, tenant, None)
        .await
        .unwrap();
    assert_eq!(report.entries_verified, 5);
}

// ─────────────────────────────────────────────────────────────────────
//  15. Performance : 1 000 traces verify in well under 5 seconds
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_chain_thousand_trace_verify_under_5_sec() {
    let h = make_chain_harness().await;
    let tenant = TenantId::system();

    // Push 1 000 single-op swarms — each adds one chain entry.
    let mut batch: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    let h = Arc::new(h);
    // Serial dispatch — the InMemoryHmacChainAuditWriter serialises per
    // tenant via its tip mutex.  We measure serial throughput here ; the
    // multi-tenant case is covered in test 12.
    let started = std::time::Instant::now();
    for i in 0..1_000u32 {
        let harness = Arc::clone(&h);
        // Avoid building 1 000 join handles : serial run keeps the test
        // hermetic + fast under contention.
        let _ = harness
            .dispatcher
            .dispatch(req_tagged(
                vec![op_validate_obj(&format!("op-{i}"))],
                tenant,
                Uuid::new_v4(),
            ))
            .await
            .unwrap();
        // Drain the dummy vector to keep clippy quiet.
        if i == 0 {
            batch.clear();
        }
    }
    let dispatch_elapsed = started.elapsed();

    // 2 000 entries are now in the chain : 1 000 swarms × (1 op + 1 envelope).
    let total = h.chain_store.total_entries().await.unwrap();
    assert_eq!(total, 2_000);

    // verify_chain should be well under 5 s.
    let verify_started = std::time::Instant::now();
    let report = verify_chain(&*h.chain_store, &h.key_source, tenant, None)
        .await
        .unwrap();
    let verify_elapsed = verify_started.elapsed();

    assert_eq!(report.entries_verified, 2_000);
    assert!(
        verify_elapsed.as_secs() < 5,
        "verify_chain took {verify_elapsed:?} for 1 000 entries (budget 5 s)"
    );
    // Sanity : dispatch budget — should not blow up either (debug builds
    // are slow ; allow 30 s before the test gets flagged).
    assert!(
        dispatch_elapsed.as_secs() < 30,
        "dispatch loop took {dispatch_elapsed:?} for 1 000 ops"
    );
}
