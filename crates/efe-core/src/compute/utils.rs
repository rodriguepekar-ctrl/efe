//! High-level helpers built on top of [`ComputeBackend`].
//!
//! These compose the trait's primitive operations into the patterns
//! that show up repeatedly in the EFE modules. They are deliberately
//! NOT part of the trait — that would bloat the trait surface and
//! force every backend implementation to ship a copy. Helpers here
//! call back into the trait; backends only need to provide the 23
//! primitives.
//!
//! ## Sprint 5 introduction
//!
//! The Sprint 5 cross-module refactor identified two consumers of
//! pairwise cosine similarities — [`alignment::policy`]'s
//! anchor-deduplication invariant (I5) and [`alignment::compose`]'s
//! cone composition. Both walk an `O(N²)` grid of `cos(a_i, a_j)`.
//! Rather than promote `pairwise_cosine` to the trait (which would
//! force a CUDA kernel just for two consumers), we expose it here.

use crate::compute::backend::ComputeBackend;
use crate::compute::error::Result;
use crate::compute::tensor::Tensor;

/// Compute the full `N × M` cosine similarity matrix between two sets
/// of vectors.
///
/// # Inputs
///
/// * `backend` : implementation of [`ComputeBackend`].
/// * `lhs`     : `N` rank-1 vectors, all of the same dimension `D`.
/// * `rhs`     : `M` rank-1 vectors, same dimension `D` as `lhs`.
///
/// When `lhs` and `rhs` point to the same slice (`Arc::ptr_eq` test in
/// callers), the symmetric upper triangle is exploited — caller decides
/// to run the symmetric helper [`pairwise_cosine_symmetric`] instead.
///
/// # Returns
///
/// A row-major `Vec<Vec<f32>>` where `out[i][j] = cos(lhs[i], rhs[j])`.
/// Returning a nested `Vec` keeps the contract idiomatic for the
/// existing `alignment` consumers, which iterate index-by-index.
///
/// # Errors
///
/// Propagates any error from
/// [`ComputeBackend::cosine_similarity_pair`].
pub fn pairwise_cosine(
    backend: &dyn ComputeBackend,
    lhs: &[&[f32]],
    rhs: &[&[f32]],
) -> Result<Vec<Vec<f32>>> {
    let mut out = Vec::with_capacity(lhs.len());
    for a in lhs {
        let ta = Tensor::<f32>::from_slice(a, [a.len()].into())?;
        let mut row = Vec::with_capacity(rhs.len());
        for b in rhs {
            let tb = Tensor::<f32>::from_slice(b, [b.len()].into())?;
            row.push(backend.cosine_similarity_pair(&ta, &tb)?);
        }
        out.push(row);
    }
    Ok(out)
}

/// Symmetric pairwise cosine: returns the strict upper triangle as a
/// flat row-major buffer, with `0.0` on the diagonal.
///
/// This is `~2×` faster than [`pairwise_cosine`] applied with `lhs == rhs`
/// because it skips the redundant lower triangle. Used by the I5
/// invariant in `alignment::policy::EffectivePolicy::validate_against`.
///
/// # Returns
///
/// A row-major `Vec<Vec<f32>>` with `out[i][j] = cos(set[i], set[j])`
/// for `j > i`, and `0.0` for `j ≤ i`.
pub fn pairwise_cosine_symmetric(
    backend: &dyn ComputeBackend,
    set: &[&[f32]],
) -> Result<Vec<Vec<f32>>> {
    let n = set.len();
    let mut out = vec![vec![0.0_f32; n]; n];
    for (i, a_slice) in set.iter().enumerate() {
        let ta = Tensor::<f32>::from_slice(a_slice, [a_slice.len()].into())?;
        for (j, b_slice) in set.iter().enumerate().skip(i + 1) {
            let tb = Tensor::<f32>::from_slice(b_slice, [b_slice.len()].into())?;
            let c = backend.cosine_similarity_pair(&ta, &tb)?;
            out[i][j] = c;
            // Don't fill out[j][i] — caller asks the strict upper
            // triangle by convention. Saves N²/2 writes.
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::CpuBackend;

    #[test]
    fn pairwise_cosine_basic() {
        let b = CpuBackend::new();
        let lhs_data = [vec![1.0_f32, 0.0], vec![0.0, 1.0]];
        let rhs_data = [vec![1.0_f32, 0.0], vec![0.5, 0.5]];
        let lhs: Vec<&[f32]> = lhs_data.iter().map(Vec::as_slice).collect();
        let rhs: Vec<&[f32]> = rhs_data.iter().map(Vec::as_slice).collect();
        let m = pairwise_cosine(&b, &lhs, &rhs).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].len(), 2);
        // cos([1,0], [1,0]) = 1
        assert!((m[0][0] - 1.0).abs() < 1e-6);
        // cos([1,0], [0.5,0.5]) = √2/2
        assert!((m[0][1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        // cos([0,1], [1,0]) = 0
        assert!(m[1][0].abs() < 1e-6);
    }

    #[test]
    fn pairwise_cosine_symmetric_upper_only() {
        let b = CpuBackend::new();
        let set_data = [
            vec![1.0_f32, 0.0],
            vec![0.0, 1.0],
            vec![1.0, 1.0],
        ];
        let set: Vec<&[f32]> = set_data.iter().map(Vec::as_slice).collect();
        let m = pairwise_cosine_symmetric(&b, &set).unwrap();
        // Diagonal must be 0 (uninitialised entries).
        assert!(m[0][0].abs() < 1e-7);
        assert!(m[1][1].abs() < 1e-7);
        assert!(m[2][2].abs() < 1e-7);
        // Upper triangle filled.
        assert!((m[0][1]).abs() < 1e-6); // orthogonal
        assert!((m[0][2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((m[1][2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        // Lower triangle untouched (still 0).
        assert!(m[1][0].abs() < 1e-7);
        assert!(m[2][0].abs() < 1e-7);
        assert!(m[2][1].abs() < 1e-7);
    }
}
