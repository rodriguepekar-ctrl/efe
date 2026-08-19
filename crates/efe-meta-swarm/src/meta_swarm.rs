//! The top-level `MetaSwarm` struct — the entry point for end users.

use std::time::Duration;

use crate::{MetaSwarmError, Result, Tensor};
use crate::iteration::{CoupledIteration, ConvergenceCriterion, StepObserver, StopReason, IterationTrace};
use crate::operators::{Lift, Pull, MetaOperator};
use crate::participants::SwarmParticipant;
use crate::safety::SafetySet;
use crate::observability::{MetaSwarmTracer, NoopTracer};

/// The full meta-swarm composition — built via
/// [`crate::builder::MetaSwarmBuilder`] and executed via [`Self::iterate`].
pub struct MetaSwarm {
    pub(crate) id: String,
    pub(crate) participants: Vec<Box<dyn SwarmParticipant>>,
    pub(crate) lifts: Vec<Box<dyn Lift>>,
    pub(crate) pulls: Vec<Box<dyn Pull>>,
    pub(crate) meta_op: Box<dyn MetaOperator>,
    pub(crate) meta_safety: Box<dyn SafetySet>,
    pub(crate) alphas: Vec<f64>,
    pub(crate) gamma: f64,
    pub(crate) criterion: ConvergenceCriterion,
    pub(crate) trace_enabled: bool,
    /// Couche 3 M1.c — fail-closed mode (H1 rejects `Stub`; non-convergence errors).
    pub(crate) strict: bool,
    pub(crate) tracer: Box<dyn MetaSwarmTracer>,
}

/// Output of [`MetaSwarm::iterate`].
#[derive(Debug)]
pub struct MetaSwarmResult {
    /// Meta-swarm id.
    pub id: String,
    /// Number of iterations executed.
    pub iterations: usize,
    /// Termination reason.
    pub stop_reason: StopReason,
    /// Final joint residual.
    pub final_residual: f64,
    /// Final meta-consensus.
    pub meta_consensus: Tensor,
    /// Final participant states (in the same order as participants).
    pub participant_states: Vec<Tensor>,
    /// Optional iteration trace.
    pub trace: Option<IterationTrace>,
    /// Wall time.
    pub elapsed: Duration,
}

impl MetaSwarmResult {
    /// Did the iteration actually converge (residual ≤ tolerance)?
    #[must_use]
    pub fn converged(&self) -> bool {
        self.stop_reason == StopReason::Converged
    }

    /// **Fail-closed accessor** — the meta-consensus, but only if the iteration
    /// converged. Returns [`MetaSwarmError::ConvergenceFailed`] otherwise, so a caller
    /// can never *accidentally* consume a non-converged consensus (the honest H1/M1.c
    /// contract: "never claim consensus if not converged"). In strict mode
    /// [`MetaSwarm::iterate`] already errors before returning; this is the safe path for
    /// lenient callers that still want the guarantee at the point of use.
    ///
    /// # Errors
    ///
    /// [`MetaSwarmError::ConvergenceFailed`] if `stop_reason != Converged`.
    pub fn require_converged(&self) -> Result<&Tensor> {
        if self.converged() {
            Ok(&self.meta_consensus)
        } else {
            Err(MetaSwarmError::ConvergenceFailed {
                iters: self.iterations,
                residual: self.final_residual,
            })
        }
    }
}

impl MetaSwarm {
    /// Start a new builder.
    #[must_use]
    pub fn builder() -> crate::builder::MetaSwarmBuilder {
        crate::builder::MetaSwarmBuilder::default()
    }

    /// Identifier of this meta-swarm.
    #[must_use]
    pub fn id(&self) -> &str { &self.id }

    /// Number of participants.
    #[must_use]
    pub fn m(&self) -> usize { self.participants.len() }

    /// Reset all participants to their initial state.
    pub fn reset(&self) -> Result<()> {
        for p in &self.participants {
            p.reset()?;
        }
        Ok(())
    }

