//! Per-tenant per-endpoint quota enforcement using Redis counters.
//!
//! Reads `tenant_feature` row (cached locally for 30s with
//! stale-while-revalidate) to know each tenant's limits ; increments
//! a Redis counter ; returns 429 if over.
//!
//! Fail-open : if Redis or PG is unavailable, allow the request through.
//! The local backpressure middleware is the safety net.
//!
//! Configuration via env :
//! - `EFE_TENANT_QUOTA_ENABLED` (default `true`).
//! - `EFE_TENANT_QUOTA_CACHE_TTL_S` (default `30`).

use efe_cache::{CacheClient, QuotaCounters};
use http::{Request, Response, StatusCode};
use sqlx::PgPool;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tower::{Layer, Service};
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct TenantQuotaConfig {
    pub enabled: bool,
    pub cache_ttl: Duration,
}

impl Default for TenantQuotaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_ttl: Duration::from_secs(30),
        }
    }
}

impl TenantQuotaConfig {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(v) = std::env::var("EFE_TENANT_QUOTA_ENABLED") {
            cfg.enabled = matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes");
        }
        if let Ok(v) = std::env::var("EFE_TENANT_QUOTA_CACHE_TTL_S") {
            if let Ok(s) = v.parse::<u64>() {
                cfg.cache_ttl = Duration::from_secs(s);
            }
        }
        cfg
    }
}

/// Limits loaded from `tenant_feature` row.
#[derive(Debug, Clone, Copy)]
pub struct TenantLimits {
    pub classify_per_min: u64,
    pub agent_run_per_hour: u64,
    pub embed_per_min: u64,
    pub search_per_min: u64,
    pub llm_tokens_per_day: u64,
}

impl Default for TenantLimits {
    fn default() -> Self {
        Self {
            classify_per_min: 100,
            agent_run_per_hour: 0,
            embed_per_min: 100,
            search_per_min: 100,
            llm_tokens_per_day: 0,
        }
    }
}

#[derive(Debug)]
struct CachedEntry {
    limits: TenantLimits,
    cached_at: Instant,
}

/// Process-local cache of TenantLimits with TTL.
struct LimitsCache {
    pg: PgPool,
    inner: RwLock<HashMap<String, CachedEntry>>,
    ttl: Duration,
}

