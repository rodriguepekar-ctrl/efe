//! EFE gRPC server entry point.
//!
//! Bootstraps telemetry (M11), validates configuration, then builds
//! either:
//!
//! * a **real Mode-1 `SwarmController`** when `EFE_BACKEND=mvp` (default),
//!   wiring `HostBackend` + `MemoryPool` + `StreamPool` + `GenesisEngine`
//!   + `MvpCpuFusion` + `MultiCriteriaDetector` + `StrictAlignmentProjector`
//!   + `PassThroughDissolver` (see `mode1.rs`), or
//! * a **stub `UnimplementedBackend`** when `EFE_BACKEND=stub`, which
//!   always returns `InvalidConfig` — handy for isolating M10/M11 wiring
//!   tests from M3..M8 logic.
//!
//! Either way the chosen backend is wrapped in an [`InstrumentedBackend`]
//! so every `Execute` call produces swarm-lifecycle metrics + span
//! events via the [`TracingTelemetryCollector`] (audit finding M11-#9).

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use tracing::{info, warn};

use efe_core::{alignment::InMemoryRegistry, EfeError, Request, Response, Result};
use efe_server::mode1::{build_mvp_backend, Mode1Config};
use efe_server::services::{ExecuteBackend, InstrumentedBackend};
use efe_server::{Server, ServerConfig, Shutdown};
use efe_telemetry::{
    init as init_telemetry, RegistrySink, TelemetryConfig, TracingTelemetryCollector,
};

/// Stub backend that always returns an `InvalidConfig` error.
///
/// Selected via `EFE_BACKEND=stub`. Useful for:
///   * smoke-testing the gRPC + telemetry wiring without exercising
///     the full fusion stack;
///   * reproducing the historical MVP behaviour when triaging a
///     regression that might live in M3..M8.
#[derive(Debug, Default)]
struct UnimplementedBackend;

#[async_trait]
impl ExecuteBackend for UnimplementedBackend {
    async fn execute(&self, _req: Request) -> Result<Response> {
        Err(EfeError::InvalidConfig(
            "stub backend selected via EFE_BACKEND=stub; set EFE_BACKEND=mvp \
             (the default) to enable the real Mode-1 SwarmController."
                .into(),
        ))
    }
}

/// Parse `EFE_BACKEND`. Empty / absent → `"mvp"`. Unknown value →
/// warning + fallback to `"mvp"` (default-safe).
fn backend_kind_from_env() -> &'static str {
    match std::env::var("EFE_BACKEND").ok().as_deref() {
        None | Some("") | Some("mvp") | Some("MVP") => "mvp",
        Some("stub") | Some("STUB") => "stub",
        Some(other) => {
            warn!(value = %other, "unknown EFE_BACKEND, defaulting to 'mvp'");
            "mvp"
        }
    }
}

/// Parse `EFE_SWARM_DIM_LATENT` — dimension of the latent space the
/// essaim should operate in. Must match the catalog / embedder used
/// by the clients. Defaults to 384 (sentence-transformers/all-MiniLM-L6-v2).
fn dim_latent_from_env() -> u32 {
    std::env::var("EFE_SWARM_DIM_LATENT")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(384)
}

/// Truthy boolean env var (true, 1, yes — case insensitive). Default `false`.
fn env_bool(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"),
        Err(_) => false,
    }
}

