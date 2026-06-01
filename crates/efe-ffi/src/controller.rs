//! `PySwarmController` — low-level wrapper for advanced users.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 13.
//!
//! Most users prefer [`crate::client::PyEfeNativeClient`] which
//! handles bundle + catalog wiring automatically. This low-level
//! controller is exposed for :
//!
//!   * Integration tests that bypass the embedder (inject a
//!     pre-computed embedding via `SwarmSpec::set_embedding`).
//!   * Research notebooks needing fine-grained control over
//!     `max_rounds`, `seed`, etc.
//!   * Validation of D26 wiring (`DT-F-WIRE-1`).
//!
//! **D25 status** : `execute()` returns a **synthetic verdict**
//! derived deterministically from the spec — the full wiring to
//! `efe_core::SwarmController::execute` lands in DT-F-WIRE-1 (D26).

use std::sync::Arc;
use std::time::Instant;

use pyo3::prelude::*;
use sha2::{Digest, Sha256};

// **Audit fix F-A6** — `catalog_err` import was never used in this
// module ; removed.
use crate::error::config_err;
use crate::types::PySwarmSpec;
use crate::verdict::PyVerdict;

/// Python-facing low-level swarm controller.
#[pyclass(name = "SwarmController", module = "efe._native")]
pub struct PySwarmController {
    /// Path to the catalog JSON (recorded for diagnostics).
    /// **Audit fix F-A8** — diagnostic-only field ; never read at
    /// runtime but useful for tracing. `#[allow(dead_code)]` until
    /// the D26 wiring (DT-F-WIRE-1) actually consumes it.
    #[allow(dead_code)]
    catalog_path: String,
    /// Resolved catalog id (parsed from the JSON top-level `id`).
    catalog_id: String,
    /// Resolved catalog version (parsed from the JSON `version`).
    catalog_version: String,
    /// Whether GPU substrate is enabled.
    gpu_enabled: bool,
    /// Optional seed for deterministic runs.
    seed: Option<u64>,
    /// Inner labels. **D25 stub** — populated from the catalog
    /// JSON. Production controller holds `Arc<SwarmController>`
    /// from efe-core (DT-F-WIRE-1).
    labels: Arc<Vec<String>>,
}

#[pymethods]
impl PySwarmController {
    /// Construct from a catalog JSON path.
    ///
    /// **D25 stub** : reads the catalog JSON, extracts `id`,
    /// `version`, and `labels`. The full controller wiring (genesis
    /// + fusion + consensus + dissolution + alignment) is DT-F-WIRE-1
    /// in D26.
    #[new]
    #[pyo3(signature = (catalog_path, *, gpu_enabled = false, seed = None))]
    pub fn new(
        catalog_path: String,
        gpu_enabled: bool,
        seed: Option<u64>,
    ) -> PyResult<Self> {
        if catalog_path.is_empty() {
            return Err(config_err("catalog_path must not be empty"));
        }

        // **D25 stub** : try to read the catalog JSON. If the file
        // is missing, derive a synthetic catalog from the path
        // string so the rest of the FFI surface remains testable
        // without a real catalog asset.
        let (catalog_id, catalog_version, labels) =
            try_load_catalog(&catalog_path).unwrap_or_else(|_| {
                synthetic_catalog_from_path(&catalog_path)
            });

        Ok(Self {
            catalog_path,
            catalog_id,
            catalog_version,
            gpu_enabled,
            seed,
            labels: Arc::new(labels),
        })
    }

    /// Catalog id (read from the JSON or synthesised).
    #[getter]
    pub fn catalog_id(&self) -> String {
        self.catalog_id.clone()
    }

    /// Catalog version.
    #[getter]
    pub fn catalog_version(&self) -> String {
        self.catalog_version.clone()
    }

    /// Whether the controller targets the GPU substrate.
    #[getter]
    pub fn gpu_enabled(&self) -> bool {
        self.gpu_enabled
    }

    /// Configured seed (None = non-deterministic).
    #[getter]
    pub fn seed(&self) -> Option<u64> {
        self.seed
    }

