//! [`Tensor<T>`] — the n-dimensional array abstraction.
//!
//! A [`Tensor`] is a cheap clonable handle backed by an [`Arc`] of storage.
//! Clones, slices, transposes and reshapes do not copy the underlying data.
//!
//! # Ownership model
//!
//! The [`TensorStorage`] is immutable behind the `Arc`. Mutation is done
//! via `as_mut_slice`, which attempts `Arc::get_mut` and returns
//! [`ComputeError::Aliased`] if other handles exist.
//!
//! Sprint 1 implements CPU storage only; CUDA storage is a phantom variant
//! behind the `cuda` feature, filled in in Sprint 3.

use std::fmt;
use std::sync::Arc;

use super::device::Device;
use super::dtype::DType;
use super::error::{ComputeError, Result};
use super::shape::{Shape, Strides};

/// N-dimensional tensor handle.
///
/// Cheap to clone: shares storage via `Arc`. The shape, strides and offset
/// are metadata owned by the handle, so different views can share the same
/// buffer.
pub struct Tensor<T: DType> {
    storage: Arc<TensorStorage<T>>,
    shape: Shape,
    strides: Strides,
    /// Offset in *elements* (not bytes) into the backing storage.
    offset: usize,
    device: Device,
}

enum TensorStorage<T: DType> {
    Cpu(Box<[T]>),
    #[cfg(feature = "cuda")]
    #[allow(dead_code)]
    Cuda {
        // Phase 3 fills this with a cudarc::driver::CudaSlice.
        // For now we just carry a placeholder so the enum has two arms.
        device: u32,
        len: usize,
    },
}

impl<T: DType> Tensor<T> {
    // ============================================================
    // Constructors (CPU)
    // ============================================================

    /// Allocate a CPU tensor filled with [`DType::zero`] values.
    pub fn zeros(shape: Shape) -> Result<Self> {
        let n = shape.numel();
        let buf: Box<[T]> = vec![T::zero(); n].into_boxed_slice();
        Ok(Self::from_box(buf, shape))
    }

    /// Allocate a CPU tensor filled with [`DType::one`] values.
    pub fn ones(shape: Shape) -> Result<Self> {
        let n = shape.numel();
        let buf: Box<[T]> = vec![T::one(); n].into_boxed_slice();
        Ok(Self::from_box(buf, shape))
    }

    /// Build a CPU tensor from a slice of data, with the given shape.
    ///
    /// Returns `Err(ShapeMismatch)` if `data.len() != shape.numel()`.
    pub fn from_slice(data: &[T], shape: Shape) -> Result<Self> {
        if data.len() != shape.numel() {
            return Err(ComputeError::ShapeMismatch {
                op: "Tensor::from_slice",
                expected: format!("len={}", shape.numel()),
                actual: format!("len={}", data.len()),
            });
        }
        let buf: Box<[T]> = data.to_vec().into_boxed_slice();
        Ok(Self::from_box(buf, shape))
    }

    /// Internal: build from an already-sized boxed slice.
    fn from_box(buf: Box<[T]>, shape: Shape) -> Self {
        let strides = Strides::contiguous(&shape);
        Tensor {
            storage: Arc::new(TensorStorage::Cpu(buf)),
            shape,
            strides,
            offset: 0,
            device: Device::Cpu,
        }
    }

    // ============================================================
    // Metadata
    // ============================================================

    /// Tensor shape.
    #[inline]
    pub fn shape(&self) -> &Shape {
        &self.shape
    }

    /// Tensor strides.
    #[inline]
    pub fn strides(&self) -> &Strides {
        &self.strides
    }

    /// Device on which the data lives.
    #[inline]
    pub fn device(&self) -> Device {
        self.device
    }

    /// Total number of elements.
    #[inline]
    pub fn numel(&self) -> usize {
        self.shape.numel()
    }

    /// Rank (number of dimensions).
    #[inline]
    pub fn rank(&self) -> usize {
        self.shape.rank()
    }

    /// Element dtype name (e.g. `"f32"`).
    #[inline]
    pub fn dtype_name(&self) -> &'static str {
        T::NAME
    }

    /// Whether the underlying layout is C-order contiguous with offset 0.
    #[inline]
    pub fn is_contiguous(&self) -> bool {
        self.offset == 0 && self.strides.is_contiguous(&self.shape)
    }

    // ============================================================
    // CPU-only accessors
    // ============================================================

    /// Read-only view of the underlying buffer if this tensor is on the CPU
    /// and contiguous at offset 0.
    pub fn as_slice(&self) -> Result<&[T]> {
        if !self.device.is_cpu() {
            return Err(ComputeError::DeviceMismatch {
                tensor_dev: self.device,
                backend_dev: Device::Cpu,
            });
        }
        if !self.is_contiguous() {
            return Err(ComputeError::NotContiguous {
                op: "Tensor::as_slice",
            });
        }
        let buf = match &*self.storage {
            TensorStorage::Cpu(b) => b.as_ref(),
            #[cfg(feature = "cuda")]
            TensorStorage::Cuda { .. } => {
                return Err(ComputeError::DeviceMismatch {
                    tensor_dev: self.device,
                    backend_dev: Device::Cpu,
                })
            }
        };
        Ok(&buf[self.offset..self.offset + self.shape.numel()])
    }

    /// Mutable view of the underlying buffer if this tensor is the sole
    /// owner of its storage (`Arc::get_mut` succeeds).
    pub fn as_mut_slice(&mut self) -> Result<&mut [T]> {
        if !self.device.is_cpu() {
            return Err(ComputeError::DeviceMismatch {
                tensor_dev: self.device,
                backend_dev: Device::Cpu,
            });
        }
        if !self.is_contiguous() {
            return Err(ComputeError::NotContiguous {
                op: "Tensor::as_mut_slice",
            });
        }
        let numel = self.shape.numel();
        let offset = self.offset;
        let storage = Arc::get_mut(&mut self.storage).ok_or(ComputeError::Aliased)?;
        let buf = match storage {
            TensorStorage::Cpu(b) => b.as_mut(),
            #[cfg(feature = "cuda")]
            TensorStorage::Cuda { .. } => {
                return Err(ComputeError::DeviceMismatch {
                    tensor_dev: Device::Cpu,
                    backend_dev: Device::Cpu,
                })
            }
        };
        Ok(&mut buf[offset..offset + numel])
    }

    /// Deep copy into a fresh contiguous buffer, breaking aliasing.
    pub fn to_owned(&self) -> Result<Self> {
        let slice = self.as_slice()?;
        Self::from_slice(slice, self.shape.clone())
    }
}

