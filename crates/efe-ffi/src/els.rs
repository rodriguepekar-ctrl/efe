//! `PyEpistemicLatentSpace` — numpy-friendly view of an ELS.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 14.
//!
//! **D25 status** : the implementation is a **stub** that synthesises
//! a deterministic ELS from `(swarm_id, agent_count, dim)` so the
//! API surface is testable end-to-end. The real wiring against
//! `efe_core::els::EpistemicLatentSpace` (which uses `Arena`-backed
//! storage) is DT-F-ELS-1 deferred to D27.
//!
//! Even with the stub, the **public API contract holds** :
//!
//!   * `n_agents`, `latent_dim` getters.
//!   * `as_array()` returns a `numpy.ndarray` of shape `(N, D)`.
//!   * `centroid()` returns a `numpy.ndarray` of shape `(D,)`.

// **Audit fix F-A7** — `PyArrayMethods` was imported as a "future
// proofing" hint but never used after the F-A5 refactor that
// switched from `reshape()` to `from_vec2_bound()`. Removed.
use numpy::{IntoPyArray, PyArray1, PyArray2};
use pyo3::prelude::*;
use sha2::{Digest, Sha256};

/// Python-facing view of an ELS.
#[pyclass(name = "EpistemicLatentSpace", module = "efe._native")]
#[derive(Debug, Clone)]
pub struct PyEpistemicLatentSpace {
    /// Symbolic identifier of the swarm that produced this ELS.
    pub swarm_id: String,
    /// Number of nano-agents (rows of the matrix).
    pub n_agents: u32,
    /// Latent dimension (cols of the matrix).
    pub latent_dim: u32,
    /// Flat row-major storage of the ELS state.
    pub data: Vec<f32>,
}

#[pymethods]
impl PyEpistemicLatentSpace {
    /// Symbolic identifier of the swarm.
    #[getter]
    fn swarm_id(&self) -> String {
        self.swarm_id.clone()
    }

    /// Number of agents (`N`).
    #[getter]
    fn n_agents(&self) -> u32 {
        self.n_agents
    }

    /// Latent dimension (`D`).
    #[getter]
    fn latent_dim(&self) -> u32 {
        self.latent_dim
    }

    /// Returns the ELS as a numpy `float32` 2-D array of shape
    /// `(n_agents, latent_dim)`.
    ///
    /// **D25 status** : this **copies** the underlying data. The
    /// truly zero-copy path via `PyArray2::from_owned_array_bound`
    /// requires an `ndarray::Array2` wrapper around the Rust data ;
    /// landing in DT-F-ELS-1 (D27).
    fn as_array<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray2<f32>>> {
        let n = self.n_agents as usize;
        let d = self.latent_dim as usize;
        // Build a (N, D) numpy array via numpy::PyArray2::from_vec2_bound
        // is the safe path (copies). Zero-copy via ndarray lands in
        // DT-F-ELS-1 (D27).
        //
        // We reshape the flat Vec<f32> into Vec<Vec<f32>> for the
        // 2-D constructor — the cost is N small allocs but matches
        // the D25 stub posture (correctness first, perf in DT-F-ELS-1).
        let mut rows: Vec<Vec<f32>> = Vec::with_capacity(n);
        for r in 0..n {
            rows.push(self.data[r * d..(r + 1) * d].to_vec());
        }
        numpy::PyArray2::from_vec2_bound(py, &rows).map_err(|e| {
            crate::error::ffi_err(format!("PyArray2::from_vec2_bound: {e}"))
        })
    }

    /// Compute and return the centroid (column-wise mean of all
    /// agents) as a numpy `float32` 1-D array of shape `(latent_dim,)`.
    ///
    /// Releases the GIL during the reduction.
    fn centroid<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyArray1<f32>>> {
        let n = self.n_agents as usize;
        let d = self.latent_dim as usize;
        let data_clone = self.data.clone();
        let centroid = py.allow_threads(move || {
            let mut acc = vec![0.0_f32; d];
            for row in 0..n {
                for col in 0..d {
                    acc[col] += data_clone[row * d + col];
                }
            }
            if n > 0 {
                let inv_n = 1.0_f32 / (n as f32);
                for v in &mut acc {
                    *v *= inv_n;
                }
            }
            acc
        });
        Ok(centroid.into_pyarray_bound(py))
    }

    fn __repr__(&self) -> String {
        format!(
            "EpistemicLatentSpace(swarm_id={:?}, n_agents={}, latent_dim={})",
            self.swarm_id, self.n_agents, self.latent_dim
        )
    }
}

impl PyEpistemicLatentSpace {
    /// **D25 stub constructor** — synthesise a deterministic ELS.
    /// Will be called by `PySwarmController::last_els()` accessor
    /// once DT-F-ELS-1 (D27) lands a snapshot path. For D25 this
    /// helper is exercised only by `tests::synthetic_*` ; flagged
    /// `#[allow(dead_code)]` to silence the audit warning until
    /// the snapshot wiring is in place. **Audit fix F-A9**.
    #[allow(dead_code)]
    pub(crate) fn synthetic(swarm_id: &str, n_agents: u32, latent_dim: u32) -> Self {
        let n = n_agents as usize;
        let d = latent_dim as usize;
        let mut data = Vec::with_capacity(n * d);

        // Deterministic seed from swarm_id.
        let mut hasher = Sha256::new();
        hasher.update(swarm_id.as_bytes());
        let seed: [u8; 32] = hasher.finalize().into();

        for row in 0..n {
            for col in 0..d {
                let byte = seed[(row + col) % 32];
                data.push((byte as f32) / 255.0 - 0.5);
            }
        }

        Self {
            swarm_id: swarm_id.to_string(),
            n_agents,
            latent_dim,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_has_correct_shape() {
        let els = PyEpistemicLatentSpace::synthetic("swarm-1", 64, 384);
        assert_eq!(els.n_agents, 64);
        assert_eq!(els.latent_dim, 384);
        assert_eq!(els.data.len(), 64 * 384);
    }

    #[test]
    fn synthetic_is_deterministic() {
        let a = PyEpistemicLatentSpace::synthetic("swarm-x", 4, 8);
        let b = PyEpistemicLatentSpace::synthetic("swarm-x", 4, 8);
        assert_eq!(a.data, b.data);
    }

    #[test]
    fn different_swarm_id_yields_different_data() {
        let a = PyEpistemicLatentSpace::synthetic("swarm-x", 4, 8);
        let b = PyEpistemicLatentSpace::synthetic("swarm-y", 4, 8);
        assert_ne!(a.data, b.data);
    }
}
