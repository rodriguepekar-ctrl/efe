//! Property-based tests for Sprint 3 — the three CORE EFE ops + 5 others.
//!
//! Generates ~100 random cases per property via proptest. Verifies
//! algebraic invariants that should hold for any well-formed input.
#![allow(clippy::float_cmp)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::cast_precision_loss)]

use efe_core::compute::{ComputeBackend, CpuBackend, Tensor};
use proptest::prelude::*;

/// Generate rank-1 tensor data with finite values.
fn finite_vec(min_len: usize, max_len: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(-10.0f32..10.0, min_len..=max_len)
}

/// Generate a positive probability vector (sum normalized by caller).
fn positive_probs(n: usize) -> impl Strategy<Value = Vec<f32>> {
    prop::collection::vec(0.01f32..1.0, n..=n)
}

// ==================================================================
// cone_project — 3 properties
// ==================================================================

proptest! {
    /// If all latents are already inside the cone (within max_dist of
    /// their nearest anchor), cone_project leaves them unchanged.
    #[test]
    fn cone_project_inside_cone_passes_through(
        vals in finite_vec(4, 16),
    ) {
        let b = CpuBackend::new();
        // Use the anchor at origin ; fit max_dist to the largest L2 norm.
        let d = 2;
        let len_b = vals.len() / d;
        if len_b < 1 { return Ok(()); }
        let values: Vec<f32> = vals.iter().take(len_b * d).copied().collect();
        let latents = Tensor::<f32>::from_slice(&values, [len_b, d].into()).unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0; 2], [1, 2].into()).unwrap();
        // Compute max L2 norm across rows
        let mut max_norm = 0.0f32;
        for i in 0..len_b {
            let sq = values[i * d].powi(2) + values[i * d + 1].powi(2);
            max_norm = max_norm.max(sq.sqrt());
        }
        // Set max_dist slightly above the max observed norm
        let max_dist = max_norm + 1.0;
        let out = b.cone_project(&latents, &anchors, max_dist).unwrap();
        let s = out.as_slice().unwrap();
        for i in 0..values.len() {
            prop_assert!((s[i] - values[i]).abs() < 1e-5);
        }
    }

    /// Idempotence : applying cone_project twice = applying once.
    #[test]
    fn cone_project_idempotent(
        vals in prop::collection::vec(-20.0f32..20.0, 6..20),
    ) {
        let b = CpuBackend::new();
        let d = 2;
        let len_b = vals.len() / d;
        if len_b < 1 { return Ok(()); }
        let values: Vec<f32> = vals.iter().take(len_b * d).copied().collect();
        let latents = Tensor::<f32>::from_slice(&values, [len_b, d].into()).unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0; 2], [1, 2].into()).unwrap();
        let once = b.cone_project(&latents, &anchors, 1.5).unwrap();
        let twice = b.cone_project(&once, &anchors, 1.5).unwrap();
        let s1 = once.as_slice().unwrap();
        let s2 = twice.as_slice().unwrap();
        for i in 0..s1.len() {
            prop_assert!((s1[i] - s2[i]).abs() < 1e-4);
        }
    }

    /// Every output point lies within max_dist of its nearest anchor.
    #[test]
    fn cone_project_output_within_max_dist(
        vals in prop::collection::vec(-10.0f32..10.0, 4..20),
        max_dist in 0.1f32..2.0,
    ) {
        let b = CpuBackend::new();
        let d = 2;
        let len_b = vals.len() / d;
        if len_b < 1 { return Ok(()); }
        let values: Vec<f32> = vals.iter().take(len_b * d).copied().collect();
        let latents = Tensor::<f32>::from_slice(&values, [len_b, d].into()).unwrap();
        let anchors = Tensor::<f32>::from_slice(&[0.0; 2], [1, 2].into()).unwrap();
        let out = b.cone_project(&latents, &anchors, max_dist).unwrap();
        let s = out.as_slice().unwrap();
        for i in 0..len_b {
            let norm = (s[i * d].powi(2) + s[i * d + 1].powi(2)).sqrt();
            prop_assert!(
                norm <= max_dist + 1e-4,
                "row {} norm {} > max_dist {}",
                i, norm, max_dist
            );
        }
    }
}

// ==================================================================
// weighted_mean — 3 properties
// ==================================================================

