//! Cross-backend equivalence tests for `cone_project` — the cornerstone
//! kernel of EFE alignment.
//!
//! Validates : if the latent is already inside the cone (distance
//! to nearest anchor ≤ max_dist), the kernel is **identity** ; if
//! outside, the latent is projected toward the nearest anchor.

#![cfg(feature = "cuda")]

use efe_core::compute::cpu::CpuBackend;
use efe_core::compute::{ComputeBackend, Shape, Tensor};

mod common;
use common::{assert_close, require_gpu};

#[test]
fn cross_cone_project_identity_when_inside() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    // 1 latent at (0.1, 0, 0), 1 anchor at (0, 0, 0), max_dist=1.0.
    // Distance = 0.1 < 1.0 → identity.
    let latents = Tensor::<f32>::from_slice(
        &[0.1, 0.0, 0.0],
        Shape::from_slice(&[1, 3]),
    )
    .unwrap();
    let anchors = Tensor::<f32>::from_slice(
        &[0.0, 0.0, 0.0],
        Shape::from_slice(&[1, 3]),
    )
    .unwrap();
    let cpu_out = cpu.cone_project(&latents, &anchors, 1.0).unwrap();
    let gpu_out = gpu.cone_project(&latents, &anchors, 1.0).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    // Should equal input.
    assert_close(gpu_out.as_slice().unwrap(), &[0.1, 0.0, 0.0], 1e-5, 1e-6);
}

#[test]
fn cross_cone_project_projection_when_outside() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    // Latent at (5, 0), anchor at (0, 0), max_dist = 1.
    // Distance = 5 > 1 → project to (1, 0).
    let latents = Tensor::<f32>::from_slice(
        &[5.0, 0.0],
        Shape::from_slice(&[1, 2]),
    )
    .unwrap();
    let anchors = Tensor::<f32>::from_slice(
        &[0.0, 0.0],
        Shape::from_slice(&[1, 2]),
    )
    .unwrap();
    let cpu_out = cpu.cone_project(&latents, &anchors, 1.0).unwrap();
    let gpu_out = gpu.cone_project(&latents, &anchors, 1.0).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-5, 1e-6);
    assert_close(gpu_out.as_slice().unwrap(), &[1.0, 0.0], 1e-4, 1e-5);
}

#[test]
fn cross_cone_project_picks_nearest_anchor() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    // Latent at (10, 10), 2 anchors : (0, 0) and (9, 9). Nearest = (9, 9).
    // Distance = √2 ≈ 1.414. With max_dist=0.5 we project toward (9, 9).
    let latents = Tensor::<f32>::from_slice(
        &[10.0, 10.0],
        Shape::from_slice(&[1, 2]),
    )
    .unwrap();
    let anchors = Tensor::<f32>::from_slice(
        &[0.0, 0.0, 9.0, 9.0],
        Shape::from_slice(&[2, 2]),
    )
    .unwrap();
    let cpu_out = cpu.cone_project(&latents, &anchors, 0.5).unwrap();
    let gpu_out = gpu.cone_project(&latents, &anchors, 0.5).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-4, 1e-5);
}

#[test]
fn cross_cone_project_batch_b32_d128() {
    let Some(gpu) = require_gpu() else { return; };
    let cpu = CpuBackend::new();

    // Pseudo-random fixture : 32 latents, 8 anchors, D=128.
    let b = 32;
    let n = 8;
    let d = 128;
    let mut lat_data = Vec::with_capacity(b * d);
    for i in 0..(b * d) {
        lat_data.push((i as f32 * 0.013).sin());
    }
    let mut anc_data = Vec::with_capacity(n * d);
    for i in 0..(n * d) {
        anc_data.push((i as f32 * 0.027).cos());
    }
    let latents =
        Tensor::<f32>::from_slice(&lat_data, Shape::from_slice(&[b, d])).unwrap();
    let anchors =
        Tensor::<f32>::from_slice(&anc_data, Shape::from_slice(&[n, d])).unwrap();

    let cpu_out = cpu.cone_project(&latents, &anchors, 0.4).unwrap();
    let gpu_out = gpu.cone_project(&latents, &anchors, 0.4).unwrap();
    assert_close(cpu_out.as_slice().unwrap(), gpu_out.as_slice().unwrap(), 1e-4, 1e-5);
}
