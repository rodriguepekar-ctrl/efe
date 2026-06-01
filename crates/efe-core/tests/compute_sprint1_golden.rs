//! **Audit-S1-bis — Sprint 1 golden oracle harness.**
//!
//! Pre-existing state (gap discovered during the post-S5 cross-sprint
//! audit): the 3 Sprint 1 JSON goldens
//!
//! - `tests/golden/l2_normalize/basic_1d.json`
//! - `tests/golden/mean/basic_2d.json`
//! - `tests/golden/argmax/basic_2d.json`
//!
//! were **purely documentary**: no Rust test ever loaded them. Editing
//! a golden left every test green — exactly the bug the Sprint 5
//! cross-sprint audit fixed for Sprint 3, now closed for Sprint 1.
//!
//! This file additionally introduces the missing
//! `tests/golden/l2_norm/basic.json` (the canonical Sprint 1 L2 norm
//! oracle). All 4 Sprint 1 ops (`l2_norm`, `l2_normalize`, `mean`,
//! `argmax`) now have at least one JSON oracle that is **actually
//! loaded**, parsed, and compared against [`ComputeBackend`] output
//! under the documented `(rtol, atol)` tolerance.
//!
//! ## Schema (shared with `tests/golden/README.md`)
//!
//! ```text
//! {
//!   "op": "<operation_name>",
//!   "version": "1.0",
//!   "note": "...",
//!   "inputs": { ... },
//!   "expected": { "shape": [...], "data": [...] },
//!   "tolerance": { "rtol": <f32>, "atol": <f32> }
//! }
//! ```
#![allow(clippy::float_cmp)]
#![allow(clippy::excessive_precision)]

use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};
use serde::Deserialize;

// ====================================================================
// Schema helpers
// ====================================================================

#[derive(Debug, Deserialize)]
struct TensorJsonF32 {
    shape: Vec<usize>,
    data: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct TensorJsonI32 {
    shape: Vec<usize>,
    data: Vec<i32>,
}

#[derive(Debug, Deserialize)]
struct Tolerance {
    rtol: f32,
    atol: f32,
}

fn assert_close_f32(got: &[f32], expected: &[f32], tol: &Tolerance, ctx: &str) {
    assert_eq!(got.len(), expected.len(), "{ctx}: length mismatch");
    for (i, (&g, &e)) in got.iter().zip(expected.iter()).enumerate() {
        let bound = tol.atol + tol.rtol * e.abs();
        assert!(
            (g - e).abs() <= bound,
            "{ctx}[{i}]: got {g}, expected {e}, diff {} > bound {bound}",
            (g - e).abs(),
        );
    }
}

fn assert_eq_i32(got: &[i32], expected: &[i32], ctx: &str) {
    assert_eq!(
        got, expected,
        "{ctx}: exact-match golden drift detected (got {got:?}, expected {expected:?})"
    );
}

// ====================================================================
// l2_norm — basic.json (NEW oracle, audit-S1-bis)
// ====================================================================

#[test]
fn golden_l2_norm_basic() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        x: TensorJsonF32,
        axis: i32,
    }

    let raw = include_str!("golden/l2_norm/basic.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let out = b.l2_norm(&x, spec.inputs.axis).unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    assert_close_f32(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "l2_norm basic",
    );
}

// ====================================================================
// l2_normalize — basic_1d.json
// ====================================================================

#[test]
fn golden_l2_normalize_basic_1d() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        x: TensorJsonF32,
        axis: i32,
        eps: f32,
    }

    let raw = include_str!("golden/l2_normalize/basic_1d.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let out = b
        .l2_normalize(&x, spec.inputs.axis, spec.inputs.eps)
        .unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    assert_close_f32(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "l2_normalize basic_1d",
    );
}

// ====================================================================
// mean — basic_2d.json
// ====================================================================

#[test]
fn golden_mean_basic_2d() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        x: TensorJsonF32,
        axis: i32,
    }

    let raw = include_str!("golden/mean/basic_2d.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let out = b.mean(&x, spec.inputs.axis).unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    assert_close_f32(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "mean basic_2d",
    );
}

// ====================================================================
// argmax — basic_2d.json
// ====================================================================

#[test]
fn golden_argmax_basic_2d() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonI32,
        #[allow(dead_code)]
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        x: TensorJsonF32,
        axis: i32,
    }

    let raw = include_str!("golden/argmax/basic_2d.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let out = b.argmax(&x, spec.inputs.axis).unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    // argmax tolerance is documented as rtol=0, atol=0 — exact match.
    assert_eq_i32(
        out.as_slice().unwrap(),
        &spec.expected.data,
        "argmax basic_2d",
    );
}

// ====================================================================
// Meta — assert exhaustivity of S1 oracles
// ====================================================================

#[test]
fn golden_all_sprint1_files_parse_as_valid_json() {
    let files: &[(&str, &str)] = &[
        ("l2_norm/basic.json", include_str!("golden/l2_norm/basic.json")),
        (
            "l2_normalize/basic_1d.json",
            include_str!("golden/l2_normalize/basic_1d.json"),
        ),
        ("mean/basic_2d.json", include_str!("golden/mean/basic_2d.json")),
        ("argmax/basic_2d.json", include_str!("golden/argmax/basic_2d.json")),
    ];
    assert_eq!(
        files.len(),
        4,
        "Sprint 1 advertises exactly 4 ops (l2_norm, l2_normalize, mean, argmax) — every one of them must have a golden oracle"
    );
    for (name, raw) in files {
        let value: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_else(|e| panic!("golden {name}: invalid JSON — {e}"));
        assert!(
            value.get("op").and_then(|v| v.as_str()).is_some(),
            "{name}: missing 'op' string"
        );
        assert!(value.get("version").is_some(), "{name}: missing 'version'");
        assert!(
            value.get("expected").is_some(),
            "{name}: missing 'expected' block"
        );
        assert!(
            value.get("tolerance").is_some(),
            "{name}: missing 'tolerance' block"
        );
    }
}