proptest! {
    /// With uniform weights, weighted_mean ≈ plain mean along axis 0.
    #[test]
    fn weighted_mean_uniform_equals_plain_mean(
        vals in prop::collection::vec(-5.0f32..5.0, 6..30),
    ) {
        let b = CpuBackend::new();
        let d = 3;
        let len_b = vals.len() / d;
        if len_b < 2 { return Ok(()); }
        let values: Vec<f32> = vals.iter().take(len_b * d).copied().collect();
        let t = Tensor::<f32>::from_slice(&values, [len_b, d].into()).unwrap();
        let weights = Tensor::<f32>::from_slice(
            &vec![1.0f32; len_b], [len_b].into(),
        ).unwrap();
        let wm = b.weighted_mean(&t, &weights, false).unwrap();
        let plain_mean = b.mean(&t, 0).unwrap();
        let ws = wm.as_slice().unwrap();
        let pms = plain_mean.as_slice().unwrap();
        for i in 0..d {
            prop_assert!((ws[i] - pms[i]).abs() < 1e-4);
        }
    }

    /// Scale-invariant : weighted_mean(v, αw) = weighted_mean(v, w) for α > 0.
    #[test]
    fn weighted_mean_scale_invariant(
        vals in prop::collection::vec(-3.0f32..3.0, 6..20),
        alpha in 0.01f32..100.0,
    ) {
        let b = CpuBackend::new();
        let d = 2;
        let len_b = vals.len() / d;
        if len_b < 1 { return Ok(()); }
        let values: Vec<f32> = vals.iter().take(len_b * d).copied().collect();
        let t = Tensor::<f32>::from_slice(&values, [len_b, d].into()).unwrap();
        let w1_vec: Vec<f32> = (0..len_b).map(|i| 0.1 + i as f32 * 0.1).collect();
        let w2_vec: Vec<f32> = w1_vec.iter().map(|x| x * alpha).collect();
        let w1 = Tensor::<f32>::from_slice(&w1_vec, [len_b].into()).unwrap();
        let w2 = Tensor::<f32>::from_slice(&w2_vec, [len_b].into()).unwrap();
        let o1 = b.weighted_mean(&t, &w1, false).unwrap();
        let o2 = b.weighted_mean(&t, &w2, false).unwrap();
        let s1 = o1.as_slice().unwrap();
        let s2 = o2.as_slice().unwrap();
        for i in 0..d {
            prop_assert!((s1[i] - s2[i]).abs() < 1e-3 * s1[i].abs().max(1.0));
        }
    }

    /// Single-weight concentration : w = [0,...,1,...,0] → values[chosen].
    #[test]
    fn weighted_mean_single_weight_selects(
        vals in prop::collection::vec(-3.0f32..3.0, 9..=9),
        chosen in 0usize..3,
    ) {
        let b = CpuBackend::new();
        let d = 3;
        let len_b = 3;
        let t = Tensor::<f32>::from_slice(&vals, [len_b, d].into()).unwrap();
        let mut w_vec = vec![0.0f32; len_b];
        w_vec[chosen] = 1.0;
        let w = Tensor::<f32>::from_slice(&w_vec, [len_b].into()).unwrap();
        let wm = b.weighted_mean(&t, &w, false).unwrap();
        let s = wm.as_slice().unwrap();
        for i in 0..d {
            prop_assert!((s[i] - vals[chosen * d + i]).abs() < 1e-5);
        }
    }
}

// ==================================================================
// moe_reconcile — 3 properties
// ==================================================================

proptest! {
    /// top_k = 1 is equivalent to selecting candidates[argmax(gate)].
    #[test]
    fn moe_top_k_one_equals_argmax_candidate(
        cands_data in prop::collection::vec(-2.0f32..2.0, 6..=6),
        gate_data in prop::collection::vec(-5.0f32..5.0, 3..=3),
    ) {
        let b = CpuBackend::new();
        let cands = Tensor::<f32>::from_slice(&cands_data, [1, 3, 2].into()).unwrap();
        let gate = Tensor::<f32>::from_slice(&gate_data, [1, 3].into()).unwrap();
        let out = b.moe_reconcile(&cands, &gate, 1).unwrap();
        let s = out.as_slice().unwrap();
        // argmax gate (tie-break smallest index)
        let mut best_i = 0;
        let mut best_v = gate_data[0];
        for (i, &v) in gate_data.iter().enumerate().skip(1) {
            if v > best_v {
                best_v = v; best_i = i;
            }
        }
        for dim in 0..2 {
            let expected = cands_data[best_i * 2 + dim];
            prop_assert!((s[dim] - expected).abs() < 1e-5);
        }
    }

    /// Output shape is (B, d) for any valid input.
    #[test]
    fn moe_reconcile_output_shape(
        batch in 1usize..4,
        k in 1usize..6,
        d_feat in 1usize..5,
    ) {
        let b = CpuBackend::new();
        let cands = Tensor::<f32>::zeros([batch, k, d_feat].into()).unwrap();
        let gate = Tensor::<f32>::zeros([batch, k].into()).unwrap();
        let top_k = k.clamp(1, 2);
        let out = b.moe_reconcile(&cands, &gate, top_k as u32).unwrap();
        prop_assert_eq!(out.shape().dims(), &[batch, d_feat]);
    }

    /// Output values are finite when inputs are finite.
    #[test]
    fn moe_reconcile_finite_output(
        cands_data in prop::collection::vec(-1.0f32..1.0, 12..=12),
        gate_data in prop::collection::vec(-3.0f32..3.0, 3..=3),
    ) {
        let b = CpuBackend::new();
        let cands = Tensor::<f32>::from_slice(&cands_data, [2, 3, 2].into()).unwrap();
        let gate_2 = [gate_data.clone(), gate_data].concat();
        let gate = Tensor::<f32>::from_slice(&gate_2, [2, 3].into()).unwrap();
        let out = b.moe_reconcile(&cands, &gate, 2).unwrap();
        for &v in out.as_slice().unwrap() {
            prop_assert!(v.is_finite());
        }
    }
}

