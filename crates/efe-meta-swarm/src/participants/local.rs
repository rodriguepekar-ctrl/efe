//! `LocalParticipant` — in-process bridge between the meta-swarm and a
//! real `efe_core::SwarmController`.
//!
//! Activated by the Cargo feature `core-integration` (off by default in
//! Phase 1, on by default in Phase 2).  Without the feature, the symbol
//! is unavailable and the crate compiles in pure-`MockParticipant`
//! mode — the configuration required by the Phase-1 master-gate
//! "binary byte-identical" invariant.
//!
//! ## Phase-1 semantic note
//!
//! `LocalParticipant::step_with_pull` calls
//! `efe_core::SwarmController::step_with_pull` (see that method's
//! docstring).  Internally, the current iterate is fed to efe-core as
//! `SwarmSpec::problem_embedding` and one round is run.  This is the
//! closest faithful Φ(x) we can compute without modifying efe-core's
//! drive() loop.  The mapping
//!
//! ```text
//!   meta-swarm "current state x_t"  ↔  efe-core "problem_embedding"
//!   meta-swarm "Φ(x_t)"             ↔  efe-core "Response.output (PassThrough, max_rounds=1)"
//! ```
//!
//! is *honest but not strict* : `Φ(x)` is what efe-core's fusion
//! produces in one round when seeded with `x`, *not* a pure mathematical
//! iteration of an idealised swarm operator.  Convergence guarantees of
//! A5 § 4 carry over as long as the resulting `Φ` is non-expansive on
//! the operational domain — verified empirically at certification time.

use std::sync::{Arc, Mutex};

use efe_core::{SwarmController, SwarmSpec};

use crate::{MetaSwarmError, Result, Tensor};
use crate::participants::{NonExpansiveCertificate, SwarmParticipant};
use crate::safety::SafetySet;

/// In-process participant wrapping an `efe_core::SwarmController`.
///
/// Bridges the synchronous [`SwarmParticipant`] contract used by the
/// meta-swarm (called from `rayon::par_iter`) and the asynchronous
/// `SwarmController::step_with_pull` API.  Internal state is guarded by
/// a `std::sync::Mutex` ;  the async controller call is bridged via the
/// injected `tokio::runtime::Handle`.
///
/// ## Thread safety
///
/// `LocalParticipant: Send + Sync`.  The state `Mutex` is held only
/// briefly around reads and writes — never across the controller's
/// `await` point — so contention is minimal and deadlock-free under the
/// `MetaSwarm`'s synchronous-barrier iteration model.
///
/// ## Panics
///
/// `step_with_pull` calls `runtime.block_on(...)`.  This **must not** be
/// invoked from inside a Tokio task ;  doing so will panic at runtime
/// with the message *"Cannot start a runtime from within a runtime"*.
/// The `MetaSwarm`'s `rayon::par_iter` worker threads are not Tokio
/// tasks, so calls from `MetaSwarm::iterate` are safe.  Callers that
/// embed `LocalParticipant` in an async server (Phase 2 gRPC) should
/// spawn the iteration on a dedicated blocking thread pool (e.g.
/// `tokio::task::spawn_blocking`).
pub struct LocalParticipant {
    id: String,
    dim: usize,
    controller: Arc<SwarmController>,
    spec: SwarmSpec,
    tenant_id: String,
    safety: Box<dyn SafetySet>,
    /// Plain `std::sync::Mutex` — the state is read/written exclusively
    /// from synchronous code paths (rayon workers).  Lock acquisition
    /// never crosses an `await` point ;  poisoning is handled by
    /// `unwrap_or_else(|e| e.into_inner())` recovery.
    state: Mutex<Tensor>,
    /// Tokio runtime handle used only for the async controller call.
    runtime: tokio::runtime::Handle,
    /// Forward-compat — Phase-3 tenant feature gating.  Stored but unused.
    tenant_feature_gate: Option<crate::TenantId>,
}

