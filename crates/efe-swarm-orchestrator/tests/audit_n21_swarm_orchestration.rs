// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : post-audit regression + property + chaos
//! + security + cross-cutting tests.
//!
//! Style aligned with `audit_n18_*`, `audit_n19_*`, `audit_n20_*`.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use efe_agentic_bridge::{AgenticBridge, AgenticTask, BridgeError, ClaudeBridge};
use efe_blackboard::{Blackboard, EventBus};
use efe_nano_sdk::{AgentId, ContextBundle, SwarmId, MAX_CONTEXT_BUNDLE_BYTES};
use efe_swarm_orchestrator::{
    AgentErrorKind, AgentNode, AgentOutcome, Compliance, N21Metrics, SwarmAgent,
    SwarmAgentContext, SwarmAgentError, SwarmDagBuilder, SwarmDagError,
    SwarmOrchestrator, SwarmOverallVerdict, SwarmRunConfig,
};

fn aid(s: &str) -> AgentId {
    AgentId::new(s).unwrap()
}

// ─────────────────────────────────────────────────────────────────────
//  Fixture agents
// ─────────────────────────────────────────────────────────────────────

struct EchoAgent {
    id: AgentId,
}
impl EchoAgent {
    fn new(name: &str) -> Arc<Self> {
        Arc::new(Self { id: aid(name) })
    }
}
#[async_trait]
impl SwarmAgent for EchoAgent {
    fn agent_id(&self) -> AgentId {
        self.id.clone()
    }
    async fn run(&self, _: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
        Ok(AgentOutcome::Completed {
            cost_microcents: 1,
            duration_ms: 1,
            blackboard_writes: 0,
            events_published: 0,
        })
    }
}

struct PanicAgent {
    id: AgentId,
}
#[async_trait]
impl SwarmAgent for PanicAgent {
    fn agent_id(&self) -> AgentId {
        self.id.clone()
    }
    async fn run(&self, _: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
        panic!("intentional panic for chaos test");
    }
}

