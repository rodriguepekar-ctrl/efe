//! β-mix kernel: `h'_i = (1 - β) * h_i + β * p` for each active row.
//!
//! Pulls each active row of the latent toward the problem embedding by
//! a factor `β ∈ [0, 1]`. This keeps the swarm anchored on the query
//! even as agents exchange information with each other.

use half::bf16;

use crate::els::views::{EmbeddingView, LatentViewMut, MaskView};

/// In-place β-mix.
///
/// # Preconditions
/// - `latent.dim() == problem.dim()`
/// - `latent.n_agents() == mask.n_agents()`
/// - `beta` finite ∈ `[0, 1]` (caller's responsibility; debug-asserted).
///
/// # Behavior
/// - For each `i` with `mask.is_active(i)`:
///   `latent[i][d] = (1 - β) * latent[i][d] + β * problem[d]` in f32.
/// - Inactive rows are untouched.
/// - `β = 0` is a no-op (useful when the schedule has decayed β to 0).
pub(crate) fn beta_mix(
    latent: &mut LatentViewMut<'_>,
    problem: &EmbeddingView<'_>,
    mask: &MaskView<'_>,
    beta: f32,
) {
    debug_assert!(beta.is_finite(), "beta must be finite");
    debug_assert!((0.0..=1.0).contains(&beta), "beta must be in [0, 1]");
    debug_assert_eq!(
        latent.dim() as usize,
        problem.as_slice().len(),
        "dim mismatch"
    );

    if beta == 0.0 {
        // β=0 is exact identity, skip altogether.
        return;
    }

    let one_minus_beta = 1.0 - beta;
    let n = latent.n_agents();
    let p = problem.as_slice();
    for i in 0..n {
        if !mask.is_active(i) {
            continue;
        }
        let row = latent.row_mut(i).expect("i < n");
        for (r, pp) in row.iter_mut().zip(p.iter()) {
            let mixed = one_minus_beta * r.to_f32() + beta * pp;
            *r = bf16::from_f32(mixed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::els::views::{EmbeddingView, LatentView, LatentViewMut, MaskView};
    use crate::memory::{Arena, DeviceBackend, HostBackend, RawAllocation};
    use std::sync::Arc;

    struct Setup {
        arena: Arena,
        raw_l: RawAllocation,
        raw_p: RawAllocation,
        raw_m: RawAllocation,
        n: u32,
        d: u32,
    }

    fn setup(
        n: u32,
        d: u32,
        fill_l: impl Fn(u32, u32) -> f32,
        fill_p: impl Fn(u32) -> f32,
        fill_m: impl Fn(u32) -> u8,
    ) -> Setup {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 18, backend, 0).unwrap();
        let raw_l = arena
            .alloc_zeroed_raw(u64::from(n) * u64::from(d) * 2)
            .unwrap();
        let raw_p = arena.alloc_zeroed_raw(u64::from(d) * 4).unwrap();
        let raw_m = arena.alloc_zeroed_raw(u64::from(n)).unwrap();
        unsafe {
            let p = raw_l.ptr.as_raw().cast::<bf16>().as_ptr();
            for i in 0..n {
                for j in 0..d {
                    *p.add((i * d + j) as usize) = bf16::from_f32(fill_l(i, j));
                }
            }
            let p = raw_p.ptr.as_raw().cast::<f32>().as_ptr();
            for j in 0..d {
                *p.add(j as usize) = fill_p(j);
            }
            let p = raw_m.ptr.as_raw().as_ptr();
            for i in 0..n {
                *p.add(i as usize) = fill_m(i);
            }
        }
        Setup {
            arena,
            raw_l,
            raw_p,
            raw_m,
            n,
            d,
        }
    }

    #[test]
    fn beta_zero_is_exact_identity() {
        let s = setup(3, 4, |_, _| 1.0, |_| 9.0, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let problem = EmbeddingView::from_raw(s.raw_p, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        beta_mix(&mut latent, &problem, &mask, 0.0);
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        for i in 0..s.n {
            for v in l.row(i).unwrap() {
                assert!((v.to_f32() - 1.0).abs() < 0.01);
            }
        }
    }

    #[test]
    fn beta_one_replaces_with_problem() {
        let s = setup(3, 4, |_, _| 1.0, |j| (j + 1) as f32, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let problem = EmbeddingView::from_raw(s.raw_p, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        beta_mix(&mut latent, &problem, &mask, 1.0);
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        for i in 0..s.n {
            let row = l.row(i).unwrap();
            for (j, v) in row.iter().enumerate() {
                let expected = (j + 1) as f32;
                assert!((v.to_f32() - expected).abs() < 0.02);
            }
        }
    }

    #[test]
    fn beta_half_produces_midpoint() {
        // latent row = [2,2,2], problem = [0,0,0]. β=0.5 → result = [1,1,1].
        let s = setup(1, 3, |_, _| 2.0, |_| 0.0, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let problem = EmbeddingView::from_raw(s.raw_p, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        beta_mix(&mut latent, &problem, &mask, 0.5);
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        for v in l.row(0).unwrap() {
            assert!((v.to_f32() - 1.0).abs() < 0.02);
        }
    }

    #[test]
    fn beta_skips_inactive_agents() {
        // Agent 1 inactive; its row should remain 1.0 regardless of β.
        let s = setup(2, 2, |_, _| 1.0, |_| 10.0, |i| u8::from(i == 0));
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let problem = EmbeddingView::from_raw(s.raw_p, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        beta_mix(&mut latent, &problem, &mask, 0.5);
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        // Row 0 (active) should be midpoint = 5.5.
        for v in l.row(0).unwrap() {
            assert!((v.to_f32() - 5.5).abs() < 0.1);
        }
        // Row 1 (inactive) must stay 1.0.
        for v in l.row(1).unwrap() {
            assert!((v.to_f32() - 1.0).abs() < 0.01);
        }
    }
}
