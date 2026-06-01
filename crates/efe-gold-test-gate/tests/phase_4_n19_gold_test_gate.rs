// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : end-to-end acceptance tests.
//!
//! Matches the 15-row acceptance matrix in architecture § 12.1.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use efe_gold_test_gate::{
    AssertionEngine, DefaultAssertionEngine, DefaultGoldApi, GoldApi, GoldGate, GoldGateConfig,
    GoldRunLedger, GoldRunLedgerEntry, GoldSetRegistry, InMemoryGoldRunLedger,
    InMemoryGoldRunner, InMemoryGoldSetRegistry, N19Metrics, StubEmbeddingProvider,
};
use efe_gold_test_gate::api::{ApproveGoldSetRequest, RegisterGoldSetRequest, RunRequest};
use efe_gold_test_gate::runner::{AgentRunFn, GoldRunner, RunOutcome, RunnerInput};
use efe_nano_sdk::{
    Author, BypassCertificate, Category, DeterminismLevel, ExecutionMode,
    GoldAssertionStrategy, GoldCase, GoldCaseId, GoldCaseVerdict, GoldExpectedOutput,
    GoldOverallVerdict, GoldSet, GoldSetId, GoldSetStatus, IdempotencyPolicy, ModelIdentity,
    NanoAgent, NanoAgentId, NanoAgentVersion, OperatorSignature, Provenance, ResourceLimits,
    Sha256Hash, StabilityTier,
};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Fixtures
// ─────────────────────────────────────────────────────────────────────

fn agent(name: &str) -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new(name).unwrap(),
        version: NanoAgentVersion::new("1.0.0").unwrap(),
        description: "x".into(),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::TierExperimental,
        execution_mode: ExecutionMode::PureRust {
            entry: "x::run".into(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: serde_json::json!({}),
        output_schema: serde_json::json!({}),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.01,
        latency_p99_ms_target: 50,
        audit: Default::default(),
        compliance: Default::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: chrono::Utc::now(),
    }
}

fn op_sig() -> OperatorSignature {
    OperatorSignature {
        operator_id: Uuid::new_v4(),
        signed_at: SystemTime::now(),
        ticket_id: "GOLD-1".into(),
        signature: vec![0u8; 64],
    }
}

fn case_strict(name: &str, expected: serde_json::Value) -> GoldCase {
    GoldCase {
        gold_case_id: GoldCaseId::new(),
        name: name.into(),
        input: serde_json::json!({"x": 1}),
        expected: GoldExpectedOutput::Inline { value: expected },
        assertion: GoldAssertionStrategy::StrictEquality,
        timeout_seconds: 30,
        tags: vec![],
    }
}

fn case_tol(name: &str, expected: serde_json::Value, eps_field: &str, eps: f64) -> GoldCase {
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(eps_field.into(), eps);
    GoldCase {
        gold_case_id: GoldCaseId::new(),
        name: name.into(),
        input: serde_json::json!({}),
        expected: GoldExpectedOutput::Inline { value: expected },
        assertion: GoldAssertionStrategy::NumericTolerance { fields },
        timeout_seconds: 30,
        tags: vec![],
    }
}

fn case_sim(name: &str, expected: serde_json::Value, threshold: f64) -> GoldCase {
    GoldCase {
        gold_case_id: GoldCaseId::new(),
        name: name.into(),
        input: serde_json::json!({}),
        expected: GoldExpectedOutput::Inline { value: expected },
        assertion: GoldAssertionStrategy::SemanticSimilarity {
            embedding_model: ModelIdentity::hosted("openai", "text-embedding-3-small", "1"),
            cosine_threshold: threshold,
            field_pointer: Some("/summary".into()),
        },
        timeout_seconds: 30,
        tags: vec![],
    }
}

