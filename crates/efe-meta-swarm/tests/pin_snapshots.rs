//! Sprint A1 P1 — PR #3 — Category 14 : Pin snapshot tests (10 tests).
//!
//! Snapshot tests via `insta` :  each test asserts that the serialised
//! representation of a specific public type matches a committed
//! `.snap` file under `tests/snapshots/`.  Drift in the on-the-wire
//! format causes the test to fail, which forces a human review.
//!
//! Workflow :
//!   * First run :  `cargo test` produces `.snap.new` files.
//!   * Review    :  `cargo insta review` to accept or reject.
//!   * Then     :  `cargo insta accept` persists the snapshot.
//!
//! These tests are deliberately small (no proptest cases) — they
//! verify *format stability*, not statistical properties.

use efe_meta_swarm::prelude::*;
use efe_meta_swarm::certification::BundleCertificate;
use efe_meta_swarm::iteration::IterationTrace;

/// Cat 14 / Test 1 — Tensor JSON snapshot (8 dim, deterministic).
#[test]
fn tensor_json_snapshot_8dim_seed42() {
    let t = Tensor::pseudo_random(8, 42);
    let s = serde_json::to_string_pretty(&t).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 2 — Tensor zero snapshot.
#[test]
fn tensor_json_snapshot_zeros_4() {
    let t = Tensor::zeros(4);
    let s = serde_json::to_string_pretty(&t).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 3 — Tensor `from_vec` round-trip stable.
#[test]
fn tensor_from_vec_json_snapshot() {
    let t = Tensor::from_vec(vec![1.5, -0.25, 2.0]);
    let s = serde_json::to_string_pretty(&t).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 4 — `MetaSwarmConfig` minimal YAML deserialisation
/// round-trip stable.
#[test]
fn meta_swarm_config_yaml_snapshot() {
    let yaml = r#"
id: "snapshot-test"
meta_space:
  dim: 64
coupling:
  gamma: 0.2
  alpha_default: 0.5
  max_iters: 500
  tolerance: 0.001
participants:
  - id: "a"
    dim: 128
  - id: "b"
    dim: 128
meta_operator:
  kind: uniform_weighted
"#;
    let cfg: MetaSwarmConfig = serde_yaml::from_str(yaml).unwrap();
    // Re-serialise to verify deterministic round-trip.
    let s = serde_yaml::to_string(&cfg).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 5 — Attention-softmax variant of `MetaOperatorConfig`.
#[test]
fn meta_operator_attention_yaml_snapshot() {
    let cfg = efe_meta_swarm::config::MetaOperatorConfig::AttentionSoftmax {
        temperature: 100.0,
    };
    let s = serde_yaml::to_string(&cfg).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 6 — `IterationTrace` JSON snapshot (2-step empty).
#[test]
fn iteration_trace_json_snapshot() {
    let mut t = IterationTrace::new();
    let mc = Tensor::from_vec(vec![0.5, -0.5, 0.0]);
    let ps = vec![
        Tensor::from_vec(vec![1.0, 0.0]),
        Tensor::from_vec(vec![0.0, 1.0]),
    ];
    t.record_step(0, 1.0, &mc, &ps);
    t.record_step(1, 0.5, &mc, &ps);
    let s = serde_json::to_string_pretty(&t).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 7 — `BundleCertificate::Stub` JSON snapshot.
#[test]
fn bundle_certificate_stub_json_snapshot() {
    let cert = BundleCertificate::Stub;
    let s = serde_json::to_string_pretty(&cert).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 8 — `BundleCertificate::LocalSignature` JSON snapshot.
#[test]
fn bundle_certificate_local_signature_json_snapshot() {
    let cert = BundleCertificate::LocalSignature {
        bundle_sha256: vec![0xAB; 32],
        l_phi: 0.85,
        l_pi:  0.5,
        l_rho: 0.5,
        signature: vec![0xCD; 64],
    };
    let s = serde_json::to_string_pretty(&cert).unwrap();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 9 — `MetaSwarmError::DimensionMismatch` Display.
#[test]
fn error_dimension_mismatch_display_snapshot() {
    let e = MetaSwarmError::DimensionMismatch { expected: 384, actual: 512 };
    let s = e.to_string();
    insta::assert_snapshot!(s);
}

/// Cat 14 / Test 10 — `MetaSwarmError::InvalidCoupling` Display.
#[test]
fn error_invalid_coupling_display_snapshot() {
    let e = MetaSwarmError::InvalidCoupling { alpha: 0.7, gamma: 0.5 };
    let s = e.to_string();
    insta::assert_snapshot!(s);
}
