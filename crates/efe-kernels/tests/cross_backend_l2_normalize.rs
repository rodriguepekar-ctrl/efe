//! Cross-backend equivalence tests for `l2_normalize`.

#![cfg(feature = "cuda")]

use efe_core::compute::cpu::CpuBackend;
use efe_core::compute::{ComputeBackend, Shape, Tensor};

mod common;
use common::{assert_close, require_gpu};

#[test]
fn cross_l2_normalize_basic() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[3.0, 4.0],
        Shape::from_slice(&[1, 2]),
    )
    .unwrap();
    let cpu_out = cpu.l2_normalize(&x, -1, 1e-6).unwrap();
    let gpu_out = gpu.l2_normalize(&x, -1, 1e-6).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    // ||(3, 4)||₂ = 5 → normalized = (0.6, 0.8)
    assert_close(gpu_out.as_slice().unwrap(), &[0.6, 0.8], 1e-5, 1e-6);
}

#[test]
fn cross_l2_normalize_preserves_unit_norm() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let mut data = Vec::with_capacity(4 * 16);
    for i in 0..(4 * 16) {
        data.push(((i as f32) * 0.13).cos());
    }
    let x = Tensor::<f32>::from_slice(&data, Shape::from_slice(&[4, 16])).unwrap();
    let cpu_out = cpu.l2_normalize(&x, -1, 1e-6).unwrap();
    let gpu_out = gpu.l2_normalize(&x, -1, 1e-6).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    for row in 0..4 {
        let row_data = &gpu_out.as_slice().unwrap()[row * 16..(row + 1) * 16];
        let norm: f32 = row_data.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "row {row} norm = {norm}");
    }
}

#[test]
fn cross_l2_normalize_idempotence_property() {
    let Some(gpu) = require_gpu() else { return; };
    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0],
        Shape::from_slice(&[1, 4]),
    )
    .unwrap();
    let once = gpu.l2_normalize(&x, -1, 1e-6).unwrap();
    let twice = gpu.l2_normalize(&once, -1, 1e-6).unwrap();
    assert_close(
        once.as_slice().unwrap(),
        twice.as_slice().unwrap(),
        1e-5,
        1e-6,
    );
}
