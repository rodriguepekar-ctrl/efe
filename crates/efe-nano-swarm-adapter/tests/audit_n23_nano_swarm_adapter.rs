// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23** : post-audit regression tests pinning the
//! externally-observable invariants from the architecture doc.
//!
//! Sister style of `audit_n18_publisher_identity.rs`,
//! `audit_n19_gold_test_gate.rs`, `audit_n22_http_gateway.rs`.
//!
//! | I-# | Invariant pinned                                                                      |
//! |-----|----------------------------------------------------------------------------------------|
//! | I3  | Catalogue canonical hash stable across builds                                          |
//! | I4  | Signature gate fails closed when policy demands it but no signature is attached        |
//! | I5  | Gold binding gate fails closed when policy demands it but no gold_set_ref attached    |
//! | I6  | Cost translation is exact (banker's rounding, no drift)                               |
//! | I7  | Adapter blackboard writes are versioned                                                |
//! | I8  | Envelope is emitted on Errored outcomes (not only Completed)                          |
//! | I13 | SwarmToNano synthesises a fresh SwarmId per call (no leak between calls)              |
//! | I20 | default_catalogue() is byte-deterministic (canonical hash invariant)                   |

use std::sync::Arc;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, EventBus};
use efe_nano_sdk::{
    AgentId, Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, NanoAgent,
    NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier, SwarmId,
};
use efe_nano_swarm_adapter::{
    cents_to_microcents, microcents_to_cents, AdapterError, GoldBindingGate, NanoEnvelopeV1,
    NanoRunner, NanoRunnerOutcome, NanoToSwarm, NanoToSwarmPolicy, SignatureGate, SwarmToNano,
    ENVELOPE_TOPIC,
};
use efe_swarm_orchestrator::{
    AgentErrorKind, AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};

// ─────────────────────────────────────────────────────────────────────
//  Fixtures
// ─────────────────────────────────────────────────────────────────────

