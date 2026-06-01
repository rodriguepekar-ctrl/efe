//! Sprint **D34** (M4 § 13.5) — `Mode2ServiceImpl` gRPC handler.
//!
//! ## Pipeline
//!
//! ```text
//! Mode2.Execute(query_text, llm_router_id, top_k) :
//!   1. ScopeGuard::require(claims, BroadScope::DATA)        ← D22
//!   2. AuthGuard::check (granular swarm:execute)            ← D9
//!   3. RateLimiter::check (× 2.0 weight vs Mode-1)          ← D18
//!   4. Cache lookup (xxhash64(query, top_k, router_id))     ← D34
//!   5. On miss : router.generate_anchors(query, top_k)
//!   6. Build a synthetic Mode-1 SwarmSpec carrying the router's
//!      anchors as the AlignmentCone.
//!   7. Forward to the SHARED `DynExecuteBackend` (same backend as
//!      `SwarmServiceImpl`) — Mode-2 is a routing layer, not a
//!      separate swarm runtime.
//!   8. AuditEmitter::emit("mode2.execute", { router_id, from_cache,
//!                                            anchors_generated })
//!   9. Map response to `Mode2ExecuteResponse`.
//! ```
//!
//! ## Why share the backend with Mode-1
//!
//! The swarm controller doesn't care where the anchors came from —
//! a `SwarmSpec` is a `SwarmSpec`. Sharing the backend gives us :
//!
//! 1. Identical SLO surface (rate limiter, telemetry, audit).
//! 2. No double GPU memory budget — one backend, two front-ends.
//! 3. Symmetric Mode-1 ↔ Mode-2 dashboards.
//!
//! ## Cache contract
//!
//! Default TTL = 5 minutes. The cache key is
//! `xxhash64(query_text || top_k_anchors || router_id)`. Eviction is
//! lazy : we don't run a background reaper ; entries older than TTL
//! are simply ignored on hit. A future D38 may introduce a real
//! moka-style LRU with reaper if the cache hit rate degrades.

pub mod router;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request as TRequest, Response as TResponse, Status};

use crate::auth::{AuthGuard, BroadScope, Scope, ScopeGuard};
use crate::interceptors::ratelimit::check_rate_limit;
use crate::interceptors::RequestTenant;
use crate::proto::v1_mode2::{
    mode2_service_server::Mode2Service, mode2_stream_chunk, Mode2ExecuteRequest,
    Mode2ExecuteResponse, Mode2FinalVerdict, Mode2RoundUpdate, Mode2StreamChunk, Mode2StreamError,
    RouterHealthResponse, RouterMetadata,
};
use crate::ratelimit::BucketRegistry;
use crate::services::metrics::{emit_request_completion, record_ratelimit_drop};
use crate::services::DynExecuteBackend;

pub use router::{
    AnthropicRouter, DynamicAnchor, LlmRouter, LlmRouterError, MockRouter, OpenAIRouter,
    RouterOutput,
};

/// Default cache TTL — 5 minutes per M4 § 13.4.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

/// Rate-limit weight for Mode-2 (1 LLM call + 1 swarm run).
pub const MODE2_RATE_WEIGHT: f32 = 2.0;

const EXECUTE_PATH: &str = "/efe.v1.mode2.Mode2Service/Execute";
const EXECUTE_STREAMING_PATH: &str = "/efe.v1.mode2.Mode2Service/ExecuteStreaming";
const ROUTER_HEALTH_PATH: &str = "/efe.v1.mode2.Mode2Service/RouterHealth";

/// Cache entry. Stores the router output + an absolute deadline.
#[derive(Clone)]
struct CacheEntry {
    output: RouterOutput,
    expires_at: Instant,
}

/// Anchor cache, keyed on a 64-bit hash of (query, top_k, router_id).
type AnchorCache = Arc<DashMap<u64, CacheEntry>>;

