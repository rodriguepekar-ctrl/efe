//! Process-wide default `ComputeBackend` for legacy façades.
//!
//! ## Why a singleton?
//!
//! The Sprint 5 cross-module refactor migrates a dozen helper functions
//! such as [`els::metrics::cosine_similarity`] (signature
//! `fn(a: &[f32], b: &[f32]) -> f32`) so that they delegate to
//! [`ComputeBackend`] instead of running their own arithmetic. Those
//! helpers have **no place to receive a backend handle** without
//! breaking ~30 external call sites that take their address verbatim.
//!
//! Rather than thread an `Arc<dyn ComputeBackend>` through every public
//! API, we expose a **single global instance** initialised lazily on
//! first access to a `CpuBackend`. New modules (typed as
//! `Arc<dyn ComputeBackend>` in their constructor) keep using
//! dependency injection — only the legacy façades reach for the global.
//!
//! ## Thread safety
//!
//! Backed by [`std::sync::OnceLock`], so the first call wins on race
//! and every subsequent caller sees the same `Arc` clone.
//!
//! ## Test override
//!
//! Tests may install a different backend via [`set_global_for_test`]
//! before any façade has called [`global_backend`]. Once `OnceLock` is
//! locked, the override is **silently ignored** — this is intentional
//! to keep production semantics deterministic. Tests that need a
//! specific backend should run with `serial_test::serial` to avoid
//! interleaving.

use std::sync::Arc;
use std::sync::OnceLock;

use crate::compute::backend::ComputeBackend;
use crate::compute::cpu::CpuBackend;

static GLOBAL: OnceLock<Arc<dyn ComputeBackend>> = OnceLock::new();

/// Returns the process-wide default `ComputeBackend`.
///
/// On first call, initialises a fresh [`CpuBackend`]. Subsequent calls
/// return a cheap clone of the same `Arc`.
///
/// Use this in legacy `&[f32] → f32` façades that cannot be migrated
/// to constructor injection without breaking their public signature.
pub fn global_backend() -> Arc<dyn ComputeBackend> {
    GLOBAL
        .get_or_init(|| Arc::new(CpuBackend::new()) as Arc<dyn ComputeBackend>)
        .clone()
}

/// Test-only hook to install a custom backend.
///
/// Returns `Ok(())` on the first successful set, `Err(GlobalAlreadySet)`
/// if the global was already initialised. Production code does not use this.
#[cfg(test)]
pub fn set_global_for_test(
    backend: Arc<dyn ComputeBackend>,
) -> std::result::Result<(), GlobalAlreadySet> {
    GLOBAL.set(backend).map_err(|_| GlobalAlreadySet)
}

/// Marker error for [`set_global_for_test`].
#[cfg(test)]
#[derive(Debug, thiserror::Error)]
#[error("global compute backend already initialised")]
pub struct GlobalAlreadySet;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compute::Tensor;

    #[test]
    fn global_backend_returns_cpu_by_default() {
        let b = global_backend();
        assert_eq!(b.name(), "cpu");
    }

    #[test]
    fn global_backend_can_perform_a_dot() {
        let b = global_backend();
        let a = Tensor::<f32>::from_slice(&[1.0, 2.0, 3.0], [3].into()).unwrap();
        let c = b.dot(&a, &a).unwrap();
        // 1·1 + 2·2 + 3·3 = 14
        assert!((c - 14.0).abs() < 1e-5);
    }

    #[test]
    fn global_backend_repeated_calls_share_arc() {
        let b1 = global_backend();
        let b2 = global_backend();
        assert!(Arc::ptr_eq(&b1, &b2));
    }
}
