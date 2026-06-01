//! CPU reduction kernels.
//!
//! * **Sprint 1** : `mean`, `argmax`
//! * **Sprint 3** : `std_dev`

use crate::compute::error::{ComputeError, Result};
use crate::compute::shape::Shape;
use crate::compute::tensor::Tensor;

/// Reduce along `axis` with a closure producing a scalar per row.
fn reduce_to_scalar<F, T>(
    x: &Tensor<f32>,
    axis: i32,
    f: F,
) -> Result<(Vec<T>, Shape)>
where
    F: Fn(&[f32]) -> T,
{
    let shape = x.shape();
    let rank = shape.rank();
    if rank == 0 {
        return Err(ComputeError::InvalidAxis { axis, rank });
    }
    let a = shape.resolve_axis(axis)?;
    let dims = shape.dims();
    let src = x.as_slice()?;

    // Output = shape minus axis `a`.
    let out_dims: Vec<usize> = dims
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != a)
        .map(|(_, &d)| d)
        .collect();
    let out_shape: Shape = out_dims.clone().into();
    let out_numel = out_shape.numel();

    // Source row-major strides.
    let mut src_strides = vec![1usize; rank];
    if rank >= 2 {
        for i in (0..rank - 1).rev() {
            src_strides[i] = src_strides[i + 1] * dims[i + 1];
        }
    }

    // Strides over the reduced shape (row-major).
    let reduced_rank = out_dims.len();
    let mut reduced_strides = vec![1usize; reduced_rank.max(1)];
    if reduced_rank >= 2 {
        for i in (0..reduced_rank - 1).rev() {
            reduced_strides[i] = reduced_strides[i + 1] * out_dims[i + 1];
        }
    }

    let dim_a = dims[a];
    let mut out: Vec<T> = Vec::with_capacity(out_numel);
    let mut gathered = vec![0.0f32; dim_a];

    for out_idx in 0..out_numel {
        // Decompose out_idx into coordinates over non-axis dims.
        let mut coords = vec![0usize; rank];
        let mut remaining = out_idx;
        let mut idx = 0;
        for (i, _) in dims.iter().enumerate() {
            if i == a {
                continue;
            }
            let stride = if reduced_rank == 0 {
                1
            } else {
                reduced_strides[idx]
            };
            coords[i] = if stride == 0 { 0 } else { remaining / stride };
            if stride > 0 {
                remaining %= stride;
            }
            idx += 1;
        }
        for k in 0..dim_a {
            coords[a] = k;
            let mut flat = 0usize;
            for (i, &c) in coords.iter().enumerate() {
                flat += c * src_strides[i];
            }
            gathered[k] = src[flat];
        }
        out.push(f(&gathered));
    }

    Ok((out, out_shape))
}

/// Mean along `axis`.
pub fn mean(x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
    let (buf, out_shape) = reduce_to_scalar(x, axis, |row| {
        let n = row.len() as f32;
        row.iter().sum::<f32>() / n
    })?;
    Tensor::from_slice(&buf, out_shape)
}

/// Standard deviation along `axis`.
///
/// # Mathematical definition
///
/// ```text
///     μ  = (1/N) · Σ x_i
///     σ² = (1/D) · Σ (x_i − μ)²      where D = N−1 if unbiased, else N
///     σ  = √σ²
/// ```
///
/// Using `unbiased = true` applies Bessel's correction to produce an
/// unbiased estimator of the population variance from a sample. This
/// matches `numpy.std(x, ddof=1)` and `torch.std(x, correction=1)`.
///
/// # Edge cases
///
/// | N | unbiased | result |
/// |---|:-------:|--------|
/// | 0 | either | `NaN` (empty reduction is ill-defined) |
/// | 1 | true | `NaN` (divide by 0) |
/// | 1 | false | `0.0` |
/// | ≥ 2 | either | standard value |
///
/// # Invariants
///
/// * `std_dev(constant, unbiased=false) = 0`
/// * `std_dev(x + c) = std_dev(x)` — shift invariant
/// * `std_dev(α·x) = |α| · std_dev(x)` — scale linearity
pub fn std_dev(x: &Tensor<f32>, axis: i32, unbiased: bool) -> Result<Tensor<f32>> {
    let (buf, out_shape) = reduce_to_scalar(x, axis, |row| {
        let n = row.len();
        if n == 0 {
            return f32::NAN;
        }
        if n == 1 {
            return if unbiased { f32::NAN } else { 0.0 };
        }
        let n_f = n as f32;
        let mean: f32 = row.iter().sum::<f32>() / n_f;
        let sum_sq_dev: f32 = row.iter().map(|v| (v - mean).powi(2)).sum();
        let divisor = if unbiased { n_f - 1.0 } else { n_f };
        (sum_sq_dev / divisor).sqrt()
    })?;
    Tensor::from_slice(&buf, out_shape)
}

/// Argmax along `axis`.
///
/// Ties are broken by returning the smallest index.
pub fn argmax(x: &Tensor<f32>, axis: i32) -> Result<Tensor<i32>> {
    let (buf, out_shape) = reduce_to_scalar(x, axis, |row| {
        let mut best_i = 0usize;
        let mut best_v = row[0];
        for (i, &v) in row.iter().enumerate().skip(1) {
            if v > best_v {
                best_v = v;
                best_i = i;
            }
        }
        best_i as i32
    })?;
    Tensor::from_slice(&buf, out_shape)
}

