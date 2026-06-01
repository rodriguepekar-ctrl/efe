//! PR #20 § 10 — workload shape catalogue.
//!
//! 4 verbs + a weighted mixed dispatcher.  Each shape implements
//! [`Workload`] which knows how to :
//!   * build the gRPC request body for a given request index,
//!   * dispatch the actual RPC call to a [`crate::client::ClientPool`],
//!   * classify the response as [`crate::stats::RequestResult`].

pub mod iterate;
pub mod mixed;
pub mod reset;
pub mod state;
pub mod step;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rand_xoshiro::Xoshiro256PlusPlus;

use crate::client::ClientPool;
use crate::config::WorkloadMix;
use crate::stats::{RequestResult, Verb};

/// A single workload step — what one request slot will do.
#[async_trait]
pub trait Workload: Send + Sync {
    /// Send a request via the pool, returning the verb + result + duration.
    async fn dispatch(
        &self,
        pool: &Arc<ClientPool>,
        rng: &mut Xoshiro256PlusPlus,
        request_index: u64,
    ) -> WorkloadOutcome;
}

/// Verbatim outcome of a workload dispatch.  The engine consumes this
/// to update stats + metrics.
#[derive(Debug, Clone)]
pub struct WorkloadOutcome {
    pub verb: Verb,
    pub result: RequestResult,
    pub duration: Duration,
    pub target: String,
    pub trace_id: String,
    pub payload_bytes: u32,
    pub error_msg: Option<String>,
}

/// Build a boxed workload from a workload mix.  Returns a mixed
/// dispatcher even when one weight is 1.0 (handles the single-verb
/// case uniformly).
#[must_use]
pub fn from_mix(mix: &WorkloadMix, dim: usize) -> Box<dyn Workload> {
    Box::new(mixed::MixedWorkload::new(mix.clone(), dim))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_mix_returns_mixed_dispatcher() {
        let mix = WorkloadMix { iterate: 1.0, ..Default::default() };
        let _w = from_mix(&mix, 16);
        // Smoke : just verify type construction succeeds.
    }
}