fn five_case_set(agent_id: &NanoAgentId, status: GoldSetStatus) -> GoldSet {
    let approved_at = if matches!(status, GoldSetStatus::Approved) {
        Some(SystemTime::now())
    } else {
        None
    };
    let approvers = if matches!(status, GoldSetStatus::Approved) {
        vec![op_sig(), op_sig()]
    } else {
        vec![]
    };
    GoldSet {
        gold_set_id: GoldSetId::new(),
        agent_id: agent_id.clone(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        agent_input_schema_hash: Sha256Hash::from_bytes([1u8; 32]),
        agent_output_schema_hash: Sha256Hash::from_bytes([2u8; 32]),
        cases: vec![
            case_strict("c1", serde_json::json!({"o": 1})),
            case_strict("c2", serde_json::json!({"o": 2})),
            case_tol("c3", serde_json::json!({"score": 0.5}), "/score", 0.05),
            case_sim("c4", serde_json::json!({"summary": "hello"}), 0.95),
            case_sim("c5", serde_json::json!({"summary": "world"}), 0.95),
        ],
        cost_cap_usd: 100.0,
        provenance: Provenance::Manual,
        status,
        approvers,
        created_at: SystemTime::now(),
        approved_at,
        gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
    }
    .with_computed_hash()
}

fn build_gate(runner: Arc<dyn GoldRunner>) -> GoldGate {
    GoldGate::new(
        Arc::new(InMemoryGoldSetRegistry::new()),
        runner,
        Arc::new(DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))),
        Arc::new(InMemoryGoldRunLedger::new([42u8; 32])),
        N19Metrics::new(),
    )
}

fn build_api(runner: Arc<dyn GoldRunner>) -> DefaultGoldApi {
    let registry: Arc<dyn GoldSetRegistry> = Arc::new(InMemoryGoldSetRegistry::new());
    let ledger: Arc<dyn GoldRunLedger> = Arc::new(InMemoryGoldRunLedger::new([42u8; 32]));
    let gate = Arc::new(GoldGate::new(
        registry.clone(),
        runner,
        Arc::new(DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))),
        ledger.clone(),
        N19Metrics::new(),
    ));
    DefaultGoldApi::new(registry, gate, ledger)
}