#[cfg(test)]
#[allow(clippy::float_cmp, clippy::excessive_precision, clippy::approx_constant)]
mod tests {
    use super::*;

    #[test]
    fn mean_1d() {
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
        let m = mean(&x, -1).unwrap();
        assert_eq!(m.rank(), 0);
        assert!((m.as_slice().unwrap()[0] - 2.5).abs() < 1e-5);
    }

    #[test]
    fn mean_2d_axis_last() {
        let x = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], // [[1,2,3],[4,5,6]]
            [2, 3].into(),
        )
        .unwrap();
        let m = mean(&x, -1).unwrap();
        assert_eq!(m.shape().dims(), &[2]);
        let s = m.as_slice().unwrap();
        assert!((s[0] - 2.0).abs() < 1e-5);
        assert!((s[1] - 5.0).abs() < 1e-5);
    }

    #[test]
    fn mean_2d_axis_first() {
        let x = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [2, 3].into(),
        )
        .unwrap();
        let m = mean(&x, 0).unwrap();
        assert_eq!(m.shape().dims(), &[3]);
        let s = m.as_slice().unwrap();
        assert!((s[0] - 2.5).abs() < 1e-5);
        assert!((s[1] - 3.5).abs() < 1e-5);
        assert!((s[2] - 4.5).abs() < 1e-5);
    }

    #[test]
    fn argmax_1d() {
        let x = Tensor::<f32>::from_slice(&[0.1, 0.9, 0.3, 0.2], [4].into()).unwrap();
        let a = argmax(&x, -1).unwrap();
        assert_eq!(a.as_slice().unwrap(), &[1]);
    }

    #[test]
    fn argmax_1d_tie_breaking_to_first() {
        let x = Tensor::<f32>::from_slice(&[0.5, 0.5, 0.5], [3].into()).unwrap();
        let a = argmax(&x, -1).unwrap();
        assert_eq!(a.as_slice().unwrap(), &[0]);
    }

    #[test]
    fn argmax_2d() {
        let x = Tensor::<f32>::from_slice(
            &[0.1, 0.5, 0.4, 0.8, 0.1, 0.1], // row 0 argmax=1, row 1 argmax=0
            [2, 3].into(),
        )
        .unwrap();
        let a = argmax(&x, -1).unwrap();
        assert_eq!(a.as_slice().unwrap(), &[1, 0]);
    }

    #[test]
    fn argmax_rejects_bad_axis() {
        let x = Tensor::<f32>::from_slice(&[1.0], [1].into()).unwrap();
        let err = argmax(&x, 3).unwrap_err();
        assert!(matches!(err, ComputeError::InvalidAxis { .. }));
    }

    // ===== Sprint 3 : std_dev =====

    #[test]
    fn std_dev_constant_is_zero() {
        let x = Tensor::<f32>::from_slice(&[5.0, 5.0, 5.0, 5.0], [4].into()).unwrap();
        let y = std_dev(&x, -1, false).unwrap();
        assert!(y.as_slice().unwrap()[0].abs() < 1e-6);
    }

    #[test]
    fn std_dev_unbiased_matches_numpy() {
        // numpy.std([1,2,3,4,5], ddof=1) = 1.5811388
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], [5].into()).unwrap();
        let y = std_dev(&x, -1, true).unwrap();
        assert!((y.as_slice().unwrap()[0] - 1.581_138_8).abs() < 1e-5);
    }

    #[test]
    fn std_dev_biased_matches_numpy() {
        // numpy.std([1,2,3,4,5], ddof=0) = 1.4142135
        let x = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0, 5.0], [5].into()).unwrap();
        let y = std_dev(&x, -1, false).unwrap();
        assert!((y.as_slice().unwrap()[0] - 1.414_213_5).abs() < 1e-5);
    }

    #[test]
    fn std_dev_shift_invariant() {
        let x1 = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0, 4.0], [4].into()).unwrap();
        let x2 = Tensor::<f32>::from_slice(&[101.0, 102.0, 103.0, 104.0], [4].into()).unwrap();
        let s1 = std_dev(&x1, -1, true).unwrap().as_slice().unwrap()[0];
        let s2 = std_dev(&x2, -1, true).unwrap().as_slice().unwrap()[0];
        assert!((s1 - s2).abs() < 1e-5);
    }

    #[test]
    fn std_dev_single_element_unbiased_is_nan() {
        let x = Tensor::<f32>::from_slice(&[42.0], [1].into()).unwrap();
        let y = std_dev(&x, -1, true).unwrap();
        assert!(y.as_slice().unwrap()[0].is_nan());
    }

    #[test]
    fn std_dev_single_element_biased_is_zero() {
        let x = Tensor::<f32>::from_slice(&[42.0], [1].into()).unwrap();
        let y = std_dev(&x, -1, false).unwrap();
        assert!(y.as_slice().unwrap()[0].abs() < 1e-6);
    }

    #[test]
    fn std_dev_2d_axis_last() {
        // x = [[1,2,3],[4,5,6]], std(axis=-1, ddof=1) = [1.0, 1.0]
        let x = Tensor::<f32>::from_slice(
            &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0],
            [2, 3].into(),
        )
        .unwrap();
        let y = std_dev(&x, -1, true).unwrap();
        let s = y.as_slice().unwrap();
        assert!((s[0] - 1.0).abs() < 1e-5);
        assert!((s[1] - 1.0).abs() < 1e-5);
    }
}
