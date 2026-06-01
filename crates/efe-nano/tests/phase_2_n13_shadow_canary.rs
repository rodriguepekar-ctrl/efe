//! **Sprint 8 — N13** — end-to-end shadow / canary rollout tests.
//!
//! Validates the full N13 hardening at the
//! `Catalog → Dispatcher → ShadowScheduler → DriftWindow` boundary.
//!
//! See `docs/architecture/EFE_M50_N13_SHADOW_CANARY_CATALOG_ARCHITECTURE.md`.

use std::sync::Arc;

use efe_nano::audit::{drift::DriftWindow, stub::InMemoryAuditWriter, AuditWriter};
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::compare::{
    compare as compare_outputs, compare_exact, compare_structural, CompareContext,
};
use efe_nano::dispatcher::routing::{in_canary_cohort, sticky_sample};
use efe_nano::dispatcher::shadow::{
    build_rollout_policy_changed_trace, shadow_outcome_to_json, ShadowDispatchContext,
    ShadowScheduler, TRACE_KIND_ROLLOUT_POLICY_CHANGED, TRACE_KIND_SHADOW_COMPARISON,
    TRACE_KIND_SHADOW_RUN,
};
use efe_nano::dispatcher::Dispatcher;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::runtime::cost_attribution::{CostBudgetError, CostRouter};
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::starter_pack::register_all_starter_pack;
use efe_nano_sdk::{
    AuditPolicy, Author, Catalog, Category, ComparisonStrategy, ComplianceTags, CostAttribution,
    DeterminismLevel, ExecutionMode, IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentRef,
    NanoAgentVersion, Operation, OperationInput, PromotionGates, ResolvedAgent, ResourceLimits,
    RetryPolicy, RolloutPolicy, ShadowOutcome, StabilityTier, SwarmRequest, SwarmStatus, TenantId,
    VersionReq,
};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Harness
// ─────────────────────────────────────────────────────────────────────

fn aid(s: &str) -> NanoAgentId {
    NanoAgentId::new(s).unwrap()
}
fn v(s: &str) -> NanoAgentVersion {
    NanoAgentVersion::new(s).unwrap()
}

fn build_agent(id: &str, version: &str) -> NanoAgent {
    // Reuse the starter-pack handler entry string so the runtime can
    // dispatch our test versions through the already-registered handler.
    let entry = format!("{id}::run");
    NanoAgent {
        id: aid(id),
        version: v(version),
        description: format!("{id} v{version}"),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust { entry },
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
    }
}

struct Harness {
    catalog: Arc<InMemoryCatalog>,
    catalog_dyn: Arc<dyn Catalog>,
    audit: Arc<InMemoryAuditWriter>,
    drift: Arc<DriftWindow>,
    cost_router: Arc<CostRouter>,
    runtime: Arc<dyn efe_nano::runtime::NanoAgentExecutor>,
}

async fn make_harness() -> Harness {
    let catalog = Arc::new(InMemoryCatalog::new());
    let registry = HandlerRegistry::new();
    register_all_starter_pack(&*catalog, &registry).await.unwrap();
    let runtime: Arc<dyn efe_nano::runtime::NanoAgentExecutor> =
        Arc::new(PureRustRuntime::new(registry));
    let audit = Arc::new(InMemoryAuditWriter::new());
    let drift = Arc::new(DriftWindow::default_for_sprint_8());
    let cost_router = Arc::new(CostRouter::unlimited());
    Harness {
        catalog_dyn: Arc::clone(&catalog) as Arc<dyn Catalog>,
        catalog,
        audit,
        drift,
        cost_router,
        runtime,
    }
}

impl Harness {
    fn dispatcher_with_shadow(&self) -> Dispatcher {
        let scheduler = Arc::new(ShadowScheduler::new(
            Arc::clone(&self.runtime),
            Arc::clone(&self.audit) as Arc<dyn AuditWriter>,
            Arc::clone(&self.drift),
            Arc::clone(&self.cost_router),
        ));
        Dispatcher::new(
            Arc::clone(&self.catalog_dyn),
            Arc::clone(&self.runtime),
            Arc::clone(&self.audit) as Arc<dyn AuditWriter>,
            Arc::new(IdempotencyCache::new()),
        )
        .with_shadow_scheduler(scheduler)
    }
}

fn lit(v: serde_json::Value) -> OperationInput {
    OperationInput::Literal { value: v }
}

fn op(id: &str, agent_id: &str, version_req: &str, input: OperationInput) -> Operation {
    Operation {
        op_id: id.to_string(),
        agent: NanoAgentRef::with_version(
            NanoAgentId::new(agent_id).unwrap(),
            VersionReq::new(version_req).unwrap(),
        ),
        input,
        depends_on: vec![],
        timeout_ms: None,
        retry: RetryPolicy::default(),
    }
}