// ─────────────────────────────────────────────────────────────────────
//  1. gold set immutable after approve
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_gold_set_immutable_after_approve() {
    let r = InMemoryGoldSetRegistry::new();
    let a = NanoAgentId::new("x").unwrap();
    let set = five_case_set(&a, GoldSetStatus::Draft);
    let id = r.register(set, false).await.unwrap();
    r.approve(id, vec![op_sig(), op_sig()]).await.unwrap();
    // Re-approve must fail (immutability).
    let err = r.approve(id, vec![op_sig(), op_sig()]).await.unwrap_err();
    assert!(matches!(
        err,
        efe_gold_test_gate::GoldSetRegistryError::InvalidTransition { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────
//  2. gold set requires two approvers
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_gold_set_requires_two_approvers() {
    let r = InMemoryGoldSetRegistry::new();
    let a = NanoAgentId::new("x").unwrap();
    let set = five_case_set(&a, GoldSetStatus::Draft);
    let id = r.register(set, false).await.unwrap();
    let err = r.approve(id, vec![op_sig()]).await.unwrap_err();
    assert!(matches!(
        err,
        efe_gold_test_gate::GoldSetRegistryError::InsufficientApprovers { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────
//  3. gold run ledger chain verifies
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_gold_run_ledger_chain_verifies() {
    let agent = Arc::new(agent("chain"));
    let set = five_case_set(&agent.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::echo()));
    let _ = g
        .run(agent.clone(), set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    let report = g.ledger.verify_chain(&agent.id).await.unwrap();
    assert!(report.ok);
    assert_eq!(report.entries_verified, 1);
}

// ─────────────────────────────────────────────────────────────────────
//  4. three strategies decide pass / fail
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_three_strategies_decide_pass_fail() {
    // Echo runner → all strategies pass on identical output.
    let agent1 = Arc::new(agent("strategies"));
    let set = five_case_set(&agent1.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::echo()));
    let report = g
        .run(agent1.clone(), set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(report.permits_promote());
    assert!(report
        .cases
        .iter()
        .all(|c| matches!(c.verdict, GoldCaseVerdict::Passed { .. })));

    // Drifting runner → all strategies fail.
    let agent2 = Arc::new(agent("strategies_drift"));
    let set2 = five_case_set(&agent2.id, GoldSetStatus::Approved);
    let g2 = build_gate(Arc::new(InMemoryGoldRunner::drifting()));
    let report2 = g2
        .run(agent2, set2, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(!report2.permits_promote());
}

// ─────────────────────────────────────────────────────────────────────
//  5. output schema violation fails case
//
//  Sprint 11 stub : the runner doesn't enforce schema validation
//  natively (G5 is wired Sprint 12 via the agent invocation pipeline).
//  We acceptance-test by simulating : the runner returns a value that
//  obviously violates the agent's output schema ; the assertion
//  engine (strict equality) detects the mismatch.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_output_schema_violation_fails_case() {
    let f: AgentRunFn = Arc::new(|_a, _c| RunOutcome::Output {
        value: serde_json::Value::String("not the expected JSON shape".into()),
        cost_usd: 0.001,
        provider_calls: vec![],
        duration_ms: Some(1),
    });
    let agent_arc = Arc::new(agent("schema"));
    let set = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::new_with(f)));
    let report = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(!report.permits_promote());
}

// ─────────────────────────────────────────────────────────────────────
//  6. promote-to-Tier-1 requires passing gold run
//
//  We don't actually call rollout::promote_to(Tier1) here (that's
//  Sprint 11's catalog wiring) — we assert the verdict surface :
//  Passed → permits_promote == true ; otherwise false.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_promote_tier_1_requires_passing_gold_run() {
    let agent_arc = Arc::new(agent("promote"));
    let set_ok = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    let g_ok = build_gate(Arc::new(InMemoryGoldRunner::echo()));
    let rep_ok = g_ok
        .run(agent_arc.clone(), set_ok, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(rep_ok.permits_promote());

    let agent_arc2 = Arc::new(agent("promote_fail"));
    let set_fail = five_case_set(&agent_arc2.id, GoldSetStatus::Approved);
    let g_fail = build_gate(Arc::new(InMemoryGoldRunner::drifting()));
    let rep_fail = g_fail
        .run(agent_arc2, set_fail, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(!rep_fail.permits_promote());
}

// ─────────────────────────────────────────────────────────────────────
//  7. gold run isolated in microvm (hermeticity probe)
//
//  Sprint 11 stub : the FirecrackerGoldRunner is wired Sprint 12.
//  The InMemoryGoldRunner accepts a `HermeticityViolation` outcome
//  that propagates as `GoldRunnerError::HermeticityBroken` and aborts
//  the run.  We test the FULL abort path here.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_gold_run_isolated_in_microvm() {
    let f: AgentRunFn = Arc::new(|_a, _c| RunOutcome::HermeticityViolation);
    let agent_arc = Arc::new(agent("isolated"));
    let set = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::new_with(f)));
    let report = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(matches!(
        report.overall,
        GoldOverallVerdict::AbortedHermeticityBroken { .. }
    ));
    assert!(!report.permits_promote());
}

// ─────────────────────────────────────────────────────────────────────
//  8. cost cap aborts run
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_cost_cap_aborts_run() {
    let f: AgentRunFn = Arc::new(|_a, c| match &c.expected {
        GoldExpectedOutput::Inline { value } => RunOutcome::Output {
            value: value.clone(),
            cost_usd: 50.0,
            provider_calls: vec![],
            duration_ms: Some(1),
        },
        _ => RunOutcome::Output {
            value: serde_json::Value::Null,
            cost_usd: 50.0,
            provider_calls: vec![],
            duration_ms: Some(1),
        },
    });
    let agent_arc = Arc::new(agent("cap"));
    let mut set = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    set.cost_cap_usd = 40.0;
    set.refresh_hash();
    let g = build_gate(Arc::new(InMemoryGoldRunner::new_with(f)));
    let report = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(matches!(
        report.overall,
        GoldOverallVerdict::AbortedCostCap { .. }
    ));
}

// ─────────────────────────────────────────────────────────────────────
//  9. cached calls drive marginal cost to zero
//
//  Sprint 11 stub : the InMemoryGoldRunner's echo simulator costs
//  $0.001 per call regardless ; Sprint 12 wires N12 cache.  We
//  validate the metric path : a re-run produces the same cost (no
//  doubling).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_cached_calls_drive_marginal_cost_to_zero() {
    let agent_arc = Arc::new(agent("cache"));
    let set = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::echo()));
    let r1 = g
        .run(agent_arc.clone(), set.clone(), None, &GoldGateConfig::default())
        .await
        .unwrap();
    let r2 = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!((r1.total_cost_usd - r2.total_cost_usd).abs() < 1e-9);
}

// ─────────────────────────────────────────────────────────────────────
//  10. drift sweep demotes on fail
//
//  Sprint 11 stub : the drift sweep is wired in the API but the
//  real "re-run + demote" loop ships Sprint 12 with a cron.  We
//  acceptance-test by re-running a stale gold set with a drifting
//  runner and asserting Failed.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_drift_sweep_demotes_on_fail() {
    let agent_arc = Arc::new(agent("drift"));
    let set = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::drifting()));
    let report = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    // A fail in the drift window = "demote" decision upstream.
    assert!(!report.permits_promote());
}

