// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : post-audit regression tests.
//!
//! These tests pin down bugs caught during the chirurgical audit
//! pass on top of the original 15 acceptance + 127 unit tests.
//!
//! | # | Defect | Where | Fix |
//! |---|---|---|---|
//! | B1 | `GoldGate::run` accepted a gold set for ANY agent (mismatch ok) | `gate.rs::run` | agent_id + agent_version equality check |
//! | B2 | `GoldSet.gold_set_hash` was a free-form field never validated against content (F6) | `gold.rs` | `compute_canonical_hash` + `verify_canonical_hash` + check in `GoldGate::run` |
//! | B3 | `lookup_gold_run` returned `Ok(None)` unconditionally — unusable | `api.rs` + `ledger/mod.rs` | new trait method `find_by_gold_run_id` + ledger scan |
//! | B4 | `GoldCaseVerdict` serde tag was originally `kind`, collided with `Errored.kind` field | `gold.rs` (already fixed during initial code gen) | `tag = "verdict"` |

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use efe_gold_test_gate::{
    DefaultAssertionEngine, GoldGate, GoldGateConfig, GoldRunLedger, GoldSetRegistry,
    InMemoryGoldRunLedger, InMemoryGoldRunner, InMemoryGoldSetRegistry, N19Metrics,
    StubEmbeddingProvider,
};
use efe_nano_sdk::{
    Author, BypassCertificate, Category, DeterminismLevel, ExecutionMode,
    GoldAssertionStrategy, GoldCase, GoldCaseId, GoldExpectedOutput, GoldGateError,
    GoldRunId, GoldSet, GoldSetId, GoldSetStatus, IdempotencyPolicy, NanoAgent, NanoAgentId,
    NanoAgentVersion, OperatorSignature, Provenance, ResourceLimits, Sha256Hash,
    StabilityTier,
};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Fixtures
// ─────────────────────────────────────────────────────────────────────

fn agent(name: &str) -> Arc<NanoAgent> {
    Arc::new(NanoAgent {
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
    })
}

fn op_sig() -> OperatorSignature {
    OperatorSignature {
        operator_id: Uuid::new_v4(),
        signed_at: SystemTime::now(),
        ticket_id: "GOLD-1".into(),
        signature: vec![0u8; 64],
    }
}

fn five_case_set_for(agent: &NanoAgent) -> GoldSet {
    let cases: Vec<_> = (0..5)
        .map(|i| GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: format!("c{i}"),
            input: serde_json::json!({}),
            expected: GoldExpectedOutput::Inline {
                value: serde_json::json!({"o": i}),
            },
            assertion: GoldAssertionStrategy::StrictEquality,
            timeout_seconds: 30,
            tags: vec![],
        })
        .collect();
    GoldSet {
        gold_set_id: GoldSetId::new(),
        agent_id: agent.id.clone(),
        agent_version: agent.version.clone(),
        agent_input_schema_hash: Sha256Hash::from_bytes([1u8; 32]),
        agent_output_schema_hash: Sha256Hash::from_bytes([2u8; 32]),
        cases,
        cost_cap_usd: 100.0,
        provenance: Provenance::Manual,
        status: GoldSetStatus::Approved,
        approvers: vec![op_sig(), op_sig()],
        created_at: SystemTime::now(),
        approved_at: Some(SystemTime::now()),
        gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
    }
    .with_computed_hash()
}

