//! [`DType`] trait and implementations for supported tensor element types.
//!
//! In Sprint 1 we ship `f32`, `i32`, `i64`, `u8`. Half-precision types
//! (`f16`, `bf16`) arrive in Sprint 3 together with the CUDA backend.

use std::fmt::Debug;

/// The trait implemented by all scalar types storable in a [`crate::compute::Tensor`].
///
/// Implementors are required to be small, trivially-copyable, and safe to
/// share across threads. They expose their canonical `NAME` (used in error
/// messages and telemetry) and size/alignment hints.
pub trait DType: Copy + Clone + Send + Sync + Debug + 'static {
    /// Canonical human-readable name (`"f32"`, `"i32"`, …).
    const NAME: &'static str;

    /// Byte size of one element.
    const SIZE: usize;

    /// Alignment requirement in bytes.
    const ALIGN: usize;

    /// The additive identity (`0`).
    fn zero() -> Self;

    /// The multiplicative identity (`1`).
    fn one() -> Self;
}

macro_rules! impl_dtype {
    ($t:ty, $name:expr, $zero:expr, $one:expr) => {
        impl DType for $t {
            const NAME: &'static str = $name;
            const SIZE: usize = std::mem::size_of::<$t>();
            const ALIGN: usize = std::mem::align_of::<$t>();

            #[inline]
            fn zero() -> Self {
                $zero
            }

            #[inline]
            fn one() -> Self {
                $one
            }
        }
    };
}

impl_dtype!(f32, "f32", 0.0, 1.0);
impl_dtype!(i32, "i32", 0, 1);
impl_dtype!(i64, "i64", 0, 1);
impl_dtype!(u8, "u8", 0, 1);

#[cfg(test)]
#[allow(clippy::float_cmp)] // testing exactly-representable constants (0.0, 1.0)
mod tests {
    use super::*;

    #[test]
    fn f32_properties() {
        assert_eq!(f32::NAME, "f32");
        assert_eq!(f32::SIZE, 4);
        assert_eq!(f32::ALIGN, 4);
        assert_eq!(f32::zero(), 0.0);
        assert_eq!(f32::one(), 1.0);
    }

    #[test]
    fn i32_properties() {
        assert_eq!(i32::NAME, "i32");
        assert_eq!(i32::SIZE, 4);
        assert_eq!(i32::zero(), 0);
        assert_eq!(i32::one(), 1);
    }

    #[test]
    fn i64_properties() {
        assert_eq!(i64::NAME, "i64");
        assert_eq!(i64::SIZE, 8);
    }

    #[test]
    fn u8_properties() {
        assert_eq!(u8::NAME, "u8");
        assert_eq!(u8::SIZE, 1);
        assert_eq!(u8::ALIGN, 1);
    }
}
