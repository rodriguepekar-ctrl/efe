//! Participant binary entry-point.
//!
//! Boot sequence :
//!   1. Parse the env-var config (`EFE_META_SWARM_PARTICIPANT_*`).
//!   2. Load + validate the shared manifest (`EFE_META_SWARM_MANIFEST`).
//!   3. Find this binary's row by `config.id`.
//!   4. Build a [`LocalParticipant`] from that row (κ, seed, dim).
//!   5. Snapshot the manifest's `iteration` block (α, γ) for the Step
//!      handler.
//!   6. PR #16 — bootstrap the TLS subsystem when
//!      `EFE_META_SWARM_TLS_REQUIRED=true`.
//!   7. Start the Prometheus exporter + the gRPC server + the SIGTERM
//!      handler, then await shutdown.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use efe_meta_swarm_proto::participant::MetaParticipantServiceServer;
use efe_meta_swarm_server::{
    config::participant::ParticipantConfig,
    error::ServerError,
    manifest,
    observability::{metrics, shutdown, tracing as obs_tracing},
    participant::{IterationParams, LocalParticipant, ParticipantService, ParticipantState},
    tls::{reload::TlsPaths, CertWatcher, TlsMetrics, TlsReloader},
};

use ::tracing::{error, info};
use tokio::sync::watch;
use tonic::transport::{Certificate as TonicCertificate, Identity, Server, ServerTlsConfig};

fn main() -> ExitCode {
    obs_tracing::init();

    // PR #16 — install the rustls crypto provider once per process.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = match ParticipantConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "participant config error");
            return ExitCode::from(2);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(config.worker_threads)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            error!(error = %e, "tokio runtime build error");
            return ExitCode::from(2);
        }
    };

    match runtime.block_on(async_main(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!(error = %e, "participant exited with error");
            ExitCode::from(1)
        }
    }
}

