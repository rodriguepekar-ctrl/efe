//! Cross-backend equivalence tests for `gelu` + `sigmoid`.

#![cfg(feature = "cuda")]

use efe_core::compute::cpu::CpuBackend;
use efe_core::compute::{ComputeBackend, Shape, Tensor};

mod common;
use common::{assert_close, require_gpu};

#[test]
fn cross_gelu_reference_points() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    // GELU(0) = 0, GELU(1) ≈ 0.8413, GELU(-1) ≈ -0.1587
    let x = Tensor::<f32>::from_slice(
        &[-2.0, -1.0, 0.0, 1.0, 2.0],
        Shape::from_slice(&[5]),
    )
    .unwrap();
    let cpu_out = cpu.gelu(&x).unwrap();
    let gpu_out = gpu.gelu(&x).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    // Sanity check on reference values.
    let v = gpu_out.as_slice().unwrap();
    assert!((v[2]).abs() < 1e-6, "GELU(0) should be 0, got {}", v[2]);
}

#[test]
fn cross_sigmoid_basic_range() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[-100.0, -1.0, 0.0, 1.0, 100.0],
        Shape::from_slice(&[5]),
    )
    .unwrap();
    let cpu_out = cpu.sigmoid(&x).unwrap();
    let gpu_out = gpu.sigmoid(&x).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    // Sanity check : sigmoid(-100) ≈ 0, sigmoid(0) = 0.5, sigmoid(100) ≈ 1.
    let v = gpu_out.as_slice().unwrap();
    assert!(v[0] < 1e-3, "sigmoid(-100) ≈ 0 ; got {}", v[0]);
    assert!((v[2] - 0.5).abs() < 1e-6, "sigmoid(0) = 0.5 ; got {}", v[2]);
    assert!(v[4] > 1.0 - 1e-3, "sigmoid(100) ≈ 1 ; got {}", v[4]);
}

#[test]
fn cross_sigmoid_no_overflow_on_extremes() {
    let Some(gpu) = require_gpu() else { return; };
    // Values that would overflow naive `1 / (1 + exp(-x))`.
    let x = Tensor::<f32>::from_slice(
        &[-1000.0, 1000.0],
        Shape::from_slice(&[2]),
    )
    .unwrap();
    let gpu_out = gpu.sigmoid(&x).unwrap();
    for &v in gpu_out.as_slice().unwrap() {
        assert!(v.is_finite(), "sigmoid produced non-finite : {v}");
    }
}
