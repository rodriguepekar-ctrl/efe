//! `CollapseStrategy` trait — contract for M7 recovery strategies.

use async_trait::async_trait;

use crate::collapse::context::CollapseContext;
use crate::collapse::outcome::CollapseOutcome;
use crate::collapse::trigger::CollapseTrigger;
use crate::errors::Result;

/// One recovery strategy in the collapse engine's escalation ladder.
///
/// Implementations:
/// - [`crate::collapse::strategies::SoftRollbackStrategy`]
/// - [`crate::collapse::strategies::NoiseInjectionStrategy`]
/// - [`crate::collapse::strategies::CentroidProjectionStrategy`]
/// - [`crate::collapse::strategies::PartialRespawnStrategy`]
/// - [`crate::collapse::strategies::FullAbortStrategy`]
/// - [`crate::collapse::strategies::StubStrategy`] (tests)
///
/// # Contract
///
/// - `apply` must respect the wallclock deadline in
///   `ctx.deadline`. If the budget is exhausted before the strategy
///   starts, it should return an outcome with `DeadlineExceeded` in
///   warnings and `success: false`.
/// - On `success: true`, the ELS must satisfy
///   `compute_stats(&els).all_finite() == true`.
/// - On failure, the ELS may be left partially modified. The engine
///   escalates to the next strategy.
/// - `applicable_to` is cheap and cannot take a borrow of the ELS.
///
/// # Errors
///
/// `apply` returns `Err` only on *contract* violations (e.g. `ELS is
/// a stub`). Normal failures are encoded in the `Ok(outcome)` where
/// `success = false`.
#[async_trait]
pub trait CollapseStrategy: Send + Sync + std::fmt::Debug {
    /// Execute the strategy. Returns an outcome describing what was
    /// done, whether it succeeded, and diagnostics for M11.
    async fn apply(&self, ctx: &mut CollapseContext<'_>) -> Result<CollapseOutcome>;

    /// Cheap predicate: would this strategy usefully address the given
    /// trigger? Does NOT require mutable access to any state.
    fn applicable_to(&self, trigger: &CollapseTrigger) -> bool;

    /// Stable name for telemetry.
    fn name(&self) -> &'static str;
}
