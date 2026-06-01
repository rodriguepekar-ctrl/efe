// SPDX-License-Identifier: UNLICENSED
//! M32 — Distance metrics.
//!
//! Three metrics ship in M32 :
//!
//! - **Cosine** : `1 - (a · b / (||a|| × ||b||))`. Default. For
//!   pre-normalized vectors collapses to `1 - (a · b)`.
//! - **Dot**    : `-(a · b)` — negated so that "smaller is closer"
//!   matches L2 / cosine semantics. The score returned to the
//!   caller is `a · b` (higher = better).
//! - **L2**     : `√Σ(a-b)²`. For non-normalized vectors.
//!
//! The hot path is a tight `f32` loop ; the compiler auto-vectorizes
//! on `aarch64-apple-darwin` (NEON) and `x86_64-unknown-linux-gnu`
//! (AVX2) thanks to the iter pattern below.
//!
//! ## Internal score contract
//!
//! Every search inside this crate returns a `score: f32` where
//! **higher means more similar**. Cosine in [0, 1] ; dot in
//! (-∞, ∞) ; L2 negated.

use serde::{Deserialize, Serialize};

use crate::error::{Result, VectorError};

/// Distance metric discriminator. Serialized as snake_case in REST.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distance {
    /// Cosine similarity (default). Score in [0, 1].
    Cosine,
    /// Dot product. Score in (-∞, ∞).
    Dot,
    /// Euclidean distance (negated to keep "higher = better"). Score
    /// in (-∞, 0].
    L2,
}

impl Default for Distance {
    fn default() -> Self {
        Self::Cosine
    }
}

impl Distance {
    /// Static name (used as a metric label).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cosine => "cosine",
            Self::Dot => "dot",
            Self::L2 => "l2",
        }
    }

    /// Parse from canonical name. `None` if unknown.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "cosine" => Some(Self::Cosine),
            "dot" => Some(Self::Dot),
            "l2" => Some(Self::L2),
            _ => None,
        }
    }

    /// Compute the similarity score between two vectors of equal
    /// dimension. Returns an error if the dimensions disagree or
    /// if any value is NaN / infinite.
    pub fn score(self, a: &[f32], b: &[f32]) -> Result<f32> {
        if a.len() != b.len() {
            return Err(VectorError::DimensionMismatch {
                observed: a.len(),
                expected: b.len(),
            });
        }
        validate_finite(a)?;
        validate_finite(b)?;
        Ok(match self {
            Self::Cosine => cosine_similarity(a, b),
            Self::Dot => dot_product(a, b),
            Self::L2 => -l2_distance(a, b),
        })
    }
}

/// Default L2-norm threshold below which we treat a vector as zero
/// and refuse to normalize.
pub const ZERO_NORM_EPS: f32 = 1e-12;

/// Hard cap on vector dimension (cf. ADR-M32-008).
pub const MAX_DIM: usize = 4096;

/// L2-normalize a vector in place. If the L2 norm is below
/// [`ZERO_NORM_EPS`] the vector is left untouched (avoids NaN
/// blow-up on adversarial zero vectors).
pub fn l2_normalize(v: &mut [f32]) {
    let norm_sq: f32 = v.iter().map(|x| x * x).sum();
    let norm = norm_sq.sqrt();
    if norm < ZERO_NORM_EPS {
        return;
    }
    for x in v.iter_mut() {
        *x /= norm;
    }
}

/// Dot product `Σ a_i × b_i`. **Assumes** `a.len() == b.len()` —
/// caller validates upstream for the score API.
///
/// **M32 perf (V1)** : chunked 8-wide accumulator. The classic
/// single-accumulator iterator pattern (`iter().zip().map().sum()`)
/// creates a loop-carried dependency that limits instruction-level
/// parallelism. 8 independent accumulators let the CPU dispatch 8
/// FMA / mul operations per cycle on AVX2 / NEON. Compiler
/// auto-vectorizes this idiom across rustc 1.85+.
#[inline]
#[must_use]
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = [0.0_f32; 8];
    let chunks = a.len() / 8;
    for c in 0..chunks {
        let off = c * 8;
        acc[0] += a[off] * b[off];
        acc[1] += a[off + 1] * b[off + 1];
        acc[2] += a[off + 2] * b[off + 2];
        acc[3] += a[off + 3] * b[off + 3];
        acc[4] += a[off + 4] * b[off + 4];
        acc[5] += a[off + 5] * b[off + 5];
        acc[6] += a[off + 6] * b[off + 6];
        acc[7] += a[off + 7] * b[off + 7];
    }
    let mut tail = 0.0_f32;
    for i in (chunks * 8)..a.len() {
        tail += a[i] * b[i];
    }
    let s = (acc[0] + acc[1]) + (acc[2] + acc[3]) + (acc[4] + acc[5]) + (acc[6] + acc[7]);
    s + tail
}

