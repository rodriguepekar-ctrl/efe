//! efe-llm-router microservice entry point (M21).

use efe_llm_router::config::RouterConfig;
use std::net::SocketAddr;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,efe_llm_router=debug".into()),
        )
        .json()
        .init();

    let config = RouterConfig::from_env()
        .map_err(|e| anyhow::anyhow!("config: {}", e))?;

    tracing::info!(
        bind = %config.grpc_bind,
        providers = ?config.enabled_providers,
        "efe-llm-router starting"
    );

    // Wire dependencies (Postgres, Redis, providers, router).
    // Stub : production wires actual SDK clients per the architecture
    // doc § 5.4 startup ordering.

    let addr: SocketAddr = config
        .grpc_bind
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid grpc_bind: {}", e))?;
    tracing::info!(%addr, "router listening (M21 scaffold)");
    Ok(())
}