/// D93 — optionally spawn the M26 REST gateway (`/v1/agent/*`)
/// concurrently with the gRPC server.
///
/// Activation : `EFE_AGENT_REST_ENABLED=true` ; default OFF so existing
/// deployments are not impacted.
///
/// When active, **every** sub-component must succeed before the binary
/// boots — a partial wiring would be worse than the gateway being off,
/// because callers would see a 503 storm. Failures are surfaced as
/// `anyhow::Error` so the operator gets a clean log line + non-zero exit.
///
/// Connection details :
/// - Postgres DSN : `EFE_AGENT_RUNTIME_DATABASE_URL` (preferred) or
///   `DATABASE_URL` (fallback for single-DB clusters).
/// - Listen addr : `EFE_AGENT_REST_ADDR` (default `0.0.0.0:8088`).
///   **Port 8088 chosen to avoid conflict with the existing `health`
///   container port 8081** (livenessProbe / readinessProbe).
///   Production deployments expose this via the
///   `charts/efe-stack/charts/efe-server/templates/service.yaml`
///   Service when `Values.agentRestGateway.enabled = true`.
///
/// See `docs/docs/adr/ADR-D93-003-rest-mount-strategy.md` for the
/// in-process trait + future-grpc-mesh rationale.
/// M27 — initialise the Commercial Edge data plane :
/// - Redis cache client (from `EFE_REDIS_URL`).
/// - DbContext (rw + ro pools) from `EFE_DATABASE_URL` + optional
///   `EFE_DATABASE_RO_URL`.
/// - Prometheus metrics (registered lazily).
///
/// Returns Ok even when individual sub-systems fail to connect — the
/// binary degrades gracefully (no cache → recompute embeddings, no ro
/// pool → fall back to primary, etc.). Only fatal when
/// `EFE_M27_REQUIRE_RW=true` AND `EFE_DATABASE_URL` is missing.
async fn bootstrap_m27_edge() -> anyhow::Result<()> {
    // Initialize cache crate metrics + middleware metrics at boot so
    // they appear in `/metrics` even before first op.
    efe_cache::metrics::init();
    efe_server::middleware::backpressure::backpressure_metrics::init();

    // Probe Redis (fail-open).
    if let Ok(url) = std::env::var("EFE_REDIS_URL") {
        match efe_cache::CacheClient::connect(&url).await {
            Ok(client) => match client.ping().await {
                Ok(()) => info!(backend = client.backend_kind, "M27 cache : Redis reachable"),
                Err(e) => warn!(?e, "M27 cache : Redis PING failed (degraded)"),
            },
            Err(e) => warn!(?e, "M27 cache : connect failed (degraded)"),
        }
    } else {
        info!("EFE_REDIS_URL not set ; M27 cache disabled (degraded)");
    }

    // Probe DbContext.
    if std::env::var("EFE_DATABASE_URL").is_ok() {
        match efe_server::db_context::DbContextConfig::from_env() {
            Ok(cfg) => match efe_server::db_context::DbContext::connect(cfg).await {
                Ok(_dbctx) => {
                    info!("M27 DbContext : rw + ro pools connected");
                    // NB : the actual DbContext value is constructed
                    // again inside operator binaries that need it (e.g.
                    // M27 mounts on a fresh router). Keeping the probe
                    // here is enough for boot-time validation.
                }
                Err(e) => {
                    if env_bool("EFE_M27_REQUIRE_RW") {
                        anyhow::bail!("EFE_M27_REQUIRE_RW=true but DbContext failed : {}", e);
                    } else {
                        warn!(?e, "M27 DbContext connect failed (degraded)");
                    }
                }
            },
            Err(e) => warn!(?e, "M27 DbContext config invalid (degraded)"),
        }
    } else {
        info!("EFE_DATABASE_URL not set ; M27 DbContext skipped");
    }

    Ok(())
}

