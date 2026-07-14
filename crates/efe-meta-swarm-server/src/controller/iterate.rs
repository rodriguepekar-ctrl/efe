//! `Iterate` orchestration : the glue between the gRPC handler and
//! the Phase-1 `MetaSwarm::iterate` algorithm.
//!
//! See `docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md` § 9 for the full
//! design.

use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_meta_swarm::prelude::{
    BoxSafety, LinearLift, LinearPull, MetaSwarm, MetaSwarmResult, StopReason, SwarmParticipant,
    UniformMetaOp,
};
use efe_meta_swarm_proto::meta_swarm::{
    IterateRequest, IterateResponse, ParticipantDescriptor, ResetRequest, ResetResponse,
    StepOnceRequest, StepOnceResponse,
};
use tokio_util::sync::CancellationToken;
use tonic::{Request, Status};
use tracing::{debug, error, info, instrument, warn, Instrument as _};

use crate::client::{tensor_to_proto, RemoteParticipant};
use crate::controller::state::ControllerState;
use crate::error::{meta_swarm_error_to_status, ServerError};
use crate::manifest::Manifest;

/// Server-side cap on `max_wall_ms` regardless of client request.
/// Mitigates threat **T7** (Iterate flood `DoS`).
pub const MAX_WALL_MS_HARD_CAP: u32 = 30_000;

/// Server-side cap on `max_iters`.  Phase-1 default convergence is
/// at ~ 50 iters ; 200 is comfortably above without enabling abuse.
pub const MAX_ITERS_HARD_CAP: u32 = 200;

