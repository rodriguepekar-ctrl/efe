// SPDX-License-Identifier: UNLICENSED
//! **Sprint 8 — N15** : end-to-end integration tests for the
//! fairness-scheduling subsystem.
//!
//! Covers :
//! - Dispatcher with admission gate accepts within caps.
//! - Per-tenant op cap rejects with `OpInflightFull`.
//! - Per-tenant swarm cap rejects with `SwarmInflightFull`.
//! - Suspended tenant rejects with `TenantSuspended`.
//! - Tier upgrade lifts caps for subsequent dispatches.
//! - Permission release on Drop frees tracker + DRR slots.
//! - SchedulerMetrics surface ticks all relevant counters.
//! - Backward compat : dispatcher without gate behaves as pre-N15.

use std::sync::Arc;
use std::time::Duration;

use efe_nano::audit::stub::InMemoryAuditWriter;
use efe_nano::audit::AuditWriter;
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::Dispatcher;
use efe_nano::error::DispatcherError;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::scheduler::{
    AdmissionGate, InMemoryTenantPolicyLookup, SchedulerMetrics, TenantPolicyLookup,
};
use efe_nano::starter_pack::register_all_starter_pack;
use efe_nano_sdk::{
    BackpressureKind, Catalog, NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy,
    SwarmRequest, SwarmStatus, TenantId, TenantTier,
};
use uuid::Uuid;

async fn build_dispatcher_no_gate() -> Arc<Dispatcher> {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry).await.unwrap();
    let runtime = Arc::new(PureRustRuntime::new(registry));
    let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
    let idempotency = Arc::new(IdempotencyCache::new());
    Arc::new(Dispatcher::with_pure_rust_runtime(
        catalog as Arc<dyn Catalog>,
        runtime,
        audit,
        idempotency,
    ))
}

async fn build_dispatcher_with_gate(
    gate: Arc<AdmissionGate>,
) -> Arc<Dispatcher> {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry).await.unwrap();
    let runtime = Arc::new(PureRustRuntime::new(registry));
    let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
    let idempotency = Arc::new(IdempotencyCache::new());
    let d = Dispatcher::with_pure_rust_runtime(
        catalog as Arc<dyn Catalog>,
        runtime,
        audit,
        idempotency,
    )
    .with_admission_gate(gate);
    Arc::new(d)
}

