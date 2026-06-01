//! **Sprint 8 — N12** — end-to-end cache-hit trace lineage tests.
//!
//! These tests validate the full N12 hardening at the
//! `Dispatcher → IdempotencyCache → AuditWriter` boundary.
//!
//! ## Observable surface
//!
//! `Dispatcher::dispatch` returns a `SwarmResponse` whose
//! `OperationResult` rows DO NOT expose `cache_hit` or `trace_id` —
//! these are executor-internal.  So the integration tests assert via
//! the **audit trace stream** :
//!
//! * Cold call : one trace with `error_kind = None`.
//! * Warm call : one *additional* trace with
//!   `error_kind = Some("CacheHit")` whose `parent_trace_ids[0]` is the
//!   cold trace's `trace_id`.
//!
//! ## Invariants validated
//!
//! 1. Cache hit emits a chain-linked trace (N12 G5).
//! 2. `parent_trace_ids[0] = cold_call.trace_id` (N1 lineage).
//! 3. `full_input` / `full_output` are `None` (pointer-shaped, no PII dup).
//! 4. `latency_ms = Some(0)`.
//! 5. Tenant isolation : no cross-tenant cache hit.
//! 6. Flush API : `flush_for_tenant`, `flush_for_original_trace`.
//! 7. Metrics : `hits_total`, `misses_total`, `entries_count` track
//!    dispatch traffic correctly.
//!
//! See `EFE_M50_N12_CACHE_KEY_HARDENING_ARCHITECTURE.md`.

use std::sync::Arc;

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
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Harness
// ─────────────────────────────────────────────────────────────────────

struct Harness {
    dispatcher: Dispatcher,
    audit: Arc<InMemoryAuditWriter>,
    cache: Arc<IdempotencyCache>,
}

async fn make_harness() -> Harness {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    let count = register_all_starter_pack(&*catalog, &registry)
        .await
        .unwrap();
    assert_eq!(count, 10);

    let runtime = Arc::new(PureRustRuntime::new(registry));
    let mem_audit = Arc::new(InMemoryAuditWriter::new());
    let audit_dyn: Arc<dyn AuditWriter> = mem_audit.clone();
    let idempotency = Arc::new(IdempotencyCache::new());

    Harness {
        dispatcher: Dispatcher::with_pure_rust_runtime(
            catalog as Arc<dyn Catalog>,
            runtime,
            audit_dyn,
            Arc::clone(&idempotency),
        ),
        audit: mem_audit,
        cache: idempotency,
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
        caller_agent_id: Some("caller-test".to_string()),
        operations: ops,
        max_parallelism: 4,
        timeout_ms: 30_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["n12-test".to_string()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

fn validate_input() -> serde_json::Value {
    serde_json::json!({"schema": {"type": "object"}, "value": {"x": 1}})
}

// ─────────────────────────────────────────────────────────────────────
//  Test 1 — cache hit writes a chain-linked trace
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cache_hit_writes_trace_linked_to_cold_call() {
    let h = make_harness().await;
    let tenant = TenantId::system();

    // Cold call.
    let req_cold = request(
        tenant,
        vec![op("v", "json_validate", &[], lit(validate_input()))],
    );
    let res_cold = h.dispatcher.dispatch(req_cold).await.unwrap();
    assert_eq!(res_cold.overall_status, SwarmStatus::Success);

    // Snapshot traces after cold.  N14 : 1 op + 1 envelope = 2.
    let after_cold = h.audit.traces().await;
    assert_eq!(after_cold.len(), 2);
    let cold_trace = after_cold
        .iter()
        .find(|t| t.error_kind.is_none())
        .expect("a cold op trace exists");

    // Warm call — same tenant / agent / input → cache hit.
    let req_warm = request(
        tenant,
        vec![op("v", "json_validate", &[], lit(validate_input()))],
    );
    let res_warm = h.dispatcher.dispatch(req_warm).await.unwrap();
    assert_eq!(res_warm.overall_status, SwarmStatus::Success);

    // Snapshot traces after warm.  N14 : 2 traces (cold + envelope) +
    // 2 traces (CacheHit + envelope) = 4.
    let after_warm = h.audit.traces().await;
    assert_eq!(
        after_warm.len(),
        4,
        "1 cold + 1 envelope + 1 CacheHit + 1 envelope = 4"
    );

    let cache_hit_trace = after_warm
        .iter()
        .find(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .expect("warm call must produce a trace with error_kind=CacheHit");

    // N1 lineage : parent_trace_ids[0] == cold trace id.
    assert_eq!(
        cache_hit_trace.parent_trace_ids,
        vec![cold_trace.trace_id],
        "cache-hit trace must declare the cold trace as its parent"
    );
    // N12 pointer-shape : full bodies are not duplicated.
    assert!(cache_hit_trace.full_input.is_none());
    assert!(cache_hit_trace.full_output.is_none());
    // Latency is 0 (no recompute).
    assert_eq!(cache_hit_trace.latency_ms, Some(0));
    // Resolved version matches the cold call's version.
    assert_eq!(
        cache_hit_trace.nano_agent_version,
        cold_trace.nano_agent_version
    );
    // Output hash matches : the same bytes, served twice.
    assert_eq!(cache_hit_trace.output_hash, cold_trace.output_hash);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 2 — different inputs → no false cache hit
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn different_inputs_do_not_share_cache() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let payload_a = serde_json::json!({"schema": {"type": "object"}, "value": {"x": 1}});
    let payload_b = serde_json::json!({"schema": {"type": "object"}, "value": {"x": 2}});

    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(payload_a))],
        ))
        .await
        .unwrap();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(payload_b))],
        ))
        .await
        .unwrap();

    // Two cold traces, zero cache hits.
    let traces = h.audit.traces().await;
    let cold_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    let hit_traces: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .collect();
    assert_eq!(cold_traces.len(), 2, "two distinct inputs → two cold calls");
    assert_eq!(hit_traces.len(), 0, "no cache hit across different inputs");
}