/// Run an `Iterate` RPC end-to-end.  Called from
/// `ControllerService::iterate` (which acquires the inflight permit
/// + sets up the `CancellationToken` before delegating here).
#[instrument(skip(state, req, cancel))]
pub async fn run_iterate(
    state: ControllerState,
    req: Request<IterateRequest>,
    cancel: CancellationToken,
) -> Result<IterateResponse, Status> {
    let start = Instant::now();
    let req_inner = req.into_inner();

    // 1. Validate against manifest.
    let manifest = state.manifest.read().await.clone();
    let validated = validate_request(&req_inner, &manifest).map_err(Status::from)?;

    let deadline_ms = validated.max_wall_ms.min(MAX_WALL_MS_HARD_CAP);

    debug!(
        deadline_ms,
        participants = manifest.participants.len(),
        meta_dim = manifest.meta.dim,
        "Iterate validated",
    );

    // 2. Connect to participants — PR #18 Span 2.
    use tracing::Instrument;
    let connect_span = crate::tracing_otel::spans::span_connect_participants(
        manifest.participants.len(),
        &format!("{:?}", state.config.discovery),
        state.tls_reloader.is_some(),
        state.config.tls_mutual,
    );
    let participants = connect_participants(&state, &manifest, &cancel)
        .instrument(connect_span)
        .await
        .map_err(Status::from)?;

    // 3. Build Phase-1 MetaSwarm wrapping remote participants.
    let meta_swarm = build_meta_swarm(&manifest, participants, &validated).map_err(Status::from)?;
    let meta_swarm = Arc::new(meta_swarm);

    // 4. Run the algorithm in spawn_blocking with cancellation +
    //    deadline race.  PR #18 Span 3 — inner_loop wraps the Phase-1
    //    algo execution.
    let inner_loop_span = crate::tracing_otel::spans::span_inner_loop(
        validated.max_iters,
        f64::from(validated.tolerance),
    );
    let inner_loop_started = std::time::Instant::now();
    let cancel_for_blocking = cancel.clone();
    let meta_for_blocking = Arc::clone(&meta_swarm);
    let join = tokio::task::spawn_blocking(move || meta_for_blocking.iterate());

    let inner_result = tokio::select! {
        biased;
        () = cancel.cancelled() => {
            warn!("Iterate cancelled by client");
            return Err(Status::cancelled("Iterate cancelled by client"));
        }
        () = tokio::time::sleep(Duration::from_millis(deadline_ms.into())) => {
            warn!(deadline_ms, "Iterate exceeded max_wall_ms");
            cancel_for_blocking.cancel();
            return Err(Status::deadline_exceeded(format!(
                "Iterate exceeded max_wall_ms = {deadline_ms}",
            )));
        }
        joined = join => match joined {
            Ok(inner) => inner,
            Err(e) => {
                error!(error = ?e, "spawn_blocking panicked");
                return Err(Status::internal(format!("iterate panicked: {e}")));
            }
        },
    };

    let result = inner_result.map_err(|e| meta_swarm_error_to_status(&e))?;

    // PR #17 audit-pass — wire inner-loop observability.  Maps the
    // Phase-1 `StopReason` to the closed `Convergence` taxonomy and
    // emits two histogram observations (iterations + final residual).
    //
    // PR #17 § 6.4 + § 6.5 — iterate_iterations + iterate_convergence_error.
    // PR #17 § 8 — safety_violations_total attribution (best-effort).
    // PR #18 § 8 — close the inner_loop span with its result attributes.
    {
        use crate::observability::metrics::{Convergence, SafetyKind};
        use efe_meta_swarm::iteration::StopReason;
        let convergence = match result.stop_reason {
            StopReason::Converged => Convergence::Converged,
            StopReason::MaxItersHit => Convergence::MaxIters,
            StopReason::Error => Convergence::Diverged,
        };
        let iters_u32 = u32::try_from(result.iterations).unwrap_or(u32::MAX);
        state
            .metrics
            .iterate()
            .observe_iterations(iters_u32, convergence);
        state
            .metrics
            .iterate()
            .observe_convergence_error(result.final_residual, convergence);

        // PR #18 Span 3 close.
        crate::tracing_otel::spans::record_inner_loop_close(
            &inner_loop_span,
            iters_u32,
            result.final_residual,
            convergence.as_str(),
            inner_loop_started.elapsed().as_micros(),
        );

        // PR #17 § 8 — when the inner loop terminated with `Error`,
        // it's most likely a safety-projection rejection (Phase-1's
        // contractive algorithm doesn't have any other failure mode).
        // We attribute the violation to the manifest's configured
        // safety variant.  participant_id is unknown at this layer
        // (Phase-3 trace surface will refine), so we use `(unknown)`.
        // Phase-2's algorithm is contractive ; this counter is
        // expected to stay at 0 in production.
        if matches!(result.stop_reason, StopReason::Error) {
            let kind = match &manifest.meta.safety {
                crate::manifest::SafetyManifest::Box { .. } => SafetyKind::Box,
                crate::manifest::SafetyManifest::Poly { .. } => SafetyKind::Poly,
                crate::manifest::SafetyManifest::Hull { .. } => SafetyKind::Hull,
            };
            state.metrics.safety().record_violation(kind, "(unknown)");

            // PR #18 Span 4 — emit a safety_check span describing
            // the rejection.  The span is short-lived (created +
            // closed in one stroke) because Phase-2 doesn't know
            // *which* inner iteration caused the violation —
            // Phase-3 will refine.
            //
            // Audit-pass : `_safety_span` must be entered (not just
            // built and dropped) so the OTel bridge's "exit" event
            // fires + the span gets its `start_time` / `end_time`
            // delta recorded.  Without `.enter()` the span never
            // becomes "active" and the bridge emits it with a
            // zero-duration window — visible in dashboards as a
            // 0-µs spike that confuses operators.
            let safety_span = crate::tracing_otel::spans::span_safety_check(
                kind.as_str(),
                "(unknown)",
                true,
            );
            {
                let _entered = safety_span.enter();
                // Phase-3 will compute the adjustment norm here ;
                // for Phase-2 we mark the span as Error so it shows
                // up in error-trace queries even without the norm.
                crate::tracing_otel::spans::record_grpc_error(
                    &safety_span,
                    "safety violation detected by inner loop (phase-2 ; participant unknown)",
                );
            }
            drop(safety_span);
        }
    }

    // Couche 3 M1.d — **fail-closed non-convergence** (architecture §4).
    // With `iteration.require_convergence` set in the manifest, a run that
    // did not converge is an ERROR, never an OK response whose non-converged
    // "consensus" a client might silently trust.  Design notes :
    //   * gRPC code = ABORTED — deliberately NOT FAILED_PRECONDITION (which
    //     `classify_grpc_status` buckets as safety_violation) and NOT
    //     INTERNAL (which conflates non-convergence with bugs).
    //   * The guard sits AFTER the metrics block so the
    //     `convergence="max_iters"` histograms keep counting.
    //   * This is a RESPONSE-layer gate.  Do NOT implement it via
    //     `MetaSwarmBuilder::strict(true)` : strict also flips the engine's
    //     H1 pre-check, and `RemoteParticipant::verify_non_expansive` still
    //     returns `Stub` (no certificate on the wire yet) — strict would
    //     brick every served Iterate.  Wire-level certificates are the
    //     Couche 3 M4 milestone.
    //   * `StepOnce` (max_iters = 1, definitionally non-converged) is exempt.
    if manifest.iteration.require_convergence && result.stop_reason != StopReason::Converged {
        warn!(
            iterations = result.iterations,
            final_residual = result.final_residual,
            stop_reason = ?result.stop_reason,
            "Iterate did NOT converge and require_convergence is set — failing closed",
        );
        return Err(Status::aborted(format!(
            "convergence failed after {} iterations (final_residual = {:e}, \
             tolerance = {:e}) — iteration.require_convergence is set : a \
             non-converged iterate is never returned as a consensus",
            result.iterations, result.final_residual, validated.tolerance,
        )));
    }

    let response = build_response(&result, start.elapsed());

    info!(
        iterations = result.iterations,
        final_residual = result.final_residual,
        stop_reason = ?result.stop_reason,
        wall_us = response.wall_us,
        "Iterate completed",
    );

    Ok(response)
}

