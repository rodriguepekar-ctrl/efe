//! `PySwarmSpec` + `PyAlignmentCone` — supporting types for the
//! low-level controller path.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 8.3 (advanced surface).
//!
//! **D25 status** : these types are **simple bags of data** that
//! mirror the public fields of their `efe_core` counterparts. The
//! D26 wiring (DT-F-WIRE-1) replaces the stub `inner_*` accessors
//! with real builder calls into `efe_core::SwarmSpecBuilder` /
//! `efe_core::AlignmentCone`.

use numpy::PyReadonlyArray1;
use pyo3::prelude::*;

use crate::error::{config_err, ffi_err};

/// Python-facing wrapper for `efe_core::types::SwarmSpec`.
#[pyclass(name = "SwarmSpec", module = "efe._native")]
#[derive(Debug, Clone)]
pub struct PySwarmSpec {
    /// Symbolic request identifier (typically a UUID v4 string).
    #[pyo3(get)]
    pub request_id: String,
    /// Symbolic catalog identifier.
    #[pyo3(get)]
    pub catalog_id: String,
    /// Embedding vector — empty until `set_embedding()` is called.
    pub embedding: Vec<f32>,
    /// Optional seed for deterministic runs.
    #[pyo3(get)]
    pub seed: Option<u64>,
    /// Maximum fusion rounds (default 20).
    #[pyo3(get)]
    pub max_rounds: u32,
}

#[pymethods]
impl PySwarmSpec {
    /// Construct manually. Most users build via the builder pattern
    /// `SwarmSpec.builder().request_id(...).build()`.
    #[new]
    #[pyo3(signature = (request_id, catalog_id, *, seed = None, max_rounds = 20))]
    pub fn new(
        request_id: String,
        catalog_id: String,
        seed: Option<u64>,
        max_rounds: u32,
    ) -> PyResult<Self> {
        if request_id.is_empty() {
            return Err(config_err("request_id must not be empty"));
        }
        if catalog_id.is_empty() {
            return Err(config_err("catalog_id must not be empty"));
        }
        if max_rounds == 0 {
            return Err(config_err("max_rounds must be > 0"));
        }
        Ok(Self {
            request_id,
            catalog_id,
            embedding: Vec::new(),
            seed,
            max_rounds,
        })
    }

    /// Returns a builder. Idiomatic Python usage :
    ///
    /// ```python
    /// spec = SwarmSpec.builder() \
    ///     .request_id("test") \
    ///     .catalog_id("router-v1") \
    ///     .embedding(np.zeros(384, dtype=np.float32)) \
    ///     .build()
    /// ```
    #[staticmethod]
    pub fn builder() -> PySwarmSpecBuilder {
        PySwarmSpecBuilder::default()
    }

    /// Set the embedding from a numpy float32 array.
    ///
    /// The array MUST be 1-D, contiguous, and of dtype `float32`.
    /// The length is recorded as the embedding dim ; the controller
    /// validates dim == bundle.dim at execute time.
    fn set_embedding<'py>(
        &mut self,
        embedding: PyReadonlyArray1<'py, f32>,
    ) -> PyResult<()> {
        let slice = embedding.as_slice().map_err(|e| {
            ffi_err(format!("embedding array not contiguous: {e}"))
        })?;
        if slice.is_empty() {
            return Err(config_err("embedding must not be empty"));
        }
        self.embedding = slice.to_vec();
        Ok(())
    }

    /// Length of the embedding vector (0 if not set).
    fn embedding_dim(&self) -> usize {
        self.embedding.len()
    }

    fn __repr__(&self) -> String {
        format!(
            "SwarmSpec(request_id={:?}, catalog_id={:?}, dim={}, max_rounds={}, seed={:?})",
            self.request_id,
            self.catalog_id,
            self.embedding.len(),
            self.max_rounds,
            self.seed
        )
    }
}

/// Builder for [`PySwarmSpec`]. Returned by `SwarmSpec.builder()`.
#[pyclass(name = "SwarmSpecBuilder", module = "efe._native")]
#[derive(Debug, Default, Clone)]
pub struct PySwarmSpecBuilder {
    request_id: Option<String>,
    catalog_id: Option<String>,
    embedding: Option<Vec<f32>>,
    seed: Option<u64>,
    max_rounds: u32,
}

#[pymethods]
impl PySwarmSpecBuilder {
    /// Set the request identifier.
    fn request_id(slf: PyRef<'_, Self>, request_id: String) -> PySwarmSpecBuilder {
        let mut b = (*slf).clone();
        b.request_id = Some(request_id);
        b
    }

    /// Set the catalog identifier.
    fn catalog_id(slf: PyRef<'_, Self>, catalog_id: String) -> PySwarmSpecBuilder {
        let mut b = (*slf).clone();
        b.catalog_id = Some(catalog_id);
        b
    }

