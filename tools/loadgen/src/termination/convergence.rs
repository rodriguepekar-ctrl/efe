//! PR #20 § 17.4 — convergence-based termination.
//!
//! Tracks the p95 latency over a sliding window ; if the metric stays
//! within `tolerance` for `consecutive_iterations` consecutive 1-s
//! samples, the run is deemed "converged" and stops.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;

use super::{StopReason, TerminationPolicy};
use crate::stats::{RunStats, Verb};

pub struct ConvergencePolicy {
    tolerance: f64,
    required_consecutive: u32,
    state: Mutex<ConvergenceState>,
}

#[derive(Default)]
struct ConvergenceState {
    last_p95: f64,
    consecutive: u32,
}

impl ConvergencePolicy {
    #[must_use]
    pub fn new(tolerance: f64, required_consecutive: u32) -> Self {
        Self {
            tolerance: tolerance.max(0.0),
            required_consecutive,
            state: Mutex::new(ConvergenceState::default()),
        }
    }
}

#[async_trait]
impl TerminationPolicy for ConvergencePolicy {
    async fn evaluate(&self, _start: Instant, stats: &Arc<RunStats>) -> Option<StopReason> {
        let snap = stats.snapshot();
        let v = snap.by_verb.get(&Verb::Iterate)?;
        if v.total < 20 {
            return None;
        }
        let p95 = v.p95_s;
        let mut s = self.state.lock();
        if (p95 - s.last_p95).abs() <= self.tolerance {
            s.consecutive += 1;
        } else {
            s.consecutive = 0;
        }
        s.last_p95 = p95;
        if s.consecutive >= self.required_consecutive {
            Some(StopReason::Converged)
        } else {
            None
        }
    }
    fn name(&self) -> &'static str { "convergence" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fires_after_stable_window() {
        let stats = Arc::new(RunStats::new(0));
        for _ in 0..100 {
            stats.record(Verb::Iterate, crate::stats::RequestResult::Ok, std::time::Duration::from_millis(50));
        }
        let p = ConvergencePolicy::new(0.001, 3);
        // Three consecutive evaluations with identical p95 → converged.
        assert!(p.evaluate(Instant::now(), &stats).await.is_none());
        assert!(p.evaluate(Instant::now(), &stats).await.is_none());
        assert!(p.evaluate(Instant::now(), &stats).await.is_none());
        let r = p.evaluate(Instant::now(), &stats).await;
        assert!(matches!(r, Some(StopReason::Converged)));
    }
}
