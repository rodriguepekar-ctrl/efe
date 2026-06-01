//! Cross-backend equivalence tests for `argmax` and `shannon_entropy`.

#![cfg(feature = "cuda")]

use efe_core::compute::cpu::CpuBackend;
use efe_core::compute::{ComputeBackend, Shape, Tensor};

mod common;
use common::{assert_close, require_gpu};

#[test]
fn cross_argmax_basic() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[1.0, 5.0, 2.0, 4.0, 3.0],
        Shape::from_slice(&[1, 5]),
    )
    .unwrap();
    let cpu_out = cpu.argmax(&x, -1).unwrap();
    let gpu_out = gpu.argmax(&x, -1).unwrap();
    assert_eq!(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap());
    assert_eq!(gpu_out.as_slice().unwrap(), &[1]);
}

#[test]
fn cross_argmax_tie_break_lowest_index() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[5.0, 3.0, 5.0, 4.0, 5.0],
        Shape::from_slice(&[1, 5]),
    )
    .unwrap();
    let cpu_out = cpu.argmax(&x, -1).unwrap();
    let gpu_out = gpu.argmax(&x, -1).unwrap();
    assert_eq!(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap());
    assert_eq!(gpu_out.as_slice().unwrap(), &[0], "tie should yield lowest index");
}

#[test]
fn cross_shannon_entropy_uniform() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    // Uniform distribution over 4 outcomes : H = log 4 ≈ 1.386
    let x = Tensor::<f32>::from_slice(
        &[0.25, 0.25, 0.25, 0.25],
        Shape::from_slice(&[1, 4]),
    )
    .unwrap();
    let cpu_out = cpu.shannon_entropy(&x, -1).unwrap();
    let gpu_out = gpu.shannon_entropy(&x, -1).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-4, 1e-5);
    let expected = (4.0_f32).ln();
    assert!((gpu_out.as_slice().unwrap()[0] - expected).abs() < 1e-4);
}

#[test]
fn cross_shannon_entropy_dirac() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    // Dirac distribution : H = 0
    let x = Tensor::<f32>::from_slice(
        &[1.0, 0.0, 0.0, 0.0],
        Shape::from_slice(&[1, 4]),
    )
    .unwrap();
    let cpu_out = cpu.shannon_entropy(&x, -1).unwrap();
    let gpu_out = gpu.shannon_entropy(&x, -1).unwrap();
    assert!((cpu_out.as_slice().unwrap()[0]).abs() < 1e-4);
    assert!((gpu_out.as_slice().unwrap()[0]).abs() < 1e-4);
}
