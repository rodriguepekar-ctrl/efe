//! Criterion benchmarks for Sprint 2 compute operations.
//!
//! Each operation is benchmarked at three problem sizes (small / medium /
//! large) so regressions are caught across the memory-hierarchy spectrum.
//!
//! Run with :
//! ```bash
//! cargo bench -p efe-core --bench compute_sprint2
//! ```
//!
//! HTML reports are generated under `target/criterion/` when the crate's
//! default criterion features are enabled.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};

fn random_vec(n: usize, seed: u64) -> Vec<f32> {
    // Deterministic, no external RNG crate needed here.
    // xorshift-based cheap PRNG.
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            ((s & 0xFFFFFF) as f32 / 0xFFFFFF as f32) * 2.0 - 1.0
        })
        .collect()
}

fn random_tensor(shape: &[usize], seed: u64) -> Tensor<f32> {
    let n: usize = shape.iter().product();
    let data = random_vec(n, seed);
    Tensor::<f32>::from_slice(&data, shape.to_vec().into()).unwrap()
}

// ====================================================================
// softmax
// ====================================================================

fn bench_softmax(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("softmax");
    for &size in &[64usize, 512, 4096] {
        let x = random_tensor(&[size], 42);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| backend.softmax(black_box(&x), -1).unwrap());
        });
    }
    group.finish();
}

// ====================================================================
// log_softmax
// ====================================================================

fn bench_log_softmax(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("log_softmax");
    for &size in &[64usize, 512, 4096] {
        let x = random_tensor(&[size], 7);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| backend.log_softmax(black_box(&x), -1).unwrap());
        });
    }
    group.finish();
}

// ====================================================================
// gelu
// ====================================================================

fn bench_gelu(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("gelu");
    for &size in &[128usize, 1024, 8192] {
        let x = random_tensor(&[size], 13);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| backend.gelu(black_box(&x)).unwrap());
        });
    }
    group.finish();
}

// ====================================================================
// dot
// ====================================================================

fn bench_dot(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("dot");
    for &size in &[128usize, 1024, 8192] {
        let a = random_tensor(&[size], 1);
        let b_t = random_tensor(&[size], 2);
        group.throughput(Throughput::Elements(size as u64));
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter(|| backend.dot(black_box(&a), black_box(&b_t)).unwrap());
        });
    }
    group.finish();
}

// ====================================================================
// matmul (square)
// ====================================================================

fn bench_matmul(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("matmul");
    group.sample_size(20); // matmul is slow naïvely — fewer samples
    for &n in &[32usize, 128, 256] {
        let a = random_tensor(&[n, n], 11);
        let b = random_tensor(&[n, n], 22);
        group.throughput(Throughput::Elements((n * n * n) as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bench, _| {
            bench.iter(|| backend.matmul(black_box(&a), black_box(&b)).unwrap());
        });
    }
    group.finish();
}

// ====================================================================
// matmul_fused_gelu (with bias)
// ====================================================================

fn bench_matmul_fused_gelu(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("matmul_fused_gelu");
    group.sample_size(20);
    for &n in &[32usize, 128, 256] {
        let a = random_tensor(&[n, n], 3);
        let b = random_tensor(&[n, n], 5);
        let bias = random_tensor(&[n], 7);
        group.throughput(Throughput::Elements((n * n * n) as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |bench, _| {
            bench.iter(|| {
                backend
                    .matmul_fused_gelu(
                        black_box(&a),
                        black_box(&b),
                        Some(black_box(&bias)),
                    )
                    .unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_softmax,
    bench_log_softmax,
    bench_gelu,
    bench_dot,
    bench_matmul,
    bench_matmul_fused_gelu
);
criterion_main!(benches);
