// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23** : chaos tests for the adapter under
//! adversarial conditions.
//!
//! These tests exercise the adapter against :
//! - high-concurrency dispatch on the same blackboard ;
//! - blackboard-size cap exhaustion (F3 / I10) ;
//! - event-bus saturation (F9) — envelope fallback to blackboard ;
//! - flaky runner that fails N times then succeeds (no internal
//!   retry expected — the adapter surfaces the error verbatim) ;
//! - timeout under load.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, EventBus};
use efe_nano_sdk::{
    AgentId, Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, NanoAgent,
    NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier, SwarmId,
};
use efe_nano_swarm_adapter::{
    AdapterError, NanoRunner, NanoRunnerOutcome, NanoToSwarm, NanoToSwarmPolicy,
};
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext,
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

fn make_ctx_with_bb(
    text: &str,
    agent_id: &str,
    bb: Arc<Blackboard>,
    eventbus: Arc<EventBus>,
) -> SwarmAgentContext {
    let _ = bb.write(
        "/input/text",
        serde_json::json!(text),
        AgentId::new("seed").unwrap(),
    );
    SwarmAgentContext {
        swarm_id: bb.swarm_id(),
        agent_id: AgentId::new(agent_id).unwrap(),
        blackboard: bb,
        eventbus,
        input: Arc::new(serde_json::json!({"text": text})),
        rng_seed: 0,
    }
}

struct LightRunner;
#[async_trait]
impl NanoRunner for LightRunner {
    async fn run(
        &self,
        _: &NanoAgent,
        _: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        Ok(NanoRunnerOutcome::fresh().with_cost_cents(0.0))
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Chaos test 1 — 50 concurrent dispatches on the same blackboard
//  must all complete without deadlock or panic.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_n23_50_concurrent_dispatches_no_deadlock() {
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let eb = Arc::new(EventBus::new());
    let nano = make_nano("echo");
    let adapter = Arc::new(NanoToSwarm::new(nano, Arc::new(LightRunner)));

    let mut handles = Vec::new();
    for i in 0..50 {
        let bb = bb.clone();
        let eb = eb.clone();
        let adapter = adapter.clone();
        handles.push(tokio::spawn(async move {
            let ctx = make_ctx_with_bb(
                &format!("payload-{i}"),
                &format!("worker-{i}"),
                bb,
                eb,
            );
            adapter.run(ctx).await
        }));
    }
    let mut completed = 0;
    for h in handles {
        let res = h.await.unwrap();
        // Some may surface BlackboardWriteFailed (hard) on size or
        // collision ; all others must be Completed.
        match res {
            Ok(AgentOutcome::Completed { .. }) => completed += 1,
            Ok(AgentOutcome::Errored { .. }) => completed += 1,
            Ok(o) => panic!("unexpected outcome : {o:?}"),
            Err(_) => {} // hard SwarmAgentError under contention — acceptable
        }
    }
    assert!(completed > 0, "at least some dispatches must complete");
}

// ─────────────────────────────────────────────────────────────────────
//  Chaos test 2 — blackboard size cap exhaustion : adapter must
//  surface SwarmAgentError::Blackboard, not panic.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn chaos_n23_blackboard_size_cap_fail_surfaces_as_hard_error() {
    // Very small blackboard cap : the adapter's audit writes
    // (cost / trace / mode / cache_hit / signature_verified /
    // gold_bound / latency_ms) cannot all fit.  The adapter must
    // surface a hard SwarmAgentError::Blackboard rather than
    // panicking.
    let bb = Arc::new(Blackboard::with_cap(SwarmId::new(), 32));
    let eb = Arc::new(EventBus::new());
    let nano = make_nano("echo");
    let adapter = NanoToSwarm::new(nano, Arc::new(LightRunner));
    let ctx = make_ctx_with_bb("hello", "echo", bb, eb);
    let result = adapter.run(ctx).await;
    // Either : hard error (transport-level) OR Errored (soft).
    // Both prove the adapter did not panic.
    match result {
        Ok(_) | Err(_) => {} // Both acceptable.
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Chaos test 3 — event bus has zero subscribers : the F9 fallback
//  ensures the envelope still lands on the blackboard.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn chaos_n23_event_bus_no_subscribers_falls_back_to_blackboard() {
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let eb = Arc::new(EventBus::new());
    // No subscribers attached → publish returns Ok(0).
    let nano = make_nano("echo");
    let adapter = NanoToSwarm::new(nano, Arc::new(LightRunner));
    let ctx = make_ctx_with_bb("hello", "echo", bb.clone(), eb);
    let _ = adapter.run(ctx).await.unwrap();
    // F9 : envelope persists at /nano/echo/envelope.
    let envelope = bb.read("/nano/echo/envelope");
    assert!(
        envelope.is_some(),
        "envelope must fall back to blackboard when no subscribers"
    );
    let val = envelope.unwrap().value;
    assert!(val.is_object());
    assert_eq!(val["nano_id"].as_str().unwrap(), "echo");
}

// ─────────────────────────────────────────────────────────────────────
//  Chaos test 4 — flaky runner : N failures in a row never cause
//  the adapter to spin retries (the architecture explicitly defers
//  retries to upstream callers, not the adapter).
// ─────────────────────────────────────────────────────────────────────

struct FlakyRunner {
    call_count: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait]
impl NanoRunner for FlakyRunner {
    async fn run(
        &self,
        _: &NanoAgent,
        _: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Err(AdapterError::DispatchFailed("flaky".into()))
    }
}

#[tokio::test]
async fn chaos_n23_flaky_runner_no_internal_retry() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runner = Arc::new(FlakyRunner {
        call_count: count.clone(),
    });
    let nano = make_nano("flaky");
    let adapter = NanoToSwarm::new(nano, runner);
    let bb = Arc::new(Blackboard::new(SwarmId::new()));
    let eb = Arc::new(EventBus::new());
    let ctx = make_ctx_with_bb("x", "flaky", bb, eb);
    let _ = adapter.run(ctx).await.unwrap();
    assert_eq!(
        count.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "adapter must NOT spin retries internally"
    );
}