impl LimitsCache {
    fn new(pg: PgPool, ttl: Duration) -> Self {
        Self {
            pg,
            inner: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// Returns cached limits if fresh, else queries PG.
    /// On PG error, returns last-known limits (stale) or Default.
    async fn limits_for(&self, tenant: &str) -> TenantLimits {
        // Fast path : read lock.
        {
            let guard = self.inner.read().await;
            if let Some(entry) = guard.get(tenant) {
                if entry.cached_at.elapsed() < self.ttl {
                    return entry.limits;
                }
            }
        }
        // Slow path : query PG.
        match self.query_pg(tenant).await {
            Ok(l) => {
                let mut guard = self.inner.write().await;
                guard.insert(
                    tenant.to_string(),
                    CachedEntry {
                        limits: l,
                        cached_at: Instant::now(),
                    },
                );
                l
            }
            Err(e) => {
                warn!(tenant = %tenant, ?e, "tenant_feature query failed, using cached/default");
                // Try stale entry first, else default.
                let guard = self.inner.read().await;
                guard.get(tenant).map(|e| e.limits).unwrap_or_default()
            }
        }
    }

    async fn query_pg(&self, tenant: &str) -> Result<TenantLimits, sqlx::Error> {
        let row = sqlx::query_as::<_, TenantLimitsRow>(
            r#"SELECT
                  classify_quota_per_min,
                  agent_run_quota_per_hour,
                  embed_quota_per_min,
                  search_quota_per_min,
                  llm_tokens_quota_per_day
               FROM tenant_feature
               WHERE id = $1"#,
        )
        .bind(tenant)
        .fetch_optional(&self.pg)
        .await?;
        Ok(row.map(Into::into).unwrap_or_default())
    }
}

#[derive(sqlx::FromRow)]
struct TenantLimitsRow {
    classify_quota_per_min: i32,
    agent_run_quota_per_hour: i32,
    embed_quota_per_min: i32,
    search_quota_per_min: i32,
    llm_tokens_quota_per_day: i64,
}

impl From<TenantLimitsRow> for TenantLimits {
    fn from(r: TenantLimitsRow) -> Self {
        Self {
            classify_per_min: r.classify_quota_per_min.max(0) as u64,
            agent_run_per_hour: r.agent_run_quota_per_hour.max(0) as u64,
            embed_per_min: r.embed_quota_per_min.max(0) as u64,
            search_per_min: r.search_quota_per_min.max(0) as u64,
            llm_tokens_per_day: r.llm_tokens_quota_per_day.max(0) as u64,
        }
    }
}

/// Bucket the request URI to a coarse endpoint label.
fn endpoint_for(path: &str) -> Option<&'static str> {
    if path.starts_with("/v1/classify") || path.starts_with("/efe.v1.SwarmService") {
        Some("classify")
    } else if path.starts_with("/v1/agent/runs") {
        Some("agent_run")
    } else if path.starts_with("/v1/embed") || path.starts_with("/v1/embeddings") {
        Some("embed")
    } else if path.starts_with("/v1/search") {
        Some("search")
    } else {
        None
    }
}

#[derive(Clone)]
pub struct TenantQuotaLayer {
    cfg: TenantQuotaConfig,
    counters: QuotaCounters,
    cache: Arc<LimitsCache>,
}

impl TenantQuotaLayer {
    pub fn new(cfg: TenantQuotaConfig, cache_client: CacheClient, pg: PgPool) -> Self {
        let counters = QuotaCounters::new(cache_client);
        let cache = Arc::new(LimitsCache::new(pg, cfg.cache_ttl));
        Self {
            cfg,
            counters,
            cache,
        }
    }
}

impl<S> Layer<S> for TenantQuotaLayer {
    type Service = TenantQuotaService<S>;
    fn layer(&self, inner: S) -> Self::Service {
        TenantQuotaService {
            inner,
            cfg: self.cfg.clone(),
            counters: self.counters.clone(),
            cache: self.cache.clone(),
        }
    }
}

#[derive(Clone)]
pub struct TenantQuotaService<S> {
    inner: S,
    cfg: TenantQuotaConfig,
    counters: QuotaCounters,
    cache: Arc<LimitsCache>,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for TenantQuotaService<S>
where
    S: Service<Request<ReqBody>, Response = Response<ResBody>> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Default + Send + 'static,
{
    type Response = Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let cfg = self.cfg.clone();
        let counters = self.counters.clone();
        let cache = self.cache.clone();
        let mut inner = self.inner.clone();
        std::mem::swap(&mut self.inner, &mut inner);

        Box::pin(async move {
            if !cfg.enabled {
                return inner.call(req).await;
            }

            let tenant = req
                .headers()
                .get("x-tenant-id")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let path = req.uri().path().to_string();
            let endpoint = endpoint_for(&path);

            // No tenant header or unbucketed path → skip quota.
            let (tenant, endpoint) = match (tenant, endpoint) {
                (Some(t), Some(e)) => (t, e),
                _ => {
                    return inner.call(req).await;
                }
            };

            let limits = cache.limits_for(&tenant).await;
            let limit = match endpoint {
                "classify" => limits.classify_per_min,
                "agent_run" => limits.agent_run_per_hour, // approx : minute counter; semantic difference noted
                "embed" => limits.embed_per_min,
                "search" => limits.search_per_min,
                _ => u64::MAX,
            };
            if limit == 0 {
                // Limit of 0 means feature disabled for this tenant.
                debug!(tenant = %tenant, endpoint, "tenant quota = 0, blocking");
                return Ok(build_429(60));
            }

            let check = match counters.check_per_minute(&tenant, endpoint, limit).await {
                Ok(c) => c,
                Err(e) => {
                    warn!(tenant = %tenant, endpoint, ?e, "quota check failed (fail-open)");
                    return inner.call(req).await;
                }
            };
            use efe_cache::quota::QuotaCheck;
            match check {
                QuotaCheck::Allow { .. } | QuotaCheck::Unknown => inner.call(req).await,
                QuotaCheck::Reject {
                    retry_after_seconds,
                    ..
                } => Ok(build_429(retry_after_seconds)),
            }
        })
    }
}

fn build_429<B: Default>(retry_after_s: u64) -> Response<B> {
    let mut resp = Response::new(B::default());
    *resp.status_mut() = StatusCode::TOO_MANY_REQUESTS;
    resp.headers_mut().insert(
        http::header::RETRY_AFTER,
        http::HeaderValue::from(retry_after_s.min(86_400) as u32),
    );
    resp.headers_mut().insert(
        http::header::CONTENT_TYPE,
        http::HeaderValue::from_static("application/json"),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_buckets() {
        assert_eq!(endpoint_for("/v1/classify"), Some("classify"));
        assert_eq!(endpoint_for("/v1/classify/batch"), Some("classify"));
        assert_eq!(endpoint_for("/v1/agent/runs"), Some("agent_run"));
        assert_eq!(endpoint_for("/v1/agent/runs/123/steps"), Some("agent_run"));
        assert_eq!(endpoint_for("/v1/embeddings"), Some("embed"));
        assert_eq!(endpoint_for("/v1/embed"), Some("embed"));
        assert_eq!(endpoint_for("/v1/search/vector"), Some("search"));
        assert_eq!(endpoint_for("/healthz"), None);
        assert_eq!(endpoint_for("/v1/dpia/status"), None);
        assert_eq!(
            endpoint_for("/efe.v1.SwarmService/Classify"),
            Some("classify")
        );
    }

    #[test]
    fn defaults_safe() {
        let l = TenantLimits::default();
        assert_eq!(l.classify_per_min, 100);
        assert_eq!(l.agent_run_per_hour, 0); // OFF by default
        assert_eq!(l.llm_tokens_per_day, 0); // OFF by default
    }

    #[test]
    fn limits_row_negative_clamps_to_zero() {
        let r = TenantLimitsRow {
            classify_quota_per_min: -5,
            agent_run_quota_per_hour: 10,
            embed_quota_per_min: 0,
            search_quota_per_min: 100,
            llm_tokens_quota_per_day: -1,
        };
        let l: TenantLimits = r.into();
        assert_eq!(l.classify_per_min, 0);
        assert_eq!(l.llm_tokens_per_day, 0);
        assert_eq!(l.agent_run_per_hour, 10);
    }
}
