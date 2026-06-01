//! Cross-backend equivalence tests for `mean` and `std_dev`.

#![cfg(feature = "cuda")]

use efe_core::compute::cpu::CpuBackend;
use efe_core::compute::{ComputeBackend, Shape, Tensor};

mod common;
use common::{assert_close, require_gpu};

#[test]
fn cross_mean_2d_rowwise() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        Shape::from_slice(&[2, 3]),
    )
    .unwrap();
    let cpu_out = cpu.mean(&x, -1).unwrap();
    let gpu_out = gpu.mean(&x, -1).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    assert_close(gpu_out.as_slice().unwrap(), &[2.0, 5.0], 1e-5, 1e-6);
}

#[test]
fn cross_std_dev_unbiased() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    // n=4, mean=2.5, variance unbiased = (sum (x-mean)²) / (n-1) = 5/3
    // std = sqrt(5/3) ≈ 1.291
    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0],
        Shape::from_slice(&[1, 4]),
    )
    .unwrap();
    let cpu_out = cpu.std_dev(&x, -1, true).unwrap();
    let gpu_out = gpu.std_dev(&x, -1, true).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-4, 1e-5);
    let expected = (5.0_f32 / 3.0).sqrt();
    assert!((gpu_out.as_slice().unwrap()[0] - expected).abs() < 1e-4);
}

#[test]
fn cross_std_dev_biased() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0],
        Shape::from_slice(&[1, 4]),
    )
    .unwrap();
    let cpu_out = cpu.std_dev(&x, -1, false).unwrap();
    let gpu_out = gpu.std_dev(&x, -1, false).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-4, 1e-5);
}

#[test]
fn cross_std_dev_uniform_yields_zero() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[7.0, 7.0, 7.0, 7.0, 7.0],
        Shape::from_slice(&[1, 5]),
    )
    .unwrap();
    let cpu_out = cpu.std_dev(&x, -1, true).unwrap();
    let gpu_out = gpu.std_dev(&x, -1, true).unwrap();
    assert!((cpu_out.as_slice().unwrap()[0]).abs() < 1e-5);
    assert!((gpu_out.as_slice().unwrap()[0]).abs() < 1e-5);
}
