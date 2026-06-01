// SPDX-License-Identifier: UNLICENSED
//! M33 — Score normalization + graph-distance decay.

/// Default decay rate for graph distance. `score = exp(-rate × hops)`.
pub const DEFAULT_DECAY_RATE: f32 = 0.5;

/// Min-max normalize a slice of dense scores in place.
pub fn normalize_dense_scores(scores: &mut [f32]) {
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

/// Exponential graph-distance decay. `hops = 0` → `1.0`.
#[must_use]
pub fn graph_decay(hops: usize, rate: f32) -> f32 {
    (-rate * hops as f32).exp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_empty_is_noop() {
        let mut v: Vec<f32> = vec![];
        normalize_dense_scores(&mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn normalize_basic() {
        let mut v = vec![0.0_f32, 5.0, 10.0];
        normalize_dense_scores(&mut v);
        assert_eq!(v, vec![0.0, 0.5, 1.0]);
    }

    #[test]
    fn normalize_constant_maps_to_one() {
        let mut v = vec![3.0_f32; 5];
        normalize_dense_scores(&mut v);
        assert!(v.iter().all(|&s| s == 1.0));
    }

    #[test]
    fn graph_decay_zero_hops_is_one() {
        assert!((graph_decay(0, 0.5) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn graph_decay_decreases_with_hops() {
        let r = 0.5;
        let s1 = graph_decay(1, r);
        let s2 = graph_decay(2, r);
        let s3 = graph_decay(3, r);
        assert!(s1 > s2);
        assert!(s2 > s3);
        assert!(s1 < 1.0);
        assert!(s3 > 0.0);
    }

    #[test]
    fn graph_decay_higher_rate_decays_faster() {
        let slow = graph_decay(3, 0.2);
        let fast = graph_decay(3, 1.5);
        assert!(slow > fast);
    }
}
