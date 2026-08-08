//! Controller binary entry-point.
//!
//! Runs a Tonic gRPC server on `EFE_META_SWARM_CONTROLLER_ADDR` and a
//! Prometheus exporter on `EFE_META_SWARM_CONTROLLER_METRICS`.  Exits
//! cleanly on SIGTERM / SIGINT.  Reloads the manifest in-place on
//! SIGHUP (PR #12).
//!
//! PR #16 — when `EFE_META_SWARM_TLS_REQUIRED=true`, the binary
//! bootstraps a [`TlsReloader`] from the mounted cert / key / CA files,
//! wires `tonic::transport::Server::tls_config(...)` with the
//! resulting `rustls::ServerConfig`, registers the 5 TLS Prometheus
//! series with the shared registry, spawns the inotify watcher +
//! debounce loop, and spawns the cert-expiry sampling loop.

use std::process::ExitCode;
use std::sync::Arc;

use efe_meta_swarm_proto::controller::MetaSwarmServiceServer;
use efe_meta_swarm_server::{
    config::controller::ControllerConfig,
    controller::{ControllerService, ControllerState},
    observability::{metrics, shutdown, tracing as obs_tracing},
    tls::{reload::TlsPaths, CertWatcher, TlsMetrics, TlsReloader},
};

use ::tracing::{error, info, warn};
use tokio::sync::watch;
use tonic::transport::{Certificate as TonicCertificate, Identity, Server, ServerTlsConfig};

fn main() -> ExitCode {
    obs_tracing::init();

    // PR #16 — install the rustls process-wide crypto provider before
    // we touch any rustls type.  rustls 0.23 requires this to be done
    // exactly once per process ; we use `ring` (FIPS-friendly,
    // identical to `tokio-rustls`'s default).  `install_default`
    // returns Err if a different provider is already installed —
    // harmless here, the binary starts cold.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = match ControllerConfig::from_env() {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "controller config error");
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
            error!(error = %e, "controller exited with error");
            ExitCode::from(1)
        }
    }
}

