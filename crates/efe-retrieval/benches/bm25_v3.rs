// SPDX-License-Identifier: UNLICENSED
//! M32 perf-rewrite — BM25 inverted index (V3) microbench.
//!
//! Compares the wall-clock of a rare-token search against
//! 100 / 1 000 / 10 000 documents. The expectation is that
//! latency scales with `Σ_t |postings(t)|` (matching docs), not
//! `N × |query|` (all docs × query tokens) as before V3.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use efe_retrieval::Bm25Index;

fn build_corpus(n: usize) -> Bm25Index {
    let idx = Bm25Index::default();
    for i in 0..n {
        // Most docs share frequent tokens to inflate the
        // pre-V3 cost. Only 1 % of docs contain the rare token.
        let mut s = String::from("common common common common common");
        if i % 100 == 0 {
            s.push_str(" rare_token");
        }
        idx.upsert(&format!("d{i}"), &s).unwrap();
    }
    idx
}

fn bench_bm25_v3(c: &mut Criterion) {
    for &n in &[100usize, 1_000, 10_000] {
        let idx = build_corpus(n);
        let mut group = c.benchmark_group(format!("bm25/v3_n={n}"));
        group.throughput(Throughput::Elements(1));

        // Rare-token query : posting list ~ 1 % of N ⇒ V3 stays
        // fast as N grows. Pre-V3 was O(N × 1).
        group.bench_function("rare_token", |b| {
            b.iter(|| idx.search("rare_token", 10).unwrap());
        });

        // Common-token query : posting list ≈ N ⇒ V3 and pre-V3
        // are roughly tied (everything matches).
        group.bench_function("common_token", |b| {
            b.iter(|| idx.search("common", 10).unwrap());
        });
        group.finish();
    }
}

criterion_group!(benches, bench_bm25_v3);
criterion_main!(benches);
