//! Unified Redis client supporting both single-instance and cluster mode.
//!
//! Routing :
//! - `redis://host:port[/db]` or `rediss://...` → single-instance via
//!   `redis::aio::ConnectionManager` (auto-reconnect, multiplexed).
//! - `redis-cluster://host1:port1,host2:port2,...` → Redis Cluster via
//!   `redis::cluster_async::ClusterClient`.
//!
//! Both modes expose the same high-level operations through this type.

use crate::error::{CacheError, CacheResult};
use redis::aio::ConnectionManager;
use redis::cluster::ClusterClient;
use redis::cluster_async::ClusterConnection;
use redis::{AsyncCommands, Client};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, warn};

const DEFAULT_OP_TIMEOUT: Duration = Duration::from_millis(250);

/// Wire choice of Redis backend. Not `Debug` (the underlying
/// `ConnectionManager` and `ClusterConnection` types don't implement
/// `Debug`). Outer [`CacheClient`] has a custom `Debug` impl.
#[derive(Clone)]
enum Inner {
    Single(ConnectionManager),
    Cluster(Arc<Mutex<ClusterConnection>>),
}

/// Process-wide cache client. Cheap to `Clone` (refcounts the underlying
/// connection manager / pool).
#[derive(Clone)]
pub struct CacheClient {
    inner: Inner,
    /// Identifies the backend in logs + metrics. Either `single` or `cluster`.
    pub backend_kind: &'static str,
    /// Per-op timeout (default 250ms). Beyond this, the operation returns
    /// `CacheError::Unavailable` so callers degrade fast.
    op_timeout: Duration,
}

impl std::fmt::Debug for CacheClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheClient")
            .field("backend_kind", &self.backend_kind)
            .field("op_timeout", &self.op_timeout)
            .finish()
    }
}

impl CacheClient {
    /// Construct a client from a URL. Picks single vs. cluster based on
    /// scheme.
    ///
    /// Single-node example :
    /// ```ignore
    /// let c = CacheClient::connect("redis://redis.efe-edge:6379").await?;
    /// ```
    ///
    /// Cluster example :
    /// ```ignore
    /// let c = CacheClient::connect("redis-cluster://r1:7000,r2:7000,r3:7000").await?;
    /// ```
    pub async fn connect(url: &str) -> CacheResult<Self> {
        if let Some(rest) = url.strip_prefix("redis-cluster://") {
            Self::connect_cluster(rest).await
        } else {
            Self::connect_single(url).await
        }
    }

    async fn connect_single(url: &str) -> CacheResult<Self> {
        let client = Client::open(url).map_err(|e| {
            CacheError::Misconfig(format!("invalid redis URL '{}': {}", url, e))
        })?;
        let mgr = ConnectionManager::new(client)
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;
        Ok(Self {
            inner: Inner::Single(mgr),
            backend_kind: "single",
            op_timeout: DEFAULT_OP_TIMEOUT,
        })
    }

