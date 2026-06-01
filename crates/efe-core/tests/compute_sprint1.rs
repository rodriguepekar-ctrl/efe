//! Sprint 1 integration tests for the `compute` module.
//!
//! These tests exercise the CPU backend end-to-end through the
//! [`ComputeBackend`] trait (the public surface), not through the private
//! helpers.
#![allow(clippy::float_cmp)]

use efe_core::compute::{ComputeBackend, ComputeError, CpuBackend, Device, Shape, Tensor};

// ------------------------------------------------------------------
// Backend metadata
// ------------------------------------------------------------------

#[test]
fn cpu_backend_device_and_name() {
    let b = CpuBackend::new();
    assert_eq!(b.device(), Device::Cpu);
    assert_eq!(b.name(), "cpu");
}

#[test]
fn cpu_backend_synchronize_is_noop() {
    let b = CpuBackend::new();
    assert!(b.synchronize().is_ok());
}

// ------------------------------------------------------------------
// l2_norm — Sprint 1
// ------------------------------------------------------------------

#[test]
fn l2_norm_simple_1d() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[3.0, 4.0], [2].into()).unwrap();
    let n = b.l2_norm(&x, -1).unwrap();
    assert_eq!(n.rank(), 0);
    assert!((n.as_slice().unwrap()[0] - 5.0).abs() < 1e-5);
}

#[test]
fn l2_norm_batched_2d() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[3.0, 4.0, 6.0, 8.0, 0.0, 0.0],
        [3, 2].into(),
    )
    .unwrap();
    let n = b.l2_norm(&x, -1).unwrap();
    let out = n.as_slice().unwrap();
    assert!((out[0] - 5.0).abs() < 1e-5);
    assert!((out[1] - 10.0).abs() < 1e-5);
    assert!(out[2].abs() < 1e-5);
}

// ------------------------------------------------------------------
// l2_normalize — Sprint 1
// ------------------------------------------------------------------

#[test]
fn l2_normalize_unit_vector_preserved() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 0.0, 0.0], [3].into()).unwrap();
    let y = b.l2_normalize(&x, -1, 1e-9).unwrap();
    let s = y.as_slice().unwrap();
    assert!((s[0] - 1.0).abs() < 1e-5);
    assert!(s[1].abs() < 1e-5);
    assert!(s[2].abs() < 1e-5);
}

#[test]
fn l2_normalize_doubles_back_to_unit() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[2.0, 2.0, 2.0, 2.0], [4].into()).unwrap();
    let y = b.l2_normalize(&x, -1, 1e-9).unwrap();
    let norm_sq: f32 = y.as_slice().unwrap().iter().map(|v| v * v).sum();
    assert!((norm_sq - 1.0).abs() < 1e-5);
}

#[test]
fn l2_normalize_batched() {
    let b = CpuBackend::new();
    // Two rows : [3,4,0] → norm 5, [0,0,1] → norm 1
    let x = Tensor::<f32>::from_slice(
        &[3.0, 4.0, 0.0, 0.0, 0.0, 1.0],
        [2, 3].into(),
    )
    .unwrap();
    let y = b.l2_normalize(&x, -1, 1e-9).unwrap();
    let s = y.as_slice().unwrap();
    // Row 0 : 3/5, 4/5, 0
    assert!((s[0] - 0.6).abs() < 1e-5);
    assert!((s[1] - 0.8).abs() < 1e-5);
    assert!(s[2].abs() < 1e-5);
    // Row 1 : 0, 0, 1
    assert!(s[3].abs() < 1e-5);
    assert!(s[4].abs() < 1e-5);
    assert!((s[5] - 1.0).abs() < 1e-5);
}

// ------------------------------------------------------------------
// mean — Sprint 1
// ------------------------------------------------------------------

#[test]
fn mean_1d() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], [5].into()).unwrap();
    let m = b.mean(&x, -1).unwrap();
    assert_eq!(m.rank(), 0);
    assert!((m.as_slice().unwrap()[0] - 3.0).abs() < 1e-5);
}

#[test]
fn mean_2d_axis_last() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        [2, 3].into(),
    )
    .unwrap();
    let m = b.mean(&x, -1).unwrap();
    let s = m.as_slice().unwrap();
    assert!((s[0] - 2.0).abs() < 1e-5);
    assert!((s[1] - 5.0).abs() < 1e-5);
}

#[test]
fn mean_2d_axis_first() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
        [2, 3].into(),
    )
    .unwrap();
    let m = b.mean(&x, 0).unwrap();
    let s = m.as_slice().unwrap();
    assert!((s[0] - 2.5).abs() < 1e-5);
    assert!((s[1] - 3.5).abs() < 1e-5);
    assert!((s[2] - 4.5).abs() < 1e-5);
}

// ------------------------------------------------------------------
// argmax — Sprint 1
// ------------------------------------------------------------------

#[test]
fn argmax_1d() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&[-1.0, 3.0, 2.0, -5.0, 4.0], [5].into()).unwrap();
    let idx = b.argmax(&x, -1).unwrap();
    assert_eq!(idx.as_slice().unwrap(), &[4]);
}

#[test]
fn argmax_batch() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(
        &[0.1, 0.5, 0.4, 0.8, 0.1, 0.1, 0.0, 0.0, 0.0],
        [3, 3].into(),
    )
    .unwrap();
    let idx = b.argmax(&x, -1).unwrap();
    assert_eq!(idx.as_slice().unwrap(), &[1, 0, 0]);
}

// ------------------------------------------------------------------
// Note: all 18 operations of the ComputeBackend trait are now implemented
// on CpuBackend (Sprint 1 + Sprint 2 + Sprint 3). No more NotImplemented
// regression tests live here — the complete coverage is in
// `compute_sprint1.rs`, `compute_sprint2.rs`, `compute_sprint3.rs`.
// ------------------------------------------------------------------

// ------------------------------------------------------------------
// Error propagation for invalid axis
// ------------------------------------------------------------------

#[test]
fn l2_norm_rejects_bad_axis() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::zeros([3].into()).unwrap();
    let err = b.l2_norm(&x, 5).unwrap_err();
    assert!(matches!(err, ComputeError::InvalidAxis { .. }));
}

#[test]
fn mean_rejects_bad_axis() {
    let b = CpuBackend::new();
    let x = Tensor::<f32>::zeros([3].into()).unwrap();
    let err = b.mean(&x, -5).unwrap_err();
    assert!(matches!(err, ComputeError::InvalidAxis { .. }));
}

// ------------------------------------------------------------------
// Shape arithmetic sanity
// ------------------------------------------------------------------

#[test]
fn shape_numel_consistency() {
    let s: Shape = [2, 3, 4].into();
    assert_eq!(s.numel(), 24);
    let s0: Shape = Shape::scalar();
    assert_eq!(s0.numel(), 1);
}
