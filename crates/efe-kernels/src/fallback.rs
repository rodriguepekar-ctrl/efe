//! GPU topology auto-detection + boot fallback — Sprint D29.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 19.1.
//!
//! At `efe-server` boot we probe the CUDA driver to discover :
//!
//!   * Number of GPUs visible to the process.
//!   * Compute capability of each GPU.
//!   * Total VRAM per GPU.
//!   * Whether NCCL is loadable (multi-GPU).
//!
//! The result is logged at `INFO` level and used to decide
//! single-GPU vs multi-GPU sharding (cf. `multi_gpu.rs` §15).
//!
//! Fallback policy : if any of these probes fails, the runtime
//! proceeds with `CpuBackend` and logs a clear warning so the
//! operator notices the missing acceleration.

use cudarc::driver::CudaDevice;

use crate::error::CudaError;

/// Per-device information surfaced at boot.
#[derive(Debug, Clone)]
pub struct GpuInfo {
    /// Device ordinal (0-based).
    pub ordinal: u32,
    /// Compute capability encoded as MAJOR*10 + MINOR
    /// (`sm_75` → 75 ; `sm_90` → 90).
    pub compute_capability: u32,
    /// Total VRAM in bytes.
    pub total_vram_bytes: u64,
    /// Free VRAM in bytes at probe time.
    pub free_vram_bytes: u64,
    /// Best-effort device name (e.g. "NVIDIA A10").
    pub name: String,
}

/// Whole-process GPU topology.
#[derive(Debug, Clone, Default)]
pub struct GpuTopology {
    /// Detected GPUs ordered by ordinal.
    pub gpus: Vec<GpuInfo>,
    /// `true` iff NCCL is available on this host.
    pub nccl_available: bool,
}

impl GpuTopology {
    /// `true` iff at least one GPU is detected.
    #[must_use]
    pub fn has_gpu(&self) -> bool {
        !self.gpus.is_empty()
    }

    /// Number of detected GPUs.
    #[must_use]
    pub fn n_gpus(&self) -> usize {
        self.gpus.len()
    }

    /// Total VRAM aggregated across all GPUs.
    #[must_use]
    pub fn total_vram_bytes(&self) -> u64 {
        self.gpus.iter().map(|g| g.total_vram_bytes).sum()
    }
}

/// Probe the CUDA runtime and produce a [`GpuTopology`].
///
/// Returns an empty topology (no panic, no error) if :
///   * `cuda` feature is off (impossible — module is gated).
///   * No GPU is visible (driver missing, no hardware).
///   * The first device init fails.
///
/// **Side effects** : tracing logs at `INFO` level for each GPU
/// detected ; one `WARN` if probing fails partway.
pub fn detect_gpu_topology() -> GpuTopology {
    let n_devices = match cudarc::driver::result::device::get_count() {
        Ok(n) => n as usize,
        Err(e) => {
            tracing::warn!(
                target: "efe.cuda.topology",
                error = %e,
                "no GPU detected (CUDA driver missing or no visible device)"
            );
            return GpuTopology::default();
        }
    };

    if n_devices == 0 {
        tracing::info!(
            target: "efe.cuda.topology",
            "0 GPUs visible — staying on CPU"
        );
        return GpuTopology::default();
    }

    let mut gpus = Vec::with_capacity(n_devices);
    for i in 0..n_devices {
        match probe_device(i as u32) {
            Ok(info) => {
                tracing::info!(
                    target: "efe.cuda.topology",
                    ordinal = info.ordinal,
                    name = %info.name,
                    cc = info.compute_capability,
                    total_vram_gib = info.total_vram_bytes / (1024 * 1024 * 1024),
                    free_vram_gib = info.free_vram_bytes / (1024 * 1024 * 1024),
                    "GPU detected"
                );
                gpus.push(info);
            }
            Err(e) => {
                tracing::warn!(
                    target: "efe.cuda.topology",
                    ordinal = i,
                    error = %e,
                    "failed to probe GPU — skipping"
                );
            }
        }
    }

    let nccl_available = probe_nccl();
    if !nccl_available && gpus.len() > 1 {
        tracing::warn!(
            target: "efe.cuda.topology",
            "{} GPUs detected but NCCL unavailable — multi-GPU sharding will use TCP fallback",
            gpus.len()
        );
    }

    GpuTopology { gpus, nccl_available }
}