fn build_gate() -> Arc<GoldGate> {
    Arc::new(GoldGate::new(
        Arc::new(InMemoryGoldSetRegistry::new()),
        Arc::new(InMemoryGoldRunner::echo()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        Arc::new(InMemoryGoldRunLedger::new([42u8; 32])),
        N19Metrics::new(),
    ))
}

// ─────────────────────────────────────────────────────────────────────
//  B1 — gold set / agent identity mismatch rejected
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b1_mismatched_agent_id_rejected() {
    let agent_a = agent("alpha");
    let agent_b = agent("beta");
    let set_for_a = five_case_set_for(&agent_a);
    let g = build_gate();
    // Try to run agent_b against a set keyed to agent_a.
    let err = g
        .run(agent_b, set_for_a, None, &GoldGateConfig::default())
        .await
        .unwrap_err();
    assert!(matches!(err, GoldGateError::Backend { ref message } if message.contains("agent_id")));
}

#[tokio::test]
async fn audit_b1_mismatched_agent_version_rejected() {
    let agent_v1 = agent("ver");
    let mut set = five_case_set_for(&agent_v1);
    // Tamper the gold set's version reference (we have to bypass the
    // hash check by re-hashing) ; the run() agent will use 1.0.0
    // but the set will claim 2.0.0.
    set.agent_version = NanoAgentVersion::new("2.0.0").unwrap();
    set.refresh_hash();
    let g = build_gate();
    let err = g
        .run(agent_v1, set, None, &GoldGateConfig::default())
        .await
        .unwrap_err();
    assert!(matches!(err, GoldGateError::Backend { ref message } if message.contains("agent_version")));
}

// ─────────────────────────────────────────────────────────────────────
//  B2 — gold set hash mismatch rejected (F6)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b2_tampered_hash_rejected() {
    let agent_arc = agent("hash_check");
    let mut set = five_case_set_for(&agent_arc);
    // Tamper with the cases (silent registry rewrite) but leave the
    // hash intact.  The gate MUST detect the divergence and abort.
    let was_hash = set.gold_set_hash;
    set.cost_cap_usd = 9_999.0; // mutation
    // (don't refresh_hash : we deliberately leave the stored hash
    // pointing to the OLD content)
    assert_eq!(set.gold_set_hash, was_hash);
    let g = build_gate();
    let err = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap_err();
    assert!(matches!(err, GoldGateError::Backend { ref message } if message.contains("hash")));
}

#[tokio::test]
async fn audit_b2_clean_hash_passes() {
    let agent_arc = agent("clean_hash");
    let set = five_case_set_for(&agent_arc);
    assert!(set.verify_canonical_hash());
    let g = build_gate();
    let report = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(report.permits_promote());
}

#[test]
fn audit_b2_compute_canonical_hash_is_deterministic() {
    let agent_arc = agent("det");
    let set = five_case_set_for(&agent_arc);
    let h1 = set.compute_canonical_hash();
    let h2 = set.compute_canonical_hash();
    assert_eq!(h1, h2);
}

#[test]
fn audit_b2_hash_changes_when_cases_change() {
    let agent_arc = agent("det");
    let s1 = five_case_set_for(&agent_arc);
    let mut s2 = s1.clone();
    s2.cases[0].name = "renamed".into();
    s2.refresh_hash();
    assert_ne!(s1.gold_set_hash, s2.gold_set_hash);
}

#[test]
fn audit_b2_hash_unchanged_when_status_changes() {
    // The canonical hash excludes mutable lifecycle fields (status,
    // approvers, created_at, approved_at).  Changing status does NOT
    // invalidate the hash.
    let agent_arc = agent("life");
    let s1 = five_case_set_for(&agent_arc);
    let mut s2 = s1.clone();
    s2.status = GoldSetStatus::Archived;
    s2.approvers.clear();
    assert_eq!(s1.compute_canonical_hash(), s2.compute_canonical_hash());
}

#[test]
fn audit_b2_hash_changes_when_cost_cap_changes() {
    let agent_arc = agent("cap");
    let mut s1 = five_case_set_for(&agent_arc);
    s1.cost_cap_usd = 100.0;
    s1.refresh_hash();
    let mut s2 = s1.clone();
    s2.cost_cap_usd = 200.0;
    s2.refresh_hash();
    assert_ne!(s1.gold_set_hash, s2.gold_set_hash);
}

#[test]
fn audit_b2_with_computed_hash_idempotent() {
    let agent_arc = agent("idem");
    let set = five_case_set_for(&agent_arc);
    let set2 = set.clone().with_computed_hash();
    assert_eq!(set.gold_set_hash, set2.gold_set_hash);
}

