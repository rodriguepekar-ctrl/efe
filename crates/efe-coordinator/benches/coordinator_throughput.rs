// SPDX-License-Identifier: UNLICENSED
//! Benchmarks Criterion : throughput Coordinator (blackboard, vote, DAG, mission).
//!
//! Usage :
//!
//! ```bash
//! PYO3_PYTHON=$(which python3.12) \
//!   cargo bench -p efe-coordinator --bench coordinator_throughput
//! ```
//!
//! Résultats HTML : `target/criterion/`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use efe_coordinator::{
    run_mission_a, run_mission_b, Ballot, BlackboardEntry, Coordinator, DagExecutor, DagNode,
    EntryKind, LlmCall, MissionARequest, MissionBRequest, MockLlmProvider, MultiLlmBridge,
    PheromoneBlackboard, ProviderRegistry, VoteEngine,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

fn rt() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}

fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 1000,
        cost_output_per_million: 3000,
        // R3.S1 : 0 → _resolved() fallback à cents × 1000.
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

// ─── Blackboard ─────────────────────────────────────────────────────

fn bench_blackboard_deposit(c: &mut Criterion) {
    let mut group = c.benchmark_group("blackboard");
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(1));
    group.bench_function("deposit_small_entry", |b| {
        let bb = PheromoneBlackboard::new();
        b.iter(|| {
            let _ = bb.deposit(BlackboardEntry::new(
                "x",
                EntryKind::Trace,
                serde_json::json!({"k": "v"}),
            ));
            if bb.bytes_used() > 200_000 {
                bb.clear();
            }
        });
    });
    group.finish();
}

fn bench_blackboard_read(c: &mut Criterion) {
    let bb = PheromoneBlackboard::new();
    for i in 0..100 {
        let _ = bb.deposit(BlackboardEntry::new(
            "x",
            EntryKind::Entity,
            serde_json::json!({"i": i}),
        ));
    }
    let mut group = c.benchmark_group("blackboard");
    group.throughput(Throughput::Elements(1));
    group.bench_function("read_kind_100_entries", |b| {
        b.iter(|| {
            let _ = bb.read_kind(EntryKind::Entity);
        });
    });
    group.finish();
}

// ─── Vote ───────────────────────────────────────────────────────────

fn bench_vote_3_llms(c: &mut Criterion) {
    let v = VoteEngine::uniform();
    let ballots = vec![
        Ballot {
            provider: "claude".into(),
            answer_text: "Paris".into(),
            self_quality: 0.9,
            latency_ms: 500,
        },
        Ballot {
            provider: "gpt4".into(),
            answer_text: "Paris".into(),
            self_quality: 0.95,
            latency_ms: 700,
        },
        Ballot {
            provider: "gemini".into(),
            answer_text: "Lyon".into(),
            self_quality: 0.8,
            latency_ms: 600,
        },
    ];
    let mut group = c.benchmark_group("vote");
    group.throughput(Throughput::Elements(1));
    group.bench_function(BenchmarkId::new("tally", "3_llms"), |b| {
        b.iter(|| {
            let _ = v.tally(&ballots, 2);
        });
    });
    group.finish();
}

// ─── DAG ────────────────────────────────────────────────────────────

fn bench_dag_diamond(c: &mut Criterion) {
    let runtime = rt();
    let exec = DagExecutor::default();
    let mut group = c.benchmark_group("dag");
    group.throughput(Throughput::Elements(4));
    group.bench_function("diamond_a_to_d", |b| {
        b.to_async(&runtime).iter(|| {
            let exec = exec.clone();
            async move {
                let a = DagNode::root("a", || async { Ok(serde_json::json!(0)) });
                let bn = DagNode::with_parents("b", ["a"], || async { Ok(serde_json::json!(0)) });
                let cn = DagNode::with_parents("c", ["a"], || async { Ok(serde_json::json!(0)) });
                let d = DagNode::with_parents("d", ["b", "c"], || async {
                    Ok(serde_json::json!(0))
                });
                let _ = exec.run(vec![a, bn, cn, d]).await;
            }
        });
    });
    group.finish();
}