// ==================================================================
// topk — 2 properties
// ==================================================================

proptest! {
    /// topk values are in strict non-increasing order.
    #[test]
    fn topk_values_sorted_desc(
        data in finite_vec(5, 32),
        k in 1usize..5,
    ) {
        let b = CpuBackend::new();
        let k_actual = k.min(data.len());
        let t = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let (vals, _idx) = b.topk(&t, k_actual as u32, -1).unwrap();
        let v = vals.as_slice().unwrap();
        for i in 1..v.len() {
            prop_assert!(v[i] <= v[i - 1] + 1e-6);
        }
    }

    /// topk indices are in valid range [0, n).
    #[test]
    fn topk_indices_in_range(
        data in finite_vec(5, 32),
        k in 1usize..5,
    ) {
        let b = CpuBackend::new();
        let n = data.len();
        let k_actual = k.min(n);
        let t = Tensor::<f32>::from_slice(&data, [n].into()).unwrap();
        let (_vals, idx) = b.topk(&t, k_actual as u32, -1).unwrap();
        let n_i32 = n as i32;
        for &i in idx.as_slice().unwrap() {
            prop_assert!((0..n_i32).contains(&i));
        }
    }
}

// ==================================================================
// multinomial_sample — 2 properties
// ==================================================================

proptest! {
    /// Same seed, same input → identical output.
    #[test]
    fn multinomial_reproducibility(
        probs_data in positive_probs(4),
        n in 10u32..50,
        seed in 0u64..1_000,
    ) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&probs_data, [4].into()).unwrap();
        let s1 = b.multinomial_sample(&t, n, seed).unwrap();
        let s2 = b.multinomial_sample(&t, n, seed).unwrap();
        prop_assert_eq!(s1.as_slice().unwrap(), s2.as_slice().unwrap());
    }

    /// All output indices are in [0, K).
    #[test]
    fn multinomial_indices_in_range(
        probs_data in positive_probs(5),
        n in 10u32..100,
        seed in 0u64..1_000,
    ) {
        let b = CpuBackend::new();
        let t = Tensor::<f32>::from_slice(&probs_data, [5].into()).unwrap();
        let s = b.multinomial_sample(&t, n, seed).unwrap();
        for &i in s.as_slice().unwrap() {
            prop_assert!((0..5).contains(&i));
        }
    }
}

// ==================================================================
// kl_divergence + std_dev — 2 properties
// ==================================================================