async fn try_start_agent_rest_gateway(
    shutdown: Shutdown,
) -> anyhow::Result<Option<tokio::task::JoinHandle<()>>> {
    if !env_bool("EFE_AGENT_REST_ENABLED") {
        info!("EFE_AGENT_REST_ENABLED!=true ; M26 REST gateway disabled");
        return Ok(None);
    }

    // ── Postgres ────────────────────────────────────────────────────
    let db_url = std::env::var("EFE_AGENT_RUNTIME_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .context(
            "EFE_AGENT_REST_ENABLED=true but neither \
             EFE_AGENT_RUNTIME_DATABASE_URL nor DATABASE_URL is set",
        )?;
    let pg = sqlx::PgPool::connect(&db_url)
        .await
        .context("failed to connect to agent runtime database")?;
    info!("agent REST gateway : postgres connected");

    // ── Profile registry (loaded once from default file) ────────────
    let profiles = Arc::new(
        efe_agent_runner::profile::ProfileRegistry::from_default_file()
            .context("failed to load profile registry")?,
    );

    // ── Tenant flags provider (DB-backed) ──────────────────────────
    let flags = Arc::new(efe_agent_runner::PgTenantFlagsProvider::new(pg.clone()));

    // ── Service + adapter ─────────────────────────────────────────
    let service = Arc::new(efe_agent_runner::service::AgentRuntimeService::new(
        pg.clone(),
        flags,
        profiles,
    ));
    let state = efe_server::v1::agent::build_agent_state(service, pg);
    let router = efe_server::v1::agent::build_agent_router(state);

    // ── Bind listener ─────────────────────────────────────────────
    let addr_str =
        std::env::var("EFE_AGENT_REST_ADDR").unwrap_or_else(|_| "0.0.0.0:8088".to_string());
    let addr: SocketAddr = addr_str
        .parse()
        .with_context(|| format!("invalid EFE_AGENT_REST_ADDR : '{}'", addr_str))?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind agent REST listener on {}", addr))?;
    info!(%addr, "agent REST gateway listening on /v1/agent/*");

    // ── Spawn with graceful shutdown ───────────────────────────────
    let handle = tokio::spawn(async move {
        let serve = axum::serve(listener, router).with_graceful_shutdown(async move {
            shutdown.notified().await;
            info!("agent REST gateway received shutdown signal");
        });
        if let Err(e) = serve.await {
            warn!(?e, "agent REST gateway serve loop exited with error");
        } else {
            info!("agent REST gateway stopped cleanly");
        }
    });

    Ok(Some(handle))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 0. MVP stance on CLI arguments: the binary is configured exclusively
    //    via environment variables (see M12 §8.4). Anything passed on the
    //    command line is ignored but logged, so an operator who expected
    //    flags does not silently wonder why their tweak had no effect.
    //    (The `env_logger::init()`-style call that logs before we have a
    //    subscriber is intentionally skipped — we emit this warning
    //    through `tracing::warn!` *after* telemetry is up.)
    let stray_argv: Vec<String> = std::env::args().skip(1).collect();

    // 1. Telemetry stack — installs the global `tracing-subscriber`
    //    (JSON logs), registers the 12 Prometheus instruments, and
    //    prepares the `/metrics` HTTP sidecar (lazily bound below).
    let telemetry_cfg = TelemetryConfig::from_env();
    let mut telemetry = init_telemetry(telemetry_cfg).context("init telemetry")?;
    telemetry
        .ensure_metrics_bound()
        .await
        .context("bind metrics endpoint")?;

    if !stray_argv.is_empty() {
        // N arguments received but none are acted on. This is a frequent
        // footgun when migrating from `flag`-style servers, so we call it
        // out loudly.
        warn!(
            argv_count = stray_argv.len(),
            "efe-server takes no CLI arguments; ignoring — configure via EFE_* env vars"
        );
    }

    // 2. Parse + validate configuration from the environment.
    //    A malformed variable crashes the process with a single, precise
    //    line in the log (see `ConfigError` variants).
    let cfg = ServerConfig::from_env().context("parse ServerConfig from environment")?;
    cfg.validate().context("validate ServerConfig")?;

    // 3. Core infrastructure. We keep a concrete `InMemoryRegistry`
    //    reference long enough to siphon its `RegistryEvent` receiver
    //    before we upcast it to `Arc<dyn AlignmentRegistry>` for the
    //    server.
    let concrete_registry = Arc::new(InMemoryRegistry::with_default_config()?);
    let events_rx = concrete_registry.take_events_receiver().await;

    // 4. Plug the M9 registry event stream into the telemetry sink so
    //    alignment breaches / cone mutations show up in the audit log
    //    and Prometheus counters.
    if let Some(rx) = events_rx {
        let audit = telemetry.audit_sink();
        tokio::spawn(RegistrySink::new(audit).run(rx));
    }

    // 5. Instrument the backend with a telemetry collector — this is
    //    where `efe_swarm_*` metrics start flowing. The collector is
    //    independent of the concrete backend, so the same wiring works
    //    for `Mode1Backend`, stubs, or future remote backends.
    let backend_kind = backend_kind_from_env();
    let dim_latent = dim_latent_from_env();
    info!(backend = %backend_kind, dim_latent, "selecting swarm backend");
    let raw_backend: Arc<dyn ExecuteBackend> = match backend_kind {
        "stub" => Arc::new(UnimplementedBackend),
        _ => {
            // Sprint D7 — pick up `EFE_FUSION_*` calibration overrides
            // from the environment so a calibration sweep can drive
            // the running server without recompiling. In production
            // these env vars are unset, so the result is identical to
            // `Mode1Config::for_dim(dim_latent)`.
            let mode1_cfg = Mode1Config::from_env_for_dim(dim_latent);
            build_mvp_backend(mode1_cfg).context("build Mode-1 SwarmController backend")?
        }
    };
    let telemetry_collector: Arc<dyn efe_core::TelemetryCollector> =
        Arc::new(TracingTelemetryCollector::new());
    let backend: Arc<dyn ExecuteBackend> =
        Arc::new(InstrumentedBackend::new(raw_backend, telemetry_collector));

    // 6. Build and serve.
    let registry_for_server: Arc<dyn efe_core::alignment::AlignmentRegistry> = concrete_registry;
    let server = Server::builder()
        .config(cfg.clone())
        .backend(backend)
        .registry(registry_for_server)
        .build()
        .await
        .context("failed to build server")?;

    let shutdown = Shutdown::new();
    let addr = cfg.listen_addr;
    info!(
        version = efe_core::VERSION,
        %addr,
        metrics_addr = ?telemetry.metrics_local_addr(),
        auth_enabled = cfg.auth.enabled,
        enable_admin = cfg.enable_admin,
        drain_ms = cfg.shutdown.grace_period.as_millis() as u64,
        "efe-server starting"
    );

    // 6.5. D93 — optionally start the M26 REST gateway concurrently
    // with the gRPC server. Off by default ; opt-in via
    // EFE_AGENT_REST_ENABLED=true. Failure is fatal so the operator
    // notices the broken wiring immediately instead of debugging a
    // silently-missing endpoint.
    let agent_rest_handle = try_start_agent_rest_gateway(shutdown.clone())
        .await
        .context("failed to start agent REST gateway")?;

    // 6.6. M27 — Commercial Edge bootstrap.
    //
    // Initialises the Redis cache plane (if EFE_REDIS_URL set), the
    // tenant quota counters, and the DbContext with read/write split.
    // None of these are required at boot ; the binary keeps working
    // when EFE_REDIS_URL is absent (degrades to no-cache). The
    // middleware Tower layers are still **prepared** here so they can
    // be applied to the router by the operator binary integrators
    // (e.g. agent REST gateway above).
    //
    // Failure mode : every step is fail-open. Cache init that fails
    // logs WARN and continues ; only DbContext (RW URL) is fatal
    // and only if EFE_M27_REQUIRE_RW=true.
    if env_bool("EFE_M27_EDGE_ENABLED") {
        match bootstrap_m27_edge().await {
            Ok(()) => info!("M27 Commercial Edge bootstrap complete"),
            Err(e) => warn!(?e, "M27 Commercial Edge bootstrap failed (degraded)"),
        }
    } else {
        info!("EFE_M27_EDGE_ENABLED!=true ; skipping M27 bootstrap (degraded)");
    }

    // `server.serve` already returns `anyhow::Result<()>` — the
    // previous `map_err(anyhow::Error::from)` was a useless conversion
    // (clippy::useless_conversion). We still capture the result before
    // shutting down telemetry so the shutdown is sequenced correctly
    // regardless of the serve outcome.
    let serve_result = server.serve(addr, shutdown).await;

    // Wait for the agent REST gateway to drain (if it was started).
    // `JoinHandle` errors are logged but do not propagate — by this
    // point the gRPC server has already decided the exit code.
    if let Some(h) = agent_rest_handle {
        match h.await {
            Ok(()) => info!("agent REST gateway joined cleanly"),
            Err(e) => warn!(?e, "agent REST gateway task panicked or cancelled"),
        }
    }

    // 7. Flush telemetry before exit. Idempotent.
    telemetry.shutdown().await;
    info!("efe-server shut down cleanly");
    serve_result
}
