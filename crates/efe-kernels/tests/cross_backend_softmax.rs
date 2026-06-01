//! Cross-backend equivalence tests for `softmax`.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 16.
//!
//! Every test asserts that the GPU output matches the CPU output
//! within the tolerance documented in `docs/compute/tolerances.md`
//! (rtol=1e-4, atol=1e-5 by default ; softmax is stable so we
//! tighten to rtol=1e-5).

#![cfg(feature = "cuda")]

use efe_core::compute::cpu::CpuBackend;
use efe_core::compute::{ComputeBackend, Shape, Tensor};
use efe_kernels::CudaBackend;

mod common;
use common::{assert_close, require_gpu};

#[test]
fn cross_softmax_basic_1d() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        Shape::from_slice(&[5]),
    )
    .unwrap();

    let cpu_out = cpu.softmax(&x, -1).unwrap();
    let gpu_out = gpu.softmax(&x, -1).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
}

#[test]
fn cross_softmax_stable_large_values() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    // Numerically tricky : naive softmax overflows here.
    let x = Tensor::<f32>::from_slice(
        &[1000.0, 1001.0, 1002.0],
        Shape::from_slice(&[3]),
    )
    .unwrap();

    let cpu_out = cpu.softmax(&x, -1).unwrap();
    let gpu_out = gpu.softmax(&x, -1).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);

    // Sanity : both sums to 1.
    let cpu_sum: f32 = cpu_out.as_slice().unwrap().iter().sum();
    let gpu_sum: f32 = gpu_out.as_slice().unwrap().iter().sum();
    assert!((cpu_sum - 1.0).abs() < 1e-5, "CPU sum {cpu_sum}");
    assert!((gpu_sum - 1.0).abs() < 1e-5, "GPU sum {gpu_sum}");
}

#[test]
fn cross_softmax_2d_rowwise() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    let mut data = Vec::with_capacity(4 * 8);
    for i in 0..4 {
        for j in 0..8 {
            data.push((i * 8 + j) as f32 * 0.1);
        }
    }
    let x = Tensor::<f32>::from_slice(&data, Shape::from_slice(&[4, 8])).unwrap();

    let cpu_out = cpu.softmax(&x, -1).unwrap();
    let gpu_out = gpu.softmax(&x, -1).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);

    // Each row sums to 1.
    for row_idx in 0..4 {
        let cpu_row = &cpu_out.as_slice().unwrap()[row_idx * 8..(row_idx + 1) * 8];
        let cpu_row_sum: f32 = cpu_row.iter().sum();
        assert!((cpu_row_sum - 1.0).abs() < 1e-5, "row {row_idx} sum {cpu_row_sum}");
    }
}

#[test]
fn cross_softmax_uniform_input_yields_uniform_output() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0; 10], Shape::from_slice(&[10])).unwrap();
    let cpu_out = cpu.softmax(&x, -1).unwrap();
    let gpu_out = gpu.softmax(&x, -1).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    // Each value should be 0.1.
    for &v in gpu_out.as_slice().unwrap() {
        assert!((v - 0.1).abs() < 1e-5);
    }
}
