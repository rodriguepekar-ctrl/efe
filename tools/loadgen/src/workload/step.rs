//! PR #20 § 10.2 — direct-participant Step workload.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use rand::Rng;
use rand_xoshiro::Xoshiro256PlusPlus;

use super::{Workload, WorkloadOutcome};
use crate::client::{ClientPool, ProtoStepInput};
use crate::stats::{RequestResult, Verb};

/// Step-heavy workload : bypasses the controller, hits a participant
/// directly via the [`ClientPool::step`] wrapper.
#[derive(Debug, Clone)]
pub struct StepWorkload {
    dim: usize,
}

impl StepWorkload {
    #[must_use]
    pub const fn new(dim: usize) -> Self {
        Self { dim }
    }
}

#[async_trait]
impl Workload for StepWorkload {
    async fn dispatch(
        &self,
        pool: &Arc<ClientPool>,
        rng: &mut Xoshiro256PlusPlus,
        request_index: u64,
    ) -> WorkloadOutcome {
        let mut data = Vec::with_capacity(self.dim);
        for _ in 0..self.dim {
            data.push(rng.gen_range(-1.0..=1.0));
        }
        let payload_bytes = (data.len() * 8) as u32;
        let input = ProtoStepInput { pulled: data, step_index: request_index as u32 };
        let started = Instant::now();
        let (result, target, trace_id, error_msg) = match pool.step(input).await {
            Ok(resp) => (RequestResult::Ok, resp.target, resp.trace_id, None),
            Err(e) => {
                let cls = match e.status_code {
                    Some(tonic::Code::DeadlineExceeded) => RequestResult::Deadline,
                    Some(tonic::Code::Cancelled) => RequestResult::Canceled,
                    Some(tonic::Code::ResourceExhausted) => RequestResult::Throttled,
                    _ => RequestResult::Error,
                };
                (cls, e.target.unwrap_or_default(), e.trace_id.unwrap_or_default(), Some(e.message))
            }
        };
        WorkloadOutcome {
            verb: Verb::Step,
            result,
            duration: started.elapsed(),
            target,
            trace_id,
            payload_bytes,
            error_msg,
        }
    }
}
