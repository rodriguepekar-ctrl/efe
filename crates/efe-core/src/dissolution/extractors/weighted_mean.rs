//! `WeightedMeanExtractor` — step 1 MVP implementation.
//!
//! Computes a per-coordinate weighted mean over the active agents.
//! The weighting scheme is a knob:
//!
//! - `Uniform` — classical arithmetic mean (default).
//! - `DistanceInverse` — `w_i = 1 / (1 + d²(h_i, μ_raw))`, a single
//!   fixed-point iteration for robustness to outliers (Sprint 8+).

use serde::{Deserialize, Serialize};

use crate::dissolution::extractors::CentroidExtractor;
use crate::els::views::{LatentView, MaskView};
use crate::errors::{EfeError, Result};

/// Weighting policy applied to each active agent before averaging.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WeightingScheme {
    /// `w_i = 1` for every active agent.
    Uniform,
    /// `w_i = 1 / (1 + dist²(h_i, μ_uniform))`. Requires one pass for
    /// the uniform mean + one pass for the weighted mean. Robust to
    /// outliers. (Sprint 8+)
    DistanceInverse,
}

impl Default for WeightingScheme {
    fn default() -> Self {
        Self::Uniform
    }
}

/// MVP extractor: weighted arithmetic mean over active agents.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeightedMeanExtractor {
    /// Weighting scheme.
    pub weighting: WeightingScheme,
}

impl WeightedMeanExtractor {
    /// Construct with the given scheme.
    #[must_use]
    pub fn with_scheme(weighting: WeightingScheme) -> Self {
        Self { weighting }
    }

    fn uniform_mean(latent: &LatentView<'_>, mask: &MaskView<'_>) -> Result<Vec<f32>> {
        let n = latent.n_agents();
        let d = latent.dim() as usize;
        let mut sum = vec![0.0f32; d];
        let mut active = 0u32;
        for i in 0..n {
            if !mask.is_active(i) {
                continue;
            }
            let row = latent.row(i)?;
            for (s, v) in sum.iter_mut().zip(row.iter()) {
                *s += v.to_f32();
            }
            active += 1;
        }
        if active == 0 {
            return Err(EfeError::Internal(
                "WeightedMeanExtractor: no active agents",
            ));
        }
        #[allow(clippy::cast_precision_loss)]
        let inv = 1.0f32 / active as f32;
        for v in &mut sum {
            *v *= inv;
        }
        Ok(sum)
    }

    fn distance_inverse_mean(latent: &LatentView<'_>, mask: &MaskView<'_>) -> Result<Vec<f32>> {
        // Pass 1: compute the uniform mean.
        let mu_uniform = Self::uniform_mean(latent, mask)?;

        // Pass 2: weighted mean with distance-inverse weights.
        //
        // Failsafe: if a row carries a non-finite value, its weight
        // is forced to 0 (we skip it) rather than poisoning the sum.
        // If every weight collapses to ≈ 0 (pathological: every row is
        // either non-finite or infinitely far from the uniform mean),
        // we fallback to the uniform mean we already have — no error,
        // the caller still gets a usable centroid.
        let n = latent.n_agents();
        let d = latent.dim() as usize;
        let mut sum = vec![0.0f32; d];
        let mut total_weight = 0.0f32;
        for i in 0..n {
            if !mask.is_active(i) {
                continue;
            }
            let row = latent.row(i)?;
            let mut dist_sq = 0.0f32;
            let mut row_finite = true;
            for (v, m) in row.iter().zip(mu_uniform.iter()) {
                let vf = v.to_f32();
                if !vf.is_finite() || !m.is_finite() {
                    row_finite = false;
                    break;
                }
                let diff = vf - *m;
                dist_sq += diff * diff;
            }
            if !row_finite || !dist_sq.is_finite() {
                continue;
            }
            let w = 1.0f32 / (1.0f32 + dist_sq);
            if !w.is_finite() {
                continue;
            }
            total_weight += w;
            for (s, v) in sum.iter_mut().zip(row.iter()) {
                *s += w * v.to_f32();
            }
        }
        if total_weight < f32::EPSILON {
            // Fallback: distance-inverse produced no usable weights,
            // but the uniform mean is already computed and valid.
            return Ok(mu_uniform);
        }
        for v in &mut sum {
            *v /= total_weight;
        }
        Ok(sum)
    }
}