    /// Set the embedding vector.
    fn embedding<'py>(
        slf: PyRef<'_, Self>,
        embedding: PyReadonlyArray1<'py, f32>,
    ) -> PyResult<PySwarmSpecBuilder> {
        let slice = embedding.as_slice().map_err(|e| {
            ffi_err(format!("embedding array not contiguous: {e}"))
        })?;
        if slice.is_empty() {
            return Err(config_err("embedding must not be empty"));
        }
        let mut b = (*slf).clone();
        b.embedding = Some(slice.to_vec());
        Ok(b)
    }

    /// Set the seed (optional ; default None = non-deterministic).
    fn seed(slf: PyRef<'_, Self>, seed: u64) -> PySwarmSpecBuilder {
        let mut b = (*slf).clone();
        b.seed = Some(seed);
        b
    }

    /// Set the maximum number of fusion rounds (default 20).
    fn max_rounds(slf: PyRef<'_, Self>, max_rounds: u32) -> PySwarmSpecBuilder {
        let mut b = (*slf).clone();
        b.max_rounds = max_rounds;
        b
    }

    /// Build the final `SwarmSpec`. Returns `ConfigError` if a
    /// required field is missing.
    fn build(&self) -> PyResult<PySwarmSpec> {
        let request_id = self
            .request_id
            .clone()
            .ok_or_else(|| config_err("request_id required"))?;
        let catalog_id = self
            .catalog_id
            .clone()
            .ok_or_else(|| config_err("catalog_id required"))?;
        let embedding = self.embedding.clone().unwrap_or_default();
        let max_rounds = if self.max_rounds == 0 {
            20
        } else {
            self.max_rounds
        };
        Ok(PySwarmSpec {
            request_id,
            catalog_id,
            embedding,
            seed: self.seed,
            max_rounds,
        })
    }
}

// ─────────────────────────────────────────────────────────────────
// AlignmentCone
// ─────────────────────────────────────────────────────────────────

/// Python-facing wrapper for `efe_core::types::AlignmentCone`.
#[pyclass(name = "AlignmentCone", module = "efe._native")]
#[derive(Debug, Clone)]
pub struct PyAlignmentCone {
    /// Cone identifier (UUID-like string).
    #[pyo3(get)]
    pub id: String,
    /// Anchor vectors (each of dim D) — flattened row-major.
    pub anchors_flat: Vec<f32>,
    /// Number of anchors.
    #[pyo3(get)]
    pub n_anchors: u32,
    /// Embedding dim D.
    #[pyo3(get)]
    pub dim: u32,
    /// Maximum projection distance (cosine) tolerated.
    #[pyo3(get)]
    pub max_projection_distance: f32,
}

#[pymethods]
impl PyAlignmentCone {
    /// Construct from anchors (numpy float32 2-D array of shape
    /// `(n_anchors, dim)`) and a max projection distance.
    #[new]
    #[pyo3(signature = (id, anchors, max_projection_distance))]
    pub fn new<'py>(
        id: String,
        anchors: numpy::PyReadonlyArray2<'py, f32>,
        max_projection_distance: f32,
    ) -> PyResult<Self> {
        if id.is_empty() {
            return Err(config_err("cone id must not be empty"));
        }
        if !max_projection_distance.is_finite() || max_projection_distance < 0.0 {
            return Err(config_err(format!(
                "max_projection_distance must be finite ≥ 0 ; got {max_projection_distance}"
            )));
        }
        let arr = anchors.as_array();
        let dims = arr.shape();
        if dims.len() != 2 {
            return Err(config_err(format!(
                "anchors must be a 2-D array ; got rank {}",
                dims.len()
            )));
        }
        let n_anchors = dims[0] as u32;
        let dim = dims[1] as u32;
        if n_anchors == 0 || dim == 0 {
            return Err(config_err("anchors must have ≥ 1 anchor and dim ≥ 1"));
        }
        let flat: Vec<f32> = arr.iter().copied().collect();
        Ok(Self {
            id,
            anchors_flat: flat,
            n_anchors,
            dim,
            max_projection_distance,
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "AlignmentCone(id={:?}, n_anchors={}, dim={}, max_dist={:.4})",
            self.id, self.n_anchors, self.dim, self.max_projection_distance
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use numpy::PyArray1;
    use pyo3::Python;

    #[test]
    fn swarm_spec_builder_round_trip() {
        Python::with_gil(|py| {
            let _ = py;
        });
        let b = PySwarmSpecBuilder::default();
        // we can't easily call PyRef-based methods outside python,
        // so we test the .build() guard logic.
        let r = b.build();
        assert!(r.is_err()); // missing request_id
    }

    #[test]
    fn swarm_spec_builder_complete_build_succeeds() {
        let b = PySwarmSpecBuilder {
            request_id: Some("req-1".into()),
            catalog_id: Some("cat-v1".into()),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            seed: Some(42),
            max_rounds: 10,
        };
        let s = b.build().unwrap();
        assert_eq!(s.request_id, "req-1");
        assert_eq!(s.catalog_id, "cat-v1");
        assert_eq!(s.embedding.len(), 3);
        assert_eq!(s.seed, Some(42));
        assert_eq!(s.max_rounds, 10);
    }

    #[test]
    fn swarm_spec_new_rejects_empty_request_id() {
        let r = PySwarmSpec::new(String::new(), "cat".into(), None, 20);
        assert!(r.is_err());
    }

    #[test]
    fn swarm_spec_new_rejects_zero_max_rounds() {
        let r = PySwarmSpec::new("req".into(), "cat".into(), None, 0);
        assert!(r.is_err());
    }
}
