//! **Audit 5 — Golden-vector oracle suite** for Sprint 3 operations.
//!
//! Prior to this file, the 10 JSON files under `tests/golden/<op>/*.json`
//! covering Sprint 3 ops were only *documentary*: the integration tests
//! hard-coded a copy of the inputs/expected values and merely mentioned
//! the golden path in a comment. Consequence: editing a golden JSON
//! would **not** cause any test to fail — the JSON was a lie waiting to
//! drift.
//!
//! This suite closes that gap. Each test :
//! 1. Loads the JSON file via `include_str!` + `serde_json`,
//! 2. Reconstructs the input tensors from the schema,
//! 3. Invokes the corresponding [`ComputeBackend`] method,
//! 4. Compares the output against `expected` under the documented
//!    `{rtol, atol}` tolerance.
//!
//! If anyone edits a golden JSON — whether by accident, by regeneration,
//! or because a dependency like `rand_pcg` changed — the change is now
//! caught by CI. This is the actual oracle contract the goldens always
//! claimed to provide.
//!
//! # Schema (shared with `sprint2_math.md` + `tests/golden/README.md`)
//!
//! ```text
//! {
//!   "op": "<operation_name>",
//!   "version": "1.0",
//!   "note": "how produced",
//!   "inputs": { named tensors + scalars },
//!   "expected": { tensor | {values, indices} },
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
struct TensorJson<T> {
    shape: Vec<usize>,
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
struct Tolerance {
    rtol: f32,
    atol: f32,
}

impl<T: Copy + Clone> TensorJson<T> {
    fn to_tensor_f32(&self) -> Tensor<f32>
    where
        T: Into<f64>,
    {
        let data_f32: Vec<f32> = self.data.iter().map(|&v| v.into() as f32).collect();
        Tensor::<f32>::from_slice(&data_f32, self.shape.clone().into()).unwrap()
    }
}

/// Assert two f32 slices are within `{rtol, atol}` tolerance.
fn assert_close(got: &[f32], expected: &[f32], tol: &Tolerance, ctx: &str) {
    assert_eq!(
        got.len(),
        expected.len(),
        "{ctx}: length mismatch ({} vs {})",
        got.len(),
        expected.len()
    );
    for (i, (&g, &e)) in got.iter().zip(expected.iter()).enumerate() {
        let diff = (g - e).abs();
        let bound = tol.atol + tol.rtol * e.abs();
        assert!(
            diff <= bound,
            "{ctx}[{i}]: got {g}, expected {e}, diff {diff} > {bound} (rtol={}, atol={})",
            tol.rtol,
            tol.atol
        );
    }
}

/// Assert two i32 slices are equal (used for exact-match goldens).
fn assert_eq_i32(got: &[i32], expected: &[i32], ctx: &str) {
    assert_eq!(
        got, expected,
        "{ctx}: exact-match golden drift detected (got {got:?}, expected {expected:?})"
    );
}

// ====================================================================
// cone_project — outside_cone_projected.json
// ====================================================================

#[test]
fn golden_cone_project_outside_cone_projected() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        latents: TensorJson<f32>,
        anchors: TensorJson<f32>,
        max_dist: f32,
    }

    let raw = include_str!("golden/cone_project/outside_cone_projected.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .cone_project(
            &spec.inputs.latents.to_tensor_f32(),
            &spec.inputs.anchors.to_tensor_f32(),
            spec.inputs.max_dist,
        )
        .unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "cone_project outside_cone_projected",
    );
}

// ====================================================================
// cone_project — inside_cone_passthrough.json (added audit 4)
// ====================================================================

#[test]
fn golden_cone_project_inside_cone_passthrough() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        latents: TensorJson<f32>,
        anchors: TensorJson<f32>,
        max_dist: f32,
    }

    let raw = include_str!("golden/cone_project/inside_cone_passthrough.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .cone_project(
            &spec.inputs.latents.to_tensor_f32(),
            &spec.inputs.anchors.to_tensor_f32(),
            spec.inputs.max_dist,
        )
        .unwrap();
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "cone_project inside_cone_passthrough",
    );
}

// ====================================================================
// weighted_mean — nonuniform_weights.json
// ====================================================================

