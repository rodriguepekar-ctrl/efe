//! Clipping + anomaly detection kernel.
//!
//! Two responsibilities in one pass:
//!
//! 1. **Anomaly gate** — if any coordinate is not finite (NaN/Inf),
//!    we return an error. The ELS state is left *as-is* for the caller
//!    to decide (restore from replay, or abort).
//! 2. **Clipping** — coordinates exceeding ±`clip_max` (but finite) are
//!    pulled back to ±`clip_max` and a warning metric is tallied.

use half::bf16;

use crate::els::views::{LatentViewMut, MaskView};
use crate::errors::{EfeError, Result};

/// Metrics emitted by `clip_and_check`.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ClipMetrics {
    /// Number of rows that had at least one coordinate clipped.
    pub clipped_rows: u32,
    /// Largest absolute value observed before clipping.
    pub max_abs: f32,
}

/// Clip each coordinate of each active row to `[-clip_max, clip_max]`.
///
/// # Errors
/// - [`EfeError::NumericalExplosion`] if any coordinate is NaN or Inf.
///   We report `round = 0` as placeholder; the orchestration layer
///   re-wraps with the real round index.
///
/// # Preconditions
/// - `clip_max > 0` (checked by `FusionConfig::validate`).
pub(crate) fn clip_and_check(
    latent: &mut LatentViewMut<'_>,
    mask: &MaskView<'_>,
    clip_max: f32,
) -> Result<ClipMetrics> {
    debug_assert!(clip_max > 0.0 && clip_max.is_finite());
    let n = latent.n_agents();
    let mut clipped_rows = 0u32;
    let mut max_abs = 0.0f32;

    for i in 0..n {
        if !mask.is_active(i) {
            continue;
        }
        let row = latent.row_mut(i).expect("i < n");
        let mut row_clipped = false;

        for r in row.iter_mut() {
            let v = r.to_f32();
            if !v.is_finite() {
                return Err(EfeError::NumericalExplosion { round: 0 });
            }
            let abs = v.abs();
            if abs > max_abs {
                max_abs = abs;
            }
            if v > clip_max {
                *r = bf16::from_f32(clip_max);
                row_clipped = true;
            } else if v < -clip_max {
                *r = bf16::from_f32(-clip_max);
                row_clipped = true;
            }
        }

        if row_clipped {
            clipped_rows += 1;
        }
    }

    Ok(ClipMetrics {
        clipped_rows,
        max_abs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::els::views::{LatentView, LatentViewMut, MaskView};
    use crate::memory::{Arena, DeviceBackend, HostBackend, RawAllocation};
    use std::sync::Arc;

    struct Setup {
        arena: Arena,
        raw_l: RawAllocation,
        raw_m: RawAllocation,
        n: u32,
        d: u32,
    }

    fn setup(
        n: u32,
        d: u32,
        fill_l: impl Fn(u32, u32) -> f32,
        fill_m: impl Fn(u32) -> u8,
    ) -> Setup {
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
        Setup {
            arena,
            raw_l,
            raw_m,
            n,
            d,
        }
    }

    #[test]
    fn clips_positive_overflow() {
        let s = setup(2, 2, |i, _| if i == 0 { 50.0 } else { 1.0 }, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let m = clip_and_check(&mut latent, &mask, 32.0).unwrap();
        assert_eq!(m.clipped_rows, 1);
        assert!(m.max_abs >= 48.0); // bf16 round-trip of 50.0 ≈ 50
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        assert!((l.row(0).unwrap()[0].to_f32() - 32.0).abs() < 0.2);
        assert!((l.row(1).unwrap()[0].to_f32() - 1.0).abs() < 0.01);
    }

    #[test]
    fn clips_negative_overflow() {
        let s = setup(1, 2, |_, _| -50.0, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let m = clip_and_check(&mut latent, &mask, 32.0).unwrap();
        assert_eq!(m.clipped_rows, 1);
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        for v in l.row(0).unwrap() {
            assert!((v.to_f32() + 32.0).abs() < 0.2);
        }
    }

    #[test]
    fn leaves_in_range_values_untouched() {
        let s = setup(2, 2, |_, _| 5.0, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let m = clip_and_check(&mut latent, &mask, 32.0).unwrap();
        assert_eq!(m.clipped_rows, 0);
        let l = LatentView::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        for i in 0..2 {
            for v in l.row(i).unwrap() {
                assert!((v.to_f32() - 5.0).abs() < 0.1);
            }
        }
    }

    #[test]
    fn detects_nan() {
        let s = setup(1, 2, |_, _| f32::NAN, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let err = clip_and_check(&mut latent, &mask, 32.0).unwrap_err();
        assert!(matches!(err, EfeError::NumericalExplosion { .. }));
    }

    #[test]
    fn detects_inf() {
        let s = setup(1, 2, |_, _| f32::INFINITY, |_| 1);
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let err = clip_and_check(&mut latent, &mask, 32.0).unwrap_err();
        assert!(matches!(err, EfeError::NumericalExplosion { .. }));
    }

    #[test]
    fn ignores_inactive_rows_even_with_nan() {
        // Inactive row with NaN must not trigger anomaly — kernel only
        // cares about active rows.
        let s = setup(
            2,
            2,
            |i, _| if i == 0 { 1.0 } else { f32::NAN },
            |i| u8::from(i == 0),
        );
        let mut latent = LatentViewMut::from_raw(s.raw_l, s.n, s.d, &s.arena).unwrap();
        let mask = MaskView::from_raw(s.raw_m, s.n, &s.arena).unwrap();
        let m = clip_and_check(&mut latent, &mask, 32.0).unwrap();
        assert_eq!(m.clipped_rows, 0);
    }
}
