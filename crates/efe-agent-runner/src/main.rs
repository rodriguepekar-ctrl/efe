//! `efe-agent-runner` controller binary.
//!
//! Two operational modes :
//!   1. **Controller** (default) — watches the `AgentRun` CRD, reconciles
//!      microVM sandbox lifecycle, talks to the M21 LLMRouter and to
//!      the embedded mcp-server (`efe-mcp-server`).
//!   2. **Healthcheck** (`efe-agent-runner --check`) — runs
//!      `validate_startup()` and exits 0/1 (used by Helm Job preStart).
//!
//! Started by Helm chart `efe-agent-runner`. Default OFF (feature flag
//! `EFE_FEATURE_AGENT_RUNTIME_ENABLED` must be `true`, plus M21 must be
//! ON, plus DPIA signed per tenant — see runbook).

use efe_agent_runner::{validate_startup, CRATE_VERSION};
use std::process::ExitCode;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> ExitCode {
    // Init structured logging (JSON in prod, pretty in dev).
    init_tracing();

    let args: Vec<String> = std::env::args().collect();
    let healthcheck_mode = args.iter().any(|a| a == "--check" || a == "--healthcheck");

    tracing::info!(version = CRATE_VERSION, mode = if healthcheck_mode { "healthcheck" } else { "controller" }, "efe-agent-runner starting");

    // Always run startup validation — fail fast.
    if let Err(err) = validate_startup() {
        tracing::error!(?err, "startup validation failed");
        return ExitCode::from(1);
    }

    if healthcheck_mode {
        tracing::info!("healthcheck OK");
        return ExitCode::SUCCESS;
    }

    // ── Read environment ───────────────────────────────────────────
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("DATABASE_URL not set");
            return ExitCode::from(2);
        }
    };
    let agent_runtime_enabled = matches!(
        std::env::var("EFE_FEATURE_AGENT_RUNTIME_ENABLED").as_deref(),
        Ok("true") | Ok("1") | Ok("yes")
    );
    if !agent_runtime_enabled {
        tracing::warn!("EFE_FEATURE_AGENT_RUNTIME_ENABLED=false → controller idles (no AgentRun reconcile)");
        // Still serve /healthz so the K8s probe stays green.
        idle_until_signal().await;
        return ExitCode::SUCCESS;
    }

    // ── Connect Postgres ───────────────────────────────────────────
    let pg = match sqlx::PgPool::connect(&database_url).await {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(?e, "failed to connect to database");
            return ExitCode::from(3);
        }
    };
    tracing::info!("postgres connected");

    // ── Connect Kubernetes ─────────────────────────────────────────
    let k8s = match kube::Client::try_default().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(?e, "failed to connect to kube apiserver");
            return ExitCode::from(4);
        }
    };
    tracing::info!("kubernetes client connected");

    // ── Spawn /metrics + /healthz HTTP server ──────────────────────
    let metrics_addr = std::env::var("EFE_M25_METRICS_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:9091".to_string());
    let metrics_handle = tokio::spawn(async move {
        if let Err(e) = serve_metrics(&metrics_addr).await {
            tracing::error!(?e, "metrics server crashed");
        }
    });

    // ── Build the shared microVM pool map (D93 fix D-2) ────────────
    // The controller AND the reaper share this Arc<Mutex<...>>. Pre-D93,
    // the reaper held its own empty map ; orphan slots were never reaped.
    let shared_pools: efe_agent_runner::controller::PoolMap =
        std::sync::Arc::new(tokio::sync::Mutex::new(Default::default()));

    // ── Spawn the controller reconcile loop ────────────────────────
    let controller_handle = tokio::spawn({
        let pg = pg.clone();
        let k8s = k8s.clone();
        let pools = shared_pools.clone();
        async move {
            if let Err(e) = efe_agent_runner::controller::run_with_pools(pg, k8s, pools).await {
                tracing::error!(?e, "controller crashed");
            }
        }
    });

    // ── Spawn the cost-cap watchdog ────────────────────────────────
    let watchdog_handle = tokio::spawn({
        let pg = pg.clone();
        async move {
            efe_agent_runner::cost::watchdog_loop(pg).await;
        }
    });

    // ── M26 : spawn the admission webhook server ──────────────────
    let webhook_enabled = matches!(
        std::env::var("EFE_M26_WEBHOOK_ENABLED").as_deref(),
        Ok("true") | Ok("1") | Ok("yes")
    );
    let webhook_handle = if webhook_enabled {
        let pg2 = pg.clone();
        let profiles = std::sync::Arc::new(
            efe_agent_runner::profile::ProfileRegistry::from_default_file()
                .unwrap_or_default(),
        );
        let state = efe_agent_runner::webhook::WebhookState { pg: pg2, profiles };
        let addr_str = std::env::var("EFE_M26_WEBHOOK_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8443".to_string());
        let addr: std::net::SocketAddr = match addr_str.parse() {
            Ok(a) => a,
            Err(e) => {
                tracing::error!(?e, "invalid EFE_M26_WEBHOOK_ADDR");
                return ExitCode::from(5);
            }
        };
        let cert_path = std::path::PathBuf::from(
            std::env::var("EFE_M26_WEBHOOK_TLS_CERT")
                .unwrap_or_else(|_| "/var/run/webhook-tls/tls.crt".to_string()),
        );
        let key_path = std::path::PathBuf::from(
            std::env::var("EFE_M26_WEBHOOK_TLS_KEY")
                .unwrap_or_else(|_| "/var/run/webhook-tls/tls.key".to_string()),
        );
        Some(tokio::spawn(async move {
            if let Err(e) = efe_agent_runner::webhook::server::serve_https(
                state, addr, cert_path, key_path,
            ).await {
                tracing::error!(?e, "webhook server crashed");
            }
        }))
    } else {
        tracing::warn!("M26 admission webhook disabled (EFE_M26_WEBHOOK_ENABLED!=true)");
        None
    };

    // ── M26 : spawn the orphan-slot reaper (D93 — shared pool map) ─
    let reaper_handle = tokio::spawn({
        let pg = pg.clone();
        let pools = shared_pools.clone();
        async move {
            efe_agent_runner::controller_reaper::reaper_loop(pg, pools).await;
        }
    });

    // ── Wait for SIGTERM ───────────────────────────────────────────
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("SIGINT received, shutting down");
        }
        _ = wait_sigterm() => {
            tracing::info!("SIGTERM received, shutting down");
        }
        r = metrics_handle => {
            tracing::warn!(?r, "metrics server exited unexpectedly");
        }
        r = controller_handle => {
            tracing::warn!(?r, "controller exited unexpectedly");
        }
        r = watchdog_handle => {
            tracing::warn!(?r, "cost watchdog exited unexpectedly");
        }
        r = reaper_handle => {
            tracing::warn!(?r, "reaper exited unexpectedly");
        }
        r = async {
            match webhook_handle {
                Some(h) => h.await.ok(),
                None => futures::future::pending::<Option<()>>().await,
            }
        } => {
            tracing::warn!(?r, "webhook server exited unexpectedly");
        }
    }

    pg.close().await;
    tracing::info!("efe-agent-runner stopped cleanly");
    ExitCode::SUCCESS
}

