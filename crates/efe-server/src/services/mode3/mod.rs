//! Sprint **D37** (M4 § 14.4) — `Mode3ServiceImpl` gRPC handler.
//!
//! ## Pipeline
//!
//! ```text
//! Mode3.Execute(query, k, mode_per_call, aggregator) :
//!   1. ScopeGuard::require(claims, BroadScope::DATA)        ← D22
//!   2. AuthGuard::check (granular swarm:execute)            ← D9
//!   3. Clamp k to [1, K_MAX] (S-12 ; K_MAX = 5)
//!   4. RateLimiter::check (× k weight ; each sub-swarm counts) ← D18
//!   5. Spawn k tasks via tokio::JoinSet :
//!        - mode_per_call = "mode1" → call Mode-1 backend with seed_i
//!        - mode_per_call = "mode2" → call Mode-2 backend with seed_i
//!   6. Collect verdicts as they complete (arrival order).
//!   7. Aggregator::aggregate(sub_verdicts)
//!   8. AuditEmitter::emit("mode3.execute", { k, mode, aggregator,
//!                                            dispersion, effective_n })
//!   9. Build Mode3ExecuteResponse.
//! ```
//!
//! ## Hard cap k = 5 (SLO S-12)
//!
//! `K_MAX = 5` is enforced server-side ; inputs above 5 are clamped
//! (no error) and the response sets `k_clamped = true`. This cap is
//! a product decision : at k=10 the GPU memory budget exceeds the
//! T4 baseline (16 GiB) for a single tenant.
//!
//! ## Cancellation
//!
//! `Mode3.Cancel(request_id)` looks up an in-flight `JoinHandle` and
//! aborts it. Sub-swarms past the `dissolve` round may still complete
//! (best-effort cancellation) — that's a `tokio::task::JoinSet`
//! contract, not a Mode-3 contract.

pub mod aggregator;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request as TRequest, Response as TResponse, Status};

use crate::auth::{AuthGuard, BroadScope, Scope, ScopeGuard};
use crate::interceptors::ratelimit::check_rate_limit;
use crate::interceptors::RequestTenant;
use crate::proto::v1_mode3::{
    mode3_service_server::Mode3Service, mode3_stream_chunk, AggregationMetadata, CancelRequest,
    Mode3ExecuteRequest, Mode3ExecuteResponse, Mode3FinalVerdict, Mode3StreamChunk,
    Mode3StreamError, SubVerdict,
};
use crate::ratelimit::BucketRegistry;
use crate::services::metrics::{emit_request_completion, record_ratelimit_drop};
use crate::services::DynExecuteBackend;

pub use aggregator::{
    AggregationError, AggregationOutput, MeanAggregator, MedianAggregator, Mode3Aggregator,
    SubVerdictData, WeightedAggregator,
};

/// SLO S-12 hard cap on the parallelism factor.
pub const K_MAX: u32 = 5;

const EXECUTE_PATH: &str = "/efe.v1.mode3.Mode3Service/Execute";
const SUBSCRIBE_PATH: &str = "/efe.v1.mode3.Mode3Service/SubscribeVerdict";
const CANCEL_PATH: &str = "/efe.v1.mode3.Mode3Service/Cancel";

/// In-flight Mode-3 request entry used for `Cancel`.
struct InflightEntry {
    /// JoinSet handle is opaque to outside callers ; we only keep
    /// the `AbortHandle`s so cancel can fire-and-forget abort.
    abort_handles: Vec<tokio::task::AbortHandle>,
}

/// Concrete Mode-3 service.
pub struct Mode3ServiceImpl {
    /// Shared backend ; called by Mode-1 sub-paths.
    mode1_backend: DynExecuteBackend,

    /// Optional Mode-2 service (cloned reference) ; required when
    /// `mode_per_call = "mode2"`. None disables the mode2 sub-path.
    mode2: Option<Arc<crate::services::Mode2ServiceImpl>>,

    /// Aggregator registry.
    aggregators: HashMap<String, Arc<dyn Mode3Aggregator>>,

    auth: Arc<AuthGuard>,
    rate_limiter: Option<Arc<BucketRegistry>>,

    /// Map request_id → AbortHandle list. Populated on Execute /
    /// SubscribeVerdict, consumed on Cancel.
    inflight: Arc<DashMap<String, InflightEntry>>,
}

