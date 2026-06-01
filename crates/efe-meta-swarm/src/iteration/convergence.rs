//! Stop criteria for the coupled iteration.

use serde::{Deserialize, Serialize};

/// Configuration of the stop criteria.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceCriterion {
    /// Maximum number of iterations.
    pub max_iters: usize,
    /// Convergence tolerance on the joint residual.
    pub tolerance: f64,
}

impl Default for ConvergenceCriterion {
    fn default() -> Self {
        Self { max_iters: 1_000, tolerance: 1e-4 }
    }
}

/// Reason an iteration loop terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    /// Converged : joint residual is below `tolerance`.
    Converged,
    /// Hit `max_iters` without converging.
    MaxItersHit,
    /// Stopped early due to a fatal error.
    Error,
}
