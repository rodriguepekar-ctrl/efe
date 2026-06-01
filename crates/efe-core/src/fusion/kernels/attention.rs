//! Attention score kernel: `s_ij = (h_i · h_j) / √D`.
//!
//! Complexity: O(N²·D). Read-only over latent rows, writes a flat
//! `N²` f32 scratch buffer. Inactive rows are filled with `-∞` so the
//! downstream softmax produces zero weight on them.
//!
//! ## Sprint 5 — deliberately not migrated to `ComputeBackend`
//!
//! This kernel reads `bf16` rows via [`LatentView`] and writes `f32`
//! scores. The compute backend (`compute::cpu`) currently exposes its
//! attention surface as
//! [`crate::compute::ComputeBackend::scaled_dot_product_attention`],
//! which (a) operates only on `Tensor<f32>`, (b) returns the full
//! attention output (softmax(Q·Kᵀ/√D)·V), not the raw scores, and
//! (c) requires rank-4 inputs.
//!
//! Migrating would require:
//! 1. A new `Tensor<bf16>` type or per-call bf16→f32 buffer copy.
//! 2. A new backend op `scaled_dot_scores` that exposes scores only.
//!
//! Both extensions land in **Sprint 7** when bf16 enters the trait.
//! For Sprint 5, the fail-fast / mask semantics of this kernel are
//! left intact and audited under the existing test suite.

use crate::els::views::{LatentView, MaskView};
use crate::fusion::kernels::KernelParams;