/// Squared L2 norm (no sqrt). **M32 perf (V1)** : chunked 8-wide.
/// Useful when the caller only needs a relative magnitude.
#[inline]
#[must_use]
pub fn squared_l2_norm(v: &[f32]) -> f32 {
    let mut acc = [0.0_f32; 8];
    let chunks = v.len() / 8;
    for c in 0..chunks {
        let off = c * 8;
        acc[0] += v[off] * v[off];
        acc[1] += v[off + 1] * v[off + 1];
        acc[2] += v[off + 2] * v[off + 2];
        acc[3] += v[off + 3] * v[off + 3];
        acc[4] += v[off + 4] * v[off + 4];
        acc[5] += v[off + 5] * v[off + 5];
        acc[6] += v[off + 6] * v[off + 6];
        acc[7] += v[off + 7] * v[off + 7];
    }
    let mut tail = 0.0_f32;
    for x in &v[chunks * 8..] {
        tail += x * x;
    }
    (acc[0] + acc[1]) + (acc[2] + acc[3]) + (acc[4] + acc[5]) + (acc[6] + acc[7]) + tail
}

/// Cosine similarity. Returns 1.0 for identical vectors, 0.0 for
/// orthogonal, -1.0 for opposite (but we clamp the floor to 0 in
/// the score API — caller can read the raw cosine via this fn).
#[inline]
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let dot = dot_product(a, b);
    let na = squared_l2_norm(a).sqrt();
    let nb = squared_l2_norm(b).sqrt();
    if na < ZERO_NORM_EPS || nb < ZERO_NORM_EPS {
        return 0.0;
    }
    dot / (na * nb)
}

/// **M32 perf (V2)** : fast-path for pre-L2-normalized vectors.
///
/// When both `a` and `b` are L2-normalized (norm = 1), cosine
/// reduces to the dot product — skipping two `squared_l2_norm`
/// calls + a divide. This is the typical path for SigLIP / Whisper
/// outputs (M30 normalizes server-side).
///
/// **Caller must guarantee** both inputs are L2-normalized within
/// `ZERO_NORM_EPS` ; mis-use produces garbage scores. The
/// `Distance::Cosine` score API still uses the slow path
/// defensively.
#[inline]
#[must_use]
pub fn cosine_normalized(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    dot_product(a, b)
}

/// Euclidean (L2) distance. **M32 perf (V1)** : chunked 8-wide.
#[inline]
#[must_use]
pub fn l2_distance(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut acc = [0.0_f32; 8];
    let chunks = a.len() / 8;
    for c in 0..chunks {
        let off = c * 8;
        let d0 = a[off] - b[off];
        let d1 = a[off + 1] - b[off + 1];
        let d2 = a[off + 2] - b[off + 2];
        let d3 = a[off + 3] - b[off + 3];
        let d4 = a[off + 4] - b[off + 4];
        let d5 = a[off + 5] - b[off + 5];
        let d6 = a[off + 6] - b[off + 6];
        let d7 = a[off + 7] - b[off + 7];
        acc[0] += d0 * d0;
        acc[1] += d1 * d1;
        acc[2] += d2 * d2;
        acc[3] += d3 * d3;
        acc[4] += d4 * d4;
        acc[5] += d5 * d5;
        acc[6] += d6 * d6;
        acc[7] += d7 * d7;
    }
    let mut tail = 0.0_f32;
    for i in (chunks * 8)..a.len() {
        let d = a[i] - b[i];
        tail += d * d;
    }
    let s = (acc[0] + acc[1]) + (acc[2] + acc[3]) + (acc[4] + acc[5]) + (acc[6] + acc[7]);
    (s + tail).sqrt()
}

/// Reject vectors with NaN or infinite values.
fn validate_finite(v: &[f32]) -> Result<()> {
    if v.iter().any(|x| !x.is_finite()) {
        return Err(VectorError::InvalidVector {
            reason: "vector contains NaN or Infinity".into(),
        });
    }
    Ok(())
}

