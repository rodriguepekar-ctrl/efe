//! PR #20 § 17.1 — `--duration=5m`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::{StopReason, TerminationPolicy};
use crate::stats::RunStats;

pub struct DurationPolicy {
    max: Duration,
}

impl DurationPolicy {
    #[must_use]
    pub const fn new(max: Duration) -> Self { Self { max } }
}

#[async_trait]
impl TerminationPolicy for DurationPolicy {
    async fn evaluate(&self, start: Instant, _stats: &Arc<RunStats>) -> Option<StopReason> {
        if start.elapsed() >= self.max { Some(StopReason::DurationReached) } else { None }
    }
    fn name(&self) -> &'static str { "duration" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fires_after_max() {
        let stats = Arc::new(RunStats::new(0));
        let p = DurationPolicy::new(Duration::from_millis(1));
        let start = Instant::now() - Duration::from_secs(1);
        assert!(matches!(p.evaluate(start, &stats).await, Some(StopReason::DurationReached)));
    }

    #[tokio::test]
    async fn does_not_fire_before_max() {
        let stats = Arc::new(RunStats::new(0));
        let p = DurationPolicy::new(Duration::from_secs(3600));
        assert!(p.evaluate(Instant::now(), &stats).await.is_none());
    }
}