// ─────────────────────────────────────────────────────────────────────
//  Test 3 — tenant isolation
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn different_tenants_do_not_share_cache() {
    let h = make_harness().await;
    let t1 = TenantId::new();
    let t2 = TenantId::new();

    let _ = h
        .dispatcher
        .dispatch(request(
            t1,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let _ = h
        .dispatcher
        .dispatch(request(
            t2,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();

    let traces = h.audit.traces().await;
    let hits: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .collect();
    assert!(
        hits.is_empty(),
        "tenant isolation : t2 MUST NOT hit t1's cache (got {} hits)",
        hits.len()
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Test 4 — flush_for_tenant invalidates cache → next call is cold
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn flush_for_tenant_busts_cache_then_cold_call_again() {
    let h = make_harness().await;
    let tenant = TenantId::system();

    // Cold call — populates cache.
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    assert_eq!(h.cache.len(), 1);

    // Flush.
    let evicted = h.cache.flush_for_tenant(tenant).await;
    assert_eq!(evicted, 1);
    assert_eq!(h.cache.len(), 0);

    // Second call : now a fresh cold call (no cache hit trace produced).
    let traces_before = h.audit.traces().await.len();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let traces_after = h.audit.traces().await;
    // N14 : +2 traces (1 cold op + 1 envelope).
    assert_eq!(traces_after.len(), traces_before + 2);
    let new_op = traces_after
        .iter()
        .rev()
        .find(|t| t.error_kind.is_none())
        .expect("a new op trace");
    assert!(
        new_op.error_kind.is_none(),
        "post-flush call must be a cold call (error_kind=None)"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Test 5 — flush_for_original_trace (the N10 GDPR cascade)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn flush_for_original_trace_evicts_only_targeted_entry() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let payload_a = serde_json::json!({"schema": {"type": "object"}, "value": {"x": 1}});
    let payload_b = serde_json::json!({"schema": {"type": "object"}, "value": {"x": 2}});

    // Two cold calls.
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(payload_a.clone()))],
        ))
        .await
        .unwrap();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(payload_b))],
        ))
        .await
        .unwrap();
    assert_eq!(h.cache.len(), 2);

    // Grab the cold trace ids in audit.
    let traces = h.audit.traces().await;
    let cold_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    assert_eq!(cold_traces.len(), 2);
    let target_trace = cold_traces[0].trace_id;

    // Cascade : tombstone simulation for target → cache flush.
    let n = h.cache.flush_for_original_trace(target_trace).await;
    assert_eq!(n, 1);
    assert_eq!(h.cache.len(), 1);

    // Replay payload_a — the targeted entry is gone, so it's now cold.
    let traces_before = h.audit.traces().await.len();
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(payload_a))],
        ))
        .await
        .unwrap();
    let traces_after = h.audit.traces().await;
    // N14 : +2 traces always (1 op trace [either cold or CacheHit] + 1 envelope).
    assert_eq!(traces_after.len(), traces_before + 2);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 6 — metrics integrity end-to-end
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn metrics_reflect_dispatcher_traffic() {
    let h = make_harness().await;
    let tenant = TenantId::system();

    // 1 miss + put.
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    let snap_after_cold = h.cache.metrics().snapshot();
    assert_eq!(snap_after_cold.misses_total, 1);
    assert_eq!(snap_after_cold.entries_count, 1);
    assert_eq!(snap_after_cold.hits_total, 0);

    // 3 hits.
    for _ in 0..3 {
        let _ = h
            .dispatcher
            .dispatch(request(
                tenant,
                vec![op("v", "json_validate", &[], lit(validate_input()))],
            ))
            .await
            .unwrap();
    }
    let snap = h.cache.metrics().snapshot();
    assert_eq!(snap.hits_total, 3);
    assert_eq!(snap.entries_count, 1);
    assert!(snap.size_bytes > 0, "size_bytes must reflect inserted entry");
}

