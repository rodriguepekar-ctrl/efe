// SPDX-License-Identifier: UNLICENSED
//! M32 flat-search microbench.

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use efe_vector::{Distance, FlatIndex, VectorDoc};
use rand::{Rng, SeedableRng};

fn random_vec(dim: usize, rng: &mut rand::rngs::StdRng) -> Vec<f32> {
    (0..dim).map(|_| rng.gen_range(-1.0..1.0_f32)).collect()
}

fn bench_flat_search(c: &mut Criterion) {
    let dim = 768;
    let mut rng = rand::rngs::StdRng::seed_from_u64(0xDEAD_BEEF);
    for &n in [100, 1_000, 10_000].iter() {
        let mut idx = FlatIndex::new(dim, Distance::Cosine);
        for i in 0..n {
            idx.insert(VectorDoc::new(format!("d{i}"), random_vec(dim, &mut rng)))
                .unwrap();
        }
        let q = random_vec(dim, &mut rng);
        let mut group = c.benchmark_group(format!("flat_search/n={n}"));
        group.throughput(Throughput::Elements(1));
        group.bench_function("top10", |b| {
            b.iter(|| idx.search(&q, 10).unwrap());
        });
        group.finish();
    }
}

criterion_group!(benches, bench_flat_search);
criterion_main!(benches);