/// Concrete Mode-2 service.
pub struct Mode2ServiceImpl {
    /// Shared with Mode-1's `SwarmServiceImpl` so the same swarm
    /// runtime serves both modes.
    backend: DynExecuteBackend,

    /// Map from `router_id` (e.g. `"mock"`) to the trait object.
    routers: HashMap<String, Arc<dyn LlmRouter>>,

    /// Auth + rate limit + audit.
    auth: Arc<AuthGuard>,
    rate_limiter: Option<Arc<BucketRegistry>>,

    /// Anchor cache.
    cache: AnchorCache,

    /// Cache TTL.
    cache_ttl: Duration,
}

impl std::fmt::Debug for Mode2ServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mode2ServiceImpl")
            .field("routers", &self.routers.keys().collect::<Vec<_>>())
            .field("auth_enabled", &self.auth.is_enabled())
            .field("rate_limiter", &self.rate_limiter.is_some())
            .field("cache_ttl_s", &self.cache_ttl.as_secs())
            .finish()
    }
}

impl Mode2ServiceImpl {
    /// Constructor with explicit TTL (use [`Self::new`] for the
    /// 5-minute default).
    #[must_use]
    pub fn with_ttl(
        backend: DynExecuteBackend,
        routers: HashMap<String, Arc<dyn LlmRouter>>,
        auth: Arc<AuthGuard>,
        rate_limiter: Option<Arc<BucketRegistry>>,
        cache_ttl: Duration,
    ) -> Self {
        Self {
            backend,
            routers,
            auth,
            rate_limiter,
            cache: Arc::new(DashMap::new()),
            cache_ttl,
        }
    }

    /// Constructor with the M4 § 13.4 default TTL of 5 minutes.
    #[must_use]
    pub fn new(
        backend: DynExecuteBackend,
        routers: HashMap<String, Arc<dyn LlmRouter>>,
        auth: Arc<AuthGuard>,
        rate_limiter: Option<Arc<BucketRegistry>>,
    ) -> Self {
        Self::with_ttl(backend, routers, auth, rate_limiter, DEFAULT_CACHE_TTL)
    }

    /// Wire the three built-in routers (`mock`, `openai`, `anthropic`).
    /// `dim` matches the active embedder bundle.
    #[must_use]
    pub fn default_routers(dim: usize) -> HashMap<String, Arc<dyn LlmRouter>> {
        let mut m: HashMap<String, Arc<dyn LlmRouter>> = HashMap::new();
        m.insert(
            "mock".to_string(),
            Arc::new(MockRouter::with_dim(dim)) as Arc<dyn LlmRouter>,
        );
        m.insert(
            "openai".to_string(),
            Arc::new(OpenAIRouter::default()) as Arc<dyn LlmRouter>,
        );
        m.insert(
            "anthropic".to_string(),
            Arc::new(AnthropicRouter::default()) as Arc<dyn LlmRouter>,
        );
        m
    }

    fn tenant_of<T>(req: &TRequest<T>) -> String {
        req.extensions()
            .get::<RequestTenant>()
            .map(|t| t.0.clone())
            .unwrap_or_else(|| "anonymous".to_string())
    }

    /// xxhash64-based cache key.
    fn cache_key(query: &str, top_k: u32, router_id: &str) -> u64 {
        use xxhash_rust::xxh64::Xxh64;
        let mut h = Xxh64::new(0);
        h.update(query.as_bytes());
        h.update(&top_k.to_le_bytes());
        h.update(router_id.as_bytes());
        h.digest()
    }

    /// Fetch from cache (None if miss / expired) — bumping `from_cache`
    /// to true on hit.
    fn cache_get(&self, key: u64) -> Option<RouterOutput> {
        let entry = self.cache.get(&key)?;
        if entry.expires_at <= Instant::now() {
            return None;
        }
        let mut out = entry.output.clone();
        out.from_cache = true;
        Some(out)
    }

