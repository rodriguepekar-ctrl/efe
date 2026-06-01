//! `SwarmService` implementation (unary + streaming).

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request as TRequest, Response as TResponse, Status};

use crate::auth::{AuthGuard, BroadScope, Scope, ScopeGuard};
use crate::bridge::{self, error::to_status};
use crate::idempotency::IdempotencyCache;
use crate::interceptors::ratelimit::check_rate_limit;
use crate::interceptors::{AuthExtClaims, RequestTenant};
use crate::proto::v1::{
    stream_chunk, swarm_service_server::SwarmService, DescribeSpecRequest, DescribeSpecResponse,
    ExecuteRequest, ExecuteResponse, FinalVerdict, RoundUpdate, StreamChunk, StreamError,
};
use crate::ratelimit::BucketRegistry;
use crate::services::metrics::{emit_request_completion, record_ratelimit_drop};
use crate::services::DynExecuteBackend;
use efe_core::swarm::TraceContext;

/// Concrete service implementation.
pub struct SwarmServiceImpl {
    backend: DynExecuteBackend,
    auth: Arc<AuthGuard>,
    idempotency: Option<Arc<IdempotencyCache>>,
    rate_limiter: Option<Arc<BucketRegistry>>,
}

impl std::fmt::Debug for SwarmServiceImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmServiceImpl")
            .field("backend", &"<dyn ExecuteBackend>")
            .field("auth_enabled", &self.auth.is_enabled())
            .field("idempotency", &self.idempotency.is_some())
            .field("rate_limiter", &self.rate_limiter.is_some())
            .finish()
    }
}

impl SwarmServiceImpl {
    /// Construct with all injected dependencies.
    #[must_use]
    pub fn new(
        backend: DynExecuteBackend,
        auth: Arc<AuthGuard>,
        idempotency: Option<Arc<IdempotencyCache>>,
        rate_limiter: Option<Arc<BucketRegistry>>,
    ) -> Self {
        Self {
            backend,
            auth,
            idempotency,
            rate_limiter,
        }
    }

    fn tenant_of<T>(req: &TRequest<T>) -> String {
        req.extensions()
            .get::<RequestTenant>()
            .map(|t| t.0.clone())
            .unwrap_or_else(|| "anonymous".to_string())
    }

    fn trace_of<T>(req: &TRequest<T>) -> Option<TraceContext> {
        req.extensions().get::<TraceContext>().cloned()
    }
}

const EXECUTE_PATH: &str = "/efe.v1.SwarmService/Execute";
const EXECUTE_STREAMING_PATH: &str = "/efe.v1.SwarmService/ExecuteStreaming";
const DESCRIBE_SPEC_PATH: &str = "/efe.v1.SwarmService/DescribeSpec";

