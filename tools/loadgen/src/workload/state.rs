//! PR #20 § 10.3 — State-query workload.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use rand_xoshiro::Xoshiro256PlusPlus;

use super::{Workload, WorkloadOutcome};
use crate::client::ClientPool;
use crate::stats::{RequestResult, Verb};

/// State-query workload — read-only, safe for production mode.  Issues
/// `State` RPCs only ; never mutates target state (AD40).
#[derive(Debug, Clone)]
pub struct StateWorkload;

impl StateWorkload {
    #[must_use]
    pub const fn new() -> Self { Self }
}

impl Default for StateWorkload {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl Workload for StateWorkload {
    async fn dispatch(
        &self,
        pool: &Arc<ClientPool>,
        _rng: &mut Xoshiro256PlusPlus,
        _request_index: u64,
    ) -> WorkloadOutcome {
        let started = Instant::now();
        let (result, target, trace_id, error_msg, payload) = match pool.state().await {
            Ok(resp) => {
                let payload = (resp.dim * 8) as u32;
                (RequestResult::Ok, resp.target, resp.trace_id, None, payload)
            }
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
                0,
            ),
        };
        WorkloadOutcome {
            verb: Verb::State,
            result,
            duration: started.elapsed(),
            target,
            trace_id,
            payload_bytes: payload,
            error_msg,
        }
    }
}