proptest! {
    /// KL(p || p) ≈ 0 for any valid probability vector.
    #[test]
    fn kl_self_is_zero(
        data in positive_probs(5),
    ) {
        let b = CpuBackend::new();
        let sum: f32 = data.iter().sum();
        let norm: Vec<f32> = data.iter().map(|v| v / sum).collect();
        let t = Tensor::<f32>::from_slice(&norm, [norm.len()].into()).unwrap();
        let kl = b.kl_divergence(&t, &t, -1, 1e-9).unwrap();
        prop_assert!(kl.as_slice().unwrap()[0].abs() < 1e-4);
    }

    /// std_dev(x + c) ≈ std_dev(x) (shift invariance).
    #[test]
    fn std_dev_shift_invariant(
        data in finite_vec(3, 16),
        shift in -10.0f32..10.0,
    ) {
        let b = CpuBackend::new();
        let t1 = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let shifted: Vec<f32> = data.iter().map(|v| v + shift).collect();
        let t2 = Tensor::<f32>::from_slice(&shifted, [shifted.len()].into()).unwrap();
        let s1 = b.std_dev(&t1, -1, true).unwrap().as_slice().unwrap()[0];
        let s2 = b.std_dev(&t2, -1, true).unwrap().as_slice().unwrap()[0];
        prop_assert!((s1 - s2).abs() < 1e-3 * s1.abs().max(1.0));
    }

    /// std_dev(α·x) ≈ |α| · std_dev(x) (scale linearity).
    #[test]
    fn std_dev_scale_linear(
        data in finite_vec(3, 16),
        alpha in -5.0f32..5.0,
    ) {
        // Avoid near-zero alpha since s1·|alpha| ≈ 0 amplifies rtol.
        if alpha.abs() < 0.1 { return Ok(()); }
        let b = CpuBackend::new();
        let t1 = Tensor::<f32>::from_slice(&data, [data.len()].into()).unwrap();
        let scaled: Vec<f32> = data.iter().map(|v| v * alpha).collect();
        let t2 = Tensor::<f32>::from_slice(&scaled, [scaled.len()].into()).unwrap();
        let s1 = b.std_dev(&t1, -1, true).unwrap().as_slice().unwrap()[0];
        let s2 = b.std_dev(&t2, -1, true).unwrap().as_slice().unwrap()[0];
        let expected = alpha.abs() * s1;
        prop_assert!(
            (s2 - expected).abs() < 1e-3 * expected.abs().max(1.0),
            "std_dev(α·x)={s2}, |α|·std_dev(x)={expected} (α={alpha})"
        );
    }

    /// KL(p || q) ≥ 0 for all valid normalized distributions.
    #[test]
    fn kl_non_negative(
        p_raw in positive_probs(5),
        q_raw in positive_probs(5),
    ) {
        let b = CpuBackend::new();
        // Normalize both to sum = 1
        let p_sum: f32 = p_raw.iter().sum();
        let q_sum: f32 = q_raw.iter().sum();
        let p_n: Vec<f32> = p_raw.iter().map(|v| v / p_sum).collect();
        let q_n: Vec<f32> = q_raw.iter().map(|v| v / q_sum).collect();
        let tp = Tensor::<f32>::from_slice(&p_n, [5].into()).unwrap();
        let tq = Tensor::<f32>::from_slice(&q_n, [5].into()).unwrap();
        let kl = b.kl_divergence(&tp, &tq, -1, 1e-9).unwrap();
        let v = kl.as_slice().unwrap()[0];
        // Must be ≥ 0 (Gibbs' inequality) and ≤ 50 (documented cap).
        prop_assert!(v >= -1e-6, "KL divergence must be non-negative, got {v}");
        prop_assert!(v <= 50.0 + 1e-6, "KL divergence must be ≤ 50, got {v}");
    }

    /// moe_reconcile with top_k = K ≡ softmax(gate_logits)-weighted sum
    /// over all candidates. This is a property-level check that runs
    /// ~100 random cases vs the single integration test.
    #[test]
    fn moe_top_k_equal_k_matches_full_softmax(
        cands_data in prop::collection::vec(-2.0f32..2.0, 8..=8),
        gate_data in prop::collection::vec(-3.0f32..3.0, 4..=4),
    ) {
        let b = CpuBackend::new();
        // cands shape (1, 4, 2), gate shape (1, 4)
        let cands = Tensor::<f32>::from_slice(&cands_data, [1, 4, 2].into()).unwrap();
        let gate = Tensor::<f32>::from_slice(&gate_data, [1, 4].into()).unwrap();

        let moe_out = b.moe_reconcile(&cands, &gate, 4).unwrap();

        // Manual : softmax(gate)-weighted sum of candidates
        let gate_t = Tensor::<f32>::from_slice(&gate_data, [4].into()).unwrap();
        let sm = b.softmax(&gate_t, -1).unwrap();
        let sm_w = sm.as_slice().unwrap();
        let mut manual = [0.0f32; 2];
        for k in 0..4 {
            for j in 0..2 {
                manual[j] += sm_w[k] * cands_data[k * 2 + j];
            }
        }

        let got = moe_out.as_slice().unwrap();
        for j in 0..2 {
            prop_assert!(
                (got[j] - manual[j]).abs() < 1e-4,
                "dim {j}: moe={}, manual={}", got[j], manual[j]
            );
        }
    }
}
