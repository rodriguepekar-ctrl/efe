//! [`Shape`] and [`Strides`] — the geometry of a tensor.
//!
//! A shape is a small vector of non-negative dimension sizes (max rank 6
//! inline). Strides are signed to permit reversed views without copying.

use std::fmt;
use std::ops::Deref;

use smallvec::SmallVec;

use super::error::{ComputeError, Result};

/// Maximum tensor rank stored inline without heap allocation.
pub const MAX_INLINE_RANK: usize = 6;

/// A tensor shape — ordered list of dimension sizes.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Shape(SmallVec<[usize; MAX_INLINE_RANK]>);

impl Shape {
    /// Create a shape from a slice of dimensions.
    #[inline]
    pub fn from_slice(dims: &[usize]) -> Self {
        Shape(SmallVec::from_slice(dims))
    }

    /// Create a new empty (rank-0 / scalar) shape.
    #[inline]
    pub fn scalar() -> Self {
        Shape(SmallVec::new())
    }

    /// Number of dimensions.
    #[inline]
    pub fn rank(&self) -> usize {
        self.0.len()
    }

    /// Total number of elements (product of dimensions; 1 for scalar).
    #[inline]
    pub fn numel(&self) -> usize {
        self.0.iter().product()
    }

    /// Returns the slice of dimensions.
    #[inline]
    pub fn dims(&self) -> &[usize] {
        &self.0
    }

    /// Resolve an axis index (supports negative axes).
    ///
    /// Returns `Err(ComputeError::InvalidAxis)` if the axis is out of range.
    pub fn resolve_axis(&self, axis: i32) -> Result<usize> {
        let rank = self.rank();
        if rank == 0 {
            return Err(ComputeError::InvalidAxis { axis, rank });
        }
        let rank_i = rank as i32;
        let a = if axis < 0 { axis + rank_i } else { axis };
        if a < 0 || a >= rank_i {
            Err(ComputeError::InvalidAxis { axis, rank })
        } else {
            Ok(a as usize)
        }
    }

    /// Return the size of a dimension (supports negative axes).
    pub fn dim(&self, axis: i32) -> Result<usize> {
        let a = self.resolve_axis(axis)?;
        Ok(self.0[a])
    }
}

impl From<&[usize]> for Shape {
    fn from(dims: &[usize]) -> Self {
        Shape::from_slice(dims)
    }
}

impl<const N: usize> From<[usize; N]> for Shape {
    fn from(dims: [usize; N]) -> Self {
        Shape(SmallVec::from_slice(&dims))
    }
}

impl From<Vec<usize>> for Shape {
    fn from(dims: Vec<usize>) -> Self {
        Shape(SmallVec::from_vec(dims))
    }
}

impl Deref for Shape {
    type Target = [usize];
    fn deref(&self) -> &[usize] {
        &self.0
    }
}

impl fmt::Debug for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Shape({:?})", self.dims())
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{d}")?;
        }
        write!(f, "]")
    }
}

/// Strides — number of element steps between consecutive indices along each axis.
#[derive(Clone, PartialEq, Eq)]
pub struct Strides(SmallVec<[isize; MAX_INLINE_RANK]>);

impl Strides {
    /// Build C-order (row-major) strides for a given shape.
    pub fn contiguous(shape: &Shape) -> Self {
        let rank = shape.rank();
        let mut s = SmallVec::<[isize; MAX_INLINE_RANK]>::with_capacity(rank);
        s.resize(rank, 1);
        if rank >= 2 {
            for i in (0..rank - 1).rev() {
                s[i] = s[i + 1] * shape.dims()[i + 1] as isize;
            }
        }
        Strides(s)
    }

    /// Returns the slice of stride values.
    #[inline]
    pub fn as_slice(&self) -> &[isize] {
        &self.0
    }

    /// Check whether these strides are C-order (row-major) contiguous for
    /// the given shape.
    pub fn is_contiguous(&self, shape: &Shape) -> bool {
        *self == Strides::contiguous(shape)
    }
}

impl Deref for Strides {
    type Target = [isize];
    fn deref(&self) -> &[isize] {
        &self.0
    }
}

impl fmt::Debug for Strides {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Strides({:?})", &self.0[..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_from_array() {
        let s: Shape = [2, 3, 4].into();
        assert_eq!(s.rank(), 3);
        assert_eq!(s.numel(), 24);
        assert_eq!(s.dims(), &[2, 3, 4]);
    }

    #[test]
    fn shape_scalar() {
        let s = Shape::scalar();
        assert_eq!(s.rank(), 0);
        assert_eq!(s.numel(), 1);
    }

    #[test]
    fn shape_display() {
        let s: Shape = [2, 3].into();
        assert_eq!(format!("{s}"), "[2, 3]");
    }

    #[test]
    fn shape_resolve_axis_positive() {
        let s: Shape = [2, 3, 4].into();
        assert_eq!(s.resolve_axis(0).unwrap(), 0);
        assert_eq!(s.resolve_axis(2).unwrap(), 2);
    }

    #[test]
    fn shape_resolve_axis_negative() {
        let s: Shape = [2, 3, 4].into();
        assert_eq!(s.resolve_axis(-1).unwrap(), 2);
        assert_eq!(s.resolve_axis(-3).unwrap(), 0);
    }

    #[test]
    fn shape_resolve_axis_out_of_range() {
        let s: Shape = [2, 3].into();
        assert!(matches!(
            s.resolve_axis(5),
            Err(ComputeError::InvalidAxis { axis: 5, rank: 2 })
        ));
        assert!(matches!(
            s.resolve_axis(-5),
            Err(ComputeError::InvalidAxis { axis: -5, rank: 2 })
        ));
    }

    #[test]
    fn strides_contiguous_2d() {
        let s: Shape = [2, 3].into();
        let st = Strides::contiguous(&s);
        // row-major: stride[0] = 3, stride[1] = 1
        assert_eq!(st.as_slice(), &[3, 1]);
    }

    #[test]
    fn strides_contiguous_3d() {
        let s: Shape = [2, 3, 4].into();
        let st = Strides::contiguous(&s);
        assert_eq!(st.as_slice(), &[12, 4, 1]);
    }

    #[test]
    fn strides_contiguous_1d() {
        let s: Shape = [5].into();
        let st = Strides::contiguous(&s);
        assert_eq!(st.as_slice(), &[1]);
    }

    #[test]
    fn strides_is_contiguous_true() {
        let s: Shape = [2, 3].into();
        let st = Strides::contiguous(&s);
        assert!(st.is_contiguous(&s));
    }
}
