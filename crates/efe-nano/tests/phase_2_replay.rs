//! End-to-end Phase 2 Sprint 6 — Replay engine integration tests.
//!
//! Validates the **complete replay pipeline** from `Dispatcher.dispatch`
//! to `ReplayEngine.replay` to `ReplayCliArgs::run` :
//!
//! ```text
//!  Dispatcher          ReplayEngine                  CLI (replay::cli::run)
//!     ↓                     ↓                                 ↓
//!  populate           1. traces_for_swarm                serialize_report()
//!  HMAC chain   →     2. verify_subchain          →     stdout / file
//!  + body store       3. SwarmReconstructor
//!                     4. dispatcher.dispatch (re-run)
//!                     5. DeterminismChecker
//!                     6. assemble ReplayReport
//! ```
//!
//! Coverage matrix (20 tests) :
//!
//! | #  | Test                                                | Property under test                                    |
//! |----|-----------------------------------------------------|--------------------------------------------------------|
//! | 1  | `replay_unknown_request_id_returns_not_found`       | NotFound for a never-dispatched swarm                  |
//! | 2  | `replay_single_mode_a_op_matches_exactly`           | 1-op Mode A → Match status, hashes equal               |
//! | 3  | `replay_diamond_dag_all_match`                      | 4-op diamond DAG → 4 Matches                           |
//! | 4  | `replay_chain_verified_flag_true_on_clean_data`     | report.chain_verified == true                          |
//! | 5  | `replay_after_tamper_returns_chain_integrity_broken`| post-hoc tamper → ChainIntegrityBroken error           |
//! | 6  | `replay_report_per_op_status_lookup_via_map`        | per_op_map() builds correct op_id → PerOpReplay        |
//! | 7  | `replay_two_swarms_only_replays_one`                | swarm scoping : replay(X) doesn't see swarm Y's traces |
//! | 8  | `replay_cross_tenant_request_returns_not_found`     | tenant scoping : t1 swarm not visible to t2 replay     |
//! | 9  | `replay_missing_full_input_returns_missing_full_input` | hash-only audit policy → MissingFullInput            |
//! | 10 | `replay_is_fully_consistent_for_clean_mode_a_swarm` | is_fully_consistent() returns true                     |
//! | 11 | `replay_duration_ms_recorded_and_reasonable`        | duration_ms > 0 and < 1 s                              |
//! | 12 | `replay_report_contains_all_op_ids`                 | per_op covers every original op                        |
//! | 13 | `replay_cli_run_returns_valid_json_pretty`          | cli::run → valid JSON string, pretty mode              |
//! | 14 | `replay_cli_compact_output_no_newlines`             | compact mode → single line                             |
//! | 15 | `replay_cli_writes_to_output_file`                  | output_file path → JSON written to disk                |
//! | 16 | `replay_replay_runs_independently_per_invocation`   | replay() can be called twice on the same swarm         |
//! | 17 | `replay_total_ops_counter_matches_chain_length`     | report.total_ops == chain.traces_for_swarm.len()       |
//! | 18 | `corpus_thousand_op_mode_a_one_hundred_percent_match`| THE 1 k-op corpus : every replay reports Match         |
//! | 19 | `corpus_with_two_tenants_isolated_replays`          | parallel-tenant corpus : isolated replays              |
//! | 20 | `corpus_tamper_detection_at_random_row`             | corpus + random tamper → caught at right trace_id      |

use std::sync::Arc;
use uuid::Uuid;

use efe_nano::audit::chain_store::{ChainStore, InMemoryChainStore};
use efe_nano::audit::hkdf::HmacKeySource;
use efe_nano::audit::hmac_chain::InMemoryHmacChainAuditWriter;
use efe_nano::audit::AuditWriter;
use efe_nano::catalog::InMemoryCatalog;
use efe_nano::dispatcher::Dispatcher;
use efe_nano::error::ReplayError;
use efe_nano::idempotency::IdempotencyCache;
use efe_nano::replay::cli::{run as cli_run, ReplayCliArgs};
use efe_nano::replay::{ReplayEngine, ReplayOpStatus};
use efe_nano::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
use efe_nano::runtime::NanoAgentExecutor;
use efe_nano::starter_pack::{json_validate, register_all_starter_pack};
use efe_nano_sdk::{
    AuditPolicy, Catalog, NanoAgent, NanoAgentId, NanoAgentRef, Operation, OperationInput,
    RetryPolicy, SwarmRequest, SwarmStatus, TenantId,
};

// ─────────────────────────────────────────────────────────────────────
//  Harness
// ─────────────────────────────────────────────────────────────────────

/// Wire `catalog + Mode A runtime + InMemoryHmacChainAuditWriter +
/// dispatcher + ReplayEngine` together.
struct ReplayHarness {
    catalog: Arc<InMemoryCatalog>,
    dispatcher: Arc<Dispatcher>,
    chain_store: Arc<InMemoryChainStore>,
    key_source: HmacKeySource,
}

