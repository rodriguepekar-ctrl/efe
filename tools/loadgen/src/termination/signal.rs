//! PR #20 § 17.5 — SIGINT / SIGTERM handler.
//!
//! Always installed.  The handler flips an atomic flag the engine
//! polls every tick.  Ctrlc-rs is portable across Unix + Windows ;
//! signal handling on *nix is otherwise tricky to get right.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use super::{StopReason, TerminationPolicy};
use crate::stats::RunStats;

pub struct SignalPolicy {
    flag: Arc<AtomicBool>,
}

impl SignalPolicy {
    /// Install the signal handler.  Idempotent (re-installs on each call
    /// are a no-op for ctrlc-rs).
    #[must_use]
    pub fn install() -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let cb_flag = flag.clone();
        // Best-effort : ignore errors when already installed.
        let _ = ctrlc::set_handler(move || {
            cb_flag.store(true, Ordering::SeqCst);
        });
        Self { flag }
    }

    /// Test hook : flip the flag manually.
    #[cfg(test)]
    pub fn fire(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl TerminationPolicy for SignalPolicy {
    async fn evaluate(&self, _start: Instant, _stats: &Arc<RunStats>) -> Option<StopReason> {
        if self.flag.load(Ordering::SeqCst) { Some(StopReason::SignalInterrupted) } else { None }
    }
    fn name(&self) -> &'static str { "signal" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fires_after_manual_flag_set() {
        let p = SignalPolicy::install();
        let stats = Arc::new(RunStats::new(0));
        assert!(p.evaluate(Instant::now(), &stats).await.is_none());
        p.fire();
        let r = p.evaluate(Instant::now(), &stats).await;
        assert!(matches!(r, Some(StopReason::SignalInterrupted)));
    }
}