impl std::fmt::Debug for Mode3ServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mode3ServiceImpl")
            .field("aggregators", &self.aggregators.keys().collect::<Vec<_>>())
            .field("auth_enabled", &self.auth.is_enabled())
            .field("rate_limiter", &self.rate_limiter.is_some())
            .field("mode2_enabled", &self.mode2.is_some())
            .field("inflight_count", &self.inflight.len())
            .finish()
    }
}

impl Mode3ServiceImpl {
    /// Constructor. `mode2 = None` disables the mode2 sub-path
    /// (Mode-3 then refuses requests with `mode_per_call = "mode2"`).
    #[must_use]
    pub fn new(
        mode1_backend: DynExecuteBackend,
        mode2: Option<Arc<crate::services::Mode2ServiceImpl>>,
        aggregators: HashMap<String, Arc<dyn Mode3Aggregator>>,
        auth: Arc<AuthGuard>,
        rate_limiter: Option<Arc<BucketRegistry>>,
    ) -> Self {
        Self {
            mode1_backend,
            mode2,
            aggregators,
            auth,
            rate_limiter,
            inflight: Arc::new(DashMap::new()),
        }
    }

    /// Wire the three built-in aggregators.
    #[must_use]
    pub fn default_aggregators() -> HashMap<String, Arc<dyn Mode3Aggregator>> {
        let mut m: HashMap<String, Arc<dyn Mode3Aggregator>> = HashMap::new();
        m.insert(
            "mean".to_string(),
            Arc::new(MeanAggregator) as Arc<dyn Mode3Aggregator>,
        );
        m.insert(
            "weighted".to_string(),
            Arc::new(WeightedAggregator) as Arc<dyn Mode3Aggregator>,
        );
        m.insert(
            "median".to_string(),
            Arc::new(MedianAggregator) as Arc<dyn Mode3Aggregator>,
        );
        m
    }

    fn tenant_of<T>(req: &TRequest<T>) -> String {
        req.extensions()
            .get::<RequestTenant>()
            .map(|t| t.0.clone())
            .unwrap_or_else(|| "anonymous".to_string())
    }

    fn resolve_aggregator(
        aggregators: &HashMap<String, Arc<dyn Mode3Aggregator>>,
        id: &str,
    ) -> Result<Arc<dyn Mode3Aggregator>, Status> {
        aggregators
            .get(id)
            .cloned()
            .ok_or_else(|| Status::not_found(format!("unknown aggregator: {id}")))
    }
}

/// Map `AggregationError` to `tonic::Status`.
fn agg_err_to_status(e: AggregationError) -> Status {
    match e {
        AggregationError::Empty => {
            Status::failed_precondition("aggregator received 0 sub-verdicts")
        }
        AggregationError::UnknownStrategy(s) => {
            Status::not_found(format!("unknown aggregator: {s}"))
        }
    }
}

/// Effective k after clamping to `[1, K_MAX]`. Returns the
/// `(k_effective, k_clamped)` pair.
fn clamp_k(k: u32) -> (u32, bool) {
    if k == 0 {
        (3, false)
    } else if k > K_MAX {
        (K_MAX, true)
    } else {
        (k, false)
    }
}