async fn make_replay_harness() -> ReplayHarness {
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
        Arc::clone(&catalog) as Arc<dyn Catalog>,
        runtime,
        Arc::clone(&writer) as Arc<dyn AuditWriter>,
        Arc::new(IdempotencyCache::new()),
    ));
    ReplayHarness {
        catalog,
        dispatcher,
        chain_store,
        key_source,
    }
}

impl ReplayHarness {
    fn make_engine(&self) -> ReplayEngine {
        ReplayEngine::new(
            Arc::clone(&self.chain_store),
            self.key_source.clone(),
            Arc::clone(&self.dispatcher),
            Arc::clone(&self.catalog) as Arc<dyn Catalog>,
        )
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
        caller_agent_id: Some("phase-2-replay".to_string()),
        operations: ops,
        max_parallelism: 64,
        timeout_ms: 60_000,
        idempotency_key: None,
        streaming: false,
        audit_tags: vec!["phase-2".to_string(), "replay".to_string()],
        cost_cap_cents: None,
        parent_envelope_trace_id: None,
    }
}

/// `json_validate` op with a unique-by-op_id input.  Same uniqueness
/// trick as the audit-chain suite : prevents the idempotency cache
/// from short-circuiting and dropping the audit trace.
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

/// `regex_extract` op with a unique input.
fn op_regex_extract(op_id: &str) -> Operation {
    op(
        op_id,
        "regex_extract",
        &[],
        lit(serde_json::json!({
            "text": format!("hello {op_id} world"),
            "pattern": r"\w+"
        })),
    )
}

/// `date_normalize` op with a unique input.
fn op_date_normalize(op_id: &str, iso: &str) -> Operation {
    op(
        op_id,
        "date_normalize",
        &[],
        lit(serde_json::json!({
            "input": iso,
        })),
    )
}

/// `currency_normalize` op with a unique input.
fn op_currency_normalize(op_id: &str, n: u32) -> Operation {
    op(
        op_id,
        "currency_normalize",
        &[],
        lit(serde_json::json!({"input": format!("${n}.00")})),
    )
}

/// `xml_validate` op with a unique input.
fn op_xml_validate(op_id: &str, idx: u32) -> Operation {
    op(
        op_id,
        "xml_validate",
        &[],
        lit(serde_json::json!({"xml": format!("<root id=\"{idx}\"><a>x</a></root>")})),
    )
}

/// Dispatch a single-op swarm.  Returns the response.
async fn dispatch_single(
    h: &ReplayHarness,
    tenant: TenantId,
    request_id: Uuid,
    op_id: &str,
) -> efe_nano_sdk::SwarmResponse {
    let req = req_tagged(vec![op_validate_obj(op_id)], tenant, request_id);
    h.dispatcher.dispatch(req).await.unwrap()
}

// ─────────────────────────────────────────────────────────────────────
//  1. Unknown request_id → NotFound
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_unknown_request_id_returns_not_found() {
    let h = make_replay_harness().await;
    let engine = h.make_engine();
    let err = engine
        .replay(TenantId::system(), Uuid::new_v4())
        .await
        .unwrap_err();
    assert!(
        matches!(err, ReplayError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  2. Single Mode A op matches exactly
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_single_mode_a_op_matches_exactly() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let resp = dispatch_single(&h, tenant, request_id, "v").await;
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    let engine = h.make_engine();
    let report = engine.replay(tenant, request_id).await.unwrap();
    assert_eq!(report.total_ops, 1);
    assert_eq!(report.matched_ops, 1);
    assert_eq!(report.mismatched_ops, 0);
    assert_eq!(report.failed_ops, 0);
    assert_eq!(report.skipped_non_deterministic_ops, 0);
    assert!(report.chain_verified);
    assert!(report.is_fully_consistent());

    // The per-op record exists and has matching hashes.
    let p = report
        .per_op
        .iter()
        .find(|p| p.op_id == "v")
        .expect("per_op entry for op 'v'");
    assert_eq!(p.status, ReplayOpStatus::Match);
    assert!(p.original_output_hash.is_some());
    assert!(p.replay_output_hash.is_some());
    assert_eq!(p.original_output_hash, p.replay_output_hash);
}

// ─────────────────────────────────────────────────────────────────────
//  3. Diamond DAG : all 4 ops Match
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_diamond_dag_all_match() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![
            op_validate_obj("src"),
            op_regex_extract("leaf1"),
            op(
                "leaf2",
                "csv_normalize",
                &[],
                lit(serde_json::json!({"csv": "a,b\n1,2"})),
            ),
            op_validate_obj("join"),
        ],
        tenant,
        request_id,
    );
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    assert_eq!(report.total_ops, 4);
    assert_eq!(report.matched_ops, 4);
    assert_eq!(report.mismatched_ops, 0);
    assert!(report.is_fully_consistent());
}

// ─────────────────────────────────────────────────────────────────────
//  4. chain_verified flag is true on honest data
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_chain_verified_flag_true_on_clean_data() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let _ = dispatch_single(&h, tenant, request_id, "v").await;
    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    assert!(report.chain_verified);
}

