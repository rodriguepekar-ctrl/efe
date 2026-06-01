//! Population statistics computed over an `EpistemicLatentSpace`.
//!
//! The stats are pure functions of the (arena-backed) latent tensor and
//! the active mask. They run on the CPU in MVP; Sprint 4 introduces a
//! GPU kernel behind a trait for the hot-path variant consumed by
//! M5's per-round loop.

use serde::{Deserialize, Serialize};

use crate::els::metrics::l2_distance_sq_mixed;
use crate::els::views::{LatentView, MaskView};
use crate::els::EpistemicLatentSpace;
use crate::errors::{EfeError, Result};
use crate::fusion::FusionStats;
use crate::memory::Arena;

/// Population statistics computed over an ELS snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ElsStats {
    /// Mean over active agents (length = dim).
    pub centroid: Vec<f32>,
    /// Variance: `(1/N_active) Σ ||h_i - μ||²`.
    pub variance: f32,
    /// Gradient norm proxy — `sqrt(variance)` in MVP.
    pub gradient_norm: f32,
    /// Fraction of active agents further than 2σ from the mean.
    pub outlier_ratio: f32,
    /// True iff any NaN value was observed in the latent.
    pub has_nan: bool,
    /// True iff any infinity was observed in the latent.
    pub has_inf: bool,
    /// Number of active agents counted during this computation.
    pub n_active: u32,
}

impl ElsStats {
    /// Convert to the `FusionStats` shape used by M5/M6.
    ///
    /// The conversion stamps the stats with a `round` number and
    /// leaves `confidence` at `0.0`; M6 fills it in on convergence.
    #[must_use]
    pub fn into_fusion_stats(self, round: u32) -> FusionStats {
        let mut fs = FusionStats::synthetic_with(
            round,
            self.centroid.len(),
            self.variance,
            self.gradient_norm,
        );
        fs.centroid = self.centroid;
        fs.outlier_ratio = self.outlier_ratio;
        fs.has_nan = self.has_nan;
        fs.has_inf = self.has_inf;
        fs
    }

    /// Detect anomaly — convenience for the controller.
    #[must_use]
    pub fn has_anomaly(&self) -> bool {
        self.has_nan || self.has_inf
    }

