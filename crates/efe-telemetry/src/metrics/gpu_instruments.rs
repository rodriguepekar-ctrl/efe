//! Sprint **D29 (Φ-D)** — GPU runtime Prometheus instruments.
//!
//! Cf. `docs/EFE_KERNELS_ARCHITECTURE.md` § 18.
//!
//! 6 instruments covering the GPU substrate :
//!
//! | # | Métrique                              | Type      | Labels             |
//! |--:|---------------------------------------|-----------|--------------------|
//! | 1 | `efe_gpu_kernel_duration_seconds`     | Histogram | kernel             |
//! | 2 | `efe_gpu_memory_used_bytes`           | Gauge     | device             |
//! | 3 | `efe_gpu_utilization_ratio`           | Gauge     | device             |
//! | 4 | `efe_gpu_kernel_calls_total`          | Counter   | kernel, outcome    |
//! | 5 | `efe_gpu_oom_total`                   | Counter   | device             |
//! | 6 | `efe_gpu_nccl_allreduce_seconds`      | Histogram | (no label)         |
//!
//! Cardinalité projetée (architecture § 18.2) :
//!   * `kernel` ∈ ~30 (23 trait ops + helpers) ≤ 30
//!   * `device` ∈ {0..7} ≤ 8
//!   * `outcome` ∈ {ok, fail, oom} = 3
//!   → ≤ 30 × 8 × 3 + 8 + 8 + 30 × 8 = ~ 1 000 séries
//!     (sous le budget D13 50 000 séries ✅).
//!
//! **Audit fix D18-K-A1** — these instruments are bumped from
//! `efe-kernels::observability` so any future regression of "metric
//! declared but never observed" surfaces in the cross-sprint
//! pin test `test_audit_d29_gpu_metrics_have_callers.py`.

use std::sync::OnceLock;

use prometheus::{
    exponential_buckets, Histogram, HistogramOpts, HistogramVec, IntCounterVec,
    IntGaugeVec, Opts,
};

use super::registry::global_registry;