impl LocalParticipant {
    /// Build a `LocalParticipant` from a configured controller and spec.
    ///
    /// # Errors
    ///
    /// Returns [`MetaSwarmError::DimensionMismatch`] if the safety set's
    /// dimension doesn't match the spec's latent dimension.
    pub fn new(
        id: impl Into<String>,
        controller: Arc<SwarmController>,
        spec: SwarmSpec,
        tenant_id: impl Into<String>,
        safety: Box<dyn SafetySet>,
        runtime: tokio::runtime::Handle,
    ) -> Result<Self> {
        let dim = spec.dim_latent.get() as usize;
        if safety.dim() != dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: dim,
                actual:   safety.dim(),
            });
        }
        Ok(Self {
            id: id.into(),
            dim,
            controller,
            spec,
            tenant_id: tenant_id.into(),
            safety,
            state: Mutex::new(Tensor::zeros(dim)),
            runtime,
            tenant_feature_gate: None,
        })
    }

    /// Phase-3 forward-compat — bind to a tenant.  Stored only ;  the
    /// Phase-1 iteration loop does not consult this field.
    #[must_use]
    pub fn with_tenant_feature_gate(mut self, tenant: crate::TenantId) -> Self {
        self.tenant_feature_gate = Some(tenant);
        self
    }

    /// Replace the initial latent state.
    ///
    /// # Errors
    ///
    /// * [`MetaSwarmError::DimensionMismatch`] if dim mismatch.
    /// * [`MetaSwarmError::Internal`] if the state contains
    ///   non-finite values (efe-core's [`SwarmSpec::validate`] would
    ///   reject these later — we surface the failure here for an
    ///   earlier and clearer error).
    pub fn set_state(&self, x: Tensor) -> Result<()> {
        if x.dim() != self.dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: self.dim,
                actual:   x.dim(),
            });
        }
        if !x.as_slice().iter().all(|v| v.is_finite()) {
            return Err(MetaSwarmError::Internal(
                "LocalParticipant::set_state: tensor contains NaN or Inf".into(),
            ));
        }
        let mut s = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *s = x;
        Ok(())
    }

    /// Box the participant for storage in a `MetaSwarm`.
    #[must_use]
    pub fn boxed(self) -> Box<dyn SwarmParticipant> {
        Box::new(self)
    }
}

impl std::fmt::Debug for LocalParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalParticipant")
            .field("id", &self.id)
            .field("dim", &self.dim)
            .field("tenant_id", &self.tenant_id)
            .field("safety", &self.safety.name())
            .finish_non_exhaustive()
    }
}

impl SwarmParticipant for LocalParticipant {
    fn id(&self) -> &str { &self.id }
    fn dim(&self) -> usize { self.dim }

    fn current_state(&self) -> Result<Tensor> {
        let s = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(s.clone())
    }

    fn step_with_pull(
        &self,
        pull: &Tensor,
        gamma: f64,
        alpha: f64,
    ) -> Result<Tensor> {
        if pull.dim() != self.dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: self.dim,
                actual:   pull.dim(),
            });
        }
        if alpha + gamma >= 1.0 || alpha < 0.0 || gamma < 0.0
            || !alpha.is_finite() || !gamma.is_finite()
        {
            return Err(MetaSwarmError::InvalidCoupling { alpha, gamma });
        }
        if !pull.as_slice().iter().all(|v| v.is_finite()) {
            return Err(MetaSwarmError::Internal(
                "LocalParticipant::step_with_pull: pull contains NaN or Inf".into(),
            ));
        }
        // 1. Snapshot the current state (brief lock).
        let current = {
            let s = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            s.clone()
        };
        // 2. Cross the async boundary into efe-core's step_with_pull.
        //    The lock from step 1 is already released — no contention with
        //    the future controller call.
        let new_state_vec = self
            .runtime
            .block_on(self.controller.step_with_pull(
                current.as_slice(),
                pull.as_slice(),
                alpha,
                gamma,
                &self.spec,
                &self.tenant_id,
            ))
            .map_err(into_meta_error)?;
        let raw = Tensor::from_vec(new_state_vec);
        // 3. Apply local safety projection.
        let projected = self.safety.project(&raw)?;
        // 4. Write back atomically.
        {
            let mut s = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *s = projected.clone();
        }
        Ok(projected)
    }

    fn verify_non_expansive(&self) -> Result<NonExpansiveCertificate> {
        // Phase 1 : stub.  Phase 3 will consult an `efe-prove`
        // certificate keyed on the controller's bundle hash.
        Ok(NonExpansiveCertificate::Stub)
    }

    fn reset(&self) -> Result<()> {
        let mut s = self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *s = Tensor::zeros(self.dim);
        Ok(())
    }
}

#[allow(clippy::needless_pass_by_value)] // signature matches map_err's FnOnce(E) -> F
fn into_meta_error(e: efe_core::EfeError) -> MetaSwarmError {
    MetaSwarmError::Internal(format!("efe-core: {e}"))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time `Send + Sync` assertion.
    #[test]
    fn local_participant_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LocalParticipant>();
    }

    /// Compile-time check : the type system accepts the public API.
    #[test]
    fn local_participant_api_surface_compiles() {
        // Verifies the API can be referenced without ambiguity.
        // We do not exercise step_with_pull here because that requires
        // a full SwarmController + 5 strategy implementations.  That
        // exercise lives in tests/integration_local_participant.rs.
        fn _expects_boxed_participant(_p: Box<dyn SwarmParticipant>) {}
    }

    /// Validate that `into_meta_error` produces a Send + Sync error.
    #[test]
    fn into_meta_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MetaSwarmError>();
        let e = efe_core::EfeError::InvalidConfig("test".into());
        let m = into_meta_error(e);
        assert!(matches!(m, MetaSwarmError::Internal(_)));
    }
}