// ─────────────────────────────────────────────────────────────────────
//  B3 — lookup_gold_run finds the row
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b3_lookup_gold_run_finds_completed_run() {
    let agent_arc = agent("lookup");
    let set = five_case_set_for(&agent_arc);
    let registry: Arc<dyn GoldSetRegistry> = Arc::new(InMemoryGoldSetRegistry::new());
    let ledger: Arc<dyn GoldRunLedger> = Arc::new(InMemoryGoldRunLedger::new([42u8; 32]));
    let g = GoldGate::new(
        registry.clone(),
        Arc::new(InMemoryGoldRunner::echo()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        ledger.clone(),
        N19Metrics::new(),
    );
    let report = g
        .run(agent_arc, set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    let row = ledger
        .find_by_gold_run_id(report.gold_run_id)
        .await
        .unwrap();
    assert!(row.is_some());
}

#[tokio::test]
async fn audit_b3_lookup_gold_run_returns_none_for_unknown_id() {
    let ledger: Arc<dyn GoldRunLedger> = Arc::new(InMemoryGoldRunLedger::new([42u8; 32]));
    let row = ledger
        .find_by_gold_run_id(GoldRunId::new())
        .await
        .unwrap();
    assert!(row.is_none());
}

#[tokio::test]
async fn audit_b3_lookup_gold_run_finds_bypass_row() {
    let agent_arc = agent("bp_lookup");
    let set = five_case_set_for(&agent_arc);
    let ledger: Arc<dyn GoldRunLedger> = Arc::new(InMemoryGoldRunLedger::new([42u8; 32]));
    let g = GoldGate::new(
        Arc::new(InMemoryGoldSetRegistry::new()),
        Arc::new(InMemoryGoldRunner::always_crashes()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        ledger.clone(),
        N19Metrics::new(),
    );
    let cert = BypassCertificate {
        approvers: vec![op_sig(), op_sig()],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400),
    };
    let report = g
        .run(agent_arc, set, Some(cert), &GoldGateConfig::default())
        .await
        .unwrap();
    assert!(report.is_bypassed());
    let row = ledger
        .find_by_gold_run_id(report.gold_run_id)
        .await
        .unwrap();
    assert!(row.is_some());
}

// ─────────────────────────────────────────────────────────────────────
//  B4 — serde tag collision regression test
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b4_gold_case_verdict_serde_uses_verdict_tag() {
    use efe_nano_sdk::{GoldCaseVerdict, GoldErrorKind};
    let v = GoldCaseVerdict::Errored {
        kind: GoldErrorKind::Timeout,
        message: "x".into(),
    };
    let json = serde_json::to_string(&v).unwrap();
    // Must contain "verdict" : "errored" (not "kind" : "errored").
    assert!(json.contains("\"verdict\":\"errored\""));
    // And ALSO contain the GoldErrorKind variant field.
    assert!(json.contains("\"kind\":\"timeout\""));
    // Round-trip.
    let back: GoldCaseVerdict = serde_json::from_str(&json).unwrap();
    assert_eq!(back, v);
}

// ─────────────────────────────────────────────────────────────────────
//  B5 — gold_run_id stable across ledger entries
//
//  The gold_run_id is the join key between the publish ledger (N18)
//  and the gold-run ledger (N19) ; it MUST be stable across the
//  GoldRunCompleted ↔ GoldRunBypassed paths.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b5_gold_run_id_in_completed_report_matches_ledger() {
    let agent_arc = agent("idmatch");
    let set = five_case_set_for(&agent_arc);
    let ledger: Arc<dyn GoldRunLedger> = Arc::new(InMemoryGoldRunLedger::new([42u8; 32]));
    let g = GoldGate::new(
        Arc::new(InMemoryGoldSetRegistry::new()),
        Arc::new(InMemoryGoldRunner::echo()),
        Arc::new(DefaultAssertionEngine::new(Arc::new(
            StubEmbeddingProvider::new(),
        ))),
        ledger.clone(),
        N19Metrics::new(),
    );
    let report = g
        .run(agent_arc.clone(), set, None, &GoldGateConfig::default())
        .await
        .unwrap();
    let history = ledger.history(&agent_arc.id, 10).await.unwrap();
    let run_id_from_ledger = history
        .iter()
        .find_map(|r| match &r.entry {
            efe_gold_test_gate::GoldRunLedgerEntry::GoldRunCompleted { report } => {
                Some(report.gold_run_id)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(run_id_from_ledger, report.gold_run_id);
}

// ─────────────────────────────────────────────────────────────────────
//  B6 — TooFewCases threshold pins the minimum exactly
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b6_too_few_cases_pins_minimum_at_five() {
    use efe_nano_sdk::{GoldSetError, GOLD_SET_MIN_CASES};
    assert_eq!(GOLD_SET_MIN_CASES, 5);
    let agent_arc = agent("min");
    let mut set = five_case_set_for(&agent_arc);
    set.cases.truncate(4);
    set.refresh_hash();
    let err = set.validate(false).unwrap_err();
    assert!(matches!(err, GoldSetError::TooFewCases { got: 4, need: 5 }));
}

// ─────────────────────────────────────────────────────────────────────
//  B7 — bypass with future valid_until > 14d rejected
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b7_bypass_valid_until_over_14_days_rejected() {
    let agent_arc = agent("life14");
    let set = five_case_set_for(&agent_arc);
    let g = build_gate();
    let cert = BypassCertificate {
        approvers: vec![op_sig(), op_sig()],
        ticket_id: "INC-1".into(),
        rationale: "x".repeat(64),
        valid_until: SystemTime::now() + Duration::from_secs(86_400 * 20),
    };
    let err = g
        .run(agent_arc, set, Some(cert), &GoldGateConfig::default())
        .await
        .unwrap_err();
    assert!(matches!(err, GoldGateError::BypassInvalidLifetime));
}

// ─────────────────────────────────────────────────────────────────────
//  B8 — exact passing threshold passes (boundary)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_b8_exactly_five_cases_passes() {
    let agent_arc = agent("five");
    let set = five_case_set_for(&agent_arc);
    assert_eq!(set.cases.len(), 5);
    assert!(set.validate(false).is_ok());
}

// ─────────────────────────────────────────────────────────────────────
//  B9 — verify_canonical_hash flips on body tamper
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b9_verify_canonical_hash_flips_on_tamper() {
    let agent_arc = agent("tamper");
    let mut set = five_case_set_for(&agent_arc);
    assert!(set.verify_canonical_hash());
    set.cases[0].name = "tampered".into();
    assert!(!set.verify_canonical_hash());
}

// ─────────────────────────────────────────────────────────────────────
//  B10 — Cross-cutting : GoldSetRef round-trip
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b10_gold_set_ref_serde_round_trip() {
    use efe_nano_sdk::GoldSetRef;
    let r = GoldSetRef {
        gold_set_id: GoldSetId::new(),
        gold_set_hash: Sha256Hash::from_bytes([3u8; 32]),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: GoldSetRef = serde_json::from_str(&json).unwrap();
    assert_eq!(r, back);
}

// ─────────────────────────────────────────────────────────────────────
//  B11 — NanoAgent.gold_set_ref field is backward-compat (None default)
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b11_pre_n19_nanoagent_json_round_trips_with_no_gold_set_ref() {
    // A pre-N19 producer omits the gold_set_ref / signature /
    // co_publishing fields ; serde must accept that.  Round-trip via
    // a freshly-built NanoAgent, serialise (skip_serializing_if
    // strips the absent fields), then re-parse and check the
    // optional fields are None.
    let a = agent("backward");
    let json = serde_json::to_string(&*a).unwrap();
    assert!(!json.contains("\"gold_set_ref\":"));
    assert!(!json.contains("\"signature\":"));
    assert!(!json.contains("\"co_publishing\":"));
    let back: NanoAgent = serde_json::from_str(&json).unwrap();
    assert!(back.gold_set_ref.is_none());
    assert!(back.signature.is_none());
    assert!(back.co_publishing.is_none());
}

// ─────────────────────────────────────────────────────────────────────
//  B12 — Sanity : GoldAssertionStrategy::SchemaValidationOnly rejected
//         on production sets
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_b12_schema_validation_only_rejected_in_validate() {
    let agent_arc = agent("schema_only");
    let mut set = five_case_set_for(&agent_arc);
    set.cases[0].assertion = GoldAssertionStrategy::SchemaValidationOnly;
    set.refresh_hash();
    let err = set.validate(false).unwrap_err();
    assert!(matches!(
        err,
        efe_nano_sdk::GoldSetError::InvalidAssertionParam { .. }
    ));
}
