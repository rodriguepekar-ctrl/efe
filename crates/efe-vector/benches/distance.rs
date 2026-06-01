// SPDX-License-Identifier: UNLICENSED
//! M32 distance-metric microbench.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use efe_vector::distance::{cosine_similarity, dot_product, l2_distance};
use rand::{Rng, SeedableRng};

fn random_vec(dim: usize, rng: &mut rand::rngs::StdRng) -> Vec<f32> {
    (0..dim).map(|_| rng.gen_range(-1.0..1.0_f32)).collect()
}

fn bench_distance(c: &mut Criterion) {
    let dims = [64, 256, 768];
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xCAFEBABE);
    for &dim in &dims {
        let a = random_vec(dim, &mut rng);
        let b = random_vec(dim, &mut rng);
        let mut group = c.benchmark_group(format!("distance/dim={dim}"));
        group.throughput(Throughput::Elements(1));
        group.bench_function("dot", |bm| {
            bm.iter(|| dot_product(&a, &b));
        });
        group.bench_function("cosine", |bm| {
            bm.iter(|| cosine_similarity(&a, &b));
        });
        group.bench_function("l2", |bm| {
            bm.iter(|| l2_distance(&a, &b));
        });
        group.finish();
    }
}

criterion_group!(benches, bench_distance);
criterion_main!(benches);
