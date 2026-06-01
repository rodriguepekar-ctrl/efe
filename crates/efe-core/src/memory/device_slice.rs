//! Typed borrowed views into device memory.
//!
//! A [`DeviceSlice`] is the primary safe abstraction M2 hands out.
//! Callers receive it from [`crate::memory::Arena::alloc_zeroed`] and
//! may hold it as long as the parent arena lives and is not reset
//! (both enforced by the Rust borrow checker).
//!
//! # Safety model
//!
//! 1. **Lifetime**: `DeviceSlice<'a, T>` borrows from the arena for
//!    `'a`. The arena's `reset` requires `&mut self`, so the compiler
//!    refuses to reset while slices are alive.
//! 2. **Generation counter**: each slice captures the arena's
//!    generation at construction. Debug builds assert equality at
//!    `as_device_ptr` time, catching bypasses via `unsafe` code.
//! 3. **Pointer access is unsafe**: `as_device_ptr` is an `unsafe` fn,
//!    forcing callers to document invariants when crossing FFI.

use std::marker::PhantomData;
use std::ptr::NonNull;

/// Marker trait for types safe to copy to / from device memory.
///
/// # Safety
///
/// Implementations must be plain-old-data (POD):
/// - no `Drop` impl
/// - no padding with meaningful bit patterns
/// - no pointers or references (would dangle on the device side)
///
/// In MVP we implement it for primitive numeric types. Sprint 2+ will
/// likely alias it to `bytemuck::Pod` for a richer ecosystem.
pub unsafe trait DeviceCopy: Copy + 'static {}

macro_rules! impl_device_copy {
    ($($t:ty),* $(,)?) => {
        $(
            unsafe impl DeviceCopy for $t {}
        )*
    };
}

impl_device_copy!(u8, u16, u32, u64, i8, i16, i32, i64, f32, f64);

/// Opaque device pointer.
///
/// Wraps a `NonNull<u8>`. On the host backend it points to host memory
/// (dereferenceable), on the CUDA backend it is a device address (NOT
/// dereferenceable on the host). Callers should never assume either
/// and route reads / writes through the backend API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DevicePtr(NonNull<u8>);

// SAFETY: backend enforces thread-safe access to the pointed memory.
unsafe impl Send for DevicePtr {}
unsafe impl Sync for DevicePtr {}

impl DevicePtr {
    /// Construct from a raw non-null pointer.
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the device it targets and must not
    /// be aliased by another `DevicePtr` that could outlive it.
    #[must_use]
    pub unsafe fn from_raw(p: NonNull<u8>) -> Self {
        Self(p)
    }

    /// Access the raw pointer (for kernel launches or FFI).
    ///
    /// # Safety
    ///
    /// See [`DevicePtr::from_raw`]. The returned pointer must not be
    /// dereferenced on the host for device-backed allocations.
    #[must_use]
    pub unsafe fn as_raw(&self) -> NonNull<u8> {
        self.0
    }

    /// Offset the pointer by `bytes`. Caller ensures the result is
    /// still within the owning allocation.
    ///
    /// # Safety
    ///
    /// `bytes` must fit inside the allocation starting at `self`.
    #[must_use]
    pub unsafe fn byte_add(&self, bytes: usize) -> Self {
        // SAFETY: callers promise the offset is in-bounds.
        let ptr = unsafe { self.0.as_ptr().add(bytes) };
        Self(NonNull::new(ptr).expect("offset of NonNull is NonNull"))
    }

    /// Convert to a typed `NonNull<T>`.
    #[must_use]
    pub fn cast<T>(self) -> NonNull<T> {
        self.0.cast()
    }
}

/// Typed borrowed view into a contiguous device region.
///
/// `'arena` ties the slice to its parent arena. `T: DeviceCopy`
/// restricts to POD types.
pub struct DeviceSlice<'arena, T: DeviceCopy> {
    ptr: NonNull<T>,
    len: usize,
    /// Arena generation at construction.
    ///
    /// Debug builds assert equality with the arena's current generation
    /// when `as_device_ptr` is called; this catches manual invalidation
    /// (e.g. an `unsafe` bypass of the borrow checker).
    generation: u32,
    _marker: PhantomData<&'arena [T]>,
}

// SAFETY: the underlying memory is managed by a backend that is itself
// Send + Sync. DeviceSlice is a borrowed view; if `T: Send + Sync`,
// the slice can cross thread boundaries.
unsafe impl<T: DeviceCopy + Send> Send for DeviceSlice<'_, T> {}
unsafe impl<T: DeviceCopy + Sync> Sync for DeviceSlice<'_, T> {}