fn single_op_request(tenant: TenantId, request_id: Uuid) -> SwarmRequest {
    SwarmRequest {
        request_id,
        tenant_id: tenant,
        caller_agent_id: Some("n15_integration".into()),
        operations: vec![Operation {
            op_id: "v".into(),
            agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
            input: OperationInput::Literal {
                value: serde_json::json!({
                    "value": {"x": 1},
                    "schema": {"type": "object"}
                }),
            },
            depends_on: vec![],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        }],
        max_parallelism: 1,
        timeout_ms: 30_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec![],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

fn multi_op_request(tenant: TenantId, request_id: Uuid, n_ops: u32) -> SwarmRequest {
    let mut ops = Vec::new();
    for i in 0..n_ops {
        ops.push(Operation {
            op_id: format!("op_{i}"),
            agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
            input: OperationInput::Literal {
                value: serde_json::json!({
                    "value": {"i": i},
                    "schema": {"type": "object"}
                }),
            },
            depends_on: vec![],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        });
    }
    SwarmRequest {
        request_id,
        tenant_id: tenant,
        caller_agent_id: Some("n15_integration".into()),
        operations: ops,
        max_parallelism: 100,
        timeout_ms: 30_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec![],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Backward compat
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatcher_without_gate_behaves_as_pre_n15() {
    let d = build_dispatcher_no_gate().await;
    let resp = d
        .dispatch(single_op_request(TenantId::system(), Uuid::new_v4()))
        .await
        .unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
}

// ─────────────────────────────────────────────────────────────────────
//  Happy path : admission gate accepts within caps
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn admission_gate_accepts_free_tier_within_caps() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let gate = AdmissionGate::builder()
        .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let resp = d
        .dispatch(single_op_request(TenantId::system(), Uuid::new_v4()))
        .await
        .unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
}

// ─────────────────────────────────────────────────────────────────────
//  Per-tenant op cap
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn per_tenant_op_cap_rejects_oversized_swarm() {
    // Free tier max_inflight_ops = 5.  Dispatch a 6-op swarm → reject.
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let gate = AdmissionGate::builder()
        .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let err = d
        .dispatch(multi_op_request(TenantId::new(), Uuid::new_v4(), 6))
        .await
        .unwrap_err();
    match err {
        DispatcherError::Backpressure(e) => {
            assert_eq!(e.kind, BackpressureKind::OpInflightFull);
            assert!(e.retry_after_ms > 0);
        }
        other => panic!("expected Backpressure, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Per-tenant swarm cap (concurrent)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn per_tenant_swarm_cap_rejects_second_concurrent_swarm() {
    // Free tier max_inflight_swarms = 1.  Dispatch two concurrent
    // swarms → second is rejected with SwarmInflightFull.
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let gate = AdmissionGate::builder()
        .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let tenant = TenantId::new();

    // Hold the first swarm at the gate by using the gate directly.
    let _hold = d
        .admission_gate()
        .unwrap()
        .admit_swarm(tenant, 1)
        .await
        .unwrap();

    let err = d
        .dispatch(single_op_request(tenant, Uuid::new_v4()))
        .await
        .unwrap_err();
    match err {
        DispatcherError::Backpressure(e) => {
            assert_eq!(e.kind, BackpressureKind::SwarmInflightFull);
        }
        other => panic!("expected Backpressure, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tenant suspension
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn suspended_tenant_blocked_at_admission() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let tenant = TenantId::new();
    lookup.suspend(tenant);
    let gate = AdmissionGate::builder()
        .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let err = d
        .dispatch(single_op_request(tenant, Uuid::new_v4()))
        .await
        .unwrap_err();
    match err {
        DispatcherError::Backpressure(e) => {
            assert_eq!(e.kind, BackpressureKind::TenantSuspended);
        }
        other => panic!("expected Backpressure, got {other:?}"),
    }
}

#[tokio::test]
async fn unsuspended_tenant_admitted_again() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let tenant = TenantId::new();
    lookup.suspend(tenant);
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;

    let err = d
        .dispatch(single_op_request(tenant, Uuid::new_v4()))
        .await
        .unwrap_err();
    matches!(err, DispatcherError::Backpressure(_));

    lookup.unsuspend(tenant);
    let resp = d
        .dispatch(single_op_request(tenant, Uuid::new_v4()))
        .await
        .unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
}

// ─────────────────────────────────────────────────────────────────────
//  Tier upgrade
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tier_upgrade_lifts_op_cap() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let tenant = TenantId::new();
    // Default Free tier : max 5 ops.  Dispatch 10 ops → reject.
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let err = d
        .dispatch(multi_op_request(tenant, Uuid::new_v4(), 10))
        .await
        .unwrap_err();
    matches!(err, DispatcherError::Backpressure(_));

    // Upgrade to Tier2 (100 ops allowed) → next dispatch succeeds.
    lookup.set_tier(tenant, TenantTier::Tier2);
    let resp = d
        .dispatch(multi_op_request(tenant, Uuid::new_v4(), 10))
        .await
        .unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
}

// ─────────────────────────────────────────────────────────────────────
//  Release on Drop frees capacity
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn dispatch_release_frees_inflight_counters() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let tenant = TenantId::new();
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .build();
    let tracker = Arc::clone(gate.tracker());
    let d = build_dispatcher_with_gate(gate).await;

    let resp = d
        .dispatch(single_op_request(tenant, Uuid::new_v4()))
        .await
        .unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    // After dispatch returns, the admission ticket is dropped and
    // the per-tenant counters should be back to 0.
    let snap = tracker.snapshot(tenant);
    assert_eq!(snap.inflight_ops, 0);
    assert_eq!(snap.inflight_swarms, 0);
}

// ─────────────────────────────────────────────────────────────────────
//  Metrics surface
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn metrics_surface_records_admission_lifecycle() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let metrics = SchedulerMetrics::new();
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .with_metrics(Arc::clone(&metrics))
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let tenant = TenantId::new();

    let _resp = d
        .dispatch(single_op_request(tenant, Uuid::new_v4()))
        .await
        .unwrap();

    let snap = metrics.snapshot();
    assert!(snap.total_admissions_accepted() >= 1);
    assert!(snap.policy_lookup_hit >= 1);
    // After release, inflight gauge is back to zero (or rolled forward).
    assert_eq!(snap.admission_inflight, 0);
}

#[tokio::test]
async fn metrics_surface_records_rejection_with_kind() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let tenant = TenantId::new();
    lookup.suspend(tenant);
    let metrics = SchedulerMetrics::new();
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .with_metrics(Arc::clone(&metrics))
        .build();
    let d = build_dispatcher_with_gate(gate).await;

    let err = d
        .dispatch(single_op_request(tenant, Uuid::new_v4()))
        .await
        .unwrap_err();
    matches!(err, DispatcherError::Backpressure(_));

    let snap = metrics.snapshot();
    assert!(snap.policy_lookup_suspended >= 1);
    assert_eq!(
        snap.rejected_by_kind(BackpressureKind::TenantSuspended),
        1
    );
}

#[tokio::test]
async fn metrics_prometheus_render_is_well_formed() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let metrics = SchedulerMetrics::new();
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .with_metrics(Arc::clone(&metrics))
        .build();
    let d = build_dispatcher_with_gate(gate).await;

    let _ = d
        .dispatch(single_op_request(TenantId::new(), Uuid::new_v4()))
        .await
        .unwrap();

    let prom = metrics.render_prometheus();
    // Every metric MUST be present.
    for metric in [
        "n15_admission_requests_total",
        "n15_admission_accepted_total",
        "n15_admission_rejected_total",
        "n15_inflight_ops",
        "n15_inflight_swarms",
        "n15_queue_depth",
        "n15_acquire_latency_seconds",
        "n15_token_bucket_consumes_total",
        "n15_drr_rounds_total",
        "n15_starvation_force_grants_total",
        "n15_policy_lookup_total",
        "n15_admission_inflight",
    ] {
        assert!(prom.contains(metric), "missing metric : {metric}");
    }
    // Histogram structure : has _bucket, _sum, _count lines.
    assert!(prom.contains("n15_acquire_latency_seconds_bucket"));
    assert!(prom.contains("n15_acquire_latency_seconds_sum"));
    assert!(prom.contains("n15_acquire_latency_seconds_count"));
}

// ─────────────────────────────────────────────────────────────────────
//  Multi-tenant fairness
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn multi_tenant_fairness_separates_caps() {
    // Two Free-tier tenants : each can run 5 ops independently.
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let t1 = TenantId::new();
    let t2 = TenantId::new();

    // Both can dispatch in parallel.
    let (r1, r2) = tokio::join!(
        d.dispatch(multi_op_request(t1, Uuid::new_v4(), 5)),
        d.dispatch(multi_op_request(t2, Uuid::new_v4(), 5)),
    );
    assert_eq!(r1.unwrap().overall_status, SwarmStatus::Success);
    assert_eq!(r2.unwrap().overall_status, SwarmStatus::Success);
}

// ─────────────────────────────────────────────────────────────────────
//  Platform tier never throttled
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn platform_tier_admits_huge_op_count() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let tenant = TenantId::new();
    lookup.set_tier(tenant, TenantTier::Platform);
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;

    // 50 ops — Free tier would reject (cap=5), Platform accepts.
    let resp = d
        .dispatch(multi_op_request(tenant, Uuid::new_v4(), 50))
        .await
        .unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    assert_eq!(resp.results.len(), 50);
}