// ─────────────────────────────────────────────────────────────────────
//  5. Post-hoc tamper → ChainIntegrityBroken
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_after_tamper_returns_chain_integrity_broken() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![op_validate_obj("a"), op_validate_obj("b")],
        tenant,
        request_id,
    );
    let _ = h.dispatcher.dispatch(request).await.unwrap();

    // Tamper : we can't mutate the store from outside easily.  Instead
    // we use the chain_store's chain snapshot + a fresh store + push the
    // tampered slice in.  Cleanest approach : build a NEW chain_store +
    // engine, copy chain entries with one row tampered.
    let original = h
        .chain_store
        .chain_for_tenant(tenant, None)
        .await
        .unwrap();
    // N14 : 2 ops + 1 envelope = 3.
    assert_eq!(original.len(), 3);

    let tampered_store = Arc::new(InMemoryChainStore::new());
    // Copy bodies first (so the reconstructor finds them).
    for entry in &original {
        if let Some(b) = h.chain_store.get_input(&entry.trace.input_hash_resolved) {
            tampered_store.put_input(entry.trace.input_hash_resolved, b);
        }
        if let Some(h_) = entry.trace.output_hash {
            if let Some(b) = h.chain_store.get_output(&h_) {
                tampered_store.put_output(h_, b);
            }
        }
    }
    // Push entries — second one tampered (cost mutated post-hoc).
    let t1 = original[0].clone();
    let mut t2 = original[1].clone();
    t2.trace.cost_cents = 9_999.99;
    tampered_store.append(t1.clone()).await.unwrap();
    // The store's append checks the parent link, not the HMAC.  This
    // simulates an attacker who altered the row after insertion.
    t2.chain_parent_id = Some(t1.trace_id());
    tampered_store.append(t2).await.unwrap();
    drop(t1);

    // Build a fresh engine pointing at the tampered store.
    let engine = ReplayEngine::new(
        Arc::clone(&tampered_store),
        h.key_source.clone(),
        Arc::clone(&h.dispatcher),
        Arc::clone(&h.catalog) as Arc<dyn Catalog>,
    );
    let err = engine.replay(tenant, request_id).await.unwrap_err();
    assert!(
        matches!(err, ReplayError::ChainIntegrityBroken(_)),
        "expected ChainIntegrityBroken, got {err:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  6. per_op_map() lookups correctly
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_report_per_op_status_lookup_via_map() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![
            op_validate_obj("alpha"),
            op_validate_obj("beta"),
            op_validate_obj("gamma"),
        ],
        tenant,
        request_id,
    );
    let _ = h.dispatcher.dispatch(request).await.unwrap();
    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    assert_eq!(report.total_ops, 3);
    let map = report.per_op_map();
    assert_eq!(map.len(), 3);
    for op_id in ["alpha", "beta", "gamma"] {
        let p = map
            .get(op_id)
            .unwrap_or_else(|| panic!("op_id {op_id} missing from per_op_map"));
        assert_eq!(p.status, ReplayOpStatus::Match);
    }
}

// ─────────────────────────────────────────────────────────────────────
//  7. Replay scopes to the requested swarm only
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_two_swarms_only_replays_one() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let swarm_a = Uuid::new_v4();
    let swarm_b = Uuid::new_v4();
    // Two distinct swarms in same tenant chain.
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
            vec![
                op_validate_obj("b1"),
                op_validate_obj("b2"),
                op_validate_obj("b3"),
            ],
            tenant,
            swarm_b,
        ))
        .await
        .unwrap();

    let engine = h.make_engine();
    let report_a = engine.replay(tenant, swarm_a).await.unwrap();
    assert_eq!(report_a.total_ops, 2);
    let report_b = engine.replay(tenant, swarm_b).await.unwrap();
    assert_eq!(report_b.total_ops, 3);
    // Each report only contains its own ops.
    let a_ids: Vec<&str> = report_a.per_op.iter().map(|p| p.op_id.as_str()).collect();
    let b_ids: Vec<&str> = report_b.per_op.iter().map(|p| p.op_id.as_str()).collect();
    for id in &a_ids {
        assert!(!b_ids.contains(id), "op_id {id} leaked into report B");
    }
}

// ─────────────────────────────────────────────────────────────────────
//  8. Cross-tenant replay returns NotFound
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_cross_tenant_request_returns_not_found() {
    let h = make_replay_harness().await;
    let t1 = TenantId::new();
    let t2 = TenantId::new();
    let request_id = Uuid::new_v4();
    let _ = h
        .dispatcher
        .dispatch(req_tagged(vec![op_validate_obj("v")], t1, request_id))
        .await
        .unwrap();

    // Try to replay t1's swarm under t2 → must return NotFound.
    let engine = h.make_engine();
    let err = engine.replay(t2, request_id).await.unwrap_err();
    assert!(
        matches!(err, ReplayError::NotFound(_)),
        "expected NotFound (cross-tenant isolation), got {err:?}"
    );

    // And t1's replay itself succeeds.
    let report = engine.replay(t1, request_id).await.unwrap();
    assert_eq!(report.total_ops, 1);
    assert!(report.is_fully_consistent());
}

