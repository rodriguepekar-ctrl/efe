//! PR #20 § 10.3 — Reset workload (state machine torture).

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use rand_xoshiro::Xoshiro256PlusPlus;

use super::{Workload, WorkloadOutcome};
use crate::client::ClientPool;
use crate::stats::{RequestResult, Verb};

/// Reset-heavy workload : issues `Reset` RPCs in sequence.  Refused in
/// production mode (AD40) unless `--allow-mutation` is explicitly set ;
/// the engine enforces the safety guard before dispatch.
#[derive(Debug, Clone)]
pub struct ResetWorkload;

impl ResetWorkload {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ResetWorkload {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Workload for ResetWorkload {
    async fn dispatch(
        &self,
        pool: &Arc<ClientPool>,
        _rng: &mut Xoshiro256PlusPlus,
        _request_index: u64,
    ) -> WorkloadOutcome {
        let started = Instant::now();
        let (result, target, trace_id, error_msg) = match pool.reset().await {
            Ok(resp) => (RequestResult::Ok, resp.target, resp.trace_id, None),
            Err(e) => (
                match e.status_code {
                    Some(tonic::Code::DeadlineExceeded) => RequestResult::Deadline,
                    Some(tonic::Code::Cancelled) => RequestResult::Canceled,
                    Some(tonic::Code::ResourceExhausted) => RequestResult::Throttled,
                    _ => RequestResult::Error,
                },
                e.target.unwrap_or_default(),
                e.trace_id.unwrap_or_default(),
                Some(e.message),
            ),
        };
        WorkloadOutcome {
            verb: Verb::Reset,
            result,
            duration: started.elapsed(),
            target,
            trace_id,
            payload_bytes: 0,
            error_msg,
        }
    }
}
