//! `PyEfeNativeClient` — high-level Python client.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 8 + § 13 + § 27.
//!
//! Drop-in replacement for `efe_tool.EfeClient` (gRPC) with a
//! shared `Bundle` + a wrapped `PySwarmController`. Releases the
//! GIL on every classify call.

use std::sync::Arc;
use std::time::Instant;

use pyo3::prelude::*;

use crate::bundle::PyBundle;
use crate::controller::PySwarmController;
use crate::error::{config_err, ffi_err};
use crate::types::PySwarmSpec;
use crate::verdict::PyVerdict;

/// Python-facing high-level client. Recommended entry point.
///
/// Construct with a `Bundle` and a catalog path. Internally holds
/// an `Arc<PySwarmController>` so multiple `EfeNativeClient` can
/// share state if constructed from the same args.
#[pyclass(name = "EfeNativeClient", module = "efe._native")]
pub struct PyEfeNativeClient {
    bundle: PyBundle,
    controller: Arc<PySwarmController>,
    /// Closed flag — set by `close()` / `__exit__`. After close,
    /// further calls return `EfeError`.
    closed: bool,
}

#[pymethods]
impl PyEfeNativeClient {
    /// Construct from a bundle + catalog path.
    ///
    /// # Arguments
    ///
    /// * `bundle` — produced by `efe.load_bundle(path)`.
    /// * `catalog_path` — path to the catalog JSON.
    /// * `gpu_enabled` — when True and the wheel was built with
    ///   `--features cuda`, use the GPU substrate ; else CPU.
    /// * `seed` — optional seed for deterministic runs.
    #[new]
    #[pyo3(signature = (bundle, catalog_path, *, gpu_enabled = false, seed = None))]
    pub fn new(
        bundle: PyRef<'_, PyBundle>,
        catalog_path: String,
        gpu_enabled: bool,
        seed: Option<u64>,
    ) -> PyResult<Self> {
        if catalog_path.is_empty() {
            return Err(config_err("catalog_path must not be empty"));
        }

        // Clone the inner Arc<InnerBundle> ; the user's PyBundle
        // continues to own its handle.
        let bundle_clone = PyBundle {
            inner: Arc::clone(&bundle.inner),
        };

        let controller = PySwarmController::new(catalog_path, gpu_enabled, seed)?;

        Ok(Self {
            bundle: bundle_clone,
            controller: Arc::new(controller),
            closed: false,
        })
    }

    /// Classify `text` and return a [`Verdict`].
    ///
    /// **Φ-C-FFI Completion (D26)** — production caller for 4
    /// Prometheus instruments :
    ///   * `efe_ffi_classify_total{endpoint=sync,outcome=ok|err}`
    ///   * `efe_ffi_classify_seconds{endpoint=sync}`
    ///   * `efe_ffi_gil_held_seconds{op=classify}`
    ///   * `efe_ffi_gil_released_seconds{op=classify}`
    ///   * `efe_ffi_marshal_in_seconds{kind=str}`
    ///   * `efe_ffi_marshal_out_seconds{kind=verdict}`
    ///
    /// Sans ce wiring, les panels FFI du dashboard Grafana
    /// `efe-ffi.json` afficheraient "no data" (= pattern Φ-D.B1/B2).
    ///
    /// # Errors
    ///
    /// * `EfeError` if `close()` has been called.
    /// * `BundleError` if `text` is empty (delegated to `Bundle.encode`).
    /// * `SwarmError` if the controller fails.
    pub fn classify(&self, py: Python<'_>, text: &str) -> PyResult<PyVerdict> {
        // Capture ASAP for the total-latency histogram (=
        // pre-validation cost is part of the user-visible latency).
        let total_started = Instant::now();

        // Helper that pushes the error to the metric + propagates.
        // **Φ-C-FFI Completion** — D-FFI-2 enforcement : both branches
        // (ok / err) must touch `efe_ffi_classify_total` so the ratio
        // is meaningful in Grafana.
        let observe_err = |err: PyErr| -> PyErr {
            efe_telemetry::metrics::ffi_instruments::note_classify(
                "sync",
                "err",
                total_started.elapsed().as_secs_f64(),
            );
            err
        };

        if self.closed {
            return Err(observe_err(ffi_err("EfeNativeClient is closed")));
        }
        if text.is_empty() {
            return Err(observe_err(ffi_err("text must not be empty")));
        }

        // ── Marshal-in : Python str → owned Rust String + arc clones.
        // C'est le seul coût pré-GIL-release qui mérite d'être tracé.
        let marshal_in_started = Instant::now();
        let bundle_inner = Arc::clone(&self.bundle.inner);
        let controller = Arc::clone(&self.controller);
        let owned_text = text.to_string();
        let request_id = generate_request_id();
        let marshal_in_seconds = marshal_in_started.elapsed().as_secs_f64();
        efe_telemetry::metrics::ffi_instruments::note_marshal_in(
            "str",
            marshal_in_seconds,
        );

        // ── Heavy work : release GIL.
        let released_started = Instant::now();
        let (verdict_core, kernel_latency_ms): (PyVerdict, f64) = py.allow_threads(move || {
            let started = Instant::now();
            // 1. Encode text → embedding.
            let embedding = bundle_inner.encode(&owned_text);
            // 2. Build a SwarmSpec.
            let spec = PySwarmSpec {
                request_id,
                catalog_id: controller.catalog_id(),
                embedding,
                seed: controller.seed(),
                max_rounds: 20,
            };
            // 3. Execute (D25 stub — synthetic verdict).
            let v = synthesise_via_helper(&controller, &spec, started);
            (v, started.elapsed().as_secs_f64() * 1000.0)
        });
        let released_seconds = released_started.elapsed().as_secs_f64();

        // ── Marshal-out : Rust verdict → patch latency_ms. Le verdict
        // est déjà un PyClass post-construct ; le marshal final
        // Python-side se fait par PyO3 sur return.
        let marshal_out_started = Instant::now();
        let mut verdict = verdict_core;
        verdict.latency_ms = kernel_latency_ms;
        let marshal_out_seconds = marshal_out_started.elapsed().as_secs_f64();
        efe_telemetry::metrics::ffi_instruments::note_marshal_out(
            "verdict",
            marshal_out_seconds,
        );

        // ── Record GIL split : held = total - released.
        let total_seconds = total_started.elapsed().as_secs_f64();
        let held_seconds = (total_seconds - released_seconds).max(0.0);
        efe_telemetry::metrics::ffi_instruments::note_gil_split(
            "classify",
            held_seconds,
            released_seconds,
        );

        // ── Final : total counter + duration histogram.
        efe_telemetry::metrics::ffi_instruments::note_classify(
            "sync",
            "ok",
            total_seconds,
        );

        Ok(verdict)
    }

