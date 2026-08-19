//! Swarm participants : the entities that the meta-swarm coordinates.
//!
//! Each participant implements the [`SwarmParticipant`] trait, which is
//! the contract the meta-swarm relies on.  The Phase-1 scaffold ships :
//!
//! * [`MockParticipant`] — a deterministic, controllable participant
//!   useful for tests, benchmarks, and the early demos.  It can model a
//!   swarm whose internal operator is the identity, a contraction, or a
//!   user-supplied closure.
//!
//! The real adapter `LocalParticipant<S: efe_core::Swarm>` is **deferred
//! to the workspace integration step** (Phase 1, week 3) because it
//! requires the actual `efe-core` API ;  the abstraction here is
//! deliberately wide enough that no refactor will be needed when we
//! integrate.

pub mod mock;
pub mod certificate;
/// Couche 3 M1.c — the empirical Lipschitz canary (pure; not feature-gated so
/// `MockParticipant` and unit tests can exercise it with a synthetic `Φ`).
pub mod lipschitz;
#[cfg(feature = "core-integration")]
pub mod local;

pub use mock::MockParticipant;
pub use certificate::NonExpansiveCertificate;
pub use lipschitz::{estimate_lipschitz, LipschitzProbe, LipschitzReport};

#[cfg(feature = "core-integration")]
pub use local::LocalParticipant;

use crate::{Result, Tensor};

/// A swarm participating in a meta-swarm.
///
/// The trait abstracts over local in-process swarms and (eventually)
/// remote gRPC-served swarms.  Phase 1 ships only the in-process kind.
///
/// Implementations must satisfy :
///
/// * **Thread safety** — concurrent reads of state, sequential writes :
///   if internal state must be mutated, use interior mutability.
/// * **Determinism modulo seed** — `step_with_pull` must be deterministic
///   given the same input ; this is required for the bit-identical
///   reproducibility of meta-swarm certificates in Phase 3+.
pub trait SwarmParticipant: Send + Sync {
    /// Unique identifier within the parent meta-swarm.
    fn id(&self) -> &str;

    /// Dimension of this participant's latent space.
    fn dim(&self) -> usize;

    /// Read the current latent state.  Cheap operation.
    fn current_state(&self) -> Result<Tensor>;

    /// Apply one coupled iteration step :
    ///
    /// $$ x_{t+1} = P_{C_m}\!\Big( (1 - \alpha - \gamma) x_t + \alpha\, \Phi_m(x_t) + \gamma\, \mathrm{pull} \Big). $$
    ///
    /// Implementations are responsible for the entire computation,
    /// including projection onto their local safety set $C_m$.  The
    /// returned tensor is the new state.
    fn step_with_pull(
        &self,
        pull: &Tensor,
        gamma: f64,
        alpha: f64,
    ) -> Result<Tensor>;

    /// Empirical check of hypothesis H1 (non-expansiveness of the
    /// internal operator $\Phi_m$).
    ///
    /// In Phase 1 we return a [`NonExpansiveCertificate::Stub`] ;
    /// Phase 3 will consult certificates issued by `efe-prove`.
    fn verify_non_expansive(&self) -> Result<NonExpansiveCertificate>;

    /// Reset the participant's state to a fresh initial value
    /// (typically zero).
    fn reset(&self) -> Result<()>;
}