impl<T: DType> Clone for Tensor<T> {
    fn clone(&self) -> Self {
        Tensor {
            storage: Arc::clone(&self.storage),
            shape: self.shape.clone(),
            strides: self.strides.clone(),
            offset: self.offset,
            device: self.device,
        }
    }
}

impl<T: DType> fmt::Debug for Tensor<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `storage` is deliberately elided: its contents can be arbitrarily
        // large and printing them in Debug would be both expensive and
        // rarely useful. Users who want the raw buffer should call
        // `as_slice()` or `to_owned()`. Storage kind/ref-count are
        // surfaced through `storage_refcount`.
        let storage_refcount = std::sync::Arc::strong_count(&self.storage);
        f.debug_struct("Tensor")
            .field("dtype", &T::NAME)
            .field("shape", &self.shape)
            .field("strides", &self.strides)
            .field("offset", &self.offset)
            .field("device", &self.device)
            .field("storage_refcount", &storage_refcount)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact-representable test values
mod tests {
    use super::*;

    #[test]
    fn zeros_has_correct_shape_and_numel() {
        let t: Tensor<f32> = Tensor::zeros([2, 3].into()).unwrap();
        assert_eq!(t.rank(), 2);
        assert_eq!(t.numel(), 6);
        assert_eq!(t.shape().dims(), &[2, 3]);
        assert!(t.as_slice().unwrap().iter().all(|&x| x == 0.0));
    }

    #[test]
    fn ones_fills_with_ones() {
        let t: Tensor<i32> = Tensor::ones([4].into()).unwrap();
        assert_eq!(t.as_slice().unwrap(), &[1, 1, 1, 1]);
    }

    #[test]
    fn from_slice_rejects_mismatched_length() {
        let err = Tensor::<f32>::from_slice(&[1.0, 2.0], [3].into()).unwrap_err();
        assert!(matches!(err, ComputeError::ShapeMismatch { .. }));
    }

    #[test]
    fn from_slice_accepts_matching_length() {
        let t = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        assert_eq!(t.as_slice().unwrap(), &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn tensor_is_contiguous_after_creation() {
        let t: Tensor<f32> = Tensor::zeros([2, 3, 4].into()).unwrap();
        assert!(t.is_contiguous());
    }

    #[test]
    fn tensor_clone_shares_storage() {
        let t: Tensor<f32> = Tensor::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let t2 = t.clone();
        // Both point to the same data
        assert_eq!(t.as_slice().unwrap(), t2.as_slice().unwrap());
    }

    #[test]
    fn as_mut_slice_fails_when_aliased() {
        let mut t = Tensor::<f32>::zeros([3].into()).unwrap();
        let _alias = t.clone();
        let err = t.as_mut_slice().unwrap_err();
        assert!(matches!(err, ComputeError::Aliased));
    }

    #[test]
    fn as_mut_slice_succeeds_when_unique() {
        let mut t = Tensor::<f32>::zeros([3].into()).unwrap();
        t.as_mut_slice().unwrap()[0] = 42.0;
        assert_eq!(t.as_slice().unwrap()[0], 42.0);
    }

    #[test]
    fn to_owned_breaks_aliasing() {
        let t = Tensor::<f32>::from_slice(&[1.0, 2.0], [2].into()).unwrap();
        let _alias = t.clone();
        let mut owned = t.to_owned().unwrap();
        owned.as_mut_slice().unwrap()[0] = 99.0;
        assert_eq!(t.as_slice().unwrap()[0], 1.0);
        assert_eq!(owned.as_slice().unwrap()[0], 99.0);
    }

    #[test]
    fn tensor_dtype_name() {
        let t: Tensor<f32> = Tensor::zeros([1].into()).unwrap();
        assert_eq!(t.dtype_name(), "f32");
    }

    #[test]
    fn tensor_device_is_cpu() {
        let t: Tensor<f32> = Tensor::zeros([1].into()).unwrap();
        assert_eq!(t.device(), Device::Cpu);
    }
}