/// Run a `StepOnce` RPC end-to-end.  Implementation : invoke
/// `Iterate` with `max_iters = 1`.
#[instrument(skip(state, _req, cancel))]
pub async fn run_step_once(
    state: ControllerState,
    _req: Request<StepOnceRequest>,
    cancel: CancellationToken,
) -> Result<StepOnceResponse, Status> {
    let manifest = state.manifest.read().await.clone();

    let participants = connect_participants(&state, &manifest, &cancel)
        .await
        .map_err(Status::from)?;
    let validated = ValidatedIterateRequest {
        max_wall_ms: MAX_WALL_MS_HARD_CAP,
        max_iters: 1,
        tolerance: 1.0e-6,
        alpha_default: manifest.iteration.alpha_default,
        coupling_gamma: manifest.iteration.coupling_gamma,
        deterministic: manifest.iteration.deterministic,
    };
    let meta =
        Arc::new(build_meta_swarm(&manifest, participants, &validated).map_err(Status::from)?);

    let m_blocking = Arc::clone(&meta);
    let join = tokio::task::spawn_blocking(move || m_blocking.iterate());

    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => {
            return Err(Status::cancelled("StepOnce cancelled"));
        }
        joined = join => match joined {
            Ok(inner) => inner.map_err(|e| meta_swarm_error_to_status(&e))?,
            Err(e) => return Err(Status::internal(format!("StepOnce panicked: {e}"))),
        },
    };

    Ok(StepOnceResponse {
        meta_consensus: Some(tensor_to_proto(&result.meta_consensus)),
        residual: result.final_residual,
    })
}

/// Run a `Reset` RPC end-to-end.  Implementation : fan-out
/// `MetaParticipantService::Reset` to every participant.
#[instrument(skip(state, _req, cancel))]
pub async fn run_reset(
    state: ControllerState,
    _req: Request<ResetRequest>,
    cancel: CancellationToken,
) -> Result<ResetResponse, Status> {
    let manifest = state.manifest.read().await.clone();
    let participants = connect_participants(&state, &manifest, &cancel)
        .await
        .map_err(Status::from)?;

    // Reset is fan-out ; participants are independent so we issue
    // calls sequentially (preserves determinism) instead of parallel.
    // Phase 3 may parallelise once we add ordering guards.
    for p in &participants {
        // Use the sync trait method on a blocking thread.
        let p = p.clone();
        let join = tokio::task::spawn_blocking(move || p.reset());
        match join.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(meta_swarm_error_to_status(&e)),
            Err(e) => return Err(Status::internal(format!("Reset panicked: {e}"))),
        }
    }

    // Controller-level `reset_gen` is not tracked in Phase 2 — each
    // participant maintains its own monotonic `reset_gen` (PR #9 via
    // `LocalParticipant::reset_gen`), surfaced through the
    // participant-side `ResetParticipantResponse`.  The controller's
    // RPC response uses `0` as a placeholder ; downstream clients
    // who need cross-participant ordering should query each
    // participant directly.  PR #12 wires SIGHUP-driven manifest
    // reload (`reload_gen` on `HealthState`) but deliberately leaves
    // `reset_gen` tracking on the controller as a Phase-3 follow-up.
    Ok(ResetResponse { reset_gen: 0 })
}

