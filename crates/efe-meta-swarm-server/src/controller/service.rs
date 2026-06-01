//! `MetaSwarmService` impl.
//!
//! PR #6 returned UNIMPLEMENTED for every RPC except `Health` and
//! `Info`.  Real bodies arrive in :
//!   * PR #8  — `Iterate`, `StepOnce`, `Reset`     (this PR)
//!   * PR #12 — `ReloadConfig`
//!   * PR #21 — `IterateStream`
//!
//! ## PR #8 wire (`Iterate` / `StepOnce` / `Reset`)
//!
//! Each handler :
//!
//! 1. tries to acquire an inflight permit ([`InflightSemaphore`]) ;
//!    refusing past `MAX_INFLIGHT` mitigates T7 (Iterate flood `DoS`).
//! 2. creates a fresh [`CancellationToken`] + [`DropGuard`] so a
//!    client disconnect (tonic drops the future) cancels participant
//!    RPCs within the deadline window (mitigates T13, fan-out leak).
//! 3. delegates to the orchestrator in [`crate::controller::iterate`].
//!
//! The permit is held by RAII in `_permit` ; dropping it on return
//! replenishes the semaphore even on early Err (`?`-bail).
//!
//! [`InflightSemaphore`]: crate::controller::inflight::InflightSemaphore
//! [`DropGuard`]: tokio_util::sync::DropGuard

use efe_meta_swarm_proto::meta_swarm::{
    HealthRequest, HealthResponse, InfoRequest, InfoResponse, IterateChunk, IterateRequest,
    IterateResponse, ReloadConfigRequest, ReloadConfigResponse, ResetRequest, ResetResponse,
    StepOnceRequest, StepOnceResponse,
};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Response, Status};
use tracing::{debug, instrument, warn};

use super::state::ControllerState;
use crate::proto::MetaSwarmService;

/// Concrete `MetaSwarmService` implementation.  Holds the shared
/// controller state.
#[derive(Debug, Clone)]
pub struct ControllerService {
    pub state: ControllerState,
}

impl ControllerService {
    #[must_use]
    pub fn new(state: ControllerState) -> Self {
        Self { state }
    }
}

#[tonic::async_trait]
impl MetaSwarmService for ControllerService {
    type IterateStreamStream = futures::stream::Empty<Result<IterateChunk, Status>>;

