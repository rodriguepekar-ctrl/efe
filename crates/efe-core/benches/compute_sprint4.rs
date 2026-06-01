//! Sprint 4 Light benchmarks — matmul naive vs BLAS + fused epilog.
//!
//! Run :
//! ```bash
//! cargo bench -p efe-core --bench compute_sprint4 -- --quick
//! cargo bench -p efe-core --bench compute_sprint4 --features accelerate -- --quick
//! ```
//!
//! Compare the two reports under `target/criterion/` to see the BLAS
//! speedup per size.
#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};

fn xorshift_vec(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed.max(1);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32) * 2.0 - 1.0
        })
        .collect()
}

fn random_tensor(shape: &[usize], seed: u64) -> Tensor<f32> {
    let n: usize = shape.iter().product();
    Tensor::<f32>::from_slice(&xorshift_vec(n, seed), shape.to_vec().into()).unwrap()
}

// ---------------------------------------------------------------------
// matmul : progression from below-threshold to above
// ---------------------------------------------------------------------

fn bench_matmul_progression(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("matmul");
    group.sample_size(15);

    for &n in &[32_usize, 64, 128, 256] {
        let a = random_tensor(&[n, n], 11);
        let b = random_tensor(&[n, n], 22);
        group.throughput(Throughput::Elements((n * n * n) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |bencher, _| {
                bencher.iter(|| backend.matmul(black_box(&a), black_box(&b)).unwrap());
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------
// matmul_fused_gelu : measures the combined effect of BLAS matmul +
// single-pass bias+GELU epilog.
// ---------------------------------------------------------------------

fn bench_matmul_fused_gelu(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("matmul_fused_gelu");
    group.sample_size(15);

    for &n in &[64_usize, 128, 256] {
        let a = random_tensor(&[n, n], 3);
        let b = random_tensor(&[n, n], 5);
        let bias = random_tensor(&[n], 7);
        group.throughput(Throughput::Elements((n * n * n) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |bencher, _| {
                bencher.iter(|| {
                    backend
                        .matmul_fused_gelu(
                            black_box(&a),
                            black_box(&b),
                            Some(black_box(&bias)),
                        )
                        .unwrap()
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------
// matmul_no_bias : same shapes, None bias — measures the GELU-only
// epilog path.
// ---------------------------------------------------------------------

fn bench_matmul_fused_gelu_no_bias(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("matmul_fused_gelu_no_bias");
    group.sample_size(15);

    for &n in &[64_usize, 128, 256] {
        let a = random_tensor(&[n, n], 11);
        let b = random_tensor(&[n, n], 22);
        group.throughput(Throughput::Elements((n * n * n) as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(n),
            &n,
            |bencher, _| {
                bencher.iter(|| {
                    backend
                        .matmul_fused_gelu(black_box(&a), black_box(&b), None)
                        .unwrap()
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_matmul_progression,
    bench_matmul_fused_gelu,
    bench_matmul_fused_gelu_no_bias,
);
criterion_main!(benches);
