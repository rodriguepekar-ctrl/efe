//! Cross-backend equivalence tests for `weighted_mean`.

#![cfg(feature = "cuda")]

use efe_core::compute::cpu::CpuBackend;
use efe_core::compute::{ComputeBackend, Shape, Tensor};

mod common;
use common::{assert_close, require_gpu};

#[test]
fn cross_weighted_mean_uniform_weights() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    // Uniform weights → simple mean.
    // values = [[1,2,3], [4,5,6], [7,8,9]], weights = [1,1,1]
    // → out = [4, 5, 6] (column means).
    let values = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        Shape::from_slice(&[3, 3]),
    )
    .unwrap();
    let weights =
        Tensor::<f32>::from_slice(&[1.0, 1.0, 1.0], Shape::from_slice(&[3])).unwrap();

    let cpu_out = cpu.weighted_mean(&values, &weights, false).unwrap();
    let gpu_out = gpu.weighted_mean(&values, &weights, false).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    assert_close(gpu_out.as_slice().unwrap(), &[4.0, 5.0, 6.0], 1e-4, 1e-5);
}

#[test]
fn cross_weighted_mean_skewed_weights() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    // weights [1, 0, 0] → out = [1, 2, 3]
    let values = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0],
        Shape::from_slice(&[3, 3]),
    )
    .unwrap();
    let weights =
        Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0], Shape::from_slice(&[3])).unwrap();
    let cpu_out = cpu.weighted_mean(&values, &weights, false).unwrap();
    let gpu_out = gpu.weighted_mean(&values, &weights, false).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
}

#[test]
fn cross_weighted_mean_with_l2_renorm() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let values = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        Shape::from_slice(&[2, 3]),
    )
    .unwrap();
    let weights = Tensor::<f32>::from_slice(&[0.7, 0.3], Shape::from_slice(&[2])).unwrap();

    let cpu_out = cpu.weighted_mean(&values, &weights, true).unwrap();
    let gpu_out = gpu.weighted_mean(&values, &weights, true).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-4, 1e-5);

    // L2 norm should be ≈ 1.
    let l2: f32 = gpu_out.as_slice().unwrap().iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!((l2 - 1.0).abs() < 1e-4, "L2 norm of GPU out = {l2}");
}

#[test]
fn cross_weighted_mean_large_n_d() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let n = 256;
    let d = 512;
    let mut vals = Vec::with_capacity(n * d);
    for i in 0..(n * d) {
        vals.push(((i as f32) * 0.001).sin());
    }
    let mut w = Vec::with_capacity(n);
    for i in 0..n {
        w.push(0.5 + 0.5 * ((i as f32) * 0.1).sin());
    }
    let values = Tensor::<f32>::from_slice(&vals, Shape::from_slice(&[n, d])).unwrap();
    let weights = Tensor::<f32>::from_slice(&w, Shape::from_slice(&[n])).unwrap();
    let cpu_out = cpu.weighted_mean(&values, &weights, false).unwrap();
    let gpu_out = gpu.weighted_mean(&values, &weights, false).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-4, 1e-5);
}