impl CentroidExtractor for WeightedMeanExtractor {
    fn extract(&self, latent: &LatentView<'_>, mask: &MaskView<'_>) -> Result<Vec<f32>> {
        if mask.n_agents() != latent.n_agents() {
            return Err(EfeError::Internal(
                "WeightedMeanExtractor: mask and latent disagree on n_agents",
            ));
        }
        if latent.dim() == 0 {
            return Err(EfeError::Internal(
                "WeightedMeanExtractor: latent.dim() == 0",
            ));
        }
        match self.weighting {
            WeightingScheme::Uniform => Self::uniform_mean(latent, mask),
            WeightingScheme::DistanceInverse => Self::distance_inverse_mean(latent, mask),
        }
    }

    fn name(&self) -> &'static str {
        "weighted-mean"
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
    fn uniform_mean_on_constant_data_returns_constant() {
        let (arena, raw_l, raw_m) = mk_latent_mask(8, 4, |_, _| 0.25, |_| 1);
        let latent = LatentView::from_raw(raw_l, 8, 4, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 8, &arena).unwrap();
        let e = WeightedMeanExtractor::default();
        let c = e.extract(&latent, &mask).unwrap();
        assert_eq!(c.len(), 4);
        for v in &c {
            assert!((v - 0.25).abs() < 0.01);
        }
    }

    #[test]
    fn uniform_mean_ignores_inactive_agents() {
        // Active agents filled with 1.0; inactive with 100.0.
        let (arena, raw_l, raw_m) = mk_latent_mask(
            4,
            2,
            |i, _| if i % 2 == 0 { 1.0 } else { 100.0 },
            |i| u8::from(i % 2 == 0),
        );
        let latent = LatentView::from_raw(raw_l, 4, 2, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let e = WeightedMeanExtractor::default();
        let c = e.extract(&latent, &mask).unwrap();
        for v in &c {
            assert!((v - 1.0).abs() < 0.01);
        }
    }

    #[test]
    fn uniform_mean_equals_arithmetic_mean() {
        // Values 0, 1, 2, 3 → mean = 1.5.
        let (arena, raw_l, raw_m) = mk_latent_mask(4, 1, |i, _| i as f32, |_| 1);
        let latent = LatentView::from_raw(raw_l, 4, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let e = WeightedMeanExtractor::default();
        let c = e.extract(&latent, &mask).unwrap();
        assert!((c[0] - 1.5).abs() < 0.1);
    }

    #[test]
    fn rejects_no_active_agents() {
        let (arena, raw_l, raw_m) = mk_latent_mask(4, 2, |_, _| 0.0, |_| 0);
        let latent = LatentView::from_raw(raw_l, 4, 2, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let e = WeightedMeanExtractor::default();
        let err = e.extract(&latent, &mask).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn distance_inverse_robust_to_outliers() {
        // 3 close agents at 1.0, one outlier at 100.0.
        let (arena, raw_l, raw_m) =
            mk_latent_mask(4, 1, |i, _| if i == 3 { 100.0 } else { 1.0 }, |_| 1);
        let latent = LatentView::from_raw(raw_l, 4, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let e = WeightedMeanExtractor::with_scheme(WeightingScheme::DistanceInverse);
        let c = e.extract(&latent, &mask).unwrap();
        // Uniform mean would be (3*1 + 100)/4 = 25.75.
        // Distance-inverse should be much closer to 1.0.
        assert!(c[0] < 10.0, "expected robust centroid, got {}", c[0]);
    }

    #[test]
    fn distance_inverse_equals_uniform_when_no_outliers() {
        let (arena, raw_l, raw_m) = mk_latent_mask(4, 1, |_, _| 1.0, |_| 1);
        let latent = LatentView::from_raw(raw_l, 4, 1, &arena).unwrap();
        let mask = MaskView::from_raw(raw_m, 4, &arena).unwrap();
        let uni = WeightedMeanExtractor::default();
        let rob = WeightedMeanExtractor::with_scheme(WeightingScheme::DistanceInverse);
        let c_uni = uni.extract(&latent, &mask).unwrap();
        let c_rob = rob.extract(&latent, &mask).unwrap();
        for (a, b) in c_uni.iter().zip(c_rob.iter()) {
            assert!((a - b).abs() < 0.01);
        }
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(WeightedMeanExtractor::default().name(), "weighted-mean");
    }
}