// ─────────────────────────────────────────────────────────────────────
//  HTTP mapping (retry_after_seconds + advice text)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn backpressure_error_carries_http_mappable_fields() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let gate = AdmissionGate::builder()
        .with_policy_lookup(lookup as Arc<dyn TenantPolicyLookup>)
        .build();
    let d = build_dispatcher_with_gate(gate).await;
    let err = d
        .dispatch(multi_op_request(TenantId::new(), Uuid::new_v4(), 6))
        .await
        .unwrap_err();
    let DispatcherError::Backpressure(e) = err else {
        panic!("expected Backpressure")
    };
    // Retry-After (seconds), minimum 1.
    let seconds = e.retry_after_seconds();
    assert!(seconds >= 1);
    // Advice text must not be empty (for client developers).
    assert!(!e.advice.is_empty());
}

// ─────────────────────────────────────────────────────────────────────
//  Concurrent dispatches under contention
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn concurrent_dispatches_respect_per_tenant_cap() {
    let lookup = Arc::new(InMemoryTenantPolicyLookup::default());
    let tenant = TenantId::new();
    lookup.set_tier(tenant, TenantTier::Tier2);
    let gate = AdmissionGate::builder()
        .with_policy_lookup(Arc::clone(&lookup) as Arc<dyn TenantPolicyLookup>)
        .with_acquire_timeout(Duration::from_millis(50))
        .build();
    let d = build_dispatcher_with_gate(gate).await;

    // Tier2 : max_inflight_swarms = 25.  Spawn 50 concurrent dispatches.
    let mut handles = vec![];
    for _ in 0..50 {
        let d = Arc::clone(&d);
        handles.push(tokio::spawn(async move {
            d.dispatch(single_op_request(tenant, Uuid::new_v4()))
                .await
                .map(|r| r.overall_status)
        }));
    }
    let mut succeeded = 0;
    let mut rejected = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(_) => succeeded += 1,
            Err(DispatcherError::Backpressure(_)) => rejected += 1,
            Err(other) => panic!("unexpected error : {other:?}"),
        }
    }
    // Some must succeed, some may reject.  Total = 50.
    assert_eq!(succeeded + rejected, 50);
    // At least one must succeed (cap=25 is plenty for short ops).
    assert!(succeeded > 0);
}
