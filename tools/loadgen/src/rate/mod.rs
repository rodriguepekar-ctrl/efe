//! PR #20 § 11 — rate control strategies.
//!
//! 3 controllers : open-loop, closed-loop, hybrid (default).
//!
//! ```text
//!  ┌────────────┬──────────────┬───────────────────────────────────────┐
//!  │ Mode       │ Strict QPS ? │ Use case                              │
//!  ├────────────┼──────────────┼───────────────────────────────────────┤
//!  │ open       │ ✓            │ Stress (rate stays high under satur.) │
//!  │ closed     │ ✗            │ Capacity ceiling (closed-loop)        │
//!  │ hybrid     │ ✓ within cap │ Production-realistic load (default)   │
//!  └────────────┴──────────────┴───────────────────────────────────────┘
//! ```

use std::sync::Arc;

use async_trait::async_trait;

pub use crate::config::OverflowPolicy;
use crate::config::RateSpec;

pub mod closed_loop;
pub mod hybrid;
pub mod token_bucket;

/// Common interface for rate controllers.
///
/// `acquire()` returns `true` if the caller may proceed, `false` if the
/// request slot was rejected (open-loop drop policy).  The caller is
/// expected to update its target QPS via [`set_target_qps`] each time
/// the pattern engine yields a fresh target.
///
/// **Permit accounting** : controllers that hold a concurrency cap
/// (closed / hybrid) reserve a permit on `acquire()` ; the engine MUST
/// call [`release`](Self::release) once the request completes, success
/// or failure.  Without `release()`, the cap is permanently consumed
/// after N=max_concurrency requests and `acquire()` deadlocks.
#[async_trait]
pub trait RateController: Send + Sync {
    /// Wait for the next request slot.  Returns `true` to send, `false`
    /// to skip (drop).
    async fn acquire(&self) -> bool;

    /// Release one concurrency slot back to the cap.  Default impl is
    /// a no-op (open-loop bucket doesn't hold permits).
    fn release(&self) {}

    /// Update the target QPS (called by the engine on each pattern tick).
    fn set_target_qps(&self, qps: f64);

    /// Current target QPS — useful for the live TUI.
    fn target_qps(&self) -> f64;

    /// Friendly name.
    fn name(&self) -> &'static str;
}

/// Factory : build a boxed [`RateController`] from a [`RateSpec`].
#[must_use]
pub fn from_spec(spec: &RateSpec) -> Arc<dyn RateController> {
    match spec.mode {
        crate::cli::RateMode::Open => Arc::new(token_bucket::TokenBucket::new(
            spec.max_concurrency.max(1) as f64,
            spec.overflow_policy,
        )),
        crate::cli::RateMode::Closed => {
            Arc::new(closed_loop::ClosedLoop::new(spec.max_concurrency.max(1)))
        }
        crate::cli::RateMode::Hybrid => Arc::new(hybrid::HybridController::new(
            spec.max_concurrency.max(1),
        )),
    }
}

/// Alias for callers that don't import `config`.
pub type Overflow = OverflowPolicy;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::RateMode;

    #[test]
    fn factory_returns_correct_controller() {
        let spec = RateSpec { mode: RateMode::Hybrid, ..Default::default() };
        let rc = from_spec(&spec);
        assert_eq!(rc.name(), "hybrid");
        let spec = RateSpec { mode: RateMode::Open, ..Default::default() };
        let rc = from_spec(&spec);
        assert_eq!(rc.name(), "open");
        let spec = RateSpec { mode: RateMode::Closed, ..Default::default() };
        let rc = from_spec(&spec);
        assert_eq!(rc.name(), "closed");
    }
}