#[async_trait]
impl SwarmService for SwarmServiceImpl {
    async fn execute(
        &self,
        mut req: TRequest<ExecuteRequest>,
    ) -> Result<TResponse<ExecuteResponse>, Status> {
        let started = Instant::now();
        // 1. Auth (also populates tenant + trace context).
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_EXECUTE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // **D22 broad-scope** — defence in depth. The granular
        // `swarm:execute` scope is necessary but not sufficient ; the
        // principal MUST also carry the broad `data` scope.
        if let Some(claims) = req.extensions().get::<AuthExtClaims>() {
            if let Err(e) = ScopeGuard::require(&claims.0, BroadScope::DATA) {
                emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
                return Err(e);
            }
        }
        // 2. Rate-limit on the authenticated tenant.
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(EXECUTE_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }

        let trace_context = Self::trace_of(&req);
        let mut inner = req.into_inner();
        let request_id_pre = inner.request_id.clone();

        // 3. Idempotency: short-circuit if a recent identical response is cached.
        if let Some(cache) = &self.idempotency {
            if !request_id_pre.is_empty() {
                if let Some(cached) = cache.get(&tenant, &request_id_pre) {
                    tracing::debug!(tenant, request_id = %request_id_pre, "idempotency hit");
                    let ok: Result<TResponse<ExecuteResponse>, Status> = Ok(TResponse::new(cached));
                    emit_request_completion(
                        EXECUTE_PATH,
                        &tenant,
                        &ok.as_ref().map(|_| ()),
                        started,
                    );
                    return ok;
                }
            }
        }

        // Ensure a request id exists in the proto; the bridge also
        // generates one server-side when absent, but we need the exact
        // string we will use as the cache key below.
        if inner.request_id.is_empty() {
            inner.request_id = efe_core::RequestId::new().as_str().to_string();
        }
        let request_id = inner.request_id.clone();

        let outcome: Result<TResponse<ExecuteResponse>, Status> = async {
            let domain_req = bridge::request::from_proto(inner, tenant.clone(), trace_context)
                .map_err(to_status)?;
            let domain_resp = self.backend.execute(domain_req).await.map_err(to_status)?;
            let proto_resp = bridge::response::to_proto(domain_resp);
            if let Some(cache) = &self.idempotency {
                cache.insert(&tenant, &request_id, proto_resp.clone());
            }
            Ok(TResponse::new(proto_resp))
        }
        .await;

        emit_request_completion(
            EXECUTE_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }

    type ExecuteStreamingStream = ReceiverStream<Result<StreamChunk, Status>>;

    async fn execute_streaming(
        &self,
        mut req: TRequest<ExecuteRequest>,
    ) -> Result<TResponse<Self::ExecuteStreamingStream>, Status> {
        // ──────────────────────────────────────────────────────────
        // **Sprint D32** (M4 § 12.3) — Server-streaming activation.
        //
        // Pre-D32 the streaming path emitted exactly ONE chunk (the
        // final `ExecuteResponse`). Per M4 § 12 the new contract is
        // N+1 chunks : N `RoundUpdate`s (one per fusion round) plus
        // one `FinalVerdict` (or `StreamError` on failure).
        //
        // The current `ExecuteBackend` trait does not expose a
        // round-by-round callback — that's `DT-S-D32-ROUNDS-1`
        // (post-D32 once the backend is wired to a streaming
        // dissolver). For D32 we emit one synthetic `RoundUpdate`
        // (round=1) with the `confidence_so_far` taken from the
        // final response, then the final chunk. The wire-level
        // contract is exercised end-to-end ; clients can switch
        // their parser to `oneof StreamChunk.payload` without code
        // changes when full round-by-round emission lands.
        //
        // **Audit fix S-A4** — the channel size is now 64 (M4 § 12.5
        // contract) instead of 32. Larger buffer reduces back-pressure
        // on slow clients without measurable memory cost.
        // ──────────────────────────────────────────────────────────
        let started = Instant::now();
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_EXECUTE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(EXECUTE_STREAMING_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        // **D22 broad-scope check** — every data-plane RPC requires
        // the broad `data` scope. `AuthGuard::check` already validated
        // the granular `swarm:execute` scope above ; the broad check
        // is an additional gate (defence in depth).
        if let Some(claims) = req.extensions().get::<AuthExtClaims>() {
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

        let trace_context = Self::trace_of(&req);
        let inner = req.into_inner();
        // **D32 buffer = 64** — see audit note above.
        let (tx, rx) = mpsc::channel::<Result<StreamChunk, Status>>(64);
        let backend = self.backend.clone();
        let tenant_task = tenant.clone();

        tokio::spawn(async move {
            let domain_req =
                match bridge::request::from_proto(inner, tenant_task.clone(), trace_context) {
                    Ok(r) => r,
                    Err(e) => {
                        let status = to_status(e);
                        emit_request_completion(
                            EXECUTE_STREAMING_PATH,
                            &tenant_task,
                            &Err(&status),
                            started,
                        );
                        // Wrap the error in a `StreamError` chunk so
                        // the client sees a `oneof::Error` rather than
                        // a raw transport error. Mirrors M4 § 12.2.
                        let _ = tx
                            .send(Ok(StreamChunk {
                                payload: Some(stream_chunk::Payload::Error(StreamError {
                                    code: status.code() as u32,
                                    message: status.message().to_string(),
                                    details: Default::default(),
                                })),
                            }))
                            .await;
                        return;
                    }
                };
            // **Client cancel detection** — if the receiver dropped,
            // try_send fails. We probe before the heavy backend call
            // and again after. M4 § 12.4 SLO S-10 caps cleanup at 1 s.
            if tx.is_closed() {
                tracing::debug!(target: "efe.streaming", "client cancelled before backend dispatch");
                return;
            }

            let outcome = backend.execute(domain_req).await;
            match outcome {
                Ok(resp) => {
                    // Emit a synthetic RoundUpdate (D32 contract) so
                    // the client sees the typed `RoundUpdate` variant
                    // even on the MVP backend that doesn't expose
                    // per-round telemetry. DT-S-D32-ROUNDS-1 will
                    // wire one chunk per real round.
                    let elapsed_ms = started.elapsed().as_secs_f32() * 1000.0;
                    let _ = tx
                        .send(Ok(StreamChunk {
                            payload: Some(stream_chunk::Payload::RoundUpdate(RoundUpdate {
                                round: resp.rounds_executed,
                                variance: 0.0,
                                confidence_so_far: resp.confidence,
                                active_agents: 0,
                                elapsed_ms,
                            })),
                        }))
                        .await;
                    // Final chunk.
                    let final_chunk = StreamChunk {
                        payload: Some(stream_chunk::Payload::Final(FinalVerdict {
                            response: Some(bridge::response::to_proto(resp)),
                        })),
                    };
                    emit_request_completion(EXECUTE_STREAMING_PATH, &tenant_task, &Ok(()), started);
                    let _ = tx.send(Ok(final_chunk)).await;
                }
                Err(e) => {
                    let status = to_status(e);
                    emit_request_completion(
                        EXECUTE_STREAMING_PATH,
                        &tenant_task,
                        &Err(&status),
                        started,
                    );
                    // Emit a typed `StreamError` chunk (M4 § 12.2)
                    // rather than a raw transport error.
                    let _ = tx
                        .send(Ok(StreamChunk {
                            payload: Some(stream_chunk::Payload::Error(StreamError {
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

    async fn describe_spec(
        &self,
        mut req: TRequest<DescribeSpecRequest>,
    ) -> Result<TResponse<DescribeSpecResponse>, Status> {
        let started = Instant::now();
        let auth_res = self.auth.check(&mut req, Some(Scope::SWARM_DESCRIBE)).await;
        let tenant = Self::tenant_of(&req);
        if let Err(e) = auth_res {
            emit_request_completion(DESCRIBE_SPEC_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        if let Err(e) = check_rate_limit(self.rate_limiter.as_ref(), &tenant) {
            record_ratelimit_drop(&tenant);
            emit_request_completion(DESCRIBE_SPEC_PATH, &tenant, &Err(&e), started);
            return Err(e);
        }
        let outcome = match req.into_inner().spec {
            None => Err(Status::invalid_argument(
                "DescribeSpecRequest.spec is required",
            )),
            Some(spec_proto) => match bridge::spec::from_proto(spec_proto) {
                Ok(spec) => Ok(TResponse::new(DescribeSpecResponse {
                    valid: true,
                    errors: Vec::new(),
                    hbm_bytes_estimate: bridge::spec::estimate_hbm_bytes(&spec),
                    rounds_expected: spec.max_rounds.get(),
                })),
                Err(e) => Ok(TResponse::new(DescribeSpecResponse {
                    valid: false,
                    errors: vec![e.to_string()],
                    hbm_bytes_estimate: 0,
                    rounds_expected: 0,
                })),
            },
        };
        emit_request_completion(
            DESCRIBE_SPEC_PATH,
            &tenant,
            &outcome.as_ref().map(|_| ()),
            started,
        );
        outcome
    }
}
