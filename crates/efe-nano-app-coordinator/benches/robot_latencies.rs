// SPDX-License-Identifier: UNLICENSED
//! Benchmarks Criterion : p50/p95/p99 par robot.
//!
//! Deux modes :
//! - **Mode mock** (par défaut) : wiremock, mesure overhead Rust (HTTP + serde
//!   + audit).  Permet de quantifier la part Rust dans la latence end-to-end.
//! - **Mode live** (`EFE_BENCH_LIVE=1`) : frappe les vrais `llama-server` aux
//!   ports 8090-8093.  Donne les vraies cibles SLO.
//!
//! Usage :
//!
//! ```bash
//! # Mode mock (rapide, sans dependence)
//! PYO3_PYTHON=$(which python3.12) \
//!   cargo bench -p efe-nano-app-coordinator --bench robot_latencies
//!
//! # Mode live (nécessite que les 4 robots tournent)
//! EFE_BENCH_LIVE=1 PYO3_PYTHON=$(which python3.12) \
//!   cargo bench -p efe-nano-app-coordinator --bench robot_latencies
//! ```
//!
//! Les résultats HTML sont dans `target/criterion/`.

use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use efe_nano_app_coordinator::clients::{
    AnalyzerClient, FilterClient, LlamaServerClient, NanoRobotClient, RouterClient,
};
use efe_nano_app_coordinator::context::{CallContext, InMemoryAuditWriter};
use efe_nano_app_coordinator::types::*;
use std::collections::HashMap;
use tokio::runtime::Runtime;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn live_mode() -> bool {
    std::env::var("EFE_BENCH_LIVE").as_deref() == Ok("1")
}

fn rt() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

// ─── Helpers : construction de clients (mock ou live) ──────────────────

fn analyzer_response_payload() -> serde_json::Value {
    let content = serde_json::json!({
        "entities": [{"text": "Apple", "kind": "ORG",
                      "start_char": 0, "end_char": 5, "confidence": 0.97}],
        "intent": "FACTUAL",
        "sentiment": 0.1,
        "language": "en",
        "one_line_summary": "Apple Q3 revenue",
        "confidence": 0.9
    });
    serde_json::json!({
        "content": content.to_string(),
        "tokens_evaluated": 14,
        "tokens_predicted": 30
    })
}

async fn build_analyzer_mock() -> (MockServer, AnalyzerClient) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(analyzer_response_payload()))
        .mount(&server)
        .await;
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: server.uri(),
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 64,
        temperature: 0.1,
        timeout: Duration::from_secs(2),
        http: reqwest::Client::new(),
    };
    (server, AnalyzerClient::from_helper(inner))
}

fn build_analyzer_live() -> AnalyzerClient {
    let url = std::env::var("ANALYZER_URL").unwrap_or_else(|_| "http://127.0.0.1:8090".into());
    AnalyzerClient::from_assets(url).expect("from_assets analyzer")
}

// ─── Benchmarks ─────────────────────────────────────────────────────────

fn bench_analyzer_invoke(c: &mut Criterion) {
    let runtime = rt();
    let mut group = c.benchmark_group("analyzer");
    group.measurement_time(Duration::from_secs(5));
    group.sample_size(50);
    group.throughput(Throughput::Elements(1));

    if live_mode() {
        let client = build_analyzer_live();
        group.bench_function(BenchmarkId::new("invoke", "live"), |b| {
            b.to_async(&runtime).iter(|| async {
                let ctx = CallContext::new(Uuid::now_v7(), Arc::new(InMemoryAuditWriter::default()));
                let input = AnalyzerInput {
                    text: "Apple reported $90B revenue in Q3 2026.".into(),
                    max_entities: 20,
                    source_agent: None,
                };
                let _ = client.invoke(input, &ctx).await;
            });
        });
    } else {
        // Build mock outside iter to avoid wiremock overhead per iteration.
        let (server, client) = runtime.block_on(async { build_analyzer_mock().await });
        let server = Arc::new(server);
        let client = Arc::new(client);

        group.bench_function(BenchmarkId::new("invoke", "mock"), |b| {
            let client = client.clone();
            let _keep_server = server.clone();
            b.to_async(&runtime).iter(|| {
                let client = client.clone();
                async move {
                    let ctx = CallContext::new(
                        Uuid::now_v7(),
                        Arc::new(InMemoryAuditWriter::default()),
                    );
                    let input = AnalyzerInput {
                        text: "Apple reported $90B revenue Q3.".into(),
                        max_entities: 20,
                        source_agent: None,
                    };
                    let _ = client.invoke(input, &ctx).await;
                }
            });
        });
    }
    group.finish();
}