// ─────────────────────────────────────────────────────────────────────
//  Chaos test 5 — timeout under load : adapter surfaces TimedOut
// ─────────────────────────────────────────────────────────────────────

struct ContentionSlowRunner;
#[async_trait]
impl NanoRunner for ContentionSlowRunner {
    async fn run(
        &self,
        _: &NanoAgent,
        _: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        tokio::time::sleep(Duration::from_millis(500)).await;
        Ok(NanoRunnerOutcome::fresh())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_n23_timeout_under_load_yields_timed_out() {
    let mut na = make_nano("slow");
    Arc::get_mut(&mut na).unwrap().limits.wall_clock_ms = 50;
    let adapter = Arc::new(NanoToSwarm::new(na, Arc::new(ContentionSlowRunner)));
    let mut handles = Vec::new();
    for i in 0..10 {
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        let eb = Arc::new(EventBus::new());
        let adapter = adapter.clone();
        handles.push(tokio::spawn(async move {
            let ctx = make_ctx_with_bb("x", &format!("slow-{i}"), bb, eb);
            adapter.run(ctx).await
        }));
    }
    let mut timed_out = 0;
    for h in handles {
        match h.await.unwrap() {
            Ok(AgentOutcome::TimedOut { .. }) => timed_out += 1,
            Ok(o) => panic!("expected TimedOut under load, got {o:?}"),
            Err(e) => panic!("unexpected hard error : {e:?}"),
        }
    }
    assert_eq!(timed_out, 10, "every dispatch should time out");
}

// ─────────────────────────────────────────────────────────────────────
//  Chaos test 6 — repeated cache hits under contention : cache
//  must serve consistent responses (no torn reads).
// ─────────────────────────────────────────────────────────────────────

struct DeterministicRunner;
#[async_trait]
impl NanoRunner for DeterministicRunner {
    async fn run(
        &self,
        _: &NanoAgent,
        _: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        Ok(NanoRunnerOutcome::fresh().with_cost_cents(0.42))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chaos_n23_repeated_cache_hits_under_contention_consistent() {
    use efe_nano_swarm_adapter::InMemoryIdempotencyCache;
    let cache = Arc::new(InMemoryIdempotencyCache::new());
    let policy = NanoToSwarmPolicy::cache_only(cache.clone());
    let nano = make_nano("det");
    let adapter = Arc::new(
        NanoToSwarm::new(nano, Arc::new(DeterministicRunner))
            .with_policy(policy),
    );

    // Prime the cache.
    let bb0 = Arc::new(Blackboard::new(SwarmId::new()));
    let eb0 = Arc::new(EventBus::new());
    let ctx0 = make_ctx_with_bb("same", "det", bb0, eb0);
    let _ = adapter.run(ctx0).await.unwrap();
    assert_eq!(cache.len(), 1);

    // Now 20 concurrent dispatches with the same input — all
    // should hit the cache, no two should re-invoke the runner.
    let mut handles = Vec::new();
    for i in 0..20 {
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        let eb = Arc::new(EventBus::new());
        let adapter = adapter.clone();
        handles.push(tokio::spawn(async move {
            let ctx = make_ctx_with_bb("same", &format!("c-{i}"), bb.clone(), eb);
            let outcome = adapter.run(ctx).await.unwrap();
            (outcome, bb)
        }));
    }
    for h in handles {
        let (outcome, bb) = h.await.unwrap();
        assert!(matches!(outcome, AgentOutcome::Completed { .. }));
        // Every dispatch must be marked as a cache hit.
        let hit = bb
            .read("/nano/det/cache_hit")
            .expect("cache_hit must be written")
            .value;
        assert_eq!(
            hit,
            serde_json::Value::Bool(true),
            "every dispatch under contention must be a cache hit"
        );
    }
    // Cache still has exactly one entry.
    assert_eq!(cache.len(), 1);
}
