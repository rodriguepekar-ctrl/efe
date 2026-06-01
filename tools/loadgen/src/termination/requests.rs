//! PR #20 § 17.2 — `--requests=N`.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use super::{StopReason, TerminationPolicy};
use crate::stats::RunStats;

pub struct RequestsPolicy {
    target: u64,
}

impl RequestsPolicy {
    #[must_use]
    pub const fn new(target: u64) -> Self { Self { target } }
}

#[async_trait]
impl TerminationPolicy for RequestsPolicy {
    async fn evaluate(&self, _start: Instant, stats: &Arc<RunStats>) -> Option<StopReason> {
        let snap = stats.snapshot();
        let total: u64 = snap.by_verb.values().map(|v| v.total).sum();
        if total >= self.target { Some(StopReason::RequestsReached) } else { None }
    }
    fn name(&self) -> &'static str { "requests" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::{RequestResult, Verb};
    use std::time::Duration;

    #[tokio::test]
    async fn fires_at_threshold() {
        let stats = Arc::new(RunStats::new(0));
        for _ in 0..10 {
            stats.record(Verb::Iterate, RequestResult::Ok, Duration::from_millis(1));
        }
        let p = RequestsPolicy::new(5);
        assert!(matches!(p.evaluate(Instant::now(), &stats).await, Some(StopReason::RequestsReached)));
    }

    #[tokio::test]
    async fn does_not_fire_below() {
        let stats = Arc::new(RunStats::new(0));
        stats.record(Verb::Iterate, RequestResult::Ok, Duration::from_millis(1));
        let p = RequestsPolicy::new(100);
        assert!(p.evaluate(Instant::now(), &stats).await.is_none());
    }
}
