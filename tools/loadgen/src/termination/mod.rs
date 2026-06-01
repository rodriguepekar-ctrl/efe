//! PR #20 § 17 — termination policies.
//!
//! 5 policies stack ; the FIRST to fire wins (composition `min`).
//!
//! ```text
//! ┌──────────────┬─────────────────────────────────────────┐
//! │ Policy       │ Trigger                                 │
//! ├──────────────┼─────────────────────────────────────────┤
//! │ duration     │ wall_clock - start > duration           │
//! │ requests     │ total request count >= N                │
//! │ slo          │ first SLO breach                        │
//! │ convergence  │ residual within tol for K iterations    │
//! │ signal       │ SIGINT / SIGTERM (always active)        │
//! └──────────────┴─────────────────────────────────────────┘
//! ```

pub mod convergence;
pub mod duration;
pub mod requests;
pub mod signal;
pub mod slo;

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::TerminationSpec;
use crate::stats::RunStats;

/// Reason the run terminated.  Recorded in the SLO report + exit code.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    DurationReached,
    RequestsReached,
    SloBreach(crate::stats::SloBreach),
    Converged,
    SignalInterrupted,
    OperatorAbort,
}

impl StopReason {
    /// Map to the [`crate::error::ExitCode`] this stop implies.
    #[must_use]
    pub fn exit_code(&self) -> crate::error::ExitCode {
        use crate::error::ExitCode;
        match self {
            Self::SloBreach(_) => ExitCode::SloBreach,
            Self::SignalInterrupted | Self::OperatorAbort => ExitCode::Interrupted,
            _ => ExitCode::Success,
        }
    }
}

/// Trait for termination policies.
///
/// `evaluate` is called by the engine every ~100ms ; returns `Some(reason)`
/// to stop the run.
#[async_trait]
pub trait TerminationPolicy: Send + Sync {
    async fn evaluate(&self, start: Instant, stats: &Arc<RunStats>) -> Option<StopReason>;
    fn name(&self) -> &'static str;
}

/// Composite policy : checks every sub-policy in turn ; first hit wins.
pub struct ComposedPolicy {
    policies: Vec<Box<dyn TerminationPolicy>>,
}

impl ComposedPolicy {
    #[must_use]
    pub fn new(policies: Vec<Box<dyn TerminationPolicy>>) -> Self {
        Self { policies }
    }
}

#[async_trait]
impl TerminationPolicy for ComposedPolicy {
    async fn evaluate(&self, start: Instant, stats: &Arc<RunStats>) -> Option<StopReason> {
        for p in &self.policies {
            if let Some(r) = p.evaluate(start, stats).await {
                return Some(r);
            }
        }
        None
    }
    fn name(&self) -> &'static str { "composed" }
}

/// Build a composite policy from the loadgen config.
#[must_use]
pub fn from_spec(spec: &TerminationSpec) -> Box<dyn TerminationPolicy> {
    let mut v: Vec<Box<dyn TerminationPolicy>> = Vec::new();
    if let Some(d) = spec.duration {
        v.push(Box::new(duration::DurationPolicy::new(d)));
    }
    if let Some(n) = spec.requests {
        v.push(Box::new(requests::RequestsPolicy::new(n)));
    }
    if !spec.until_slo.is_empty() {
        v.push(Box::new(slo::SloPolicy::new(spec.until_slo.clone())));
    }
    if let Some(c) = &spec.convergence {
        v.push(Box::new(convergence::ConvergencePolicy::new(c.tolerance, c.consecutive_iterations)));
    }
    // Signal policy is always wired so SIGINT works.
    v.push(Box::new(signal::SignalPolicy::install()));
    Box::new(ComposedPolicy::new(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn composed_returns_first_hit() {
        let stats = Arc::new(RunStats::new(0));
        let policies: Vec<Box<dyn TerminationPolicy>> = vec![
            Box::new(duration::DurationPolicy::new(Duration::from_secs(0))),
            Box::new(requests::RequestsPolicy::new(100)),
        ];
        let c = ComposedPolicy::new(policies);
        let stop = c.evaluate(Instant::now(), &stats).await;
        assert!(matches!(stop, Some(StopReason::DurationReached)));
    }
}