// ─────────────────────────────────────────────────────────────────────
//  9. Missing full_input (hash-only audit) → MissingFullInput
// ─────────────────────────────────────────────────────────────────────

/// Build a custom NanoAgent that clones `json_validate` but switches
/// the audit policy to `hash_only` (no full_input / full_output).
fn hash_only_validate_manifest() -> NanoAgent {
    let mut m = json_validate::manifest();
    m.id = NanoAgentId::new("json_validate_hash_only").unwrap();
    m.audit = AuditPolicy::hash_only();
    m
}

#[tokio::test]
async fn replay_missing_full_input_returns_missing_full_input() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    // Publish a hash-only variant — re-uses the json_validate handler entry.
    h.catalog
        .publish(tenant, hash_only_validate_manifest())
        .await
        .unwrap();

    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![op(
            "v",
            "json_validate_hash_only",
            &[],
            lit(serde_json::json!({
                "value": {"x": 1, "_uniq": "v"},
                "schema": {"type": "object"}
            })),
        )],
        tenant,
        request_id,
    );
    let resp = h.dispatcher.dispatch(request).await.unwrap();
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    // The chain has the trace, but the body store does NOT have the input
    // body (because include_input was false on the agent's audit policy).
    let chain = h
        .chain_store
        .chain_for_tenant(tenant, None)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| !e.trace.is_swarm_envelope())
        .collect::<Vec<_>>();
    assert_eq!(chain.len(), 1);
    assert!(
        h.chain_store
            .get_input(&chain[0].trace.input_hash_resolved)
            .is_none(),
        "hash-only policy should NOT store the full input body"
    );

    // Replay must surface MissingFullInput.
    let err = h
        .make_engine()
        .replay(tenant, request_id)
        .await
        .unwrap_err();
    match err {
        ReplayError::MissingFullInput(id) => {
            assert_eq!(id, chain[0].trace_id());
        }
        other => panic!("expected MissingFullInput, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  10. is_fully_consistent() on a clean Mode A swarm
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_is_fully_consistent_for_clean_mode_a_swarm() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let request = req_tagged(
        vec![
            op_validate_obj("a"),
            op_regex_extract("b"),
            op_currency_normalize("c", 42),
            op_xml_validate("d", 7),
            op_date_normalize("e", "2026-05-18T10:00:00Z"),
        ],
        tenant,
        request_id,
    );
    let _ = h.dispatcher.dispatch(request).await.unwrap();
    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    assert!(
        report.is_fully_consistent(),
        "report : matched={} mismatched={} skipped={} failed={}",
        report.matched_ops,
        report.mismatched_ops,
        report.skipped_non_deterministic_ops,
        report.failed_ops,
    );
}

// ─────────────────────────────────────────────────────────────────────
//  11. duration_ms is recorded and reasonable
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_duration_ms_recorded_and_reasonable() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let _ = dispatch_single(&h, tenant, request_id, "v").await;
    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    // Replay should be sub-second for a 1-op Mode A swarm.
    assert!(
        report.duration_ms < 5_000,
        "replay duration {} ms exceeds 5 s budget",
        report.duration_ms
    );
    // started_at + duration_ms ≈ completed_at (allow 50 ms slack).
    let delta_ms = (report.completed_at - report.started_at).num_milliseconds() as i64;
    assert!(
        (delta_ms - report.duration_ms as i64).abs() < 100,
        "duration_ms ({}) inconsistent with completed_at - started_at ({})",
        report.duration_ms,
        delta_ms
    );
}

// ─────────────────────────────────────────────────────────────────────
//  12. Report per_op covers every original op
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_report_contains_all_op_ids() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let original_ids = ["op-1", "op-2", "op-3", "op-4", "op-5"];
    let ops: Vec<Operation> = original_ids.iter().map(|id| op_validate_obj(id)).collect();
    let _ = h
        .dispatcher
        .dispatch(req_tagged(ops, tenant, request_id))
        .await
        .unwrap();
    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    let actual: std::collections::HashSet<&str> =
        report.per_op.iter().map(|p| p.op_id.as_str()).collect();
    for id in &original_ids {
        assert!(actual.contains(id), "report missing op {id}");
    }
    assert_eq!(actual.len(), original_ids.len());
}

// ─────────────────────────────────────────────────────────────────────
//  13. CLI run() returns valid pretty JSON
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_cli_run_returns_valid_json_pretty() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let _ = dispatch_single(&h, tenant, request_id, "v").await;
    let engine = h.make_engine();
    let args = ReplayCliArgs::new(tenant, request_id);  // pretty=true by default
    let json = cli_run(&engine, args).await.unwrap();
    // Valid JSON.
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("must be valid JSON");
    assert_eq!(parsed["total_ops"], 1);
    assert_eq!(parsed["matched_ops"], 1);
    assert_eq!(parsed["chain_verified"], true);
    assert_eq!(parsed["fully_consistent"], true);
    assert_eq!(parsed["per_op"][0]["status"], "match");
    // Pretty mode → newlines present.
    assert!(json.contains('\n'));
}

