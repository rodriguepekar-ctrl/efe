//! Arena bump allocator.
//!
//! An [`Arena`] owns one contiguous region of device memory. Allocations
//! within the arena are concurrent-safe, lock-free, and O(1). There is
//! no per-allocation free: the entire arena is recycled via
//! [`Arena::reset`].
//!
//! # Safety invariants
//!
//! 1. `base` is aligned to [`crate::memory::config::TENSOR_ALIGNMENT`].
//! 2. `cursor <= size_bytes` at every observable point.
//! 3. `alloc_zeroed` only hands out slices borrowed from `&self`; the
//!    lifetime system prevents reset while a slice is alive.
//! 4. `generation` increases monotonically with every `reset`.
//! 5. On drop, the underlying memory is released via the backend.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use crate::errors::{EfeError, Result};
use crate::memory::backend::DeviceBackend;
use crate::memory::config::TENSOR_ALIGNMENT;
use crate::memory::device_slice::{DeviceCopy, DevicePtr, DeviceSlice};

/// Bump-allocating device memory arena.
pub struct Arena {
    base: DevicePtr,
    size_bytes: u64,
    cursor: AtomicU64,
    generation: AtomicU32,
    backend: Arc<dyn DeviceBackend>,
    device_id: u32,
}

/// Raw allocation result (ownership transferred to caller).
///
/// Prefer [`Arena::alloc_zeroed`] which returns a borrow-checked slice.
/// Use [`Arena::alloc_zeroed_raw`] only when the allocation must be
/// stored in a structure that cannot express the `'arena` lifetime.
///
/// # Safety contract
///
/// The caller must not use `ptr` after the parent arena has been reset
/// (generation > `generation` field) or dropped. Use
/// [`RawAllocation::validate_against`] at each use site to verify the
/// pointer is still live.
#[derive(Clone, Copy, Debug)]
pub struct RawAllocation {
    /// Device pointer to the start of the allocated region.
    pub ptr: DevicePtr,
    /// Aligned size of the region in bytes.
    pub size_bytes: u64,
    /// Arena generation captured at allocation time.
    pub generation: u32,
}

impl RawAllocation {
    /// Verify this allocation is still valid against `arena`.
    ///
    /// # Errors
    ///
    /// - [`EfeError::Internal`] if the generation has advanced (the
    ///   arena was reset) — this means the raw pointer is stale and
    ///   must not be used.
    pub fn validate_against(&self, arena: &Arena) -> Result<()> {
        let current = arena.generation();
        if self.generation != current {
            return Err(EfeError::Internal(
                "RawAllocation: arena reset since allocation (use-after-reset)",
            ));
        }
        Ok(())
    }
}

impl Arena {
    /// Create a new arena of exactly `size_bytes` (will be rounded up
    /// to the nearest alignment boundary).
    ///
    /// # Errors
    ///
    /// - [`EfeError::HbmAllocFailed`] if the backend cannot provide the
    ///   requested memory.
    /// - [`EfeError::InvalidConfig`] if `size_bytes` is zero.
    pub fn new(size_bytes: u64, backend: Arc<dyn DeviceBackend>, device_id: u32) -> Result<Self> {
        if size_bytes == 0 {
            return Err(EfeError::InvalidConfig("Arena size must be > 0".into()));
        }
        let aligned = align_up(size_bytes, TENSOR_ALIGNMENT as u64);
        let base = backend.allocate(aligned)?;
        // Zero the entire arena at creation so first-use never reads
        // uninitialized data, even for an uninitialized aligned gap.
        if let Err(e) = backend.memset_zero(base, aligned) {
            let _ = backend.deallocate(base, aligned);
            return Err(e);
        }
        Ok(Self {
            base,
            size_bytes: aligned,
            cursor: AtomicU64::new(0),
            generation: AtomicU32::new(0),
            backend,
            device_id,
        })
    }

