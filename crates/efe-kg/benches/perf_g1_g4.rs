// SPDX-License-Identifier: UNLICENSED
//! M33 perf-rewrite microbench — G1 (for_each_out_edge), G2+G3 (BFS
//! parent-pointer + pre-alloc), G4 (bulk_upsert).
//!
//! Run :
//!
//! ```bash
//! cargo bench -p efe-kg --bench perf_g1_g4
//! ```
//!
//! Groups :
//!
//! 1. `bulk_upsert/g4` — bulk vs single upsert wall-clock at
//!    1k / 10k entity inserts.
//! 2. `bfs/g1_g2_g3` — BFS depth=3 on a graph of 10 k nodes.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use efe_kg::{bfs, Entity, EntityKind, Graph, Predicate, Relation, TraversalFilter};

fn build_chain(n: usize) -> Graph {
    let g = Graph::new();
    let entities: Vec<Entity> = (0..n)
        .map(|i| Entity::new(format!("e{i}"), EntityKind::Misc, format!("e{i}")))
        .collect();
    g.bulk_upsert_entities(entities).unwrap();
    let relations: Vec<Relation> = (0..n - 1)
        .map(|i| {
            Relation::new(
                format!("r{i}"),
                format!("e{i}"),
                Predicate::new("relates_to").unwrap(),
                format!("e{}", i + 1),
                0.9,
            )
        })
        .collect();
    g.bulk_upsert_relations(relations).unwrap();
    g
}

fn build_dense_graph(n: usize, fan_out: usize) -> Graph {
    // A graph where each node connects to `fan_out` next nodes
    // (modulo wrap-around) — closer to BFS workloads on real
    // knowledge graphs.
    let g = Graph::new();
    let entities: Vec<Entity> = (0..n)
        .map(|i| Entity::new(format!("e{i}"), EntityKind::Misc, format!("e{i}")))
        .collect();
    g.bulk_upsert_entities(entities).unwrap();
    let mut relations = Vec::with_capacity(n * fan_out);
    for i in 0..n {
        for k in 1..=fan_out {
            let j = (i + k) % n;
            relations.push(Relation::new(
                format!("r-{i}-{j}"),
                format!("e{i}"),
                Predicate::new("relates_to").unwrap(),
                format!("e{j}"),
                0.9,
            ));
        }
    }
    g.bulk_upsert_relations(relations).unwrap();
    g
}

fn bench_bulk_upsert_g4(c: &mut Criterion) {
    for &n in &[1_000usize, 10_000] {
        // Pre-build the entity vec so the bench measures only the
        // graph-side cost.
        let entities: Vec<Entity> = (0..n)
            .map(|i| Entity::new(format!("e{i}"), EntityKind::Misc, format!("e{i}")))
            .collect();

        let mut group = c.benchmark_group(format!("bulk_upsert/g4_n={n}"));
        group.throughput(Throughput::Elements(n as u64));

        // Single-call baseline : `upsert_entity` inside a loop.
        group.bench_function("single_upsert_loop", |b| {
            b.iter_batched(
                || entities.clone(),
                |ents| {
                    let g = Graph::new();
                    for e in ents {
                        g.upsert_entity(e).unwrap();
                    }
                },
                BatchSize::SmallInput,
            );
        });

        // V/G4 : one bulk call.
        group.bench_function("bulk_upsert_G4", |b| {
            b.iter_batched(
                || entities.clone(),
                |ents| {
                    let g = Graph::new();
                    g.bulk_upsert_entities(ents).unwrap();
                },
                BatchSize::SmallInput,
            );
        });
        group.finish();
    }
}

fn bench_bfs_g1_g2_g3(c: &mut Criterion) {
    for &(n, fan_out) in &[(1_000usize, 5usize), (10_000, 10)] {
        let g = build_dense_graph(n, fan_out);
        let mut group = c.benchmark_group(format!("bfs/g1_g2_g3_n={n}_fan={fan_out}"));
        group.throughput(Throughput::Elements(1));
        group.bench_function("bfs_depth_3_top_100", |b| {
            b.iter(|| bfs(&g, "e0", 3, &TraversalFilter::default(), 100).unwrap());
        });
        group.bench_function("bfs_depth_4_top_1000", |b| {
            b.iter(|| bfs(&g, "e0", 4, &TraversalFilter::default(), 1000).unwrap());
        });
        group.finish();
    }
}

fn bench_bfs_chain(c: &mut Criterion) {
    // Chain : worst-case path-length growth → G2 parent-pointer
    // shines vs the pre-G2 path-clone approach.
    let g = build_chain(1_000);
    let mut group = c.benchmark_group("bfs/chain_n=1k");
    group.throughput(Throughput::Elements(1));
    group.bench_function("bfs_full_chain_top_1000", |b| {
        b.iter(|| bfs(&g, "e0", 6, &TraversalFilter::default(), 1000).unwrap());
    });
    group.finish();
}

criterion_group!(
    perf_g1_g4,
    bench_bulk_upsert_g4,
    bench_bfs_g1_g2_g3,
    bench_bfs_chain,
);
criterion_main!(perf_g1_g4);