fn make_nano(id: &str) -> Arc<NanoAgent> {
    Arc::new(NanoAgent {
        id: NanoAgentId::new(id).unwrap(),
        version: NanoAgentVersion::new("1.0.0").unwrap(),
        description: "x".into(),
        author: Author::Efe,
        category: Category::Extraction,
        stability_tier: StabilityTier::Tier1,
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

fn make_ctx(text: &str, agent_id: &str) -> SwarmAgentContext {
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    bb.write(
        "/input/text",
        serde_json::json!(text),
        AgentId::new("seed").unwrap(),
    )
    .unwrap();
    SwarmAgentContext {
        swarm_id: bb.swarm_id(),
        agent_id: AgentId::new(agent_id).unwrap(),
        blackboard: bb,
        eventbus: Arc::new(EventBus::new()),
        input: Arc::new(serde_json::json!({"text": text})),
        rng_seed: 0,
    }
}

struct EchoRunner;
#[async_trait]
impl NanoRunner for EchoRunner {
    async fn run(
        &self,
        _: &NanoAgent,
        ctx: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        let text = ctx
            .blackboard
            .read("/input/text")
            .and_then(|v| v.value.as_str().map(String::from))
            .unwrap_or_default();
        ctx.blackboard
            .write(
                "/echo",
                serde_json::json!(text),
                ctx.agent_id.clone(),
            )
            .map_err(|e| AdapterError::BlackboardWriteFailed {
                pointer: "/echo".into(),
                message: e.to_string(),
            })?;
        Ok(NanoRunnerOutcome::fresh()
            .with_blackboard_writes(1)
            .with_cost_cents(0.0))
    }
}

// ─────────────────────────────────────────────────────────────────────
//  I3 + I20 — catalogue canonical hash stable across runs
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_n23_i20_canonical_hash_stable_across_calls() {
    let h1 = efe_nano_preanalysis::canonical_hash();
    let h2 = efe_nano_preanalysis::canonical_hash();
    let h3 = efe_nano_preanalysis::canonical_hash();
    assert_eq!(h1, h2);
    assert_eq!(h2, h3);
    assert_eq!(h1.len(), 64);
}

#[test]
fn audit_n23_i20_default_catalogue_size_six() {
    let c = efe_nano_preanalysis::default_catalogue();
    assert_eq!(c.len(), 6);
}

#[test]
fn audit_n23_i20_default_catalogue_order_deterministic() {
    let c1 = efe_nano_preanalysis::default_catalogue();
    let c2 = efe_nano_preanalysis::default_catalogue();
    let ids1: Vec<&str> = c1.iter().map(|(n, _)| n.id.as_str()).collect();
    let ids2: Vec<&str> = c2.iter().map(|(n, _)| n.id.as_str()).collect();
    assert_eq!(ids1, ids2);
}

// ─────────────────────────────────────────────────────────────────────
//  I4 — signature gate fails closed
// ─────────────────────────────────────────────────────────────────────

struct AlwaysOkSig;
#[async_trait]
impl SignatureGate for AlwaysOkSig {
    async fn verify(
        &self,
        _: &NanoAgent,
        _: &efe_nano_sdk::PublisherSignature,
    ) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn audit_n23_i4_signature_required_but_missing_signature_emits_errored() {
    let nano = make_nano("test_agent");
    let mut policy = NanoToSwarmPolicy::experimental_fast();
    policy.verify_signature = true;
    policy.signature_gate = Some(Arc::new(AlwaysOkSig));
    let adapter = NanoToSwarm::new(nano, Arc::new(EchoRunner)).with_policy(policy);
    let outcome = adapter.run(make_ctx("x", "test-agent")).await.unwrap();
    match outcome {
        AgentOutcome::Errored { kind, .. } => {
            assert_eq!(kind, AgentErrorKind::ComplianceFailed);
        }
        o => panic!("expected Errored, got {o:?}"),
    }
}

struct AlwaysRejectSig;
#[async_trait]
impl SignatureGate for AlwaysRejectSig {
    async fn verify(
        &self,
        _: &NanoAgent,
        _: &efe_nano_sdk::PublisherSignature,
    ) -> Result<(), String> {
        Err("forged".into())
    }
}

#[tokio::test]
async fn audit_n23_i4_signature_gate_rejects_forgery() {
    // Build a nano WITH a signature ; the gate rejects it.
    let mut na = make_nano("test_agent");
    Arc::get_mut(&mut na).unwrap().signature = Some(efe_nano_sdk::PublisherSignature {
        publisher_id: efe_nano_sdk::PublisherId::from_uuid(uuid::Uuid::nil()),
        key_id: efe_nano_sdk::KeyId::from_uuid(uuid::Uuid::nil()),
        algorithm: efe_nano_sdk::SignatureAlgorithm::Ed25519,
        signature: vec![0u8; 64],
        signed_at: std::time::SystemTime::now(),
        canonical_hash: efe_nano_sdk::Sha256Hash::from_bytes([0u8; 32]),
    });
    let mut policy = NanoToSwarmPolicy::experimental_fast();
    policy.verify_signature = true;
    policy.signature_gate = Some(Arc::new(AlwaysRejectSig));
    let adapter = NanoToSwarm::new(na, Arc::new(EchoRunner)).with_policy(policy);
    let outcome = adapter.run(make_ctx("x", "test-agent")).await.unwrap();
    match outcome {
        AgentOutcome::Errored { kind, message } => {
            assert_eq!(kind, AgentErrorKind::ComplianceFailed);
            assert!(message.to_lowercase().contains("forged"));
        }
        o => panic!("expected Errored, got {o:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  I5 — gold binding gate fails closed
// ─────────────────────────────────────────────────────────────────────

struct AlwaysOkGold;
#[async_trait]
impl GoldBindingGate for AlwaysOkGold {
    async fn assert_approved(&self, _: &NanoAgent) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn audit_n23_i5_gold_required_but_no_ref_emits_errored() {
    let nano = make_nano("test_agent");
    let mut policy = NanoToSwarmPolicy::experimental_fast();
    policy.require_gold_binding = true;
    policy.gold_gate = Some(Arc::new(AlwaysOkGold));
    let adapter = NanoToSwarm::new(nano, Arc::new(EchoRunner)).with_policy(policy);
    let outcome = adapter.run(make_ctx("x", "test-agent")).await.unwrap();
    match outcome {
        AgentOutcome::Errored { kind, .. } => {
            assert_eq!(kind, AgentErrorKind::ComplianceFailed);
        }
        o => panic!("expected Errored, got {o:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  I6 — exact cost translation
// ─────────────────────────────────────────────────────────────────────

#[test]
fn audit_n23_i6_cost_translation_exact_for_typical_values() {
    let cases = [
        (0.0_f64, 0_i64),
        (0.01, 100),
        (0.0001, 1),
        (1.0, 10_000),
        (-2.5, -25_000),
    ];
    for (cents, expected) in cases {
        let mc = cents_to_microcents(cents);
        assert_eq!(mc, expected, "{cents} cents → microcents");
        let back = microcents_to_cents(mc);
        assert!((back - cents).abs() < 1e-9, "{cents} → {mc} → {back}");
    }
}

#[test]
fn audit_n23_i6_cost_translation_nan_collapses_to_zero() {
    assert_eq!(cents_to_microcents(f64::NAN), 0);
    assert_eq!(cents_to_microcents(f64::INFINITY), 0);
    assert_eq!(cents_to_microcents(f64::NEG_INFINITY), 0);
}

// ─────────────────────────────────────────────────────────────────────
//  I7 — blackboard writes are versioned
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_n23_i7_adapter_blackboard_writes_are_versioned() {
    let nano = make_nano("echo");
    let adapter = NanoToSwarm::new(nano, Arc::new(EchoRunner));
    let ctx = make_ctx("hello", "echo");
    let bb = ctx.blackboard.clone();
    let _ = adapter.run(ctx).await.unwrap();
    let cost = bb.read("/nano/echo/cost").unwrap();
    let trace = bb.read("/nano/echo/trace").unwrap();
    // First-time writes must have version 1.
    assert_eq!(cost.version, 1);
    assert_eq!(trace.version, 1);
}

#[tokio::test]
async fn audit_n23_i7_repeat_dispatch_increments_version() {
    let nano = make_nano("echo");
    let adapter = NanoToSwarm::new(Arc::clone(&nano), Arc::new(EchoRunner));
    let ctx1 = make_ctx("hello", "echo");
    let bb = ctx1.blackboard.clone();
    let _ = adapter.run(ctx1).await.unwrap();

    // Second dispatch on the same blackboard.
    let adapter2 = NanoToSwarm::new(nano, Arc::new(EchoRunner));
    let ctx2 = SwarmAgentContext {
        swarm_id: bb.swarm_id(),
        agent_id: AgentId::new("echo").unwrap(),
        blackboard: bb.clone(),
        eventbus: Arc::new(EventBus::new()),
        input: Arc::new(serde_json::json!({"text": "world"})),
        rng_seed: 0,
    };
    let _ = adapter2.run(ctx2).await.unwrap();
    let cost = bb.read("/nano/echo/cost").unwrap();
    assert_eq!(cost.version, 2);
}

// ─────────────────────────────────────────────────────────────────────
//  I8 — envelope emitted on Errored outcomes
// ─────────────────────────────────────────────────────────────────────

struct AlwaysFailRunner;
#[async_trait]
impl NanoRunner for AlwaysFailRunner {
    async fn run(
        &self,
        _: &NanoAgent,
        _: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        Err(AdapterError::DispatchFailed("boom".into()))
    }
}

#[tokio::test]
async fn audit_n23_i8_envelope_emitted_on_errored_outcome() {
    let nano = make_nano("test_agent");
    let adapter = NanoToSwarm::new(nano, Arc::new(AlwaysFailRunner));
    let ctx = make_ctx("x", "test-agent");
    let mut sub = ctx.eventbus.subscribe();
    let outcome = adapter.run(ctx).await.unwrap();
    assert!(matches!(outcome, AgentOutcome::Errored { .. }));
    let ev = sub.try_recv().expect("envelope on bus even on Errored");
    assert_eq!(ev.topic, ENVELOPE_TOPIC);
    let env: NanoEnvelopeV1 = serde_json::from_value(ev.payload).unwrap();
    assert_eq!(
        env.outcome,
        efe_nano_swarm_adapter::NanoEnvelopeOutcome::Errored
    );
}

// ─────────────────────────────────────────────────────────────────────
//  I13 — SwarmToNano fresh SwarmId per call
// ─────────────────────────────────────────────────────────────────────

struct CapturingSwarmAgent {
    seen: Arc<std::sync::Mutex<Vec<SwarmId>>>,
}
#[async_trait]
impl SwarmAgent for CapturingSwarmAgent {
    fn agent_id(&self) -> AgentId {
        AgentId::new("capture").unwrap()
    }
    async fn run(
        &self,
        ctx: SwarmAgentContext,
    ) -> Result<AgentOutcome, SwarmAgentError> {
        self.seen.lock().unwrap().push(ctx.swarm_id);
        ctx.blackboard
            .write(
                "/output",
                serde_json::json!("ok"),
                ctx.agent_id.clone(),
            )
            .ok();
        Ok(AgentOutcome::Completed {
            cost_microcents: 0,
            duration_ms: 1,
            blackboard_writes: 1,
            events_published: 0,
        })
    }
}

#[tokio::test]
async fn audit_n23_i13_swarm_to_nano_fresh_swarm_id_per_call() {
    use efe_nano_sdk::traits::runtime::Runtime;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let agent = Arc::new(CapturingSwarmAgent {
        seen: seen.clone(),
    });
    let adapter = SwarmToNano::new(agent);
    let nano = (*make_nano("test_agent")).clone();
    for _ in 0..5 {
        let _ = adapter
            .execute(&nano, serde_json::json!({"text": "x"}))
            .await
            .unwrap();
    }
    let observed = seen.lock().unwrap().clone();
    assert_eq!(observed.len(), 5);
    // All distinct.
    let mut sorted = observed.iter().map(|s| s.as_uuid()).collect::<Vec<_>>();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 5, "SwarmIds should be distinct per call");
}

// ─────────────────────────────────────────────────────────────────────
//  Defense-in-depth — adapter does NOT panic on adversarial inputs
// ─────────────────────────────────────────────────────────────────────

struct PanickingRunner;
#[async_trait]
impl NanoRunner for PanickingRunner {
    async fn run(
        &self,
        _: &NanoAgent,
        _: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        // Return an error rather than panicking — adapters do NOT
        // swallow panics (the N22 CatchPanicLayer does that
        // upstream).  This test pins that pretending-to-be-fatal
        // errors surface as Errored, not panics.
        Err(AdapterError::DispatchFailed("simulated panic".into()))
    }
}

#[tokio::test]
async fn audit_n23_dispatch_returning_dispatch_failed_does_not_panic() {
    let nano = make_nano("test_agent");
    let adapter = NanoToSwarm::new(nano, Arc::new(PanickingRunner));
    let outcome = adapter
        .run(make_ctx("x", "test-agent"))
        .await
        .unwrap();
    match outcome {
        AgentOutcome::Errored { message, .. } => {
            assert!(message.contains("simulated panic"));
        }
        o => panic!("expected Errored, got {o:?}"),
    }
}
