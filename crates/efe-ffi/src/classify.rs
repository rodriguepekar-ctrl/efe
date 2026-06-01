//! `classify_zero_copy()` — top-level one-shot helper.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 15.
//!
//! The simplest possible API : one function call, one Verdict back.
//! Useful for CLI scripts and notebooks that don't need a persistent
//! client object.
//!
//! **D25 status** : per-call bundle + controller construction. The
//! LRU bundle / controller cache lands in DT-F-CACHE-1/2 (D26).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use pyo3::prelude::*;

use crate::bundle::{load_bundle, PyBundle};
use crate::cache::{global as bundle_cache, CacheKey};
use crate::client::PyEfeNativeClient;
use crate::error::{config_err, bundle_err};
use crate::verdict::PyVerdict;

/// One-shot classify : load bundle, build controller, run swarm,
/// return verdict.
///
/// # Arguments
///
/// * `text` — input text to classify (non-empty).
/// * `catalog_id` — symbolic catalog identifier (logged + threaded
///   into the spec ; the actual catalog is loaded from `catalog_path`).
/// * `bundle_path` — path to the embedder bundle on disk.
/// * `catalog_path` — path to the catalog JSON.
/// * `gpu_enabled` — opt into GPU substrate (default `False`).
/// * `seed` — optional seed for deterministic runs.
///
/// # Errors
///
/// * `ConfigError` if any string argument is empty.
/// * `BundleError` if the bundle cannot be loaded.
/// * `CatalogError` if the catalog cannot be loaded.
/// * `SwarmError` if the swarm execution fails.
///
/// # Example
///
/// ```python
/// from efe import classify_zero_copy
///
/// verdict = classify_zero_copy(
///     "I want to dispute the charge",
///     catalog_id="router-v1",
///     bundle_path="/etc/efe/bundles/active",
///     catalog_path="/etc/efe/catalogs/router-v1.json",
/// )
/// print(verdict.label)
/// ```
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
pub fn classify_zero_copy(
    py: Python<'_>,
    text: &str,
    catalog_id: &str,
    bundle_path: &str,
    catalog_path: &str,
    gpu_enabled: bool,
    seed: Option<u64>,
) -> PyResult<PyVerdict> {
    if text.is_empty() {
        return Err(config_err("text must not be empty"));
    }
    if catalog_id.is_empty() {
        return Err(config_err("catalog_id must not be empty"));
    }
    if bundle_path.is_empty() {
        return Err(config_err("bundle_path must not be empty"));
    }
    if catalog_path.is_empty() {
        return Err(config_err("catalog_path must not be empty"));
    }

    let _started = Instant::now();
    let _ = catalog_id; // future : verify spec.catalog_id == catalog_id

    // **Φ-C-FFI Completion (D26)** — bundle LRU cache (G17). Pre-fix
    // pattern (D25 stub) loaded the bundle on EVERY call, which on
    // disk-cold paths cost 50–100 ms. With the cache, hits are O(1)
    // Arc clones ; misses match the D25 cost. Le cache est keyed
    // par `(canonical_path, mtime)` pour catch les bundle swaps.
    let cache_key = match CacheKey::probe(&PathBuf::from(bundle_path)) {
        Some(k) => k,
        None => {
            return Err(bundle_err(format!(
                "bundle path not found or inaccessible: {bundle_path}",
            )));
        }
    };
    let cached_bundle: Arc<PyBundle> = bundle_cache().get_or_load(
        cache_key,
        || load_bundle(bundle_path),
    )?;

    // Materialise a `PyBundle` from the cached `Arc<PyBundle>` so
    // we can hand a `PyRef` to `PyEfeNativeClient::new`. The inner
    // `InnerBundle` Arc is shared, so this is cheap.
    let py_bundle = PyBundle {
        inner: Arc::clone(&cached_bundle.inner),
    };
    let bundle_ref = Py::new(py, py_bundle)?;
    let bundle_pyref = bundle_ref.borrow(py);

    let mut client = PyEfeNativeClient::new(
        bundle_pyref,
        catalog_path.to_string(),
        gpu_enabled,
        seed,
    )?;

    let verdict = client.classify(py, text)?;
    client.close();
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    // Real-Python tests live in `python/efe/tests/test_classify.py`.
    // Rust-side coverage focuses on the input-validation guard rails.
    use super::*;
    use pyo3::Python;

    #[test]
    fn classify_zero_copy_rejects_empty_text() {
        Python::with_gil(|py| {
            let r = classify_zero_copy(py, "", "cat", "bundle", "/tmp/cat.json", false, None);
            assert!(r.is_err());
        });
    }

    #[test]
    fn classify_zero_copy_rejects_empty_catalog_id() {
        Python::with_gil(|py| {
            let r = classify_zero_copy(py, "hi", "", "bundle", "/tmp/cat.json", false, None);
            assert!(r.is_err());
        });
    }

    #[test]
    fn classify_zero_copy_rejects_empty_bundle_path() {
        Python::with_gil(|py| {
            let r = classify_zero_copy(py, "hi", "cat", "", "/tmp/cat.json", false, None);
            assert!(r.is_err());
        });
    }
}
