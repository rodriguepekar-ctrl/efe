//! `efe._native` — in-process Python bindings for EFE.
//!
//! Sprint **D25 (Φ-C)**. Cf. `docs/EFE_FFI_ARCHITECTURE.md`.
//!
//! # Dependency direction
//!
//! `efe-ffi → efe-core (→ efe-kernels)` — never the reverse.
//! Keeps `efe-core` Python-free and the workspace compilable
//! without a Python toolchain.
//!
//! # Module layout
//!
//! * [`error`] — Python exception hierarchy + `From<EfeError>`.
//! * [`verdict`] — `PyVerdict` immutable result type.
//! * [`bundle`] — `PyBundle` + `load_bundle()`.
//! * [`types`] — `PySwarmSpec`, `PyAlignmentCone`.
//! * [`els`] — `PyEpistemicLatentSpace` + numpy interop.
//! * [`controller`] — `PySwarmController` low-level.
//! * [`client`] — `PyEfeNativeClient` high-level.
//! * [`classify`] — `classify_zero_copy()` one-shot helper.
//! * [`async_`] — async `aclassify_zero_copy()` via `pyo3-async-runtimes`.
//!
//! # Build path
//!
//! Built via maturin (`maturin develop` for dev, `maturin build` for
//! release). The crate produces a `cdylib` (Python loadable) AND
//! an `rlib` (Rust-side benches).

#![warn(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(not(test), deny(clippy::unwrap_used))]
#![cfg_attr(not(test), deny(clippy::expect_used))]
// **Φ-C-FFI Completion audit** — autorise unwrap()/expect() en mode
// test. Sans cet allow, `cargo clippy --tests` échouait sur les
// tests existants (verdict.rs::to_dict_carries_all_fields,
// types.rs::swarm_spec_builder_complete_build_succeeds,
// cache.rs::insert_then_get_returns_same_arc, etc.). Le Cargo.toml
// utilise `priority = -1` pour rendre ces denies overridable.
#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(test, allow(clippy::expect_used))]

use pyo3::prelude::*;

pub mod async_;
pub mod bundle;
/// Sprint **Φ-C-FFI Completion (D26)** — LRU bundle cache + global
/// singleton. Production caller for the `efe_ffi_bundle_cache_hits_total`
/// Prometheus instrument. Cf.
/// `docs/EFE_PYO3_FFI_COMPLETION_ARCHITECTURE.md` § 16 entry #05.
pub mod cache;
pub mod classify;
pub mod client;
pub mod controller;
pub mod els;
pub mod error;
pub mod types;
pub mod verdict;

// ─────────────────────────────────────────────────────────────────
// Public Rust re-exports (preserved from pre-D25)
// ─────────────────────────────────────────────────────────────────

pub use efe_core::{EfeError, NanoAgentId, Result, SwarmId, TemplateCode, VERSION};

// ─────────────────────────────────────────────────────────────────
// Crate-level helpers
// ─────────────────────────────────────────────────────────────────