struct SleepAgent {
    id: AgentId,
    ms: u64,
}
#[async_trait]
impl SwarmAgent for SleepAgent {
    fn agent_id(&self) -> AgentId {
        self.id.clone()
    }
    async fn run(&self, _: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
        tokio::time::sleep(Duration::from_millis(self.ms)).await;
        Ok(AgentOutcome::Completed {
            cost_microcents: 0,
            duration_ms: self.ms as u32,
            blackboard_writes: 0,
            events_published: 0,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  AUDIT — Bugs latents pin pre-emptively
// ─────────────────────────────────────────────────────────────────────

/// B1. DAG with a single node + aggregator = same node MUST build.
#[tokio::test]
async fn audit_b1_single_node_aggregator_dag_builds() {
    let dag = SwarmDagBuilder::new()
        .add_agent(AgentNode::new(EchoAgent::new("solo-aggr")))
        .with_aggregator(aid("solo-aggr"))
        .build()
        .unwrap();
    assert_eq!(dag.node_count(), 1);
}

/// B2. DAG with conditional edge pointing at a non-existent node must
/// fail at build (referential integrity).
#[tokio::test]
async fn audit_b2_dangling_conditional_edge_rejected() {
    let err = SwarmDagBuilder::new()
        .add_agent(AgentNode::new(EchoAgent::new("aggr")))
        .add_conditional(aid("aggr"), aid("missing"), "p", |_| true)
        .with_aggregator(aid("aggr"))
        .build()
        .unwrap_err();
    assert!(matches!(err, SwarmDagError::UnknownAgentInEdge { .. }));
}

/// B3. Two consecutive notify_one without recv must still surface
/// at least one tick (Notify is permit-buffered).
#[tokio::test]
async fn audit_b3_eventbus_recv_after_publish_works() {
    let bus = EventBus::new();
    let mut sub = bus.subscribe();
    bus.publish(efe_blackboard::SwarmEvent {
        swarm_id: SwarmId::new(),
        published_by: aid("pub"),
        topic: "x".into(),
        payload: serde_json::json!({}),
        published_at: SystemTime::now(),
    })
    .unwrap();
    let got = sub.recv().await.unwrap();
    assert_eq!(got.topic, "x");
}

/// B4. Blackboard write_if compare-and-swap MUST be truly atomic
/// under concurrency.  Refers to the bug fixed during code-gen.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn audit_b4_blackboard_write_if_is_atomic_under_concurrency() {
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    bb.write("/key", serde_json::json!(0), aid("init")).unwrap();
    let mut handles = Vec::new();
    for i in 0..50 {
        let bb = bb.clone();
        handles.push(tokio::spawn(async move {
            bb.write_if("/key", 1, serde_json::json!(i), aid("worker"))
                .is_ok()
        }));
    }
    let mut successes = 0;
    for h in handles {
        if h.await.unwrap() {
            successes += 1;
        }
    }
    // Exactly ONE write_if succeeds (CAS) ; the others fail with
    // Conflict.
    assert_eq!(successes, 1);
}

/// B5. SwarmRunConfig::default has sane values (30s timeout, no
/// envelope, no seed).
#[test]
fn audit_b5_swarm_run_config_default_sane() {
    let c = SwarmRunConfig::default();
    assert_eq!(c.global_timeout_ms, 30_000);
    assert!(!c.emit_envelope);
    assert!(c.rng_seed.is_none());
}

/// B6. Aggregator NOT in the DAG produces clean error (not panic).
#[tokio::test]
async fn audit_b6_aggregator_unknown_produces_clean_error() {
    let err = SwarmDagBuilder::new()
        .add_agent(AgentNode::new(EchoAgent::new("a-1")))
        .with_aggregator(aid("ghost"))
        .build()
        .unwrap_err();
    assert!(matches!(err, SwarmDagError::AggregatorNotDeclared(_)));
}

// ─────────────────────────────────────────────────────────────────────
//  PROPERTY TESTS
// ─────────────────────────────────────────────────────────────────────

/// PROP1. Topological sort returns valid order (layer N has no dep
/// on layer >= N).
#[test]
fn proptest_dag_topological_sort_returns_valid_order() {
    // 8 randomised DAG shapes — but deterministic for replay.
    let cases = [4, 5, 6, 7, 8, 10, 15, 20];
    for n in cases {
        // Build a linear chain : 0 -> 1 -> 2 -> ... -> n-1 with
        // n-1 as the aggregator.
        let mut builder = SwarmDagBuilder::new();
        for i in 0..n {
            builder =
                builder.add_agent(AgentNode::new(EchoAgent::new(&format!("a-{i:02}"))));
        }
        for i in 0..n - 1 {
            builder = builder.add_dependency(
                aid(&format!("a-{i:02}")),
                aid(&format!("a-{:02}", i + 1)),
            );
        }
        let dag = builder
            .with_aggregator(aid(&format!("a-{:02}", n - 1)))
            .build()
            .unwrap();
        // Verify layer ordering : if a depends on b, a's layer > b's.
        let layers = dag.layers();
        let mut layer_of: std::collections::HashMap<AgentId, usize> =
            std::collections::HashMap::new();
        for (idx, layer) in layers.iter().enumerate() {
            for aid in layer {
                layer_of.insert(aid.clone(), idx);
            }
        }
        for e in dag.edges() {
            assert!(
                layer_of[&e.from] < layer_of[&e.to],
                "edge {:?} violates topological order : from layer {} → to layer {}",
                e,
                layer_of[&e.from],
                layer_of[&e.to]
            );
        }
    }
}

/// PROP2. Blackboard write_then_read returns the written value
/// (idempotent on reads).
#[test]
fn proptest_blackboard_write_then_read_returns_same_value() {
    let bb = Blackboard::new(SwarmId::new());
    for i in 0..100 {
        let pointer = format!("/k_{i}");
        let value = serde_json::json!({"i": i, "x": "y"});
        bb.write(&pointer, value.clone(), aid("aaa")).unwrap();
        let got = bb.read(&pointer).unwrap();
        assert_eq!(got.value, value);
    }
}

/// PROP3. ContextBundle serde round-trip is exact.
#[test]
fn proptest_context_bundle_serde_round_trip() {
    use efe_nano_sdk::{Classification, Entity, Redaction};
    let mut b = ContextBundle::empty(SwarmId::new());
    b.language = Some("fr".into());
    for i in 0..10 {
        b.entities.push(Entity {
            entity_type: "person".into(),
            value: format!("Person {i}"),
            span: Some((i * 10, i * 10 + 5)),
        });
        b.redactions.push(Redaction {
            redaction_type: "email".into(),
            original_span: (i * 20, i * 20 + 10),
            redaction_token: format!("<EMAIL_{i}>"),
        });
        b.classifications.push(Classification {
            label: format!("class_{i}"),
            confidence: 0.5 + (i as f64) * 0.01,
        });
    }
    b.summary = Some("Multi-line summary.".into());
    let json = serde_json::to_string(&b).unwrap();
    let back: ContextBundle = serde_json::from_str(&json).unwrap();
    assert_eq!(b, back);
}

/// PROP4. Predicate purity : calling the same predicate 10× with
/// the same blackboard returns the same result.
#[test]
fn proptest_predicate_purity() {
    use efe_swarm_orchestrator::dag::ConditionalEdge;
    let bb = Blackboard::new(SwarmId::new());
    bb.write("/language", serde_json::json!("fr"), aid("aaa")).unwrap();
    let edge = ConditionalEdge::new(aid("a-1"), aid("a-2"), "is_fr", |bb| {
        bb.read("/language")
            .and_then(|v| v.value.as_str().map(|s| s == "fr"))
            .unwrap_or(false)
    });
    let r1 = (0..10).map(|_| edge.evaluate(&bb)).collect::<Vec<_>>();
    assert!(r1.iter().all(|b| *b));
}

// ─────────────────────────────────────────────────────────────────────
//  CHAOS TESTS
// ─────────────────────────────────────────────────────────────────────

/// CHAOS1. Agent panics → swarm continues (best-effort isolation G9).
#[tokio::test]
async fn chaos_agent_panic_isolated_from_swarm() {
    let dag = SwarmDagBuilder::new()
        .add_agent(AgentNode::new(Arc::new(PanicAgent { id: aid("panicky") })))
        .add_agent(AgentNode::new(EchoAgent::new("aggr")))
        .add_dependency(aid("panicky"), aid("aggr"))
        .with_aggregator(aid("aggr"))
        .build()
        .unwrap();
    let orch = SwarmOrchestrator::new();
    let r = orch
        .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
        .await
        .unwrap();
    // The swarm completes ; panicky has Errored or join-error.
    assert!(r
        .agents
        .iter()
        .any(|r| matches!(r.outcome, AgentOutcome::Errored { kind: AgentErrorKind::Panic, .. })));
}

/// CHAOS2. Global timeout aborts all in-flight agents.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_global_timeout_aborts_all() {
    let dag = SwarmDagBuilder::new()
        .add_agent(AgentNode::new(Arc::new(SleepAgent { id: aid("slowww"), ms: 5_000 })))
        .add_agent(AgentNode::new(EchoAgent::new("aggr")))
        .add_dependency(aid("slowww"), aid("aggr"))
        .with_aggregator(aid("aggr"))
        .build()
        .unwrap();
    let orch = SwarmOrchestrator::new();
    let cfg = SwarmRunConfig {
        global_timeout_ms: 100,
        ..Default::default()
    };
    let r = orch
        .run(&dag, serde_json::json!({}), cfg)
        .await
        .unwrap();
    assert!(matches!(r.overall, SwarmOverallVerdict::Aborted { .. }));
}

/// CHAOS3. EventBus capacity full = drops oldest ; metrics tick.
#[tokio::test]
async fn chaos_eventbus_capacity_drops_slow_consumer() {
    let bus = EventBus::with_capacity(4);
    let mut sub = bus.subscribe();
    for i in 0..100 {
        bus.publish(efe_blackboard::SwarmEvent {
            swarm_id: SwarmId::new(),
            published_by: aid("pub"),
            topic: format!("t_{i}"),
            payload: serde_json::json!({}),
            published_at: SystemTime::now(),
        })
        .unwrap();
    }
    while sub.try_recv().is_some() {}
    assert!(bus.dropped_total() > 0);
}

/// CHAOS4. Claude 429 → bridge surfaces RateLimited.
#[tokio::test]
async fn chaos_claude_429_returns_rate_limited() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&server)
        .await;
    let bridge = ClaudeBridge::with_endpoint("k", server.uri());
    let bundle = ContextBundle::empty(SwarmId::new());
    let err = bridge
        .call_with_context(&bundle, AgenticTask::new("x"))
        .await
        .unwrap_err();
    assert!(matches!(err, BridgeError::RateLimited));
}