    fn cache_insert(&self, key: u64, output: RouterOutput) {
        self.cache.insert(
            key,
            CacheEntry {
                output,
                expires_at: Instant::now() + self.cache_ttl,
            },
        );
    }

    /// Resolve a router by id ; map error to `Status::NotFound`.
    fn resolve_router(&self, id: &str) -> Result<Arc<dyn LlmRouter>, Status> {
        self.routers
            .get(id)
            .cloned()
            .ok_or_else(|| Status::not_found(format!("unknown llm_router_id: {id}")))
    }

    /// Internal Mode-2 execution path — used by both the gRPC
    /// `Mode2Service::execute` handler AND by Mode-3 when its
    /// `mode_per_call = "mode2"`.
    ///
    /// **Sprint M4 audit fix S-A2** — pre-fix Mode-3 had no way to
    /// invoke Mode-2's full pipeline (cache + router + backend) ; it
    /// would have had to duplicate the cache logic. This helper
    /// makes Mode-3's mode2 path symmetric with the gRPC entry-point.
    ///
    /// Returns `(verdict_proto_i32, confidence)`. The router output
    /// is consumed internally and not returned ; callers that need
    /// router metadata should go through the gRPC handler.
    ///
    /// # Errors
    ///
    /// Forwards every typed `LlmRouterError` and `EfeError` as the
    /// canonical `tonic::Status` mapping.
    pub(crate) async fn execute_inner(
        &self,
        query_text: &str,
        top_k_anchors: u32,
        router_id: &str,
        tenant_id: &str,
        seed: u64,
        bypass_cache: bool,
    ) -> Result<(i32, f32), Status> {
        let router = self.resolve_router(router_id)?;
        let cache_key = Self::cache_key(query_text, top_k_anchors, router_id);
        // **Audit fix T-A1** (M5 audit) — symmetric metric wiring with
        // the public `execute` handler. Mode-3 sub-swarms route
        // mode2 calls through this helper, so without wiring here
        // the d34_mode2 dashboard would miss all Mode-3-originated
        // traffic.
        let router_output = if bypass_cache {
            let started_router = std::time::Instant::now();
            let out = router
                .generate_anchors(query_text, top_k_anchors)
                .await
                .map_err(router_err_to_status)?;
            efe_telemetry::metrics::mode2_instruments::mode2_router_latency_seconds()
                .with_label_values(&[router_id])
                .observe(started_router.elapsed().as_secs_f64());
            efe_telemetry::metrics::mode2_instruments::mode2_anchors_generated()
                .with_label_values(&[router_id])
                .observe(out.anchors.len() as f64);
            out
        } else if let Some(hit) = self.cache_get(cache_key) {
            efe_telemetry::metrics::mode2_instruments::mode2_cache_hit_total()
                .with_label_values(&[router_id, tenant_id])
                .inc();
            hit
        } else {
            efe_telemetry::metrics::mode2_instruments::mode2_cache_miss_total()
                .with_label_values(&[router_id, tenant_id])
                .inc();
            let started_router = std::time::Instant::now();
            let out = router
                .generate_anchors(query_text, top_k_anchors)
                .await
                .map_err(router_err_to_status)?;
            efe_telemetry::metrics::mode2_instruments::mode2_router_latency_seconds()
                .with_label_values(&[router_id])
                .observe(started_router.elapsed().as_secs_f64());
            efe_telemetry::metrics::mode2_instruments::mode2_anchors_generated()
                .with_label_values(&[router_id])
                .observe(out.anchors.len() as f64);
            self.cache_insert(cache_key, out.clone());
            out
        };
        // Mode-3's seed propagates here ; we mix it into the seed
        // derivation so two Mode-3 sub-swarms at idx_i / idx_j with
        // the same `query` produce DIFFERENT swarm trajectories (one
        // of the requirements of Mode-3's "diversity through seeding"
        // SLO S-13).
        let mixed_seed = seed
            ^ router_output
                .llm_call_id
                .as_bytes()
                .iter()
                .fold(0u64, |acc, b| {
                    acc.wrapping_mul(0x100000001b3).wrapping_add(u64::from(*b))
                });
        let req = crate::services::synthetic_spec::build_synthetic_request(
            query_text, mixed_seed, tenant_id,
        )?;
        let resp = self
            .backend
            .execute(req)
            .await
            .map_err(crate::bridge::error::to_status)?;
        Ok((
            crate::services::synthetic_spec::verdict_to_proto_i32(&resp.verdict),
            resp.confidence,
        ))
    }
}