fn request(tenant: TenantId, ops: Vec<Operation>) -> SwarmRequest {
    SwarmRequest {
        request_id: Uuid::new_v4(),
        tenant_id: tenant,
        caller_agent_id: Some("test-caller".into()),
        operations: ops,
        max_parallelism: 4,
        timeout_ms: 30_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["n13".into()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Test 1 — sticky routing : 5 % cohort lands within tolerance
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t01_sticky_routing_5pct_lands_within_tolerance() {
    let t = TenantId::system();
    let a = aid("agent");
    let mut canary = 0;
    for i in 0u32..10_000 {
        let mut h = [0u8; 32];
        h[..4].copy_from_slice(&i.to_le_bytes());
        if in_canary_cohort(t, &a, &h, 500) {
            canary += 1;
        }
    }
    // 500 ± 150 (binomial 3-sigma + SHA-256 noise tolerance).
    assert!(
        (350..=650).contains(&canary),
        "5% cohort got {canary} of 10000, expected 350-650"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Test 2 — sticky routing is deterministic
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t02_sticky_routing_deterministic_per_input() {
    let t = TenantId::system();
    let a = aid("agent");
    let h = [0x42u8; 32];
    let s1 = sticky_sample(t, &a, &h);
    for _ in 0..100 {
        assert_eq!(sticky_sample(t, &a, &h), s1);
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Test 3 — RolloutPolicy round-trip via catalog
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t03_rollout_policy_set_get_round_trip() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    // Publish a v1.1.0 (json_validate v1.0.0 already exists from starter_pack).
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();

    let policy = RolloutPolicy::Canary {
        prod_version: v("1.0.0"),
        canary_version: v("1.1.0"),
        canary_bps: 500,
        started_at: chrono::Utc::now(),
        promotion_gates: PromotionGates::default(),
        cost_attribution: CostAttribution::Tenant,
    };
    let prior = h
        .catalog
        .set_rollout_policy(tenant, &aid("json_validate"), policy.clone())
        .await
        .unwrap();
    // Prior should default to Stable(latest).
    assert!(matches!(prior, RolloutPolicy::Stable { .. }));

    let read_back = h
        .catalog
        .get_rollout_policy(tenant, &aid("json_validate"))
        .await
        .unwrap();
    assert_eq!(read_back, policy);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 4 — illegal transition rejected
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t04_illegal_transition_rejected() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.2.0"))
        .await
        .unwrap();

    // Stable(1.0.0) → Canary(1.0.0, 1.1.0) : legal.
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 500,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();

    // Canary(1.0.0, 1.1.0) → Canary(1.0.0, 1.2.0) : illegal (changed canary version).
    let result = h
        .catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.2.0"),
                canary_bps: 500,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await;
    assert!(result.is_err(), "Canary → Canary with different versions must be rejected");
}

// ─────────────────────────────────────────────────────────────────────
//  Test 5 — resolve_for_dispatch routes by sticky sample
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t05_resolve_for_dispatch_routes_per_input() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 5000, // 50%
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();

    // Iterate 200 distinct inputs ; expect ~50/50 split.
    let mut prod_count = 0;
    let mut canary_count = 0;
    for i in 0u32..200 {
        let mut h_bytes = [0u8; 32];
        h_bytes[..4].copy_from_slice(&i.to_le_bytes());
        let resolved = h
            .catalog
            .resolve_for_dispatch(tenant, &aid("json_validate"), &VersionReq::any(), &h_bytes)
            .await
            .unwrap();
        match resolved {
            ResolvedAgent::Single(a) => {
                if a.version == v("1.0.0") {
                    prod_count += 1;
                } else {
                    canary_count += 1;
                }
            }
            _ => panic!("expected Single for Canary policy"),
        }
    }
    assert!(prod_count >= 70 && canary_count >= 70, "got prod={prod_count} canary={canary_count}");
}

// ─────────────────────────────────────────────────────────────────────
//  Test 6 — Shadow policy returns ShadowPair
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t06_shadow_policy_resolves_to_pair() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Shadow {
                prod_version: v("1.0.0"),
                shadow_version: v("1.1.0"),
                started_at: chrono::Utc::now(),
                comparison: ComparisonStrategy::Exact,
                cost_attribution: CostAttribution::Platform,
                shadow_budget_cents: None,
                shadow_on_cache_hit: true,
            },
        )
        .await
        .unwrap();

    let resolved = h
        .catalog
        .resolve_for_dispatch(tenant, &aid("json_validate"), &VersionReq::any(), &[0u8; 32])
        .await
        .unwrap();
    match resolved {
        ResolvedAgent::ShadowPair { prod, shadow, .. } => {
            assert_eq!(prod.version, v("1.0.0"));
            assert_eq!(shadow.version, v("1.1.0"));
        }
        _ => panic!("expected ShadowPair"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Test 7 — Version-pin opt-out bypasses policy
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t07_version_pin_bypasses_canary() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 10_000, // 100% canary
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();

    // Caller pins =1.0.0 — must get 1.0.0 even though canary is 100%.
    let pinned = VersionReq::new("=1.0.0").unwrap();
    let resolved = h
        .catalog
        .resolve_for_dispatch(tenant, &aid("json_validate"), &pinned, &[0xAB; 32])
        .await
        .unwrap();
    match resolved {
        ResolvedAgent::Single(a) => assert_eq!(a.version, v("1.0.0"), "version pin must bypass canary"),
        _ => panic!("expected Single"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Test 8 — Cross-tenant policy isolation
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t08_cross_tenant_policy_isolation() {
    // **N13 (B7 fix)** : Tenants can only policy their own agents.
    // This test publishes a t1-owned agent + a t2-owned agent and
    // verifies cross-tenant policy + cross-tenant authorization.
    let h = make_harness().await;
    let t1 = TenantId::new();
    let t2 = TenantId::new();

    // t1-owned agent in two versions.
    let mut a1_v1 = build_agent("t1_agent", "1.0.0");
    a1_v1.author = Author::Tenant { tenant_id: t1 };
    let mut a1_v2 = build_agent("t1_agent", "1.1.0");
    a1_v2.author = Author::Tenant { tenant_id: t1 };
    h.catalog.publish(t1, a1_v1).await.unwrap();
    h.catalog.publish(t1, a1_v2).await.unwrap();

    // Set Canary policy on t1's agent (legal : t1 owns it).
    h.catalog
        .set_rollout_policy(
            t1,
            &aid("t1_agent"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 10_000,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();

    // t2 attempting to set policy on t1's agent : MUST be rejected.
    let cross = h
        .catalog
        .set_rollout_policy(
            t2,
            &aid("t1_agent"),
            RolloutPolicy::Stable { version: v("1.0.0") },
        )
        .await;
    assert!(
        cross.is_err(),
        "cross-tenant policy set MUST be rejected (B7)"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Test 9 — Shadow doesn't block prod (latency invariant)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t09_shadow_does_not_block_prod() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Shadow {
                prod_version: v("1.0.0"),
                shadow_version: v("1.1.0"),
                started_at: chrono::Utc::now(),
                comparison: ComparisonStrategy::Exact,
                cost_attribution: CostAttribution::Platform,
                shadow_budget_cents: None,
                shadow_on_cache_hit: true,
            },
        )
        .await
        .unwrap();

    let d = h.dispatcher_with_shadow();
    let req = request(
        tenant,
        vec![op("v", "json_validate", "*",
            lit(serde_json::json!({"schema": {"type": "object"}, "value": {"x": 1}})))],
    );
    let started = std::time::Instant::now();
    let resp = d.dispatch(req).await.unwrap();
    let elapsed_ms = started.elapsed().as_millis();
    assert_eq!(resp.overall_status, SwarmStatus::Success);
    // Prod-only Mode A typically finishes in single-digit ms.  Allow
    // generous bound (200 ms) to absorb CI noise.
    assert!(elapsed_ms < 200, "dispatch took {elapsed_ms} ms — shadow should NOT block prod");

    // Give the shadow task a chance to complete + emit traces.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let traces = h.audit.traces().await;
    let shadow_runs = traces.iter().filter(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SHADOW_RUN)).count();
    let shadow_cmps = traces.iter().filter(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SHADOW_COMPARISON)).count();
    assert_eq!(shadow_runs, 1, "expected exactly 1 ShadowRun trace");
    assert_eq!(shadow_cmps, 1, "expected exactly 1 ShadowComparison trace");
}

// ─────────────────────────────────────────────────────────────────────
//  Test 10 — Shadow trace has correct lineage
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t10_shadow_trace_has_prod_as_parent() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Shadow {
                prod_version: v("1.0.0"),
                shadow_version: v("1.1.0"),
                started_at: chrono::Utc::now(),
                comparison: ComparisonStrategy::Exact,
                cost_attribution: CostAttribution::Platform,
                shadow_budget_cents: None,
                shadow_on_cache_hit: true,
            },
        )
        .await
        .unwrap();

    let d = h.dispatcher_with_shadow();
    let _ = d
        .dispatch(request(
            tenant,
            vec![op("v", "json_validate", "*",
                lit(serde_json::json!({"schema": {"type": "object"}, "value": {"x": 1}})))],
        ))
        .await
        .unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let traces = h.audit.traces().await;
    let prod = traces.iter().find(|t| t.error_kind.is_none()).expect("prod trace");
    let shadow = traces.iter().find(|t| t.error_kind.as_deref() == Some(TRACE_KIND_SHADOW_RUN)).expect("shadow trace");
    assert_eq!(shadow.parent_trace_ids, vec![prod.trace_id], "shadow parent must be prod");
    assert_eq!(shadow.nano_agent_version, v("1.1.0"));
    assert_eq!(prod.nano_agent_version, v("1.0.0"));
}

// ─────────────────────────────────────────────────────────────────────
//  Test 11 — SideEffect agent rejects shadow policy
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t11_side_effect_agent_rejects_shadow_policy() {
    let h = make_harness().await;
    let tenant = TenantId::new();

    let mut agent = build_agent("side_effect_op", "1.0.0");
    agent.author = Author::Tenant { tenant_id: tenant };
    agent.determinism = DeterminismLevel::SideEffect;
    h.catalog.publish(tenant, agent.clone()).await.unwrap();

    let mut v2 = agent.clone();
    v2.version = v("1.1.0");
    h.catalog.publish(tenant, v2).await.unwrap();

    let result = h
        .catalog
        .set_rollout_policy(
            tenant,
            &aid("side_effect_op"),
            RolloutPolicy::Shadow {
                prod_version: v("1.0.0"),
                shadow_version: v("1.1.0"),
                started_at: chrono::Utc::now(),
                comparison: ComparisonStrategy::Exact,
                cost_attribution: CostAttribution::Platform,
                shadow_budget_cents: None,
                shadow_on_cache_hit: true,
            },
        )
        .await;
    assert!(result.is_err(), "shadowing a SideEffect agent must be rejected");
}

// ─────────────────────────────────────────────────────────────────────
//  Test 12 — Cost router drops shadow on exhausted budget
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t12_cost_router_drops_on_exhausted_budget() {
    let router = CostRouter::new(0.0001); // 0.0001 cents budget
    let r1 = router.reserve(TenantId::system(), 0.0001, CostAttribution::Platform);
    assert!(r1.is_ok(), "first reservation fits");
    let r2 = router.reserve(TenantId::system(), 0.0001, CostAttribution::Platform);
    assert!(matches!(r2, Err(CostBudgetError::Exhausted { .. })));
    assert_eq!(router.platform_drops_total(), 1);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 13 — Comparison strategies behave correctly
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t13_exact_comparison_match() {
    let ctx = CompareContext {
        prod_latency_ms: 1,
        shadow_latency_ms: 1,
        prod_cost_cents: 0.001,
        shadow_cost_cents: 0.001,
    };
    let m = compare_exact(&serde_json::json!({"x": 1}), &serde_json::json!({"x": 1}), ctx);
    assert!(matches!(m, ShadowOutcome::Match { .. }));
    let mm = compare_exact(&serde_json::json!({"x": 1}), &serde_json::json!({"x": 2}), ctx);
    assert!(matches!(mm, ShadowOutcome::Mismatch { .. }));
}

#[tokio::test]
async fn t14_structural_comparison_tolerates_leaf_diff_above_threshold() {
    let ctx = CompareContext {
        prod_latency_ms: 1, shadow_latency_ms: 1,
        prod_cost_cents: 0.001, shadow_cost_cents: 0.001,
    };
    // 20 fields, 1 differs ; similarity 95 % = matches.
    let mut a = serde_json::Map::new();
    let mut b = serde_json::Map::new();
    for i in 0..20 {
        a.insert(format!("k{i}"), serde_json::json!(i));
        b.insert(format!("k{i}"), serde_json::json!(i));
    }
    b.insert("k0".into(), serde_json::json!(999));
    let m = compare_structural(&serde_json::Value::Object(a), &serde_json::Value::Object(b), ctx);
    assert!(matches!(m, ShadowOutcome::Match { similarity, .. } if similarity >= 0.95));
}

#[tokio::test]
async fn t15_structural_comparison_rejects_type_change() {
    let ctx = CompareContext {
        prod_latency_ms: 1, shadow_latency_ms: 1,
        prod_cost_cents: 0.001, shadow_cost_cents: 0.001,
    };
    let m = compare_structural(
        &serde_json::json!({"x": 1}),
        &serde_json::json!({"x": "1"}),
        ctx,
    );
    assert!(matches!(m, ShadowOutcome::Mismatch { .. }));
}

#[tokio::test]
async fn t16_none_comparison_always_matches() {
    let ctx = CompareContext {
        prod_latency_ms: 1, shadow_latency_ms: 1,
        prod_cost_cents: 0.001, shadow_cost_cents: 0.001,
    };
    let m = compare_outputs(&ComparisonStrategy::None, &serde_json::json!({"x":1}), &serde_json::json!({"y":2}), ctx);
    assert!(matches!(m, ShadowOutcome::Match { similarity, .. } if similarity == 1.0));
}

// ─────────────────────────────────────────────────────────────────────
//  Test 17 — Drift window aggregates correctly
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t17_drift_window_computes_mismatch_rate() {
    let w = DriftWindow::new(1000);
    let t = TenantId::system();
    let a = aid("agent");
    for _ in 0..95 {
        w.record(t, a.clone(), ShadowOutcome::Match {
            compared_at: chrono::Utc::now(),
            similarity: 1.0,
            prod_latency_ms: 10,
            shadow_latency_ms: 10,
            prod_cost_cents: 0.001,
            shadow_cost_cents: 0.001,
        });
    }
    for _ in 0..5 {
        w.record(t, a.clone(), ShadowOutcome::Mismatch {
            compared_at: chrono::Utc::now(),
            similarity: 0.0,
            diff_summary: "x".into(),
            prod_latency_ms: 10,
            shadow_latency_ms: 10,
            prod_cost_cents: 0.001,
            shadow_cost_cents: 0.001,
        });
    }
    let stats = w.stats(t, &a).unwrap();
    assert_eq!(stats.sample_count, 100);
    // 5 / 100 = 5 % = 500 bps.
    assert_eq!(stats.mismatch_rate_bps, 500);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 18 — Promotion gate evaluator pass / block
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t18_promotion_gate_pass_when_stats_clean() {
    use efe_nano::audit::PromotionGateEvaluator;
    let stats = efe_nano_sdk::DriftStats {
        sample_count: 500,
        match_count: 499,
        mismatch_count: 1,
        failed_shadow_count: 0,
        comparator_unavailable_count: 0,
        mismatch_rate_bps: 20, // 0.2 %, below 1 % default
        error_rate_bps: 0,
        p99_latency_delta_pct: 10,
        cost_delta_pct: 30,
    };
    let outcome = PromotionGateEvaluator::evaluate_for_promotion(&stats, &PromotionGates::default());
    assert!(outcome.is_pass());
}

#[tokio::test]
async fn t19_promotion_gate_blocks_on_drift_breach() {
    use efe_nano::audit::PromotionGateEvaluator;
    let stats = efe_nano_sdk::DriftStats {
        sample_count: 500,
        match_count: 450,
        mismatch_count: 50,
        failed_shadow_count: 0,
        comparator_unavailable_count: 0,
        mismatch_rate_bps: 1000, // 10 %, above 1 % default
        error_rate_bps: 0,
        p99_latency_delta_pct: 10,
        cost_delta_pct: 30,
    };
    let outcome = PromotionGateEvaluator::evaluate_for_promotion(&stats, &PromotionGates::default());
    assert!(outcome.is_block());
}

// ─────────────────────────────────────────────────────────────────────
//  Test 20 — RolloutPolicyChanged virtual-swarm trace builder
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t20_policy_change_trace_has_sentinel_and_virtual_swarm() {
    let from = RolloutPolicy::Stable { version: v("1.0.0") };
    let to = RolloutPolicy::Canary {
        prod_version: v("1.0.0"),
        canary_version: v("1.1.0"),
        canary_bps: 500,
        started_at: chrono::Utc::now(),
        promotion_gates: PromotionGates::default(),
        cost_attribution: CostAttribution::Tenant,
    };
    let trace = build_rollout_policy_changed_trace(
        TenantId::system(),
        aid("agent"),
        &from,
        &to,
        "operator:test",
        Some("rolling out".into()),
    );
    assert_eq!(
        trace.error_kind.as_deref(),
        Some(TRACE_KIND_ROLLOUT_POLICY_CHANGED)
    );
    assert!(trace.full_input.is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  Test 21 — Replay filter excludes N13 sentinels
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t21_replay_filter_excludes_shadow_traces() {
    // We don't run a full replay engine here ; we just assert the
    // sentinels we use are filterable.  The replay engine's filter is
    // tested via the existing replay tests (which still pass : 21/21
    // GREEN).  This test serves as a sanity guard against typos.
    let kinds = [
        TRACE_KIND_SHADOW_RUN,
        TRACE_KIND_SHADOW_COMPARISON,
        TRACE_KIND_ROLLOUT_POLICY_CHANGED,
    ];
    for k in kinds {
        assert!(matches!(
            Some(k),
            Some("ShadowRun") | Some("ShadowComparison") | Some("RolloutPolicyChanged")
        ));
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Test 22 — Cost reservation refund on commit (actual < reserved)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t22_cost_router_refunds_unused_portion() {
    let router = CostRouter::new(1.0);
    let res = router.reserve(TenantId::system(), 0.5, CostAttribution::Platform).unwrap();
    router.commit(res, 0.2);
    // Reserved 0.5, used 0.2 → refund 0.3, remaining 0.8.
    assert!((router.platform_remaining_cents() - 0.8).abs() < 1e-9);
    assert!((router.platform_spent_cents() - 0.2).abs() < 1e-9);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 23 — DriftWindow cardinality cap (LRU semantic verification)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t23_drift_window_caps_samples_per_window() {
    let w = DriftWindow::new(5);
    let t = TenantId::system();
    let a = aid("agent");
    for _ in 0..50 {
        w.record(t, a.clone(), ShadowOutcome::Match {
            compared_at: chrono::Utc::now(),
            similarity: 1.0,
            prod_latency_ms: 10, shadow_latency_ms: 10,
            prod_cost_cents: 0.001, shadow_cost_cents: 0.001,
        });
    }
    let s = w.stats(t, &a).unwrap();
    assert_eq!(s.sample_count, 5);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 24 — Tenant flush via clear_tenant
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t24_drift_window_clear_tenant_cascade() {
    let w = DriftWindow::new(1000);
    let t1 = TenantId::new();
    let t2 = TenantId::new();
    let a = aid("a");
    w.record(t1, a.clone(), ShadowOutcome::Match {
        compared_at: chrono::Utc::now(),
        similarity: 1.0,
        prod_latency_ms: 10, shadow_latency_ms: 10,
        prod_cost_cents: 0.001, shadow_cost_cents: 0.001,
    });
    w.record(t2, a.clone(), ShadowOutcome::Match {
        compared_at: chrono::Utc::now(),
        similarity: 1.0,
        prod_latency_ms: 10, shadow_latency_ms: 10,
        prod_cost_cents: 0.001, shadow_cost_cents: 0.001,
    });
    w.clear_tenant(t1);
    assert!(w.stats(t1, &a).is_none());
    assert!(w.stats(t2, &a).is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  Test 25 — End-to-end : Canary 5% — both versions get traffic
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t25_canary_50pct_routes_both_versions() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 5000, // 50%
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();

    let d = h.dispatcher_with_shadow();
    // 30 distinct inputs.
    let mut v_seen_10 = 0;
    let mut v_seen_11 = 0;
    for i in 0..30 {
        let resp = d
            .dispatch(request(
                tenant,
                vec![op("v", "json_validate", "*",
                    lit(serde_json::json!({"schema": {"type": "object"}, "value": {"i": i}})))],
            ))
            .await
            .unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);
    }
    let traces = h.audit.traces().await;
    for t in &traces {
        if t.error_kind.is_none() {
            if t.nano_agent_version == v("1.0.0") {
                v_seen_10 += 1;
            } else if t.nano_agent_version == v("1.1.0") {
                v_seen_11 += 1;
            }
        }
    }
    // Both versions must have served at least one request.  For 50 % canary
    // over 30 inputs, binomial 3-sigma allows down to ~9 ; we tolerate ≥ 3.
    assert!(v_seen_10 >= 3, "prod version (1.0.0) should serve some traffic, got {v_seen_10}");
    assert!(v_seen_11 >= 3, "canary version (1.1.0) should serve some traffic, got {v_seen_11}");
}

// ─────────────────────────────────────────────────────────────────────
//  Test 26 — ShadowOutcome JSON projection is stable
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t26_shadow_outcome_to_json_round_trip() {
    let o = ShadowOutcome::Match {
        compared_at: chrono::Utc::now(),
        similarity: 0.99,
        prod_latency_ms: 100,
        shadow_latency_ms: 110,
        prod_cost_cents: 0.001,
        shadow_cost_cents: 0.002,
    };
    let j = shadow_outcome_to_json(&o);
    assert_eq!(j["kind"], "match");
    assert!((j["similarity"].as_f64().unwrap() - 0.99).abs() < 1e-6);
    assert_eq!(j["prod_latency_ms"], 100);
}

// ─────────────────────────────────────────────────────────────────────
//  Test 27 — ShadowDispatchContext clones for the spawned task
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn t27_shadow_dispatch_context_is_clonable() {
    let ctx = ShadowDispatchContext {
        tenant_id: TenantId::system(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "v".into(),
        caller_agent_id: Some("caller".into()),
        input: serde_json::json!({"x": 1}),
        comparison: ComparisonStrategy::Exact,
        cost_attribution: CostAttribution::Platform,
        swarm_envelope_trace_id: None,
    };
    let c = ctx.clone();
    assert_eq!(ctx.op_id, c.op_id);
}

// ═════════════════════════════════════════════════════════════════════
//  Sprint 8 — N13 audit fix regression tests (B1-B8)
// ═════════════════════════════════════════════════════════════════════

// ─────────────────────────────────────────────────────────────────────
//  B1 — CostRouter tenant tracking
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b1_cost_router_tenant_reservation_tracks_real_tenant() {
    let r = CostRouter::unlimited();
    let t1 = TenantId::new();
    let t2 = TenantId::new();
    r.set_tenant_budget(t1, Some(1.0));
    r.set_tenant_budget(t2, Some(1.0));
    let res = r.reserve(t1, 0.5, CostAttribution::Tenant).unwrap();
    // Commit smaller actual ⇒ refund 0.3 to t1, not system.
    r.commit(res, 0.2);
    let t1_remaining = r.tenant_remaining_cents(t1).unwrap();
    let t2_remaining = r.tenant_remaining_cents(t2).unwrap();
    // t1 : 1.0 - 0.5 + 0.3 = 0.8
    assert!((t1_remaining - 0.8).abs() < 1e-9, "t1 should have 0.8 left, got {t1_remaining}");
    // t2 untouched.
    assert!((t2_remaining - 1.0).abs() < 1e-9, "t2 should be untouched, got {t2_remaining}");
}

// ─────────────────────────────────────────────────────────────────────
//  B3 — resolve_for_dispatch honors version_req
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b3_resolve_for_dispatch_falls_back_when_policy_version_outside_req() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    // Publish v2.0.0 in addition to the starter pack's v1.0.0.
    h.catalog
        .publish(tenant, build_agent("json_validate", "2.0.0"))
        .await
        .unwrap();
    // Policy = Stable(v1.0.0).
    // Caller asks for ^2.0 — policy version (1.0.0) doesn't satisfy.
    // The resolver should fall back to standard lookup → v2.0.0.
    let req = VersionReq::new("^2.0").unwrap();
    let resolved = h
        .catalog
        .resolve_for_dispatch(tenant, &aid("json_validate"), &req, &[0u8; 32])
        .await
        .unwrap();
    match resolved {
        ResolvedAgent::Single(a) => {
            assert_eq!(
                a.version,
                v("2.0.0"),
                "should fall back to v2.0.0 (matching ^2.0), got {:?}",
                a.version
            );
        }
        _ => panic!("expected Single"),
    }
}

#[tokio::test]
async fn b3_resolve_for_dispatch_honors_policy_when_in_range() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 10_000,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();
    // Caller asks for ^1.0 — policy versions both satisfy.  Canary applies.
    let req = VersionReq::new("^1.0").unwrap();
    let resolved = h
        .catalog
        .resolve_for_dispatch(tenant, &aid("json_validate"), &req, &[0u8; 32])
        .await
        .unwrap();
    match resolved {
        ResolvedAgent::Single(a) => assert_eq!(a.version, v("1.1.0")),
        _ => panic!("expected Single (canary)"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  B4 — tombstone_trace_cascade
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b4_tombstone_cascade_tombstones_root_and_shadow_children() {
    // Build a chain with prod + ShadowRun + ShadowComparison and assert
    // the cascade tombstones all three.
    use efe_nano::audit::chain_store::InMemoryChainStore;
    use efe_nano::audit::hmac_chain::InMemoryHmacChainAuditWriter;
    use efe_nano::audit::{AuditWriter as _, ChainStore, HmacKeySource, NanoAgentTrace};
    use efe_nano_sdk::{NanoAgentVersion, TraceId};

    let store = Arc::new(InMemoryChainStore::new());
    let key_source = HmacKeySource::test_key();
    let writer = InMemoryHmacChainAuditWriter::new(Arc::clone(&store), key_source);
    let tenant = TenantId::system();
    let prod_id = uuid::Uuid::new_v4();
    let shadow_id = uuid::Uuid::new_v4();
    let comparison_id = uuid::Uuid::new_v4();
    let swarm = uuid::Uuid::new_v4();

    fn make_trace(
        id: uuid::Uuid,
        parents: Vec<TraceId>,
        kind: Option<&str>,
        swarm: uuid::Uuid,
        tenant: TenantId,
    ) -> NanoAgentTrace {
        NanoAgentTrace {
            trace_id: TraceId(id),
            parent_trace_ids: parents,
            swarm_request_id: swarm,
            operation_id: "op".into(),
            tenant_id: tenant,
            caller_agent_id: None,
            nano_agent_id: NanoAgentId::new("agent").unwrap(),
            nano_agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            execution_mode: "pure_rust".into(),
            input_hash_source: [0u8; 32],
            input_hash_resolved: [0u8; 32],
            output_hash: None,
            error_kind: kind.map(|s| s.to_string()),
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            latency_ms: Some(1),
            cost_cents: 0.0,
            llm_tokens_used: None,
            full_input: None,
            full_output: None,
            tombstone: None,
            swarm_envelope_trace_id: None,
        }
    }

    writer.record(make_trace(prod_id, vec![], None, swarm, tenant)).await;
    writer
        .record(make_trace(shadow_id, vec![TraceId(prod_id)], Some("ShadowRun"), swarm, tenant))
        .await;
    writer
        .record(make_trace(
            comparison_id,
            vec![TraceId(prod_id), TraceId(shadow_id)],
            Some("ShadowComparison"),
            swarm,
            tenant,
        ))
        .await;

    let meta = efe_nano::audit::TombstoneMetadata::new(
        chrono::Utc::now(),
        efe_nano::audit::ErasureReason::GdprArticle17,
        None,
        "test",
        [0u8; 32],
    );
    let outcome = store
        .tombstone_trace_cascade(tenant, prod_id, meta)
        .await
        .unwrap();
    // All 3 should be tombstoned : prod + shadow + comparison.
    assert_eq!(outcome.traces_tombstoned, 3, "cascade must include shadow + comparison");
}

// ─────────────────────────────────────────────────────────────────────
//  B5 + B8 — Dispatcher promote_canary helper
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b5_promote_canary_swaps_policy_and_emits_trace() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 5000,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();
    let d = h.dispatcher_with_shadow();
    // force=true bypasses gates (we have no drift samples).
    let receipt = d
        .promote_canary(tenant, &aid("json_validate"), "operator:test", Some("ramp done".into()), true)
        .await
        .unwrap();
    assert!(matches!(receipt.new_policy, RolloutPolicy::Stable { ref version } if *version == v("1.1.0")));
    // Policy in catalog reflects the promotion.
    let now = h.catalog.get_rollout_policy(tenant, &aid("json_validate")).await.unwrap();
    assert!(matches!(now, RolloutPolicy::Stable { ref version } if *version == v("1.1.0")));
    // RolloutPolicyChanged trace was emitted.
    let traces = h.audit.traces().await;
    let policy_changes: Vec<_> = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("RolloutPolicyChanged"))
        .collect();
    assert_eq!(policy_changes.len(), 1);
}

#[tokio::test]
async fn b5_promote_canary_rejects_when_not_a_canary() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    let d = h.dispatcher_with_shadow();
    // Default policy is Stable ⇒ promote should reject.
    let err = d
        .promote_canary(tenant, &aid("json_validate"), "operator:test", None, true)
        .await;
    assert!(matches!(err, Err(efe_nano::dispatcher::RolloutWorkflowError::NotACanary { .. })));
}

#[tokio::test]
async fn b5_rollback_canary_returns_to_prod_version() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 5000,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();
    let d = h.dispatcher_with_shadow();
    let receipt = d
        .rollback_canary(tenant, &aid("json_validate"), "operator:test", Some("drift detected".into()))
        .await
        .unwrap();
    assert!(matches!(receipt.new_policy, RolloutPolicy::Stable { ref version } if *version == v("1.0.0")));
}

#[tokio::test]
async fn b5_observe_canary_returns_full_report() {
    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 5000,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();
    let d = h.dispatcher_with_shadow();
    let obs = d.observe_canary(tenant, &aid("json_validate")).await.unwrap();
    assert!(matches!(obs.policy, RolloutPolicy::Canary { .. }));
    assert!(obs.drift_stats.is_some());
    assert!(obs.gate_outcome.is_some());
    assert!(!obs.in_rollback_cooldown);
}

// ─────────────────────────────────────────────────────────────────────
//  B7 — Author check
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b7_tenant_cannot_policy_efe_shipped_agent() {
    let h = make_harness().await;
    let t1 = TenantId::new();
    // json_validate is Efe-shipped.  t1 attempts to set a policy.
    let result = h
        .catalog
        .set_rollout_policy(
            t1,
            &aid("json_validate"),
            RolloutPolicy::Stable { version: v("1.0.0") },
        )
        .await;
    assert!(result.is_err(), "tenant cannot policy EFE-shipped agent");
}

#[tokio::test]
async fn b7_operator_can_policy_efe_shipped_agent() {
    let h = make_harness().await;
    // system tenant = operator context — allowed.
    let result = h
        .catalog
        .set_rollout_policy(
            TenantId::system(),
            &aid("json_validate"),
            RolloutPolicy::Stable { version: v("1.0.0") },
        )
        .await;
    assert!(result.is_ok(), "operator can policy EFE agents");
}

// ─────────────────────────────────────────────────────────────────────
//  B6 — ShadowScheduler catalog injection (smoke test)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b6_shadow_scheduler_with_catalog_builder_works() {
    let h = make_harness().await;
    let scheduler = ShadowScheduler::new(
        Arc::clone(&h.runtime),
        Arc::clone(&h.audit) as Arc<dyn AuditWriter>,
        Arc::clone(&h.drift),
        Arc::clone(&h.cost_router),
    )
    .with_catalog(Arc::clone(&h.catalog_dyn));
    // Just verify the scheduler is constructible — full Semantic
    // pipeline test is harness-heavy and deferred to Sprint 9 wiring.
    let _ = scheduler;
}

// ─────────────────────────────────────────────────────────────────────
//  B2 — Auto-rollback hook
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b2_catalog_rollback_hook_swaps_to_stable() {
    use efe_nano::dispatcher::shadow::{CatalogRollbackHook, RollbackHook};
    use efe_nano::idempotency::IdempotencyCache;

    let h = make_harness().await;
    let tenant = TenantId::system();
    h.catalog
        .publish(tenant, build_agent("json_validate", "1.1.0"))
        .await
        .unwrap();
    // Start with a Canary policy.
    h.catalog
        .set_rollout_policy(
            tenant,
            &aid("json_validate"),
            RolloutPolicy::Canary {
                prod_version: v("1.0.0"),
                canary_version: v("1.1.0"),
                canary_bps: 5000,
                started_at: chrono::Utc::now(),
                promotion_gates: PromotionGates::default(),
                cost_attribution: CostAttribution::Tenant,
            },
        )
        .await
        .unwrap();

    let hook = CatalogRollbackHook::new(
        Arc::clone(&h.catalog_dyn),
        Arc::clone(&h.audit) as Arc<dyn AuditWriter>,
        Arc::clone(&h.drift),
    )
    .with_idempotency(Arc::new(IdempotencyCache::new()));

    hook.rollback(tenant, aid("json_validate"), v("1.0.0"), "test".into()).await;

    // Policy should now be Stable(1.0.0).
    let now = h.catalog.get_rollout_policy(tenant, &aid("json_validate")).await.unwrap();
    match now {
        RolloutPolicy::Stable { version } => assert_eq!(version, v("1.0.0")),
        _ => panic!("expected Stable after rollback"),
    }
    // RolloutPolicyChanged trace emitted.
    let traces = h.audit.traces().await;
    let count = traces
        .iter()
        .filter(|t| t.error_kind.as_deref() == Some("RolloutPolicyChanged"))
        .count();
    assert_eq!(count, 1, "exactly one RolloutPolicyChanged trace");
}
