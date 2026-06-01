//! Row projection onto the alignment cone.
//!
//! For each active row, call `M4::project_vector_onto_anchors` with the
//! (row, anchors, α) triple. The result is an f32 vector blended with
//! the convex hull of the anchors by factor `α`.

use half::bf16;

use crate::els::projection::project_vector_onto_anchors;
use crate::els::views::{AnchorsView, LatentViewMut, MaskView};
use crate::errors::Result;

/// Project every active row onto the anchors' convex hull by strength `α`.
///
/// # Preconditions
/// - `latent.dim() == anchors.dim()`.
/// - `alpha ∈ [0, 1]` (enforced by `project_vector_onto_anchors`).
///
/// # Behavior
/// - `α = 0` is an exact identity and short-circuits.
/// - For each active row:
///   - Upcast bf16 → f32 into `row_scratch`.
///   - Call M4's projection.
///   - Downcast result back to bf16 in-place.
/// - If the total anchor weight was zero for some row (projection
///   returned the input unchanged), `anchor_zero_weight` is set to `true`
///   in the returned metric — the orchestrator may emit a warning.
///
/// # Errors
/// - Propagates [`EfeError::Internal`] if anchors is empty (invariant
///   violation — callers should not pass empty anchors).
/// - Propagates [`EfeError::InvalidConfig`] on dim mismatch.
pub(crate) fn project_rows(
    latent: &mut LatentViewMut<'_>,
    anchors: &AnchorsView<'_>,
    mask: &MaskView<'_>,
    alpha: f32,
    row_scratch: &mut [f32],
) -> Result<bool> {
    debug_assert_eq!(
        latent.dim() as usize,
        row_scratch.len(),
        "row_scratch must be D"
    );

    if alpha == 0.0 {
        // α=0 means keep latent as-is.
        return Ok(false);
    }

    // Materialize the anchors host-side once per round.
    let anchors_host = anchors.to_host();
    let n = latent.n_agents();
    let mut zero_weight_seen = false;

    for i in 0..n {
        if !mask.is_active(i) {
            continue;
        }
        let row = latent.row_mut(i).expect("i < n");

        // bf16 → f32 roundtrip.
        for (r_f, r_bf) in row_scratch.iter_mut().zip(row.iter()) {
            *r_f = r_bf.to_f32();
        }

        let (projected, _delta) = project_vector_onto_anchors(row_scratch, &anchors_host, alpha)?;

        // Detect "no anchor weight" (delta == 0 and we asked α > 0): the
        // row was orthogonal to all anchors. This is a useful signal.
        //
        // We cannot rely on `delta == 0` alone — it can be zero when
        // projected coincides with row (unlikely but possible). A more
        // robust check is comparing the projected to the input: if
        // equal, projection was identity.
        let identical = projected
            .iter()
            .zip(row_scratch.iter())
            .all(|(p, r)| (p - r).abs() < f32::EPSILON);
        if identical {
            zero_weight_seen = true;
        }

        for (r_bf, p_f) in row.iter_mut().zip(projected.iter()) {
            *r_bf = bf16::from_f32(*p_f);
        }
    }

    Ok(zero_weight_seen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::els::views::{AnchorsView, LatentView, LatentViewMut, MaskView};
    use crate::memory::{Arena, DeviceBackend, HostBackend, RawAllocation};
    use std::sync::Arc;

    struct Setup {
        arena: Arena,
        raw_l: RawAllocation,
        raw_a: RawAllocation,
        raw_m: RawAllocation,
        n: u32,
        d: u32,
        k: u32,
    }

    #[allow(clippy::many_single_char_names)]
    fn setup(
        n: u32,
        d: u32,
        k: u32,
        fill_l: impl Fn(u32, u32) -> f32,
        fill_a: impl Fn(u32, u32) -> f32,
        fill_m: impl Fn(u32) -> u8,
    ) -> Setup {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 18, backend, 0).unwrap();
        let raw_l = arena
            .alloc_zeroed_raw(u64::from(n) * u64::from(d) * 2)
            .unwrap();
        let raw_a = arena
            .alloc_zeroed_raw(u64::from(k) * u64::from(d) * 4)
            .unwrap();
        let raw_m = arena.alloc_zeroed_raw(u64::from(n)).unwrap();
        unsafe {
            let p = raw_l.ptr.as_raw().cast::<bf16>().as_ptr();
            for i in 0..n {
                for j in 0..d {
                    *p.add((i * d + j) as usize) = bf16::from_f32(fill_l(i, j));
                }
            }
            let p = raw_a.ptr.as_raw().cast::<f32>().as_ptr();
            for a in 0..k {
                for j in 0..d {
                    *p.add((a * d + j) as usize) = fill_a(a, j);
                }
            }
            let p = raw_m.ptr.as_raw().as_ptr();
            for i in 0..n {
                *p.add(i as usize) = fill_m(i);
            }
        }
        Setup {
            arena,
            raw_l,
            raw_a,
            raw_m,
            n,
            d,
            k,
        }
    }

    #[test]
    fn alpha_zero_identity() {
        let s = setup(2, 3, 1, |_, _| 1.0, |_, _| 5.0, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let anchors = AnchorsView::from_raw(s.raw_a, s.k, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let mut scratch = vec![0.0f32; s.d as usize];
        let z = project_rows(&mut latent, &anchors, &mask, 0.0, &mut scratch).unwrap();
        assert!(!z); // α=0 short-circuits, no zero-weight seen
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        for v in l.row(0).unwrap() {
            assert!((v.to_f32() - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn projection_pulls_toward_anchor() {
        // latent row = [1,0,0], anchor = [1,0,0] → projection towards itself
        // should be identity (cos = 1, w = 1, centroid = anchor).
        let s = setup(
            1,
            3,
            1,
            |_, j| if j == 0 { 1.0 } else { 0.0 },
            |_, j| if j == 0 { 1.0 } else { 0.0 },
            |_| 1,
        );
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let anchors = AnchorsView::from_raw(s.raw_a, s.k, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let mut scratch = vec![0.0f32; s.d as usize];
        let _ = project_rows(&mut latent, &anchors, &mask, 0.5, &mut scratch).unwrap();
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        // Projection onto anchor [1,0,0] with α=0.5 = 0.5*[1,0,0] + 0.5*[1,0,0] = [1,0,0]
        assert!((l.row(0).unwrap()[0].to_f32() - 1.0).abs() < 0.02);
    }

    #[test]
    fn projection_orthogonal_signals_zero_weight() {
        // latent = [1,0,0], anchor = [0,1,0]. Cosine = 0 → total weight = 0
        // → projection returns input unchanged.
        let s = setup(
            1,
            3,
            1,
            |_, j| if j == 0 { 1.0 } else { 0.0 },
            |_, j| if j == 1 { 1.0 } else { 0.0 },
            |_| 1,
        );
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let anchors = AnchorsView::from_raw(s.raw_a, s.k, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let mut scratch = vec![0.0f32; s.d as usize];
        let zero = project_rows(&mut latent, &anchors, &mask, 0.5, &mut scratch).unwrap();
        assert!(zero);
    }
}