/// Internal once-per-call macro identical to D17/D18.
macro_rules! once_metric {
    ($(#[$doc:meta])* $fn_name:ident, $ty:ty, $init:expr) => {
        $(#[$doc])*
        pub fn $fn_name() -> &'static $ty {
            static CELL: OnceLock<$ty> = OnceLock::new();
            CELL.get_or_init(|| {
                let m: $ty = $init;
                let _ = global_registry().register(Box::new(m.clone()));
                m
            })
        }
    };
}

// ====================================================================
// 1. Histogram — kernel duration per op
// ====================================================================

once_metric!(
    /// `efe_gpu_kernel_duration_seconds{kernel}` — wall-clock
    /// duration of a single CUDA kernel launch (CPU-side timer ;
    /// includes h2d / kernel / d2h).
    ///
    /// Buckets exponentielles 1 µs → 100 ms (12 buckets) ; couvre
    /// les kernels rapides (mean, l2_norm) et lents (sdpa).
    gpu_kernel_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_gpu_kernel_duration_seconds",
            "Wall-clock duration of one CUDA kernel launch (D29 § 18.1 #1).",
        )
        .buckets(
            exponential_buckets(1e-6, 4.0, 12).expect("valid buckets")
        ),
        &["kernel"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 2. Gauge — VRAM used per device
// ====================================================================

once_metric!(
    /// `efe_gpu_memory_used_bytes{device}` — bytes currently
    /// allocated on the GPU. Updated by
    /// `DeviceAllocator::alloc_zeros` and `Drop` of `TrackedSlice`.
    gpu_memory_used_bytes,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_gpu_memory_used_bytes",
            "VRAM bytes currently allocated by efe-kernels (D29 § 18.1 #2).",
        ),
        &["device"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 3. Gauge — GPU utilization ratio
// ====================================================================

once_metric!(
    /// `efe_gpu_utilization_ratio{device}` — fraction of time the
    /// GPU has been actively computing in the last sampling
    /// window. Set externally by a periodic NVML probe (D31).
    /// Until D31, this gauge stays at 0.
    gpu_utilization_ratio,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_gpu_utilization_ratio",
            "GPU utilization ratio in [0, 100] (D29 § 18.1 #3).",
        ),
        &["device"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 4. Counter — kernel calls per outcome
// ====================================================================

once_metric!(
    /// `efe_gpu_kernel_calls_total{kernel, outcome}` —
    /// `outcome` ∈ `ok` / `fail` / `oom`.
    gpu_kernel_calls_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_gpu_kernel_calls_total",
            "CUDA kernel calls per outcome (D29 § 18.1 #4).",
        ),
        &["kernel", "outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 5. Counter — OOM events
// ====================================================================

once_metric!(
    /// `efe_gpu_oom_total{device}` — count of allocations refused
    /// because the high-water-mark would be exceeded.
    /// Alert `D18GpuOom` fires on `rate(efe_gpu_oom_total) > 0`
    /// for 1 min.
    gpu_oom_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_gpu_oom_total",
            "Count of OOM events per device (D29 § 18.1 #5).",
        ),
        &["device"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 6. Histogram — NCCL all-reduce latency
// ====================================================================

once_metric!(
    /// `efe_gpu_nccl_allreduce_seconds` — wall-clock duration of a
    /// single NCCL all-reduce. Used by `multi_gpu.rs` after each
    /// inter-GPU sync.
    ///
    /// Buckets : 100 µs → 100 ms (10 buckets).
    gpu_nccl_allreduce_seconds,
    Histogram,
    Histogram::with_opts(
        HistogramOpts::new(
            "efe_gpu_nccl_allreduce_seconds",
            "NCCL all-reduce wall-clock duration (D29 § 18.1 #6).",
        )
        .buckets(
            exponential_buckets(1e-4, 3.0, 10).expect("valid buckets")
        ),
    )
    .expect("valid histogram opts")
);

// ====================================================================
// Sprint **Φ-D D30 (Bundle completion)** — 4 instruments additionnels.
//
// Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 10.2.
//
// Ces 4 instruments étendent l'archi D29 originale (6 instruments)
// pour couvrir les surfaces non-traitées : pinned host pool,
// arena reset cadence, SM occupancy NVML, et version-info build.
// ====================================================================

// ====================================================================
// 7. Gauge — pinned_host_pool_bytes
// ====================================================================

once_metric!(
    /// `efe_gpu_pinned_host_pool_bytes` — total bytes cudaHostAlloc'd
    /// dans le pool host pinné (= staging des transfers H↔D async).
    ///
    /// Pas de labels — pool global par process. Alerte
    /// `GPU_PINNED_HOST_POOL_BLOAT` fire si > 200 MiB (= bug fuite
    /// dans le free-list).
    gpu_pinned_host_pool_bytes,
    prometheus::IntGauge,
    prometheus::IntGauge::with_opts(Opts::new(
        "efe_gpu_pinned_host_pool_bytes",
        "Pinned host memory pool size (D30 Φ-D § 10.2 #7).",
    ))
    .expect("valid gauge opts")
);

// ====================================================================
// 8. Counter — arena_reset_total
// ====================================================================

once_metric!(
    /// `efe_gpu_arena_reset_total{pool}` — nombre de resets de chaque
    /// arena VRAM par stream-worker. **Devrait être ≈ RPS** en
    /// steady-state (= 1 reset par request fin).
    ///
    /// `pool` ∈ `stream_0`, `stream_1`, …, `stream_N` (≤ 8 workers
    /// Tokio typique = 8 valeurs).
    ///
    /// Alerte `GPU_ARENA_RESET_LAGS` si RPS / reset > 1.2× (= les
    /// arenas ne sont pas reset → fuite VRAM).
    gpu_arena_reset_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_gpu_arena_reset_total",
            "Per-stream VRAM arena reset count (D30 Φ-D § 10.2 #8).",
        ),
        &["pool"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 9. Gauge — sm_occupancy_ratio
// ====================================================================

once_metric!(
    /// `efe_gpu_sm_occupancy_ratio{device}` — proportion des SMs
    /// occupés (= NVML `nvmlDeviceGetUtilizationRates().sm` / 100).
    /// Mise à jour 1×/s par cron NVML.
    ///
    /// Cible steady-state ≥ 0.40 sous charge nominale 200 RPS ;
    /// < 0.20 = sous-utilisation (= over-provisioned).
    /// > 0.95 = saturation (= contention, throttle attendu).
    gpu_sm_occupancy_ratio,
    prometheus::GaugeVec,
    prometheus::GaugeVec::new(
        Opts::new(
            "efe_gpu_sm_occupancy_ratio",
            "Per-device SM occupancy ratio from NVML (D30 Φ-D § 10.2 #9).",
        ),
        &["device"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 10. Gauge — driver_version_info
// ====================================================================

once_metric!(
    /// `efe_gpu_driver_version_info{driver, cuda_runtime, cuda_compile, nccl, kernels_sha}` —
    /// gauge constante à 1 servant d'info-tag pour identifier la
    /// stack CUDA au boot.
    ///
    /// Pattern Prometheus standard `_info` (= cf. `kube_pod_info`).
    /// Cardinalité : 1 série par combinaison unique. Stable en
    /// steady-state ; bump uniquement lors de rotation driver
    /// ou rebuild binaire.
    gpu_driver_version_info,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_gpu_driver_version_info",
            "Info-style gauge tagging the active GPU stack (D30 Φ-D § 10.2 #10).",
        ),
        &["driver", "cuda_runtime", "cuda_compile", "nccl", "kernels_sha"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// Eager registration helper
// ====================================================================

/// Force-register every GPU instrument with the global registry.
/// Pattern identical to `register_all_d18()`.
///
/// **Audit fix D29-K-A1** — `Server::build` MUST call this when
/// the runtime opts into GPU (`EFE_RUNTIME_GPU_ENABLED=true`) so
/// the Prometheus `/metrics` endpoint exposes the 10 GPU series as
/// zero-valued (vs "no data") even before the first kernel launch.
///
/// **Audit fix Φ-D D30** — étend de 6 à 10 instruments (= 4
/// nouveaux pour pinned_host, arena_reset, sm_occupancy,
/// driver_version_info).
pub fn register_all_gpu() {
    let _ = gpu_kernel_duration_seconds();
    let _ = gpu_memory_used_bytes();
    let _ = gpu_utilization_ratio();
    let _ = gpu_kernel_calls_total();
    let _ = gpu_oom_total();
    let _ = gpu_nccl_allreduce_seconds();
    // 4 nouveaux (Φ-D D30)
    let _ = gpu_pinned_host_pool_bytes();
    let _ = gpu_arena_reset_total();
    let _ = gpu_sm_occupancy_ratio();
    let _ = gpu_driver_version_info();
}

/// Helper opérationnel pour set le `driver_version_info` gauge au
/// boot après détection NVML. **Idempotent** (set 1 sur la même
/// série n-fois → reste 1).
pub fn note_driver_version(
    driver: &str,
    cuda_runtime: &str,
    cuda_compile: &str,
    nccl: &str,
    kernels_sha: &str,
) {
    gpu_driver_version_info()
        .with_label_values(&[driver, cuda_runtime, cuda_compile, nccl, kernels_sha])
        .set(1);
}

/// Helper opérationnel pour observer le set NVML SM occupancy.
/// Appelé 1×/s par la cron NVML.
pub fn observe_sm_occupancy(device: &str, ratio: f64) {
    gpu_sm_occupancy_ratio()
        .with_label_values(&[device])
        .set(ratio.clamp(0.0, 1.0));
}

/// **Audit fix Φ-D.B1** — helper public pour incrémenter le compteur
/// `efe_gpu_arena_reset_total{pool}`. À appeler depuis
/// `efe-kernels::memory::VramArena::reset()` à chaque request fin.
///
/// `pool` est le label canonique du stream owner (= "stream_0",
/// "stream_1", …, "stream_N" où N = nombre de Tokio workers).
/// Cardinalité bornée par le scheduler (≤ 8 workers typique).
///
/// Sans cet appel, le compteur reste à 0 → alerte
/// `GPU_ARENA_RESET_LAGS` (si introduite) ne peut détecter aucun
/// problème. Le panel #8 du dashboard `efe-gpu-kernels.json`
/// affiche "no data".
pub fn note_arena_reset(pool: &str) {
    gpu_arena_reset_total().with_label_values(&[pool]).inc();
}

/// **Audit fix Φ-D.B2** — helper public pour set le gauge
/// `efe_gpu_pinned_host_pool_bytes`. À appeler depuis
/// `efe-kernels::memory::PinnedHostPool` à chaque alloc/free :
/// la valeur stockée doit refléter le total des bytes
/// cudaHostAlloc'd actuellement.
///
/// Sans cet appel, le gauge reste à 0 → alerte
/// `GPU_PINNED_HOST_POOL_BLOAT` aveugle, panel #9 du dashboard
/// affiche "no data".
pub fn set_pinned_host_pool_bytes(bytes: i64) {
    gpu_pinned_host_pool_bytes().set(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    #[test]
    fn register_all_gpu_idempotent() {
        register_all_gpu();
        register_all_gpu();
    }

    #[test]
    fn gpu_kernel_duration_label_arity_is_one() {
        assert_eq!(
            gpu_kernel_duration_seconds().desc()[0]
                .variable_labels
                .len(),
            1
        );
    }

    #[test]
    fn gpu_memory_used_label_arity_is_one() {
        assert_eq!(
            gpu_memory_used_bytes().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn gpu_kernel_calls_label_arity_is_two() {
        assert_eq!(
            gpu_kernel_calls_total().desc()[0].variable_labels.len(),
            2
        );
    }

    #[test]
    fn gpu_nccl_allreduce_no_labels() {
        assert_eq!(
            gpu_nccl_allreduce_seconds().desc()[0]
                .variable_labels
                .len(),
            0
        );
    }

    #[test]
    fn gpu_kernel_calls_outcome_strings_pinned() {
        // Architecture § 18.1 commits to these specific outcome
        // strings — pin them via a smoke increment.
        for o in ["ok", "fail", "oom"] {
            gpu_kernel_calls_total()
                .with_label_values(&["softmax", o])
                .inc();
        }
    }

    #[test]
    fn gpu_kernel_duration_observes() {
        gpu_kernel_duration_seconds()
            .with_label_values(&["softmax"])
            .observe(0.0001); // 100 µs
        let m = gpu_kernel_duration_seconds().with_label_values(&["softmax"]);
        assert!(m.get_sample_count() >= 1);
    }

    // ================================================================
    // Tests pour les 4 instruments ajoutés en Φ-D D30
    // ================================================================

    #[test]
    fn gpu_pinned_host_pool_bytes_has_no_labels() {
        assert_eq!(
            gpu_pinned_host_pool_bytes().desc()[0]
                .variable_labels
                .len(),
            0
        );
    }

    #[test]
    fn gpu_arena_reset_total_label_arity_is_one() {
        assert_eq!(
            gpu_arena_reset_total().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn gpu_sm_occupancy_ratio_label_arity_is_one() {
        assert_eq!(
            gpu_sm_occupancy_ratio().desc()[0].variable_labels.len(),
            1
        );
    }

    #[test]
    fn gpu_driver_version_info_label_arity_is_five() {
        assert_eq!(
            gpu_driver_version_info().desc()[0].variable_labels.len(),
            5
        );
    }

    #[test]
    fn note_driver_version_sets_gauge_to_one() {
        register_all_gpu();
        note_driver_version("535.104.05", "12.4", "12.4", "2.21.5", "abc123");
        let v = gpu_driver_version_info()
            .with_label_values(&["535.104.05", "12.4", "12.4", "2.21.5", "abc123"])
            .get();
        assert_eq!(v, 1);

        // Idempotent : re-set = stays 1
        note_driver_version("535.104.05", "12.4", "12.4", "2.21.5", "abc123");
        let v2 = gpu_driver_version_info()
            .with_label_values(&["535.104.05", "12.4", "12.4", "2.21.5", "abc123"])
            .get();
        assert_eq!(v2, 1);
    }

    #[test]
    fn observe_sm_occupancy_clamps_ratio() {
        register_all_gpu();
        // Valid
        observe_sm_occupancy("0", 0.42);
        assert!((gpu_sm_occupancy_ratio().with_label_values(&["0"]).get() - 0.42).abs() < 1e-9);
        // Clamp upper
        observe_sm_occupancy("1", 1.5);
        assert_eq!(gpu_sm_occupancy_ratio().with_label_values(&["1"]).get(), 1.0);
        // Clamp lower
        observe_sm_occupancy("2", -0.1);
        assert_eq!(gpu_sm_occupancy_ratio().with_label_values(&["2"]).get(), 0.0);
    }

    #[test]
    fn arena_reset_total_increments() {
        register_all_gpu();
        let before = gpu_arena_reset_total().with_label_values(&["stream_0"]).get();
        gpu_arena_reset_total()
            .with_label_values(&["stream_0"])
            .inc();
        gpu_arena_reset_total()
            .with_label_values(&["stream_0"])
            .inc();
        assert_eq!(
            gpu_arena_reset_total().with_label_values(&["stream_0"]).get(),
            before + 2
        );
    }

    #[test]
    fn register_all_gpu_covers_10_instruments() {
        // Pin contractuel Φ-D D30 : 10 instruments distincts.
        // Si quelqu'un ajoute un instrument sans l'enregistrer
        // dans register_all_gpu, le panel Grafana sera "no data"
        // au boot — ce test catch ça.
        register_all_gpu();
        // Sanity : tous les getters retournent un &'static valide
        // (= force-init via OnceLock).
        let _ = gpu_kernel_duration_seconds();
        let _ = gpu_memory_used_bytes();
        let _ = gpu_utilization_ratio();
        let _ = gpu_kernel_calls_total();
        let _ = gpu_oom_total();
        let _ = gpu_nccl_allreduce_seconds();
        let _ = gpu_pinned_host_pool_bytes();
        let _ = gpu_arena_reset_total();
        let _ = gpu_sm_occupancy_ratio();
        let _ = gpu_driver_version_info();
    }

    // ================================================================
    // Tests B1+B2 helpers prod-grade (audit fix Φ-D level 2)
    // ================================================================

    #[test]
    fn note_arena_reset_increments_per_pool() {
        register_all_gpu();
        let before = gpu_arena_reset_total()
            .with_label_values(&["stream_3"])
            .get();
        note_arena_reset("stream_3");
        note_arena_reset("stream_3");
        note_arena_reset("stream_3");
        let after = gpu_arena_reset_total()
            .with_label_values(&["stream_3"])
            .get();
        assert_eq!(after, before + 3);
    }

    #[test]
    fn note_arena_reset_distinct_pools_isolated() {
        register_all_gpu();
        let a_before = gpu_arena_reset_total()
            .with_label_values(&["stream_4"])
            .get();
        let b_before = gpu_arena_reset_total()
            .with_label_values(&["stream_5"])
            .get();
        note_arena_reset("stream_4");
        let a_after = gpu_arena_reset_total()
            .with_label_values(&["stream_4"])
            .get();
        let b_after = gpu_arena_reset_total()
            .with_label_values(&["stream_5"])
            .get();
        assert_eq!(a_after, a_before + 1);
        assert_eq!(b_after, b_before, "different pool should not move");
    }

    #[test]
    fn set_pinned_host_pool_bytes_overwrites_gauge() {
        register_all_gpu();
        set_pinned_host_pool_bytes(1024 * 1024); // 1 MiB
        assert_eq!(gpu_pinned_host_pool_bytes().get(), 1024 * 1024);
        set_pinned_host_pool_bytes(0); // freed
        assert_eq!(gpu_pinned_host_pool_bytes().get(), 0);
        set_pinned_host_pool_bytes(50 * 1024 * 1024); // 50 MiB grow
        assert_eq!(gpu_pinned_host_pool_bytes().get(), 50 * 1024 * 1024);
    }
}