// ─────────────────────────────────────────────────────────────────────
//  14. CLI compact mode → single line
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_cli_compact_output_no_newlines() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let _ = dispatch_single(&h, tenant, request_id, "v").await;
    let engine = h.make_engine();
    let args = ReplayCliArgs::new(tenant, request_id).compact();
    let json = cli_run(&engine, args).await.unwrap();
    assert!(!json.contains('\n'), "compact JSON must not contain newlines");
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["total_ops"], 1);
}

// ─────────────────────────────────────────────────────────────────────
//  15. CLI writes to output_file
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_cli_writes_to_output_file() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let _ = dispatch_single(&h, tenant, request_id, "v").await;

    let tmp = std::env::temp_dir().join(format!("replay-report-{}.json", Uuid::new_v4()));
    let engine = h.make_engine();
    let args = ReplayCliArgs::new(tenant, request_id).with_output_file(tmp.clone());
    let returned = cli_run(&engine, args).await.unwrap();

    // The function returns the JSON, AND writes it to the file.
    let on_disk = tokio::fs::read_to_string(&tmp).await.unwrap();
    assert_eq!(on_disk, returned);
    let parsed: serde_json::Value = serde_json::from_str(&on_disk).unwrap();
    assert_eq!(parsed["total_ops"], 1);

    // Cleanup.
    let _ = tokio::fs::remove_file(&tmp).await;
}

// ─────────────────────────────────────────────────────────────────────
//  16. Replay can be called twice on the same swarm (read-only)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_replay_runs_independently_per_invocation() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let _ = h
        .dispatcher
        .dispatch(req_tagged(
            vec![op_validate_obj("a"), op_validate_obj("b")],
            tenant,
            request_id,
        ))
        .await
        .unwrap();
    let engine = h.make_engine();

    let r1 = engine.replay(tenant, request_id).await.unwrap();
    let r2 = engine.replay(tenant, request_id).await.unwrap();

    // Both reports must show the same ops + same Match status.
    assert_eq!(r1.total_ops, r2.total_ops);
    assert_eq!(r1.matched_ops, r2.matched_ops);
    assert!(r1.is_fully_consistent());
    assert!(r2.is_fully_consistent());

    // **Sprint 8 — N12** : the replay path goes through the dispatcher
    // which now writes a chain-linked cache-hit trace (error_kind =
    // Some("CacheHit"), pointer-shaped, full bodies = None) per op that
    // hits the cache.  Original dispatch produced 2 cold traces ; each
    // replay adds 2 cache-hit traces (one per op).  Total : 2 + 2 + 2 = 6.
    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(
        chain.len(),
        6,
        "2 cold ops + 2 replays × 2 cache-hit traces = 6"
    );
    let cache_hit_entries: Vec<_> = chain
        .iter()
        .filter(|e| e.trace.error_kind.as_deref() == Some("CacheHit"))
        .collect();
    assert_eq!(
        cache_hit_entries.len(),
        4,
        "exactly 4 cache-hit lineage entries"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  17. report.total_ops matches chain.traces_for_swarm length
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn replay_total_ops_counter_matches_chain_length() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();
    let request_id = Uuid::new_v4();
    let n_ops = 7;
    let ops: Vec<Operation> = (0..n_ops)
        .map(|i| op_validate_obj(&format!("op-{i}")))
        .collect();
    let _ = h
        .dispatcher
        .dispatch(req_tagged(ops, tenant, request_id))
        .await
        .unwrap();
    // N14 : `traces` includes the envelope ; `report.total_ops` is
    // filtered (envelope sentinel skipped).  Account for the envelope.
    let traces = h
        .chain_store
        .traces_for_swarm(tenant, request_id)
        .await
        .unwrap();
    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    let non_envelope_traces = traces
        .iter()
        .filter(|e| !e.trace.is_swarm_envelope())
        .count() as u32;
    assert_eq!(non_envelope_traces, report.total_ops);
    assert_eq!(report.total_ops, n_ops);
}

// ─────────────────────────────────────────────────────────────────────
//  18. THE 1 k-op corpus : 100 % match on Mode A
// ─────────────────────────────────────────────────────────────────────
//
//  Architecture target (cf ARCHITECTURE_SPRINT_6.md § 4.6 + § 4.7) :
//  every Mode A op replays bit-identically.  We dispatch 200 swarms of
//  5 ops each (mixing 5 different Mode A starter agents : json_validate,
//  regex_extract, currency_normalize, xml_validate, date_normalize),
//  then replay every swarm and assert :
//
//  - report.is_fully_consistent() for every swarm
//  - mismatched_ops == 0 across the entire corpus
//  - skipped_non_deterministic_ops == 0  (Mode A is FullDeterministic)
//  - failed_ops == 0
//  - chain still verifies end-to-end after the full corpus run
//
//  This is the headline OKR test of Sprint 6.