    #[instrument(skip(self, req))]
    async fn iterate(
        &self,
        req: Request<IterateRequest>,
    ) -> Result<Response<IterateResponse>, Status> {
        // PR #17 — wrap the entire handler in timing + outcome bookkeeping.
        use crate::observability::metrics::{FailureReason, InflightGuard, Outcome};
        use std::time::Instant;
        use tracing::Instrument;

        let started = Instant::now();
        // PR #17 — inflight gauge ++ on entry / -- on Drop (panic-safe).
        let _inflight_g = InflightGuard::new(self.state.metrics.iterate());

        // PR #18 Span 1 — build the iterate root span with all 12
        // required attributes.  The handler body runs inside this
        // span via `Instrument` so child spans (connect_participants,
        // inner_loop) inherit it as parent.
        let req_inner_ref = req.get_ref();
        let manifest_snapshot = self.state.manifest.read().await;
        let iter_cfg = req_inner_ref.iteration.clone().unwrap_or_default();
        let iterate_span = crate::tracing_otel::spans::span_iterate(
            &manifest_snapshot.version,
            iter_cfg.max_iters,
            f64::from(iter_cfg.tolerance),
            f64::from(iter_cfg.alpha_default),
            f64::from(iter_cfg.coupling_gamma),
            iter_cfg.deterministic,
            manifest_snapshot.participants.len(),
            &format!("{:?}", self.state.config.discovery),
            self.state.tls_reloader.is_some(),
        );
        drop(manifest_snapshot);

        // T7 mitigation : reject when MAX_INFLIGHT saturated.
        let _permit = match self.state.inflight.try_acquire() {
            Some(p) => p,
            None => {
                warn!(
                    inflight = self.state.inflight.inflight(),
                    max = self.state.inflight.max(),
                    "Iterate rejected : MAX_INFLIGHT saturated",
                );
                // PR #17 — record throttle outcome.
                let elapsed = started.elapsed();
                let manifest_version = self.state.manifest.read().await.version.clone();
                self.state.metrics.iterate().observe_duration(
                    elapsed,
                    Outcome::Throttled,
                    &manifest_version,
                );
                self.state
                    .metrics
                    .iterate()
                    .record_total(Outcome::Throttled, FailureReason::Other);
                return Err(Status::resource_exhausted(format!(
                    "MAX_INFLIGHT={} reached — retry later",
                    self.state.inflight.max(),
                )));
            }
        };

        // T13 mitigation : drop_guard cancels participant RPCs if the
        // client disconnects (tonic drops this future).
        let cancel = CancellationToken::new();
        let _drop_guard = cancel.clone().drop_guard();

        debug!(
            inflight = self.state.inflight.inflight(),
            "Iterate accepted",
        );

        let manifest_version = self.state.manifest.read().await.version.clone();
        // PR #18 — run the iterate inside the span so child spans
        // inherit it as parent.
        let result = crate::controller::iterate::run_iterate(self.state.clone(), req, cancel)
            .instrument(iterate_span.clone())
            .await;
        let elapsed = started.elapsed();

        // PR #17 — record outcome + duration.
        match &result {
            Ok(resp) => {
                self.state.metrics.iterate().observe_duration(
                    elapsed,
                    Outcome::Ok,
                    &manifest_version,
                );
                self.state
                    .metrics
                    .iterate()
                    .record_total(Outcome::Ok, FailureReason::Other);
                // PR #18 — close-time attributes for the iterate span.
                // Audit-pass — map the proto `stop_reason` enum to the
                // STABILITY_CONTRACT.md closed taxonomy
                // ({converged, max_iters_reached, error, cancelled})
                // instead of the previous always-"converged" stub.
                let stop_reason = stop_reason_to_str(resp.stop_reason);
                crate::tracing_otel::spans::record_iterate_close(
                    &iterate_span,
                    resp.iterations,
                    resp.final_residual,
                    stop_reason,
                    elapsed.as_micros(),
                    "ok",
                    None,
                );
                crate::tracing_otel::spans::record_grpc_ok(&iterate_span);
            }
            Err(status) => {
                let (outcome, reason) = classify_grpc_status(status);
                self.state
                    .metrics
                    .iterate()
                    .observe_duration(elapsed, outcome, &manifest_version);
                self.state.metrics.iterate().record_total(outcome, reason);
                // PR #18 — record error on the span.
                crate::tracing_otel::spans::record_iterate_close(
                    &iterate_span,
                    0,
                    0.0,
                    "error",
                    elapsed.as_micros(),
                    outcome.as_str(),
                    Some(reason.as_str()),
                );
                crate::tracing_otel::spans::record_grpc_error(
                    &iterate_span,
                    &status.message().to_string(),
                );
            }
        }

        Ok(Response::new(result?))
    }

    #[instrument(skip(self, _req))]
    async fn iterate_stream(
        &self,
        _req: Request<IterateRequest>,
    ) -> Result<Response<Self::IterateStreamStream>, Status> {
        Err(Status::unimplemented(
            "MetaSwarmService::IterateStream — wired in PR #21",
        ))
    }

    #[instrument(skip(self, req))]
    async fn step_once(
        &self,
        req: Request<StepOnceRequest>,
    ) -> Result<Response<StepOnceResponse>, Status> {
        let _permit = self.state.inflight.try_acquire().ok_or_else(|| {
            Status::resource_exhausted(format!(
                "MAX_INFLIGHT={} reached — retry later",
                self.state.inflight.max(),
            ))
        })?;
        let cancel = CancellationToken::new();
        let _drop_guard = cancel.clone().drop_guard();

        let response =
            crate::controller::iterate::run_step_once(self.state.clone(), req, cancel).await?;
        Ok(Response::new(response))
    }

