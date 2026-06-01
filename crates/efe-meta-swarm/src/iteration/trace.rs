//! Trace of an iteration : per-step residual and (optionally) the
//! meta-consensus and participant states.
//!
//! Trace capture is opt-in via `MetaSwarmBuilder::trace(true)` ; default is
//! `false` to avoid memory pressure in production.

use serde::{Deserialize, Serialize};

use crate::Tensor;

/// One step of the iteration trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationStep {
    /// Iteration index, zero-based.
    pub step: usize,
    /// Joint residual at this step.
    pub residual: f64,
    /// Meta consensus at this step (after projection on $C_{\text{meta}}$).
    pub meta_consensus: Tensor,
    /// Participant states after the step.
    pub participant_states: Vec<Tensor>,
}

/// Full iteration trace.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct IterationTrace {
    steps: Vec<IterationStep>,
}

impl IterationTrace {
    /// Build an empty trace.
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Record one step.
    pub fn record_step(
        &mut self,
        step: usize,
        residual: f64,
        meta_consensus: &Tensor,
        participant_states: &[Tensor],
    ) {
        self.steps.push(IterationStep {
            step,
            residual,
            meta_consensus: meta_consensus.clone(),
            participant_states: participant_states.to_vec(),
        });
    }

    /// All recorded steps.
    #[must_use]
    pub fn steps(&self) -> &[IterationStep] { &self.steps }

    /// Number of steps recorded.
    #[must_use]
    pub fn len(&self) -> usize { self.steps.len() }

    /// Whether the trace is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.steps.is_empty() }

    /// Residuals as a single `Vec` ;  useful for plotting.
    #[must_use]
    pub fn residuals(&self) -> Vec<f64> {
        self.steps.iter().map(|s| s.residual).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_introspect() {
        let mut t = IterationTrace::new();
        assert!(t.is_empty());
        let mc = Tensor::zeros(4);
        let ps = vec![Tensor::zeros(4)];
        t.record_step(0, 1.0, &mc, &ps);
        t.record_step(1, 0.5, &mc, &ps);
        assert_eq!(t.len(), 2);
        assert_eq!(t.residuals(), vec![1.0, 0.5]);
    }
}