// ---------------------------------------------------------------------------
//  Helpers : validation, connection, MetaSwarm build, response build.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct ValidatedIterateRequest {
    pub(crate) max_wall_ms: u32,
    pub(crate) max_iters: u32,
    pub(crate) tolerance: f64,
    pub(crate) alpha_default: f64,
    pub(crate) coupling_gamma: f64,
    /// Forwarded for future use (Phase-1 iterate is deterministic by
    /// construction ; PR #18 will gate non-deterministic paths on this).
    #[allow(dead_code)]
    pub(crate) deterministic: bool,
}

pub(crate) fn validate_request(
    req: &IterateRequest,
    manifest: &Manifest,
) -> Result<ValidatedIterateRequest, ServerError> {
    let iter = req
        .iteration
        .as_ref()
        .ok_or_else(|| ServerError::Config("IterateRequest.iteration is required".into()))?;

    if iter.max_iters == 0 {
        return Err(ServerError::Config(
            "iteration.max_iters must be > 0".into(),
        ));
    }
    if iter.max_iters > MAX_ITERS_HARD_CAP {
        return Err(ServerError::Config(format!(
            "iteration.max_iters = {} exceeds server cap {}",
            iter.max_iters, MAX_ITERS_HARD_CAP,
        )));
    }
    // Reject non-positive AND NaN.  `x <= 0` lets NaN through (NaN
    // compares false to everything) which would propagate into
    // Phase-1's convergence math ; check NaN explicitly first.
    if iter.tolerance.is_nan() || iter.tolerance <= 0.0 {
        return Err(ServerError::Config(format!(
            "iteration.tolerance = {} must be positive (and not NaN)",
            iter.tolerance,
        )));
    }
    if iter.max_wall_ms == 0 {
        return Err(ServerError::Config(
            "iteration.max_wall_ms must be > 0".into(),
        ));
    }

    // Expected participant dim match (T8 mitigation).
    if !req.expected_participants.is_empty() {
        validate_expected_against_manifest(&req.expected_participants, manifest)?;
    }

    // Initial-states dim match (T8 defence in depth).
    if !req.initial_states.is_empty() {
        if req.initial_states.len() != manifest.participants.len() {
            return Err(ServerError::Config(format!(
                "IterateRequest.initial_states.len() = {} but manifest has {} participants",
                req.initial_states.len(),
                manifest.participants.len(),
            )));
        }
        for (i, t) in req.initial_states.iter().enumerate() {
            let expected = manifest.participants[i].dim;
            if t.dim != expected {
                return Err(ServerError::Config(format!(
                    "initial_states[{i}].dim = {} but manifest expects {expected}",
                    t.dim,
                )));
            }
            if t.dim as usize != t.data.len() {
                return Err(ServerError::Config(format!(
                    "initial_states[{i}] : dim = {} but data.len() = {}",
                    t.dim,
                    t.data.len(),
                )));
            }
        }
    }

    Ok(ValidatedIterateRequest {
        max_wall_ms: iter.max_wall_ms,
        max_iters: iter.max_iters,
        tolerance: iter.tolerance,
        alpha_default: iter.alpha_default,
        coupling_gamma: iter.coupling_gamma,
        deterministic: iter.deterministic,
    })
}

fn validate_expected_against_manifest(
    expected: &[ParticipantDescriptor],
    manifest: &Manifest,
) -> Result<(), ServerError> {
    if expected.len() != manifest.participants.len() {
        return Err(ServerError::Config(format!(
            "expected_participants.len() = {} but manifest has {}",
            expected.len(),
            manifest.participants.len(),
        )));
    }
    for (i, e) in expected.iter().enumerate() {
        let m = &manifest.participants[i];
        if e.id != m.id {
            return Err(ServerError::Config(format!(
                "expected_participants[{i}].id = {:?} but manifest has {:?}",
                e.id, m.id,
            )));
        }
        if e.dim != m.dim {
            return Err(ServerError::Config(format!(
                "expected_participants[{i}].dim = {} but manifest has {}",
                e.dim, m.dim,
            )));
        }
        // Reject NaN explicitly : a NaN client-side kappa would slip
        // through `(NaN).abs() > 1e-6 == false` and reach Phase-1's
        // `MetaSwarmBuilder` as a hidden invariant violation.
        //
        // Couche 3 M1.d : manifest kappa is an Option (absent on efe-core
        // rows).  An efe-core row advertises the sentinel kappa = 0.0 on the
        // wire (see `ParticipantState::descriptor`), so the expectation
        // check compares against `unwrap_or(0.0)` — the engine identity
        // itself is checked via the manifest `controller` block presence,
        // not via the deprecated mock constant.
        let manifest_kappa = m.kappa.unwrap_or(0.0);
        let diff = (e.kappa - manifest_kappa).abs();
        if diff.is_nan() || diff > 1.0e-6 {
            return Err(ServerError::Config(format!(
                "expected_participants[{i}].kappa = {} but manifest has {manifest_kappa} (or NaN sent)",
                e.kappa,
            )));
        }
    }
    Ok(())
}

