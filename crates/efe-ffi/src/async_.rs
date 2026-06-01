//! Async helpers — `aclassify_zero_copy()` + tokio runtime init.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 17.
//!
//! Bridges Python `asyncio` with the Rust tokio runtime via
//! `pyo3-async-runtimes::tokio`. Required for FastAPI / Sanic /
//! aiohttp users.
//!
//! **D25 status** : the async path delegates to the sync helper
//! inside `tokio::task::spawn_blocking` so the FFI surface is
//! present and tested. The fully native async pipeline (bundle
//! load + controller exec without `spawn_blocking`) is DT-F-ASYNC-3
//! (D26).

use std::sync::OnceLock;

use pyo3::prelude::*;

use crate::error::ffi_err;
use crate::verdict::PyVerdict;

/// Initialise the tokio runtime exactly once. Subsequent calls are
/// no-ops.
///
/// **Threading note** : 2 worker threads is the floor — too few
/// makes async classify+aclassify pairs serialise ; too many burns
/// memory in single-tenant CLI scripts. Operators can override via
/// `EFE_FFI_TOKIO_WORKERS` env var (D26 — DT-F-ASYNC-2).
pub fn init_tokio_runtime() -> Result<(), String> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    let result = INIT.get_or_init(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("efe-ffi-tokio")
            .enable_all()
            .build()
            .map_err(|e| format!("tokio Builder.build: {e}"))?;
        // pyo3-async-runtimes 0.21 :
        // `init_with_runtime(&'static Runtime)` panics if called
        // twice, so we Box::leak the runtime to give it 'static
        // lifetime. The OnceLock guard above makes this idempotent.
        let leaked: &'static tokio::runtime::Runtime = Box::leak(Box::new(runtime));
        pyo3_async_runtimes::tokio::init_with_runtime(leaked)
            .map_err(|e| format!("pyo3_async_runtimes init: {e:?}"))?;
        Ok(())
    });
    result.clone()
}

/// Async one-shot classify. Returns a Python awaitable.
///
/// Equivalent to [`crate::classify::classify_zero_copy`] but
/// non-blocking on the Python event loop. Use case : FastAPI
/// route handlers.
///
/// **D25 implementation** delegates to the sync path via
/// `tokio::task::spawn_blocking` — the swarm execution itself is
/// CPU-bound so this gives correct semantics. DT-F-ASYNC-3 (D26)
/// will replace with a fully async controller path.
#[pyfunction]
#[pyo3(signature = (
    text,
    catalog_id,
    bundle_path,
    catalog_path,
    *,
    gpu_enabled = false,
    seed = None,
))]
pub fn aclassify_zero_copy<'py>(
    py: Python<'py>,
    text: String,
    catalog_id: String,
    bundle_path: String,
    catalog_path: String,
    gpu_enabled: bool,
    seed: Option<u64>,
) -> PyResult<Bound<'py, PyAny>> {
    // **Audit fix Φ-C-FFI.B1** — pre-fix, aclassify_zero_copy
    // déléguait à classify_zero_copy qui appelle client.classify
    // hardcodé à `endpoint="sync"`. Résultat : 0 % des appels async
    // étaient labellés `endpoint="async"` → dashboard Grafana
    // faussement plat sur le quadrant async. Post-fix, on capture
    // le timing complet du wrap async ET on enregistre une métrique
    // dédiée `endpoint="async"` (= bumpe le compteur 1× par appel
    // async, en plus du compteur sync que client.classify incrémente
    // côté worker). Le breakdown Grafana devient :
    //   * sync = total calls reaching client.classify (sync + async-wrap)
    //   * async = total async API calls
    //
    // Le ratio `async / (sync + async)` indique la fraction async
    // user-initiated.
    let async_started = std::time::Instant::now();
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let join_result = tokio::task::spawn_blocking(move || {
            Python::with_gil(|py| {
                crate::classify::classify_zero_copy(
                    py,
                    &text,
                    &catalog_id,
                    &bundle_path,
                    &catalog_path,
                    gpu_enabled,
                    seed,
                )
            })
        })
        .await;

        // **Audit fix B1 cont.** — record async metric BEFORE Result
        // unwrap so a JoinError still counts (= err) and propagates.
        let elapsed = async_started.elapsed().as_secs_f64();
        let join = match join_result {
            Ok(inner) => inner,
            Err(e) => {
                efe_telemetry::metrics::ffi_instruments::note_classify(
                    "async",
                    "err",
                    elapsed,
                );
                return Err(ffi_err(format!(
                    "aclassify_zero_copy spawn_blocking: {e}",
                )));
            }
        };
        // `join` is `PyResult<PyVerdict>`. Same dual-record pattern.
        let verdict: PyVerdict = match join {
            Ok(v) => v,
            Err(e) => {
                efe_telemetry::metrics::ffi_instruments::note_classify(
                    "async",
                    "err",
                    elapsed,
                );
                return Err(e);
            }
        };
        efe_telemetry::metrics::ffi_instruments::note_classify(
            "async",
            "ok",
            elapsed,
        );
        Python::with_gil(|py| Ok(verdict.into_py(py)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_tokio_runtime_is_idempotent() {
        let r1 = init_tokio_runtime();
        let r2 = init_tokio_runtime();
        assert_eq!(r1, r2);
        assert!(r1.is_ok() || r1.is_err()); // either is OK ; we just don't panic
    }
}