/// Compute `scores[i*N + j] = (h_i · h_j) / (√D · temperature)` for
/// active `i` and active `j`. Masked entries are set to `-∞` so the
/// softmax assigns zero weight to them.
///
/// # Preconditions
/// - `scores.len() == N * N`
/// - `latent.n_agents() == mask.n_agents() == N`
/// - `latent.dim() == D`
///
/// # Notes
/// - Accumulator in f32 even though latent is bf16.
/// - Division by `√D · temperature` applied after the dot product to
///   preserve bf16 precision.
/// - If `params.mask_self_attention`, the diagonal is `-∞`.
/// - The kernel is deliberately written in a straight double-loop to
///   help the optimizer auto-vectorize; Rayon/SIMD variants live behind
///   feature flags in Sprint 5+.
pub(crate) fn compute_scores(
    latent: &LatentView<'_>,
    mask: &MaskView<'_>,
    scores: &mut [f32],
    params: &KernelParams,
) {
    let n = latent.n_agents() as usize;
    let d = latent.dim() as usize;
    debug_assert_eq!(
        scores.len(),
        n.checked_mul(n).expect("N² overflow"),
        "scores buffer must be N*N"
    );
    debug_assert_eq!(mask.n_agents() as usize, n, "mask and latent disagree on N");

    // Precompute the scale once. `1/sqrt(D)` never divides by zero
    // because D > 0 is enforced upstream; additionally temperature is
    // validated > 0 in FusionConfig.
    let scale = if d == 0 {
        0.0
    } else {
        1.0 / ((d as f32).sqrt() * params.temperature)
    };

    for i in 0..n {
        let row_i = i * n;
        if !mask.is_active(i as u32) {
            // Agent i is inactive: it doesn't attend to anyone and no
            // other agent attends *from* it as a source. The softmax
            // downstream converts a full -∞ row into a zero-attention
            // row, and our aggregate step treats zero-attention rows as
            // identity (preserving the inactive row's latent).
            for s in &mut scores[row_i..row_i + n] {
                *s = f32::NEG_INFINITY;
            }
            continue;
        }
        // Reading row once. `row(i)` is infallible here because
        // `i < n_agents` — we hold a valid `LatentView`.
        let h_i = latent.row(i as u32).expect("row in range");
        for j in 0..n {
            if !mask.is_active(j as u32) {
                scores[row_i + j] = f32::NEG_INFINITY;
                continue;
            }
            if params.mask_self_attention && i == j {
                scores[row_i + j] = f32::NEG_INFINITY;
                continue;
            }
            let h_j = latent.row(j as u32).expect("row in range");
            // Inline f32 accumulation over bf16 values.
            let mut acc = 0.0f32;
            for (x, y) in h_i.iter().zip(h_j.iter()) {
                acc += x.to_f32() * y.to_f32();
            }
            scores[row_i + j] = acc * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::els::views::{LatentView, MaskView};
    use crate::memory::{Arena, DeviceBackend, HostBackend};
    use half::bf16;
    use std::sync::Arc;

    /// Helper — build a (latent, mask) pair inside a fresh arena.
    fn mk_latent_mask(
        n: u32,
        d: u32,
        fill_l: impl Fn(u32, u32) -> f32,
        fill_m: impl Fn(u32) -> u8,
    ) -> (Arena, Vec<f32>) {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 18, backend, 0).unwrap();
        let raw_l = arena
            .alloc_zeroed_raw(u64::from(n) * u64::from(d) * 2)
            .unwrap();
        let raw_m = arena.alloc_zeroed_raw(u64::from(n)).unwrap();
        unsafe {
            let p = raw_l.ptr.as_raw().cast::<bf16>().as_ptr();
            for i in 0..n {
                for j in 0..d {
                    *p.add((i * d + j) as usize) = bf16::from_f32(fill_l(i, j));
                }
            }
            let p = raw_m.ptr.as_raw().as_ptr();
            for i in 0..n {
                *p.add(i as usize) = fill_m(i);
            }
        }
        // Capture the latent/mask for test inspection.
        let latent = LatentView::from_raw(raw_l, n, d, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, n, &arena).unwrap();

        let mut scores = vec![0.0f32; (n as usize) * (n as usize)];
        let params = KernelParams {
            temperature: 1.0,
            mask_self_attention: false,
            beta: 0.0,
            alpha: 0.0,
            clip_max: 32.0,
        };
        compute_scores(&latent, &mask, &mut scores, &params);
        (arena, scores)
    }

    #[test]
    fn scores_symmetric_for_symmetric_latent() {
        // All agents have row = [i as f32 + 1, 0] (linear but distinct).
        // Dot products symmetric → scores[i,j] == scores[j,i].
        let n = 4u32;
        let d = 2u32;
        let (_arena, scores) = mk_latent_mask(
            n,
            d,
            |i, j| if j == 0 { (i + 1) as f32 } else { 0.0 },
            |_| 1,
        );
        for i in 0..n as usize {
            for j in 0..n as usize {
                let s_ij = scores[i * n as usize + j];
                let s_ji = scores[j * n as usize + i];
                assert!((s_ij - s_ji).abs() < 1e-4, "asymmetry at ({i},{j})");
            }
        }
    }

    #[test]
    fn scores_inactive_row_all_neg_inf() {
        let n = 3u32;
        let d = 2u32;
        let (_arena, scores) = mk_latent_mask(n, d, |_, _| 0.5, |i| u8::from(i != 1));
        let n_us = n as usize;
        // Row 1 is inactive → all entries -∞
        for j in 0..n_us {
            assert!(scores[n_us + j] == f32::NEG_INFINITY);
        }
        // Column 1 must be -∞ in every row because agent 1 is an
        // invalid *target* too.
        for i in 0..n_us {
            assert!(scores[i * n_us + 1] == f32::NEG_INFINITY);
        }
    }

    #[test]
    fn scores_self_masked_when_requested() {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 18, backend, 0).unwrap();
        let raw_l = arena.alloc_zeroed_raw(4 * 2 * 2).unwrap();
        let raw_m = arena.alloc_zeroed_raw(4).unwrap();
        unsafe {
            let p = raw_l.ptr.as_raw().cast::<bf16>().as_ptr();
            for i in 0..4u32 {
                *p.add((i * 2) as usize) = bf16::from_f32(1.0);
                *p.add((i * 2 + 1) as usize) = bf16::from_f32(1.0);
            }
            let p = raw_m.ptr.as_raw().as_ptr();
            for i in 0..4 {
                *p.add(i as usize) = 1;
            }
        }
        let latent = LatentView::from_raw(raw_l, 4, 2, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let mut scores = vec![0.0f32; 16];
        let params = KernelParams {
            temperature: 1.0,
            mask_self_attention: true,
            beta: 0.0,
            alpha: 0.0,
            clip_max: 32.0,
        };
        compute_scores(&latent, &mask, &mut scores, &params);
        for i in 0..4 {
            assert!(scores[i * 4 + i] == f32::NEG_INFINITY, "diag not masked");
        }
    }

    #[test]
    fn scores_scaled_by_sqrt_d() {
        // Latent = all-ones. Dot product = D. Divided by √D → √D.
        let n = 2u32;
        let d = 4u32;
        let (_arena, scores) = mk_latent_mask(n, d, |_, _| 1.0, |_| 1);
        let expected = (d as f32).sqrt();
        for &s in &scores {
            assert!((s - expected).abs() < 1e-4, "got {s} want {expected}");
        }
    }

    #[test]
    fn scores_temperature_scales() {
        // Doubling temperature halves the scores.
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 18, backend, 0).unwrap();
        let raw_l = arena.alloc_zeroed_raw(2 * 4 * 2).unwrap();
        let raw_m = arena.alloc_zeroed_raw(2).unwrap();
        unsafe {
            let p = raw_l.ptr.as_raw().cast::<bf16>().as_ptr();
            for k in 0..8 {
                *p.add(k as usize) = bf16::from_f32(1.0);
            }
            let p = raw_m.ptr.as_raw().as_ptr();
            *p.add(0) = 1;
            *p.add(1) = 1;
        }
        let latent = LatentView::from_raw(raw_l, 2, 4, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 2, &arena).unwrap();
        let mut s1 = vec![0.0f32; 4];
        let mut s2 = vec![0.0f32; 4];
        compute_scores(
            &latent,
            &mask,
            &mut s1,
            &KernelParams {
                temperature: 1.0,
                mask_self_attention: false,
                beta: 0.0,
                alpha: 0.0,
                clip_max: 32.0,
            },
        );
        compute_scores(
            &latent,
            &mask,
            &mut s2,
            &KernelParams {
                temperature: 2.0,
                mask_self_attention: false,
                beta: 0.0,
                alpha: 0.0,
                clip_max: 32.0,
            },
        );
        for (a, b) in s1.iter().zip(s2.iter()) {
            assert!((a - 2.0 * b).abs() < 1e-3);
        }
    }
}
