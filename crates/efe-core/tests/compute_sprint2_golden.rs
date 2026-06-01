//! **Audit-S2-bis — Sprint 2 golden oracle harness.**
//!
//! Pre-existing state (gap discovered during the post-S5 cross-sprint
//! audit): the **8 Sprint 2 JSON goldens** lived under
//! `tests/golden/{dot, gelu, log_softmax, matmul, matmul_fused_gelu,
//! softmax}/` but **no Rust test ever loaded them**. The golden
//! infrastructure was documentary only — exactly the bug the Sprint 5
//! cross-sprint audit fixed for Sprint 3, then Sprint 1, now closed
//! for Sprint 2.
//!
//! This file loads each S2 golden via `include_str!` + `serde_json`,
//! reconstructs the input tensors, dispatches through
//! [`ComputeBackend`], and verifies the output under the documented
//! `(rtol, atol)` tolerance. Editing any S2 golden now triggers a
//! detectable test failure.
#![allow(clippy::float_cmp)]
#![allow(clippy::excessive_precision)]

use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};
use serde::Deserialize;

// ====================================================================
// Schema helpers (shared with sprint1_golden, sprint3_golden)
// ====================================================================

#[derive(Debug, Deserialize)]
struct TensorJsonF32 {
    shape: Vec<usize>,
    data: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct Tolerance {
    rtol: f32,
    atol: f32,
}

fn assert_close(got: &[f32], expected: &[f32], tol: &Tolerance, ctx: &str) {
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

fn assert_close_scalar(got: f32, expected: f32, tol: &Tolerance, ctx: &str) {
    let bound = tol.atol + tol.rtol * expected.abs();
    assert!(
        (got - expected).abs() <= bound,
        "{ctx}: got {got}, expected {expected}, diff {} > bound {bound}",
        (got - expected).abs(),
    );
}

// ====================================================================
// dot — basic.json
// ====================================================================

#[test]
fn golden_dot_basic() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        a: TensorJsonF32,
        b: TensorJsonF32,
    }

    let raw = include_str!("golden/dot/basic.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let ta = Tensor::<f32>::from_slice(&spec.inputs.a.data, spec.inputs.a.shape.into()).unwrap();
    let tb = Tensor::<f32>::from_slice(&spec.inputs.b.data, spec.inputs.b.shape.into()).unwrap();
    let scalar = b.dot(&ta, &tb).unwrap();
    // The golden encodes the scalar as `data: [70.0]` shape=[].
    assert_eq!(spec.expected.data.len(), 1);
    assert_close_scalar(scalar, spec.expected.data[0], &spec.tolerance, "dot basic");
}

// ====================================================================
// gelu — reference_points.json
// ====================================================================

#[test]
fn golden_gelu_reference_points() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        x: TensorJsonF32,
    }

    let raw = include_str!("golden/gelu/reference_points.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let y = b.gelu(&x).unwrap();
    assert_eq!(y.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        y.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "gelu reference_points",
    );
}

// ====================================================================
// log_softmax — basic_1d.json
// ====================================================================

#[test]
fn golden_log_softmax_basic_1d() {
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

    let raw = include_str!("golden/log_softmax/basic_1d.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let y = b.log_softmax(&x, spec.inputs.axis).unwrap();
    assert_close(
        y.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "log_softmax basic_1d",
    );
}

// ====================================================================
// matmul — 2x2.json
// ====================================================================

#[test]
fn golden_matmul_2x2() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        a: TensorJsonF32,
        b: TensorJsonF32,
    }

    let raw = include_str!("golden/matmul/2x2.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let ta = Tensor::<f32>::from_slice(&spec.inputs.a.data, spec.inputs.a.shape.into()).unwrap();
    let tb = Tensor::<f32>::from_slice(&spec.inputs.b.data, spec.inputs.b.shape.into()).unwrap();
    let c = b.matmul(&ta, &tb).unwrap();
    assert_eq!(c.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        c.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "matmul 2x2",
    );
}

// ====================================================================
// matmul — 3x4_times_4x2.json
// ====================================================================

