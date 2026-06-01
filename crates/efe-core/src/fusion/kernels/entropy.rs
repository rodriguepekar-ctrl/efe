//! Row-entropy kernel for attention distributions.
//!
//! Given an `N×N` softmax matrix (row-stochastic) and a mask, computes
//! for each active row `i`:
//!
//! ```text
//! H_i = -Σ_j α_ij * log(α_ij)   (0 * log(0) := 0)
//! ```
//!
//! Returns `(mean, min, max)` over active rows.
//!
//! Consumed by M5's `FusionStats` and ultimately M6. Low entropy means
//! the agent has "polarized" toward a few sources — consensus emerges.
//! High entropy (≈ log(N)) means uniform attention — no decision yet.
//!
//! ## Sprint 5 — façade over `ComputeBackend::shannon_entropy`
//!
//! Pre-Sprint-5 the kernel rolled its own `−Σ p · ln(p)` loop. After
//! Sprint 5 it delegates to [`crate::compute::ComputeBackend::shannon_entropy`]
//! per active row. The mask handling and aggregation (mean / min /
//! max) remain here as domain-specific glue.

use crate::compute::{global_backend, Tensor};
use crate::els::views::MaskView;

/// Compute `(mean, min, max)` of row-entropies over active rows.
///
/// # Preconditions
/// - `softmax.len() == n * n`
/// - `mask.n_agents() == n`
///
/// # Returns
/// - `(0.0, 0.0, 0.0)` if no active rows contribute (edge case).
/// - `(mean, min, max)` in `[0, ln(n)]` otherwise.
pub(crate) fn attention_entropy_stats(
    softmax: &[f32],
    n: usize,
    mask: &MaskView<'_>,
) -> (f32, f32, f32) {
    debug_assert_eq!(
        softmax.len(),
        n.checked_mul(n).expect("N² overflow"),
        "softmax must be N*N"
    );

    let backend = global_backend();
    let mut sum = 0.0f32;
    let mut count = 0u32;
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;

    for i in 0..n {
        if !mask.is_active(i as u32) {
            continue;
        }
        let row = &softmax[i * n..(i + 1) * n];

        // Skip zero-attention rows (all probs 0) — they are treated as
        // identity by aggregate, and their entropy is undefined.
        let row_sum: f32 = row.iter().sum();
        if row_sum < f32::EPSILON {
            continue;
        }

        // Delegate the inner −Σ p · ln(p) computation to the backend.
        // We absorb any backend error (which would only fire on
        // negative or non-finite probabilities — the kernel's
        // upstream softmax already guarantees neither) into the
        // identity contribution `h = 0`, preserving the legacy
        // resilience of this kernel.
        let h = match Tensor::<f32>::from_slice(row, [n].into()) {
            Ok(t) => match backend.shannon_entropy(&t, -1) {
                Ok(out) => out
                    .as_slice()
                    .map(|s| s[0])
                    .unwrap_or(0.0),
                Err(_) => 0.0,
            },
            Err(_) => 0.0,
        };

        sum += h;
        count += 1;
        if h < min {
            min = h;
        }
        if h > max {
            max = h;
        }
    }

    if count == 0 {
        (0.0, 0.0, 0.0)
    } else {
        #[allow(clippy::cast_precision_loss)]
        let mean = sum / count as f32;
        (mean, min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::els::views::MaskView;
    use crate::memory::{Arena, DeviceBackend, HostBackend};
    use std::sync::Arc;

    #[test]
    fn uniform_distribution_has_log_n_entropy() {
        // N=4, softmax = uniform 1/4 → H = ln(4)
        let n = 4;
        let softmax = vec![0.25f32; n * n];
        let active = vec![true; n];
        // We can't easily borrow a leaking mask — use a simpler path:
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 10, backend, 0).unwrap();
        let raw = arena.alloc_zeroed_raw(n as u64).unwrap();
        unsafe {
            let p = raw.ptr.as_raw().as_ptr();
            for (i, a) in active.iter().enumerate() {
                *p.add(i) = u8::from(*a);
            }
        }
        let mask = MaskView::from_raw(raw, n as u32, &arena).unwrap();
        let (mean, min, max) = attention_entropy_stats(&softmax, n, &mask);
        let expected = (n as f32).ln();
        assert!((mean - expected).abs() < 1e-3);
        assert!((min - expected).abs() < 1e-3);
        assert!((max - expected).abs() < 1e-3);
    }

    #[test]
    fn one_hot_distribution_has_zero_entropy() {
        let n = 3;
        // Every row is one-hot at position 0.
        let mut softmax = vec![0.0f32; n * n];
        for i in 0..n {
            softmax[i * n] = 1.0;
        }
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 10, backend, 0).unwrap();
        let raw = arena.alloc_zeroed_raw(n as u64).unwrap();
        unsafe {
            for i in 0..n {
                *raw.ptr.as_raw().as_ptr().add(i) = 1;
            }
        }
        let mask = MaskView::from_raw(raw, n as u32, &arena).unwrap();
        let (mean, min, max) = attention_entropy_stats(&softmax, n, &mask);
        assert!(mean.abs() < 1e-5);
        assert!(min.abs() < 1e-5);
        assert!(max.abs() < 1e-5);
    }

    #[test]
    fn inactive_rows_ignored() {
        let n = 3;
        // Row 0: uniform; Row 1: one-hot; Row 2: inactive (any).
        let mut softmax = vec![1.0 / 3.0f32; n * n];
        softmax[3] = 1.0;
        softmax[4] = 0.0;
        softmax[5] = 0.0;
        // Mask: agent 2 inactive.
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 10, backend, 0).unwrap();
        let raw = arena.alloc_zeroed_raw(3).unwrap();
        unsafe {
            *raw.ptr.as_raw().as_ptr().add(0) = 1;
            *raw.ptr.as_raw().as_ptr().add(1) = 1;
            *raw.ptr.as_raw().as_ptr().add(2) = 0;
        }
        let mask = MaskView::from_raw(raw, 3, &arena).unwrap();
        let (mean, min, max) = attention_entropy_stats(&softmax, n, &mask);
        // Row 0: entropy = ln(3), Row 1: entropy = 0. Mean = ln(3)/2.
        let expected_mean = 3f32.ln() / 2.0;
        assert!((mean - expected_mean).abs() < 1e-3);
        assert!((min - 0.0).abs() < 1e-5);
        assert!((max - 3f32.ln()).abs() < 1e-3);
    }

    #[test]
    fn zero_attention_rows_skipped() {
        let n = 2;
        let softmax = vec![0.0f32; n * n];
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 10, backend, 0).unwrap();
        let raw = arena.alloc_zeroed_raw(n as u64).unwrap();
        unsafe {
            for i in 0..n {
                *raw.ptr.as_raw().as_ptr().add(i) = 1;
            }
        }
        let mask = MaskView::from_raw(raw, n as u32, &arena).unwrap();
        let (mean, min, max) = attention_entropy_stats(&softmax, n, &mask);
        assert!(mean.abs() < f32::EPSILON);
        assert!(min.abs() < f32::EPSILON);
        assert!(max.abs() < f32::EPSILON);
    }

    #[test]
    fn entropy_bounds_are_tight() {
        // Property: 0 ≤ H_i ≤ ln(N).
        let n = 8;
        let mut softmax = vec![0.0f32; n * n];
        for i in 0..n {
            let v = (i as f32).mul_add(0.1, 0.1);
            for j in 0..n {
                softmax[i * n + j] = v / (n as f32 * 0.5 + (i as f32) * 0.1 * n as f32);
            }
            // Normalize to sum 1.
            let s: f32 = softmax[i * n..(i + 1) * n].iter().sum();
            if s > 0.0 {
                for x in &mut softmax[i * n..(i + 1) * n] {
                    *x /= s;
                }
            }
        }
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let arena = Arena::new(1 << 10, backend, 0).unwrap();
        let raw = arena.alloc_zeroed_raw(n as u64).unwrap();
        unsafe {
            for i in 0..n {
                *raw.ptr.as_raw().as_ptr().add(i) = 1;
            }
        }
        let mask = MaskView::from_raw(raw, n as u32, &arena).unwrap();
        let (mean, min, max) = attention_entropy_stats(&softmax, n, &mask);
        let upper = (n as f32).ln() + 1e-3;
        assert!(mean >= 0.0 && mean <= upper);
        assert!(min >= 0.0 && min <= upper);
        assert!(max >= 0.0 && max <= upper);
    }
}
