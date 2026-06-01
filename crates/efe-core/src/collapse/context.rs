//! `CollapseContext<'a>` — mutable borrow passed down to each strategy.
//!
//! Every strategy receives a single `CollapseContext` which bundles:
//! - `&mut EpistemicLatentSpace`  (target)
//! - `&Arena`                     (memory lifetime)
//! - `Arc<dyn ReplayableFusion>`  (replay access + snapshot restore)
//! - Round/trigger/verdict metadata
//! - A PRNG and a hard deadline

use std::sync::Arc;
use std::time::Instant;

use rand_pcg::Pcg64;

use crate::collapse::config::CollapseConfig;
use crate::collapse::trigger::CollapseTrigger;
use crate::consensus::verdict::ConsensusVerdict;
use crate::els::EpistemicLatentSpace;
use crate::fusion::strategy::ReplayableFusion;
use crate::memory::Arena;

/// Per-collapse context passed to each strategy's `apply()` method.
///
/// Lifetime `'a` is tied to the controller's call into
/// `CollapseEngine::handle` — no part of the context outlives that call.
pub struct CollapseContext<'a> {
    /// Mutable ELS handle. Strategies may call `split_mut`,
    /// `ensure_scratch_latent`, etc.
    pub els: &'a mut EpistemicLatentSpace,
    /// Arena backing the ELS; needed by M4 view constructors.
    pub arena: &'a Arena,
    /// Fusion backend, for access to the replay buffer.
    pub fusion: Arc<dyn ReplayableFusion>,
    /// Round index at which the collapse was triggered.
    pub round: u32,
    /// The trigger that invoked the collapse.
    pub trigger: CollapseTrigger,
    /// The verdict from M6, if available. Some triggers (`Manual`) have none.
    pub verdict: Option<ConsensusVerdict>,
    /// PRNG seeded per-call; safe to clone and reuse.
    pub rng: Pcg64,
    /// Hard wallclock deadline — strategies should respect it.
    pub deadline: Instant,
    /// Read-only config snapshot.
    pub config: &'a CollapseConfig,
}

impl CollapseContext<'_> {
    /// True if the current time is past the deadline.
    #[must_use]
    pub fn past_deadline(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Remaining budget (μs). Saturates at 0 if already past.
    #[must_use]
    pub fn remaining_budget_us(&self) -> u32 {
        let now = Instant::now();
        if now >= self.deadline {
            0
        } else {
            u32::try_from(self.deadline.duration_since(now).as_micros()).unwrap_or(u32::MAX)
        }
    }
}

impl std::fmt::Debug for CollapseContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollapseContext")
            .field("round", &self.round)
            .field("trigger", &self.trigger)
            .field("verdict", &self.verdict)
            .field("remaining_budget_us", &self.remaining_budget_us())
            .finish_non_exhaustive()
    }
}
