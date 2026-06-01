//! The coupled iteration loop.
//!
//! Implements Eq. (1*) of the A1 architecture document :
//!
//! $$ x^{(m)}_{t+1} = P_{C_m}\!\Big( (1 - \alpha_m - \gamma) x^{(m)}_t
//!     + \alpha_m \Phi_m(x^{(m)}_t) + \gamma \rho_m(P_{C_{\text{meta}}}(\Psi(\pi_1(x^{(1)}_t), \ldots))) \Big) $$
//!
//! for $m = 1, \ldots, M$.

use std::time::Instant;
use rayon::prelude::*;

use crate::{MetaSwarmError, Result, Tensor};
use crate::operators::{Lift, Pull, MetaOperator};
use crate::participants::SwarmParticipant;
use crate::safety::SafetySet;

use super::{ConvergenceCriterion, StopReason, IterationTrace};

/// Per-step output, used internally and by the public `step_once` API.
#[derive(Debug, Clone)]
pub struct StepResult {
    /// Joint residual after the step.
    pub residual: f64,
    /// Meta-consensus after projection.
    pub meta_consensus: Tensor,
    /// New participant states.
    pub participant_states: Vec<Tensor>,
}

/// The coupled iteration engine.  Holds borrowed references to all the
/// components and exposes a single `run` entry point.
pub struct CoupledIteration<'a> {
    pub(crate) participants: &'a [Box<dyn SwarmParticipant>],
    pub(crate) lifts:        &'a [Box<dyn Lift>],
    pub(crate) pulls:        &'a [Box<dyn Pull>],
    pub(crate) meta_op:      &'a dyn MetaOperator,
    pub(crate) meta_safety:  &'a dyn SafetySet,
    pub(crate) alphas:       &'a [f64],
    pub(crate) gamma:        f64,
    pub(crate) criterion:    ConvergenceCriterion,
    pub(crate) trace:        bool,
}

impl<'a> CoupledIteration<'a> {
    /// Run a single step.  Reads current states, computes the next.
    pub fn step_once(&self) -> Result<StepResult> {
        // 1+2 : votes (parallel).
        let votes: Vec<Tensor> = self.participants.par_iter()
            .zip(self.lifts.par_iter())
            .map(|(p, lift)| {
                let s = p.current_state()?;
                lift.apply(&s)
            })
            .collect::<Result<Vec<_>>>()?;

        // 3 : aggregate.
        let z = self.meta_op.aggregate(&votes)?;

        // 4 : project meta.
        let z_tilde = self.meta_safety.project(&z)?;

        // 5 : pulls (parallel).
        let pulls: Vec<Tensor> = self.pulls.par_iter()
            .map(|pull| pull.apply(&z_tilde))
            .collect::<Result<Vec<_>>>()?;

        // Snapshot current states for residual computation.
        let prev_states: Vec<Tensor> = self.participants.iter()
            .map(|p| p.current_state())
            .collect::<Result<Vec<_>>>()?;

        // 6 : intra-swarm steps (parallel barrier).
        let new_states: Vec<Tensor> = self.participants.par_iter().enumerate()
            .map(|(i, p)| {
                let alpha = self.alphas[i];
                p.step_with_pull(&pulls[i], self.gamma, alpha)
            })
            .collect::<Result<Vec<_>>>()?;

        // Residual = √Σ‖x_new - x_old‖².
        let mut sq = 0.0_f64;
        for (n, o) in new_states.iter().zip(&prev_states) {
            sq += n.distance_squared(o)?;
        }
        let residual = sq.sqrt();

        Ok(StepResult {
            residual,
            meta_consensus: z_tilde,
            participant_states: new_states,
        })
    }

    /// Run the iteration to convergence (or `max_iters`).
    pub fn run(&self) -> Result<RunOutput> {
        let start = Instant::now();
        let mut trace = if self.trace { Some(IterationTrace::new()) } else { None };

        // Pre-validation : H1 for each participant.
        for p in self.participants {
            let cert = p.verify_non_expansive()?;
            if !cert.is_valid() {
                return Err(MetaSwarmError::HypothesisCheckFailed { hypothesis: "H1" });
            }
        }

        let mut step = 0;
        let mut last: Option<StepResult> = None;
        let stop_reason;

        loop {
            if step >= self.criterion.max_iters {
                stop_reason = StopReason::MaxItersHit;
                break;
            }
            let r = self.step_once()?;
            if let Some(t) = trace.as_mut() {
                t.record_step(step, r.residual, &r.meta_consensus, &r.participant_states);
            }
            let residual = r.residual;
            last = Some(r);
            step += 1;
            if residual <= self.criterion.tolerance {
                stop_reason = StopReason::Converged;
                break;
            }
        }

        let last = last.ok_or(MetaSwarmError::Internal("no step executed".into()))?;
        let elapsed = start.elapsed();

        Ok(RunOutput {
            iterations: step,
            stop_reason,
            final_residual: last.residual,
            meta_consensus: last.meta_consensus,
            participant_states: last.participant_states,
            trace,
            elapsed,
        })
    }
}

/// Full output of an iteration run.
#[derive(Debug)]
pub struct RunOutput {
    /// Number of iterations actually executed.
    pub iterations: usize,
    /// Why the loop stopped.
    pub stop_reason: StopReason,
    /// Final joint residual.
    pub final_residual: f64,
    /// Final meta-consensus (after projection on $C_{\text{meta}}$).
    pub meta_consensus: Tensor,
    /// Final participant states.
    pub participant_states: Vec<Tensor>,
    /// Optional trace.
    pub trace: Option<IterationTrace>,
    /// Wall time.
    pub elapsed: std::time::Duration,
}