/// Returns the wheel version (`CARGO_PKG_VERSION` at compile time).
///
/// Used by Python `efe.version()` to log the active build at boot.
#[pyfunction]
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// PyO3 entry point. Mapped by maturin to the Python module name
/// `efe._native`. Python users `from efe._native import <X>` ; the
/// `efe/__init__.py` wrapper re-exports the surface so
/// `from efe import <X>` works directly.
///
/// # Module composition
///
/// 1. Top-level functions : `classify_zero_copy`,
///    `aclassify_zero_copy`, `load_bundle`, `version`.
/// 2. Classes : `Verdict`, `Bundle`, `EpistemicLatentSpace`,
///    `EfeNativeClient`, `SwarmController`, `SwarmSpec`,
///    `AlignmentCone`.
/// 3. Exception hierarchy : `EfeError` and 9 sub-classes.
///
/// # Tokio runtime init
///
/// `pyo3-async-runtimes::tokio::init_with_runtime` is called once
/// at module load. The runtime uses 2 worker threads (matches the
/// architecture `RuntimeConfig::gpu_streams` default minus 2 for
/// async overhead).
#[pymodule]
#[pyo3(name = "_native")]
fn efe_native(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    // **Φ-C-FFI Completion** — measure the wheel cold-load latency.
    // Records to the `efe_ffi_wheel_load_seconds{python_version}`
    // histogram exactly once per process — i.e. SLO-FFI-COLD-START
    // is observable in Prometheus even before the first classify.
    let wheel_load_started = std::time::Instant::now();

    // **Φ-C-FFI Completion** — install the panic-to-tracing hook
    // (DF1 + § 13.2). Without it, a Rust panic surfaces in Python as
    // a `PanicException` with the *trimmed* default message — devs
    // lose the backtrace. The hook re-routes via `tracing::error!`
    // so operators see the full panic on stderr / journald.
    install_panic_hook();

    // **Φ-C-FFI Completion** — boot-time registration of the 8
    // Prometheus instruments. Idempotent. Without it, the Grafana
    // `efe-ffi.json` panels would show "no data" until the first
    // classify lands → confusing baseline view.
    efe_telemetry::metrics::register_all_ffi();

    // Initialise the tokio runtime once. `pyo3-async-runtimes`
    // panics if called twice ; the OnceLock guard inside
    // `async_::init_tokio_runtime` ensures idempotency.
    async_::init_tokio_runtime().map_err(|e| {
        error::FfiError::new_err(format!("tokio runtime init: {e}"))
    })?;

    // ── Top-level functions ──────────────────────────────────
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(classify::classify_zero_copy, m)?)?;
    m.add_function(wrap_pyfunction!(async_::aclassify_zero_copy, m)?)?;
    m.add_function(wrap_pyfunction!(bundle::load_bundle, m)?)?;

    // ── Classes ──────────────────────────────────────────────
    m.add_class::<verdict::PyVerdict>()?;
    m.add_class::<bundle::PyBundle>()?;
    m.add_class::<els::PyEpistemicLatentSpace>()?;
    m.add_class::<client::PyEfeNativeClient>()?;
    m.add_class::<controller::PySwarmController>()?;
    m.add_class::<types::PySwarmSpec>()?;
    // **Audit fix F-A1** — pre-fix the builder class returned by
    // `SwarmSpec.builder()` was NOT registered with the Python
    // module, so any user code calling `.build()` would receive a
    // `TypeError: cannot create efe._native.SwarmSpecBuilder`. Now
    // explicit so the builder pattern works end-to-end from Python.
    m.add_class::<types::PySwarmSpecBuilder>()?;
    m.add_class::<types::PyAlignmentCone>()?;

    // ── Exception hierarchy ──────────────────────────────────
    error::register_exceptions(py, m)?;

    // **Φ-C-FFI Completion** — record the wheel cold-load latency
    // exactly once. The label is the runtime CPython MAJOR.MINOR
    // version (e.g. "3.10", "3.13") so operators can compare cold
    // start across Python versions in the same Grafana panel.
    //
    // SLO `SLO-FFI-COLD-START` targets ≤ 100 ms p95 ; the test
    // `python/efe/tests/test_cold_start.py` re-measures by spawning
    // a fresh subprocess, but this in-module hook also writes the
    // value to Prometheus for live ops dashboards.
    {
        let elapsed = wheel_load_started.elapsed().as_secs_f64();
        let py_version = py.version_info();
        let label = format!("{}.{}", py_version.major, py_version.minor);
        efe_telemetry::metrics::ffi_instruments::note_wheel_load(&label, elapsed);
    }

    Ok(())
}

/// **Φ-C-FFI Completion (DF1 § 13.2)** — installe un panic hook qui
/// log le panic AVANT de propager au default hook (= ce que PyO3
/// transforme en `PanicException` côté Python).
///
/// Sans le hook, un panic produit un message court côté Python sans
/// backtrace utilisable par les opérateurs. Avec le hook, le panic
/// est tracé sur stderr / journald avec sa pleine forme avant de
/// devenir une exception Python.
///
/// **Audit fix Φ-C-FFI.B3** — pre-fix utilisait uniquement
/// `tracing::error!` qui est silently dropped si aucun subscriber
/// `tracing` n'est initialisé (= cas typique d'un user qui fait
/// `import efe` sans avoir setup tracing). Post-fix : double-channel
///
///   1. `tracing::error!` (= production path, structured logging) ;
///   2. `eprintln!` (= fallback inconditionnel sur stderr).
///
/// Le `eprintln!` garantit que l'opérateur voit TOUJOURS le panic,
/// même sur un setup minimal.
///
/// Idempotent : appelé une seule fois au `#[pymodule]` init, mais
/// le default hook est sauvegardé dans le closure ; ré-invoquer
/// la fonction enchaînerait les hooks (= traces duplicates), donc
/// l'appel reste cantonné à `efe_native()`.
fn install_panic_hook() {
    use std::sync::Once;
    static INSTALLED: Once = Once::new();
    INSTALLED.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Channel 1 : structured tracing (= production path).
            tracing::error!("efe-ffi rust panic: {info}");
            // Channel 2 : raw stderr fallback (= guaranteed visibility
            // even without tracing subscriber).
            eprintln!("efe-ffi rust panic: {info}");
            default(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_non_empty() {
        assert!(!version().is_empty());
    }

    #[test]
    fn re_exports_work() {
        let _ = SwarmId::new();
        assert_eq!(TemplateCode::NaSem.as_str(), "na-sem");
        assert!(!VERSION.is_empty());
    }

    #[test]
    fn install_panic_hook_is_idempotent() {
        install_panic_hook();
        install_panic_hook();
        // No panic ; the `Once` guards a single install.
    }
}