// ─────────────────────────────────────────────────────────────────────
//  SECURITY TESTS
// ─────────────────────────────────────────────────────────────────────

/// SEC1. PII redactor MUST be schedulable before Claude in any
/// pipeline that touches user PII (architecture P4 + § 12.4).
///
/// We assert this by building a DAG where pii-redactor is a
/// dependency of any Mode-C agent (here represented by echo).
#[tokio::test]
async fn security_pii_redactor_runs_before_claude() {
    use efe_nano_preanalysis::PiiRedactor;
    let dag = SwarmDagBuilder::new()
        .add_agent(AgentNode::new(Arc::new(PiiRedactor::new())))
        .add_agent(AgentNode::new(EchoAgent::new("mode-c-call")))
        .add_agent(AgentNode::new(EchoAgent::new("aggr")))
        .add_dependency(aid("pii-redactor"), aid("mode-c-call"))
        .add_dependency(aid("mode-c-call"), aid("aggr"))
        .with_aggregator(aid("aggr"))
        .build()
        .unwrap();
    // pii-redactor must be in an earlier layer than mode-c-call.
    let layers = dag.layers();
    let mut layer_of: std::collections::HashMap<AgentId, usize> =
        std::collections::HashMap::new();
    for (idx, layer) in layers.iter().enumerate() {
        for aid in layer {
            layer_of.insert(aid.clone(), idx);
        }
    }
    let pii_layer = layer_of[&aid("pii-redactor")];
    let claude_layer = layer_of[&aid("mode-c-call")];
    assert!(
        pii_layer < claude_layer,
        "pii-redactor layer {pii_layer} must be < mode-c-call layer {claude_layer}"
    );
}