    #[instrument(skip(self, req))]
    async fn reset(&self, req: Request<ResetRequest>) -> Result<Response<ResetResponse>, Status> {
        let _permit = self.state.inflight.try_acquire().ok_or_else(|| {
            Status::resource_exhausted(format!(
                "MAX_INFLIGHT={} reached — retry later",
                self.state.inflight.max(),
            ))
        })?;
        let cancel = CancellationToken::new();
        let _drop_guard = cancel.clone().drop_guard();

        let response =
            crate::controller::iterate::run_reset(self.state.clone(), req, cancel).await?;
        Ok(Response::new(response))
    }

    #[instrument(
        skip(self, req),
        fields(
            inline_meta = ?req.get_ref().new_meta.is_some(),
            inline_participants = req.get_ref().new_participants.len(),
        ),
    )]
    async fn reload_config(
        &self,
        req: Request<ReloadConfigRequest>,
    ) -> Result<Response<ReloadConfigResponse>, Status> {
        let req_inner = req.into_inner();

        // AD11 : empty body ⇒ re-read manifest from disk ; non-empty
        // body ⇒ apply inline.  `drain_inflight` is recorded in the
        // response but PR #12 does not actually abort in-flight
        // Iterates — they ride the pre-reload snapshot to completion
        // (architecture § 5.2).  The bool stays in the response for
        // future wiring (PR #20).
        let outcome = if req_inner.new_meta.is_none() && req_inner.new_participants.is_empty() {
            crate::controller::reload::apply_reload_from_disk(&self.state)
                .await
                .map_err(Status::from)?
        } else {
            // Inline override : PR #12 risk **PR12-R5** — this path
            // bypasses the on-disk audit trail.  Operators should
            // prefer the SIGHUP-driven file-reload workflow for
            // GitOps reproducibility.  We emit a `warn!` so the
            // inline overrides are easy to grep from logs and review
            // boards can spot them.
            warn!(
                inline_participants = req_inner.new_participants.len(),
                drain_inflight = req_inner.drain_inflight,
                "ReloadConfig RPC : inline override (no disk write) — \
                 audit trail relies on operator's RPC history, not the \
                 manifest file (cf. PR12-R5)",
            );

            // Capture the current iteration block + version so the
            // inline-built manifest inherits them (the proto doesn't
            // carry these fields).
            let (current_iteration, current_version) = {
                let m = self.state.manifest.read().await;
                (m.iteration.clone(), m.version.clone())
            };
            let new_manifest = crate::controller::reload::build_inline_manifest(
                &req_inner,
                current_iteration,
                current_version,
            )
            .map_err(Status::from)?;
            crate::controller::reload::apply_reload_with_manifest(&self.state, new_manifest)
                .await
                .map_err(Status::from)?
        };

        Ok(Response::new(ReloadConfigResponse {
            reload_gen: outcome.new_reload_gen,
            applied_changes: outcome.applied_changes,
            reload_wall_us: outcome.elapsed_us,
            // PR #12 never aborts in-flight Iterates ; AD8 + § 5.2
            // (in-flight Arc snapshot keeps them coherent).  When
            // PR #20 ships drain-with-abort this flips dynamically.
            inflight_aborted: false,
        }))
    }

    #[instrument(skip(self, _req))]
    async fn health(
        &self,
        _req: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        // `HealthState.inflight` (AtomicU32) is reserved for PR #17's
        // metrics integration ; the authoritative *current* count is
        // the semaphore.  Stitch it in here so `Health` reflects what
        // the gate actually sees.
        let mut snap = self.state.health.snapshot();
        snap.inflight = u32::try_from(self.state.inflight.inflight()).unwrap_or(u32::MAX);
        Ok(Response::new(snap))
    }

    #[instrument(skip(self, _req))]
    async fn info(&self, _req: Request<InfoRequest>) -> Result<Response<InfoResponse>, Status> {
        Ok(Response::new(self.state.info_snapshot()))
    }
}

