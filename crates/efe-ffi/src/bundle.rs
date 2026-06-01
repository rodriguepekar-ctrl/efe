//! `PyBundle` + `load_bundle()` — embedder bundle wrapper.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 16.
//!
//! **D25 status** : the bundle is a **stub** — `encode(text)`
//! returns a deterministic SHA-256-derived embedding rather than a
//! real ONNX/sentence-transformers forward pass. Wiring to the
//! real `efe_server::embedder::Bundle` is DT-F-WIRE-2 (D26). The
//! stub allows the rest of the FFI surface (Verdict, Client, etc.)
//! to be tested + benchmarked end-to-end without a heavy embedder.

use std::path::PathBuf;
use std::sync::Arc;

use numpy::IntoPyArray;
use numpy::PyArray1;
use pyo3::prelude::*;
use sha2::{Digest, Sha256};

// **Audit fix F-A5** — `ffi_err` was imported but never used in
// this module. Removed.
use crate::error::bundle_err;

/// Inner Rust struct — not exposed to Python.
///
/// **D25 stub** : tracks the bundle path + a fixed embedding
/// dimension. The `encode()` path produces a deterministic vector
/// derived from `SHA256(text || bundle_id)` so cross-test
/// reproducibility holds.
pub struct InnerBundle {
    /// Symbolic bundle identifier (mirrors `Manifest.bundle_id` in
    /// the production path).
    pub id: String,
    /// Symbolic bundle version (mirrors `Manifest.version`).
    pub version: String,
    /// Source path on disk (recorded for diagnostics).
    pub path: PathBuf,
    /// Fingerprint as 32-byte SHA-256 of the manifest. Stub : SHA-256
    /// of the path string.
    pub fingerprint: [u8; 32],
    /// Embedding dimension. Hard-coded to 384 for the D25 stub
    /// (matches paraphrase-multilingual-MiniLM-L12-v2 default).
    pub dim: usize,
}

impl InnerBundle {
    /// Encode `text` into a deterministic embedding vector.
    ///
    /// **D25 stub algorithm** :
    ///
    ///   * `seed = SHA256(text || id)` — 32 bytes.
    ///   * `vec[i] = (seed[i % 32] as f32) / 255.0 - 0.5` for
    ///     `i ∈ [0, dim)`.
    ///   * L2-normalised at the end.
    ///
    /// This is **NOT** a real semantic embedding — it does NOT
    /// preserve meaning. It only guarantees :
    ///
    ///   * Same text → same vector (determinism).
    ///   * Different text → different vector (with overwhelming
    ///     probability).
    ///   * L2-norm = 1.0 (sane numerical regime for downstream
    ///     fusion / cone projection).
    pub fn encode(&self, text: &str) -> Vec<f32> {
        let mut hasher = Sha256::new();
        hasher.update(text.as_bytes());
        hasher.update(self.id.as_bytes());
        let seed: [u8; 32] = hasher.finalize().into();

        let mut out = Vec::with_capacity(self.dim);
        for i in 0..self.dim {
            let byte = seed[i % 32];
            out.push((byte as f32) / 255.0 - 0.5);
        }

        // L2-normalise.
        let norm: f32 = out.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut out {
                *v /= norm;
            }
        }

        out
    }
}

impl std::fmt::Debug for InnerBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InnerBundle")
            .field("id", &self.id)
            .field("version", &self.version)
            .field("dim", &self.dim)
            .field("path", &self.path)
            .finish()
    }
}

/// Python-facing bundle.
///
/// Holds an `Arc<InnerBundle>` so multiple `EfeNativeClient` /
/// `SwarmController` instances can share the same loaded bundle
/// without re-loading.
#[pyclass(name = "Bundle", module = "efe._native")]
pub struct PyBundle {
    pub(crate) inner: Arc<InnerBundle>,
}

#[pymethods]
impl PyBundle {
    /// Bundle identifier (e.g. `"paraphrase-multilingual-MiniLM-L12-v2"`).
    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    /// Bundle version (semver string).
    #[getter]
    fn version(&self) -> String {
        self.inner.version.clone()
    }

    /// Hex-encoded SHA-256 fingerprint (64 chars).
    #[getter]
    fn fingerprint(&self) -> String {
        hex::encode(self.inner.fingerprint)
    }

    /// Embedding dimension.
    #[getter]
    fn dim(&self) -> usize {
        self.inner.dim
    }

    fn __repr__(&self) -> String {
        format!(
            "Bundle(id={:?}, version={:?}, dim={}, fingerprint={}…)",
            self.inner.id,
            self.inner.version,
            self.inner.dim,
            &hex::encode(self.inner.fingerprint)[..16]
        )
    }

