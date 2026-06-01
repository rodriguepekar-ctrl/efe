//! `MedianCoordinateExtractor` — per-coordinate median extraction.
//!
//! Robust alternative to the weighted mean: each dimension of the
//! centroid is the median of that dimension across active agents.
//! Complexity `O(N·D·log N)` — heavier than the mean but tolerant to
//! a few outlier agents.

use crate::dissolution::extractors::CentroidExtractor;
use crate::els::views::{LatentView, MaskView};
use crate::errors::{EfeError, Result};

/// Robust extractor: per-coordinate median over active agents.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MedianCoordinateExtractor;

impl CentroidExtractor for MedianCoordinateExtractor {
    fn extract(&self, latent: &LatentView<'_>, mask: &MaskView<'_>) -> Result<Vec<f32>> {
        if mask.n_agents() != latent.n_agents() {
            return Err(EfeError::Internal(
                "MedianCoordinateExtractor: mask and latent disagree on n_agents",
            ));
        }
        let n = latent.n_agents();
        let d = latent.dim() as usize;
        if d == 0 {
            return Err(EfeError::Internal(
                "MedianCoordinateExtractor: latent.dim() == 0",
            ));
        }

        // Pre-collect active rows as f32 to avoid re-casting bf16 per column.
        let mut active_rows: Vec<Vec<f32>> = Vec::new();
        for i in 0..n {
            if !mask.is_active(i) {
                continue;
            }
            let row = latent.row(i)?;
            active_rows.push(row.iter().map(|v| v.to_f32()).collect());
        }
        if active_rows.is_empty() {
            return Err(EfeError::Internal(
                "MedianCoordinateExtractor: no active agents",
            ));
        }

        let mut result = vec![0.0f32; d];
        let mut column: Vec<f32> = Vec::with_capacity(active_rows.len());
        for j in 0..d {
            column.clear();
            // Filter out non-finite entries: they would poison
            // `partial_cmp` → `Ordering::Equal` fallback and produce
            // an arbitrary median. Better to ignore and compute the
            // median of the finite subset.
            for row in &active_rows {
                let v = row[j];
                if v.is_finite() {
                    column.push(v);
                }
            }
            if column.is_empty() {
                // Every agent has NaN/Inf in this coordinate — fail
                // rather than return a bogus median. The collapse
                // engine (M7) should have caught this earlier; if we
                // reach here, the upstream contract is broken.
                return Err(EfeError::Internal(
                    "MedianCoordinateExtractor: all values non-finite in a coordinate",
                ));
            }
            column.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mid = column.len() / 2;
            result[j] = if column.len() % 2 == 0 {
                0.5 * (column[mid - 1] + column[mid])
            } else {
                column[mid]
            };
        }
        Ok(result)
    }

    fn name(&self) -> &'static str {
        "median-coord"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::els::views::{LatentView, MaskView};
    use crate::memory::{Arena, DeviceBackend, HostBackend, RawAllocation};
    use half::bf16;
    use std::sync::Arc;

    fn mk_latent_mask(
        n: u32,
        d: u32,
        fill_l: impl Fn(u32, u32) -> f32,
        fill_m: impl Fn(u32) -> u8,
    ) -> (Arena, RawAllocation, RawAllocation) {
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
        (arena, raw_l, raw_m)
    }

    #[test]
    fn median_on_constant_equals_constant() {
        let (arena, raw_l, raw_m) = mk_latent_mask(5, 3, |_, _| 0.42, |_| 1);
        let latent = LatentView::from_raw(raw_l, 5, 3, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 5, &arena).unwrap();
        let e = MedianCoordinateExtractor;
        let c = e.extract(&latent, &mask).unwrap();
        for v in &c {
            assert!((v - 0.42).abs() < 0.01);
        }
    }

    #[test]
    fn median_of_odd_count_picks_middle() {
        // Values 1, 2, 3, 4, 5 → median = 3.
        let (arena, raw_l, raw_m) = mk_latent_mask(5, 1, |i, _| (i + 1) as f32, |_| 1);
        let latent = LatentView::from_raw(raw_l, 5, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 5, &arena).unwrap();
        let c = MedianCoordinateExtractor.extract(&latent, &mask).unwrap();
        assert!((c[0] - 3.0).abs() < 0.1);
    }