/// PR #18 helper — map the proto `IterateResponse.stop_reason` (i32)
/// to the STABLE attribute taxonomy committed in STABILITY_CONTRACT
/// § 8.3 (`meta_swarm.iterate.stop_reason`).
///
/// Closed taxonomy : `converged`, `max_iters_reached`, `error`,
/// `cancelled`, `participant_down`.  Unknown enum discriminants map
/// to `"unknown"` so the histogram bucket stays stable.
fn stop_reason_to_str(code: i32) -> &'static str {
    use efe_meta_swarm_proto::meta_swarm::StopReason as ProtoStop;
    if let Ok(s) = ProtoStop::try_from(code) {
        match s {
            ProtoStop::Converged => "converged",
            ProtoStop::MaxIters => "max_iters_reached",
            ProtoStop::SafetyFail => "error",
            // `Deadline` (controller-imposed wall clock) and
            // `ConfigReload` (operator-triggered abort) both map to
            // the `cancelled` bucket — the iterate didn't finish on
            // its own terms but wasn't an algorithmic failure either.
            ProtoStop::Deadline | ProtoStop::ConfigReload => "cancelled",
            ProtoStop::ParticipantDown => "participant_down",
            ProtoStop::Unspecified => "unknown",
        }
    } else {
        "unknown"
    }
}

