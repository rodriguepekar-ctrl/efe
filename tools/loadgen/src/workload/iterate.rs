//! PR #20 § 10.1 — Iterate-heavy workload.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use rand::Rng;
use rand_xoshiro::Xoshiro256PlusPlus;

use super::{Workload, WorkloadOutcome};
use crate::client::{ClientPool, ProtoIterateInput};
use crate::stats::{RequestResult, Verb};

/// Generates an `IterateRequest` per call.  Tensor values derived
/// deterministically from the request index + RNG (AD24).
#[derive(Debug, Clone)]
pub struct IterateWorkload {
    dim: usize,
}

impl IterateWorkload {
    #[must_use]
    pub const fn new(dim: usize) -> Self {
        Self { dim }
    }

    /// Build the proto-ready iterate input deterministically from the
    /// request index + RNG.  Exposed for the replay engine.
    pub fn build_input(&self, rng: &mut Xoshiro256PlusPlus, request_index: u64) -> ProtoIterateInput {
        let mut data = Vec::with_capacity(self.dim);
        for _ in 0..self.dim {
            // Values in [-1.0, 1.0] — typical contractive iterate input.
            data.push(rng.gen_range(-1.0..=1.0));
        }
        ProtoIterateInput { initial: data, request_index }
    }
}

#[async_trait]
impl Workload for IterateWorkload {
    async fn dispatch(
        &self,
        pool: &Arc<ClientPool>,
        rng: &mut Xoshiro256PlusPlus,
        request_index: u64,
    ) -> WorkloadOutcome {
        let input = self.build_input(rng, request_index);
        let payload_bytes = (input.initial.len() * 8) as u32;
        let started = Instant::now();
        let (result, target, trace_id, error_msg) = match pool.iterate(input).await {
            Ok(resp) => (RequestResult::Ok, resp.target, resp.trace_id, None),
            Err(e) => {
                let cls = classify_status(&e);
                let target = e.target.unwrap_or_default();
                let trace_id = e.trace_id.unwrap_or_default();
                (cls, target, trace_id, Some(e.message))
            }
        };
        WorkloadOutcome {
            verb: Verb::Iterate,
            result,
            duration: started.elapsed(),
            target,
            trace_id,
            payload_bytes,
            error_msg,
        }
    }
}

/// Map a gRPC status to the closed [`RequestResult`] taxonomy.
fn classify_status(e: &crate::client::ClientError) -> RequestResult {
    match e.status_code {
        Some(tonic::Code::DeadlineExceeded) => RequestResult::Deadline,
        Some(tonic::Code::Cancelled) => RequestResult::Canceled,
        Some(tonic::Code::ResourceExhausted) => RequestResult::Throttled,
        _ => RequestResult::Error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn build_input_produces_deterministic_bytes() {
        let w = IterateWorkload::new(8);
        let mut rng1 = Xoshiro256PlusPlus::seed_from_u64(42);
        let mut rng2 = Xoshiro256PlusPlus::seed_from_u64(42);
        let a = w.build_input(&mut rng1, 0);
        let b = w.build_input(&mut rng2, 0);
        assert_eq!(a.initial, b.initial);
        assert_eq!(a.initial.len(), 8);
    }

    #[test]
    fn values_in_range_minus_one_to_one() {
        let w = IterateWorkload::new(64);
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(123);
        let input = w.build_input(&mut rng, 0);
        for v in &input.initial {
            assert!((-1.0..=1.0).contains(v), "value out of range : {v}");
        }
    }
}