    /// Number of labels in the catalog.
    pub fn n_labels(&self) -> usize {
        self.labels.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "SwarmController(catalog_id={:?}, version={:?}, n_labels={}, gpu={}, seed={:?})",
            self.catalog_id,
            self.catalog_version,
            self.labels.len(),
            self.gpu_enabled,
            self.seed
        )
    }

    /// Execute one swarm classification + return a [`Verdict`].
    ///
    /// Releases the GIL during the heavy work.
    ///
    /// **Audit fix F-A2 + F-A4 (D25 audit pass 1)** : pre-fix the
    /// `started.elapsed()` was captured BEFORE the work moved into
    /// `allow_threads` — the verdict reported ~0 ms latency
    /// regardless of the actual elapsed time. The fix moves the
    /// measurement INSIDE the closure so it captures the synthesis
    /// duration (and, post-DT-F-WIRE-1, the real swarm execution).
    ///
    /// **Audit fix F-A4** : the synthesis no longer hashes on
    /// `spec.request_id` (which is unique per call) — that broke
    /// SLO F-12 (determinism). Now hashes on the **embedding bytes**
    /// + catalog_id + seed so two calls with the same `(text, catalog,
    /// seed)` produce identical verdicts.
    ///
    /// **D25 stub algorithm** :
    ///   * Take `SHA256(embedding_bytes || catalog_id || seed)`.
    ///   * Use the first 4 bytes as an index into `labels`.
    ///   * `confidence = (5th-8th bytes as u32) / u32::MAX`.
    ///   * `rounds_executed = 4 + ((9th byte) % 16)`.
    fn execute(&self, py: Python<'_>, spec: &PySwarmSpec) -> PyResult<PyVerdict> {
        let labels = Arc::clone(&self.labels);
        let embedding = spec.embedding.clone();
        let catalog_id = self.catalog_id.clone();
        let catalog_version = self.catalog_version.clone();
        let seed = self.seed.or(spec.seed).unwrap_or(0);

        // GIL released during the synthesis. Latency measured INSIDE
        // the closure so it captures the actual work duration.
        let verdict = py.allow_threads(move || {
            let started = Instant::now();
            let v = synthesise_verdict(
                &embedding,
                &catalog_id,
                &catalog_version,
                &labels,
                seed,
                0.0,  // placeholder — we patch the latency_ms below
            );
            (v, started.elapsed().as_secs_f64() * 1000.0)
        });

        let (mut verdict, latency_ms) = verdict;
        verdict.latency_ms = latency_ms;
        Ok(verdict)
    }
}

// ─────────────────────────────────────────────────────────────────
// Rust-only accessors (NOT exposed to Python).
//
// **Audit fix F-A3** — `pub fn labels()` returning `Arc<Vec<String>>`
// can't live inside `#[pymethods]` because PyO3 has no `OkWrap`
// impl for `Arc<Vec<String>>`. We expose it in a separate `impl`
// block ; only Rust crate consumers (e.g. `client.rs`) call it.
// ─────────────────────────────────────────────────────────────────

impl PySwarmController {
    /// Returns the catalog labels — a clone-cheap `Arc<Vec<String>>`.
    /// Used by [`crate::client`] to build verdict labels matching
    /// the catalog rather than synthesised placeholders.
    pub fn labels(&self) -> Arc<Vec<String>> {
        Arc::clone(&self.labels)
    }
}

// ─────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────

/// **D25 stub** — synthesise a verdict deterministically from
/// `(embedding, catalog_id, seed)`. Real path : DT-F-WIRE-1 (D26).
///
/// **Audit fix F-A4** — pre-fix the hash mixed in `request_id`
/// (which is unique per call) ; same `(text, catalog, seed)` would
/// produce different verdicts on two calls. The architecture
/// commits to determinism (SLO F-12), so the hash now uses the
/// embedding bytes — a deterministic function of `(text,
/// bundle_id)` — plus catalog_id + seed.
// **Φ-C-FFI Completion** — visibilité bumpée de `pub(crate)` à `pub`
// pour autoriser les Criterion benches (`benches/ffi_overhead.rs`) à
// mesurer l'overhead du stub D25 directement. Le helper reste un
// **stub** que DT-F-WIRE-1 (D29) remplace par le vrai
// `efe_core::SwarmController::execute` — le contrat public reste
// "synthèse déterministe", pas une promesse de behaviour réel.
pub fn synthesise_verdict(
    embedding: &[f32],
    catalog_id: &str,
    catalog_version: &str,
    labels: &[String],
    seed: u64,
    initial_latency_ms: f64,
) -> PyVerdict {
    let mut hasher = Sha256::new();
    // Hash the embedding bytes (deterministic given same text +
    // bundle). For numerically equivalent embeddings (rare but
    // possible across bundle versions), the verdict is identical.
    for v in embedding {
        hasher.update(v.to_le_bytes());
    }
    hasher.update(catalog_id.as_bytes());
    hasher.update(seed.to_le_bytes());
    let h: [u8; 32] = hasher.finalize().into();

    let label_idx = u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as usize % labels.len().max(1);
    let label = labels
        .get(label_idx)
        .cloned()
        .unwrap_or_else(|| "unknown".to_string());

    let conf_u32 = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
    let confidence = (conf_u32 as f32) / (u32::MAX as f32);

    let rounds_executed = 4 + (h[8] as u32 % 16);

    let swarm_id = format!(
        "swarm-{}",
        &hex::encode(&h[..8])
    );

    PyVerdict::from_synthetic(
        label,
        confidence,
        format!("cone-{catalog_id}"),
        rounds_executed,
        swarm_id,
        catalog_version.to_string(),
        initial_latency_ms,
    )
}

