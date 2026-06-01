//! GPU runtime observability hooks — Sprint D28.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 18.
//!
//! Six Prometheus instruments are exposed by
//! `efe-telemetry::metrics::gpu_instruments`. This module ships
//! light wrappers that bump them from inside the CUDA backend so
//! the metric surface is centralized and impossible to forget
//! (recurrent "wiring forgotten" defect class — cf. D17-AC2 /
//! D18-A3 audits).
//!
//! Each wrapper here is a no-op when the `efe-telemetry` `gpu`
//! instruments are absent at link time — but the canonical build
//! (i.e. `efe-server`) always pulls them in.

use std::time::Instant;

/// Returned by [`record_kernel_call`]. When dropped, observes the
/// elapsed wallclock duration into
/// `efe_gpu_kernel_duration_seconds{kernel}` and bumps the
/// `efe_gpu_kernel_calls_total{kernel, outcome="ok"}` counter.
///
/// On error path callers must call [`KernelCallGuard::fail`] before
/// drop — that flips the outcome label to `"fail"` so the alert
/// `D18GpuKernelFailures` fires correctly.
#[must_use = "drop the guard at the end of the kernel call to record telemetry"]
pub struct KernelCallGuard {
    kernel: &'static str,
    start: Instant,
    failed: bool,
}

impl KernelCallGuard {
    /// Mark this call as failed. Idempotent.
    pub fn fail(&mut self) {
        self.failed = true;
    }
}

impl Drop for KernelCallGuard {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed().as_secs_f64();
        record_kernel_duration(self.kernel, elapsed);
        record_kernel_outcome(
            self.kernel,
            if self.failed { "fail" } else { "ok" },
        );
    }
}

/// Start a kernel call observation. The returned guard records the
/// `efe_gpu_kernel_duration_seconds{kernel}` histogram at drop.
///
/// # Example
///
/// ```ignore
/// fn softmax(&self, x: &Tensor<f32>, axis: i32) -> Result<Tensor<f32>> {
///     let _g = record_kernel_call("softmax");
///     // ... actual launch ...
///     Ok(out)
/// }
/// ```
#[must_use]
pub fn record_kernel_call(kernel: &'static str) -> KernelCallGuard {
    KernelCallGuard {
        kernel,
        start: Instant::now(),
        failed: false,
    }
}

/// Bump `efe_gpu_kernel_duration_seconds{kernel}` histogram.
///
/// **Audit fix C-A12 (D28 audit pass 3)** — the previous version
/// declared a private helper `telemetry_kernel_duration() ->
/// Option<&'static prometheus::HistogramVec>` to centralise the
/// metric lookup. That return type **NAMED** `prometheus::HistogramVec`
/// in the function signature, which requires the `prometheus` crate
/// to be in scope. But `efe-kernels` does NOT declare `prometheus`
/// as a dependency — the metrics surface lives in `efe-telemetry`.
/// Compile error with `--features cuda`.
///
/// The fix : inline every metric call directly. The intermediate
/// helper is removed ; we never name `prometheus::*` types in
/// `efe-kernels`. Type inference handles the rest.
pub fn record_kernel_duration(kernel: &'static str, elapsed_seconds: f64) {
    #[cfg(feature = "cuda")]
    {
        efe_telemetry::metrics::gpu_instruments::gpu_kernel_duration_seconds()
            .with_label_values(&[kernel])
            .observe(elapsed_seconds);
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = (kernel, elapsed_seconds);
    }
}

/// Bump `efe_gpu_kernel_calls_total{kernel, outcome}` counter.
pub fn record_kernel_outcome(kernel: &'static str, outcome: &'static str) {
    #[cfg(feature = "cuda")]
    {
        efe_telemetry::metrics::gpu_instruments::gpu_kernel_calls_total()
            .with_label_values(&[kernel, outcome])
            .inc();
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = (kernel, outcome);
    }
}

/// Bump `efe_gpu_oom_total{device}` counter when an alloc fails.
pub fn record_oom(device: u32) {
    #[cfg(feature = "cuda")]
    {
        efe_telemetry::metrics::gpu_instruments::gpu_oom_total()
            .with_label_values(&[&device.to_string()])
            .inc();
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = device;
    }
}

/// Set `efe_gpu_memory_used_bytes{device}` gauge. Called by the
/// `DeviceAllocator` after each successful alloc / dealloc.
pub fn record_memory_used(device: u32, bytes: u64) {
    #[cfg(feature = "cuda")]
    {
        efe_telemetry::metrics::gpu_instruments::gpu_memory_used_bytes()
            .with_label_values(&[&device.to_string()])
            .set(bytes as i64);
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = (device, bytes);
    }
}

/// **Audit fix K-A7** — optional NaN/Inf scan helper. When the
/// `auto_check_finite` Cargo feature is active OR
/// `EFE_RUNTIME_GPU_CHECK_FINITE=true` env var is set, the
/// `CudaBackend` can call into this helper to probe a device
/// buffer right after a kernel completes.
///
/// Returns `Ok(0)` when the input is fully finite ; `Ok(n)` when
/// `n > 0` non-finite values were found ; `Err(_)` on driver error.
///
/// **D28 status** : the helper is a public stub here. The actual
/// kernel launch + counter readback wiring is done in
/// `crate::backend::CudaBackend::check_finite_post_kernel` (D29
/// follow-up — DT-K-D29-FINITE-1) so the cardinality budget +
/// extra h2d round-trip cost are explicit.
#[cfg(feature = "cuda")]
#[must_use]
pub fn check_finite_enabled_runtime() -> bool {
    cfg!(feature = "auto_check_finite")
        || std::env::var("EFE_RUNTIME_GPU_CHECK_FINITE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
}

/// Observe `efe_gpu_nccl_allreduce_seconds`. Called from
/// `multi_gpu.rs` after each all-reduce.
pub fn record_nccl_allreduce(elapsed_seconds: f64) {
    #[cfg(feature = "cuda")]
    {
        efe_telemetry::metrics::gpu_instruments::gpu_nccl_allreduce_seconds()
            .observe(elapsed_seconds);
    }
    #[cfg(not(feature = "cuda"))]
    {
        let _ = elapsed_seconds;
    }
}

// ─────────────────────────────────────────────────────────────────────
// **Audit fix C-A12 (D28 audit pass 3)** — the lazy-lookup helpers
// that lived here previously named `prometheus::*` types in their
// return signatures, which required `prometheus` to be in scope of
// the `efe-kernels` crate. Since `efe-kernels` does not depend on
// `prometheus` directly, those signatures broke the
// `--features cuda` build. The helpers are deleted ; every metric
// call above is now inlined, leaving type inference (rather than
// explicit type naming) to resolve `prometheus::*`.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn guard_records_at_drop() {
        // Even when `cuda` feature is off, the guard must not panic.
        let guard = record_kernel_call("test_kernel");
        thread::sleep(Duration::from_millis(1));
        drop(guard);
    }

    #[test]
    fn guard_fail_path() {
        let mut guard = record_kernel_call("test_kernel_fail");
        guard.fail();
        drop(guard);
    }

    #[test]
    fn record_oom_no_panic() {
        record_oom(0);
        record_oom(7);
    }

    #[test]
    fn record_memory_used_no_panic() {
        record_memory_used(0, 1024);
        record_memory_used(0, 0);
    }
}