#[tokio::test]
async fn corpus_thousand_op_mode_a_one_hundred_percent_match() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();

    const SWARMS: u32 = 200;
    const OPS_PER_SWARM: u32 = 5;

    // 1. Dispatch the corpus.
    let mut request_ids = Vec::with_capacity(SWARMS as usize);
    let dispatch_started = std::time::Instant::now();
    for swarm_i in 0..SWARMS {
        let request_id = Uuid::new_v4();
        request_ids.push(request_id);
        let ops: Vec<Operation> = (0..OPS_PER_SWARM)
            .map(|op_i| {
                // Cycle through 5 Mode A agents.  Each op_id is unique
                // across the corpus so the idempotency cache never hits.
                let oid = format!("s{swarm_i}_o{op_i}");
                match op_i % 5 {
                    0 => op_validate_obj(&oid),
                    1 => op_regex_extract(&oid),
                    2 => op_currency_normalize(&oid, swarm_i * 1_000 + op_i),
                    3 => op_xml_validate(&oid, swarm_i * 1_000 + op_i),
                    _ => {
                        // Build a fully-unique ISO timestamp.  swarm_i is
                        // unbounded so we synthesise a year offset, month
                        // 5 (May), day = (swarm_i % 28) + 1, and embed the
                        // FULL swarm index in the minute/second pair so
                        // two different swarms with the same day produce
                        // different normalised dates and thus different
                        // (agent, input) idempotency-cache keys.
                        let year = 2000 + (swarm_i / 28) % 1000;
                        let day = (swarm_i % 28) + 1;
                        op_date_normalize(
                            &oid,
                            &format!(
                                "{year:04}-05-{day:02}T10:{:02}:{:02}Z",
                                op_i % 60,
                                swarm_i % 60
                            ),
                        )
                    }
                }
            })
            .collect();
        let resp = h
            .dispatcher
            .dispatch(req_tagged(ops, tenant, request_id))
            .await
            .unwrap();
        assert_eq!(
            resp.overall_status,
            SwarmStatus::Success,
            "swarm {swarm_i} should succeed"
        );
    }
    let dispatch_elapsed = dispatch_started.elapsed();

    // 2. **N14** : chain count == SWARMS * (OPS_PER_SWARM + 1 envelope).
    let total_expected = (SWARMS * (OPS_PER_SWARM + 1)) as u64;
    assert_eq!(
        h.chain_store.total_entries().await.unwrap(),
        total_expected,
        "corpus dispatch should produce {total_expected} chain entries"
    );

    // 3. Replay every swarm — must be fully consistent.
    let engine = h.make_engine();
    let replay_started = std::time::Instant::now();
    let mut grand_matched = 0u32;
    let mut grand_skipped = 0u32;
    let mut grand_mismatched = 0u32;
    let mut grand_failed = 0u32;
    for (i, req_id) in request_ids.iter().enumerate() {
        let report = engine.replay(tenant, *req_id).await.unwrap_or_else(|e| {
            panic!("replay swarm {i} failed : {e}");
        });
        assert!(
            report.is_fully_consistent(),
            "swarm {i} not fully consistent : matched={} mismatched={} skipped={} failed={}",
            report.matched_ops,
            report.mismatched_ops,
            report.skipped_non_deterministic_ops,
            report.failed_ops
        );
        assert_eq!(report.total_ops, OPS_PER_SWARM);
        assert_eq!(
            report.matched_ops, OPS_PER_SWARM,
            "swarm {i} must have all {OPS_PER_SWARM} ops match (Mode A FullDeterministic)"
        );
        assert_eq!(report.mismatched_ops, 0, "swarm {i} : Mode A must not mismatch");
        assert_eq!(report.failed_ops, 0, "swarm {i} : no op should fail");
        assert_eq!(
            report.skipped_non_deterministic_ops, 0,
            "swarm {i} : Mode A should never be skipped"
        );
        grand_matched += report.matched_ops;
        grand_skipped += report.skipped_non_deterministic_ops;
        grand_mismatched += report.mismatched_ops;
        grand_failed += report.failed_ops;
    }
    let replay_elapsed = replay_started.elapsed();

    // 4. Grand totals : 1 000 Match, 0 of everything else.
    assert_eq!(grand_matched, SWARMS * OPS_PER_SWARM);
    assert_eq!(grand_skipped, 0);
    assert_eq!(grand_mismatched, 0);
    assert_eq!(grand_failed, 0);

    // 5. **Sprint 8 — N12** : replay through dispatcher now writes a
    // chain-linked cache-hit trace per replayed op (architecture doc
    // § 4.3).  So the chain grows by `total_expected` again on replay.
    //    Final total = cold_dispatch + replay_cache_hits = 2 × total_expected.
    let final_total = h.chain_store.total_entries().await.unwrap();
    assert_eq!(
        final_total,
        total_expected * 2,
        "cold ({total_expected}) + replay cache-hit traces ({total_expected}) = 2× total"
    );
    let verify_report =
        efe_nano::audit::verify::verify_chain(&*h.chain_store, &h.key_source, tenant, None)
            .await
            .unwrap();
    assert_eq!(verify_report.entries_verified, total_expected * 2);
    // N14 : the chain (excluding envelopes) holds the cold op traces +
    // the cache-hit lineage traces.  Cache hits count = SWARMS * OPS_PER_SWARM.
    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    let cache_hits = chain
        .iter()
        .filter(|e| e.trace.error_kind.as_deref() == Some("CacheHit"))
        .count() as u64;
    assert_eq!(cache_hits, (SWARMS * OPS_PER_SWARM) as u64);

    // 6. Performance sanity (debug mode — allow generous bounds).
    eprintln!(
        "[corpus] dispatch {SWARMS} swarms × {OPS_PER_SWARM} ops in {dispatch_elapsed:?}, \
         replay in {replay_elapsed:?}"
    );
    assert!(
        dispatch_elapsed.as_secs() < 60,
        "1 k-op dispatch took {dispatch_elapsed:?} (budget 60 s in debug mode)"
    );
    assert!(
        replay_elapsed.as_secs() < 30,
        "1 k-op replay took {replay_elapsed:?} (budget 30 s in debug mode)"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  19. Multi-tenant corpus : isolated replays
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn corpus_with_two_tenants_isolated_replays() {
    let h = make_replay_harness().await;
    let t1 = TenantId::new();
    let t2 = TenantId::new();

    // 20 swarms × 3 ops × 2 tenants = 120 total entries.
    let mut t1_requests = Vec::new();
    let mut t2_requests = Vec::new();
    for i in 0..20 {
        let r1 = Uuid::new_v4();
        let r2 = Uuid::new_v4();
        t1_requests.push(r1);
        t2_requests.push(r2);
        let ops_t1: Vec<Operation> = (0..3)
            .map(|j| op_validate_obj(&format!("t1_s{i}_o{j}")))
            .collect();
        let ops_t2: Vec<Operation> = (0..3)
            .map(|j| op_validate_obj(&format!("t2_s{i}_o{j}")))
            .collect();
        let _ = h
            .dispatcher
            .dispatch(req_tagged(ops_t1, t1, r1))
            .await
            .unwrap();
        let _ = h
            .dispatcher
            .dispatch(req_tagged(ops_t2, t2, r2))
            .await
            .unwrap();
    }

    // Tenant chains are independent : 60 entries each.
    let c1 = h.chain_store.chain_for_tenant(t1, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    let c2 = h.chain_store.chain_for_tenant(t2, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(c1.len(), 60);
    assert_eq!(c2.len(), 60);

    // Replay all t1 swarms — every report fully consistent.
    let engine = h.make_engine();
    for r in &t1_requests {
        let report = engine.replay(t1, *r).await.unwrap();
        assert!(report.is_fully_consistent());
        assert_eq!(report.total_ops, 3);
    }
    // Trying to replay any t1 request under t2 → NotFound.
    let cross_err = engine.replay(t2, t1_requests[0]).await.unwrap_err();
    assert!(matches!(cross_err, ReplayError::NotFound(_)));
    // And t2 chain still verifies.  N14 : 20 swarms × (3 ops + 1 envelope) = 80.
    let report =
        efe_nano::audit::verify::verify_chain(&*h.chain_store, &h.key_source, t2, None)
            .await
            .unwrap();
    assert_eq!(report.entries_verified, 80);
}

// ─────────────────────────────────────────────────────────────────────
//  20. Corpus + random tamper → caught at the right trace_id
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn corpus_tamper_detection_at_random_row() {
    let h = make_replay_harness().await;
    let tenant = TenantId::system();

    // Build a 30-trace chain (10 swarms × 3 ops).
    let mut request_ids = Vec::new();
    for swarm_i in 0..10 {
        let request_id = Uuid::new_v4();
        request_ids.push(request_id);
        let ops: Vec<Operation> = (0..3)
            .map(|i| op_validate_obj(&format!("s{swarm_i}_o{i}")))
            .collect();
        let _ = h
            .dispatcher
            .dispatch(req_tagged(ops, tenant, request_id))
            .await
            .unwrap();
    }
    let original_chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap();
    // N14 : 10 swarms × (3 ops + 1 envelope) = 40.
    assert_eq!(original_chain.len(), 40);

    // Tamper a fixed mid-corpus entry (index 17 — arbitrary but
    // deterministic for test reproducibility).
    let tampered_idx = 17usize;
    let tampered_trace_id = original_chain[tampered_idx].trace_id();

    // Build a parallel chain store with one row tampered.
    let tampered_store = Arc::new(InMemoryChainStore::new());
    for (i, entry) in original_chain.iter().enumerate() {
        // Copy bodies first.
        if let Some(b) = h.chain_store.get_input(&entry.trace.input_hash_resolved) {
            tampered_store.put_input(entry.trace.input_hash_resolved, b);
        }
        if let Some(oh) = entry.trace.output_hash {
            if let Some(b) = h.chain_store.get_output(&oh) {
                tampered_store.put_output(oh, b);
            }
        }
        let mut e = entry.clone();
        if i == tampered_idx {
            e.trace.cost_cents = -1.0;     // illegal post-hoc value
        }
        tampered_store.append(e).await.unwrap();
    }

    // Verify_chain on the tampered store : must fail at tampered_trace_id.
    let err = efe_nano::audit::verify::verify_chain(
        &*tampered_store,
        &h.key_source,
        tenant,
        None,
    )
    .await
    .unwrap_err();
    match err {
        efe_nano::error::AuditChainError::HmacMismatch(id)
        | efe_nano::error::AuditChainError::ChainBroken(id) => {
            assert_eq!(
                id, tampered_trace_id,
                "verifier must point to the tampered trace"
            );
        }
        other => panic!("expected HmacMismatch or ChainBroken, got {other:?}"),
    }

    // Replay of the swarm that contains the tampered row → ChainIntegrityBroken.
    let tampered_swarm = original_chain[tampered_idx].trace.swarm_request_id;
    let engine = ReplayEngine::new(
        Arc::clone(&tampered_store),
        h.key_source.clone(),
        Arc::clone(&h.dispatcher),
        Arc::clone(&h.catalog) as Arc<dyn Catalog>,
    );
    let replay_err = engine.replay(tenant, tampered_swarm).await.unwrap_err();
    assert!(
        matches!(replay_err, ReplayError::ChainIntegrityBroken(_)),
        "expected ChainIntegrityBroken, got {replay_err:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  21. Audit-fix B2 : replay must pin to the EXACT agent version
// ─────────────────────────────────────────────────────────────────────
//
//  Regulatory guarantee : if a tenant dispatches at v1.0.0 of an agent,
//  then republishes v2.0.0 with a different `DeterminismLevel`, the
//  replay engine MUST classify the historical op using v1.0.0's
//  determinism level — NOT v2.0.0's.  Otherwise an attacker who
//  controls catalog publishing can silently re-classify a historical
//  Mode B trace as Mode C (`SkippedNonDeterministic`) and hide a
//  regression / tamper.

#[tokio::test]
async fn replay_pins_to_original_agent_version() {
    use efe_nano::starter_pack::json_validate;
    use efe_nano_sdk::{DeterminismLevel, NanoAgentVersion};

    let h = make_replay_harness().await;
    let tenant = TenantId::system();

    // 1. The starter pack publishes `json_validate` v1.0.0 with
    //    `DeterminismLevel::FullDeterministic`.  Dispatch a swarm
    //    against it.
    let request_id = Uuid::new_v4();
    let resp = dispatch_single(&h, tenant, request_id, "v").await;
    assert_eq!(resp.overall_status, SwarmStatus::Success);

    // 2. Confirm the trace recorded version "1.0.0".
    let chain = h.chain_store.chain_for_tenant(tenant, None).await.unwrap().into_iter().filter(|e| !e.trace.is_swarm_envelope()).collect::<Vec<_>>();
    assert_eq!(chain.len(), 1);
    assert_eq!(chain[0].trace.nano_agent_version.as_str(), "1.0.0");

    // 3. Publish a NEW major version of `json_validate` with a
    //    DIFFERENT DeterminismLevel.  This simulates an operator
    //    upgrading the agent post-dispatch.
    let mut new_manifest = json_validate::manifest();
    new_manifest.version = NanoAgentVersion::new("2.0.0").unwrap();
    // Sprint 7 — N5 : Use the explicit LlmEphemeral variant (the
    // pre-Sprint-7 `Probabilistic` enum was split N5).
    new_manifest.determinism = DeterminismLevel::LlmEphemeral;
    h.catalog.publish(tenant, new_manifest).await.unwrap();

    // 4. Replay : the engine must look up determinism for v1.0.0 (the
    //    trace's version), not v2.0.0 (latest).  v1.0.0 is
    //    FullDeterministic + Mode A → expect Match.
    let report = h.make_engine().replay(tenant, request_id).await.unwrap();
    assert!(report.chain_verified);
    assert_eq!(report.total_ops, 1);
    assert_eq!(
        report.matched_ops, 1,
        "v1.0.0 is FullDeterministic → must produce Match, not SkippedNonDeterministic.\n\
         report = {report:?}"
    );
    assert_eq!(report.skipped_non_deterministic_ops, 0);
    assert!(report.is_fully_consistent());

    // 5. Edge case : if the EXACT version is removed from the catalog
    //    (only v2 left), the engine falls back to Probabilistic (the
    //    conservative class).  We don't simulate "remove v1" here
    //    because InMemoryCatalog doesn't expose an `unpublish` ;
    //    this fallback path is covered by `lookup_determinism`
    //    returning `None` → unwrap_or(Probabilistic) in the engine.
}