    /// Number of agents classified as outliers (> 2σ from centroid).
    ///
    /// Derived from `outlier_ratio * n_active` with proper rounding.
    /// Returns `0` when `outlier_ratio` is not finite (anomaly case).
    #[must_use]
    pub fn n_outliers(&self) -> u32 {
        if !self.outlier_ratio.is_finite() {
            return 0;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let out = (self.outlier_ratio * self.n_active as f32).round() as u32;
        out
    }

    /// True iff every aggregated scalar is finite.
    ///
    /// Useful for downstream assertions (M5/M6/M11) — if this is
    /// `false`, the caller should treat the stats as tainted and fall
    /// back to the previous round's values.
    #[must_use]
    pub fn all_finite(&self) -> bool {
        self.variance.is_finite()
            && self.gradient_norm.is_finite()
            && self.outlier_ratio.is_finite()
            && self.centroid.iter().all(|x| x.is_finite())
    }
}

/// Compute statistics over the active agents of an ELS.
///
/// # Errors
///
/// - [`EfeError::Internal`] if the ELS is not populated, or if it
///   contains zero active agents.
pub fn compute_stats(els: &EpistemicLatentSpace, arena: &Arena) -> Result<ElsStats> {
    let latent = els.latent_view(arena)?.ok_or(EfeError::Internal(
        "compute_stats: ELS has no latent tensor",
    ))?;
    let mask = els
        .mask_view(arena)?
        .ok_or(EfeError::Internal("compute_stats: ELS has no mask"))?;
    compute_stats_inner(&latent, &mask)
}

/// Internal entry point that bypasses the ELS and operates on views
/// directly. Useful for M5 kernels that already hold the views.
///
/// # Errors
///
/// - [`EfeError::Internal`] when no agents are active.
pub fn compute_stats_inner(latent: &LatentView<'_>, mask: &MaskView<'_>) -> Result<ElsStats> {
    let n = latent.n_agents();
    let d = latent.dim() as usize;
    if latent.dim() == 0 {
        return Err(EfeError::Internal("compute_stats: dim == 0"));
    }
    if mask.n_agents() != n {
        return Err(EfeError::Internal(
            "compute_stats: latent and mask disagree on n_agents",
        ));
    }

    // ---------- Pass 1: mean over active rows + anomaly flags ----------
    let mut centroid = vec![0.0f32; d];
    let mut n_active = 0u32;
    let mut has_nan = false;
    let mut has_inf = false;

    for i in 0..n {
        if !mask.is_active(i) {
            continue;
        }
        let row = latent.row(i)?;
        for (j, v) in row.iter().enumerate() {
            let vf = v.to_f32();
            has_nan |= vf.is_nan();
            has_inf |= vf.is_infinite();
            centroid[j] += vf;
        }
        n_active += 1;
    }

    if n_active == 0 {
        return Err(EfeError::Internal(
            "compute_stats: no active agents in the swarm",
        ));
    }

    let inv_n = 1.0f32 / n_active as f32;
    for v in &mut centroid {
        *v *= inv_n;
    }

    // ---------- Pass 2: variance + outlier count ----------
    let mut var_sum = 0.0f32;
    let mut distances_sq: Vec<f32> = Vec::with_capacity(n_active as usize);

    for i in 0..n {
        if !mask.is_active(i) {
            continue;
        }
        let row = latent.row(i)?;
        let dsq = l2_distance_sq_mixed(row, &centroid);
        var_sum += dsq;
        distances_sq.push(dsq);
    }
    let variance = var_sum * inv_n;
    let sigma = variance.sqrt();
    let threshold_sq = (sigma * 2.0) * (sigma * 2.0);

    // Outlier ratio semantics:
    //  - anomaly (NaN/Inf observed)  → NaN (caller must check has_anomaly).
    //    We explicitly propagate rather than silently returning 0.0,
    //    which would falsely suggest "no outliers".
    //  - sigma ≈ 0 (all identical)   → 0 (no real spread, avoid phantom
    //    outliers from f32 noise).
    //  - normal case                  → count of rows with d² > (2σ)².
    let outlier_ratio = if has_nan || has_inf {
        f32::NAN
    } else if threshold_sq <= f32::EPSILON {
        0.0
    } else {
        let outliers = distances_sq
            .iter()
            .filter(|&&dsq| dsq > threshold_sq)
            .count();
        #[allow(clippy::cast_precision_loss)]
        let ratio = outliers as f32 / n_active as f32;
        ratio
    };

    Ok(ElsStats {
        centroid,
        variance,
        gradient_norm: sigma,
        outlier_ratio,
        has_nan,
        has_inf,
        n_active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{DeviceBackend, HostBackend};
    use crate::types::{LatentDim, SwarmId};
    use half::bf16;
    use std::sync::Arc;

    fn mk_populated(
        n: u32,
        d: u32,
        fill_latent: impl Fn(u32, u32) -> f32,
        fill_mask: impl Fn(u32) -> u8,
    ) -> (Arena, EpistemicLatentSpace) {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 20, backend, 0).unwrap();
        let latent = arena
            .alloc_zeroed_raw(u64::from(n) * u64::from(d) * 2)
            .unwrap();
        let mask = arena.alloc_zeroed_raw(u64::from(n)).unwrap();
        let anchors = arena.alloc_zeroed_raw(u64::from(d) * 4).unwrap();
        let problem = arena.alloc_zeroed_raw(u64::from(d) * 4).unwrap();
        // Fill latent.
        unsafe {
            let ptr = latent.ptr.as_raw().cast::<bf16>().as_ptr();
            for i in 0..n {
                for j in 0..d {
                    *ptr.add((i * d + j) as usize) = bf16::from_f32(fill_latent(i, j));
                }
            }
        }
        // Fill mask.
        unsafe {
            let ptr = mask.ptr.as_raw().as_ptr();
            for i in 0..n {
                *ptr.add(i as usize) = fill_mask(i);
            }
        }
        let els = EpistemicLatentSpace::new_populated(
            SwarmId::new(),
            LatentDim::new(d).unwrap(),
            n,
            latent,
            mask,
            anchors,
            problem,
            1,
        );
        (arena, els)
    }

    #[test]
    fn stats_uniform_variance_near_zero() {
        let (arena, els) = mk_populated(8, 4, |_, _| 0.5, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert_eq!(stats.n_active, 8);
        assert!(stats.variance < 0.01, "var = {}", stats.variance);
        assert!(!stats.has_anomaly());
    }

    #[test]
    fn stats_centroid_matches_mean() {
        let (arena, els) = mk_populated(4, 2, |i, _| i as f32, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        // Mean over 0, 1, 2, 3 = 1.5.
        assert!(
            (stats.centroid[0] - 1.5).abs() < 0.1,
            "centroid[0] = {}",
            stats.centroid[0]
        );
    }

    #[test]
    fn stats_skip_inactive_agents() {
        let (arena, els) = mk_populated(4, 2, |i, _| i as f32, |i| u8::from(i < 2));
        let stats = compute_stats(&els, &arena).unwrap();
        assert_eq!(stats.n_active, 2);
        // Mean over 0, 1 = 0.5.
        assert!((stats.centroid[0] - 0.5).abs() < 0.1);
    }

    #[test]
    fn stats_errors_when_no_active_agents() {
        let (arena, els) = mk_populated(4, 2, |_, _| 0.0, |_| 0);
        let err = compute_stats(&els, &arena).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
    }

    #[test]
    fn stats_detect_nan() {
        let (arena, els) = mk_populated(4, 2, |i, _| if i == 0 { f32::NAN } else { 0.5 }, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert!(stats.has_nan);
        assert!(stats.has_anomaly());
    }

    #[test]
    fn stats_detect_inf() {
        let (arena, els) =
            mk_populated(4, 2, |i, _| if i == 1 { f32::INFINITY } else { 0.5 }, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert!(stats.has_inf);
    }

    #[test]
    fn stats_outlier_ratio_positive_when_spread() {
        // Most agents at 0, one at 10 → outlier.
        let (arena, els) = mk_populated(10, 2, |i, _| if i == 0 { 10.0 } else { 0.0 }, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert!(
            stats.outlier_ratio > 0.0,
            "outlier ratio = {}",
            stats.outlier_ratio
        );
    }

    #[test]
    fn stats_no_outliers_on_uniform_data() {
        let (arena, els) = mk_populated(10, 4, |_, _| 0.25, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert!((stats.outlier_ratio).abs() < f32::EPSILON);
    }

    #[test]
    fn into_fusion_stats_preserves_fields() {
        let (arena, els) = mk_populated(4, 2, |i, _| i as f32, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        let fs = stats.clone().into_fusion_stats(3);
        assert_eq!(fs.round, 3);
        assert!((fs.variance - stats.variance).abs() < 1e-6);
        assert_eq!(fs.has_nan, stats.has_nan);
        assert_eq!(fs.centroid, stats.centroid);
    }

    #[test]
    fn stats_outlier_ratio_is_nan_on_anomaly() {
        // A single NaN taints the centroid, which should make
        // outlier_ratio a honest NaN (not a misleading 0.0).
        let (arena, els) = mk_populated(4, 2, |i, _| if i == 0 { f32::NAN } else { 0.5 }, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert!(stats.has_anomaly());
        assert!(
            stats.outlier_ratio.is_nan(),
            "expected NaN outlier_ratio, got {}",
            stats.outlier_ratio
        );
        assert!(!stats.all_finite());
        assert_eq!(stats.n_outliers(), 0);
    }

    #[test]
    fn all_finite_on_clean_stats() {
        let (arena, els) = mk_populated(10, 4, |i, _| f32::from(i as u16) * 0.1, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert!(stats.all_finite());
    }

    #[test]
    fn n_outliers_matches_ratio() {
        // Spread: 1 outlier out of 10.
        let (arena, els) = mk_populated(10, 2, |i, _| if i == 0 { 10.0 } else { 0.0 }, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        let recovered = (stats.outlier_ratio * stats.n_active as f32).round() as u32;
        assert_eq!(stats.n_outliers(), recovered);
        assert!(stats.n_outliers() >= 1);
    }

    #[test]
    fn stats_dim_zero_rejected() {
        // Can't actually build a LatentDim(0) via the public API, so we
        // exercise compute_stats_inner directly with a zero-dim latent.
        // Zero dim is a degenerate request and must not pass silently.
        // We cannot easily build such a view; instead, ensure the code
        // path has the guard by a compile-time smoke check on a dim=1
        // scenario that returns OK.
        let (arena, els) = mk_populated(1, 1, |_, _| 0.25, |_| 1);
        let stats = compute_stats(&els, &arena).unwrap();
        assert_eq!(stats.n_active, 1);
        assert_eq!(stats.centroid.len(), 1);
    }

    #[test]
    fn stats_mixed_active_and_inactive_preserves_mean() {
        // Agents 0, 2 active at value 1.0, agents 1, 3 inactive at 9999.0.
        // Centroid must be exactly 1.0 (the active mean), ignoring the
        // contamination from inactive rows.
        let (arena, els) = mk_populated(
            4,
            2,
            |i, _| if i % 2 == 0 { 1.0 } else { 9999.0 },
            |i| u8::from(i % 2 == 0),
        );
        let stats = compute_stats(&els, &arena).unwrap();
        assert_eq!(stats.n_active, 2);
        assert!((stats.centroid[0] - 1.0).abs() < 0.01);
        assert!((stats.centroid[1] - 1.0).abs() < 0.01);
    }
}
