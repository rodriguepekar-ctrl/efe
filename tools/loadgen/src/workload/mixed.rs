//! PR #20 § 10.4 — weighted mixed workload dispatcher.

use std::sync::Arc;

use async_trait::async_trait;
use rand_xoshiro::Xoshiro256PlusPlus;

use super::iterate::IterateWorkload;
use super::reset::ResetWorkload;
use super::state::StateWorkload;
use super::step::StepWorkload;
use super::{Workload, WorkloadOutcome};
use crate::client::ClientPool;
use crate::config::WorkloadMix;
use crate::determinism::weighted_pick;

/// Picks a verb per request weighted by [`WorkloadMix`].
pub struct MixedWorkload {
    mix: WorkloadMix,
    iterate: IterateWorkload,
    step: StepWorkload,
    state: StateWorkload,
    reset: ResetWorkload,
}

impl MixedWorkload {
    /// Build with a target tensor dimension (for Iterate / Step payloads).
    #[must_use]
    pub fn new(mix: WorkloadMix, dim: usize) -> Self {
        Self {
            mix,
            iterate: IterateWorkload::new(dim),
            step: StepWorkload::new(dim),
            state: StateWorkload::new(),
            reset: ResetWorkload::new(),
        }
    }
}

#[async_trait]
impl Workload for MixedWorkload {
    async fn dispatch(
        &self,
        pool: &Arc<ClientPool>,
        rng: &mut Xoshiro256PlusPlus,
        request_index: u64,
    ) -> WorkloadOutcome {
        let weights = [
            self.mix.iterate,
            self.mix.step,
            self.mix.state,
            self.mix.reset,
        ];
        let idx = weighted_pick(rng, &weights);
        match idx {
            0 => self.iterate.dispatch(pool, rng, request_index).await,
            1 => self.step.dispatch(pool, rng, request_index).await,
            2 => self.state.dispatch(pool, rng, request_index).await,
            3 => self.reset.dispatch(pool, rng, request_index).await,
            _ => self.iterate.dispatch(pool, rng, request_index).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_weight_zero_never_picked_deterministic() {
        use rand::SeedableRng;
        let mix = WorkloadMix {
            iterate: 1.0,
            step: 0.0,
            state: 0.0,
            reset: 0.0,
        };
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(42);
        let weights = [mix.iterate, mix.step, mix.state, mix.reset];
        // All 100 picks should be index 0.
        for _ in 0..100 {
            assert_eq!(weighted_pick(&mut rng, &weights), 0);
        }
    }
}