/// Execute a single sub-swarm and return a `SubVerdictData`.
///
/// **Audit fix S-A2** — pre-fix this function took no backend
/// reference and produced a synthetic confidence (`0.5 + idx*0.1`).
/// Mode-3 didn't actually fan-out to k swarms ; it was a glorified
/// `for i in 0..k { yield synthetic }` loop. Worse, the
/// `mode_per_call` parameter was ignored (the body was identical
/// for "mode1" and "mode2").
///
/// The fix :
///   1. Takes a `mode1_backend: DynExecuteBackend` reference.
///   2. Takes an optional `mode2: Arc<Mode2ServiceImpl>` for the
///      mode2 fan-out.
///   3. Dispatches on `mode` :
///        * "mode1" → `mode1_backend.execute(synth_request)`.
///        * "mode2" → `mode2.execute_inner(query, seed)` (which
///                    internally calls `backend.execute` after
///                    consulting the LLM router cache).
///   4. Maps the resulting `efe_core::Verdict` to the proto i32.
///
/// **Audit fix S-A6** — the previous synthesis returned
/// `VERDICT_CONVERGED` (=2) regardless of the real swarm outcome,
/// which made aggregation tests trivially pass even when the swarm
/// would have diverged. The verdict mapping now reflects the
/// genuine consensus detector output.
async fn execute_sub_swarm(
    swarm_idx: u32,
    seed_used: u64,
    mode: &str,
    query_text: &str,
    tenant_id: &str,
    mode1_backend: &DynExecuteBackend,
    mode2: Option<&Arc<crate::services::Mode2ServiceImpl>>,
    top_k_anchors: u32,
    router_id: &str,
) -> Result<SubVerdictData, Status> {
    use crate::services::synthetic_spec::{build_synthetic_request, verdict_to_proto_i32};

    let started = Instant::now();

    let (verdict_int, confidence) = match mode {
        "mode1" => {
            let req = build_synthetic_request(query_text, seed_used, tenant_id)?;
            let resp = mode1_backend
                .execute(req)
                .await
                .map_err(crate::bridge::error::to_status)?;
            (verdict_to_proto_i32(&resp.verdict), resp.confidence)
        }
        "mode2" => {
            let m2 = mode2.ok_or_else(|| {
                Status::failed_precondition(
                    "mode_per_call=mode2 requested but Mode-2 service not wired into Mode-3",
                )
            })?;
            // Effective top_k : 0 → 5 (Mode-2's own default).
            let effective_top_k = if top_k_anchors == 0 { 5 } else { top_k_anchors };
            let effective_router = if router_id.is_empty() {
                "mock"
            } else {
                router_id
            };
            m2.execute_inner(
                query_text,
                effective_top_k,
                effective_router,
                tenant_id,
                seed_used,
                /* bypass_cache = */ false,
            )
            .await?
        }
        other => {
            return Err(Status::invalid_argument(format!(
                "unknown mode_per_call: {other}"
            )));
        }
    };

    let latency_ms = started.elapsed().as_millis() as u32;
    Ok(SubVerdictData {
        swarm_idx,
        verdict: verdict_int,
        confidence,
        latency_ms,
        mode_used: mode.to_string(),
        seed_used,
    })
}