async fn connect_participants(
    state: &ControllerState,
    manifest: &Manifest,
    cancel: &CancellationToken,
) -> Result<Vec<RemoteParticipant>, ServerError> {
    let endpoints = lookup_endpoints(state, manifest)?;

    // PR #16 — Snapshot the TLS client config once, per Iterate call.
    // The reloader's ArcSwap means this is a lock-free read ; we
    // capture an `Arc<TlsClientConfig>` here and reuse it for every
    // participant connect in this iteration.  If the cert rotates
    // mid-iteration, the next Iterate will get the updated config.
    //
    // The use of `https://` URI scheme is REQUIRED for tonic-tls ; we
    // rewrite the endpoint URL accordingly when TLS is active.
    let tls_client_snapshot = state
        .tls_reloader
        .as_ref()
        .map(|r| (*r.current_client_config()).clone());

    let mut out = Vec::with_capacity(manifest.participants.len());

    // Sequential connect preserves determinism.  Phase 3 may
    // parallelise with `join_all` once endpoint-order is canonical.
    for (i, p) in manifest.participants.iter().enumerate() {
        let endpoint = endpoints.get(i).ok_or_else(|| {
            ServerError::Discovery(format!("no endpoint resolved for participant {:?}", p.id))
        })?;

        // PR #16 — choose the URI scheme based on TLS state.  tonic
        // tolerates `http://` over a TLS-configured channel but emits
        // a runtime warning ; using `https://` is cleaner.
        let scheme_endpoint = if endpoint.starts_with("http") {
            endpoint.clone()
        } else if tls_client_snapshot.is_some() {
            format!("https://{endpoint}")
        } else {
            format!("http://{endpoint}")
        };

        // PR #16 — when TLS is active, derive the expected peer SAN
        // from the resolved endpoint URL itself : `host:port` ⇒ host.
        // This matches what cert-manager wrote into the participant's
        // `Certificate.dnsNames[]` (the chart side mirrors the K8s
        // Service FQDN there).
        //
        // We do NOT use `state.config.tls_peer_san` here — that env
        // var is a single-string fallback for *all* participants (used
        // in dev) ; for production we derive per-participant.  The
        // env-var hint is honoured only when it's set AND the derived
        // SAN is empty (defensive fallback for static-localhost).
        let participant_tls = tls_client_snapshot.as_ref().map(|base| {
            let san = extract_san_from_endpoint(&scheme_endpoint);
            let resolved_san = if san.is_empty() {
                state
                    .config
                    .tls_peer_san
                    .clone()
                    .unwrap_or_else(|| p.id.clone())
            } else {
                san
            };
            crate::tls::TlsClientConfig {
                cert_chain_pem: base.cert_chain_pem.clone(),
                key_pem: base.key_pem.clone(),
                server_ca_pem: base.server_ca_pem.clone(),
                min_protocol: base.min_protocol,
                expected_server_san: resolved_san,
            }
        });

        // PR #18 Span 2b — per-participant connect span (CLIENT
        // kind).  Wraps the actual RemoteParticipant::connect_with_tls
        // call so dashboards can see per-participant connect latency
        // and TLS expected SAN.  Audit-pass : the helper existed in
        // spans.rs but was never wired here.
        let expected_san_for_span = participant_tls.as_ref().map(|c| c.expected_server_san.as_str());
        let connect_span = crate::tracing_otel::spans::span_participant_connect(
            &p.id,
            &scheme_endpoint,
            expected_san_for_span,
        );
        let connect_started = std::time::Instant::now();

        let rp = match RemoteParticipant::connect_with_tls(
            p.id.clone(),
            p.dim as usize,
            scheme_endpoint.clone(),
            cancel.clone(),
            participant_tls,
        )
        .instrument(connect_span.clone())
        .await
        {
            Ok(rp) => {
                // PR #17 — mark participant as up on successful connect.
                state.metrics.health().mark_up(&p.id);
                connect_span.record(
                    "meta_swarm.connect.elapsed_us",
                    u64::try_from(connect_started.elapsed().as_micros()).unwrap_or(u64::MAX),
                );
                crate::tracing_otel::spans::record_grpc_ok(&connect_span);
                rp
            }
            Err(e) => {
                // PR #17 — mark participant down on connect failure.
                state.metrics.health().mark_down(&p.id);
                connect_span.record(
                    "meta_swarm.connect.elapsed_us",
                    u64::try_from(connect_started.elapsed().as_micros()).unwrap_or(u64::MAX),
                );
                crate::tracing_otel::spans::record_grpc_error(
                    &connect_span,
                    &format!("connect failed: {e}"),
                );
                drop(connect_span);
                return Err(ServerError::Discovery(format!("connect to {}: {e}", p.id)));
            }
        };
        drop(connect_span);

        out.push(rp);
    }

    Ok(out)
}