#[test]
fn golden_weighted_mean_nonuniform_weights() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        values: TensorJson<f32>,
        weights: TensorJson<f32>,
        renormalize_l2: bool,
    }

    let raw = include_str!("golden/weighted_mean/nonuniform_weights.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .weighted_mean(
            &spec.inputs.values.to_tensor_f32(),
            &spec.inputs.weights.to_tensor_f32(),
            spec.inputs.renormalize_l2,
        )
        .unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "weighted_mean nonuniform_weights",
    );
}

// ====================================================================
// moe_reconcile — topk1_argmax.json
// ====================================================================

#[test]
fn golden_moe_reconcile_topk1_argmax() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        candidates: TensorJson<f32>,
        gate_logits: TensorJson<f32>,
        top_k: u32,
    }

    let raw = include_str!("golden/moe_reconcile/topk1_argmax.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .moe_reconcile(
            &spec.inputs.candidates.to_tensor_f32(),
            &spec.inputs.gate_logits.to_tensor_f32(),
            spec.inputs.top_k,
        )
        .unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "moe_reconcile topk1_argmax",
    );
}

// ====================================================================
// moe_reconcile — topk_full_softmax.json (added audit 4)
// ====================================================================

#[test]
fn golden_moe_reconcile_topk_full_softmax() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        candidates: TensorJson<f32>,
        gate_logits: TensorJson<f32>,
        top_k: u32,
    }

    let raw = include_str!("golden/moe_reconcile/topk_full_softmax.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .moe_reconcile(
            &spec.inputs.candidates.to_tensor_f32(),
            &spec.inputs.gate_logits.to_tensor_f32(),
            spec.inputs.top_k,
        )
        .unwrap();
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "moe_reconcile topk_full_softmax",
    );
}

// ====================================================================
// multinomial_sample — seed_42_reproducibility.json
// ====================================================================

#[test]
fn golden_multinomial_seed_42_reproducibility() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJsonI32,
        #[allow(dead_code)]
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        probs: TensorJson<f32>,
        n: u32,
        seed: u64,
    }
    #[derive(Deserialize)]
    struct TensorJsonI32 {
        shape: Vec<usize>,
        data: Vec<i32>,
    }

    let raw = include_str!("golden/multinomial_sample/seed_42_reproducibility.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .multinomial_sample(
            &spec.inputs.probs.to_tensor_f32(),
            spec.inputs.n,
            spec.inputs.seed,
        )
        .unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    // Exact-match: tolerance rtol=0, atol=0 per the reproducibility contract.
    assert_eq_i32(
        out.as_slice().unwrap(),
        &spec.expected.data,
        "multinomial seed=42 reproducibility",
    );
}

// ====================================================================
// kl_divergence — p_equals_q.json
// ====================================================================

#[test]
fn golden_kl_divergence_p_equals_q() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        p: TensorJson<f32>,
        q: TensorJson<f32>,
        axis: i32,
        eps: f32,
    }

    let raw = include_str!("golden/kl_divergence/p_equals_q.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .kl_divergence(
            &spec.inputs.p.to_tensor_f32(),
            &spec.inputs.q.to_tensor_f32(),
            spec.inputs.axis,
            spec.inputs.eps,
        )
        .unwrap();
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "kl_divergence p_equals_q",
    );
}

// ====================================================================
// std_dev — unbiased_numpy.json
// ====================================================================

#[test]
fn golden_std_dev_unbiased_numpy() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        x: TensorJson<f32>,
        axis: i32,
        unbiased: bool,
    }

    let raw = include_str!("golden/std_dev/unbiased_numpy.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .std_dev(
            &spec.inputs.x.to_tensor_f32(),
            spec.inputs.axis,
            spec.inputs.unbiased,
        )
        .unwrap();
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "std_dev unbiased_numpy",
    );
}

// ====================================================================
// topk — k3_of_5.json
// ====================================================================