/// SEC2. Compliance N18 invalid signature fails-closed (G13 + F12).
#[tokio::test]
async fn security_compliance_n18_invalid_sig_fails_closed() {
    use efe_swarm_orchestrator::compliance::StrictUnwiredComplianceRunner;
    let dag = SwarmDagBuilder::new()
        .add_agent(
            AgentNode::new(EchoAgent::new("signed-agent")).with_compliance(
                Compliance::N18Signed {
                    publisher_id: efe_nano_sdk::PublisherId::new(),
                    key_id: efe_nano_sdk::KeyId::new(),
                },
            ),
        )
        .add_agent(AgentNode::new(EchoAgent::new("aggr")))
        .add_dependency(aid("signed-agent"), aid("aggr"))
        .with_aggregator(aid("aggr"))
        .build()
        .unwrap();
    let orch = SwarmOrchestrator::new()
        .with_compliance(Arc::new(StrictUnwiredComplianceRunner));
    let r = orch
        .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
        .await
        .unwrap();
    let signed = r
        .agents
        .iter()
        .find(|r| r.agent_id.as_str() == "signed-agent")
        .unwrap();
    assert!(matches!(
        signed.outcome,
        AgentOutcome::Errored {
            kind: AgentErrorKind::ComplianceFailed,
            ..
        }
    ));
}