/// PR #16 helper — extract the DNS-only part of a participant
/// endpoint URL.  Used to derive the expected SAN for the TLS
/// handshake from the resolved Service FQDN.
///
/// Examples :
///
/// ```text
/// "https://controller.efe.svc.cluster.local:50080" → "controller.efe.svc.cluster.local"
/// "http://10.0.0.5:50091"                          → "10.0.0.5"
/// "supplier-scoring.efe.svc:50091"                 → "supplier-scoring.efe.svc"
/// "127.0.0.1:50091"                                → "127.0.0.1"
/// ```
///
/// Strips the scheme + path + port.  IPv6-bracket form is handled.
/// On malformed input, returns the empty string ; the caller falls
/// back to `state.config.tls_peer_san`.
fn extract_san_from_endpoint(endpoint: &str) -> String {
    let after_scheme = endpoint
        .split_once("://")
        .map_or(endpoint, |(_scheme, rest)| rest);
    let authority = after_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or(after_scheme);
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split_once(']').map(|(h, _)| h).unwrap_or(stripped)
    } else if authority.contains("::") {
        authority
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    host.to_string()
}

/// Resolve participant endpoints.  Phase 2 uses the static-localhost
/// discovery : the controller's `ControllerConfig.static_participants`
/// — already populated at startup from `EFE_META_SWARM_PARTICIPANTS`.
/// PR #11 will replace this with a `Discovery` trait + caching.
fn lookup_endpoints(
    state: &ControllerState,
    manifest: &Manifest,
) -> Result<Vec<String>, ServerError> {
    use std::collections::HashMap;

    let map: HashMap<&str, &str> = state
        .config
        .static_participants
        .iter()
        .map(|sp| (sp.id.as_str(), sp.addr.as_str()))
        .collect();

    if map.is_empty() {
        return Err(ServerError::Discovery(
            "no static_participants configured (set EFE_META_SWARM_PARTICIPANTS)".into(),
        ));
    }

    let mut endpoints = Vec::with_capacity(manifest.participants.len());
    for p in &manifest.participants {
        let ep = *map.get(p.id.as_str()).ok_or_else(|| {
            ServerError::Discovery(format!(
                "no static_participants entry for manifest participant {:?}",
                p.id,
            ))
        })?;
        endpoints.push(ep.to_string());
    }

    Ok(endpoints)
}

