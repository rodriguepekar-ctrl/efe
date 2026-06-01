//! `MockParticipant` — Phase-1 testable participant.
//!
//! Models a swarm whose internal operator $\Phi_m$ is one of :
//!
//! * **Identity** — $\Phi(x) = x$.  Trivial fixed point ; useful to
//!   isolate the coupling dynamics.
//! * **Contraction** — $\Phi(x) = \kappa \cdot x$ for some $\kappa \in
//!   [0, 1)$.  Contracts to the origin in the absence of coupling.
//! * **Closure** — any user-supplied non-expansive function.
//!
//! Internal state is protected by a `Mutex` ;  `step_with_pull` takes
//! `&self` so the participant can be used from multiple rayon worker
//! threads.

use std::sync::Mutex;

use crate::{MetaSwarmError, Result, Tensor};
use super::{SwarmParticipant, NonExpansiveCertificate};

/// A test-grade participant whose internal operator and safety set are
/// configurable.
pub struct MockParticipant {
    id:        String,
    dim:       usize,
    /// Internal operator $\Phi_m : \mathcal{H}_m \to \mathcal{H}_m$.
    phi:       Box<dyn Fn(&Tensor) -> Tensor + Send + Sync>,
    /// Local safety set ;  defaults to `BoxSafety::unit_ball`.
    safety:    Box<dyn crate::safety::SafetySet>,
    /// Current latent state, behind a Mutex for interior mutability.
    state:     Mutex<Tensor>,
}

impl std::fmt::Debug for MockParticipant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockParticipant")
            .field("id", &self.id)
            .field("dim", &self.dim)
            .field("safety", &self.safety.name())
            .finish_non_exhaustive()
    }
}

impl MockParticipant {
    /// Build a participant with the identity operator and a unit-box safety
    /// set, initial state drawn from `Tensor::pseudo_random`.
    pub fn with_dim(id: impl Into<String>, dim: usize) -> Self {
        Self::with_identity(id, dim, 42)
    }

    /// As above with a custom seed for the initial state.
    pub fn with_identity(id: impl Into<String>, dim: usize, seed: u64) -> Self {
        let id_s = id.into();
        let state = Tensor::pseudo_random(dim, seed);
        Self {
            id: id_s,
            dim,
            phi: Box::new(|x: &Tensor| x.clone()),
            safety: Box::new(crate::safety::BoxSafety::symmetric(dim, 10.0)),
            state: Mutex::new(state),
        }
    }

    /// Build with a contractive operator $\Phi(x) = \kappa \cdot x$.
    pub fn with_contraction(
        id: impl Into<String>,
        dim: usize,
        kappa: f32,
        seed: u64,
    ) -> Result<Self> {
        if !(0.0..1.0).contains(&kappa) {
            return Err(MetaSwarmError::Internal(
                format!("MockParticipant::with_contraction: kappa must be in [0, 1), got {kappa}")
            ));
        }
        let state = Tensor::pseudo_random(dim, seed);
        Ok(Self {
            id: id.into(),
            dim,
            phi: Box::new(move |x: &Tensor| x.scale(kappa)),
            safety: Box::new(crate::safety::BoxSafety::symmetric(dim, 10.0)),
            state: Mutex::new(state),
        })
    }

    /// Build with a custom $\Phi$.  The caller asserts non-expansiveness.
    pub fn with_phi<F>(id: impl Into<String>, dim: usize, phi: F, seed: u64) -> Self
    where F: Fn(&Tensor) -> Tensor + Send + Sync + 'static
    {
        Self {
            id: id.into(),
            dim,
            phi: Box::new(phi),
            safety: Box::new(crate::safety::BoxSafety::symmetric(dim, 10.0)),
            state: Mutex::new(Tensor::pseudo_random(dim, seed)),
        }
    }

    /// Replace the safety set.
    #[must_use]
    pub fn with_safety(mut self, safety: Box<dyn crate::safety::SafetySet>) -> Self {
        self.safety = safety;
        self
    }

    /// Replace the initial state.  Useful for examples and tests.
    pub fn set_state(&self, x: Tensor) -> Result<()> {
        if x.dim() != self.dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: self.dim,
                actual:   x.dim(),
            });
        }
        let mut s = self.state.lock().map_err(|_| MetaSwarmError::LockPoisoned)?;
        *s = x;
        Ok(())
    }

    /// Convenience for `Box::new(self) as Box<dyn SwarmParticipant>`.
    #[must_use]
    pub fn boxed(self) -> Box<dyn SwarmParticipant>
    {
        Box::new(self)
    }
}