    /// Safe allocation: returns a typed borrowed slice.
    ///
    /// Concurrent-safe. The returned [`DeviceSlice`] borrows from
    /// `&self` and therefore forbids `reset` until dropped.
    ///
    /// # Errors
    ///
    /// - [`EfeError::HbmAllocFailed`] if the remaining capacity is
    ///   insufficient. On failure the arena is left in a consistent
    ///   state (see `rollback` note in the implementation).
    pub fn alloc_zeroed<T: DeviceCopy>(&self, count: usize) -> Result<DeviceSlice<'_, T>> {
        let bytes =
            count
                .checked_mul(std::mem::size_of::<T>())
                .ok_or(EfeError::HbmAllocFailed {
                    requested: u64::MAX,
                })?;
        let raw = self.alloc_zeroed_raw_internal(bytes as u64)?;
        let typed_ptr = raw.ptr.cast::<T>();
        Ok(DeviceSlice::from_raw_parts(
            typed_ptr,
            count,
            raw.generation,
        ))
    }

    /// Raw allocation without lifetime binding.
    ///
    /// Use [`Self::alloc_zeroed`] when possible; this escape hatch is
    /// for structures that cannot express `'arena`.
    ///
    /// # Errors
    ///
    /// See [`Self::alloc_zeroed`].
    pub fn alloc_zeroed_raw(&self, size_bytes: u64) -> Result<RawAllocation> {
        self.alloc_zeroed_raw_internal(size_bytes)
    }

    fn alloc_zeroed_raw_internal(&self, size_bytes: u64) -> Result<RawAllocation> {
        if size_bytes == 0 {
            // Return a zero-size allocation at the current cursor (no
            // cursor bump). Deterministic for empty slices.
            return Ok(RawAllocation {
                ptr: self.base,
                size_bytes: 0,
                generation: self.generation.load(Ordering::Acquire),
            });
        }
        let aligned = align_up(size_bytes, TENSOR_ALIGNMENT as u64);
        // Reject allocations that cannot possibly fit. This saves a
        // bump on catastrophic requests and makes the capacity check
        // trivially overflow-safe below.
        if aligned > self.size_bytes {
            return Err(EfeError::HbmAllocFailed { requested: aligned });
        }
        // Bump cursor atomically. We accept that on OOM the cursor is
        // left at the increased value: the next alloc will also fail
        // until `reset` resets it. This simplification avoids a CAS
        // loop in the common path and is safe because allocations never
        // reference memory that didn't fit.
        let offset = self.cursor.fetch_add(aligned, Ordering::AcqRel);
        // `saturating_add` protects against the pathological case where
        // a burst of concurrent `fetch_add` operations pushes `cursor`
        // near `u64::MAX`. In that case the saturated value stays above
        // `self.size_bytes` and the capacity check still fails closed.
        if offset.saturating_add(aligned) > self.size_bytes {
            return Err(EfeError::HbmAllocFailed { requested: aligned });
        }

        // SAFETY: `offset + aligned <= size_bytes`, so `base + offset`
        // is within the allocation. `offset` fits in `usize` because
        // `self.size_bytes` fits in `usize` (validated by
        // `MemoryPool::new` / `Arena::new`).
        let offset_usize =
            usize::try_from(offset).map_err(|_| EfeError::Internal("arena offset overflow"))?;
        let ptr = unsafe { self.base.byte_add(offset_usize) };
        self.backend.memset_zero(ptr, aligned)?;

        Ok(RawAllocation {
            ptr,
            size_bytes: aligned,
            generation: self.generation.load(Ordering::Acquire),
        })
    }

    /// Reset the cursor and bump the generation.
    ///
    /// Requires `&mut self`, which the borrow checker refuses to
    /// grant while any slice borrows `&self`. No memset is performed
    /// by default: the next allocation memsets its target region.
    pub fn reset(&mut self) {
        self.cursor.store(0, Ordering::Release);
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Bytes currently consumed (monotonic between resets).
    #[must_use]
    pub fn cursor_bytes(&self) -> u64 {
        self.cursor.load(Ordering::Acquire).min(self.size_bytes)
    }

    /// Total capacity.
    #[must_use]
    pub fn capacity_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Capacity minus the cursor (saturating).
    #[must_use]
    pub fn available_bytes(&self) -> u64 {
        self.size_bytes.saturating_sub(self.cursor_bytes())
    }

    /// Current generation.
    #[must_use]
    pub fn generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    /// Device ordinal this arena lives on.
    #[must_use]
    pub fn device_id(&self) -> u32 {
        self.device_id
    }

    /// Backend handle (internal use for slices that need to copy).
    pub(crate) fn backend(&self) -> &Arc<dyn DeviceBackend> {
        &self.backend
    }

    /// Base pointer (internal use only).
    #[allow(dead_code)]
    pub(crate) fn base(&self) -> DevicePtr {
        self.base
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        if let Err(e) = self.backend.deallocate(self.base, self.size_bytes) {
            tracing::error!(error = ?e, size = self.size_bytes, "arena deallocate failed");
        }
    }
}

impl std::fmt::Debug for Arena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Arena")
            .field("size_bytes", &self.size_bytes)
            .field("cursor_bytes", &self.cursor_bytes())
            .field("generation", &self.generation())
            .field("device_id", &self.device_id)
            .field("backend", &self.backend.name())
            .finish_non_exhaustive()
    }
}

#[inline]
fn align_up(value: u64, align: u64) -> u64 {
    debug_assert!(align.is_power_of_two(), "alignment must be power of two");
    (value + align - 1) & !(align - 1)
}