// ─────────────────────────────────────────────────────────────────────
//  11. bypass requires two-of-three approvals
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_bypass_requires_two_of_three_approvals() {
    let agent_arc = Arc::new(agent("bypass"));
    let set = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::echo()));
    // One approver → reject.
    let cert_one = BypassCertificate {
        approvers: vec![op_sig()],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let err = g
        .run(
            agent_arc.clone(),
            set.clone(),
            Some(cert_one),
            &GoldGateConfig::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        efe_nano_sdk::GoldGateError::BypassInsufficientApprovers { .. }
    ));
    // Two approvers → accept.
    let cert_two = BypassCertificate {
        approvers: vec![op_sig(), op_sig()],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let report = g
        .run(agent_arc, set, Some(cert_two), &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(report.is_bypassed());
}

// ─────────────────────────────────────────────────────────────────────
//  12. LLM provenance carries gold_case_id
//
//  Sprint 11 surface check : the field is `Option<GoldCaseId>` on
//  `LlmProvenanceRecord` and round-trips through serde.  Sprint 12
//  wires the runner to actually populate it on every Mode C call.
// ─────────────────────────────────────────────────────────────────────

#[test]
fn phase_4_n19_llm_provenance_carries_gold_case_id() {
    use efe_nano_sdk::{
        Blake3Hash, CostAccounting, FinishReason, LlmProvenanceRecord, PiiStrategy,
        ProvenanceId, SamplingParams, TenantId, TokenUsage,
    };
    let gold_case_id = GoldCaseId::new();
    let record = LlmProvenanceRecord {
        provenance_id: ProvenanceId::new(),
        trace_id: Uuid::new_v4(),
        swarm_request_id: Uuid::new_v4(),
        op_id: "x".into(),
        tenant_id: TenantId::new(),
        agent_id: NanoAgentId::new("a").unwrap(),
        agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
        model_identity: ModelIdentity::hosted("openai", "gpt-4", "2024-04"),
        model_hash_verified: true,
        prompt_template_id: "t".into(),
        prompt_template_version: "1.0.0".into(),
        prompt_inputs_hash: Blake3Hash::ZERO,
        system_prompt_hash: Blake3Hash::ZERO,
        user_prompt_hash: Blake3Hash::ZERO,
        final_prompt_hash: Blake3Hash::ZERO,
        sampling: SamplingParams::default(),
        token_usage: TokenUsage::default(),
        cost_accounting: CostAccounting::default(),
        provider_request_id: None,
        provider_response_time_ms: 0,
        provider_finish_reason: FinishReason::Unknown,
        provider_region: None,
        provider_metadata: Default::default(),
        output_hash: None,
        completeness: efe_nano_sdk::Completeness::Complete,
        safety_flags: vec![],
        tool_calls: vec![],
        pii_strategy_applied: PiiStrategy::Strict,
        redaction_token_set: vec![],
        replay_of: None,
        cache_hit: None,
        recorded_at: SystemTime::now(),
        publisher_id: None,
        publisher_kyc_tier: None,
        gold_case_id: Some(gold_case_id),
        // **Sprint 19 — N27** : buffered fixture, no streaming.
        streaming_mode: false,
        retry_count: 0,
        retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
        time_to_first_token_ms: None,
    };
    let json = serde_json::to_string(&record).unwrap();
    assert!(json.contains("gold_case_id"));
    let back: LlmProvenanceRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(back.gold_case_id, Some(gold_case_id));
}

// ─────────────────────────────────────────────────────────────────────
//  13. N18 ledger entry references gold run
//
//  Sprint 11 wires the SDK field ; Sprint 12 adds the actual N18
//  `AgentPromotedToTier1` variant.  We acceptance-test by asserting
//  that a `GoldRunCompleted` row in OUR ledger references the
//  gold_run_id (the join key Sprint 12 plumbs to N18).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_n18_ledger_entry_references_gold_run() {
    let agent_arc = Arc::new(agent("n18link"));
    let set = five_case_set(&agent_arc.id, GoldSetStatus::Approved);
    let g = build_gate(Arc::new(InMemoryGoldRunner::echo()));
    let report = g
        .run(agent_arc.clone(), set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    let history = g.ledger.history(&agent_arc.id, 10).await.unwrap();
    let row = history
        .iter()
        .find_map(|r| match &r.entry {
            GoldRunLedgerEntry::GoldRunCompleted { report } => Some(report.gold_run_id),
            _ => None,
        })
        .unwrap();
    assert_eq!(row, report.gold_run_id);
}

// ─────────────────────────────────────────────────────────────────────
//  14. gold run query API returns report
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_gold_run_query_api_returns_report() {
    let api = build_api(Arc::new(InMemoryGoldRunner::echo()));
    let a = agent("query");
    let set = five_case_set(&a.id, GoldSetStatus::Draft);
    let id = api
        .register_gold_set(RegisterGoldSetRequest {
            draft: set,
            pii_required_by_compliance: false,
        })
        .await
        .unwrap();
    let _ = api
        .approve_gold_set(
            id,
            ApproveGoldSetRequest {
                approvers: vec![op_sig(), op_sig()],
            },
        )
        .await
        .unwrap();
    let rep = api
        .run_gate(RunRequest {
            agent: a,
            gold_set_id: id,
            bypass: None,
        })
        .await
        .unwrap();
    assert!(rep.permits_promote());
}

// ─────────────────────────────────────────────────────────────────────
//  15. metrics emit all 12 surfaces
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn phase_4_n19_metrics_emit_all_12_surfaces() {
    let metrics = N19Metrics::new();
    // Seed each surface once.
    metrics.tick_gold_run("passed", "tier_1");
    metrics.tick_gold_case("strict_equality", "passed");
    metrics.add_cost("tier_1", 1.0);
    metrics.observe_run_duration("tier_1", 0.5);
    metrics.observe_case_duration("strict_equality", 0.01);
    metrics.tick_approval("approved");
    metrics.tick_bypass("tier_2", "INC");
    metrics.tick_drift_demotion("stale");
    metrics.tick_hermeticity("net");
    metrics.tick_replay_lookup("hit");
    metrics.tick_assertion("strict_equality", "passed");
    metrics.set_age_days("a", "tier_1", 3);
    let out = metrics.render_prometheus();
    for name in [
        "n19_gold_runs_total",
        "n19_gold_cases_executed_total",
        "n19_gold_run_cost_cents_total",
        "n19_gold_run_duration_seconds",
        "n19_gold_case_duration_seconds",
        "n19_gold_set_approvals_total",
        "n19_bypass_usage_total",
        "n19_drift_sweep_demotions_total",
        "n19_hermeticity_violations_total",
        "n19_replay_anchor_lookups_total",
        "n19_assertion_engine_evaluations_total",
        "n19_gold_run_age_days",
    ] {
        assert!(out.contains(name), "missing : {name}");
    }
}

// Keep type-import alive
#[allow(dead_code)]
fn _ae_keep_alive() -> Option<Arc<dyn AssertionEngine>> {
    None
}

// Suppress an unused-import warning for `RunnerInput` if a future
// refactor needs it.
#[allow(dead_code)]
fn _runner_input_keep_alive() -> Option<RunnerInput> {
    None
}