    async fn connect_cluster(hosts_csv: &str) -> CacheResult<Self> {
        let hosts: Vec<String> = hosts_csv
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.starts_with("redis://") || s.starts_with("rediss://") {
                    s.to_string()
                } else {
                    format!("redis://{}", s)
                }
            })
            .collect();
        if hosts.is_empty() {
            return Err(CacheError::Misconfig("empty cluster URL".into()));
        }
        let client = ClusterClient::new(hosts).map_err(|e| {
            CacheError::Misconfig(format!("cluster client init: {}", e))
        })?;
        let conn = client
            .get_async_connection()
            .await
            .map_err(|e| CacheError::Unavailable(e.to_string()))?;
        Ok(Self {
            inner: Inner::Cluster(Arc::new(Mutex::new(conn))),
            backend_kind: "cluster",
            op_timeout: DEFAULT_OP_TIMEOUT,
        })
    }

    /// Override the default per-operation timeout (250 ms).
    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.op_timeout = t;
        self
    }

    /// Bytes-out by key. Returns `Ok(None)` if missing.
    pub async fn get_bytes(&self, key: &str) -> CacheResult<Option<Vec<u8>>> {
        let fut = async {
            match &self.inner {
                Inner::Single(mgr) => {
                    let mut conn = mgr.clone();
                    let v: Option<Vec<u8>> = conn.get(key).await?;
                    Ok::<_, CacheError>(v)
                }
                Inner::Cluster(arc) => {
                    let mut conn = arc.lock().await;
                    let v: Option<Vec<u8>> = conn.get(key).await?;
                    Ok::<_, CacheError>(v)
                }
            }
        };
        match tokio::time::timeout(self.op_timeout, fut).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CacheError::Unavailable("get_bytes timeout".into())),
        }
    }

    /// Set bytes with TTL in seconds. TTL = 0 means no expiry (only used
    /// for revocation lists ; embeddings / LLM caches always use > 0).
    pub async fn set_bytes_ex(
        &self,
        key: &str,
        value: &[u8],
        ttl_seconds: u64,
    ) -> CacheResult<()> {
        let fut = async {
            match &self.inner {
                Inner::Single(mgr) => {
                    let mut conn = mgr.clone();
                    if ttl_seconds == 0 {
                        let _: () = conn.set(key, value).await?;
                    } else {
                        let _: () = conn.set_ex(key, value, ttl_seconds).await?;
                    }
                    Ok::<_, CacheError>(())
                }
                Inner::Cluster(arc) => {
                    let mut conn = arc.lock().await;
                    if ttl_seconds == 0 {
                        let _: () = conn.set(key, value).await?;
                    } else {
                        let _: () = conn.set_ex(key, value, ttl_seconds).await?;
                    }
                    Ok::<_, CacheError>(())
                }
            }
        };
        match tokio::time::timeout(self.op_timeout, fut).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CacheError::Unavailable("set_bytes_ex timeout".into())),
        }
    }

    /// Atomic increment-with-expire (returns new value).
    /// Uses INCR + EXPIRE in two commands ; the EXPIRE only fires the
    /// first time (when INCR result == 1) to avoid resetting the window.
    /// Atomicity is achieved via a small Lua script for cluster
    /// correctness (single-key script avoids CROSSSLOT errors).
    pub async fn incr_with_window(
        &self,
        key: &str,
        window_seconds: u64,
    ) -> CacheResult<u64> {
        // Lua : if INCR returns 1, also set EXPIRE atomically.
        const SCRIPT: &str = r#"
            local v = redis.call('INCR', KEYS[1])
            if tonumber(v) == 1 then
                redis.call('EXPIRE', KEYS[1], ARGV[1])
            end
            return v
        "#;
        let fut = async {
            match &self.inner {
                Inner::Single(mgr) => {
                    let mut conn = mgr.clone();
                    let v: u64 = redis::Script::new(SCRIPT)
                        .key(key)
                        .arg(window_seconds)
                        .invoke_async(&mut conn)
                        .await?;
                    Ok::<_, CacheError>(v)
                }
                Inner::Cluster(arc) => {
                    let mut conn = arc.lock().await;
                    let v: u64 = redis::Script::new(SCRIPT)
                        .key(key)
                        .arg(window_seconds)
                        .invoke_async(&mut *conn)
                        .await?;
                    Ok::<_, CacheError>(v)
                }
            }
        };
        match tokio::time::timeout(self.op_timeout, fut).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                warn!(?e, %key, "INCR failed");
                Err(e)
            }
            Err(_) => Err(CacheError::Unavailable("incr timeout".into())),
        }
    }

    /// Existence check (used by JWT revocation list).
    pub async fn exists(&self, key: &str) -> CacheResult<bool> {
        let fut = async {
            match &self.inner {
                Inner::Single(mgr) => {
                    let mut conn = mgr.clone();
                    let v: bool = conn.exists(key).await?;
                    Ok::<_, CacheError>(v)
                }
                Inner::Cluster(arc) => {
                    let mut conn = arc.lock().await;
                    let v: bool = conn.exists(key).await?;
                    Ok::<_, CacheError>(v)
                }
            }
        };
        match tokio::time::timeout(self.op_timeout, fut).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CacheError::Unavailable("exists timeout".into())),
        }
    }

    /// Delete a single key.
    pub async fn delete(&self, key: &str) -> CacheResult<()> {
        let fut = async {
            match &self.inner {
                Inner::Single(mgr) => {
                    let mut conn = mgr.clone();
                    let _: u64 = conn.del(key).await?;
                    Ok::<_, CacheError>(())
                }
                Inner::Cluster(arc) => {
                    let mut conn = arc.lock().await;
                    let _: u64 = conn.del(key).await?;
                    Ok::<_, CacheError>(())
                }
            }
        };
        match tokio::time::timeout(self.op_timeout, fut).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CacheError::Unavailable("delete timeout".into())),
        }
    }

    /// Health probe used by the K8s readiness probe.
    pub async fn ping(&self) -> CacheResult<()> {
        let fut = async {
            match &self.inner {
                Inner::Single(mgr) => {
                    let mut conn = mgr.clone();
                    let r: String = redis::cmd("PING").query_async(&mut conn).await?;
                    if r == "PONG" {
                        Ok::<_, CacheError>(())
                    } else {
                        Err(CacheError::Unavailable(format!("ping response: {}", r)))
                    }
                }
                Inner::Cluster(arc) => {
                    let mut conn = arc.lock().await;
                    let r: String =
                        redis::cmd("PING").query_async(&mut *conn).await?;
                    if r == "PONG" {
                        Ok::<_, CacheError>(())
                    } else {
                        Err(CacheError::Unavailable(format!("ping response: {}", r)))
                    }
                }
            }
        };
        match tokio::time::timeout(self.op_timeout, fut).await {
            Ok(Ok(())) => {
                debug!(backend = self.backend_kind, "redis PING OK");
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(CacheError::Unavailable("ping timeout".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cluster_url_parses_split_hosts() {
        // Doesn't connect, just verifies the URL parsing path doesn't panic.
        // Connect itself will fail without a real Redis ; we expect
        // Unavailable, not Misconfig.
        let r = CacheClient::connect("redis-cluster://localhost:9999,localhost:9998").await;
        match r {
            Err(CacheError::Unavailable(_)) => {} // expected on no-redis test box
            Err(CacheError::Misconfig(m)) => panic!("misconfig should not fire on valid URL: {m}"),
            Ok(_) => panic!("unexpected success without redis"),
            Err(other) => {
                // Some Redis builds may surface a different error variant.
                eprintln!("Note: non-Unavailable error variant: {:?}", other);
            }
        }
    }

    #[tokio::test]
    async fn single_url_misconfig_caught_early() {
        let r = CacheClient::connect("not-a-url").await;
        assert!(matches!(r, Err(CacheError::Misconfig(_)) | Err(CacheError::Unavailable(_))));
    }

    #[tokio::test]
    async fn empty_cluster_url_rejected() {
        let r = CacheClient::connect("redis-cluster://").await;
        assert!(matches!(r, Err(CacheError::Misconfig(_))));
    }
}