/// **D25 stub** — try to load a catalog JSON. Returns (id, version,
/// labels). The expected JSON shape :
///
/// ```json
/// {
///   "id": "router-v1",
///   "version": "1.0.0",
///   "labels": ["billing-dispute", "tech-support", "refund", "other"]
/// }
/// ```
fn try_load_catalog(
    path: &str,
) -> Result<(String, String, Vec<String>), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes)?;
    let id = parsed
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("catalog JSON missing 'id'")?
        .to_string();
    let version = parsed
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.0")
        .to_string();
    let labels: Vec<String> = parsed
        .get("labels")
        .and_then(|v| v.as_array())
        .ok_or("catalog JSON missing 'labels' array")?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    if labels.is_empty() {
        return Err("catalog has no labels".into());
    }
    Ok((id, version, labels))
}

/// Synthesise a catalog from a path string when the JSON file is
/// missing. Used only in the D25 stub to keep tests reproducible
/// without real catalog files.
fn synthetic_catalog_from_path(path: &str) -> (String, String, Vec<String>) {
    let id = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown-catalog")
        .to_string();
    let version = "0.0.0-stub".to_string();
    let labels = vec![
        "billing-dispute".to_string(),
        "tech-support".to_string(),
        "refund-request".to_string(),
        "general-inquiry".to_string(),
        "other".to_string(),
    ];
    (id, version, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesise_verdict_is_deterministic() {
        let labels = vec!["a".into(), "b".into(), "c".into()];
        let emb = vec![0.1_f32, 0.2, 0.3, 0.4];
        let v1 = synthesise_verdict(&emb, "cat-1", "v1", &labels, 42, 0.0);
        let v2 = synthesise_verdict(&emb, "cat-1", "v1", &labels, 42, 0.0);
        assert_eq!(v1.label, v2.label);
        assert_eq!(v1.swarm_id, v2.swarm_id);
        assert!((v1.confidence - v2.confidence).abs() < 1e-6);
    }

    #[test]
    fn synthesise_verdict_different_embedding_yields_different_verdict() {
        let labels = vec!["a".into(), "b".into(), "c".into()];
        let v1 = synthesise_verdict(&[0.1_f32, 0.2], "cat-1", "v1", &labels, 0, 0.0);
        let v2 = synthesise_verdict(&[0.3_f32, 0.4], "cat-1", "v1", &labels, 0, 0.0);
        assert_ne!(v1.swarm_id, v2.swarm_id);
    }

    #[test]
    fn synthesise_verdict_different_seed_yields_different_verdict() {
        let labels = vec!["a".into(), "b".into(), "c".into()];
        let emb = vec![0.1_f32, 0.2, 0.3];
        let v1 = synthesise_verdict(&emb, "cat-1", "v1", &labels, 42, 0.0);
        let v2 = synthesise_verdict(&emb, "cat-1", "v1", &labels, 43, 0.0);
        assert_ne!(v1.swarm_id, v2.swarm_id);
    }

    #[test]
    fn synthetic_catalog_from_path_extracts_stem() {
        let (id, version, labels) = synthetic_catalog_from_path("/etc/efe/catalogs/router-v1.json");
        assert_eq!(id, "router-v1");
        assert_eq!(version, "0.0.0-stub");
        assert!(!labels.is_empty());
    }
}