impl SwarmParticipant for MockParticipant {
    fn id(&self) -> &str { &self.id }
    fn dim(&self) -> usize { self.dim }

    fn current_state(&self) -> Result<Tensor> {
        let s = self.state.lock().map_err(|_| MetaSwarmError::LockPoisoned)?;
        Ok(s.clone())
    }

    fn step_with_pull(&self, pull: &Tensor, gamma: f64, alpha: f64) -> Result<Tensor> {
        if pull.dim() != self.dim {
            return Err(MetaSwarmError::DimensionMismatch {
                expected: self.dim,
                actual:   pull.dim(),
            });
        }
        if alpha + gamma >= 1.0 {
            return Err(MetaSwarmError::InvalidCoupling { alpha, gamma });
        }
        let mut s = self.state.lock().map_err(|_| MetaSwarmError::LockPoisoned)?;
        let x = s.clone();
        let phi_x = (self.phi)(&x);
        let lambda1 = (1.0 - alpha - gamma) as f32;
        let lambda2 = alpha as f32;
        let lambda3 = gamma as f32;
        let raw = Tensor::affine3(lambda1, &x, lambda2, &phi_x, lambda3, pull)?;
        let projected = self.safety.project(&raw)?;
        *s = projected.clone();
        Ok(projected)
    }

    fn verify_non_expansive(&self) -> Result<NonExpansiveCertificate> {
        // Phase 1 : always Stub.  Phase 3 will check a real certificate.
        Ok(NonExpansiveCertificate::Stub)
    }

    fn reset(&self) -> Result<()> {
        let mut s = self.state.lock().map_err(|_| MetaSwarmError::LockPoisoned)?;
        *s = Tensor::zeros(self.dim);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_participant_converges_under_zero_coupling() {
        // No coupling, no pull effect : an identity participant with relax
        // alpha simply stays at its initial state.
        let p = MockParticipant::with_identity("p", 4, 1);
        let initial = p.current_state().unwrap();
        let zero_pull = Tensor::zeros(4);
        let next = p.step_with_pull(&zero_pull, 0.0, 0.5).unwrap();
        // (1 - 0.5) x + 0.5 Φ(x) + 0 pull = x  (since Φ = id)
        assert!(initial.distance(&next).unwrap() < 1e-5);
    }

    #[test]
    fn contraction_participant_drifts_toward_zero() {
        let p = MockParticipant::with_contraction("p", 4, 0.5, 1).unwrap();
        let zero_pull = Tensor::zeros(4);
        let initial = p.current_state().unwrap();
        // Run many steps with no coupling.
        for _ in 0..20 {
            p.step_with_pull(&zero_pull, 0.0, 0.5).unwrap();
        }
        let final_state = p.current_state().unwrap();
        // ‖x_t‖ → 0 under κ=0.5 contraction without pull.
        assert!(final_state.norm() < initial.norm() * 0.5);
    }

    #[test]
    fn pull_pulls_toward_target() {
        let p = MockParticipant::with_identity("p", 4, 1);
        p.set_state(Tensor::zeros(4)).unwrap();
        let target = Tensor::from_vec(vec![1.0, 0.0, 0.0, 0.0]);
        // Apply pull with a healthy coupling.
        for _ in 0..30 {
            p.step_with_pull(&target, 0.3, 0.3).unwrap();
        }
        let final_state = p.current_state().unwrap();
        // We should have drifted noticeably toward the target.
        let dist_to_target = final_state.distance(&target).unwrap();
        assert!(dist_to_target < 1.0);
    }

    #[test]
    fn invalid_coupling_is_rejected() {
        let p = MockParticipant::with_identity("p", 4, 1);
        let pull = Tensor::zeros(4);
        match p.step_with_pull(&pull, 0.7, 0.5) {
            Err(MetaSwarmError::InvalidCoupling { .. }) => (),
            other => panic!("expected InvalidCoupling, got {other:?}"),
        }
    }

    #[test]
    fn certificate_is_stub_in_phase_1() {
        let p = MockParticipant::with_dim("p", 4);
        let cert = p.verify_non_expansive().unwrap();
        assert!(cert.is_valid());
        matches!(cert, NonExpansiveCertificate::Stub);
    }
}
