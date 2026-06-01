//! Criterion benchmarks for Sprint 3 — the 8 consensus / attention /
//! sampling ops, including the **3 core EFE kernels**.
//!
//! Run with :
//! ```bash
//! cargo bench -p efe-core --bench compute_sprint3 -- --quick
//! ```
#![allow(clippy::cast_precision_loss)]

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};

fn xorshift_vec(n: usize, seed: u64) -> Vec<f32> {
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
    let data = xorshift_vec(n, seed);
    Tensor::<f32>::from_slice(&data, shape.to_vec().into()).unwrap()
}

fn positive_tensor(shape: &[usize], seed: u64) -> Tensor<f32> {
    let n: usize = shape.iter().product();
    let data: Vec<f32> = xorshift_vec(n, seed).into_iter().map(|v| v.abs() + 0.01).collect();
    Tensor::<f32>::from_slice(&data, shape.to_vec().into()).unwrap()
}

// ====================================================================
// cone_project — CORE EFE
// ====================================================================

fn bench_cone_project(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("cone_project");
    for &(batch, n_anchors, d) in &[(32_usize, 10, 128), (32, 100, 128), (128, 10, 256)] {
        let latents = random_tensor(&[batch, d], 11);
        let anchors = random_tensor(&[n_anchors, d], 22);
        group.throughput(Throughput::Elements((batch * n_anchors * d) as u64));
        group.bench_with_input(
            BenchmarkId::new("B_N_d", format!("{batch}x{n_anchors}x{d}")),
            &(batch, n_anchors, d),
            |b, _| {
                b.iter(|| {
                    backend.cone_project(
                        black_box(&latents),
                        black_box(&anchors),
                        0.5,
                    ).unwrap()
                })
            },
        );
    }
    group.finish();
}

// ====================================================================
// weighted_mean — CORE EFE
// ====================================================================

fn bench_weighted_mean(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("weighted_mean");
    for &(batch, d) in &[(16_usize, 64), (256, 128), (4096, 128)] {
        let values = random_tensor(&[batch, d], 3);
        let weights = positive_tensor(&[batch], 5);
        group.throughput(Throughput::Elements((batch * d) as u64));
        group.bench_with_input(
            BenchmarkId::new("B_d", format!("{batch}x{d}")),
            &(batch, d),
            |b, _| {
                b.iter(|| {
                    backend.weighted_mean(
                        black_box(&values),
                        black_box(&weights),
                        false,
                    ).unwrap()
                })
            },
        );
    }
    group.finish();
}

// ====================================================================
// moe_reconcile — CORE EFE
// ====================================================================

fn bench_moe_reconcile(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("moe_reconcile");
    for &(batch, k, d) in &[(32_usize, 4, 128), (32, 8, 128), (128, 16, 256)] {
        let candidates = random_tensor(&[batch, k, d], 7);
        let gate = random_tensor(&[batch, k], 9);
        group.throughput(Throughput::Elements((batch * k * d) as u64));
        group.bench_with_input(
            BenchmarkId::new("B_K_d", format!("{batch}x{k}x{d}")),
            &(batch, k, d),
            |b, _| {
                b.iter(|| {
                    backend.moe_reconcile(
                        black_box(&candidates),
                        black_box(&gate),
                        2,
                    ).unwrap()
                })
            },
        );
    }
    group.finish();
}

// ====================================================================
// topk
// ====================================================================

fn bench_topk(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("topk");
    for &n in &[128_usize, 1024, 8192] {
        let x = random_tensor(&[n], 13);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| backend.topk(black_box(&x), 5, -1).unwrap())
        });
    }
    group.finish();
}

// ====================================================================
// scaled_dot_product_attention
// ====================================================================

fn bench_sdpa(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("sdpa");
    group.sample_size(20);
    for &t in &[16_usize, 32, 64] {
        let d_k = 64;
        let q = random_tensor(&[1, 1, t, d_k], 17);
        let k = random_tensor(&[1, 1, t, d_k], 19);
        let v = random_tensor(&[1, 1, t, d_k], 23);
        group.throughput(Throughput::Elements((t * t * d_k) as u64));
        group.bench_with_input(BenchmarkId::from_parameter(t), &t, |b, _| {
            b.iter(|| {
                backend
                    .scaled_dot_product_attention(
                        black_box(&q),
                        black_box(&k),
                        black_box(&v),
                        None,
                    )
                    .unwrap()
            })
        });
    }
    group.finish();
}

// ====================================================================
// multinomial_sample
// ====================================================================

fn bench_multinomial_sample(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("multinomial_sample");
    for &n in &[10u32, 100, 1000] {
        let probs = positive_tensor(&[32, 16], 29);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| backend.multinomial_sample(black_box(&probs), n, 42).unwrap())
        });
    }
    group.finish();
}

// ====================================================================
// kl_divergence
// ====================================================================

fn bench_kl_divergence(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("kl_divergence");
    for &k in &[32_usize, 256, 2048] {
        let p = positive_tensor(&[32, k], 31);
        let q = positive_tensor(&[32, k], 37);
        group.throughput(Throughput::Elements((32 * k) as u64));
        group.bench_with_input(BenchmarkId::from_parameter(k), &k, |b, _| {
            b.iter(|| backend.kl_divergence(black_box(&p), black_box(&q), -1, 1e-9).unwrap())
        });
    }
    group.finish();
}

// ====================================================================
// std_dev
// ====================================================================

fn bench_std_dev(c: &mut Criterion) {
    let backend = CpuBackend::new();
    let mut group = c.benchmark_group("std_dev");
    for &n in &[128_usize, 1024, 8192] {
        let x = random_tensor(&[n], 41);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| backend.std_dev(black_box(&x), -1, true).unwrap())
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_cone_project,
    bench_weighted_mean,
    bench_moe_reconcile,
    bench_topk,
    bench_sdpa,
    bench_multinomial_sample,
    bench_kl_divergence,
    bench_std_dev,
);
criterion_main!(benches);
