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
    /// `‖z − z̃‖` : de combien l'**enveloppe de sûreté** a ramené l'agrégat brut `z` dans l'ensemble
    /// admissible pour produire `z̃` (`0.0` = enveloppe inactive). Scellé par la provenance v2.
    pub safety_delta: f64,
    /// Les **votes liftés** des participants (leurs contributions au consensus, en méta-espace), retenus
    /// pour l'observateur. L'engine ne les **hache pas** (il reste crypto-free) ; c'est la provenance v3
    /// (feature-gated) qui en scelle un hash de contenu par participant.
    pub votes: Vec<Tensor>,
}

/// Le **contexte** d'un pas observé, passé à [`StepObserver::on_step`]. `#[non_exhaustive]` : de futurs
/// paliers (hashes des contributions participants, etc.) pourront **ajouter des champs sans casser** les
/// implémentations d'observateur, qui ne font que *lire* ce contexte.
#[non_exhaustive]
pub struct ObservedStep<'a> {
    /// Indice du pas (0-based).
    pub step: usize,
    /// Résidu joint à ce pas.
    pub residual: f64,
    /// Méta-consensus après projection de sûreté (le vecteur z̃).
    pub meta_consensus: &'a Tensor,
    /// `‖z − z̃‖` — de combien l'enveloppe de sûreté a agi à ce pas (`0.0` = inactive).
    pub safety_delta: f64,
    /// v3 : les **votes liftés** des participants (leurs contributions), pour sceller un hash de contenu
    /// par participant. L'observateur les hache lui-même s'il le souhaite ; l'engine ne les hache pas.
    pub votes: &'a [Tensor],
}

/// Observateur **par-pas** du moteur — crypto-free, zéro dépendance. Le moteur ne connaît que ce
/// trait ; la provenance (scellement HMAC), un dashboard live, un export streaming… en sont des
/// implémentations externes. Invariant de **non-interférence** : l'observateur ne reçoit que le
/// résultat DÉJÀ calculé et ne le modifie jamais — activer un observateur ne change **rien** au
/// numérique (mêmes `meta_consensus`, `iterations`, `residual`).
pub trait StepObserver {
    /// Appelé après chaque pas, avec le [`ObservedStep`] (déjà calculé — non-interférence).
    fn on_step(&mut self, observed: &ObservedStep);
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
    /// Couche 3 M1.c — strict (fail-closed) mode: the H1 pre-check rejects unverified
    /// (`Stub`) participants. Empirical / signed certificates are checked in both modes.
    pub(crate) strict:       bool,
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

        // ‖z − z̃‖ : de combien l'enveloppe de sûreté a ramené l'agrégat brut dans l'admissible.
        let safety_delta = z.distance_squared(&z_tilde)?.sqrt();

        Ok(StepResult {
            residual,
            meta_consensus: z_tilde,
            participant_states: new_states,
            safety_delta,
            votes,
        })
    }

    /// Run the iteration to convergence (or `max_iters`).
    ///
    /// `observer` is an optional per-step [`StepObserver`] (provenance sealing, dashboard, …). It sees
    /// only the already-computed step and cannot perturb the numerics (non-interference invariant).
    pub fn run(&self, mut observer: Option<&mut dyn StepObserver>) -> Result<RunOutput> {
        let start = Instant::now();
        let mut trace = if self.trace { Some(IterationTrace::new()) } else { None };

        // Pre-validation : H1 (non-expansiveness) for each participant. Couche 3 M1.c —
        // this guard is now LIVE: a `LocalParticipant` returns a real `Empirical`
        // certificate from its Lipschitz canary, so `accepts` fails-closed when the canary
        // measured `L̂ ≥ 1 − margin`. In strict mode an unverified `Stub` is also rejected.
        for p in self.participants {
            let cert = p.verify_non_expansive()?;
            if !cert.accepts(self.strict) {
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
            // Le seam de provenance : l'observateur voit le pas DÉJÀ calculé (non-interférence).
            if let Some(o) = observer.as_deref_mut() {
                o.on_step(&ObservedStep { step, residual: r.residual, meta_consensus: &r.meta_consensus, safety_delta: r.safety_delta, votes: &r.votes });
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
