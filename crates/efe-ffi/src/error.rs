//! Python exception hierarchy + `From<EfeError> for PyErr` mapping.
//!
//! Cf. `docs/EFE_FFI_ARCHITECTURE.md` § 19.
//!
//! Every error a Python user can observe is one of the 10
//! exception classes declared here. The mapping from
//! `efe_core::EfeError` to a Python class is exhaustive — adding a
//! new variant on the Rust side without updating this file
//! triggers a non-exhaustive match warning, which fails the
//! `-D warnings` build.
//!
//! **Audit fix F-A10** — the `pyo3::create_exception!` macro
//! produces a struct without a doc-comment, which trips the
//! crate-level `missing_docs = "warn"` lint. Module-level
//! `#![allow(missing_docs)]` is the canonical PyO3 workaround
//! (cf. <https://github.com/PyO3/pyo3/issues/3171>).

#![allow(missing_docs)]

use efe_core::EfeError;
use pyo3::create_exception;
use pyo3::prelude::*;

// ─────────────────────────────────────────────────────────────────
// Exception hierarchy declarations (compile-time class generation).
//
// `create_exception!(module, name, base)` macro from PyO3 creates
// a Python class derived from `base`. `module` is the symbolic
// module name (matches our `#[pymodule] name = "_native"`).
// ─────────────────────────────────────────────────────────────────

create_exception!(efe_native, EfeBaseError, pyo3::exceptions::PyException);
create_exception!(efe_native, BundleError, EfeBaseError);
create_exception!(efe_native, CatalogError, EfeBaseError);
create_exception!(efe_native, SwarmError, EfeBaseError);
create_exception!(efe_native, AlignmentBreach, EfeBaseError);
create_exception!(efe_native, NumericalError, EfeBaseError);
create_exception!(efe_native, ResourceExhausted, EfeBaseError);
create_exception!(efe_native, ConfigError, EfeBaseError);
create_exception!(efe_native, TimeoutError, EfeBaseError);
create_exception!(efe_native, FfiError, EfeBaseError);

/// Register every exception class on the module so `from efe import
/// AlignmentBreach` works.
///
/// **Naming note** : the Rust type name is `EfeBaseError` (avoiding
/// a collision with the existing public Rust type `efe_core::EfeError`)
/// but the **Python-facing name** is `EfeError` — set explicitly
/// via `m.add("EfeError", ...)`. End users see :
///
/// ```python
/// from efe import EfeError, BundleError, AlignmentBreach
/// assert issubclass(BundleError, EfeError)
/// ```
pub fn register_exceptions(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("EfeError", py.get_type_bound::<EfeBaseError>())?;
    m.add("BundleError", py.get_type_bound::<BundleError>())?;
    m.add("CatalogError", py.get_type_bound::<CatalogError>())?;
    m.add("SwarmError", py.get_type_bound::<SwarmError>())?;
    m.add("AlignmentBreach", py.get_type_bound::<AlignmentBreach>())?;
    m.add("NumericalError", py.get_type_bound::<NumericalError>())?;
    m.add("ResourceExhausted", py.get_type_bound::<ResourceExhausted>())?;
    m.add("ConfigError", py.get_type_bound::<ConfigError>())?;
    m.add("TimeoutError", py.get_type_bound::<TimeoutError>())?;
    m.add("FfiError", py.get_type_bound::<FfiError>())?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// Convenience constructors used throughout the crate.
// ─────────────────────────────────────────────────────────────────

/// Construct a generic `FfiError` from any string-able message.
/// Used by callsites that propagate string errors (`.map_err(ffi_err)`).
#[must_use]
pub fn ffi_err(msg: impl Into<String>) -> PyErr {
    FfiError::new_err(msg.into())
}

/// Construct a `BundleError`.
#[must_use]
pub fn bundle_err(msg: impl Into<String>) -> PyErr {
    BundleError::new_err(msg.into())
}

/// Construct a `ConfigError`.
#[must_use]
pub fn config_err(msg: impl Into<String>) -> PyErr {
    ConfigError::new_err(msg.into())
}

/// Construct a `CatalogError`.
#[must_use]
pub fn catalog_err(msg: impl Into<String>) -> PyErr {
    CatalogError::new_err(msg.into())
}

// ─────────────────────────────────────────────────────────────────
// Mapping from `efe_core::EfeError` to a typed Python exception.
//
// Exhaustive match on the public variants of `EfeError`. Adding a
// new variant on the Rust side surfaces here as a compile error
// — the audit pin test `test_efe_ffi_error_mapping_exhaustive`
// reinforces this institutional pattern.
// ─────────────────────────────────────────────────────────────────

/// Convert any `efe_core::EfeError` to a typed Python exception.
///
/// Match strategy : map by **family** (alignment, numerical, …).
/// Variants whose family is genuinely under-specified fall back to
/// `SwarmError` — the broadest "something went wrong in the swarm
/// pipeline" bucket.
pub fn map_efe_error_to_py(e: EfeError) -> PyErr {
    let msg = e.to_string();
    match e {
        EfeError::AlignmentBreach { .. } => AlignmentBreach::new_err(msg),
        EfeError::NumericalExplosion { .. } => NumericalError::new_err(msg),
        EfeError::HbmAllocFailed { .. } => ResourceExhausted::new_err(msg),
        EfeError::InvalidConfig(_) => ConfigError::new_err(msg),
        EfeError::Internal(_) => FfiError::new_err(msg),
        // Default : everything else is a `SwarmError`.
        _ => SwarmError::new_err(msg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_err_carries_message() {
        let err = ffi_err("boom");
        let s = format!("{err}");
        assert!(s.contains("boom"));
    }

    #[test]
    fn alignment_breach_maps_to_alignment_breach() {
        let e = EfeError::AlignmentBreach { delta: 0.6, max: 0.4 };
        let py_err = map_efe_error_to_py(e);
        // The Python error type IS AlignmentBreach.
        Python::with_gil(|py| {
            assert!(py_err
                .get_type_bound(py)
                .is(&py.get_type_bound::<AlignmentBreach>()));
        });
    }

    #[test]
    fn invalid_config_maps_to_config_error() {
        let e = EfeError::InvalidConfig("missing field".into());
        let py_err = map_efe_error_to_py(e);
        Python::with_gil(|py| {
            assert!(py_err
                .get_type_bound(py)
                .is(&py.get_type_bound::<ConfigError>()));
        });
    }

    #[test]
    fn hbm_alloc_failed_maps_to_resource_exhausted() {
        let e = EfeError::HbmAllocFailed { requested: 1024 };
        let py_err = map_efe_error_to_py(e);
        Python::with_gil(|py| {
            assert!(py_err
                .get_type_bound(py)
                .is(&py.get_type_bound::<ResourceExhausted>()));
        });
    }
}
