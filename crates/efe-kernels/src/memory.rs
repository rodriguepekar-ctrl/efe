//! Device memory allocator with high-water-mark guard — Sprint D28.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 14.2.
//!
//! Refuses allocations once total VRAM usage exceeds 80 % of the
//! detected free pool, surfacing `CudaError::OutOfMemory` instead
//! of letting the driver return a deep-stack failure halfway
//! through a request. The threshold is operator-tunable via
//! `EFE_RUNTIME_GPU_MEMORY_HWM_PERCENT` env var (consumed at boot
//! by `efe-server`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cudarc::driver::{CudaDevice, CudaSlice, DeviceRepr};

use crate::error::CudaError;
use crate::observability;

/// Default high-water mark : 80 % of detected total VRAM.
pub const DEFAULT_HIGH_WATER_PERCENT: u8 = 80;

/// VRAM-aware allocator. One instance per `CudaBackend`.
pub struct DeviceAllocator {
    device: Arc<CudaDevice>,
    /// Current bytes in flight (sum of every live `TrackedSlice`).
    /// Wrapped in `Arc<AtomicU64>` so each `TrackedSlice` can hold
    /// its own clone and decrement on drop without referencing the
    /// allocator itself.
    in_flight_bytes: Arc<AtomicU64>,
    /// Total VRAM available on this device (`cudaMemGetInfo` total).
    total_vram_bytes: u64,
    /// High-water mark : refuse allocs above this many bytes.
    high_water_bytes: u64,
}

impl std::fmt::Debug for DeviceAllocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceAllocator")
            .field("device_ordinal", &self.device.ordinal())
            .field("in_flight_bytes", &self.in_flight_bytes.load(Ordering::Relaxed))
            .field("total_vram_bytes", &self.total_vram_bytes)
            .field("high_water_bytes", &self.high_water_bytes)
            .finish()
    }
}

impl DeviceAllocator {
    /// Construct an allocator with the default 80 % threshold.
    pub fn new(device: Arc<CudaDevice>) -> Result<Self, CudaError> {
        Self::with_threshold_percent(device, DEFAULT_HIGH_WATER_PERCENT)
    }

    /// Construct with an explicit percent (1-95).
    pub fn with_threshold_percent(
        device: Arc<CudaDevice>,
        percent: u8,
    ) -> Result<Self, CudaError> {
        if !(1..=95).contains(&percent) {
            return Err(CudaError::InitFailed(format!(
                "high-water mark percent must be in [1, 95] ; got {percent}"
            )));
        }
        // Probe total VRAM via cudaMemGetInfo through cudarc.
        let (_free, total) = device.memory_info().map_err(|e| {
            CudaError::InitFailed(format!(
                "cudaMemGetInfo on device {}: {e}",
                device.ordinal()
            ))
        })?;
        let high_water_bytes = (total as u128 * u128::from(percent) / 100) as u64;
        Ok(Self {
            device,
            in_flight_bytes: Arc::new(AtomicU64::new(0)),
            total_vram_bytes: total,
            high_water_bytes,
        })
    }

    /// Allocate `len` zero-initialised elements of type `T` on the
    /// device. Returns `CudaError::OutOfMemory` if the allocation
    /// would push `in_flight_bytes` beyond the high-water mark.
    ///
    /// **Audit fix C-A30 (D28 audit pass 3)** — pre-fix the trait
    /// bound was `T: DeviceRepr + Default`. The `Default` bound was
    /// not required by `cudarc::driver::CudaDevice::alloc_zeros<T>`
    /// (which requires only `DeviceRepr`) ; it surfaced as a false
    /// constraint at every callsite. Removed.
    pub fn alloc_zeros<T>(&self, len: usize) -> Result<TrackedSlice<T>, CudaError>
    where
        T: DeviceRepr,
    {
        let bytes = (len * std::mem::size_of::<T>()) as u64;

        // Reserve the budget atomically. `fetch_add` then check ;
        // rollback if too high.
        let prev = self.in_flight_bytes.fetch_add(bytes, Ordering::Relaxed);
        if prev + bytes > self.high_water_bytes {
            self.in_flight_bytes.fetch_sub(bytes, Ordering::Relaxed);
            observability::record_oom(self.device.ordinal() as u32);
            return Err(CudaError::OutOfMemory {
                device: self.device.ordinal() as u32,
                requested_bytes: bytes as usize,
                available_bytes: self.high_water_bytes.saturating_sub(prev) as usize,
            });
        }

        // Allocate via cudarc. **Audit fix C-A29 (D28 audit pass 3)**
        // — pre-fix the error path called `.with_observability("alloc_zeros")`
        // on the `CudaError::OutOfMemory` ; that helper records
        // `efe_gpu_kernel_calls_total{kernel="alloc_zeros", outcome="fail"}`.
        // But the caller's `KernelCallGuard` was scoped over an
        // outer kernel name (e.g. `softmax`) and observed its own
        // `outcome="ok"` at drop time → DOUBLE COUNT for two distinct
        // kernel names while the caller's kernel was actually a
        // failure. Removed `.with_observability` ; the global OOM
        // counter `record_oom(device)` above remains the single
        // source of truth for VRAM exhaustion.
        let slice = match self.device.alloc_zeros::<T>(len) {
            Ok(s) => s,
            Err(_e) => {
                self.in_flight_bytes.fetch_sub(bytes, Ordering::Relaxed);
                observability::record_oom(self.device.ordinal() as u32);
                return Err(CudaError::OutOfMemory {
                    device: self.device.ordinal() as u32,
                    requested_bytes: bytes as usize,
                    available_bytes: 0,
                });
            }
        };

        observability::record_memory_used(
            self.device.ordinal() as u32,
            self.in_flight_bytes.load(Ordering::Relaxed),
        );
        tracing::debug!(
            target: "efe.cuda.alloc",
            device = self.device.ordinal(),
            bytes,
            in_flight = self.in_flight_bytes.load(Ordering::Relaxed),
            "device alloc"
        );
        Ok(TrackedSlice {
            inner: slice,
            bytes,
            device_ordinal: self.device.ordinal() as u32,
            in_flight_bytes: Arc::clone(&self.in_flight_bytes),
        })
    }

