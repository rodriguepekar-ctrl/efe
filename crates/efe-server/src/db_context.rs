//! M27 — Postgres connection context with read/write split.
//!
//! Builds two `sqlx::PgPool` instances :
//! - `pool_rw` : writes + read-your-own-write (primary).
//! - `pool_ro` : analytical reads + non-mutating SELECTs (replica).
//!
//! Both go through PgBouncer in production (transaction pooling).
//!
//! Routing table : see `docs/EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md` § 10.4.
//!
//! Configuration via env :
//! - `EFE_DATABASE_URL` — primary (write) DSN. Required.
//! - `EFE_DATABASE_RO_URL` — replica (read-only) DSN. Optional ; if
//!   absent, `pool_ro` mirrors `pool_rw` (single-instance fallback).
//! - `EFE_FALLBACK_RO_TO_RW` — if `true`, force-route ro queries to
//!   primary (used during high replica lag, see RB_M27_REPLICA_LAG).

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Combined read/write connection context for efe-server.
#[derive(Clone, Debug)]
pub struct DbContext {
    pub pool_rw: PgPool,
    pub pool_ro: PgPool,
    /// When true, every `ro_pool()` call returns `pool_rw` (runtime
    /// fallback). Toggled by configmap at runtime via the
    /// `EFE_FALLBACK_RO_TO_RW` env reload.
    pub fallback_to_rw: Arc<AtomicBool>,
}

#[derive(Debug, Clone)]
pub struct DbContextConfig {
    pub rw_url: String,
    pub ro_url: Option<String>,
    pub max_connections_rw: u32,
    pub max_connections_ro: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_lifetime: Duration,
    pub fallback_to_rw: bool,
}

impl DbContextConfig {
    /// Build from env. `EFE_DATABASE_URL` required.
    pub fn from_env() -> anyhow::Result<Self> {
        let rw_url = std::env::var("EFE_DATABASE_URL")
            .map_err(|_| anyhow::anyhow!("EFE_DATABASE_URL not set (required for DbContext)"))?;
        let ro_url = std::env::var("EFE_DATABASE_RO_URL").ok();
        let max_rw = std::env::var("EFE_DATABASE_MAX_CONN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(20);
        let max_ro = std::env::var("EFE_DATABASE_RO_MAX_CONN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(50);
        let fallback = matches!(
            std::env::var("EFE_FALLBACK_RO_TO_RW").as_deref(),
            Ok("true") | Ok("1") | Ok("yes")
        );
        Ok(Self {
            rw_url,
            ro_url,
            max_connections_rw: max_rw,
            max_connections_ro: max_ro,
            acquire_timeout: Duration::from_secs(10),
            idle_timeout: Duration::from_secs(300),
            max_lifetime: Duration::from_secs(1800),
            fallback_to_rw: fallback,
        })
    }
}

impl DbContext {
    /// Build a context from a config. Connects both pools eagerly with a
    /// single ping each — fails fast on misconfig.
    pub async fn connect(cfg: DbContextConfig) -> anyhow::Result<Self> {
        let pool_rw = build_pool(&cfg.rw_url, cfg.max_connections_rw, &cfg).await?;
        info!(
            max = cfg.max_connections_rw,
            "DbContext : pool_rw connected"
        );
        let pool_ro = if let Some(url) = cfg.ro_url.as_deref() {
            match build_pool(url, cfg.max_connections_ro, &cfg).await {
                Ok(p) => {
                    info!(
                        max = cfg.max_connections_ro,
                        "DbContext : pool_ro connected (read replica)"
                    );
                    p
                }
                Err(e) => {
                    warn!(
                        ?e,
                        "DbContext : pool_ro connect failed, falling back to pool_rw"
                    );
                    pool_rw.clone()
                }
            }
        } else {
            info!("DbContext : EFE_DATABASE_RO_URL not set, pool_ro = pool_rw");
            pool_rw.clone()
        };
        Ok(Self {
            pool_rw,
            pool_ro,
            fallback_to_rw: Arc::new(AtomicBool::new(cfg.fallback_to_rw)),
        })
    }

    /// Return the write pool (always primary).
    pub fn rw(&self) -> &PgPool {
        &self.pool_rw
    }

    /// Return the read pool — primary if fallback is on, replica otherwise.
    pub fn ro(&self) -> &PgPool {
        if self.fallback_to_rw.load(Ordering::Relaxed) {
            &self.pool_rw
        } else {
            &self.pool_ro
        }
    }

    /// Runtime toggle. Set via reloaded configmap.
    pub fn set_fallback_to_rw(&self, on: bool) {
        self.fallback_to_rw.store(on, Ordering::SeqCst);
        warn!(on, "DbContext : ro fallback to rw toggled");
    }
}

async fn build_pool(url: &str, max: u32, cfg: &DbContextConfig) -> anyhow::Result<PgPool> {
    let opts = PgConnectOptions::from_str(url)
        .map_err(|e| anyhow::anyhow!("invalid PG URL : {}", e))?
        // M27 PgBouncer transaction pooling : disable prepared statement
        // cache so it's safe across recycled server connections.
        .statement_cache_capacity(0);
    let pool = PgPoolOptions::new()
        .max_connections(max)
        .min_connections(2)
        .acquire_timeout(cfg.acquire_timeout)
        .idle_timeout(Some(cfg.idle_timeout))
        .max_lifetime(Some(cfg.max_lifetime))
        .test_before_acquire(true)
        .connect_with(opts)
        .await
        .map_err(|e| anyhow::anyhow!("PG connect failed : {}", e))?;
    // Single ping to fail fast.
    sqlx::query("SELECT 1")
        .execute(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("PG ping failed : {}", e))?;
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // env vars are process-global ; serialize tests that mutate them.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn config_from_env_behaviour() {
        let _g = ENV_LOCK.lock().expect("env mutex");

        // ── 1. Without EFE_DATABASE_URL → error ──
        std::env::remove_var("EFE_DATABASE_URL");
        std::env::remove_var("EFE_DATABASE_RO_URL");
        std::env::remove_var("EFE_DATABASE_MAX_CONN");
        std::env::remove_var("EFE_FALLBACK_RO_TO_RW");
        let r = DbContextConfig::from_env();
        assert!(r.is_err(), "missing EFE_DATABASE_URL should error");

        // ── 2. With EFE_DATABASE_URL set → defaults ──
        std::env::set_var("EFE_DATABASE_URL", "postgres://test@x/y");
        let c = DbContextConfig::from_env().expect("config should parse");
        assert_eq!(c.max_connections_rw, 20);
        assert_eq!(c.max_connections_ro, 50);
        assert!(!c.fallback_to_rw);

        // ── 3. Override max conn ──
        std::env::set_var("EFE_DATABASE_MAX_CONN", "100");
        let c2 = DbContextConfig::from_env().expect("config should parse");
        assert_eq!(c2.max_connections_rw, 100);

        // ── 4. Fallback flag honored ──
        std::env::set_var("EFE_FALLBACK_RO_TO_RW", "true");
        let c3 = DbContextConfig::from_env().expect("config should parse");
        assert!(c3.fallback_to_rw);

        // cleanup
        std::env::remove_var("EFE_DATABASE_URL");
        std::env::remove_var("EFE_DATABASE_MAX_CONN");
        std::env::remove_var("EFE_FALLBACK_RO_TO_RW");
    }
}