    /// Health check. **D25 stub** : returns `True` if the bundle
    /// dim ≥ 1 and the controller has at least one label.
    fn healthcheck(&self) -> bool {
        if self.closed {
            return false;
        }
        self.bundle.inner.dim > 0 && self.controller.n_labels() > 0
    }

    /// Close the client. Idempotent.
    pub fn close(&mut self) {
        self.closed = true;
    }

    /// Context manager entry — returns `self`.
    fn __enter__(slf: PyRefMut<'_, Self>) -> PyResult<PyRefMut<'_, Self>> {
        Ok(slf)
    }

    /// Context manager exit — calls `close()`.
    #[pyo3(signature = (_exc_type=None, _exc_val=None, _tb=None))]
    fn __exit__(
        &mut self,
        _exc_type: Option<PyObject>,
        _exc_val: Option<PyObject>,
        _tb: Option<PyObject>,
    ) -> bool {
        self.close();
        false // do not swallow exceptions
    }

    fn __repr__(&self) -> String {
        format!(
            "EfeNativeClient(bundle_id={:?}, catalog_id={:?}, closed={})",
            self.bundle.inner.id,
            self.controller.catalog_id(),
            self.closed
        )
    }
}

// ─────────────────────────────────────────────────────────────────
// Helpers — bridges between Python-method-bound state and the
// raw synthesise path inside `allow_threads`.
// ─────────────────────────────────────────────────────────────────

/// Generate a request id (UUIDv4-like ; here a short hex from the
/// system clock + RNG salt for D25 stub).
fn generate_request_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("req-{:x}", nanos)
}

/// **Audit fix F-A3 + F-A4 (D25 audit pass 1)** — pre-fix this
/// helper :
///   * Built fake labels `["label-0", "label-1", …]` because the
///     real `controller.labels` field was private (F-A3).
///   * Hashed on `spec.request_id` (unique per call) which broke
///     determinism (F-A4) — same seed + same text produced
///     different verdicts.
///
/// Post-fix : pulls real labels via the new public accessor
/// `controller.labels()`, and hashes on the deterministic
/// `(embedding, catalog_id, seed)` tuple by delegating to
/// `controller::synthesise_verdict` which is now `pub(crate)`.
fn synthesise_via_helper(
    controller: &PySwarmController,
    spec: &PySwarmSpec,
    started: Instant,
) -> PyVerdict {
    let labels = controller.labels();
    let seed = controller.seed().or(spec.seed).unwrap_or(0);
    let mut verdict = crate::controller::synthesise_verdict(
        &spec.embedding,
        &controller.catalog_id(),
        &controller.catalog_version(),
        labels.as_slice(),
        seed,
        0.0,  // patched below
    );
    verdict.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_request_id_starts_with_req() {
        let id = generate_request_id();
        assert!(id.starts_with("req-"));
        assert!(id.len() > 4);
    }
}