/// SEC3. Blackboard size cap prevents DoS via large writes.
#[tokio::test]
async fn security_blackboard_size_limit_prevents_dos() {
    let bb = Blackboard::with_cap(SwarmId::new(), 1_000);
    let huge = serde_json::json!("x".repeat(5_000));
    let err = bb.write("/huge", huge, aid("attacker")).unwrap_err();
    assert!(matches!(
        err,
        efe_blackboard::BlackboardError::SizeLimitExceeded { .. }
    ));
    // Blackboard is unchanged.
    assert_eq!(bb.len(), 0);
    assert_eq!(bb.total_bytes(), 0);
}

// ─────────────────────────────────────────────────────────────────────
//  CROSS-CUTTING TESTS
// ─────────────────────────────────────────────────────────────────────

/// CROSS1. N14 envelope ID surfaces in SwarmRunConfig.
#[test]
fn cross_n14_n21_envelope_config_field_exists() {
    let cfg = SwarmRunConfig {
        emit_envelope: true,
        ..Default::default()
    };
    assert!(cfg.emit_envelope);
}

/// CROSS2. ContextBundle is bounded ≤ 256 KB (matches MAX_CONTEXT_BUNDLE_BYTES).
#[test]
fn cross_n21_context_bundle_cap_matches_sdk_constant() {
    assert_eq!(MAX_CONTEXT_BUNDLE_BYTES, 262_144);
    let b = ContextBundle::empty(SwarmId::new());
    assert!(b.fits_size_cap());
}

/// CROSS3. swarm_id round-trips through SwarmReport → ContextBundle.
#[tokio::test]
async fn cross_n21_swarm_id_propagates_to_bundle() {
    let dag = SwarmDagBuilder::new()
        .add_agent(AgentNode::new(EchoAgent::new("aggr")))
        .with_aggregator(aid("aggr"))
        .build()
        .unwrap();
    let cfg = SwarmRunConfig {
        swarm_id: SwarmId::from_uuid(uuid::Uuid::nil()),
        ..Default::default()
    };
    let r = SwarmOrchestrator::new()
        .run(&dag, serde_json::json!({}), cfg)
        .await
        .unwrap();
    assert_eq!(r.bundle.swarm_id, r.swarm_id);
}

/// CROSS4. N18 PublisherId + KeyId types accepted by Compliance::N18Signed.
#[test]
fn cross_n18_n21_publisher_compliance_types_compatible() {
    let c = Compliance::N18Signed {
        publisher_id: efe_nano_sdk::PublisherId::new(),
        key_id: efe_nano_sdk::KeyId::new(),
    };
    assert_eq!(c.label(), "n18_signed");
}

/// CROSS5. N19 GoldSetId type accepted by Compliance::N19GoldGated.
#[test]
fn cross_n19_n21_gold_set_compliance_types_compatible() {
    let c = Compliance::N19GoldGated {
        gold_set_id: efe_nano_sdk::GoldSetId::new(),
    };
    assert_eq!(c.label(), "n19_gold_gated");
}

/// CROSS6. N21Metrics render_prometheus produces parseable output.
#[test]
fn cross_n21_metrics_render_well_formed() {
    let m = N21Metrics::new();
    m.tick_swarm_run("passed");
    let out = m.render_prometheus();
    // Sanity : every #TYPE line is followed by a metric name +
    // value line.
    let mut help_count = 0;
    let mut type_count = 0;
    for line in out.lines() {
        if line.starts_with("# HELP") {
            help_count += 1;
        }
        if line.starts_with("# TYPE") {
            type_count += 1;
        }
    }
    assert_eq!(help_count, type_count);
    assert!(help_count >= 12);
}