    #[test]
    fn median_of_even_count_averages_middle_pair() {
        // Values 1, 2, 3, 4 → median = 2.5.
        let (arena, raw_l, raw_m) = mk_latent_mask(4, 1, |i, _| (i + 1) as f32, |_| 1);
        let latent = LatentView::from_raw(raw_l, 4, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let c = MedianCoordinateExtractor.extract(&latent, &mask).unwrap();
        assert!((c[0] - 2.5).abs() < 0.1);
    }

    #[test]
    fn median_robust_to_single_outlier() {
        // Values 1, 1, 1, 1, 100 → median = 1 (robust), mean would be 20.8.
        let (arena, raw_l, raw_m) =
            mk_latent_mask(5, 1, |i, _| if i == 4 { 100.0 } else { 1.0 }, |_| 1);
        let latent = LatentView::from_raw(raw_l, 5, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 5, &arena).unwrap();
        let c = MedianCoordinateExtractor.extract(&latent, &mask).unwrap();
        assert!((c[0] - 1.0).abs() < 0.5, "got {}", c[0]);
    }

    #[test]
    fn median_ignores_inactive_agents() {
        // Active: 1, 2. Inactive: 1000. Median over {1, 2} = 1.5.
        let (arena, raw_l, raw_m) = mk_latent_mask(
            3,
            1,
            |i, _| match i {
                0 => 1.0,
                1 => 2.0,
                _ => 1000.0,
            },
            |i| u8::from(i < 2),
        );
        let latent = LatentView::from_raw(raw_l, 3, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 3, &arena).unwrap();
        let c = MedianCoordinateExtractor.extract(&latent, &mask).unwrap();
        assert!((c[0] - 1.5).abs() < 0.1);
    }

    #[test]
    fn median_rejects_no_active() {
        let (arena, raw_l, raw_m) = mk_latent_mask(4, 2, |_, _| 0.0, |_| 0);
        let latent = LatentView::from_raw(raw_l, 4, 2, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let err = MedianCoordinateExtractor
            .extract(&latent, &mask)
            .unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn median_single_agent_equals_that_agent() {
        let (arena, raw_l, raw_m) =
            mk_latent_mask(4, 2, |i, j| (i * 10 + j) as f32, |i| u8::from(i == 2));
        let latent = LatentView::from_raw(raw_l, 4, 2, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let c = MedianCoordinateExtractor.extract(&latent, &mask).unwrap();
        // Only agent 2 active: row = [20, 21].
        assert!((c[0] - 20.0).abs() < 0.5);
        assert!((c[1] - 21.0).abs() < 0.5);
    }

    #[test]
    fn median_filters_nan_and_returns_finite_median() {
        // With NaN filtering, the median over {NaN, 1.0, 1.0} is
        // median over {1.0, 1.0} = 1.0 (finite, deterministic).
        let (arena, raw_l, raw_m) =
            mk_latent_mask(3, 1, |i, _| if i == 0 { f32::NAN } else { 1.0 }, |_| 1);
        let latent = LatentView::from_raw(raw_l, 3, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 3, &arena).unwrap();
        let c = MedianCoordinateExtractor.extract(&latent, &mask).unwrap();
        assert!(c[0].is_finite());
        assert!((c[0] - 1.0).abs() < 0.01);
    }

    #[test]
    fn median_all_nan_in_coord_errors() {
        // All agents have NaN in coordinate 0 → explicit Internal error
        // (not a bogus median).
        let (arena, raw_l, raw_m) = mk_latent_mask(3, 1, |_, _| f32::NAN, |_| 1);
        let latent = LatentView::from_raw(raw_l, 3, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 3, &arena).unwrap();
        let err = MedianCoordinateExtractor
            .extract(&latent, &mask)
            .unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(MedianCoordinateExtractor.name(), "median-coord");
    }
}