impl<T: DeviceCopy> DeviceSlice<'_, T> {
    /// Construct from raw parts. Not part of the public API in Sprint 1
    /// (only the `Arena` can produce a slice).
    #[must_use]
    pub(crate) fn from_raw_parts(ptr: NonNull<T>, len: usize, generation: u32) -> Self {
        Self {
            ptr,
            len,
            generation,
            _marker: PhantomData,
        }
    }

    /// Number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the slice is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Size of the slice in bytes.
    #[must_use]
    pub fn size_bytes(&self) -> usize {
        self.len * std::mem::size_of::<T>()
    }

    /// The generation of the arena this slice was allocated from.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Raw device pointer for FFI / kernel launches.
    ///
    /// # Safety
    ///
    /// - On CUDA backends, the pointer is a device address and must
    ///   not be dereferenced on the host.
    /// - The caller must ensure the arena is not reset concurrently.
    ///
    /// Debug builds verify the generation matches `expected_generation`.
    #[must_use]
    pub unsafe fn as_device_ptr(&self, expected_generation: u32) -> NonNull<T> {
        debug_assert_eq!(
            self.generation, expected_generation,
            "DeviceSlice used after its arena was reset: slice_gen={} arena_gen={}",
            self.generation, expected_generation
        );
        self.ptr
    }

    /// Pointer without generation check. Prefer [`Self::as_device_ptr`].
    ///
    /// # Safety
    ///
    /// See [`Self::as_device_ptr`]. This method skips the debug
    /// assertion; use only in hot paths where the caller has otherwise
    /// verified the arena is still live.
    #[must_use]
    pub unsafe fn as_device_ptr_unchecked(&self) -> NonNull<T> {
        self.ptr
    }

    /// Split at `mid`. Both halves share `'arena`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::EfeError::InvalidConfig`] if `mid > len`.
    pub fn split_at(self, mid: usize) -> crate::Result<(Self, Self)> {
        if mid > self.len {
            return Err(crate::EfeError::InvalidConfig(format!(
                "split_at({mid}) out of bounds (len={})",
                self.len
            )));
        }
        let head = Self {
            ptr: self.ptr,
            len: mid,
            generation: self.generation,
            _marker: PhantomData,
        };
        // SAFETY: the offset `mid * size_of::<T>()` is within the
        // original slice because we checked `mid <= len`.
        let tail_ptr = unsafe { NonNull::new_unchecked(self.ptr.as_ptr().add(mid)) };
        let tail = Self {
            ptr: tail_ptr,
            len: self.len - mid,
            generation: self.generation,
            _marker: PhantomData,
        };
        Ok((head, tail))
    }
}

impl<T: DeviceCopy> std::fmt::Debug for DeviceSlice<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceSlice")
            .field("len", &self.len)
            .field("size_bytes", &self.size_bytes())
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_copy_impls_exist() {
        fn accepts<T: DeviceCopy>() {}
        accepts::<u8>();
        accepts::<f32>();
        accepts::<i64>();
    }

    #[test]
    fn slice_size_bytes() {
        // Construct a slice over dummy memory (host-backed).
        let mut buf = vec![0u32; 256];
        let ptr = NonNull::new(buf.as_mut_ptr()).expect("non-null");
        let slice: DeviceSlice<'_, u32> = DeviceSlice::from_raw_parts(ptr, 128, 7);
        assert_eq!(slice.len(), 128);
        assert_eq!(slice.size_bytes(), 128 * 4);
        assert_eq!(slice.generation(), 7);
    }

    #[test]
    fn slice_split_at() {
        let mut buf = vec![0u8; 256];
        let ptr = NonNull::new(buf.as_mut_ptr()).expect("non-null");
        let slice: DeviceSlice<'_, u8> = DeviceSlice::from_raw_parts(ptr, 256, 0);
        let (head, tail) = slice.split_at(64).expect("ok");
        assert_eq!(head.len(), 64);
        assert_eq!(tail.len(), 192);
    }

    #[test]
    fn slice_split_at_oob_returns_err() {
        let mut buf = vec![0u8; 16];
        let ptr = NonNull::new(buf.as_mut_ptr()).expect("non-null");
        let slice: DeviceSlice<'_, u8> = DeviceSlice::from_raw_parts(ptr, 16, 0);
        assert!(slice.split_at(100).is_err());
    }

    #[test]
    fn slice_is_empty() {
        let mut buf = vec![0u8; 16];
        let ptr = NonNull::new(buf.as_mut_ptr()).expect("non-null");
        let empty: DeviceSlice<'_, u8> = DeviceSlice::from_raw_parts(ptr, 0, 0);
        assert!(empty.is_empty());
    }
}
