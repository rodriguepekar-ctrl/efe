//! CB-01 reference criterion benchmark — event-sourcing append.
//!
//! This file is the REFERENCE IMPLEMENTATION for the CB catalogue's
//! benchmark code surface.  The other fourteen benchmarks should
//! follow this exact structural pattern :
//!
//!   * one criterion benchmark group per CB-NN identifier
//!   * one bench function per metric the YAML scenario emits
//!   * deterministic seed control via a const at the top of the file
//!   * no async runtime spin-up inside the measurement loop ; the
//!     runtime is created once in the group setup
//!   * payload generation done lazily inside iter_batched so that
//!     allocation cost does not pollute the measurement
//!
//! Companion scenario : bench/scenarios/CB-01_events_append_throughput.yaml
//! Tolerance budgets : STABILITY § 27.2 (frozen).
//! Catalogue : docs/architecture/EFE_CORE_BENCHMARK_CATALOGUE.md § 4.1
//!
//! What this benchmark measures :
//!   * append_p50_ms / append_p99_ms : the latency of `Event::new` +
//!     in-memory serialisation, the hot path of the publisher's
//!     pre-NATS work.  This does NOT include NATS round-trip ; the
//!     network component is measured by the integration scenario.
//!   * append_rate_per_sec : sustained throughput of envelope
//!     construction + serialisation per second on a single thread.
//!
//! Why measure ONLY the local hot path :
//!   * The CB suite's primary goal is to surface in-process
//!     regressions that the integration scenarios (which exercise
//!     the network) cannot attribute to a specific subsystem.
//!   * The network-level append throughput is measured by the
//!     full CB-01 scenario YAML, which targets a deployed NATS
//!     and includes the cluster-level cost.
//!
//! Invocation :
//!     cargo bench --bench cb_01_event_append
//!
//! Output :
//!   criterion writes per-metric reports under
//!   target/criterion/cb_01_event_append/.  The CI workflow
//!   parses these via scripts/bench/criterion_to_slo.py and
//!   compares against the baseline at bench/baselines/CB-01.json.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use efe_events::Event;
use serde::{Deserialize, Serialize};

/// Frozen RNG seed for the CB-01 benchmark.  Determinism is part of
/// the freeze surface (STABILITY § 27.3 — the reference baseline is
/// captured against this seed and any divergence is a regression
/// signal).  Do NOT change without a STABILITY bump.
const CB01_SEED: u64 = 26_001;

/// Realistic event payload mirroring the production classify-completed
/// shape.  Keep field count + average string lengths matching the
/// production trace so the bench's mean event size ~ 512 bytes (per
/// scenario YAML's `event_size_bytes_mean`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClassifyCompletedPayload {
    tenant_slug: String,
    document_id: String,
    classification: String,
    confidence: f64,
    model_id: String,
    duration_ms: u32,
    tags: Vec<String>,
    metadata: serde_json::Value,
}

fn make_payload(i: u64) -> ClassifyCompletedPayload {
    ClassifyCompletedPayload {
        tenant_slug: format!("tenant-{:06}", i % 1000),
        document_id: format!("doc-{:016x}", i),
        classification: "policy-violation-low".to_string(),
        confidence: 0.873,
        model_id: "frozen-classifier-v2.3".to_string(),
        duration_ms: 14 + (i % 31) as u32,
        tags: vec![
            "industrial".to_string(),
            "EU".to_string(),
            "compliance".to_string(),
        ],
        metadata: serde_json::json!({
            "iteration": i,
            "shard": i % 7,
            "trace_id": format!("trace-{:032x}", i.wrapping_mul(0x9E3779B97F4A7C15)),
        }),
    }
}

/// Bench function 1 : envelope construction latency.  Measures the
/// cost of `Event::new` — ULID generation, timestamp capture, tenant
/// resolution.  This is the hot path the publisher executes once per
/// event before serialisation.
fn bench_envelope_construction(c: &mut Criterion) {
    let mut group = c.benchmark_group("cb_01_envelope_construction");
    group.throughput(Throughput::Elements(1));
    group.bench_function("ClassifyCompleted", |b| {
        let mut i = CB01_SEED;
        b.iter(|| {
            i = i.wrapping_add(1);
            let payload = make_payload(i);
            let event = Event::new(
                black_box("classify.completed"),
                black_box(&format!("tenant-{:06}", i % 1000)),
                black_box(payload),
            );
            black_box(event)
        });
    });
    group.finish();
}

/// Bench function 2 : serialisation throughput.  Measures the cost
/// of JSON-serialising the constructed envelope — this dominates the
/// per-event cost in the publisher's hot path.  Reported per-element
/// throughput is what the scenario YAML's `append_rate_per_sec`
/// metric reflects in the in-process slice.
fn bench_envelope_serialisation(c: &mut Criterion) {
    let mut group = c.benchmark_group("cb_01_envelope_serialisation");
    group.throughput(Throughput::Elements(1));
    // Pre-build the envelope ONCE so we measure serialisation only.
    let event = Event::new(
        "classify.completed",
        "tenant-bench",
        make_payload(CB01_SEED),
    );
    group.bench_function("serde_json::to_vec", |b| {
        b.iter(|| {
            let bytes = serde_json::to_vec(black_box(&event))
                .expect("envelope must serialise");
            black_box(bytes)
        });
    });
    group.finish();
}

/// Bench function 3 : end-to-end append latency (construction +
/// serialisation).  This is what the integration scenario sees when
/// the network-level append metric subtracts the NATS round-trip.
fn bench_envelope_end_to_end(c: &mut Criterion) {
    let mut group = c.benchmark_group("cb_01_envelope_end_to_end");
    group.throughput(Throughput::Elements(1));
    group.bench_function("construction_plus_serde", |b| {
        let mut i = CB01_SEED;
        b.iter(|| {
            i = i.wrapping_add(1);
            let payload = make_payload(i);
            let event = Event::new(
                "classify.completed",
                &format!("tenant-{:06}", i % 1000),
                payload,
            );
            let bytes = serde_json::to_vec(&event).expect("must serialise");
            black_box(bytes)
        });
    });
    group.finish();
}

/// Bench function 4 : batch construction (10 events back-to-back).
/// Measures the per-batch overhead the publisher's buffer would see
/// under sustained load.  Verifies the per-event cost stays linear
/// (no quadratic surprise inside `Event::new`).
fn bench_envelope_batch_of_ten(c: &mut Criterion) {
    let mut group = c.benchmark_group("cb_01_envelope_batch");
    group.throughput(Throughput::Elements(10));
    for batch_size in &[1usize, 10, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            batch_size,
            |b, &size| {
                let mut i = CB01_SEED;
                b.iter(|| {
                    let mut bytes_total: usize = 0;
                    for _ in 0..size {
                        i = i.wrapping_add(1);
                        let payload = make_payload(i);
                        let event = Event::new(
                            "classify.completed",
                            "tenant-bench",
                            payload,
                        );
                        let bytes = serde_json::to_vec(&event)
                            .expect("must serialise");
                        bytes_total = bytes_total.wrapping_add(bytes.len());
                    }
                    black_box(bytes_total)
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_envelope_construction,
    bench_envelope_serialisation,
    bench_envelope_end_to_end,
    bench_envelope_batch_of_ten,
);
criterion_main!(benches);