// ── Internals ──────────────────────────────────────────────────────

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,kube=warn,sqlx=warn"));
    let json = std::env::var("EFE_LOG_FORMAT").as_deref() == Ok("json");
    if json {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().json().with_target(true))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer().compact())
            .init();
    }
}

#[cfg(unix)]
async fn wait_sigterm() {
    use tokio::signal::unix::{signal, SignalKind};
    if let Ok(mut s) = signal(SignalKind::terminate()) {
        s.recv().await;
    }
}

#[cfg(not(unix))]
async fn wait_sigterm() {
    futures::future::pending::<()>().await;
}

async fn idle_until_signal() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = wait_sigterm() => {}
    }
}

/// Tiny `/metrics` + `/healthz` server. Avoids pulling in axum/actix to
/// keep the controller binary lean.
async fn serve_metrics(addr: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(addr).await?;
    tracing::info!(%addr, "metrics + healthz server listening");

    loop {
        let (mut sock, _peer) = listener.accept().await?;
        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req.split_whitespace().nth(1).unwrap_or("/");

            let body = if path.starts_with("/metrics") {
                let metric_families = prometheus::gather();
                let mut buf = Vec::new();
                let encoder = prometheus::TextEncoder::new();
                use prometheus::Encoder;
                encoder.encode(&metric_families, &mut buf).unwrap_or(());
                String::from_utf8(buf).unwrap_or_default()
            } else {
                "ok".to_string()
            };

            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body,
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        });
    }
}