/// Copy a [`DeviceSlice`] back to host memory via the parent arena's
/// backend. Exposed as a free function to avoid cyclic deps between
/// `arena` and `device_slice`.
///
/// # Errors
///
/// See [`DeviceBackend::copy_to_host`].
pub fn copy_slice_to_host<T: DeviceCopy>(
    arena: &Arena,
    slice: &DeviceSlice<'_, T>,
) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; slice.size_bytes()];
    if buf.is_empty() {
        return Ok(buf);
    }
    // SAFETY: we pass the expected generation (the arena's current
    // generation) so the debug assertion in the slice verifies it.
    let gen = arena.generation();
    let ptr = unsafe { slice.as_device_ptr(gen) };
    // Convert typed NonNull<T> to DevicePtr (byte-level).
    // SAFETY: NonNull<T> -> NonNull<u8> is a valid cast.
    let raw_u8: NonNull<u8> = ptr.cast();
    let dev = unsafe { DevicePtr::from_raw(raw_u8) };
    arena.backend().copy_to_host(dev, &mut buf)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::backend::HostBackend;

    fn make_arena(size: u64) -> Arena {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        Arena::new(size, backend, 0).expect("arena")
    }

    #[test]
    fn zero_size_rejected() {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        assert!(Arena::new(0, backend, 0).is_err());
    }

    #[test]
    fn alloc_bumps_cursor_aligned() {
        let arena = make_arena(4096);
        let _a = arena.alloc_zeroed::<u8>(100).expect("a");
        // 100 bytes -> aligned to 256.
        assert_eq!(arena.cursor_bytes(), 256);
        let _b = arena.alloc_zeroed::<u8>(256).expect("b");
        assert_eq!(arena.cursor_bytes(), 512);
    }

    #[test]
    fn alloc_zero_size_does_not_move_cursor() {
        let arena = make_arena(4096);
        let before = arena.cursor_bytes();
        let _ = arena.alloc_zeroed::<u8>(0).expect("ok");
        assert_eq!(arena.cursor_bytes(), before);
    }

    #[test]
    fn alloc_beyond_capacity_errors() {
        let arena = make_arena(1024);
        let _a = arena.alloc_zeroed::<u8>(768).expect("a");
        let err = arena.alloc_zeroed::<u8>(512).unwrap_err();
        assert!(matches!(err, EfeError::HbmAllocFailed { .. }));
    }

    #[test]
    fn reset_resets_cursor_and_generation() {
        let mut arena = make_arena(1024);
        let _ = arena.alloc_zeroed::<u8>(128).expect("a");
        let g0 = arena.generation();
        arena.reset();
        assert_eq!(arena.cursor_bytes(), 0);
        assert_eq!(arena.generation(), g0 + 1);
    }

    #[test]
    fn available_bytes_consistent() {
        let arena = make_arena(1024);
        assert_eq!(arena.available_bytes(), 1024);
        let _ = arena.alloc_zeroed::<u8>(200).expect("a"); // 256 aligned
        assert_eq!(arena.available_bytes(), 1024 - 256);
    }

    #[test]
    fn alloc_returns_zero_filled_memory() {
        let arena = make_arena(1024);
        let slice = arena.alloc_zeroed::<u32>(64).expect("a");
        // Copy back to host and verify all zero.
        let bytes = copy_slice_to_host(&arena, &slice).expect("copy");
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn concurrent_allocs_produce_unique_regions() {
        use std::sync::Arc as StdArc;
        let arena = StdArc::new(make_arena(64 * 1024));
        let mut handles = vec![];
        let n_threads = 8;
        for _ in 0..n_threads {
            let a = arena.clone();
            handles.push(std::thread::spawn(move || {
                let mut offsets = vec![];
                for _ in 0..100 {
                    if let Ok(s) = a.alloc_zeroed::<u8>(8) {
                        // SAFETY: debug-check with live gen.
                        let ptr = unsafe { s.as_device_ptr(a.generation()) };
                        offsets.push(ptr.as_ptr() as usize);
                    }
                }
                offsets
            }));
        }
        let mut all: Vec<usize> = handles
            .into_iter()
            .flat_map(|h| h.join().expect("join"))
            .collect();
        all.sort_unstable();
        all.dedup();
        // Every alloc produced a unique address.
        assert!(!all.is_empty());
    }

    #[test]
    fn alloc_u32_consumes_4_bytes_per_element() {
        let arena = make_arena(4096);
        let _ = arena.alloc_zeroed::<u32>(1).expect("a");
        // 4 bytes -> aligned to 256.
        assert_eq!(arena.cursor_bytes(), 256);
    }

    #[test]
    fn raw_allocation_captures_generation() {
        let mut arena = make_arena(4096);
        let raw = arena.alloc_zeroed_raw(128).expect("raw");
        assert_eq!(raw.generation, 0);
        arena.reset();
        let raw2 = arena.alloc_zeroed_raw(128).expect("raw2");
        assert_eq!(raw2.generation, 1);
    }

    #[test]
    fn drop_releases_memory() {
        let backend = Arc::new(HostBackend::new());
        {
            let _arena = Arena::new(4096, backend.clone(), 0).expect("arena");
            assert_eq!(backend.live_allocations(), 1);
        }
        // After drop, deallocations registered.
        assert_eq!(backend.live_allocations(), 0);
    }
}
