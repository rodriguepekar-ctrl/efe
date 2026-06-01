// SPDX-License-Identifier: UNLICENSED
//! M32 perf-rewrite microbenchmarks (V1 – V6).
//!
//! Run :
//!
//! ```bash
//! cargo bench -p efe-vector --bench perf_v1_v6
//! ```
//!
//! Groups :
//!
//! 1. `distance/v1_chunked` — chunked-vs-naive dot / cosine / L2.
//! 2. `distance/v2_cosine_normalized` — fast-path skips norms.
//! 3. `quantize/v5_hamming_u64` — u64-chunked vs byte-by-byte.
//! 4. `flat_search/v1_dim768` — full flat search at typical dim.
//!
//! The naive baselines stay inlined here so the bench produces an
//! apples-to-apples comparison even after refactors.

use criterion::{criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use efe_vector::{
    cosine_normalized, cosine_similarity, dot_product, l2_distance, l2_normalize,
    squared_l2_norm, BinaryVector, Distance, FlatIndex, VectorDoc,
};
use rand::{Rng, SeedableRng};

fn rand_vec(dim: usize, rng: &mut rand::rngs::StdRng) -> Vec<f32> {
    (0..dim).map(|_| rng.gen_range(-1.0..1.0_f32)).collect()
}

/// Naive single-accumulator dot product — baseline for V1.
#[inline(never)]
fn naive_dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[inline(never)]
fn naive_cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na < 1e-12 || nb < 1e-12 {
        return 0.0;
    }
    dot / (na * nb)
}

#[inline(never)]
fn naive_l2(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        let d = x - y;
        acc += d * d;
    }
    acc.sqrt()
}

/// Naive byte-by-byte hamming (V5 baseline).
#[inline(never)]
fn naive_hamming(a: &BinaryVector, b: &BinaryVector) -> u32 {
    a.bits
        .iter()
        .zip(b.bits.iter())
        .map(|(x, y)| (x ^ y).count_ones())
        .sum()
}

fn bench_distance_v1(c: &mut Criterion) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xCAFEBABE);
    for &dim in &[64, 256, 768] {
        let a = rand_vec(dim, &mut rng);
        let b = rand_vec(dim, &mut rng);
        let mut group = c.benchmark_group(format!("distance/v1_dim={dim}"));
        group.throughput(Throughput::Elements(1));

        group.bench_function("naive_dot", |bm| bm.iter(|| naive_dot(&a, &b)));
        group.bench_function("chunked_dot_V1", |bm| bm.iter(|| dot_product(&a, &b)));

        group.bench_function("naive_cosine", |bm| bm.iter(|| naive_cosine(&a, &b)));
        group.bench_function("chunked_cosine_V1", |bm| {
            bm.iter(|| cosine_similarity(&a, &b))
        });

        group.bench_function("naive_l2", |bm| bm.iter(|| naive_l2(&a, &b)));
        group.bench_function("chunked_l2_V1", |bm| bm.iter(|| l2_distance(&a, &b)));

        group.bench_function("squared_l2_norm_V1", |bm| {
            bm.iter(|| squared_l2_norm(&a))
        });
        group.finish();
    }
}

fn bench_cosine_normalized_v2(c: &mut Criterion) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0x12345);
    for &dim in &[64, 768] {
        let mut a = rand_vec(dim, &mut rng);
        let mut b = rand_vec(dim, &mut rng);
        l2_normalize(&mut a);
        l2_normalize(&mut b);
        let mut group = c.benchmark_group(format!("distance/v2_dim={dim}"));
        group.throughput(Throughput::Elements(1));
        group.bench_function("cosine_similarity_slow", |bm| {
            bm.iter(|| cosine_similarity(&a, &b))
        });
        group.bench_function("cosine_normalized_V2", |bm| {
            bm.iter(|| cosine_normalized(&a, &b))
        });
        group.finish();
    }
}

fn bench_hamming_v5(c: &mut Criterion) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xABCDEF);
    for &dim in &[256, 768] {
        let v1 = rand_vec(dim, &mut rng);
        let v2 = rand_vec(dim, &mut rng);
        let q1 = BinaryVector::from_f32(&v1).unwrap();
        let q2 = BinaryVector::from_f32(&v2).unwrap();
        let mut group = c.benchmark_group(format!("quantize/v5_dim={dim}"));
        group.throughput(Throughput::Elements(1));
        group.bench_function("naive_hamming", |bm| bm.iter(|| naive_hamming(&q1, &q2)));
        group.bench_function("u64_hamming_V5", |bm| bm.iter(|| q1.hamming(&q2)));
        group.finish();
    }
}

fn bench_flat_search_v1(c: &mut Criterion) {
    let dim = 768;
    let n = 1_000;
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xFEEDFEED);
    let mut idx = FlatIndex::new(dim, Distance::Cosine);
    for i in 0..n {
        idx.insert(VectorDoc::new(
            format!("d{i}"),
            rand_vec(dim, &mut rng),
        ))
        .unwrap();
    }
    let q = rand_vec(dim, &mut rng);
    let mut group = c.benchmark_group("flat_search/v1_n=1k");
    group.throughput(Throughput::Elements(1));
    group.bench_function("top10", |b| {
        b.iter_batched(
            || q.clone(),
            |q| idx.search(&q, 10).unwrap(),
            BatchSize::SmallInput,
        );
    });
    group.finish();
}

criterion_group!(
    perf_v1_v6,
    bench_distance_v1,
    bench_cosine_normalized_v2,
    bench_hamming_v5,
    bench_flat_search_v1,
);
criterion_main!(perf_v1_v6);