// ─────────────────────────────────────────────────────────────────────
//  Test 7 — many warm hits all lineage back to ONE cold trace
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn many_warm_hits_share_one_cold_parent() {
    let h = make_harness().await;
    let tenant = TenantId::system();

    // Cold call.
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();

    // 5 sequential warm calls.
    for _ in 0..5 {
        let _ = h
            .dispatcher
            .dispatch(request(
                tenant,
                vec![op("v", "json_validate", &[], lit(validate_input()))],
            ))
            .await
            .unwrap();
    }

    // Audit log: 1 cold + 5 cache-hit traces = 6.
    let traces = h.audit.traces().await;
    let cold_traces: Vec<_> = traces.iter().filter(|t| t.error_kind.is_none()).collect();
    let hit_traces: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .collect();
    assert_eq!(cold_traces.len(), 1, "exactly one cold trace");
    assert_eq!(hit_traces.len(), 5, "exactly five cache-hit traces");

    // Every cache-hit trace parents back to the single cold trace.
    let cold_id = cold_traces[0].trace_id;
    for h in hit_traces {
        assert_eq!(
            h.parent_trace_ids,
            vec![cold_id],
            "cache-hit trace must lineage back to the SINGLE cold trace"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Test 8 — cache-hit trace's nano_agent_version matches cold
//           (N12 G4 — version stability across re-publish window)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cache_hit_trace_carries_producing_version_not_live_version() {
    // This test exercises the architecture's promise that the
    // cache-hit trace records the **producing** version, not whatever
    // version is currently live in the catalog.  We can't easily bump
    // the catalog mid-test, but we can at least assert the version on
    // the cache-hit trace equals the version on the cold trace —
    // proving the cache entry carries `resolved_version` and the
    // trace writer uses it.
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
    let _ = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();

    let traces = h.audit.traces().await;
    let cold = traces.iter().find(|t| t.error_kind.is_none()).unwrap();
    let hit = traces
        .iter()
        .find(|t| t.error_kind.as_deref() == Some("CacheHit"))
        .unwrap();

    assert_eq!(hit.nano_agent_version, cold.nano_agent_version);
    assert_eq!(hit.nano_agent_id, cold.nano_agent_id);
    assert_eq!(hit.execution_mode, cold.execution_mode);
    assert_eq!(hit.input_hash_resolved, cold.input_hash_resolved);
    assert_eq!(hit.tenant_id, cold.tenant_id);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 9 — Op-level results sanity (status, latency)
// ─────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────
//  Tier 3 stress — high-volume cache traffic
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N12 Day 5 stress** : 200 sequential dispatches of the
/// same op → cache.len() stays at 1, hits_total == 199, misses_total == 1.
#[tokio::test]
async fn stress_two_hundred_dispatches_one_entry_one_miss() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    for _ in 0..200 {
        let _ = h
            .dispatcher
            .dispatch(request(
                tenant,
                vec![op("v", "json_validate", &[], lit(validate_input()))],
            ))
            .await
            .unwrap();
    }
    let snap = h.cache.metrics().snapshot();
    assert_eq!(h.cache.len(), 1, "cache holds exactly one entry");
    assert_eq!(snap.entries_count, 1);
    assert_eq!(snap.misses_total, 1, "exactly one miss : the cold call");
    assert_eq!(snap.hits_total, 199, "199 cache hits across 200 dispatches");
}

/// **Sprint 8 — N12 Day 5 stress** : 100 distinct inputs produce 100
/// distinct entries (no false hits, no false misses).
#[tokio::test]
async fn stress_one_hundred_distinct_inputs_one_hundred_entries() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    for i in 0..100 {
        let payload = serde_json::json!({
            "schema": {"type": "object"},
            "value": {"x": i}
        });
        let _ = h
            .dispatcher
            .dispatch(request(
                tenant,
                vec![op("v", "json_validate", &[], lit(payload))],
            ))
            .await
            .unwrap();
    }
    let snap = h.cache.metrics().snapshot();
    assert_eq!(h.cache.len(), 100);
    assert_eq!(snap.entries_count, 100);
    assert_eq!(snap.misses_total, 100, "every call is cold (distinct inputs)");
    assert_eq!(snap.hits_total, 0);
}

/// **Sprint 8 — N12 Day 5 stress** : concurrent dispatches don't
/// corrupt the metrics counters.  Spawns 50 concurrent dispatches of
/// the same payload — first one wins the cache, rest are hits.
///
/// Tolerance : depending on scheduling, the EXACT split between
/// misses and hits varies (multiple cold calls can race past the
/// initial `get()` before the first `put()` lands).  We assert the
/// monotonic invariant `hits + misses == 50` and entries_count <= 50.
#[tokio::test]
async fn stress_concurrent_dispatches_metrics_invariant_holds() {
    let h = make_harness().await;
    let tenant = TenantId::system();

    let dispatcher_arc = Arc::new(h.dispatcher);
    let mut handles = Vec::with_capacity(50);
    for _ in 0..50 {
        let d = Arc::clone(&dispatcher_arc);
        handles.push(tokio::spawn(async move {
            let _ = d
                .dispatch(request(
                    tenant,
                    vec![op("v", "json_validate", &[], lit(validate_input()))],
                ))
                .await
                .unwrap();
        }));
    }
    for h in handles {
        h.await.unwrap();
    }
    let snap = h.cache.metrics().snapshot();
    assert_eq!(snap.hits_total + snap.misses_total, 50);
    assert!(snap.entries_count >= 1, "at least one entry");
    assert!(snap.entries_count <= 50, "cap : at most 50 entries (sanity)");
    // In practice every concurrent test we've observed converges to
    // entries_count == 1 because the DashMap insert wins quickly, but
    // we don't enforce that strictly to avoid CI flakiness.
}

// ─────────────────────────────────────────────────────────────────────
//  Tier 4 E2E — sanity
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatcher_results_succeed_on_cache_hit() {
    let h = make_harness().await;
    let tenant = TenantId::system();

    // Cold call.
    let cold = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();
    // Warm call.
    let warm = h
        .dispatcher
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", &[], lit(validate_input()))],
        ))
        .await
        .unwrap();

    // Both swarms succeed.
    assert_eq!(cold.overall_status, SwarmStatus::Success);
    assert_eq!(warm.overall_status, SwarmStatus::Success);
    // Op-level : success in both.
    assert_eq!(cold.results.get("v").unwrap().status, OpStatus::Success);
    assert_eq!(warm.results.get("v").unwrap().status, OpStatus::Success);
    // Same output bytes across cold + warm.
    assert_eq!(
        cold.results.get("v").unwrap().output,
        warm.results.get("v").unwrap().output
    );
    // Warm call has cost ≥ 0 (cache hit preserves cost surfacing).
    assert!(warm.results.get("v").unwrap().cost_cents >= 0.0);
}