async fn async_main(config: ParticipantConfig) -> Result<(), ServerError> {
    // PR #18 — the OTel SDK is initialised AFTER `ParticipantState::new`
    // below so the BatchSpanProcessor records into the same
    // OtelExporterMetrics instance that the Prometheus exporter
    // surfaces on `/metrics` (AC20).
    let mut _otel_guard: Option<efe_meta_swarm_server::tracing_otel::TracerGuard> = None;

    let manifest_path = std::env::var_os("EFE_META_SWARM_MANIFEST").map_or_else(
        || PathBuf::from("/etc/efe-meta-swarm/manifest.yaml"),
        PathBuf::from,
    );

    info!(
        version = efe_meta_swarm_server::VERSION,
        git_sha = efe_meta_swarm_server::GIT_SHA,
        profile = efe_meta_swarm_server::PROFILE,
        id = %config.id,
        grpc_bind = %config.grpc_bind,
        metrics_bind = %config.metrics_bind,
        worker_threads = config.worker_threads,
        manifest_path = %manifest_path.display(),
        tls_required = config.tls_required,
        tls_mutual = config.tls_mutual,
        tls_min_protocol = ?config.tls_min_protocol,
        "participant starting",
    );

    // 1. Load + validate the shared manifest.
    let manifest = match manifest::load_and_validate(&manifest_path) {
        Ok(m) => m,
        Err(e) => {
            error!(
                error = %e,
                manifest_path = %manifest_path.display(),
                "manifest load/validate failed",
            );
            return Err(e);
        }
    };

    // 2. Find this participant's row.
    let row = match manifest::find_participant(&manifest, &config.id) {
        Some(r) => r.clone(),
        None => {
            error!(
                id = %config.id,
                participants = manifest.participants.len(),
                "participant id not found in manifest",
            );
            return Err(ServerError::Config(format!(
                "participant id {:?} not declared in manifest",
                config.id,
            )));
        }
    };

    // 3. Build the in-process participant.
    let local = LocalParticipant::from_manifest_row(&row).map_err(|e| {
        ServerError::Config(format!(
            "LocalParticipant build failed for {:?} : {e}",
            config.id,
        ))
    })?;

    info!(
        id = %local.id(),
        dim = local.dim(),
        kappa = local.kappa(),
        seed = local.seed(),
        "participant loaded from manifest row",
    );

    // 4. Snapshot the iteration knobs.
    let iteration = IterationParams {
        alpha_default: manifest.iteration.alpha_default,
        coupling_gamma: manifest.iteration.coupling_gamma,
    };

    // 5. Build state + service.
    let state = ParticipantState::new(config.clone(), local, iteration);

    // PR #18 — initialise the OpenTelemetry SDK with the participant
    // role + the OtelExporterMetrics handle from the state's metrics
    // registry.  The BatchSpanProcessor's export-result counters now
    // surface on `/metrics` (AC20).
    _otel_guard = Some(
        efe_meta_swarm_server::tracing_otel::init_otel_or_disabled_with_metrics(
            efe_meta_swarm_server::tracing_otel::Role::Participant,
            state.metrics.otel_exporter().clone(),
        ),
    );
    info!(
        otel_enabled = _otel_guard.as_ref().map(|g| g.is_enabled()).unwrap_or(false),
        "OTel SDK init complete",
    );

    let svc = ParticipantService::new(state.clone());

    // ─── PR #16 : bootstrap TLS subsystem ──────────────────────────
    let server_tls = match bootstrap_tls(&config, &state).await {
        Ok(opt) => opt,
        Err(e) => {
            error!(error = %e, "TLS bootstrap failed — fail-closed (AD25)");
            return Err(e);
        }
    };

    // 5a. Install the SIGHUP handler **before** we start serving so
    // a SIGHUP arriving during the gRPC bind window is captured by
    // the kernel.  PR #12 § 11 — the handler reloads the manifest
    // from disk and atomically swaps the participant's
    // `IterationParams` (α / γ).  Own-row drift (κ / dim / seed) is
    // rejected with the binary keeping the previous params.
    //
    // We discard the worker's `JoinHandle` — it lives for the binary's
    // lifetime ; SIGTERM tears down the tokio runtime which drops it.
    let sighup_state = state.clone();
    let _sighup_worker = shutdown::install_sighup_handler(move || {
        let state = sighup_state.clone();
        async move {
            match efe_meta_swarm_server::participant::apply_reload_from_disk(&state).await {
                Ok(outcome) => {
                    info!(
                        new_reload_gen = outcome.new_reload_gen,
                        elapsed_us = outcome.elapsed_us,
                        "SIGHUP : participant manifest reloaded",
                    );
                }
                Err(e) => {
                    error!(
                        error = %e,
                        "SIGHUP : participant reload rejected ; \
                         running state unchanged (AD6 transactional)",
                    );
                }
            }
        }
    });

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let metrics_handle = {
        let rx = shutdown_rx.clone();
        let registry = state.metrics.clone();
        let addr = config.metrics_bind;
        tokio::spawn(async move {
            if let Err(e) = metrics::spawn_exporter(addr, registry, rx).await {
                error!(error = %e, "metrics exporter failed");
            }
        })
    };

    let server_root = Server::builder().timeout(std::time::Duration::from_secs(60));
    let mut server_root = if let Some(tls) = server_tls {
        info!("participant gRPC : TLS active");
        server_root
            .tls_config(tls)
            .map_err(ServerError::Transport)?
    } else {
        info!("participant gRPC : plain-text (TLS disabled)");
        server_root
    };
    // PR #18 AC6/AC7/AC8 — install the W3C Trace Context server
    // interceptor so every inbound RPC's `traceparent` is bound to
    // the current tracing span as parent.  The downstream
    // `tracing::info_span!("efe.meta_swarm.step", ...)` in
    // `participant/service.rs::step` then inherits the controller's
    // `efe.meta_swarm.iterate` as parent — closing the propagation
    // loop initiated by the controller's `ClientTraceInterceptor`.
    let server = server_root.add_service(MetaParticipantServiceServer::with_interceptor(
        svc,
        efe_meta_swarm_server::tracing_otel::interceptor::ServerTraceInterceptor::new(),
    ));

    let grpc_handle = {
        let mut shutdown_rx = shutdown_rx.clone();
        let addr = config.grpc_bind;
        tokio::spawn(async move {
            info!(addr = %addr, "gRPC server listening");
            let serve = server.serve_with_shutdown(addr, async move {
                let _ = shutdown_rx.changed().await;
                info!("gRPC server shutting down");
            });
            if let Err(e) = serve.await {
                error!(error = %e, "gRPC server failed");
            }
        })
    };

    let kind = shutdown::await_termination().await;
    info!(signal = kind, "shutting down");

    // PR #18 Span 8 — wrap the participant drain window in an OTel
    // span (mirror of the controller's `efe.meta_swarm.shutdown.drain`).
    // The participant has no `inflight` counter (Phase-2 participants
    // are stateless step handlers), so `inflight_at_*` is hard-coded
    // to 0 ; the meaningful telemetry is the signal + elapsed time.
    // Audit-pass G9b — without this span the participant fleet's
    // shutdown latency is invisible to dashboards.
    let drain = std::time::Duration::from_secs(30);
    let drain_secs = drain.as_secs_f64();
    let drain_span =
        efe_meta_swarm_server::tracing_otel::spans::span_shutdown_drain(kind, drain_secs, 0);
    let drain_started = std::time::Instant::now();
    let _drain_entered = drain_span.clone();
    let _entered = _drain_entered.enter();

    state.health.set_not_serving();
    let _ = shutdown_tx.send(true);

    let mut drain_outcome = "clean";
    if let Err(e) = tokio::time::timeout(drain, grpc_handle).await {
        error!(error = %e, "participant gRPC server did not drain in time");
        drain_outcome = "deadline";
    }
    if let Err(e) = tokio::time::timeout(drain, metrics_handle).await {
        error!(error = %e, "participant metrics exporter did not drain in time");
        drain_outcome = "deadline";
    }

    // Close the drain span with the outcome attributes.
    let elapsed_secs = drain_started.elapsed().as_secs_f64();
    drop(_entered);
    efe_meta_swarm_server::tracing_otel::spans::record_shutdown_drain_close(
        &drain_span,
        0,
        elapsed_secs,
        drain_outcome,
    );

    info!(
        elapsed_secs,
        outcome = drain_outcome,
        "participant stopped",
    );
    Ok(())
}

