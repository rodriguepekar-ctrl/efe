//! Device memory backend abstraction.
//!
//! A [`DeviceBackend`] implements the primitive operations of allocating,
//! deallocating, zeroing and copying device memory. M2 code paths route
//! every low-level operation through this trait so the rest of the
//! system is agnostic to the actual hardware backing.
//!
//! Two backends are defined:
//!
//! - [`HostBackend`] — MVP default. Backs HBM with page-aligned host
//!   memory, letting the whole system run on a laptop without a GPU.
//! - `CudaBackend` — real CUDA backend, arrives in Sprint 2.
//!
//! The host backend is intentionally slow-free-ish: it synchronously
//! zeroes memory with `memset`, which is fast enough for development
//! and, more importantly, makes concurrency bugs deterministic.

use std::alloc::{alloc, dealloc, Layout};
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::Mutex;

use crate::errors::{EfeError, Result};
use crate::memory::config::TENSOR_ALIGNMENT;
use crate::memory::device_slice::DevicePtr;

/// Low-level device memory operations.
///
/// Implementations must be `Send + Sync`.
pub trait DeviceBackend: Send + Sync + std::fmt::Debug {
    /// Allocate `size_bytes`, aligned to [`TENSOR_ALIGNMENT`].
    ///
    /// The backend is responsible for honoring alignment even if the
    /// underlying OS allocator does not.
    ///
    /// # Errors
    ///
    /// - [`EfeError::HbmAllocFailed`] on OOM.
    /// - [`EfeError::Internal`] on layout errors (should not happen
    ///   because sizes are validated upstream).
    fn allocate(&self, size_bytes: u64) -> Result<DevicePtr>;

    /// Deallocate a region previously returned by [`Self::allocate`].
    ///
    /// Re-deallocation of the same pointer is undefined behavior and
    /// the backend is allowed (but not required) to detect it.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::Internal`] if the pointer was not produced
    /// by this backend.
    fn deallocate(&self, ptr: DevicePtr, size_bytes: u64) -> Result<()>;

    /// Zero `size_bytes` starting at `ptr`.
    ///
    /// On host backends this is synchronous (blocking `memset`).
    /// On real CUDA backends this will be an async `cudaMemsetAsync`
    /// on the default stream.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::KernelFailure`] on CUDA failure.
    fn memset_zero(&self, ptr: DevicePtr, size_bytes: u64) -> Result<()>;

    /// Copy `dst.len()` bytes from device memory at `src` to `dst`.
    ///
    /// On host backends this is a memcpy.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::KernelFailure`] on CUDA failure.
    fn copy_to_host(&self, src: DevicePtr, dst: &mut [u8]) -> Result<()>;

    /// Copy `src.len()` bytes from host `src` to device memory at `dst`.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::KernelFailure`] on CUDA failure.
    fn copy_from_host(&self, src: &[u8], dst: DevicePtr) -> Result<()>;

    /// Backend identity string for logs and telemetry.
    fn name(&self) -> &'static str;
}

/// MVP backend: aligned host memory, behaves like a slow GPU.
///
/// Allocations are tracked in a `Mutex<HashMap>` so we can rebuild the
/// `Layout` at deallocation time (host `dealloc` requires the exact
/// layout used at allocation).
pub struct HostBackend {
    allocations: Mutex<HashMap<usize, Layout>>,
}

impl HostBackend {
    /// Build a fresh host backend with no outstanding allocations.
    #[must_use]
    pub fn new() -> Self {
        Self {
            allocations: Mutex::new(HashMap::new()),
        }
    }

    /// Live allocation count (test helper).
    #[doc(hidden)]
    #[must_use]
    pub fn live_allocations(&self) -> usize {
        self.allocations
            .lock()
            .expect("host backend allocations mutex poisoned")
            .len()
    }
}

impl Default for HostBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HostBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostBackend")
            .field("live_allocations", &self.live_allocations())
            .finish()
    }
}

impl DeviceBackend for HostBackend {
    fn allocate(&self, size_bytes: u64) -> Result<DevicePtr> {
        let size_usize = usize::try_from(size_bytes)
            .map_err(|_| EfeError::InvalidConfig("allocation size overflows usize".into()))?;
        let aligned_size = (size_usize + TENSOR_ALIGNMENT - 1) & !(TENSOR_ALIGNMENT - 1);
        let layout = Layout::from_size_align(aligned_size, TENSOR_ALIGNMENT)
            .map_err(|_| EfeError::Internal("invalid layout for host allocation"))?;

        // SAFETY: `layout.size() > 0` because `aligned_size >= TENSOR_ALIGNMENT`.
        let raw = unsafe { alloc(layout) };
        let ptr = NonNull::new(raw).ok_or(EfeError::HbmAllocFailed {
            requested: aligned_size as u64,
        })?;

        self.allocations
            .lock()
            .expect("host backend allocations mutex poisoned")
            .insert(ptr.as_ptr() as usize, layout);

        // SAFETY: `ptr` is non-null and valid for `layout.size()` bytes.
        let dev = unsafe { DevicePtr::from_raw(ptr) };
        Ok(dev)
    }