// ─── LLM bridge ─────────────────────────────────────────────────────

fn bench_bridge_call_one(c: &mut Criterion) {
    let runtime = rt();
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("ok")));
    let bridge = Arc::new(MultiLlmBridge::new(reg));
    let mut group = c.benchmark_group("bridge");
    group.throughput(Throughput::Elements(1));
    group.bench_function("call_one_mock", |b| {
        b.to_async(&runtime).iter(|| {
            let br = bridge.clone();
            async move {
                let _ = br.call_one("c", LlmCall::simple("?", 10)).await;
            }
        });
    });
    group.finish();
}

fn bench_bridge_call_many_3(c: &mut Criterion) {
    let runtime = rt();
    let reg = ProviderRegistry::new();
    for id in &["claude", "gpt4", "gemini"] {
        reg.register(Arc::new(MockLlmProvider::new(*id).with_response("ok")));
    }
    let bridge = Arc::new(MultiLlmBridge::new(reg));
    let ids = vec!["claude".to_string(), "gpt4".to_string(), "gemini".to_string()];
    let mut group = c.benchmark_group("bridge");
    group.throughput(Throughput::Elements(3));
    group.bench_function("call_many_3_mocks_parallel", |b| {
        b.to_async(&runtime).iter(|| {
            let br = bridge.clone();
            let ids = ids.clone();
            async move {
                let _ = br.call_many(&ids, LlmCall::simple("?", 10)).await;
            }
        });
    });
    group.finish();
}

// ─── Mission A ──────────────────────────────────────────────────────

fn bench_mission_a_minimal(c: &mut Criterion) {
    let runtime = rt();
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("ok")));
    let coord = Arc::new(Coordinator::builder().registry(reg).build().unwrap());
    let mut metas = HashMap::new();
    metas.insert("c".to_string(), meta());
    let mk = move || MissionARequest {
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["c".into()],
        llm_metadata: metas.clone(),
        budget: Budget {
            max_cost_cents: 100,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
        task_class_scoring_table: None,
    };
    let mut group = c.benchmark_group("workflow");
    group.measurement_time(Duration::from_secs(8));
    group.throughput(Throughput::Elements(1));
    group.bench_function("mission_a_mock_only", |b| {
        b.to_async(&runtime).iter(|| {
            let coord = coord.clone();
            let req = mk();
            async move {
                let _ = run_mission_a(&coord, req).await;
            }
        });
    });
    group.finish();
}

// ─── Mission B ──────────────────────────────────────────────────────

fn bench_mission_b_three_llms(c: &mut Criterion) {
    let runtime = rt();
    let reg = ProviderRegistry::new();
    for id in &["a", "b", "d"] {
        reg.register(Arc::new(MockLlmProvider::new(*id).with_response("Paris")));
    }
    let coord = Arc::new(Coordinator::builder().registry(reg).build().unwrap());
    let llms = vec!["a".into(), "b".into(), "d".into()];
    let mk = move || MissionBRequest {
        user_text: "hi".into(),
        task_class: TaskClass::FactualQa,
        llms: llms.clone(),
        llm_metadata: HashMap::new(),
        vote_weights: HashMap::new(),
        quorum: Some(2),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    };
    let mut group = c.benchmark_group("workflow");
    group.measurement_time(Duration::from_secs(8));
    group.throughput(Throughput::Elements(1));
    group.bench_function("mission_b_3_mocks_parallel", |b| {
        b.to_async(&runtime).iter(|| {
            let coord = coord.clone();
            let req = mk();
            async move {
                let _ = run_mission_b(&coord, req).await;
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_blackboard_deposit,
    bench_blackboard_read,
    bench_vote_3_llms,
    bench_dag_diamond,
    bench_bridge_call_one,
    bench_bridge_call_many_3,
    bench_mission_a_minimal,
    bench_mission_b_three_llms,
);
criterion_main!(benches);
