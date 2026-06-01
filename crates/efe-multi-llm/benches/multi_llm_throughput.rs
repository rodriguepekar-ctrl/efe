// SPDX-License-Identifier: UNLICENSED
//! Benchmarks Criterion : throughput Phase 3 (vote, reformulator, decompose, etc).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::runtime::Runtime;

use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::reformulator_ext::SourceDialect;
use efe_multi_llm::vote_ext::borda::BordaBallot;
use efe_multi_llm::{
    borda_tally, condorcet_tally, run_vote_3llm, BudgetTracker, CanonicalReformulator,
    CanonicalSchemaId, Decomposer, VoteMode, Vote3LlmRequest,
};
use efe_nano_app_coordinator::types::router::TaskClass;

fn rt() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}

// ─── Vote algos ─────────────────────────────────────────────────────

fn bench_borda_3_candidates(c: &mut Criterion) {
    let ballots = vec![
        BordaBallot {
            voter: "claude".into(),
            ranking: vec!["claude".into(), "gpt".into(), "gemini".into()],
        },
        BordaBallot {
            voter: "gpt".into(),
            ranking: vec!["gpt".into(), "claude".into(), "gemini".into()],
        },
        BordaBallot {
            voter: "gemini".into(),
            ranking: vec!["gemini".into(), "claude".into(), "gpt".into()],
        },
    ];
    let mut group = c.benchmark_group("vote");
    group.throughput(Throughput::Elements(1));
    group.bench_function(BenchmarkId::new("borda", "3_candidates"), |b| {
        b.iter(|| {
            let _ = borda_tally(&ballots);
        });
    });
    group.finish();
}

fn bench_condorcet_3_candidates(c: &mut Criterion) {
    let ballots = vec![
        BordaBallot {
            voter: "v1".into(),
            ranking: vec!["A".into(), "B".into(), "C".into()],
        },
        BordaBallot {
            voter: "v2".into(),
            ranking: vec!["A".into(), "C".into(), "B".into()],
        },
        BordaBallot {
            voter: "v3".into(),
            ranking: vec!["B".into(), "A".into(), "C".into()],
        },
    ];
    let mut group = c.benchmark_group("vote");
    group.throughput(Throughput::Elements(1));
    group.bench_function(BenchmarkId::new("condorcet", "3_candidates"), |b| {
        b.iter(|| {
            let _ = condorcet_tally(&ballots);
        });
    });
    group.finish();
}

// ─── Reformulator ───────────────────────────────────────────────────

fn bench_reformulator_claim_xml(c: &mut Criterion) {
    let r = CanonicalReformulator::new();
    let xml = "<claim>The Federal Reserve raised rates by 25bp.</claim>\n<source>WSJ</source>";
    let mut group = c.benchmark_group("reformulator");
    group.throughput(Throughput::Elements(1));
    group.bench_function("to_claim_claude_xml", |b| {
        b.iter(|| {
            let _ = r.to_claim(xml, SourceDialect::ClaudeXml);
        });
    });
    group.finish();
}

fn bench_reformulator_code_markdown(c: &mut Criterion) {
    let r = CanonicalReformulator::new();
    let md = "```rust\nfn fib(n: u32) -> u64 {\n  match n { 0 => 0, 1 => 1, _ => fib(n-1) + fib(n-2) }\n}\n```";
    let mut group = c.benchmark_group("reformulator");
    group.throughput(Throughput::Elements(1));
    group.bench_function("to_code_gemini_md", |b| {
        b.iter(|| {
            let _ = r.to_code(md, SourceDialect::GeminiMarkdown);
        });
    });
    group.finish();
}

fn bench_reformulator_route_all_5_schemas(c: &mut Criterion) {
    let r = CanonicalReformulator::new();
    let raw = "Some answer with reasoning bullet 1. And bullet 2. Confidence: 0.9.";
    let mut group = c.benchmark_group("reformulator");
    group.throughput(Throughput::Elements(5));
    group.bench_function("route_5_schemas", |b| {
        b.iter(|| {
            for target in [
                CanonicalSchemaId::PlanV1,
                CanonicalSchemaId::CodeV1,
                CanonicalSchemaId::ClaimV1,
                CanonicalSchemaId::ArgumentV1,
                CanonicalSchemaId::SynthesisV1,
            ] {
                let _ = r.route(raw, SourceDialect::Gpt4Json, target, "gpt_5");
            }
        });
    });
    group.finish();
}

// ─── Decomposer ─────────────────────────────────────────────────────

fn bench_decomposer_3_subtasks(c: &mut Criterion) {
    let d = Decomposer::default();
    let text = "Translate this Python to Rust. Then explain the algorithm. Then write a haiku.";
    let mut group = c.benchmark_group("decompose");
    group.throughput(Throughput::Elements(1));
    group.bench_function("3_subtasks", |b| {
        b.iter(|| {
            let _ = d.decompose(text);
        });
    });
    group.finish();
}

// ─── BudgetTracker ──────────────────────────────────────────────────

fn bench_budget_tracker_charge(c: &mut Criterion) {
    let t = BudgetTracker::new(1_000_000);
    let mut group = c.benchmark_group("budget");
    group.throughput(Throughput::Elements(1));
    group.bench_function("try_charge_atomic", |b| {
        b.iter(|| {
            let _ = t.try_charge("claude", 1);
        });
    });
    group.finish();
}

// ─── Workflow ───────────────────────────────────────────────────────

fn bench_vote_3llm_mock(c: &mut Criterion) {
    let runtime = rt();
    let reg = ProviderRegistry::new();
    for id in &["claude", "gpt", "gemini"] {
        reg.register(Arc::new(MockLlmProvider::new(*id).with_response("Paris.")));
    }
    let coord = Arc::new(Coordinator::builder().registry(reg).build().unwrap());
    let mk = move || Vote3LlmRequest {
        user_text: "Capital of France?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Borda,
        quorum: Some(2),
        max_budget_cents: 100,
        max_total_latency: Duration::from_secs(5),
    };
    let mut group = c.benchmark_group("workflow");
    group.measurement_time(Duration::from_secs(8));
    group.throughput(Throughput::Elements(1));
    group.bench_function("vote_3llm_mock_borda", |b| {
        b.to_async(&runtime).iter(|| {
            let coord = coord.clone();
            let req = mk();
            async move {
                let _ = run_vote_3llm(&coord, req).await;
            }
        });
    });
    group.finish();
    let _ = HashMap::<String, f32>::new(); // silence unused import
}

criterion_group!(
    benches,
    bench_borda_3_candidates,
    bench_condorcet_3_candidates,
    bench_reformulator_claim_xml,
    bench_reformulator_code_markdown,
    bench_reformulator_route_all_5_schemas,
    bench_decomposer_3_subtasks,
    bench_budget_tracker_charge,
    bench_vote_3llm_mock,
);
criterion_main!(benches);