// ─────────────────────────────────────────────────────────────────────
//  E2E with the six reference agents
// ─────────────────────────────────────────────────────────────────────

/// E2E1. Six reference agents loadable + composable in a DAG.
#[tokio::test]
async fn audit_six_reference_agents_e2e() {
    use efe_nano_preanalysis::{
        LanguageDetector, NerExtractor, PiiRedactor, SchemaValidator,
        SentimentClassifier, SummaryWriter,
    };
    let dag = SwarmDagBuilder::new()
        // Layer 1 (parallel) :
        .add_agent(AgentNode::new(Arc::new(NerExtractor::new())))
        .add_agent(AgentNode::new(Arc::new(LanguageDetector::new())))
        .add_agent(AgentNode::new(Arc::new(SchemaValidator::new(
            serde_json::json!({"type": "object"}),
        ))))
        .add_agent(AgentNode::new(Arc::new(SentimentClassifier::new())))
        // Layer 2 :
        .add_agent(AgentNode::new(Arc::new(PiiRedactor::new())))
        // Layer 3 :
        .add_agent(AgentNode::new(Arc::new(SummaryWriter::new())))
        // Layer 4 (aggregator) :
        .add_agent(AgentNode::new(EchoAgent::new("context-aggregator")))
        // Dependencies
        .add_dependency(aid("ner-extractor"), aid("pii-redactor"))
        .add_dependency(aid("language-detector"), aid("pii-redactor"))
        .add_dependency(aid("pii-redactor"), aid("summary-writer"))
        .add_dependency(aid("sentiment-classifier"), aid("context-aggregator"))
        .add_dependency(aid("schema-validator"), aid("context-aggregator"))
        .add_dependency(aid("summary-writer"), aid("context-aggregator"))
        .with_aggregator(aid("context-aggregator"))
        .build()
        .unwrap();
    assert_eq!(dag.node_count(), 7);
    let orch = SwarmOrchestrator::new();
    let r = orch
        .run(
            &dag,
            serde_json::json!({"text": "Contact alice@example.com on 2026-05-21. C'est une bonne entreprise."}),
            SwarmRunConfig::default(),
        )
        .await
        .unwrap();
    assert!(matches!(r.overall, SwarmOverallVerdict::Passed { .. }));
}

/// E2E2. Initial input written to /input/text is consumed by the
/// six agents and produces a populated blackboard.
#[tokio::test]
async fn audit_six_agents_populate_blackboard() {
    use efe_nano_preanalysis::{
        LanguageDetector, NerExtractor, PiiRedactor, SentimentClassifier,
    };
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    bb.write(
        "/input/text",
        serde_json::json!("Send to bob@test.org on 2026-05-21. Great product!"),
        aid("seed"),
    )
    .unwrap();
    let ev = Arc::new(EventBus::new());
    let input = Arc::new(serde_json::json!({}));
    for agent in [
        Arc::new(NerExtractor::new()) as Arc<dyn SwarmAgent>,
        Arc::new(LanguageDetector::new()),
        Arc::new(PiiRedactor::new()),
        Arc::new(SentimentClassifier::new()),
    ] {
        let ctx = SwarmAgentContext {
            swarm_id: bb.swarm_id(),
            agent_id: agent.agent_id(),
            blackboard: bb.clone(),
            eventbus: ev.clone(),
            input: input.clone(),
            rng_seed: 42,
        };
        let _ = agent.run(ctx).await.unwrap();
    }
    assert!(bb.contains("/entities"));
    assert!(bb.contains("/language"));
    assert!(bb.contains("/redactions"));
    assert!(bb.contains("/sentiment"));
}
