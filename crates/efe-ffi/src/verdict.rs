//! `PyVerdict` — Python-facing classification result.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 12.
//!
//! Mirrors the structure of `efe_core::dissolution::OutputBundle`
//! plus controller-level metadata (swarm_id, rounds_executed,
//! catalog_version, latency). Frozen `#[pyclass]` — every field is
//! read-only after construction.

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

/// Result of one classify call.
///
/// All fields are immutable after construction (`frozen` pyclass).
/// Comparable via `==` ; serializable via `to_dict()` for JSON.
#[pyclass(name = "Verdict", frozen, module = "efe._native")]
#[derive(Debug, Clone)]
pub struct PyVerdict {
    /// Predicted label.
    #[pyo3(get)]
    pub label: String,
    /// Confidence score in `[0.0, 1.0]`.
    #[pyo3(get)]
    pub confidence: f32,
    /// Identifier of the alignment cone that gated this verdict.
    #[pyo3(get)]
    pub cone_id: String,
    /// Number of fusion rounds executed before convergence.
    #[pyo3(get)]
    pub rounds_executed: u32,
    /// Unique identifier of this swarm run.
    #[pyo3(get)]
    pub swarm_id: String,
    /// Version of the catalog hit (semver string).
    #[pyo3(get)]
    pub catalog_version: String,
    /// Top-N alternative labels with their scores.
    #[pyo3(get)]
    pub sub_scores: Vec<(String, f32)>,
    /// Wall-clock latency of the classify call in milliseconds.
    #[pyo3(get)]
    pub latency_ms: f64,
}

#[pymethods]
impl PyVerdict {
    /// Construct manually. Production paths use the internal
    /// `from_response` constructor instead.
    #[new]
    #[pyo3(signature = (
        label,
        confidence,
        cone_id,
        rounds_executed,
        swarm_id,
        catalog_version,
        sub_scores = vec![],
        latency_ms = 0.0,
    ))]
    pub fn new(
        label: String,
        confidence: f32,
        cone_id: String,
        rounds_executed: u32,
        swarm_id: String,
        catalog_version: String,
        sub_scores: Vec<(String, f32)>,
        latency_ms: f64,
    ) -> Self {
        Self {
            label,
            confidence,
            cone_id,
            rounds_executed,
            swarm_id,
            catalog_version,
            sub_scores,
            latency_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "Verdict(label={:?}, confidence={:.4}, rounds={}, latency_ms={:.2})",
            self.label, self.confidence, self.rounds_executed, self.latency_ms
        )
    }

    fn __eq__(&self, other: &Self) -> bool {
        self.label == other.label
            && (self.confidence - other.confidence).abs() < 1e-6
            && self.cone_id == other.cone_id
            && self.rounds_executed == other.rounds_executed
            && self.swarm_id == other.swarm_id
            && self.catalog_version == other.catalog_version
    }

    fn __hash__(&self) -> u64 {
        // Python hash should be stable per-process. We hash the
        // identifying fields ; confidence is excluded because float
        // hash + equality with epsilon is brittle.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        self.label.hash(&mut h);
        self.cone_id.hash(&mut h);
        self.swarm_id.hash(&mut h);
        h.finish()
    }

    /// Convert to a Python dict — useful for JSON serialization.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new_bound(py);
        dict.set_item("label", &self.label)?;
        dict.set_item("confidence", self.confidence)?;
        dict.set_item("cone_id", &self.cone_id)?;
        dict.set_item("rounds_executed", self.rounds_executed)?;
        dict.set_item("swarm_id", &self.swarm_id)?;
        dict.set_item("catalog_version", &self.catalog_version)?;
        // Convert sub_scores Vec<(String, f32)> into a Python list
        // of tuples.
        let sub_scores_list = PyList::new_bound(py, &self.sub_scores);
        dict.set_item("sub_scores", sub_scores_list)?;
        dict.set_item("latency_ms", self.latency_ms)?;
        Ok(dict)
    }
}

impl PyVerdict {
    /// Build from internal Rust types. Not exposed to Python.
    ///
    /// **D25 status** : the `OutputBundle` argument is a placeholder
    /// — the real wiring through `efe_core::SwarmController::execute`
    /// lives in `controller.rs` and `client.rs`. This constructor
    /// is the joint produit by all those paths.
    pub(crate) fn from_synthetic(
        label: String,
        confidence: f32,
        cone_id: String,
        rounds_executed: u32,
        swarm_id: String,
        catalog_version: String,
        latency_ms: f64,
    ) -> Self {
        Self {
            label,
            confidence,
            cone_id,
            rounds_executed,
            swarm_id,
            catalog_version,
            sub_scores: Vec::new(),
            latency_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyo3::Python;

    #[test]
    fn new_then_repr_matches_format() {
        let v = PyVerdict::new(
            "billing-dispute".into(),
            0.9876,
            "cone-1".into(),
            12,
            "swarm-abc".into(),
            "v1".into(),
            vec![],
            42.5,
        );
        let repr = v.__repr__();
        assert!(repr.contains("billing-dispute"));
        assert!(repr.contains("0.9876"));
        assert!(repr.contains("rounds=12"));
        assert!(repr.contains("42.50"));
    }

    #[test]
    fn eq_within_epsilon() {
        let a = PyVerdict::new(
            "x".into(), 0.5_f32, "c".into(), 1, "s".into(), "v".into(), vec![], 0.0
        );
        let mut b = a.clone();
        b.confidence = 0.5_f32 + 1e-7;
        assert!(a.__eq__(&b));
        b.confidence = 0.6_f32;
        assert!(!a.__eq__(&b));
    }

    #[test]
    fn to_dict_carries_all_fields() {
        Python::with_gil(|py| {
            let v = PyVerdict::new(
                "hello".into(),
                0.7,
                "cone".into(),
                5,
                "swarm".into(),
                "1.0.0".into(),
                vec![("alt-1".into(), 0.3)],
                10.0,
            );
            let d = v.to_dict(py).unwrap();
            assert_eq!(
                d.get_item("label").unwrap().unwrap().extract::<String>().unwrap(),
                "hello"
            );
            assert_eq!(
                d.get_item("rounds_executed").unwrap().unwrap().extract::<u32>().unwrap(),
                5
            );
        });
    }
}