fn build_meta_swarm(
    manifest: &Manifest,
    participants: Vec<RemoteParticipant>,
    validated: &ValidatedIterateRequest,
) -> Result<MetaSwarm, ServerError> {
    use crate::manifest::SafetyManifest;

    let meta_dim = manifest.meta.dim as usize;
    let mut builder = MetaSwarm::builder()
        .id(format!("iter-{}", uuid_simple()))
        .meta_space_dim(meta_dim);

    // Add each participant + its lift / pull operators.
    for (i, p) in participants.into_iter().enumerate() {
        let manifest_p = &manifest.participants[i];
        let dim = manifest_p.dim as usize;
        let pid = manifest_p.id.clone();

        let lift = LinearLift::identity(dim, meta_dim)
            .map_err(|e| ServerError::Manifest(format!("LinearLift::identity for {pid}: {e}")))?;
        let pull = LinearPull::identity(meta_dim, dim)
            .map_err(|e| ServerError::Manifest(format!("LinearPull::identity for {pid}: {e}")))?;

        builder = builder
            .add_participant(Box::new(p))
            .lift_for(pid.clone(), Box::new(lift))
            .pull_for(pid, Box::new(pull));
    }

    builder = builder.meta_operator(Box::new(UniformMetaOp::new(meta_dim)));

    match &manifest.meta.safety {
        SafetyManifest::Box { radius } => {
            builder = builder.meta_safety(Box::new(BoxSafety::symmetric(meta_dim, *radius)));
        }
        SafetyManifest::Poly { .. } | SafetyManifest::Hull { .. } => {
            return Err(ServerError::Manifest(
                "Poly / Hull safety not yet wired in PR #8 ; use Box (Phase 3 follow-up)".into(),
            ));
        }
    }

    builder = builder
        .alpha_default(validated.alpha_default)
        .coupling_gamma(validated.coupling_gamma)
        .tolerance(validated.tolerance)
        .max_iters(validated.max_iters as usize)
        .trace(false);

    builder
        .build()
        .map_err(|e| ServerError::Manifest(format!("MetaSwarm build failed: {e}")))
}

fn build_response(result: &MetaSwarmResult, elapsed: Duration) -> IterateResponse {
    use efe_meta_swarm_proto::meta_swarm::StopReason as ProtoStop;

    // Phase-1 StopReason → proto StopReason mapping.
    let stop_reason = match result.stop_reason {
        StopReason::Converged => ProtoStop::Converged,
        StopReason::MaxItersHit => ProtoStop::MaxIters,
        StopReason::Error => ProtoStop::ParticipantDown, // closest semantic
    } as i32;

    IterateResponse {
        iterations: u32::try_from(result.iterations).unwrap_or(u32::MAX),
        final_residual: result.final_residual,
        meta_consensus: Some(tensor_to_proto(&result.meta_consensus)),
        final_states: result
            .participant_states
            .iter()
            .map(tensor_to_proto)
            .collect(),
        stop_reason,
        wall_us: u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
        trace_id: String::new(),
    }
}