/// PR #17 helper — classify a tonic `Status` into the closed
/// `(Outcome, FailureReason)` taxonomy.  Used by the Iterate handler
/// to label `iterate_duration_seconds` + `iterate_total`.
///
/// AD18 — the mapping is exhaustive over `tonic::Code` ; adding a
/// gRPC code would surface as a `_ => (Error, Other)` fallthrough
/// (caught by `iterate_total{failure_reason="other"}` growing).
fn classify_grpc_status(
    status: &Status,
) -> (
    crate::observability::metrics::Outcome,
    crate::observability::metrics::FailureReason,
) {
    use crate::observability::metrics::{FailureReason, Outcome};
    use tonic::Code;
    match status.code() {
        Code::Ok => (Outcome::Ok, FailureReason::Other),
        Code::DeadlineExceeded => (Outcome::Deadline, FailureReason::DeadlineExceeded),
        Code::Cancelled => (Outcome::Canceled, FailureReason::Cancelled),
        Code::ResourceExhausted => (Outcome::Throttled, FailureReason::Other),
        Code::InvalidArgument => (Outcome::Error, FailureReason::DimMismatch),
        Code::Unavailable => (Outcome::Error, FailureReason::ParticipantDown),
        Code::FailedPrecondition => (Outcome::Error, FailureReason::SafetyViolation),
        Code::Unauthenticated | Code::PermissionDenied => (Outcome::Error, FailureReason::Other),
        Code::Internal | Code::DataLoss | Code::Unknown => {
            (Outcome::Error, FailureReason::Internal)
        }
        _ => (Outcome::Error, FailureReason::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::controller::ControllerConfig;
    // `Seek` brought into scope so `tempfile::NamedTempFile::as_file_mut().rewind()`
    // works in the disk-rewrite reload tests below.
    use std::io::Seek as _;

    fn test_state() -> ControllerState {
        let config = ControllerConfig {
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            discovery: crate::config::controller::DiscoveryMode::StaticLocalhost,
            static_participants: vec![],
            manifest_path: "/dev/null".into(),
            max_inflight: 10,
            default_max_wall_ms: 15_000,
            worker_threads: 1,
            reflection_enabled: false,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            // PR #16 — TLS knobs off in service unit tests.
            tls_mutual: false,
            tls_min_protocol: crate::tls::ProtocolVersion::Tls13,
            tls_allowed_peer_sans: Vec::new(),
            tls_peer_san: None,
            drain_timeout: std::time::Duration::from_secs(30),
        };
        ControllerState::new_for_tests(config)
    }

    #[tokio::test]
    async fn health_returns_serving() {
        let svc = ControllerService::new(test_state());
        let resp = svc.health(Request::new(HealthRequest {})).await.unwrap();
        let h = resp.into_inner();
        assert_eq!(h.status, 1); // STATUS_SERVING
        assert_eq!(h.inflight, 0);
    }

    #[tokio::test]
    async fn health_inflight_reflects_semaphore_state() {
        // Reading the semaphore through `Health` was a discrepancy
        // before PR #8 : the `HealthState.inflight` counter is never
        // incremented by the iterate handler.  We stitch the
        // authoritative count from `InflightSemaphore` at snapshot
        // time ; this test guards the stitch.
        let state = test_state();
        let _p1 = state.inflight.try_acquire().expect("p1");
        let _p2 = state.inflight.try_acquire().expect("p2");
        let svc = ControllerService::new(state);
        let resp = svc.health(Request::new(HealthRequest {})).await.unwrap();
        assert_eq!(resp.into_inner().inflight, 2);
    }

    #[tokio::test]
    async fn health_returns_reloading_status_during_swap_window() {
        // **PR #12 AC20** — `Health` returns `RELOADING (3)` while the
        // reload write-lock window is open.  In production this
        // window is microseconds (cf. § 13.4) ; here we synthesise it
        // by calling `set_reloading()` directly (the public API used
        // by `controller::reload::apply_reload`) and querying
        // Health BEFORE the matching `set_serving()`.
        let state = test_state();
        state.health.set_reloading();
        let svc = ControllerService::new(state.clone());
        let resp = svc.health(Request::new(HealthRequest {})).await.unwrap();
        let h = resp.into_inner();
        assert_eq!(h.status, 3, "RELOADING == 3 in HealthResponse proto");

        // Flip back to SERVING (mirroring `apply_reload`'s post-swap
        // step) ; Health snaps back.
        state.health.set_serving();
        let resp = svc.health(Request::new(HealthRequest {})).await.unwrap();
        assert_eq!(
            resp.into_inner().status,
            1,
            "back to SERVING after reload window closes"
        );
    }

    #[tokio::test]
    async fn info_includes_proto_version() {
        let svc = ControllerService::new(test_state());
        let resp = svc.info(Request::new(InfoRequest {})).await.unwrap();
        let i = resp.into_inner();
        assert_eq!(i.proto_version, "v1");
        // PR #8 : info_snapshot now mirrors the manifest.  The test
        // manifest (`ControllerState::new_for_tests`) has exactly
        // one participant ("stub").
        assert_eq!(i.configured_participants.len(), 1);
        assert_eq!(i.configured_participants[0].id, "stub");
        // PR #12 : `last_reload_gen` mirrors `Health.reload_gen` ;
        // fresh state has gen 0.
        assert_eq!(i.last_reload_gen, 0);
    }

    #[tokio::test]
    async fn info_last_reload_gen_mirrors_health_after_reload() {
        // PR #12 : after a successful reload, `Info.last_reload_gen`
        // must reflect the bump (operators query Info for the
        // canonical "current generation" alongside topology).
        let state = test_state();
        // Bump reload_gen synthetically via the public API.
        let _ = state.health.bump_reload_gen(); // gen = 1
        let _ = state.health.bump_reload_gen(); // gen = 2
        let svc = ControllerService::new(state);
        let resp = svc.info(Request::new(InfoRequest {})).await.unwrap();
        let i = resp.into_inner();
        assert_eq!(i.last_reload_gen, 2);
    }

    #[tokio::test]
    async fn iterate_with_empty_request_rejects_failed_precondition() {
        // PR #8 wires Iterate ; an empty request lacks `iteration`
        // and is rejected by validate_request → FailedPrecondition.
        let svc = ControllerService::new(test_state());
        let err = svc
            .iterate(Request::new(IterateRequest::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(
            err.message().contains("iteration"),
            "unexpected error message: {}",
            err.message(),
        );
    }

    #[tokio::test]
    async fn iterate_rejects_when_inflight_saturated() {
        // T7 mitigation : when MAX_INFLIGHT is exhausted, Iterate
        // must reject with `ResourceExhausted`.  We exhaust the
        // semaphore here by hand and verify the handler rejects.
        let state = test_state();
        let mut held = Vec::new();
        while let Some(p) = state.inflight.try_acquire() {
            held.push(p);
        }
        let svc = ControllerService::new(state);
        let err = svc
            .iterate(Request::new(IterateRequest::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::ResourceExhausted);
        drop(held);
    }

    #[tokio::test]
    async fn step_once_no_participants_returns_unavailable() {
        // `test_state()` has `static_participants: vec![]`, so
        // `connect_participants` short-circuits with Discovery →
        // UNAVAILABLE.  This is the off-the-shelf failure mode when
        // the operator forgot `EFE_META_SWARM_PARTICIPANTS`.
        let svc = ControllerService::new(test_state());
        let err = svc
            .step_once(Request::new(StepOnceRequest::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unavailable, "{err:?}");
    }

    #[tokio::test]
    async fn reset_no_participants_returns_unavailable() {
        let svc = ControllerService::new(test_state());
        let err = svc
            .reset(Request::new(ResetRequest::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unavailable, "{err:?}");
    }

    // -----------------------------------------------------------------
    //  PR #12 ReloadConfig tests.  Replace the old "stub returns
    //  UNIMPLEMENTED" test with the real wire coverage.
    // -----------------------------------------------------------------

    /// Build a `ControllerState` whose `manifest_path` points at a
    /// real tempfile.  Required by the disk-reload paths below.
    fn test_state_with_disk_manifest() -> (
        ControllerState,
        tempfile::NamedTempFile,
        crate::manifest::Manifest,
    ) {
        use crate::manifest::{
            IterationManifest, Manifest, MetaSpaceManifest, ParticipantManifest, SafetyManifest,
        };
        use std::io::Write;

        let manifest = Manifest {
            version: "v0.2.0".into(),
            meta: MetaSpaceManifest {
                dim: 16,
                safety: SafetyManifest::Box { radius: 50.0 },
            },
            participants: vec![
                ParticipantManifest {
                    id: "supplier-scoring".into(),
                    dim: 8,
                    kappa: 0.6,
                    seed: 11,
                    blas_backend: "none".into(),
                },
                ParticipantManifest {
                    id: "contract-risk".into(),
                    dim: 8,
                    kappa: 0.7,
                    seed: 22,
                    blas_backend: "none".into(),
                },
            ],
            iteration: IterationManifest::default(),
        };

        let yaml = serde_yaml::to_string(&manifest).expect("serialize");
        let mut f = tempfile::NamedTempFile::new().expect("tempfile");
        f.write_all(yaml.as_bytes()).expect("write");
        f.flush().expect("flush");

        let config = crate::config::controller::ControllerConfig {
            grpc_bind: "127.0.0.1:0".parse().unwrap(),
            metrics_bind: "127.0.0.1:0".parse().unwrap(),
            discovery: crate::config::controller::DiscoveryMode::StaticLocalhost,
            static_participants: vec![],
            manifest_path: f.path().to_path_buf(),
            max_inflight: 10,
            default_max_wall_ms: 15_000,
            worker_threads: 1,
            reflection_enabled: false,
            tls_required: false,
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
            // PR #16 — TLS knobs off in service-reload integration test.
            tls_mutual: false,
            tls_min_protocol: crate::tls::ProtocolVersion::Tls13,
            tls_allowed_peer_sans: Vec::new(),
            tls_peer_san: None,
            drain_timeout: std::time::Duration::from_secs(30),
        };
        let state = ControllerState::new(config, manifest.clone());
        (state, f, manifest)
    }

    #[tokio::test]
    async fn reload_config_with_empty_request_loads_from_disk() {
        // AD11 : empty body re-reads the file.  We mutate the file's
        // alpha and assert the RPC picks it up.
        use std::io::Write;
        let (state, mut f, mut m) = test_state_with_disk_manifest();
        m.iteration.alpha_default = 0.85;
        // Rewind + rewrite the tempfile.
        let yaml = serde_yaml::to_string(&m).unwrap();
        f.as_file_mut().set_len(0).unwrap();
        f.as_file_mut().rewind().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f.flush().unwrap();

        let svc = ControllerService::new(state);
        let resp = svc
            .reload_config(Request::new(ReloadConfigRequest::default()))
            .await
            .expect("disk reload ok");
        let r = resp.into_inner();
        assert_eq!(r.reload_gen, 1, "reload_gen must bump exactly once");
        // applied_changes mentions the alpha drift.
        assert!(
            r.applied_changes
                .iter()
                .any(|s| s.contains("alpha_default")),
            "applied_changes must list the diff ; got {:?}",
            r.applied_changes,
        );
        assert!(!r.inflight_aborted, "PR #12 does not abort in-flight");
    }

    #[tokio::test]
    async fn reload_config_with_inline_meta_applies_inline() {
        // AD11 : non-empty body applies inline WITHOUT touching disk.
        use efe_meta_swarm_proto::meta_swarm::{
            meta_space_config::SafetyGeometry, BoxSafety as ProtoBoxSafety, MetaSpaceConfig,
            ParticipantDescriptor,
        };
        let (state, _f, _m) = test_state_with_disk_manifest();

        let req = ReloadConfigRequest {
            new_meta: Some(MetaSpaceConfig {
                dim: 16,
                safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius: 77.0 })),
            }),
            new_participants: vec![
                ParticipantDescriptor {
                    id: "supplier-scoring".into(),
                    dim: 8,
                    kappa: 0.6,
                    seed: 11,
                    build_sha: String::new(),
                    blas_backend: String::new(),
                    features: vec![],
                },
                ParticipantDescriptor {
                    id: "contract-risk".into(),
                    dim: 8,
                    kappa: 0.7,
                    seed: 22,
                    build_sha: String::new(),
                    blas_backend: String::new(),
                    features: vec![],
                },
            ],
            drain_inflight: false,
        };
        let svc = ControllerService::new(state.clone());
        let resp = svc
            .reload_config(Request::new(req))
            .await
            .expect("inline ok");
        let r = resp.into_inner();
        assert_eq!(r.reload_gen, 1);
        // Manifest reflects the override.
        let post = state.manifest.read().await;
        match post.meta.safety {
            crate::manifest::SafetyManifest::Box { radius } => {
                assert!((radius - 77.0).abs() < 1e-6);
            }
            _ => panic!("Box expected"),
        }
        assert!(r
            .applied_changes
            .iter()
            .any(|s| s.contains("meta.safety.radius")));
    }

    #[tokio::test]
    async fn reload_config_returns_failed_precondition_on_bad_disk_manifest() {
        // Disk manifest gets corrupted ; the RPC must return
        // FAILED_PRECONDITION (Manifest error class).  reload_gen
        // must stay at 0 because the reload was rejected before any
        // state mutation.
        use std::io::Write;
        let (state, mut f, _m) = test_state_with_disk_manifest();
        f.as_file_mut().set_len(0).unwrap();
        f.as_file_mut().rewind().unwrap();
        f.write_all(b"this :: is not valid yaml [oops").unwrap();
        f.flush().unwrap();

        let svc = ControllerService::new(state.clone());
        let err = svc
            .reload_config(Request::new(ReloadConfigRequest::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert_eq!(state.health.snapshot().reload_gen, 0);
    }

    #[tokio::test]
    async fn reload_config_rejects_dim_drift_with_failed_precondition() {
        // Inline payload mutates a participant's dim → rejected with
        // FAILED_PRECONDITION.  The running state must be untouched.
        use efe_meta_swarm_proto::meta_swarm::{
            meta_space_config::SafetyGeometry, BoxSafety as ProtoBoxSafety, MetaSpaceConfig,
            ParticipantDescriptor,
        };
        let (state, _f, _m) = test_state_with_disk_manifest();

        let req = ReloadConfigRequest {
            new_meta: Some(MetaSpaceConfig {
                dim: 16,
                safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius: 50.0 })),
            }),
            new_participants: vec![
                ParticipantDescriptor {
                    id: "supplier-scoring".into(),
                    dim: 16, // illegal drift from 8
                    kappa: 0.6,
                    seed: 11,
                    build_sha: String::new(),
                    blas_backend: String::new(),
                    features: vec![],
                },
                ParticipantDescriptor {
                    id: "contract-risk".into(),
                    dim: 8,
                    kappa: 0.7,
                    seed: 22,
                    build_sha: String::new(),
                    blas_backend: String::new(),
                    features: vec![],
                },
            ],
            drain_inflight: false,
        };
        let svc = ControllerService::new(state.clone());
        let err = svc.reload_config(Request::new(req)).await.unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("dim drift"));
        assert_eq!(state.health.snapshot().reload_gen, 0);
    }

    #[tokio::test]
    async fn iterate_stream_returns_unimplemented_with_pr21_message() {
        let svc = ControllerService::new(test_state());
        let err = svc
            .iterate_stream(Request::new(IterateRequest::default()))
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::Unimplemented);
        assert!(err.message().contains("PR #21"));
    }
}