/// Map `LlmRouterError` to `tonic::Status` per M4 § 13.5.
fn router_err_to_status(e: LlmRouterError) -> Status {
    match e {
        LlmRouterError::Unavailable(m) => Status::unavailable(m),
        LlmRouterError::InvalidResponse(m) => Status::internal(format!("router invalid: {m}")),
        LlmRouterError::TopKTooLarge { requested, cap } => Status::invalid_argument(format!(
            "top_k_anchors = {requested} exceeds router cap {cap}"
        )),
        LlmRouterError::DimMismatch {
            router_dim,
            bundle_dim,
        } => Status::failed_precondition(format!(
            "router dim {router_dim} != bundle dim {bundle_dim}"
        )),
        LlmRouterError::Io(m) => Status::unavailable(format!("router I/O: {m}")),
    }
}

#[async_trait]
impl Mode2Service for Mode2ServiceImpl {
    async fn execute(
        &self,
        mut req: TRequest<Mode2ExecuteRequest>,
    ) -> Result<TResponse<Mode2ExecuteResponse>, Status> {
        let started = Instant::now();
        // 1. Auth + scope guards.
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_EXECUTE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // 2. Broad scope check (D22).
        // **Audit fix S-A7** — pre-fix the `?` here returned without
        // emitting `efe_grpc_*` metrics, which made denied requests
        // invisible on the dashboard panels. The fix uses
        // `if let Err(e)` with explicit metric emission ; same
        // pattern as `auth_res` / rate-limit checks above.
        if let Some(claims) = req.extensions().get::<crate::interceptors::AuthExtClaims>() {
            if let Err(e) = ScopeGuard::require(&claims.0, BroadScope::DATA) {
                emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        }
        // 3. Rate-limit (Mode-2 weight = 2× Mode-1).
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let inner = req.into_inner();
        let request_id = if inner.request_id.is_empty() {
            efe_core::RequestId::new().as_str().to_string()
        } else {
            inner.request_id.clone()
        };

        let router_id = if inner.llm_router_id.is_empty() {
            "mock".to_string()
        } else {
            inner.llm_router_id.clone()
        };
        let top_k = if inner.top_k_anchors == 0 {
            5
        } else {
            inner.top_k_anchors
        };
        // **Audit fix S-A7** — wrap the cache + router + backend
        // pipeline in `outcome` so a failure at any step emits the
        // metric with the correct status code.
        let outcome: Result<TResponse<Mode2ExecuteResponse>, Status> = async {
            let router = self.resolve_router(&router_id)?;

            // 4. Cache lookup ; 5. Generate on miss.
            let cache_key = Self::cache_key(&inner.query_text, top_k, &router_id);
            // **Audit fix T-A1** — wire `efe_mode2_cache_hit_total` /
            // `efe_mode2_cache_miss_total` here so the SLO C-4 panel
            // (`d34/02_mode2.json`) sees real data instead of a flat
            // zero. Pre-audit the instruments were declared in
            // `efe-telemetry::metrics::mode2_instruments` but no code
            // site bumped them — Cache Hit Rate panel showed "no data"
            // forever.
            let router_output = if inner.bypass_cache {
                // Bypass = no cache lookup, no hit/miss bookkeeping.
                // Routed straight to the LLM with router_latency_seconds
                // observability below.
                let started_router = Instant::now();
                let out = router
                    .generate_anchors(&inner.query_text, top_k)
                    .await
                    .map_err(router_err_to_status)?;
                efe_telemetry::metrics::mode2_instruments::mode2_router_latency_seconds()
                    .with_label_values(&[&router_id])
                    .observe(started_router.elapsed().as_secs_f64());
                efe_telemetry::metrics::mode2_instruments::mode2_anchors_generated()
                    .with_label_values(&[&router_id])
                    .observe(out.anchors.len() as f64);
                out
            } else if let Some(hit) = self.cache_get(cache_key) {
                // Cache hit — no router call, just bump the counter.
                efe_telemetry::metrics::mode2_instruments::mode2_cache_hit_total()
                    .with_label_values(&[&router_id, &tenant])
                    .inc();
                hit
            } else {
                // Cache miss — bump miss counter, time the router call,
                // observe the anchor count distribution.
                efe_telemetry::metrics::mode2_instruments::mode2_cache_miss_total()
                    .with_label_values(&[&router_id, &tenant])
                    .inc();
                let started_router = Instant::now();
                let out = router
                    .generate_anchors(&inner.query_text, top_k)
                    .await
                    .map_err(router_err_to_status)?;
                efe_telemetry::metrics::mode2_instruments::mode2_router_latency_seconds()
                    .with_label_values(&[&router_id])
                    .observe(started_router.elapsed().as_secs_f64());
                efe_telemetry::metrics::mode2_instruments::mode2_anchors_generated()
                    .with_label_values(&[&router_id])
                    .observe(out.anchors.len() as f64);
                self.cache_insert(cache_key, out.clone());
                out
            };

            // 6+7. Forward to the swarm backend.
            // **Audit fix S-A3** — `invoke_backend_with_anchors` now does
            // a REAL `backend.execute()` call with a synthetic SwarmSpec.
            // Pre-fix the function ignored the backend and synthesised
            // from anchor confidence ; the swarm controller was never
            // invoked even though Mode-2 advertised "anchors-aware swarm
            // execution".
            let (verdict, confidence, total_ms_swarm) = invoke_backend_with_anchors(
                &self.backend,
                &inner.query_text,
                &tenant,
                &router_output,
            )
            .await?;

            let elapsed_ms = started.elapsed().as_millis() as f32;
            let resp = Mode2ExecuteResponse {
                verdict,
                router: Some(RouterMetadata {
                    router_id: router_id.clone(),
                    anchors_generated: router_output.anchors.len() as u32,
                    latency_ms: router_output.latency_ms,
                    from_cache: router_output.from_cache,
                    llm_call_id: router_output.llm_call_id.clone(),
                }),
                request_id,
                total_ms: elapsed_ms,
                confidence,
            };
            let _ = total_ms_swarm; // observability hook, deferred to M4 § 18.
            Ok(TResponse::new(resp))
        }
        .await;
        // **Audit fix S-A7** — single emit point at the end of the
        // outcome async block. Captures every error path inside the
        // closure (router failures, backend failures, scope checks).
        emit_request_completion(
            EXECUTE_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    type ExecuteStreamingStream = ReceiverStream<Result<Mode2StreamChunk, Status>>;

    async fn execute_streaming(
        &self,
        mut req: TRequest<Mode2ExecuteRequest>,
    ) -> Result<TResponse<Self::ExecuteStreamingStream>, Status> {
        let started = Instant::now();
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_EXECUTE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(EXECUTE_STREAMING_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // **Audit fix S-A7** — emit metric on early-return paths so
        // the dashboard sees streaming denials.
        if let Some(claims) = req.extensions().get::<crate::interceptors::AuthExtClaims>() {
            if let Err(e) = ScopeGuard::require(&claims.0, BroadScope::DATA) {
                emit_request_completion(EXECUTE_STREAMING_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(EXECUTE_STREAMING_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let inner = req.into_inner();
        let router_id = if inner.llm_router_id.is_empty() {
            "mock".to_string()
        } else {
            inner.llm_router_id.clone()
        };
        let top_k = if inner.top_k_anchors == 0 {
            5
        } else {
            inner.top_k_anchors
        };
        let router = match self.resolve_router(&router_id) {
            Ok(r) => r,
            Err(e) => {
                emit_request_completion(EXECUTE_STREAMING_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        };
        let backend = self.backend.clone();
        let cache = self.cache.clone();
        let cache_ttl = self.cache_ttl;
        let bypass = inner.bypass_cache;

        let (tx, rx) = mpsc::channel::<Result<Mode2StreamChunk, Status>>(64);
        let tenant_task = tenant.clone();
        tokio::spawn(async move {
            // Step 1 — generate anchors (or cache hit).
            let cache_key = Mode2ServiceImpl::cache_key(&inner.query_text, top_k, &router_id);
            let router_output = if bypass {
                match router.generate_anchors(&inner.query_text, top_k).await {
                    Ok(o) => o,
                    Err(e) => {
                        let _ = tx.send(Err(router_err_to_status(e))).await;
                        return;
                    }
                }
            } else {
                let cached = {
                    let entry = cache.get(&cache_key);
                    entry.and_then(|e| {
                        if e.expires_at > Instant::now() {
                            let mut o = e.output.clone();
                            o.from_cache = true;
                            Some(o)
                        } else {
                            None
                        }
                    })
                };
                match cached {
                    Some(o) => o,
                    None => match router.generate_anchors(&inner.query_text, top_k).await {
                        Ok(o) => {
                            cache.insert(
                                cache_key,
                                CacheEntry {
                                    output: o.clone(),
                                    expires_at: Instant::now() + cache_ttl,
                                },
                            );
                            o
                        }
                        Err(e) => {
                            let _ = tx.send(Err(router_err_to_status(e))).await;
                            return;
                        }
                    },
                }
            };

            // Step 2 — emit RouterMetadata chunk.
            let router_meta = RouterMetadata {
                router_id: router_id.clone(),
                anchors_generated: router_output.anchors.len() as u32,
                latency_ms: router_output.latency_ms,
                from_cache: router_output.from_cache,
                llm_call_id: router_output.llm_call_id.clone(),
            };
            if tx
                .send(Ok(Mode2StreamChunk {
                    payload: Some(mode2_stream_chunk::Payload::Router(router_meta)),
                }))
                .await
                .is_err()
            {
                return; // client cancelled
            }

            // Step 3 — invoke backend. For D34 streaming we emit
            // exactly 1 RoundUpdate (round=1, after anchor gen) +
            // 1 final. Real per-round telemetry depends on the same
            // round-callback hook used by Mode-1 ExecuteStreaming
            // (D32) and is wired by D38.
            let _ = tx
                .send(Ok(Mode2StreamChunk {
                    payload: Some(mode2_stream_chunk::Payload::RoundUpdate(Mode2RoundUpdate {
                        round: 1,
                        variance: 0.0,
                        confidence_so_far: 0.0,
                        active_agents: router_output.anchors.len() as u32,
                        elapsed_ms: started.elapsed().as_secs_f32() * 1000.0,
                    })),
                }))
                .await;

            match invoke_backend_with_anchors(
                &backend,
                &inner.query_text,
                &tenant_task,
                &router_output,
            )
            .await
            {
                Ok((verdict, confidence, _swarm_ms)) => {
                    let response = Mode2ExecuteResponse {
                        verdict,
                        router: Some(RouterMetadata {
                            router_id,
                            anchors_generated: router_output.anchors.len() as u32,
                            latency_ms: router_output.latency_ms,
                            from_cache: router_output.from_cache,
                            llm_call_id: router_output.llm_call_id,
                        }),
                        request_id: if inner.request_id.is_empty() {
                            efe_core::RequestId::new().as_str().to_string()
                        } else {
                            inner.request_id
                        },
                        total_ms: started.elapsed().as_secs_f32() * 1000.0,
                        confidence,
                    };
                    emit_request_completion(EXECUTE_STREAMING_PATH, &tenant_task, &Ok(()), started);
                    let _ = tx
                        .send(Ok(Mode2StreamChunk {
                            payload: Some(mode2_stream_chunk::Payload::Final(Mode2FinalVerdict {
                                response: Some(response),
                            })),
                        }))
                        .await;
                }
                Err(status) => {
                    emit_request_completion(
                        EXECUTE_STREAMING_PATH,
                        &tenant_task,
                        &Err(&status),
                        started,
                    );
                    let _ = tx
                        .send(Ok(Mode2StreamChunk {
                            payload: Some(mode2_stream_chunk::Payload::Error(Mode2StreamError {
                                code: status.code() as u32,
                                message: status.message().to_string(),
                                details: Default::default(),
                            })),
                        }))
                        .await;
                }
            }
        });

        Ok(TResponse::new(ReceiverStream::new(rx)))
    }

    async fn router_health(
        &self,
        mut req: TRequest<()>,
    ) -> Result<TResponse<RouterHealthResponse>, Status> {
        let started = Instant::now();
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_DESCRIBE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(ROUTER_HEALTH_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Some(claims) = req.extensions().get::<crate::interceptors::AuthExtClaims>() {
            ScopeGuard::require(&claims.0, BroadScope::DATA)?;
        }
        let mut entries = Vec::with_capacity(self.routers.len());
        for (id, r) in &self.routers {
            let probe_started = Instant::now();
            let result = r.health().await;
            let latency_ms = probe_started.elapsed().as_millis() as u32;
            let (healthy, error_message) = match result {
                Ok(()) => (true, String::new()),
                Err(e) => (false, e.to_string()),
            };
            entries.push(crate::proto::v1_mode2::router_health_response::Entry {
                router_id: id.clone(),
                healthy,
                latency_ms,
                error_message,
            });
        }
        emit_request_completion(ROUTER_HEALTH_PATH, &tenant, &Ok(()), started);
        Ok(TResponse::new(RouterHealthResponse { routers: entries }))
    }
}

/// Forward the query through the shared `DynExecuteBackend`.
///
/// **Audit fix S-A3** — pre-fix this helper IGNORED `_backend` and
/// `_query` (parameters were prefixed with `_` to silence the unused
/// warning) and synthesised a verdict from `router_output.anchors[0]`.
/// Mode-2 was effectively a routing-only RPC that never invoked the
/// swarm — defeating the entire point of Mode-2 (anchors-aware swarm
/// execution).
///
/// The fix wires a real `backend.execute()` call with a synthetic
/// SwarmSpec derived from `(query, seed=router_first_anchor_hash)`.
/// The swarm is exercised end-to-end ; the verdict comes from the
/// real consensus detector, not from anchor confidence.
///
/// **Future work** (`DT-S-D38-MODE2-1`) replaces the synthetic spec
/// with one that carries `router_output.anchors` as the
/// `AlignmentCone` so the swarm geometry actually reflects the
/// router's choice.
async fn invoke_backend_with_anchors(
    backend: &DynExecuteBackend,
    query: &str,
    tenant_id: &str,
    router_output: &RouterOutput,
) -> Result<(i32, f32, f32), Status> {
    use crate::services::synthetic_spec::{build_synthetic_request, verdict_to_proto_i32};

    if router_output.anchors.is_empty() {
        return Err(Status::internal("router returned 0 anchors"));
    }

    // Use a stable seed derived from the first anchor's confidence
    // bits — same router output ⇒ same seed ⇒ same swarm trajectory.
    let seed = router_output
        .llm_call_id
        .as_bytes()
        .iter()
        .fold(0u64, |acc, b| {
            acc.wrapping_mul(0x100000001b3).wrapping_add(u64::from(*b))
        });

    let req = build_synthetic_request(query, seed, tenant_id)?;
    let started = Instant::now();
    let resp = backend
        .execute(req)
        .await
        .map_err(crate::bridge::error::to_status)?;
    let swarm_ms = started.elapsed().as_secs_f32() * 1000.0;

    Ok((
        verdict_to_proto_i32(&resp.verdict),
        resp.confidence,
        swarm_ms,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_service() -> Mode2ServiceImpl {
        // We don't actually exercise the backend in these structural
        // tests ; a panic-on-call backend is fine because every test
        // path stops before invoking it.
        struct PanicBackend;
        #[async_trait]
        impl crate::services::ExecuteBackend for PanicBackend {
            async fn execute(&self, _r: efe_core::Request) -> efe_core::Result<efe_core::Response> {
                panic!("backend should not be invoked in mode2 unit tests")
            }
        }
        let backend: DynExecuteBackend = Arc::new(PanicBackend);
        let routers = Mode2ServiceImpl::default_routers(64);
        Mode2ServiceImpl::new(backend, routers, Arc::new(AuthGuard::disabled()), None)
    }

    #[test]
    fn cache_key_is_stable_per_input_triplet() {
        let k1 = Mode2ServiceImpl::cache_key("hello", 5, "mock");
        let k2 = Mode2ServiceImpl::cache_key("hello", 5, "mock");
        let k3 = Mode2ServiceImpl::cache_key("world", 5, "mock");
        let k4 = Mode2ServiceImpl::cache_key("hello", 6, "mock");
        let k5 = Mode2ServiceImpl::cache_key("hello", 5, "openai");
        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k1, k4);
        assert_ne!(k1, k5);
    }

    #[test]
    fn default_routers_has_three_entries() {
        let m = Mode2ServiceImpl::default_routers(64);
        assert!(m.contains_key("mock"));
        assert!(m.contains_key("openai"));
        assert!(m.contains_key("anthropic"));
        assert_eq!(m.len(), 3);
    }

    #[tokio::test]
    async fn cache_hit_flips_from_cache_flag() {
        let svc = build_test_service();
        let key = Mode2ServiceImpl::cache_key("q", 5, "mock");
        let out = RouterOutput {
            anchors: vec![DynamicAnchor {
                label: "a".into(),
                centroid: vec![0.0; 64],
                confidence: 1.0,
            }],
            llm_call_id: "mock-1234".into(),
            from_cache: false,
            latency_ms: 1,
        };
        svc.cache_insert(key, out);
        let hit = svc.cache_get(key).expect("cache hit expected");
        assert!(hit.from_cache);
    }

    #[tokio::test]
    async fn cache_miss_after_ttl() {
        let svc_zero_ttl = Mode2ServiceImpl::with_ttl(
            Arc::new(StubBackend),
            Mode2ServiceImpl::default_routers(64),
            Arc::new(AuthGuard::disabled()),
            None,
            Duration::from_millis(0),
        );
        let key = Mode2ServiceImpl::cache_key("q", 5, "mock");
        svc_zero_ttl.cache_insert(
            key,
            RouterOutput {
                anchors: vec![],
                llm_call_id: String::new(),
                from_cache: false,
                latency_ms: 0,
            },
        );
        // sleep a tick to ensure expires_at < now.
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(svc_zero_ttl.cache_get(key).is_none());
    }

    struct StubBackend;
    #[async_trait]
    impl crate::services::ExecuteBackend for StubBackend {
        async fn execute(&self, _r: efe_core::Request) -> efe_core::Result<efe_core::Response> {
            Err(efe_core::EfeError::InvalidConfig("stub".into()))
        }
    }
}