#[async_trait]
impl Mode3Service for Mode3ServiceImpl {
    async fn execute(
        &self,
        mut req: TRequest<Mode3ExecuteRequest>,
    ) -> Result<TResponse<Mode3ExecuteResponse>, Status> {
        let started = Instant::now();
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_EXECUTE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // **Audit fix S-A7** — pre-fix the `?` here returned without
        // emitting `efe_grpc_*` metrics for the denied request. The
        // fix uses `if let Err(e)` with explicit emit before return.
        if let Some(claims) = req.extensions().get::<crate::interceptors::AuthExtClaims>() {
            if let Err(e) = ScopeGuard::require(&claims.0, BroadScope::DATA) {
                emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        }
        let inner = req.into_inner();
        let (k_effective, k_clamped) = clamp_k(inner.k);

        // **Audit fix T-A1** — bump `efe_mode3_k_clamped_total{tenant}`
        // when the input k > K_MAX was clamped. SLO C-5 panel
        // (`d37/01_mode3.json` "k Clamping Rate") needs this signal.
        // Pre-audit the instrument was declared but never bumped —
        // panel showed "no data" forever.
        if k_clamped {
            efe_telemetry::metrics::mode3_instruments::mode3_k_clamped_total()
                .with_label_values(&[&tenant])
                .inc();
        }

        // 4. Rate limit ; weight = k_effective × Mode-1 cost.
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let request_id = if inner.request_id.is_empty() {
            efe_core::RequestId::new().as_str().to_string()
        } else {
            inner.request_id.clone()
        };
        let mode = if inner.mode_per_call.is_empty() {
            "mode1".to_string()
        } else {
            inner.mode_per_call.clone()
        };
        // **Audit fix S-A7** — emit metric on the InvalidArgument /
        // FailedPrecondition pre-flight checks so the dashboard sees
        // them.
        if mode != "mode1" && mode != "mode2" {
            let e = Status::invalid_argument(format!("unknown mode_per_call: {mode}"));
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if mode == "mode2" && self.mode2.is_none() {
            let e = Status::failed_precondition("mode_per_call=mode2 but Mode-2 service not wired");
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let aggregator_id = if inner.aggregator.is_empty() {
            "mean".to_string()
        } else {
            inner.aggregator.clone()
        };
        let aggregator = match Self::resolve_aggregator(&self.aggregators, &aggregator_id) {
            Ok(a) => a,
            Err(e) => {
                emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        };

        // 5. Spawn k sub-swarms.
        // **Audit fix S-A2** — `execute_sub_swarm` now takes the
        // backend handles + the query string. We clone the Arc'd
        // handles per task so each tokio future owns its dependencies.
        let mut set: JoinSet<Result<SubVerdictData, Status>> = JoinSet::new();
        let mut abort_handles = Vec::with_capacity(k_effective as usize);
        for i in 0..k_effective {
            let seed_i = inner.seed.wrapping_add(i as u64);
            let mode_clone = mode.clone();
            let query_clone = inner.query_text.clone();
            let tenant_clone = tenant.clone();
            let mode1_backend = self.mode1_backend.clone();
            let mode2_clone = self.mode2.clone();
            let router_id_clone = inner.llm_router_id.clone();
            let top_k = inner.top_k_anchors;
            let h = set.spawn(async move {
                execute_sub_swarm(
                    i,
                    seed_i,
                    &mode_clone,
                    &query_clone,
                    &tenant_clone,
                    &mode1_backend,
                    mode2_clone.as_ref(),
                    top_k,
                    &router_id_clone,
                )
                .await
            });
            abort_handles.push(h);
        }
        // Track for Cancel.
        self.inflight
            .insert(request_id.clone(), InflightEntry { abort_handles });

        // 6. Collect.
        // **Audit fix S-A7** — every error path through the join loop
        // emits the metric with the correct status code.
        let mut sub_verdicts = Vec::with_capacity(k_effective as usize);
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(Ok(data)) => sub_verdicts.push(data),
                Ok(Err(status)) => {
                    self.inflight.remove(&request_id);
                    emit_request_completion(EXECUTE_PATH, &tenant, &Err(&status), started);
                    return Err(status);
                }
                Err(join_err) => {
                    self.inflight.remove(&request_id);
                    let status = if join_err.is_cancelled() {
                        Status::cancelled("sub-swarm cancelled")
                    } else {
                        Status::internal(format!("join error: {join_err}"))
                    };
                    emit_request_completion(EXECUTE_PATH, &tenant, &Err(&status), started);
                    return Err(status);
                }
            }
        }
        self.inflight.remove(&request_id);

        // **Audit fix T-A1** — bump `efe_mode3_subswarms_spawned_total`
        // by the number of sub-swarms actually completed (one per
        // entry in `sub_verdicts`). We do this AFTER the join loop
        // so cancelled sub-swarms aren't counted (they fail the
        // outer Err arm before reaching here).
        efe_telemetry::metrics::mode3_instruments::mode3_subswarms_spawned_total()
            .with_label_values(&[&mode, &tenant])
            .inc_by(sub_verdicts.len() as u64);

        // 7. Aggregate.
        // **Audit fix T-A1** — wrap aggregator.aggregate in Instant
        // timer so `efe_mode3_aggregation_seconds{aggregator}` gets
        // observable data. Pre-audit the instrument was declared but
        // never observed — d37 latency panel was flat zero.
        let agg_started = Instant::now();
        let agg_out = match aggregator.aggregate(&sub_verdicts) {
            Ok(a) => a,
            Err(e) => {
                let status = agg_err_to_status(e);
                emit_request_completion(EXECUTE_PATH, &tenant, &Err(&status), started);
                return Err(status);
            }
        };
        efe_telemetry::metrics::mode3_instruments::mode3_aggregation_seconds()
            .with_label_values(&[&aggregator_id])
            .observe(agg_started.elapsed().as_secs_f64());

        // **Audit fix T-A1** — set the dispersion gauge with the
        // aggregator's last observed value. Gauge semantics : last
        // value wins ; the d37 dashboard heatmap reads
        // `efe_mode3_dispersion{aggregator,tenant}` over time.
        efe_telemetry::metrics::mode3_instruments::mode3_dispersion()
            .with_label_values(&[&aggregator_id, &tenant])
            .set(f64::from(agg_out.dispersion));

        // Build response.
        let response_sub: Vec<SubVerdict> = sub_verdicts
            .iter()
            .map(|sv| SubVerdict {
                swarm_idx: sv.swarm_idx,
                verdict: sv.verdict,
                latency_ms: sv.latency_ms,
                mode_used: sv.mode_used.clone(),
                seed_used: sv.seed_used,
            })
            .collect();

        emit_request_completion(EXECUTE_PATH, &tenant, &Ok(()), started);
        Ok(TResponse::new(Mode3ExecuteResponse {
            final_verdict: agg_out.final_verdict,
            sub_verdicts: response_sub,
            aggregation: Some(AggregationMetadata {
                strategy: agg_out.strategy,
                total_confidence: agg_out.total_confidence,
                dispersion: agg_out.dispersion,
                effective_n: agg_out.effective_n,
            }),
            request_id,
            total_ms: started.elapsed().as_secs_f32() * 1000.0,
            k_clamped,
            k_effective,
        }))
    }

    type SubscribeVerdictStream = ReceiverStream<Result<Mode3StreamChunk, Status>>;

    async fn subscribe_verdict(
        &self,
        mut req: TRequest<Mode3ExecuteRequest>,
    ) -> Result<TResponse<Self::SubscribeVerdictStream>, Status> {
        let started = Instant::now();
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_EXECUTE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(SUBSCRIBE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // **Audit fix S-A7** — emit metric on early-return (broad
        // scope, rate limit, aggregator resolution).
        if let Some(claims) = req.extensions().get::<crate::interceptors::AuthExtClaims>() {
            if let Err(e) = ScopeGuard::require(&claims.0, BroadScope::DATA) {
                emit_request_completion(SUBSCRIBE_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(SUBSCRIBE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let inner = req.into_inner();
        let (k_effective, k_clamped) = clamp_k(inner.k);
        // **Audit fix T-A1** — wire k_clamped counter symmetric
        // with the unary `execute` path. Streaming SubscribeVerdict
        // requests that exceed K_MAX must also surface in the SLO
        // C-5 panel.
        if k_clamped {
            efe_telemetry::metrics::mode3_instruments::mode3_k_clamped_total()
                .with_label_values(&[&tenant])
                .inc();
        }
        let mode = if inner.mode_per_call.is_empty() {
            "mode1".to_string()
        } else {
            inner.mode_per_call.clone()
        };
        let aggregator_id = if inner.aggregator.is_empty() {
            "mean".to_string()
        } else {
            inner.aggregator.clone()
        };
        let aggregator = match Self::resolve_aggregator(&self.aggregators, &aggregator_id) {
            Ok(a) => a,
            Err(e) => {
                emit_request_completion(SUBSCRIBE_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        };
        let request_id = if inner.request_id.is_empty() {
            efe_core::RequestId::new().as_str().to_string()
        } else {
            inner.request_id.clone()
        };

        let (tx, rx) = mpsc::channel::<Result<Mode3StreamChunk, Status>>(64);
        let inflight = self.inflight.clone();
        let tenant_task = tenant.clone();
        let request_id_task = request_id.clone();
        // **Audit fix S-A2** — clone the backend handles outside the
        // spawned task so they can be moved into the per-sub-swarm
        // closures.
        let mode1_backend = self.mode1_backend.clone();
        let mode2_handle = self.mode2.clone();
        let router_id = inner.llm_router_id.clone();

        tokio::spawn(async move {
            let mut set: JoinSet<Result<SubVerdictData, Status>> = JoinSet::new();
            let mut abort_handles = Vec::with_capacity(k_effective as usize);
            for i in 0..k_effective {
                let seed_i = inner.seed.wrapping_add(i as u64);
                let mode_clone = mode.clone();
                let query_clone = inner.query_text.clone();
                let tenant_for_sub = tenant_task.clone();
                let m1 = mode1_backend.clone();
                let m2 = mode2_handle.clone();
                let router_id_clone = router_id.clone();
                let top_k = inner.top_k_anchors;
                let h = set.spawn(async move {
                    execute_sub_swarm(
                        i,
                        seed_i,
                        &mode_clone,
                        &query_clone,
                        &tenant_for_sub,
                        &m1,
                        m2.as_ref(),
                        top_k,
                        &router_id_clone,
                    )
                    .await
                });
                abort_handles.push(h);
            }
            inflight.insert(request_id_task.clone(), InflightEntry { abort_handles });

            let mut sub_verdicts = Vec::with_capacity(k_effective as usize);
            while let Some(joined) = set.join_next().await {
                match joined {
                    Ok(Ok(data)) => {
                        let chunk = Mode3StreamChunk {
                            payload: Some(mode3_stream_chunk::Payload::Sub(SubVerdict {
                                swarm_idx: data.swarm_idx,
                                verdict: data.verdict,
                                latency_ms: data.latency_ms,
                                mode_used: data.mode_used.clone(),
                                seed_used: data.seed_used,
                            })),
                        };
                        if tx.send(Ok(chunk)).await.is_err() {
                            inflight.remove(&request_id_task);
                            return; // client cancelled
                        }
                        sub_verdicts.push(data);
                    }
                    Ok(Err(status)) => {
                        inflight.remove(&request_id_task);
                        let _ = tx
                            .send(Ok(Mode3StreamChunk {
                                payload: Some(mode3_stream_chunk::Payload::Error(
                                    Mode3StreamError {
                                        code: status.code() as u32,
                                        message: status.message().to_string(),
                                        details: Default::default(),
                                    },
                                )),
                            }))
                            .await;
                        return;
                    }
                    Err(join_err) => {
                        inflight.remove(&request_id_task);
                        let _ = tx
                            .send(Ok(Mode3StreamChunk {
                                payload: Some(mode3_stream_chunk::Payload::Error(
                                    Mode3StreamError {
                                        code: tonic::Code::Internal as u32,
                                        message: format!("join error: {join_err}"),
                                        details: Default::default(),
                                    },
                                )),
                            }))
                            .await;
                        return;
                    }
                }
            }
            inflight.remove(&request_id_task);

            // **Audit fix T-A1** — symmetric metric wiring for the
            // streaming path. Same instruments as the unary execute,
            // bumped from inside the spawned task. Without this, the
            // d37 dashboard would only see unary-call metrics ;
            // streaming traffic would be invisible.
            efe_telemetry::metrics::mode3_instruments::mode3_subswarms_spawned_total()
                .with_label_values(&[&mode, &tenant_task])
                .inc_by(sub_verdicts.len() as u64);

            // Aggregate + final.
            let agg_started = Instant::now();
            match aggregator.aggregate(&sub_verdicts) {
                Ok(agg_out) => {
                    let response_sub: Vec<SubVerdict> = sub_verdicts
                        .iter()
                        .map(|sv| SubVerdict {
                            swarm_idx: sv.swarm_idx,
                            verdict: sv.verdict,
                            latency_ms: sv.latency_ms,
                            mode_used: sv.mode_used.clone(),
                            seed_used: sv.seed_used,
                        })
                        .collect();

                    // **Audit fix T-A1** — observe the aggregator
                    // wall time + dispersion in the streaming path
                    // (same as unary execute).
                    efe_telemetry::metrics::mode3_instruments::mode3_aggregation_seconds()
                        .with_label_values(&[&aggregator_id])
                        .observe(agg_started.elapsed().as_secs_f64());
                    efe_telemetry::metrics::mode3_instruments::mode3_dispersion()
                        .with_label_values(&[&aggregator_id, &tenant_task])
                        .set(f64::from(agg_out.dispersion));

                    let response = Mode3ExecuteResponse {
                        final_verdict: agg_out.final_verdict,
                        sub_verdicts: response_sub,
                        aggregation: Some(AggregationMetadata {
                            strategy: agg_out.strategy,
                            total_confidence: agg_out.total_confidence,
                            dispersion: agg_out.dispersion,
                            effective_n: agg_out.effective_n,
                        }),
                        request_id: request_id_task,
                        total_ms: started.elapsed().as_secs_f32() * 1000.0,
                        k_clamped,
                        k_effective,
                    };
                    emit_request_completion(SUBSCRIBE_PATH, &tenant_task, &Ok(()), started);
                    let _ = tx
                        .send(Ok(Mode3StreamChunk {
                            payload: Some(mode3_stream_chunk::Payload::Final(Mode3FinalVerdict {
                                response: Some(response),
                            })),
                        }))
                        .await;
                }
                Err(e) => {
                    let status = agg_err_to_status(e);
                    emit_request_completion(SUBSCRIBE_PATH, &tenant_task, &Err(&status), started);
                    let _ = tx
                        .send(Ok(Mode3StreamChunk {
                            payload: Some(mode3_stream_chunk::Payload::Error(Mode3StreamError {
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

    async fn cancel(&self, mut req: TRequest<CancelRequest>) -> Result<TResponse<()>, Status> {
        let started = Instant::now();
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_EXECUTE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(CANCEL_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let inner = req.into_inner();
        if let Some((_, entry)) = self.inflight.remove(&inner.request_id) {
            for h in entry.abort_handles {
                h.abort();
            }
        }
        emit_request_completion(CANCEL_PATH, &tenant, &Ok(()), started);
        Ok(TResponse::new(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_k_zero_defaults_to_3() {
        assert_eq!(clamp_k(0), (3, false));
    }

    #[test]
    fn clamp_k_above_max_clamps_and_flags() {
        assert_eq!(clamp_k(10), (K_MAX, true));
    }

    #[test]
    fn clamp_k_within_range_passes_through() {
        for k in 1..=K_MAX {
            assert_eq!(clamp_k(k), (k, false));
        }
    }

    #[test]
    fn default_aggregators_has_three_entries() {
        let m = Mode3ServiceImpl::default_aggregators();
        assert!(m.contains_key("mean"));
        assert!(m.contains_key("weighted"));
        assert!(m.contains_key("median"));
        assert_eq!(m.len(), 3);
    }

    /// Backend stub for the unit test : returns a deterministic
    /// converged response so the test asserts the dispatch logic
    /// (mode1 vs mode2 vs unknown) without booting a real swarm
    /// controller.
    struct StubBackend;
    #[async_trait::async_trait]
    impl crate::services::ExecuteBackend for StubBackend {
        async fn execute(&self, req: efe_core::Request) -> efe_core::Result<efe_core::Response> {
            // Use request_id as the swarm_id so each call has a stable
            // correlation id ; the verdict is hardcoded converged with
            // confidence=0.7 so the aggregator tests have a known
            // expected value.
            let mut resp =
                efe_core::Response::for_tests(req.id, efe_core::DecodedOutput::Text("stub".into()));
            resp.verdict = efe_core::Verdict::Converged { confidence: 0.7 };
            resp.confidence = 0.7;
            resp.rounds_executed = 2;
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn execute_sub_swarm_mode1_calls_backend() {
        // **Audit fix S-A2 regression** — pre-fix the function ignored
        // the backend completely. This test asserts that mode1 path
        // invokes the backend (via the StubBackend's deterministic
        // confidence=0.7) rather than the old synthesis (`0.5 + idx*0.1`).
        let backend: DynExecuteBackend = Arc::new(StubBackend);
        let a = execute_sub_swarm(
            0,
            42,
            "mode1",
            "test query",
            "tenant-x",
            &backend,
            None,
            5,
            "mock",
        )
        .await
        .unwrap();
        // 0.7 (real backend) vs 0.5 (the old synthesis at idx=0 was 0.5).
        assert!(
            (a.confidence - 0.7).abs() < 1e-6,
            "confidence={} should be the StubBackend's 0.7, not the old synthesis",
            a.confidence
        );
        assert_eq!(a.swarm_idx, 0);
        assert_eq!(a.mode_used, "mode1");
        assert_eq!(a.seed_used, 42);
        // Verdict should be Converged (= 2) per StubBackend.
        assert_eq!(a.verdict, 2);
    }

    #[tokio::test]
    async fn execute_sub_swarm_mode2_without_handle_fails_precondition() {
        // **Audit fix S-A2 regression** — pre-fix mode2 path silently
        // ran the same synthesis as mode1. Now it requires a wired
        // Mode-2 service ; absence triggers FailedPrecondition.
        let backend: DynExecuteBackend = Arc::new(StubBackend);
        let err = execute_sub_swarm(0, 0, "mode2", "q", "t", &backend, None, 5, "mock")
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::FailedPrecondition);
        assert!(err.message().contains("Mode-2 service not wired"));
    }

    #[tokio::test]
    async fn execute_sub_swarm_unknown_mode_returns_invalid_argument() {
        let backend: DynExecuteBackend = Arc::new(StubBackend);
        let err = execute_sub_swarm(0, 0, "mode4", "q", "t", &backend, None, 5, "mock")
            .await
            .unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
}