#[test]
fn golden_topk_k3_of_5() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: Expected,
        #[allow(dead_code)]
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        x: TensorJson<f32>,
        k: u32,
        axis: i32,
    }
    #[derive(Deserialize)]
    struct Expected {
        values: TensorJson<f32>,
        indices: TensorJsonI32,
    }
    #[derive(Deserialize)]
    struct TensorJsonI32 {
        shape: Vec<usize>,
        data: Vec<i32>,
    }

    let raw = include_str!("golden/topk/k3_of_5.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let (vals, idx) = b
        .topk(
            &spec.inputs.x.to_tensor_f32(),
            spec.inputs.k,
            spec.inputs.axis,
        )
        .unwrap();
    assert_eq!(vals.shape().dims(), spec.expected.values.shape.as_slice());
    assert_eq!(idx.shape().dims(), spec.expected.indices.shape.as_slice());
    // Golden advertises rtol=0, atol=0 : topk tie-breaking must be exact.
    assert_eq!(vals.as_slice().unwrap(), &spec.expected.values.data[..]);
    assert_eq_i32(
        idx.as_slice().unwrap(),
        &spec.expected.indices.data,
        "topk k3_of_5 indices",
    );
}

// ====================================================================
// scaled_dot_product_attention — identity_uniform.json
// ====================================================================

#[test]
fn golden_sdpa_identity_uniform() {
    #[derive(Deserialize)]
    struct Spec {
        inputs: Inputs,
        expected: TensorJson<f32>,
        tolerance: Tolerance,
    }
    #[derive(Deserialize)]
    struct Inputs {
        q: TensorJson<f32>,
        k: TensorJson<f32>,
        v: TensorJson<f32>,
    }

    let raw = include_str!("golden/scaled_dot_product_attention/identity_uniform.json");
    let spec: Spec = serde_json::from_str(raw).unwrap();
    let b = CpuBackend::new();
    let out = b
        .scaled_dot_product_attention(
            &spec.inputs.q.to_tensor_f32(),
            &spec.inputs.k.to_tensor_f32(),
            &spec.inputs.v.to_tensor_f32(),
            None,
        )
        .unwrap();
    assert_eq!(out.shape().dims(), spec.expected.shape.as_slice());
    assert_close(
        out.as_slice().unwrap(),
        &spec.expected.data,
        &spec.tolerance,
        "sdpa identity_uniform",
    );
}

// ====================================================================
// Meta-test : the 10 golden JSON files under Sprint 3 categories
// actually parse as valid JSON. Serves as a canary if someone ships
// a malformed file that wouldn't be caught by the op-specific tests
// above (e.g. an unrelated field).
// ====================================================================

#[test]
fn golden_all_sprint3_files_parse_as_valid_json() {
    let files: &[(&str, &str)] = &[
        (
            "cone_project/outside_cone_projected.json",
            include_str!("golden/cone_project/outside_cone_projected.json"),
        ),
        (
            "cone_project/inside_cone_passthrough.json",
            include_str!("golden/cone_project/inside_cone_passthrough.json"),
        ),
        (
            "weighted_mean/nonuniform_weights.json",
            include_str!("golden/weighted_mean/nonuniform_weights.json"),
        ),
        (
            "moe_reconcile/topk1_argmax.json",
            include_str!("golden/moe_reconcile/topk1_argmax.json"),
        ),
        (
            "moe_reconcile/topk_full_softmax.json",
            include_str!("golden/moe_reconcile/topk_full_softmax.json"),
        ),
        (
            "multinomial_sample/seed_42_reproducibility.json",
            include_str!("golden/multinomial_sample/seed_42_reproducibility.json"),
        ),
        (
            "kl_divergence/p_equals_q.json",
            include_str!("golden/kl_divergence/p_equals_q.json"),
        ),
        (
            "std_dev/unbiased_numpy.json",
            include_str!("golden/std_dev/unbiased_numpy.json"),
        ),
        (
            "topk/k3_of_5.json",
            include_str!("golden/topk/k3_of_5.json"),
        ),
        (
            "scaled_dot_product_attention/identity_uniform.json",
            include_str!("golden/scaled_dot_product_attention/identity_uniform.json"),
        ),
    ];
    assert_eq!(
        files.len(),
        10,
        "Sprint 3 advertises exactly 10 golden vectors (see sprint3_math.md §11)"
    );
    for (name, raw) in files {
        let value: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_else(|e| panic!("golden {name}: invalid JSON — {e}"));
        // Every golden must declare its op, version, expected and tolerance.
        assert!(
            value.get("op").and_then(|v| v.as_str()).is_some(),
            "{name}: missing 'op' string"
        );
        assert!(
            value.get("version").is_some(),
            "{name}: missing 'version'"
        );
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