    /// Encode `text` to its embedding vector.
    ///
    /// Returns a `numpy.float32` 1-D array of shape `(dim,)`.
    /// Releases the GIL during the encode (D25 stub : SHA-256
    /// path ; production path : ONNX forward pass).
    fn encode<'py>(
        &self,
        py: Python<'py>,
        text: &str,
    ) -> PyResult<Bound<'py, PyArray1<f32>>> {
        if text.is_empty() {
            return Err(bundle_err("text must not be empty"));
        }
        let inner = Arc::clone(&self.inner);
        let owned_text = text.to_string();
        let embedding = py.allow_threads(move || inner.encode(&owned_text));
        Ok(embedding.into_pyarray_bound(py))
    }
}

/// Top-level `load_bundle(path)` function.
///
/// Loads (or stubs) the bundle at `path` and returns a `Bundle`
/// wrapper. **D25 stub** : reads the path's last segment as `id`,
/// derives a fingerprint from `SHA256(canonical_path)`, and pins
/// `dim = 384`. Real loader lives in DT-F-WIRE-2 (D26).
///
/// # Errors
///
/// * `BundleError` if `path` is empty.
/// * `FfiError` if the path canonicalisation fails (rare — only
///   weird symlink loops).
#[pyfunction]
#[pyo3(text_signature = "(path)")]
pub fn load_bundle(path: &str) -> PyResult<PyBundle> {
    if path.is_empty() {
        return Err(bundle_err("path must not be empty"));
    }
    let pb = PathBuf::from(path);

    // Derive a stable bundle identifier from the last segment.
    let id = pb
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "unknown-bundle".to_string());
    let version = "0.0.0-stub".to_string();

    // Fingerprint = SHA256 of the canonical path string.
    let canonical = pb
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| pb.to_string_lossy().into_owned());
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let fingerprint: [u8; 32] = hasher.finalize().into();

    let inner = InnerBundle {
        id,
        version,
        path: pb,
        fingerprint,
        dim: 384,
    };

    Ok(PyBundle {
        inner: Arc::new(inner),
    })
}

// ====================================================================
// Test-only synthetic constructor (Φ-C-FFI Completion D26).
//
// Used by `cache::tests` + `controller::tests` to materialise a
// `PyBundle` without touching disk / `load_bundle`. Gated behind
// `#[cfg(test)]` so it never leaks into a production build of the
// cdylib wheel — note : `#[cfg(test)]` propagates across modules of
// the same crate during a `cargo test` build, so `cache::tests` sees
// it just fine.
// ====================================================================

#[cfg(test)]
impl PyBundle {
    /// Construct a fully-formed `PyBundle` synthetically. Reserved
    /// for cross-module Rust-side tests where the disk path doesn't
    /// matter (only the inner Arc identity does).
    ///
    /// The `dim` is fixed at 384 (= matches the D25 stub).
    pub fn synthetic_for_test() -> Self {
        let inner = InnerBundle {
            id: "test-bundle".to_string(),
            version: "0.0.0-test".to_string(),
            path: std::path::PathBuf::from("/dev/null"),
            fingerprint: [0u8; 32],
            dim: 384,
        };
        Self {
            inner: std::sync::Arc::new(inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inner_encode_is_deterministic() {
        let inner = InnerBundle {
            id: "test".into(),
            version: "0.0.0".into(),
            path: PathBuf::from("/tmp"),
            fingerprint: [0u8; 32],
            dim: 16,
        };
        let a = inner.encode("hello");
        let b = inner.encode("hello");
        assert_eq!(a, b);
        let c = inner.encode("world");
        assert_ne!(a, c);
    }

    #[test]
    fn inner_encode_l2_norm_is_one() {
        let inner = InnerBundle {
            id: "test".into(),
            version: "0.0.0".into(),
            path: PathBuf::from("/tmp"),
            fingerprint: [0u8; 32],
            dim: 384,
        };
        let v = inner.encode("hello");
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "L2 norm {norm} != 1.0");
    }

    #[test]
    fn inner_encode_dim_matches() {
        let inner = InnerBundle {
            id: "t".into(),
            version: "0".into(),
            path: PathBuf::from("/"),
            fingerprint: [0u8; 32],
            dim: 384,
        };
        let v = inner.encode("text");
        assert_eq!(v.len(), 384);
    }

    #[test]
    fn load_bundle_rejects_empty_path() {
        Python::with_gil(|py| {
            let _ = py;
            let r = load_bundle("");
            assert!(r.is_err());
        });
    }
}