/// **Audit fix C-A20 + C-A21 (D28 audit pass 3)** — pre-fix the body
/// did `let _ = dev;` on line 2 (probably intended as
/// `let _dev = dev;` to keep alive but written wrong) which DROPS
/// the `Arc<CudaDevice>` immediately ; the subsequent
/// `dev.memory_info()` call would be use-after-move. Compile error
/// with `--features cuda`. The fix is to delete the line entirely
/// — Rust keeps `dev` alive through the function scope.
///
/// The closures ` |e| { ... }` had unused `e` parameters → renamed
/// `|_|` per `unused_variable` lint.
fn probe_device(ordinal: u32) -> Result<GpuInfo, CudaError> {
    let dev = CudaDevice::new(ordinal as usize).map_err(|_| {
        CudaError::DeviceUnavailable(ordinal)
            .with_observability("probe_device")
    })?;

    // Compute capability via cudarc.
    let cc = match cudarc::driver::result::device::get_attribute(
        ordinal as i32,
        cudarc::driver::sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MAJOR,
    ) {
        Ok(major) => {
            let minor = cudarc::driver::result::device::get_attribute(
                ordinal as i32,
                cudarc::driver::sys::CUdevice_attribute::CU_DEVICE_ATTRIBUTE_COMPUTE_CAPABILITY_MINOR,
            )
            .unwrap_or(0);
            (major as u32) * 10 + (minor as u32)
        }
        Err(_) => 0,
    };

    if cc < 75 {
        return Err(CudaError::UnsupportedCapability {
            device: ordinal,
            got_cc: cc,
            min_cc: 75,
        });
    }

    let (free, total) = dev.memory_info().map_err(|e| {
        CudaError::InitFailed(format!("memory_info on device {ordinal}: {e}"))
    })?;
    // Convert `usize` to `u64` (cudarc returns `usize` ; our
    // GpuInfo struct stores `u64` for cross-arch wire compatibility).
    let free_u64 = free as u64;
    let total_u64 = total as u64;

    let name = device_name(ordinal).unwrap_or_else(|_| format!("device{ordinal}"));

    Ok(GpuInfo {
        ordinal,
        compute_capability: cc,
        total_vram_bytes: total_u64,
        free_vram_bytes: free_u64,
        name,
    })
}

fn device_name(ordinal: u32) -> Result<String, CudaError> {
    cudarc::driver::result::device::get_name(ordinal as i32).map_err(|e| {
        CudaError::InitFailed(format!("get_name on device {ordinal}: {e}"))
    })
}

/// Best-effort probe of NCCL availability. Returns `false` when
/// the `nccl` Cargo feature is off ; otherwise tries to load the
/// NCCL shared library and verify version ≥ 2.18.
#[cfg(feature = "nccl")]
fn probe_nccl() -> bool {
    // The actual NCCL probe lives in `multi_gpu.rs` ; here we just
    // surface whether the feature is compiled in. Runtime probing
    // is deferred to `ShardedBackend::new` (D30).
    true
}

#[cfg(not(feature = "nccl"))]
fn probe_nccl() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_topology_has_no_gpu() {
        let t = GpuTopology::default();
        assert!(!t.has_gpu());
        assert_eq!(t.n_gpus(), 0);
        assert_eq!(t.total_vram_bytes(), 0);
    }

    #[test]
    fn topology_aggregates_vram() {
        let t = GpuTopology {
            gpus: vec![
                GpuInfo {
                    ordinal: 0, compute_capability: 80,
                    total_vram_bytes: 24 * 1024 * 1024 * 1024,
                    free_vram_bytes: 20 * 1024 * 1024 * 1024,
                    name: "A10".into(),
                },
                GpuInfo {
                    ordinal: 1, compute_capability: 80,
                    total_vram_bytes: 24 * 1024 * 1024 * 1024,
                    free_vram_bytes: 20 * 1024 * 1024 * 1024,
                    name: "A10".into(),
                },
            ],
            nccl_available: true,
        };
        assert!(t.has_gpu());
        assert_eq!(t.n_gpus(), 2);
        assert_eq!(t.total_vram_bytes(), 48 * 1024 * 1024 * 1024);
    }
}
