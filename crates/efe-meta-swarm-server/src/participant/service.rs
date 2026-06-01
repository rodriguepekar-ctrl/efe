//! `MetaParticipantService` impl.
//!
//! PR #6 returned UNIMPLEMENTED for every RPC except `Health` and
//! `Capabilities`.  PR #9 wires :
//!   * `Step`  — `LocalParticipant::step` via `spawn_blocking`.
//!   * `Reset` — `LocalParticipant::reset` via `spawn_blocking`.
//!   * `State` — `LocalParticipant::current_state` via `spawn_blocking`.
//!
//! `StepStream` remains `UNIMPLEMENTED` (PR #21).
//!
//! # Sync ↔ async bridge
//!
//! Phase-1's `step_with_pull` / `current_state` / `reset` / `set_state`
//! are sync (they hold a `Mutex<Tensor>` internally).  Running them
//! directly on a tokio worker would block the executor ; we wrap each
//! call in [`tokio::task::spawn_blocking`] (mirror of PR #8's
//! controller-side bridge).
//!
//! # Cancellation
//!
//! When the client disconnects, tonic drops the handler future.  The
//! in-flight `spawn_blocking` continues — Phase-1 is non-interruptible
//! — but its result is discarded.  Phase-1 calls are short (< 100 µs
//! for dim ≤ 1024) so the worst-case orphan duration is bounded.

use std::time::Instant;

use efe_meta_swarm::prelude::Tensor;
use efe_meta_swarm_proto::meta_swarm::{
    CapabilitiesRequest, CapabilitiesResponse, HealthRequest, HealthResponse,
    ResetParticipantRequest, ResetParticipantResponse, StateRequest, StateResponse, StepRequest,
    StepResponse, TensorF32,
};
use tonic::{Request, Response, Status};
use tracing::{debug, instrument, warn};

use super::state::ParticipantState;
use super::validate;
use crate::client::{proto_to_tensor, tensor_to_proto};
use crate::error::meta_swarm_error_to_status;
use crate::proto::MetaParticipantService;

#[derive(Debug, Clone)]
pub struct ParticipantService {
    pub state: ParticipantState,
}