#[test]
fn golden_matmul_3x4_times_4x2() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        a: TensorJsonF32,
        b: TensorJsonF32,
    }

    let raw = include_str!("golden/matmul/3x4_times_4x2.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let ta = Tensor::<f32>::from_slice(&spec.inputs.a.data, spec.inputs.a.shape.into()).unwrap();
    let tb = Tensor::<f32>::from_slice(&spec.inputs.b.data, spec.inputs.b.shape.into()).unwrap();
    let c = b.matmul(&ta, &tb).unwrap();
    assert_eq!(c.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        c.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "matmul 3x4_times_4x2",
    );
}

// ====================================================================
// matmul_fused_gelu — with_bias.json
// ====================================================================

#[test]
fn golden_matmul_fused_gelu_with_bias() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonF32,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        a: TensorJsonF32,
        b: TensorJsonF32,
        bias: TensorJsonF32,
    }

    let raw = include_str!("golden/matmul_fused_gelu/with_bias.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let ta = Tensor::<f32>::from_slice(&spec.inputs.a.data, spec.inputs.a.shape.into()).unwrap();
    let tb = Tensor::<f32>::from_slice(&spec.inputs.b.data, spec.inputs.b.shape.into()).unwrap();
    let bias = Tensor::<f32>::from_slice(&spec.inputs.bias.data, spec.inputs.bias.shape.into()).unwrap();
    let c = b.matmul_fused_gelu(&ta, &tb, Some(&bias)).unwrap();
    assert_eq!(c.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        c.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "matmul_fused_gelu with_bias",
    );
}

// ====================================================================
// softmax — basic_1d.json
// ====================================================================

#[test]
fn golden_softmax_basic_1d() {
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

    let raw = include_str!("golden/softmax/basic_1d.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let y = b.softmax(&x, spec.inputs.axis).unwrap();
    assert_close(
        y.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "softmax basic_1d",
    );
}

// ====================================================================
// softmax — stable_large_values.json
// ====================================================================

#[test]
fn golden_softmax_stable_large_values() {
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

    let raw = include_str!("golden/softmax/stable_large_values.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let y = b.softmax(&x, spec.inputs.axis).unwrap();
    assert_close(
        y.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "softmax stable_large_values",
    );
}

// ====================================================================
// softmax — uniform_2d.json
// ====================================================================

#[test]
fn golden_softmax_uniform_2d() {
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

    let raw = include_str!("golden/softmax/uniform_2d.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let x = Tensor::<f32>::from_slice(&spec.inputs.x.data, spec.inputs.x.shape.into()).unwrap();
    let y = b.softmax(&x, spec.inputs.axis).unwrap();
    assert_eq!(y.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        y.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "softmax uniform_2d",
    );
}

// ====================================================================
// Meta — assert exhaustivity of S2 oracles
// ====================================================================

#[test]
fn golden_all_sprint2_files_parse_as_valid_json() {
    let files: &[(&str, &str)] = &[
        ("dot/basic.json", include_str!("golden/dot/basic.json")),
        (
            "gelu/reference_points.json",
            include_str!("golden/gelu/reference_points.json"),
        ),
        (
            "log_softmax/basic_1d.json",
            include_str!("golden/log_softmax/basic_1d.json"),
        ),
        ("matmul/2x2.json", include_str!("golden/matmul/2x2.json")),
        (
            "matmul/3x4_times_4x2.json",
            include_str!("golden/matmul/3x4_times_4x2.json"),
        ),
        (
            "matmul_fused_gelu/with_bias.json",
            include_str!("golden/matmul_fused_gelu/with_bias.json"),
        ),
        (
            "softmax/basic_1d.json",
            include_str!("golden/softmax/basic_1d.json"),
        ),
        (
            "softmax/stable_large_values.json",
            include_str!("golden/softmax/stable_large_values.json"),
        ),
        (
            "softmax/uniform_2d.json",
            include_str!("golden/softmax/uniform_2d.json"),
        ),
    ];
    assert_eq!(
        files.len(),
        9,
        "Sprint 2 advertises 8 ops (dot, gelu, log_softmax, matmul, matmul_fused_gelu, softmax, plus 2 extra matmul/softmax variants) — all 9 JSON files must exist and parse"
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