    /// Direct host-to-device copy from a host slice into a freshly
    /// allocated device buffer. Convenience for the common case in
    /// `CudaBackend::<op>(host_tensor)` where we copy in, run the
    /// kernel, and copy out.
    pub fn h2d_copy<T>(&self, host: &[T]) -> Result<TrackedSlice<T>, CudaError>
    where
        T: DeviceRepr,
    {
        let mut dev = self.alloc_zeros::<T>(host.len())?;
        self.device
            .htod_sync_copy_into(host, &mut dev.inner)
            .map_err(|e| CudaError::FromCuda {
                code: -1,
                msg: format!("htod_sync_copy_into: {e}"),
            })?;
        Ok(dev)
    }

    /// Reverse direction : copy a device buffer back to a host
    /// `Vec<T>`. Synchronises the default stream implicitly via
    /// cudarc.
    ///
    /// **Note** : the `Default + Clone` trait bounds are required
    /// by `cudarc::driver::CudaDevice::dtoh_sync_copy` for the
    /// `Vec<T>` allocation path. We keep them explicit here.
    pub fn d2h_copy<T>(&self, dev: &CudaSlice<T>) -> Result<Vec<T>, CudaError>
    where
        T: DeviceRepr + Default + Clone,
    {
        self.device
            .dtoh_sync_copy(dev)
            .map_err(|e| CudaError::FromCuda {
                code: -1,
                msg: format!("dtoh_sync_copy: {e}"),
            })
    }

    /// Bytes currently in flight (across all live `TrackedSlice`).
    #[must_use]
    pub fn in_flight_bytes(&self) -> u64 {
        self.in_flight_bytes.load(Ordering::Relaxed)
    }

    /// Total VRAM detected on this device.
    #[must_use]
    pub fn total_vram_bytes(&self) -> u64 {
        self.total_vram_bytes
    }

    /// High-water mark threshold.
    #[must_use]
    pub fn high_water_bytes(&self) -> u64 {
        self.high_water_bytes
    }

    /// Borrow the underlying `Arc<CudaDevice>`. Used by the backend
    /// to spawn streams + cuBLAS handles.
    #[must_use]
    pub fn device(&self) -> &Arc<CudaDevice> {
        &self.device
    }
}

/// Wrapper around `CudaSlice<T>` that decrements the allocator's
/// `in_flight_bytes` counter on drop.
///
/// `TrackedSlice` is the **only** way to obtain a device buffer
/// through `efe-kernels`. Direct `CudaSlice::alloc_zeros` calls
/// would bypass the high-water-mark guard and silently exhaust
/// VRAM ; clippy lint `unwrap_used` plus a code review checklist
/// guard against that drift (DT-K-MEM-2 audit pin).
pub struct TrackedSlice<T: DeviceRepr> {
    inner: CudaSlice<T>,
    bytes: u64,
    device_ordinal: u32,
    in_flight_bytes: Arc<AtomicU64>,
}

impl<T: DeviceRepr> std::fmt::Debug for TrackedSlice<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrackedSlice")
            .field("bytes", &self.bytes)
            .field("device_ordinal", &self.device_ordinal)
            .finish()
    }
}

impl<T: DeviceRepr> TrackedSlice<T> {
    /// Borrow the inner `CudaSlice<T>` for kernel launches.
    #[must_use]
    pub fn as_slice(&self) -> &CudaSlice<T> {
        &self.inner
    }

    /// Mutable borrow of the inner slice.
    pub fn as_mut_slice(&mut self) -> &mut CudaSlice<T> {
        &mut self.inner
    }

    /// Bytes occupied by this slice on the device.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Number of `T` elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// `true` iff zero-length.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.len() == 0
    }
}

impl<T: DeviceRepr> Drop for TrackedSlice<T> {
    fn drop(&mut self) {
        self.in_flight_bytes
            .fetch_sub(self.bytes, Ordering::Relaxed);
        tracing::trace!(
            target: "efe.cuda.alloc",
            device = self.device_ordinal,
            bytes = self.bytes,
            "device dealloc"
        );
    }
}

#[cfg(test)]
mod tests {
    // GPU-only tests. CI matrix `test-gpu-smoke` exercises the
    // actual device path. Here we cover the **constructor invariants**
    // that are independent of an actual GPU.
    use super::DEFAULT_HIGH_WATER_PERCENT;

    #[test]
    fn default_high_water_percent_is_eighty() {
        assert_eq!(DEFAULT_HIGH_WATER_PERCENT, 80);
    }
}