fn uuid_simple() -> String {
    use std::time::SystemTime;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{
        IterationManifest, Manifest, MetaSpaceManifest, ParticipantManifest, SafetyManifest,
    };
    use efe_meta_swarm_proto::meta_swarm::{
        BoxSafety as ProtoBoxSafety, IterateRequest, IterationConfig, MetaSpaceConfig, TensorF32,
    };

    // ─── PR #16 — `extract_san_from_endpoint` unit tests ───────────

    #[test]
    fn pr16_extract_san_strips_scheme_and_port() {
        assert_eq!(
            extract_san_from_endpoint("https://controller.efe.svc.cluster.local:50080"),
            "controller.efe.svc.cluster.local"
        );
        assert_eq!(
            extract_san_from_endpoint("http://controller.efe.svc:50080"),
            "controller.efe.svc"
        );
    }

    #[test]
    fn pr16_extract_san_handles_no_scheme() {
        assert_eq!(
            extract_san_from_endpoint("supplier-scoring.efe.svc:50091"),
            "supplier-scoring.efe.svc"
        );
        assert_eq!(extract_san_from_endpoint("10.0.0.5:50091"), "10.0.0.5");
    }

    #[test]
    fn pr16_extract_san_handles_ipv6_bracket() {
        assert_eq!(
            extract_san_from_endpoint("https://[2001:db8::1]:50080"),
            "2001:db8::1"
        );
        assert_eq!(extract_san_from_endpoint("[::1]:50080"), "::1");
    }

    #[test]
    fn pr16_extract_san_handles_bare_ipv6() {
        // No bracket, no port — bare IPv6 (the `contains("::")` branch).
        assert_eq!(extract_san_from_endpoint("::1"), "::1");
        assert_eq!(extract_san_from_endpoint("2001:db8::1"), "2001:db8::1");
    }

    #[test]
    fn pr16_extract_san_strips_path() {
        assert_eq!(
            extract_san_from_endpoint("https://controller.efe.svc:50080/grpc"),
            "controller.efe.svc"
        );
    }

    #[test]
    fn pr16_extract_san_no_port() {
        assert_eq!(
            extract_san_from_endpoint("controller.efe.svc"),
            "controller.efe.svc"
        );
        assert_eq!(extract_san_from_endpoint("127.0.0.1"), "127.0.0.1");
    }

    fn good_manifest() -> Manifest {
        Manifest {
            version: "v0.2.0".into(),
            meta: MetaSpaceManifest {
                dim: 16,
                safety: SafetyManifest::Box { radius: 5.0 },
            },
            participants: vec![ParticipantManifest {
                id: "p-1".into(),
                dim: 8,
                kappa: Some(0.6),
                seed: Some(1),
                blas_backend: "none".into(),
                controller: None,
            }],
            iteration: IterationManifest::default(),
        }
    }

    fn good_request() -> IterateRequest {
        use efe_meta_swarm_proto::meta_swarm::meta_space_config::SafetyGeometry;
        IterateRequest {
            expected_participants: vec![],
            meta: Some(MetaSpaceConfig {
                dim: 16,
                safety_geometry: Some(SafetyGeometry::Box(ProtoBoxSafety { radius: 5.0 })),
            }),
            iteration: Some(IterationConfig {
                max_iters: 100,
                tolerance: 1.0e-4,
                alpha_default: 0.5,
                coupling_gamma: 0.2,
                deterministic: true,
                max_wall_ms: 15_000,
            }),
            initial_states: vec![],
            traceparent: String::new(),
        }
    }

    #[test]
    fn validate_request_happy_path() {
        let m = good_manifest();
        let r = good_request();
        let v = validate_request(&r, &m).expect("happy path");
        assert_eq!(v.max_iters, 100);
        assert_eq!(v.max_wall_ms, 15_000);
        assert!(v.deterministic);
    }

    #[test]
    fn validate_request_rejects_missing_iteration() {
        let m = good_manifest();
        let mut r = good_request();
        r.iteration = None;
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn validate_request_rejects_zero_max_iters() {
        let m = good_manifest();
        let mut r = good_request();
        r.iteration.as_mut().unwrap().max_iters = 0;
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn validate_request_rejects_max_iters_over_cap() {
        let m = good_manifest();
        let mut r = good_request();
        r.iteration.as_mut().unwrap().max_iters = MAX_ITERS_HARD_CAP + 1;
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref msg) if msg.contains("cap")));
    }

    #[test]
    fn validate_request_rejects_non_positive_tolerance() {
        let m = good_manifest();
        let mut r = good_request();
        r.iteration.as_mut().unwrap().tolerance = 0.0;
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));

        r.iteration.as_mut().unwrap().tolerance = -1.0;
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn validate_request_rejects_nan_tolerance() {
        // A malicious client could send NaN to slip past `x > 0` and
        // poison Phase-1's convergence math.  The validator MUST
        // reject it.
        let m = good_manifest();
        let mut r = good_request();
        r.iteration.as_mut().unwrap().tolerance = f64::NAN;
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref msg) if msg.contains("NaN")));
    }

    #[test]
    fn validate_request_rejects_nan_kappa_in_expected_participants() {
        // Symmetric NaN protection on the per-participant kappa
        // shape check : `(NaN).abs() > 1e-6` is false, so a naive
        // implementation would let NaN through.
        use efe_meta_swarm_proto::meta_swarm::ParticipantDescriptor;
        let m = good_manifest();
        let mut r = good_request();
        r.expected_participants = vec![ParticipantDescriptor {
            id: "p-1".into(),
            dim: 8,
            kappa: f32::NAN,
            seed: 1,
            build_sha: String::new(),
            blas_backend: "none".into(),
            features: vec![],
            config_hash: None,
        }];
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref msg) if msg.contains("kappa")));
    }

    #[test]
    fn validate_request_rejects_zero_max_wall_ms() {
        let m = good_manifest();
        let mut r = good_request();
        r.iteration.as_mut().unwrap().max_wall_ms = 0;
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn validate_request_rejects_dim_mismatch_initial_states() {
        let m = good_manifest();
        let mut r = good_request();
        r.initial_states = vec![TensorF32 {
            dim: 7, // manifest expects 8
            data: vec![0.0; 7],
        }];
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref msg) if msg.contains("dim")));
    }

    #[test]
    fn validate_request_rejects_initial_states_length_mismatch() {
        let m = good_manifest();
        let mut r = good_request();
        // Manifest has 1 participant ; we pass 2 initial states.
        r.initial_states = vec![
            TensorF32 {
                dim: 8,
                data: vec![0.0; 8],
            },
            TensorF32 {
                dim: 8,
                data: vec![0.0; 8],
            },
        ];
        let err = validate_request(&r, &m).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }
}