async fn async_main(
    config: ControllerConfig,
) -> Result<(), efe_meta_swarm_server::error::ServerError> {
    // PR #18 — the OTel SDK is initialised AFTER `ControllerState::new`
    // below, so the BatchSpanProcessor records into the same
    // `OtelExporterMetrics` instance that the Prometheus exporter
    // surfaces on `/metrics` (AC20).  See § 17.3 of the architecture
    // doc.  Hold a placeholder so the rest of this function can refer
    // to `_otel_guard` and shutdown still works after error paths.
    let mut _otel_guard: Option<efe_meta_swarm_server::tracing_otel::TracerGuard> = None;

    info!(
        version = efe_meta_swarm_server::VERSION,
        git_sha = efe_meta_swarm_server::GIT_SHA,
        profile = efe_meta_swarm_server::PROFILE,
        grpc_bind = %config.grpc_bind,
        metrics_bind = %config.metrics_bind,
        worker_threads = config.worker_threads,
        discovery = ?config.discovery,
        tls_required = config.tls_required,
        tls_mutual = config.tls_mutual,
        tls_min_protocol = ?config.tls_min_protocol,
        "controller starting",
    );

    // Load + validate the manifest at startup.  PR #8 made the
    // manifest load-or-fail at startup ; PR #12 wires the in-place
    // SIGHUP-driven hot reload below via `install_sighup_handler` +
    // `controller::reload::apply_reload_from_disk`.
    let manifest = match efe_meta_swarm_server::manifest::load_and_validate(&config.manifest_path) {
        Ok(m) => m,
        Err(e) => {
            error!(error = %e, manifest_path = %config.manifest_path.display(),
                "manifest load/validate failed");
            return Err(e);
        }
    };
    info!(
        manifest_version = %manifest.version,
        participants = manifest.participants.len(),
        meta_dim = manifest.meta.dim,
        "manifest loaded",
    );

    let mut state = ControllerState::new(config.clone(), manifest);

    // ── Couche 3 M2 — la clé et le tenant du reçu de consensus (fail-closed) ────────────────
    //
    // Résolu ICI, avant que le serveur gRPC ne se lie : une provenance demandée sans clé sûre est
    // un REFUS DE DÉMARRER, pas un repli silencieux sur une clé de dev publiquement connue. Même
    // patron que `frontier_boot_guard` (Couche 2 M4) et `h1_boot_gate` (Couche 3 M1.d) — un
    // opérateur qui lit « préflight OK » doit pouvoir en déduire ce que le système garantit.
    match efe_meta_swarm_server::controller::provenance::ProvenanceConfig::from_env() {
        Ok(Some(cfg)) => {
            if cfg.is_forgeable() {
                // Le dire, et le RE-dire : un reçu sous clé de dev ressemble à une preuve.
                warn!(
                    key_source = cfg.label(),
                    "provenance de consensus ACTIVE sous la clé de DÉVELOPPEMENT — le \
                     meta_provenance_root servi est FORGEABLE par quiconque connaît cette clé \
                     (elle est publique). Ne PAS présenter ces reçus comme une preuve.",
                );
            } else {
                info!(key_source = cfg.label(), "provenance de consensus active — chaque Iterate scelle ses pas");
            }
            state.set_provenance(std::sync::Arc::new(cfg));
        }
        Ok(None) => {
            info!(
                "provenance de consensus INACTIVE (aucun {} déclaré) — le fil portera un \
                 meta_provenance_root VIDE : aucun reçu n'est produit, et c'est le signal honnête.",
                efe_meta_swarm_server::controller::provenance::TENANT_ENV,
            );
        }
        Err(why) => {
            error!(error = %why, "provenance de consensus mal configurée — refus de démarrer");
            return Err(efe_meta_swarm_server::error::ServerError::Config(why));
        }
    }

    // PR #18 — initialise the OpenTelemetry SDK inside the tokio
    // runtime context.  The BatchSpanProcessor uses tokio for async
    // export ; calling init_otel from `fn main()` (outside the runtime)
    // would panic at first export.  The guard is held for the whole
    // function lifetime ; its Drop flushes pending spans (5s timeout).
    //
    // We pass the OtelExporterMetrics handle from the state's metrics
    // registry so the BatchSpanProcessor's export results surface on
    // `/metrics` as the 3 PR #18 series (`exported_spans_total`,
    // `dropped_spans_total{reason}`, `export_duration_seconds`).
    _otel_guard = Some(
        efe_meta_swarm_server::tracing_otel::init_otel_or_disabled_with_metrics(
            efe_meta_swarm_server::tracing_otel::Role::Controller,
            state.metrics.otel_exporter().clone(),
        ),
    );
    info!(
        otel_enabled = _otel_guard.as_ref().map(|g| g.is_enabled()).unwrap_or(false),
        "OTel SDK init complete",
    );

    // ─── PR #16 : bootstrap TLS subsystem ──────────────────────────
    //
    // When `tls_required=true`, build a [`TlsReloader`] from the mounted
    // cert+key+CA, register the 5 TLS Prometheus series with the
    // controller's shared registry, spawn the inotify watcher +
    // cert-expiry sampling loops, and capture the [`ServerTlsConfig`]
    // we'll hand to the tonic `Server::tls_config(...)` below.
    //
    // The reloader returned here is plumbed into `ControllerState`
    // so that `controller::iterate::connect_participants` can dial
    // each participant over mTLS.
    let (server_tls, tls_reloader) = match bootstrap_tls(&config, &state).await {
        Ok(pair) => pair,
        Err(e) => {
            error!(error = %e, "TLS bootstrap failed — fail-closed (AD25)");
            return Err(e);
        }
    };
    if let Some(r) = tls_reloader.clone() {
        state.set_tls_reloader(r);
    }
    // Re-bind to immutable now that all setup is done.
    let state = state;
    let svc = ControllerService::new(state.clone());

    // Install the SIGHUP handler **before** we start serving traffic
    // so a SIGHUP that arrives during the gRPC bind window is
    // captured by the kernel.  PR #12 § 8.2 — the handler dispatches
    // to a background task ; concurrent SIGHUPs are coalesced via
    // `Notify::notify_one`.  We deliberately discard the JoinHandle
    // — the worker lives for the binary's lifetime ; SIGTERM tears
    // down the tokio runtime which drops it.
    let sighup_state = state.clone();
    let _sighup_worker = shutdown::install_sighup_handler(move || {
        let state = sighup_state.clone();
        async move {
            match efe_meta_swarm_server::controller::apply_reload_from_disk(&state).await {
                Ok(outcome) => {
                    info!(
                        new_reload_gen = outcome.new_reload_gen,
                        participants = outcome.participants,
                        elapsed_us = outcome.elapsed_us,
                        changes = ?outcome.applied_changes,
                        "SIGHUP : manifest reloaded",
                    );
                }
                Err(e) => {
                    // Failure leaves the running state untouched
                    // (AD6 transactional reload).  Operator can fix
                    // the manifest and SIGHUP again.
                    error!(error = %e, "SIGHUP : reload rejected ; running state unchanged");
                }
            }
        }
    });

    // Shutdown channel : flipped to `true` on SIGTERM/SIGINT ; both
    // the gRPC server and the metrics exporter watch it.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Spawn the Prometheus exporter.
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

    let _ = config.reflection_enabled; // suppress unused warning ; PR #18 wires.

    // Build the gRPC server.  PR #16 — apply tls_config if TLS active.
    let server_root = Server::builder().timeout(std::time::Duration::from_secs(60));
    let mut server_root = if let Some(tls) = server_tls {
        info!("controller gRPC : TLS active");
        server_root
            .tls_config(tls)
            .map_err(efe_meta_swarm_server::error::ServerError::Transport)?
    } else {
        info!("controller gRPC : plain-text (TLS disabled)");
        server_root
    };
    // PR #18 AC6/AC7/AC8 — install the W3C Trace Context server
    // interceptor so every inbound RPC's `traceparent` header is
    // extracted and bound to the current tracing span as parent.
    // The result : downstream `tracing::info_span!` invocations in
    // the handler inherit the upstream span as their parent.
    let server = server_root.add_service(MetaSwarmServiceServer::with_interceptor(
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

    // Wait for SIGTERM / SIGINT.
    let kind = shutdown::await_termination().await;
    info!(signal = kind, "shutting down");

    // PR #18 Span 8 — wrap the drain window in an OTel span so SRE
    // dashboards can see how long the binary took to flush in-flight
    // work + whether the deadline was hit.
    let drain_secs = config.drain_timeout.as_secs_f64();
    let inflight_at_start = u32::try_from(state.inflight.inflight()).unwrap_or(u32::MAX);
    let drain_span = efe_meta_swarm_server::tracing_otel::spans::span_shutdown_drain(
        kind,
        drain_secs,
        inflight_at_start,
    );
    let drain_started = std::time::Instant::now();

    let _drain_entered = drain_span.clone();
    let _entered = _drain_entered.enter();

    // Mark unhealthy : load balancers should stop sending traffic.
    state.health.set_not_serving();

    // Trigger shutdown.
    let _ = shutdown_tx.send(true);

    // Wait for both tasks (bounded by the drain timeout).
    let drain = config.drain_timeout;
    let mut drain_outcome = "clean";
    if let Err(e) = tokio::time::timeout(drain, grpc_handle).await {
        error!(error = %e, "gRPC server did not drain in time");
        drain_outcome = "deadline";
    }
    if let Err(e) = tokio::time::timeout(drain, metrics_handle).await {
        error!(error = %e, "metrics exporter did not drain in time");
        drain_outcome = "deadline";
    }

    // Close the drain span with the outcome attributes.
    let inflight_at_end = u32::try_from(state.inflight.inflight()).unwrap_or(u32::MAX);
    let elapsed_secs = drain_started.elapsed().as_secs_f64();
    drop(_entered);
    efe_meta_swarm_server::tracing_otel::spans::record_shutdown_drain_close(
        &drain_span,
        inflight_at_end,
        elapsed_secs,
        drain_outcome,
    );

    info!(
        elapsed_secs,
        outcome = drain_outcome,
        inflight_at_end,
        "controller stopped",
    );
    Ok(())
}

/// PR #16 — bootstrap the TLS subsystem.
///
/// Returns `Ok(None)` when `tls_required=false` (no-op for backward
/// compat with PR #14 head-of-main).
/// Returns `Ok(Some(ServerTlsConfig))` when TLS is active ; the
/// `ServerTlsConfig` is ready to hand to `tonic::transport::Server::
/// tls_config(...)`.  In that case the [`TlsReloader`] + watcher +
/// expiry-sampling loop have all been spawned ; the reloader is kept
/// alive by the spawned tasks holding an [`Arc`] to it.
async fn bootstrap_tls(
    config: &ControllerConfig,
    state: &ControllerState,
) -> Result<
    (Option<ServerTlsConfig>, Option<Arc<TlsReloader>>),
    efe_meta_swarm_server::error::ServerError,
> {
    if !config.tls_required {
        return Ok((None, None));
    }

    // Validated by ControllerConfig::from_env upstream.
    let cert_path = config.tls_cert_path.clone().ok_or_else(|| {
        efe_meta_swarm_server::error::ServerError::Config(
            "tls_required=true but tls_cert_path unset".into(),
        )
    })?;
    let key_path = config.tls_key_path.clone().ok_or_else(|| {
        efe_meta_swarm_server::error::ServerError::Config(
            "tls_required=true but tls_key_path unset".into(),
        )
    })?;
    let ca_path = config.tls_ca_path.clone().unwrap_or_else(|| {
        // Default mount location when EFE_META_SWARM_TLS_CA is unset
        // but mutual TLS isn't required.  The reloader treats the
        // CA bundle as optional in that case.
        let mut p = cert_path.clone();
        p.set_file_name("ca.crt");
        p
    });

    let tls_paths = TlsPaths {
        cert_chain: cert_path,
        key: key_path,
        ca: ca_path,
    };

    // Register TLS metrics on the shared registry.
    let tls_metrics = Arc::new(
        TlsMetrics::register(state.metrics.registry())
            .map_err(|e| efe_meta_swarm_server::error::ServerError::Config(e.to_string()))?,
    );

    // Build the reloader.  Failures here are fail-closed (AD25).
    let reloader = Arc::new(
        TlsReloader::bootstrap(
            tls_paths.clone(),
            config.tls_mutual,
            config.tls_allowed_peer_sans.clone(),
            config
                .tls_peer_san
                .clone()
                .unwrap_or_else(|| "participant".to_string()),
            config.tls_min_protocol,
            tls_metrics.clone(),
        )
        .map_err(|e| efe_meta_swarm_server::error::ServerError::Config(e.to_string()))?,
    );

    // Spawn the cert-expiry sampling loop (60s cadence).
    let _expiry_task = reloader.clone().spawn_cert_expiry_loop();

    // Spawn the inotify watcher + debounce loop.
    let (watcher, rx) = CertWatcher::watch(vec![
        tls_paths.cert_chain.clone(),
        tls_paths.key.clone(),
        tls_paths.ca.clone(),
    ])
    .map_err(|e| efe_meta_swarm_server::error::ServerError::Config(e.to_string()))?;
    // Attach the watcher to the reloader so its OS inotify handles
    // are released cleanly when the reloader's last Arc drops (binary
    // shutdown).  Replaces the prior `Box::leak` pattern.
    reloader.attach_watcher(watcher);

    let _reload_task = reloader.clone().spawn_reload_task(rx);

    // Build the initial ServerTlsConfig.
    //
    // PR #16 — tonic 0.12 exposes a narrow `ServerTlsConfig` API
    // (identity + client_ca_root + client_auth_optional).  The
    // protocol version + cipher suite policy declared in
    // [`crate::tls::config::TlsServerConfig`] is enforced at the
    // rustls layer in our integration tests (which drive
    // `tokio_rustls::TlsAcceptor` directly), but at the binary
    // transport layer we accept tonic's bundled rustls defaults
    // (TLS 1.2+1.3, no RSA-only suites).  This matches the
    // values.schema.json policy (`tls.minProtocolVersion ∈
    // {"1.2","1.3"}`) without requiring a fork of tonic.
    let server_cfg_owned = reloader.current_server_config();
    let identity = Identity::from_pem(&server_cfg_owned.cert_chain_pem, &server_cfg_owned.key_pem);
    let mut server_tls = ServerTlsConfig::new().identity(identity);
    if config.tls_mutual {
        let ca_pem = server_cfg_owned.client_ca_pem.as_ref().ok_or_else(|| {
            efe_meta_swarm_server::error::ServerError::Config(
                "tls_mutual=true but client_ca_pem missing on reload state".into(),
            )
        })?;
        let ca = TonicCertificate::from_pem(ca_pem);
        server_tls = server_tls.client_ca_root(ca).client_auth_optional(false);
    }

    // ArcSwap-based reload coordinates the **client-side** rotation
    // (controller→participant channels are rebuilt with the new
    // cert ; existing channels keep going on session keys derived at
    // handshake time).  The server-side TLS material is captured by
    // tonic at `tls_config(...)` time : for hot-rotation of the
    // server cert we rely on the 30s drain window (AD10) plus the
    // next inbound connection picking up the new cert via a Pod
    // refresh.  Phase-3 will extend this to a Server.tls_config_swap
    // API once tonic upstream exposes it.
    info!(
        cert = ?tls_paths.cert_chain.display(),
        key = ?tls_paths.key.display(),
        ca = ?tls_paths.ca.display(),
        mutual = config.tls_mutual,
        min_protocol = ?config.tls_min_protocol,
        "PR #16 TLS bootstrap complete",
    );

    // Return the reloader so the caller can plumb it into
    // ControllerState ; outbound (controller→participant) channels
    // call `current_client_config()` on every Iterate to pick up
    // rotated certs without restarting.
    Ok((Some(server_tls), Some(reloader)))
}