/// PR #16 — bootstrap TLS for the participant.
///
/// Mirror of `controller.rs::bootstrap_tls` ; both binaries follow
/// the same lifecycle (load + register metrics + spawn watcher +
/// expiry loop + reload task), but the participant verifies the
/// controller's cert SAN whereas the controller verifies a (possibly
/// larger) participant SAN allowlist.
async fn bootstrap_tls(
    config: &ParticipantConfig,
    state: &ParticipantState,
) -> Result<Option<ServerTlsConfig>, ServerError> {
    if !config.tls_required {
        return Ok(None);
    }

    let cert_path = config
        .tls_cert_path
        .clone()
        .ok_or_else(|| ServerError::Config("tls_required=true but tls_cert_path unset".into()))?;
    let key_path = config
        .tls_key_path
        .clone()
        .ok_or_else(|| ServerError::Config("tls_required=true but tls_key_path unset".into()))?;
    let ca_path = config.tls_ca_path.clone().unwrap_or_else(|| {
        let mut p = cert_path.clone();
        p.set_file_name("ca.crt");
        p
    });

    let tls_paths = TlsPaths {
        cert_chain: cert_path,
        key: key_path,
        ca: ca_path,
    };

    let tls_metrics = Arc::new(
        TlsMetrics::register(state.metrics.registry())
            .map_err(|e| ServerError::Config(e.to_string()))?,
    );

    let reloader = Arc::new(
        TlsReloader::bootstrap(
            tls_paths.clone(),
            config.tls_mutual,
            config.tls_allowed_peer_sans.clone(),
            config
                .tls_peer_san
                .clone()
                .unwrap_or_else(|| "controller".to_string()),
            config.tls_min_protocol,
            tls_metrics,
        )
        .map_err(|e| ServerError::Config(e.to_string()))?,
    );

    let _expiry_task = reloader.clone().spawn_cert_expiry_loop();

    let (watcher, rx) = CertWatcher::watch(vec![
        tls_paths.cert_chain.clone(),
        tls_paths.key.clone(),
        tls_paths.ca.clone(),
    ])
    .map_err(|e| ServerError::Config(e.to_string()))?;
    // Attach the watcher to the reloader so its OS handles release
    // cleanly on shutdown (Box::leak retired in PR #16 audit-pass).
    reloader.attach_watcher(watcher);

    let _reload_task = reloader.clone().spawn_reload_task(rx);

    let server_cfg_owned = reloader.current_server_config();
    let identity = Identity::from_pem(&server_cfg_owned.cert_chain_pem, &server_cfg_owned.key_pem);
    let mut server_tls = ServerTlsConfig::new().identity(identity);
    if config.tls_mutual {
        let ca_pem = server_cfg_owned.client_ca_pem.as_ref().ok_or_else(|| {
            ServerError::Config("tls_mutual=true but client_ca_pem missing on reload state".into())
        })?;
        let ca = TonicCertificate::from_pem(ca_pem);
        server_tls = server_tls.client_ca_root(ca).client_auth_optional(false);
    }

    info!(
        cert = ?tls_paths.cert_chain.display(),
        key = ?tls_paths.key.display(),
        ca = ?tls_paths.ca.display(),
        mutual = config.tls_mutual,
        min_protocol = ?config.tls_min_protocol,
        "PR #16 participant TLS bootstrap complete",
    );
    let _ = &reloader;

    Ok(Some(server_tls))
}
