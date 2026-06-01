//! Weighted aggregation: `h'_i = Σ_j α_ij * h_j`.
//!
//! Reads the old latent through a `LatentView`, writes the new latent
//! into a separate buffer (double-buffering — avoids self-read during
//! the summation). The f32 accumulator buffer is passed in.
//!
//! ## Sprint 5 — deliberately not migrated to `ComputeBackend`
//!
//! Algebraically this kernel is `new = softmax_weights · old` — a
//! `(N, N) · (N, D) = (N, D)` matmul. A direct migration via
//! [`crate::compute::ComputeBackend::matmul`] would require:
//!
//! 1. A `Tensor<bf16>` type or per-row bf16↔f32 conversion (the
//!    backend is `f32`-only at S5).
//! 2. Masking on top of matmul: inactive rows must preserve their
//!    `old[i]`, and zero-weight rows fall back to identity. The
//!    matmul op does not encode these branches.
//!
//! Both extensions are part of the **Sprint 7** bf16 + masked-matmul
//! work. For Sprint 5 the kernel keeps its fp16-aware loop unchanged
//! and the existing test suite remains the contract.

use half::bf16;

use crate::els::views::{LatentView, LatentViewMut, MaskView};

/// Aggregate the latent rows with the given softmax weights.
///
/// # Preconditions
/// - `old.n_agents() == new.n_agents() == mask.n_agents() == N`
/// - `old.dim() == new.dim() == D`
/// - `softmax.len() == N*N`
/// - `acc_scratch.len() == D`
///
/// # Behavior
/// - For row `i` active with at least one positive weight:
///   `new[i] = Σ_j softmax[i,j] * old[j]` (f32 accumulation).
/// - For row `i` active but with zero total weight (all softmax entries
///   zero — "zero-attention row"): copy `old[i]` to `new[i]` (identity).
/// - For row `i` inactive: copy `old[i]` to `new[i]` unchanged.
pub(crate) fn aggregate(
    old: &LatentView<'_>,
    new: &mut LatentViewMut<'_>,
    mask: &MaskView<'_>,
    softmax: &[f32],
    acc_scratch: &mut [f32],
) {
    let n = old.n_agents() as usize;
    let d = old.dim() as usize;
    debug_assert_eq!(
        softmax.len(),
        n.checked_mul(n).expect("N² overflow"),
        "softmax buffer must be N*N"
    );
    debug_assert_eq!(acc_scratch.len(), d, "acc scratch must be D");
    debug_assert_eq!(new.n_agents() as usize, n);
    debug_assert_eq!(new.dim() as usize, d);

    for i in 0..n {
        // Borrow the target row once — this avoids revalidating the
        // slice through `row_mut` twice in the same iteration.
        let out = new.row_mut(i as u32).expect("i < N by construction");

        if !mask.is_active(i as u32) {
            // Inactive: preserve old bytes verbatim.
            out.copy_from_slice(old.row(i as u32).expect("i < N"));
            continue;
        }

        // Compute row's total weight — if zero, fallback to identity.
        let weights = &softmax[i * n..(i + 1) * n];
        let weight_sum: f32 = weights.iter().sum();
        if weight_sum < f32::EPSILON {
            out.copy_from_slice(old.row(i as u32).expect("i < N"));
            continue;
        }

        // Zero accumulator.
        for a in acc_scratch.iter_mut() {
            *a = 0.0;
        }

        // Weighted sum in f32.
        for (j, &w) in weights.iter().enumerate() {
            if w == 0.0 {
                continue;
            }
            let row_j = old.row(j as u32).expect("j < N");
            for (a, v) in acc_scratch.iter_mut().zip(row_j.iter()) {
                *a += w * v.to_f32();
            }
        }

        // Write back in bf16.
        for (o, a) in out.iter_mut().zip(acc_scratch.iter()) {
            *o = bf16::from_f32(*a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::els::views::{LatentView, LatentViewMut, MaskView};
    use crate::memory::{Arena, DeviceBackend, HostBackend};
    use std::sync::Arc;

    /// Build two latent allocations (old + new), fill old with
    /// `fill(i,j)` and a mask with `fill_m(i)`. Returns the arena and
    /// the raw handles so tests can verify bytes.
    struct TestBuffers {
        arena: Arena,
        raw_old: crate::memory::RawAllocation,
        raw_new: crate::memory::RawAllocation,
        raw_mask: crate::memory::RawAllocation,
        n: u32,
        d: u32,
    }

    fn setup(
        n: u32,
        d: u32,
        fill: impl Fn(u32, u32) -> f32,
        fill_m: impl Fn(u32) -> u8,
    ) -> TestBuffers {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 18, backend, 0).unwrap();
        let raw_old = arena
            .alloc_zeroed_raw(u64::from(n) * u64::from(d) * 2)
            .unwrap();
        let raw_new = arena
            .alloc_zeroed_raw(u64::from(n) * u64::from(d) * 2)
            .unwrap();
        let raw_mask = arena.alloc_zeroed_raw(u64::from(n)).unwrap();
        unsafe {
            let p = raw_old.ptr.as_raw().cast::<bf16>().as_ptr();
            for i in 0..n {
                for j in 0..d {
                    *p.add((i * d + j) as usize) = bf16::from_f32(fill(i, j));
                }
            }
            let p = raw_mask.ptr.as_raw().as_ptr();
            for i in 0..n {
                *p.add(i as usize) = fill_m(i);
            }
        }
        TestBuffers {
            arena,
            raw_old,
            raw_new,
            raw_mask,
            n,
            d,
        }
    }

    #[test]
    fn aggregate_identity_on_zero_weights() {
        // All weights zero → new row == old row for active, and
        // inactive rows are also identity.
        let buf = setup(4, 3, |i, _| (i as f32) + 1.0, |_| 1);
        let old = LatentView::from_raw(buf.raw_old, buf.n, buf.d, &buf.arena).unwrap();
        let mut new = LatentViewMut::from_raw(buf.raw_new, buf.n, buf.d, &buf.arena).unwrap();
        let mask = MaskView::from_raw(buf.raw_mask, buf.n, &buf.arena).unwrap();
        let softmax = vec![0.0f32; 16];
        let mut acc = vec![0.0f32; 3];
        aggregate(&old, &mut new, &mask, &softmax, &mut acc);
        // new[i][0] should equal old[i][0] because weights are zero → identity.
        let new_immut = LatentView::from_raw(buf.raw_new, buf.n, buf.d, &buf.arena).unwrap();
        for i in 0..4 {
            let row = new_immut.row(i).unwrap();
            assert!((row[0].to_f32() - ((i + 1) as f32)).abs() < 0.01);
        }
    }

    #[test]
    fn aggregate_weighted_sum_matches_formula() {
        // 2 agents: old = [[1,0],[0,1]], softmax = [[0.5,0.5],[0.5,0.5]].
        // Expected: new = [[0.5,0.5],[0.5,0.5]].
        let buf = setup(2, 2, |i, j| if i == j { 1.0 } else { 0.0 }, |_| 1);
        let old = LatentView::from_raw(buf.raw_old, buf.n, buf.d, &buf.arena).unwrap();
        let mut new = LatentViewMut::from_raw(buf.raw_new, buf.n, buf.d, &buf.arena).unwrap();
        let mask = MaskView::from_raw(buf.raw_mask, buf.n, &buf.arena).unwrap();
        let softmax = vec![0.5f32, 0.5, 0.5, 0.5];
        let mut acc = vec![0.0f32; 2];
        aggregate(&old, &mut new, &mask, &softmax, &mut acc);
        let new_immut = LatentView::from_raw(buf.raw_new, buf.n, buf.d, &buf.arena).unwrap();
        for i in 0..2 {
            let row = new_immut.row(i).unwrap();
            assert!((row[0].to_f32() - 0.5).abs() < 0.01);
            assert!((row[1].to_f32() - 0.5).abs() < 0.01);
        }
    }

    #[test]
    fn aggregate_respects_inactive_rows() {
        // Agent 1 inactive — its row must be untouched.
        let buf = setup(2, 2, |i, _| (i + 1) as f32, |i| u8::from(i == 0));
        let old = LatentView::from_raw(buf.raw_old, buf.n, buf.d, &buf.arena).unwrap();
        let mut new = LatentViewMut::from_raw(buf.raw_new, buf.n, buf.d, &buf.arena).unwrap();
        let mask = MaskView::from_raw(buf.raw_mask, buf.n, &buf.arena).unwrap();
        let softmax = vec![1.0f32, 0.0, 0.0, 0.0];
        let mut acc = vec![0.0f32; 2];
        aggregate(&old, &mut new, &mask, &softmax, &mut acc);
        let new_immut = LatentView::from_raw(buf.raw_new, buf.n, buf.d, &buf.arena).unwrap();
        // Row 1 (inactive) must equal old row 1 (= 2.0 everywhere).
        let row1 = new_immut.row(1).unwrap();
        for v in row1 {
            assert!((v.to_f32() - 2.0).abs() < 0.01);
        }
    }

    #[test]
    fn aggregate_preserves_norm_bound() {
        // If ||old_i|| ≤ B and weights are a probability distribution,
        // then ||new_i|| ≤ B (convex combination). Test: all rows
        // norm = √D, softmax uniform → new norm ≤ √D.
        let n = 4u32;
        let d = 4u32;
        let buf = setup(n, d, |_, _| 1.0, |_| 1);
        let old = LatentView::from_raw(buf.raw_old, n, d, &buf.arena).unwrap();
        let mut new = LatentViewMut::from_raw(buf.raw_new, n, d, &buf.arena).unwrap();
        let mask = MaskView::from_raw(buf.raw_mask, n, &buf.arena).unwrap();
        let w = 1.0 / (n as f32);
        let softmax = vec![w; (n * n) as usize];
        let mut acc = vec![0.0f32; d as usize];
        aggregate(&old, &mut new, &mask, &softmax, &mut acc);
        let new_immut = LatentView::from_raw(buf.raw_new, n, d, &buf.arena).unwrap();
        for i in 0..n {
            let row = new_immut.row(i).unwrap();
            let norm_sq: f32 = row.iter().map(|v| v.to_f32().powi(2)).sum();
            assert!(norm_sq.sqrt() <= (d as f32).sqrt() + 0.01);
        }
    }
}