    fn deallocate(&self, ptr: DevicePtr, _size_bytes: u64) -> Result<()> {
        // SAFETY: `as_raw` returns the NonNull stored at allocate time.
        let raw = unsafe { ptr.as_raw() };
        let addr = raw.as_ptr() as usize;
        let layout = self
            .allocations
            .lock()
            .expect("host backend allocations mutex poisoned")
            .remove(&addr)
            .ok_or(EfeError::Internal("deallocate: unknown pointer"))?;
        // SAFETY: `raw.as_ptr()` was returned by `alloc(layout)` and
        // has not been deallocated since.
        unsafe { dealloc(raw.as_ptr(), layout) };
        Ok(())
    }

    fn memset_zero(&self, ptr: DevicePtr, size_bytes: u64) -> Result<()> {
        let size = usize::try_from(size_bytes)
            .map_err(|_| EfeError::InvalidConfig("memset size overflows usize".into()))?;
        if size == 0 {
            return Ok(());
        }
        // SAFETY: ptr points to a block of >= size_bytes owned by the host backend.
        unsafe {
            std::ptr::write_bytes(ptr.as_raw().as_ptr(), 0, size);
        }
        Ok(())
    }

    fn copy_to_host(&self, src: DevicePtr, dst: &mut [u8]) -> Result<()> {
        if dst.is_empty() {
            return Ok(());
        }
        // SAFETY: src points to a block of >= dst.len() bytes (caller contract).
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_raw().as_ptr(), dst.as_mut_ptr(), dst.len());
        }
        Ok(())
    }

    fn copy_from_host(&self, src: &[u8], dst: DevicePtr) -> Result<()> {
        if src.is_empty() {
            return Ok(());
        }
        // SAFETY: dst points to a block of >= src.len() bytes (caller contract).
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), dst.as_raw().as_ptr(), src.len());
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "host"
    }
}

impl Drop for HostBackend {
    fn drop(&mut self) {
        let live = self.live_allocations();
        if live > 0 {
            tracing::warn!(
                live_allocations = live,
                "HostBackend dropped with outstanding allocations (leak)"
            );
            // Best-effort cleanup: drain and deallocate anyway.
            let mut guard = self
                .allocations
                .lock()
                .expect("host backend allocations mutex poisoned");
            for (addr, layout) in guard.drain() {
                // SAFETY: each entry was registered by `allocate`.
                unsafe {
                    let raw = std::ptr::NonNull::new_unchecked(addr as *mut u8);
                    dealloc(raw.as_ptr(), layout);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_returns_aligned_pointer() {
        let backend = HostBackend::new();
        let ptr = backend.allocate(1024).expect("alloc");
        // SAFETY: as_raw is safe to read (returns the stored pointer).
        let addr = unsafe { ptr.as_raw().as_ptr() } as usize;
        assert_eq!(addr % TENSOR_ALIGNMENT, 0);
        backend.deallocate(ptr, 1024).expect("dealloc");
    }

    #[test]
    fn alloc_tracks_live_allocations() {
        let backend = HostBackend::new();
        assert_eq!(backend.live_allocations(), 0);
        let p1 = backend.allocate(512).expect("a1");
        let p2 = backend.allocate(1024).expect("a2");
        assert_eq!(backend.live_allocations(), 2);
        backend.deallocate(p1, 512).expect("d1");
        assert_eq!(backend.live_allocations(), 1);
        backend.deallocate(p2, 1024).expect("d2");
        assert_eq!(backend.live_allocations(), 0);
    }

    #[test]
    fn memset_zero_fills() {
        let backend = HostBackend::new();
        let ptr = backend.allocate(64).expect("alloc");
        // Write some non-zero bytes first.
        unsafe {
            std::ptr::write_bytes(ptr.as_raw().as_ptr(), 0xAB, 64);
        }
        backend.memset_zero(ptr, 64).expect("memset");
        let mut buf = vec![0u8; 64];
        backend.copy_to_host(ptr, &mut buf).expect("copy");
        assert!(buf.iter().all(|&b| b == 0));
        backend.deallocate(ptr, 64).expect("dealloc");
    }

    #[test]
    fn copy_roundtrip() {
        let backend = HostBackend::new();
        let ptr = backend.allocate(32).expect("alloc");
        let source: Vec<u8> = (0..32).collect();
        backend.copy_from_host(&source, ptr).expect("upload");
        let mut back = vec![0u8; 32];
        backend.copy_to_host(ptr, &mut back).expect("download");
        assert_eq!(source, back);
        backend.deallocate(ptr, 32).expect("dealloc");
    }

    #[test]
    fn deallocate_unknown_pointer_errors() {
        let backend = HostBackend::new();
        let other = HostBackend::new();
        let p = other.allocate(128).expect("alloc");
        let err = backend.deallocate(p, 128).unwrap_err();
        assert!(matches!(err, EfeError::Internal(_)));
        // Clean up via the correct owner.
        other.deallocate(p, 128).expect("ok");
    }

    #[test]
    fn empty_memset_noop() {
        let backend = HostBackend::new();
        let ptr = backend.allocate(64).expect("alloc");
        backend.memset_zero(ptr, 0).expect("noop");
        backend.deallocate(ptr, 64).expect("dealloc");
    }
}