    /// Read the current state of each participant.
    pub fn current_states(&self) -> Result<Vec<Tensor>> {
        self.participants.iter().map(|p| p.current_state()).collect()
    }

    /// Run the coupled iteration to convergence (or `max_iters`).
    pub fn iterate(&self) -> Result<MetaSwarmResult> {
        self.run_engine(None)
    }

    /// Comme [`MetaSwarm::iterate`], mais en poussant **chaque pas** de consensus à un
    /// [`StepObserver`] (scellement de provenance, dashboard live, export streaming…). L'observateur
    /// ne voit que le pas **déjà calculé** : activer l'observation ne change **rien** au numérique.
    pub fn iterate_observed(&self, observer: &mut dyn StepObserver) -> Result<MetaSwarmResult> {
        self.run_engine(Some(observer))
    }

    /// Cœur partagé : construit le moteur, l'exécute (avec un observateur optionnel), mappe la sortie.
    fn run_engine(&self, observer: Option<&mut dyn StepObserver>) -> Result<MetaSwarmResult> {
        self.tracer.iteration_started(&self.id, self.criterion.max_iters);

        let engine = CoupledIteration {
            participants: &self.participants,
            lifts:        &self.lifts,
            pulls:        &self.pulls,
            meta_op:      self.meta_op.as_ref(),
            meta_safety:  self.meta_safety.as_ref(),
            alphas:       &self.alphas,
            gamma:        self.gamma,
            criterion:    self.criterion.clone(),
            trace:        self.trace_enabled,
            strict:       self.strict,
        };
        let out = engine.run(observer)?;

        // Couche 3 M1.c — strict fail-closed on non-convergence: never surface a
        // non-converged consensus as a result. (In lenient mode, the caller must inspect
        // `stop_reason` itself — see `MetaSwarmResult::require_converged`.)
        if self.strict && out.stop_reason != StopReason::Converged {
            return Err(MetaSwarmError::ConvergenceFailed {
                iters: out.iterations,
                residual: out.final_residual,
            });
        }

        self.tracer.iteration_ended(
            &self.id,
            out.iterations,
            out.final_residual,
            out.elapsed.as_micros(),
        );
        self.tracer.consensus_produced(out.meta_consensus.dim());

        Ok(MetaSwarmResult {
            id: self.id.clone(),
            iterations: out.iterations,
            stop_reason: out.stop_reason,
            final_residual: out.final_residual,
            meta_consensus: out.meta_consensus,
            participant_states: out.participant_states,
            trace: out.trace,
            elapsed: out.elapsed,
        })
    }

    /// Helper for testing : take one step.
    pub fn step_once(&self) -> Result<Tensor> {
        let engine = CoupledIteration {
            participants: &self.participants,
            lifts:        &self.lifts,
            pulls:        &self.pulls,
            meta_op:      self.meta_op.as_ref(),
            meta_safety:  self.meta_safety.as_ref(),
            alphas:       &self.alphas,
            gamma:        self.gamma,
            criterion:    self.criterion.clone(),
            trace:        false,
            strict:       self.strict,
        };
        Ok(engine.step_once()?.meta_consensus)
    }
}

impl std::fmt::Debug for MetaSwarm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaSwarm")
            .field("id", &self.id)
            .field("M", &self.participants.len())
            .field("gamma", &self.gamma)
            .field("alphas", &self.alphas)
            .field("max_iters", &self.criterion.max_iters)
            .field("tolerance", &self.criterion.tolerance)
            .field("trace_enabled", &self.trace_enabled)
            .finish_non_exhaustive()
    }
}

// Allow taking ownership of NoopTracer as the default.
impl Default for Box<dyn MetaSwarmTracer> {
    fn default() -> Self { Box::new(NoopTracer) }
}

// Stop the `MetaSwarmError` warnings about unused : a placeholder use
// in the public API surface keeps it accessible to integrators.
#[allow(dead_code)]
fn _unused() -> MetaSwarmError {
    MetaSwarmError::EmptyMetaSwarm
}