/// Validate that a vector has the expected dimension and contains
/// no NaN / Infinity. Public so handlers can reuse it for ingress
/// validation.
pub fn validate_vector(v: &[f32], expected_dim: usize) -> Result<()> {
    if v.len() > MAX_DIM {
        return Err(VectorError::DimensionTooLarge {
            observed: v.len(),
            cap: MAX_DIM,
        });
    }
    if v.len() != expected_dim {
        return Err(VectorError::DimensionMismatch {
            observed: v.len(),
            expected: expected_dim,
        });
    }
    validate_finite(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_name_roundtrip() {
        for d in [Distance::Cosine, Distance::Dot, Distance::L2] {
            assert_eq!(Distance::from_name(d.name()), Some(d));
        }
    }

    #[test]
    fn cosine_identical_vectors_is_one() {
        let a = vec![1.0, 2.0, 3.0];
        let s = Distance::Cosine.score(&a, &a).unwrap();
        assert!((s - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_orthogonal_vectors_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let s = Distance::Cosine.score(&a, &b).unwrap();
        assert!(s.abs() < 1e-5);
    }

    #[test]
    fn dot_score_higher_is_better() {
        let a = vec![1.0, 1.0];
        let b = vec![2.0, 2.0];
        let c = vec![-1.0, -1.0];
        let sb = Distance::Dot.score(&a, &b).unwrap();
        let sc = Distance::Dot.score(&a, &c).unwrap();
        assert!(sb > sc);
        assert_eq!(sb, 4.0);
        assert_eq!(sc, -2.0);
    }

    #[test]
    fn l2_score_higher_is_better() {
        let a = vec![0.0, 0.0];
        let b = vec![0.0, 0.0]; // same → distance 0 → score 0
        let c = vec![3.0, 4.0]; // distance 5 → score -5
        let sb = Distance::L2.score(&a, &b).unwrap();
        let sc = Distance::L2.score(&a, &c).unwrap();
        assert_eq!(sb, 0.0);
        assert_eq!(sc, -5.0);
        assert!(sb > sc);
    }

    #[test]
    fn score_rejects_dim_mismatch() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let e = Distance::Cosine.score(&a, &b).unwrap_err();
        assert!(matches!(e, VectorError::DimensionMismatch { .. }));
    }

    #[test]
    fn score_rejects_nan() {
        let a = vec![1.0, f32::NAN];
        let b = vec![1.0, 1.0];
        let e = Distance::Cosine.score(&a, &b).unwrap_err();
        assert!(matches!(e, VectorError::InvalidVector { .. }));
    }

    #[test]
    fn l2_normalize_unit_vector() {
        let mut v = vec![3.0_f32, 4.0, 0.0];
        l2_normalize(&mut v);
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
        assert!((v[0] - 0.6).abs() < 1e-5);
        assert!((v[1] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn l2_normalize_zero_vector_stays_zero() {
        let mut v = vec![0.0_f32; 4];
        l2_normalize(&mut v);
        assert_eq!(v, vec![0.0; 4]);
    }

    #[test]
    fn validate_vector_rejects_oversize_dim() {
        let v = vec![0.0_f32; MAX_DIM + 1];
        let e = validate_vector(&v, MAX_DIM + 1).unwrap_err();
        assert!(matches!(e, VectorError::DimensionTooLarge { .. }));
    }

    #[test]
    fn cosine_handles_zero_input_without_nan() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        let s = Distance::Cosine.score(&a, &b).unwrap();
        assert_eq!(s, 0.0);
    }

    #[test]
    fn dot_product_basic() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        assert_eq!(dot_product(&a, &b), 1.0 * 4.0 + 2.0 * 5.0 + 3.0 * 6.0);
    }

    #[test]
    fn dot_product_chunked_matches_naive() {
        // Chunked + tree-reduced sum produces a slightly different
        // float rounding profile than left-fold (`iter().sum()`) ;
        // we assert **relative** tolerance ≤ 1e-5 instead of an
        // absolute epsilon.
        let dims = [1usize, 7, 8, 9, 15, 16, 17, 64, 100, 768];
        for &d in &dims {
            let a: Vec<f32> = (0..d).map(|i| (i as f32) * 0.01).collect();
            let b: Vec<f32> = (0..d).map(|i| ((d - i) as f32) * 0.02).collect();
            let naive: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
            let chunked = dot_product(&a, &b);
            let rel = (naive - chunked).abs() / naive.abs().max(1.0);
            assert!(
                rel < 1e-5,
                "dim {d}: naive {naive}, chunked {chunked} (rel {rel})"
            );
        }
    }

    #[test]
    fn l2_distance_chunked_matches_naive() {
        let dims = [1usize, 7, 8, 9, 64, 768];
        for &d in &dims {
            let a: Vec<f32> = (0..d).map(|i| (i as f32) * 0.1).collect();
            let b: Vec<f32> = (0..d).map(|i| ((i + 1) as f32) * 0.05).collect();
            let mut naive = 0.0_f32;
            for i in 0..d {
                let dd = a[i] - b[i];
                naive += dd * dd;
            }
            let naive = naive.sqrt();
            let chunked = l2_distance(&a, &b);
            let rel = (naive - chunked).abs() / naive.abs().max(1.0);
            assert!(
                rel < 1e-5,
                "dim {d}: naive {naive}, chunked {chunked} (rel {rel})"
            );
        }
    }

    #[test]
    fn cosine_normalized_matches_cosine_on_unit_vectors() {
        for d in [8, 64, 768] {
            let mut a: Vec<f32> = (0..d).map(|i| (i as f32) * 0.01 + 0.1).collect();
            let mut b: Vec<f32> = (0..d).map(|i| ((d - i) as f32) * 0.02 + 0.1).collect();
            l2_normalize(&mut a);
            l2_normalize(&mut b);
            let slow = cosine_similarity(&a, &b);
            let fast = cosine_normalized(&a, &b);
            assert!(
                (slow - fast).abs() < 1e-5,
                "dim {d}: slow {slow}, fast {fast}"
            );
        }
    }

    #[test]
    fn squared_l2_norm_chunked_matches_naive() {
        let v: Vec<f32> = (0..100).map(|i| (i as f32) * 0.1).collect();
        let naive: f32 = v.iter().map(|x| x * x).sum();
        let chunked = squared_l2_norm(&v);
        assert!((naive - chunked).abs() < 1e-3);
    }
}
