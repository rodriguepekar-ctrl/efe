// SPDX-License-Identifier: UNLICENSED
//! M32 — Score normalization utilities.
//!
//! Used by the weighted hybrid fusion. Two normalizers ship :
//!
//! - **min-max** : `(s - min) / (max - min)` → [0, 1].
//! - **z-score** : `(s - μ) / σ` → unit-variance, zero-mean.
//!
//! Both functions return the **same** Vec<f32> they received,
//! with scores normalized in place.

/// Min-max normalize a slice of scores in place. If `max == min`
/// (all scores equal), every output is set to 1.0 to avoid
/// divide-by-zero while still ranking docs as "equally relevant".
pub fn minmax_normalize(scores: &mut [f32]) {
    if scores.is_empty() {
        return;
    }
    let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
    for &s in scores.iter() {
        if s < min {
            min = s;
        }
        if s > max {
            max = s;
        }
    }
    let range = max - min;
    if range.abs() < f32::EPSILON {
        for s in scores.iter_mut() {
            *s = 1.0;
        }
        return;
    }
    for s in scores.iter_mut() {
        *s = (*s - min) / range;
    }
}

/// Z-score normalize a slice of scores in place. Output mean = 0,
/// std = 1. If `σ` is zero (all equal) every output is set to 0.0.
pub fn zscore_normalize(scores: &mut [f32]) {
    if scores.is_empty() {
        return;
    }
    let n = scores.len() as f32;
    let mean: f32 = scores.iter().sum::<f32>() / n;
    let var: f32 = scores.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / n;
    let std = var.sqrt();
    if std.abs() < f32::EPSILON {
        for s in scores.iter_mut() {
            *s = 0.0;
        }
        return;
    }
    for s in scores.iter_mut() {
        *s = (*s - mean) / std;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minmax_empty_is_noop() {
        let mut v: Vec<f32> = vec![];
        minmax_normalize(&mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn minmax_basic() {
        let mut v = vec![0.0_f32, 5.0, 10.0];
        minmax_normalize(&mut v);
        assert_eq!(v[0], 0.0);
        assert!((v[1] - 0.5).abs() < 1e-6);
        assert_eq!(v[2], 1.0);
    }

    #[test]
    fn minmax_constant_maps_to_one() {
        let mut v = vec![3.0_f32; 5];
        minmax_normalize(&mut v);
        assert!(v.iter().all(|&s| s == 1.0));
    }

    #[test]
    fn zscore_zero_mean_unit_variance() {
        let mut v = vec![1.0_f32, 2.0, 3.0, 4.0, 5.0];
        zscore_normalize(&mut v);
        let mean: f32 = v.iter().sum::<f32>() / v.len() as f32;
        assert!(mean.abs() < 1e-5);
        let var: f32 = v.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / v.len() as f32;
        assert!((var - 1.0).abs() < 1e-5);
    }

    #[test]
    fn zscore_constant_maps_to_zero() {
        let mut v = vec![3.0_f32; 5];
        zscore_normalize(&mut v);
        assert!(v.iter().all(|&s| s == 0.0));
    }
}