impl ParticipantService {
    #[must_use]
    pub fn new(state: ParticipantState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl MetaParticipantService for ParticipantService {
    type StepStreamStream = futures::stream::Empty<Result<StepResponse, Status>>;

    #[instrument(skip(self, req), fields(participant_id = %self.state.local.id()))]
    async fn step(&self, req: Request<StepRequest>) -> Result<Response<StepResponse>, Status> {
        use crate::observability::metrics::{FailureReason, Outcome, PayloadDirection};

        let start = Instant::now();
        let participant_id = self.state.local.id().to_string();
        let req_inner = req.into_inner();

        // PR #17 — observe inbound payload size (T48 visibility).
        let payload_in_bytes = if let Some(pulled) = req_inner.pulled_from_meta.as_ref() {
            let bytes = pulled.data.len().saturating_mul(std::mem::size_of::<f32>());
            self.state
                .metrics
                .step()
                .observe_payload_bytes(bytes, PayloadDirection::In);
            bytes
        } else {
            0
        };

        // PR #18 Span 5 — build the participant step span.  The
        // server-side tonic interceptor (installed in bin/participant.rs)
        // has already extracted the inbound `traceparent` and set it as
        // the current tracing span's parent ; this span is therefore a
        // child of the controller's `efe.meta_swarm.iterate`.
        let step_span = crate::tracing_otel::spans::span_step(
            &participant_id,
            req_inner.step_index,
            self.state.local.dim() as u32,
            payload_in_bytes,
        );
        let _step_entered = step_span.clone();
        let _entered = _step_entered.enter();

        // 1. Validate at the boundary (defence in depth ; Phase-1 also checks).
        let (pulled_proto, step_index) =
            match validate::validate_step(&req_inner, &self.state.local) {
                Ok(p) => p,
                Err(e) => {
                    // PR #17 — record dim_mismatch / validation failure.
                    let elapsed = start.elapsed();
                    self.state.metrics.step().observe_duration(
                        elapsed,
                        &participant_id,
                        Outcome::Error,
                    );
                    self.state.metrics.step().record_total(
                        &participant_id,
                        Outcome::Error,
                        FailureReason::DimMismatch,
                    );
                    return Err(Status::from(e));
                }
            };

        // 2. Convert proto → Phase-1 Tensor (validated finite).
        let pulled_tensor = match proto_to_tensor(pulled_proto) {
            Ok(t) => t,
            Err(e) => {
                let elapsed = start.elapsed();
                self.state.metrics.step().observe_duration(
                    elapsed,
                    &participant_id,
                    Outcome::Error,
                );
                self.state.metrics.step().record_total(
                    &participant_id,
                    Outcome::Error,
                    FailureReason::DimMismatch,
                );
                return Err(Status::failed_precondition(e.to_string()));
            }
        };

        // 3. Run the sync Phase-1 step on the blocking pool.
        let local = self.state.local.clone();
        let iter_snap = self.state.iteration.load();
        let gamma = iter_snap.coupling_gamma;
        let alpha = iter_snap.alpha_default;
        let dim_u32 = u32::try_from(self.state.local.dim()).unwrap_or(u32::MAX);
        let kappa = f64::from(self.state.local.kappa());

        // PR #18 Span 6 — child of `efe.meta_swarm.step` capturing the
        // CPU-bound Phase-1 work executed on the blocking pool.  We
        // construct the span on the caller thread (so it inherits the
        // current `step` span as parent) and record close-time
        // attributes after `join.await` returns.
        let step_local_span = crate::tracing_otel::spans::span_step_local(
            gamma, alpha, dim_u32, kappa,
        );
        let step_local_started = std::time::Instant::now();
        let _step_local_entered = step_local_span.clone();
        let _entered_local = _step_local_entered.enter();
        let join = tokio::task::spawn_blocking(move || {
            local.step(&pulled_tensor, gamma, alpha, step_index)
        });

        let new_state = match join.await {
            Ok(Ok(t)) => {
                crate::tracing_otel::spans::record_step_local_close(
                    &step_local_span,
                    step_local_started.elapsed().as_micros(),
                    "ok",
                    None,
                );
                t
            }
            Ok(Err(e)) => {
                crate::tracing_otel::spans::record_step_local_close(
                    &step_local_span,
                    step_local_started.elapsed().as_micros(),
                    "error",
                    Some("internal"),
                );
                let elapsed = start.elapsed();
                self.state.metrics.step().observe_duration(
                    elapsed,
                    &participant_id,
                    Outcome::Error,
                );
                self.state.metrics.step().record_total(
                    &participant_id,
                    Outcome::Error,
                    FailureReason::Internal,
                );
                return Err(meta_swarm_error_to_status(&e));
            }
            Err(e) => {
                warn!(error = ?e, "step panicked");
                crate::tracing_otel::spans::record_step_local_close(
                    &step_local_span,
                    step_local_started.elapsed().as_micros(),
                    "error",
                    Some("panicked"),
                );
                let elapsed = start.elapsed();
                self.state.metrics.step().observe_duration(
                    elapsed,
                    &participant_id,
                    Outcome::Error,
                );
                self.state.metrics.step().record_total(
                    &participant_id,
                    Outcome::Error,
                    FailureReason::Internal,
                );
                return Err(Status::internal(format!("step panicked: {e}")));
            }
        };
        // Drop the local span guard so subsequent code (close of `step`)
        // is no longer attributed to `step.local`.
        drop(_entered_local);

        // 4. Optional residual self-report : ‖pulled − new_state‖₂ in f64.
        let local_residual = compute_residual(pulled_proto, &new_state);
        let elapsed = start.elapsed();
        let elapsed_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);

        debug!(step_index, elapsed_us, local_residual, "step completed",);

        // PR #17 — observe outbound payload size + success metrics.
        let out_proto = tensor_to_proto(&new_state);
        let payload_out_bytes = out_proto
            .data
            .len()
            .saturating_mul(std::mem::size_of::<f32>());
        self.state
            .metrics
            .step()
            .observe_payload_bytes(payload_out_bytes, PayloadDirection::Out);
        self.state
            .metrics
            .step()
            .observe_duration(elapsed, &participant_id, Outcome::Ok);
        self.state
            .metrics
            .step()
            .record_total(&participant_id, Outcome::Ok, FailureReason::Other);

        // PR #18 — close the step span with the success attributes.
        crate::tracing_otel::spans::record_step_close(
            &step_span,
            payload_out_bytes,
            local_residual,
            elapsed.as_micros(),
            "ok",
            None,
        );
        crate::tracing_otel::spans::record_grpc_ok(&step_span);

        Ok(Response::new(StepResponse {
            new_state: Some(out_proto),
            local_residual,
            step_us: elapsed_us,
        }))
    }

    #[instrument(skip(self, _req), fields(participant_id = %self.state.local.id()))]
    async fn step_stream(
        &self,
        _req: Request<tonic::Streaming<StepRequest>>,
    ) -> Result<Response<Self::StepStreamStream>, Status> {
        Err(Status::unimplemented(
            "MetaParticipantService::StepStream — wired in PR #21",
        ))
    }

    #[instrument(skip(self, req), fields(participant_id = %self.state.local.id()))]
    async fn reset(
        &self,
        req: Request<ResetParticipantRequest>,
    ) -> Result<Response<ResetParticipantResponse>, Status> {
        let req_inner = req.into_inner();

        // 1. Validate.
        let initial_opt =
            validate::validate_reset(&req_inner, &self.state.local).map_err(Status::from)?;

        // 2. Convert proto → Phase-1 Tensor (if any).
        let initial_tensor = match initial_opt {
            Some(p) => {
                Some(proto_to_tensor(p).map_err(|e| Status::failed_precondition(e.to_string()))?)
            }
            None => None,
        };

        // 3. Run reset on the blocking pool.
        let local = self.state.local.clone();
        let join = tokio::task::spawn_blocking(move || local.reset(initial_tensor.as_ref()));

        let reset_gen = match join.await {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(meta_swarm_error_to_status(&e)),
            Err(e) => return Err(Status::internal(format!("reset panicked: {e}"))),
        };

        Ok(Response::new(ResetParticipantResponse { reset_gen }))
    }

    #[instrument(skip(self, _req), fields(participant_id = %self.state.local.id()))]
    async fn state(&self, _req: Request<StateRequest>) -> Result<Response<StateResponse>, Status> {
        let local = self.state.local.clone();
        let join = tokio::task::spawn_blocking(move || local.current_state());

        let tensor = match join.await {
            Ok(Ok(t)) => t,
            Ok(Err(e)) => return Err(meta_swarm_error_to_status(&e)),
            Err(e) => return Err(Status::internal(format!("state panicked: {e}"))),
        };

        let step_index_last = u32::try_from(self.state.local.step_index_last()).unwrap_or(u32::MAX);

        Ok(Response::new(StateResponse {
            current: Some(tensor_to_proto(&tensor)),
            step_index_last,
        }))
    }

    #[instrument(skip(self, _req))]
    async fn capabilities(
        &self,
        _req: Request<CapabilitiesRequest>,
    ) -> Result<Response<CapabilitiesResponse>, Status> {
        Ok(Response::new(CapabilitiesResponse {
            descriptor: Some(self.state.descriptor()),
        }))
    }

    #[instrument(skip(self, _req))]
    async fn health(
        &self,
        _req: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        Ok(Response::new(self.state.health.snapshot()))
    }
}

/// `‖pulled − new_state‖₂` in f64.  Returns 0 if the dims disagree
/// (caller has already validated, so this is purely defensive).
///
/// The f64 accumulator avoids intermediate overflow for `dim ≥ 10_000`.
fn compute_residual(pulled: &TensorF32, new_state: &Tensor) -> f64 {
    if pulled.data.len() != new_state.dim() {
        return 0.0;
    }
    let new_slice = new_state.as_slice();
    let sum_sq: f64 = pulled
        .data
        .iter()
        .zip(new_slice.iter())
        .map(|(a, b)| {
            let d = f64::from(*a) - f64::from(*b);
            d * d
        })
        .sum();
    sum_sq.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::participant::ParticipantConfig;

    fn test_state() -> ParticipantState {
        let config = ParticipantConfig {
            id: "test-participant".into(),
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            worker_threads: 1,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            // PR #16 — TLS knobs off by default in service unit tests.
            tls_mutual: false,
            tls_min_protocol: crate::tls::ProtocolVersion::Tls13,
            tls_allowed_peer_sans: Vec::new(),
            tls_peer_san: None,
        };
        ParticipantState::new_for_tests(config)
    }

    fn good_step_request() -> StepRequest {
        StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.1; 8],
            }),
            step_index: 7,
        }
    }

    #[tokio::test]
    async fn health_returns_serving() {
        let svc = ParticipantService::new(test_state());
        let resp = svc.health(Request::new(HealthRequest {})).await.unwrap();
        assert_eq!(resp.into_inner().status, 1);
    }

    #[tokio::test]
    async fn capabilities_descriptor_uses_local_participant_values() {
        let svc = ParticipantService::new(test_state());
        let resp = svc
            .capabilities(Request::new(CapabilitiesRequest {}))
            .await
            .unwrap();
        let desc = resp.into_inner().descriptor.expect("descriptor present");
        assert_eq!(desc.id, "test-participant");
        assert_eq!(desc.dim, 8);
        assert!((desc.kappa - 0.6).abs() < 1e-6);
        assert_eq!(desc.seed, 11);
        assert!(desc.features.iter().any(|f| f == "determinism"));
        assert!(desc.features.iter().any(|f| f == "step_index"));
    }

    #[tokio::test]
    async fn step_happy_path() {
        let svc = ParticipantService::new(test_state());
        let resp = svc
            .step(Request::new(good_step_request()))
            .await
            .expect("step");
        let inner = resp.into_inner();
        let new_state = inner.new_state.expect("new_state present");
        assert_eq!(new_state.dim, 8);
        assert_eq!(new_state.data.len(), 8);
        assert!(inner.local_residual >= 0.0);
        // step_us is wall-time in microseconds ; non-zero on real hardware
        // but conceivably 0 if the clock resolution is too coarse.
        // We only assert it's at most a generous bound.
        assert!(inner.step_us < 100_000_000); // 100 s ceiling
    }

    #[tokio::test]
    async fn step_rejects_missing_pulled_from_meta() {
        let svc = ParticipantService::new(test_state());
        let mut req = good_step_request();
        req.pulled_from_meta = None;
        let err = svc.step(Request::new(req)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("required"));
    }

    #[tokio::test]
    async fn step_rejects_dim_mismatch() {
        let svc = ParticipantService::new(test_state());
        let mut req = good_step_request();
        req.pulled_from_meta = Some(TensorF32 {
            dim: 7,
            data: vec![0.0; 7],
        });
        let err = svc.step(Request::new(req)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("expects 8"));
    }

    #[tokio::test]
    async fn step_rejects_nan_in_pulled() {
        let svc = ParticipantService::new(test_state());
        let mut req = good_step_request();
        req.pulled_from_meta.as_mut().unwrap().data[3] = f32::NAN;
        let err = svc.step(Request::new(req)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("not finite"));
    }

    #[tokio::test]
    async fn reset_default_bumps_reset_gen() {
        let svc = ParticipantService::new(test_state());
        let r1 = svc
            .reset(Request::new(ResetParticipantRequest::default()))
            .await
            .expect("reset 1");
        assert_eq!(r1.into_inner().reset_gen, 1);
        let r2 = svc
            .reset(Request::new(ResetParticipantRequest::default()))
            .await
            .expect("reset 2");
        assert_eq!(r2.into_inner().reset_gen, 2);
    }

    #[tokio::test]
    async fn reset_with_initial_validates_dim() {
        let svc = ParticipantService::new(test_state());
        let err = svc
            .reset(Request::new(ResetParticipantRequest {
                initial: Some(TensorF32 {
                    dim: 4,
                    data: vec![0.0; 4],
                }),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn state_returns_current_tensor() {
        let svc = ParticipantService::new(test_state());
        let resp = svc
            .state(Request::new(StateRequest {}))
            .await
            .expect("state");
        let inner = resp.into_inner();
        let current = inner.current.expect("current present");
        assert_eq!(current.dim, 8);
        assert_eq!(current.data.len(), 8);
        assert_eq!(inner.step_index_last, 0); // no step yet
    }

    #[tokio::test]
    async fn state_step_index_last_reflects_step_input() {
        let svc = ParticipantService::new(test_state());
        let mut req = good_step_request();
        req.step_index = 42;
        let _ = svc.step(Request::new(req)).await.expect("step");
        let resp = svc
            .state(Request::new(StateRequest {}))
            .await
            .expect("state");
        assert_eq!(resp.into_inner().step_index_last, 42);
    }

    #[test]
    fn step_stream_associated_type_is_empty_stream() {
        // Compile-time guard : the trait's associated type must be
        // `futures::stream::Empty<Result<StepResponse, Status>>` so
        // the handler can return `Err(Status::unimplemented(...))`
        // without yielding any value.
        //
        // The actual runtime behaviour — the handler returns
        // UNIMPLEMENTED with a "PR #21" message — is exercised over
        // real gRPC by `integration_step_smoke.rs::
        // step_stream_returns_unimplemented_over_grpc`.  Constructing
        // a `tonic::Streaming<StepRequest>` is impractical in unit
        // tests (it has no public constructor).
        let _: <ParticipantService as MetaParticipantService>::StepStreamStream =
            futures::stream::empty();
    }
}