fn bench_filter_invoke(c: &mut Criterion) {
    let runtime = rt();
    let client = Arc::new(FilterClient::mock());

    // Pré-calcule l'input pour mesurer uniquement l'overhead du robot.
    use efe_nano_app_coordinator::clients::filter::MockEmbedder;
    use efe_nano_app_coordinator::clients::EmbeddingBackend;
    let cap = runtime.block_on(async {
        MockEmbedder::default().embed("test message").await.unwrap()
    });
    let mut caps = HashMap::new();
    caps.insert("claude".to_string(), cap.clone());
    caps.insert("gpt4".to_string(), cap);
    let mut budgets = HashMap::new();
    budgets.insert("claude".to_string(), 5000);
    budgets.insert("gpt4".to_string(), 5000);

    let input = FilterInput {
        message_summary: "test message".into(),
        available_agents: vec!["claude".into(), "gpt4".into()],
        agent_capabilities: caps,
        context_budgets: budgets,
    };

    let mut group = c.benchmark_group("filter");
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(1));

    group.bench_function("invoke_2_agents", |b| {
        let client = client.clone();
        let input = input.clone();
        b.to_async(&runtime).iter(|| {
            let client = client.clone();
            let input = input.clone();
            async move {
                let ctx = CallContext::new(
                    Uuid::now_v7(),
                    Arc::new(InMemoryAuditWriter::default()),
                );
                let _ = client.invoke(input, &ctx).await;
            }
        });
    });
    group.finish();
}

fn bench_router_invoke(c: &mut Criterion) {
    use efe_nano_app_coordinator::types::router::TaskClass;
    let runtime = rt();
    let client = Arc::new(RouterClient::new());

    let mut meta = HashMap::new();
    let mut caps = HashMap::new();
    caps.insert("FACTUAL_QA".into(), 0.9);
    meta.insert(
        "claude".into(),
        LlmMetadata {
            cost_input_per_million: 1500,
            cost_output_per_million: 7500,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            cost_input_per_million_microcents: 0,
            cost_output_per_million_microcents: 0,
            p50_latency_ms: 900,
            p99_latency_ms: 3000,
            success_rate: 0.99,
            capability_scores: caps.clone(),
            circuit_open: false,
        },
    );
    meta.insert(
        "gpt4".into(),
        LlmMetadata {
            cost_input_per_million: 1000,
            cost_output_per_million: 3000,
            // R3.S1 : 0 → _resolved() fallback à cents × 1000.
            cost_input_per_million_microcents: 0,
            cost_output_per_million_microcents: 0,
            p50_latency_ms: 1200,
            p99_latency_ms: 4500,
            success_rate: 0.995,
            capability_scores: caps,
            circuit_open: false,
        },
    );

    let input = RouterInput {
        task: TaskDescription {
            prompt_summary: "factual question".into(),
            task_class: TaskClass::FactualQa,
            estimated_input_tokens: 1000,
            expected_output_tokens: 500,
        },
        available_llms: vec!["claude".into(), "gpt4".into()],
        llm_metadata: meta,
        user_budget: Budget {
            max_cost_cents: 10_000,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
    };

    let mut group = c.benchmark_group("router");
    group.measurement_time(Duration::from_secs(5));
    group.throughput(Throughput::Elements(1));

    group.bench_function("invoke_2_llms", |b| {
        let client = client.clone();
        let input = input.clone();
        b.to_async(&runtime).iter(|| {
            let client = client.clone();
            let input = input.clone();
            async move {
                let ctx = CallContext::new(
                    Uuid::now_v7(),
                    Arc::new(InMemoryAuditWriter::default()),
                );
                let _ = client.invoke(input, &ctx).await;
            }
        });
    });
    group.finish();
}

fn bench_cosine_similarity(c: &mut Criterion) {
    use efe_nano_app_coordinator::clients::filter::cosine;
    let v1: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.001).sin()).collect();
    let v2: Vec<f32> = (0..1024).map(|i| (i as f32 * 0.001).cos()).collect();

    c.bench_function("cosine_1024d", |b| {
        b.iter(|| {
            let _ = cosine(&v1, &v2);
        });
    });
}

fn bench_blake3_hash_canonical_json(c: &mut Criterion) {
    use efe_nano_app_coordinator::context::blake3_canonical_json;
    let out = AnalyzerOutput {
        entities: vec![
            Entity {
                text: "Federal Reserve".into(),
                kind: EntityKind::Org,
                start_char: 0,
                end_char: 15,
                confidence: 0.98,
            };
            5
        ],
        intent: IntentClass::Factual,
        sentiment: 0.0,
        language: IsoLang::En,
        one_line_summary: "Lorem ipsum dolor sit amet consectetur adipiscing elit".into(),
        confidence: 0.9,
    };
    c.bench_function("blake3_canonical_json_analyzer_output", |b| {
        b.iter(|| {
            let _ = blake3_canonical_json(&out);
        });
    });
}

criterion_group!(
    benches,
    bench_analyzer_invoke,
    bench_filter_invoke,
    bench_router_invoke,
    bench_cosine_similarity,
    bench_blake3_hash_canonical_json,
);
criterion_main!(benches);
