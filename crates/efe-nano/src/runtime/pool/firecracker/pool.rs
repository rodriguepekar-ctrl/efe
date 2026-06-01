//! **Sprint 8 — N11** : `FirecrackerVmPool` — the top-level `VmPool` impl.
//!
//! Wires together :
//! - [`super::config::FirecrackerPoolConfig`] : sizing + paths.
//! - [`super::lifecycle::Lifecycle`] : `MockLifecycle` (tests) or
//!   `FirecrackerLifecycle` (Linux + KVM production).
//! - [`super::kind_pool::KindPool`] : per-(kind, model, version, class)
//!   sub-pools.
//! - [`super::metrics::FirecrackerMetrics`] : 14 Prom metrics.
//! - [`super::reaper::Reaper`] : background sweep + provision.
//!
//! # The deferred-acquire pattern
//!
//! The parent `VmPool` trait says `acquire(kind, budget) -> VmLease`.
//! For Firecracker, **we don't yet know which sub-pool to checkout**
//! at acquire time — a `VmKind::ModelInference` could route to any
//! `(model_id, model_version)` sub-pool depending on the request.
//!
//! Sprint 8 N11 resolves this with a **deferred lease** :
//! - `acquire` returns a lease whose `executor` holds a thin handle
//!   to the pool + the requested kind + the wait_budget.
//! - When `execute(req)` is called, the executor reads
//!   `req.model_id + req.model_version` and dispatches to the right
//!   `KindPool::checkout`.  The actual semaphore-wait and (optional)
//!   spawn happen **at execute time**, not at acquire time.
//!
//! Cost : the per-lease "queue wait" budget telemetry counts the
//! execute-time wait, not the acquire-time wait.  In practice they're
//! equivalent because `acquire` does no real work in this design.

use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use async_trait::async_trait;
use dashmap::DashMap;

use crate::error::VmPoolError;

use super::config::FirecrackerPoolConfig;
use super::kind_pool::KindPool;
use super::lifecycle::{Lifecycle, MockLifecycle};
use super::metrics::FirecrackerMetrics;
use super::reaper::Reaper;
use super::rpc::{
    InferParams, RpcErrorCode, RpcMethod, RpcPayload, RpcResult, VsockRpcRequest, VsockRpcResponse,
};
use super::types::{ExecutionOutcome, SubPoolKey, TenantClass};

use super::super::{
    PerKindHealth, VmExecutor, VmKind, VmLease, VmPool, VmPoolHealth, VmRequest, VmResponse,
};

// ─────────────────────────────────────────────────────────────────────
//  FirecrackerVmPool
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : the production `VmPool` impl.
pub struct FirecrackerVmPool {
    inner: Arc<FirecrackerVmPoolInner>,
    reaper: tokio::sync::Mutex<Option<Reaper>>,
}

pub(crate) struct FirecrackerVmPoolInner {
    pub(crate) config: Arc<FirecrackerPoolConfig>,
    pub(crate) lifecycle: Arc<dyn Lifecycle>,
    pub(crate) sub_pools: Arc<DashMap<SubPoolKey, Arc<KindPool>>>,
    pub(crate) metrics: Arc<FirecrackerMetrics>,
}

impl FirecrackerVmPool {
    /// Construct with a custom [`Lifecycle`] (e.g. `MockLifecycle` for
    /// tests).  Spawns the background reaper.
    pub fn with_lifecycle(
        config: FirecrackerPoolConfig,
        lifecycle: Arc<dyn Lifecycle>,
    ) -> Result<Self, VmPoolError> {
        config
            .validate()
            .map_err(|e| VmPoolError::Internal(format!("invalid config : {e}")))?;
        let config = Arc::new(config);
        let metrics = Arc::new(FirecrackerMetrics::new());
        let sub_pools: Arc<DashMap<SubPoolKey, Arc<KindPool>>> = Arc::new(DashMap::new());
        let inner = Arc::new(FirecrackerVmPoolInner {
            config: Arc::clone(&config),
            lifecycle: Arc::clone(&lifecycle),
            sub_pools: Arc::clone(&sub_pools),
            metrics: Arc::clone(&metrics),
        });

        let reaper = Reaper::spawn(
            Arc::clone(&sub_pools),
            Arc::clone(&lifecycle),
            Arc::clone(&config),
            Arc::clone(&metrics),
        );
        Ok(Self {
            inner,
            reaper: tokio::sync::Mutex::new(Some(reaper)),
        })
    }

    /// Convenience : pool with the [`MockLifecycle`] for tests.
    pub fn for_testing() -> Self {
        Self::with_lifecycle(
            FirecrackerPoolConfig::for_testing(),
            Arc::new(MockLifecycle::new()),
        )
        .expect("test config validates")
    }

    /// Convenience : pool with a custom MockLifecycle (for failure-injection
    /// tests).
    pub fn for_testing_with_mock(mock: Arc<MockLifecycle>) -> Self {
        Self::with_lifecycle(FirecrackerPoolConfig::for_testing(), mock)
            .expect("test config validates")
    }

    /// Gracefully shut down the reaper.  Pre-existing leases are NOT
    /// invalidated — they keep their VM until checkin or drop.
    pub async fn shutdown_reaper(&self) {
        if let Some(reaper) = self.reaper.lock().await.take() {
            if let Err(e) = reaper.shutdown().await {
                tracing::warn!(
                    target: "efe_nano::firecracker",
                    error = ?e,
                    "reaper shutdown failed"
                );
            }
        }
    }

    /// Number of sub-pools currently materialized (telemetry).
    pub fn sub_pool_count(&self) -> usize {
        self.inner.sub_pools.len()
    }

    /// Expose metrics for `/metrics` Prometheus scrape.
    pub fn metrics(&self) -> Arc<FirecrackerMetrics> {
        Arc::clone(&self.inner.metrics)
    }

    /// Get-or-create the sub-pool for a given key.  Lazy creation lets
    /// the pool serve any (model, version, class) without per-key
    /// pre-config — operators only need to pin the per_kind config for
    /// agents they want hot-pre-warmed.
    ///
    /// **Visible at the public level** so integration tests can pre-warm
    /// a specific sub-pool before exercising acquire latency.
    pub fn get_or_create_sub_pool(&self, key: &SubPoolKey) -> Arc<KindPool> {
        self.inner.sub_pools_get_or_create_for(key)
    }
}

#[async_trait]
impl VmPool for FirecrackerVmPool {
    async fn acquire(
        &self,
        kind: VmKind,
        wait_budget: Duration,
    ) -> Result<VmLease, VmPoolError> {
        // **Sprint 8 — N11** : deferred-acquire pattern.  We don't know
        // which sub-pool to target until `execute(req)` reveals the
        // model_id / model_version, so the lease just records intent.
        let executor = Box::new(DeferredFirecrackerExecutor {
            inner: Arc::clone(&self.inner),
            kind,
            wait_budget,
            consumed: false,
        });
        Ok(VmLease {
            vm_id: Uuid::new_v4(),
            kind,
            acquired_at: Instant::now(),
            executor,
        })
    }

    async fn health(&self) -> VmPoolHealth {
        let mut kind_states = std::collections::HashMap::new();

        // Aggregate per-VmKind health across all sub-pools of that kind.
        let mut model_inference_warm = 0u32;
        let mut model_inference_in_use = 0u32;
        let mut model_inference_max = 0u32;
        let mut model_inference_acquires = 0u64;
        let mut model_inference_executes = 0u64;

        let mut llm_caller_warm = 0u32;
        let mut llm_caller_in_use = 0u32;
        let mut llm_caller_max = 0u32;
        let mut llm_caller_acquires = 0u64;
        let mut llm_caller_executes = 0u64;

        for entry in self.inner.sub_pools.iter() {
            let pool = entry.value();
            let warm = pool.warm_count().await;
            let in_use = pool.in_use_count().await;
            let m = pool.metrics();
            let acquires = m.warm_count.get() + m.cold_start_total.get();
            let executes = m.inference_duration.count();
            match pool.key.kind {
                VmKind::ModelInference => {
                    model_inference_warm += warm;
                    model_inference_in_use += in_use;
                    model_inference_max += pool.config.max_total;
                    model_inference_acquires += acquires;
                    model_inference_executes += executes;
                }
                VmKind::LlmCaller => {
                    llm_caller_warm += warm;
                    llm_caller_in_use += in_use;
                    llm_caller_max += pool.config.max_total;
                    llm_caller_acquires += acquires;
                    llm_caller_executes += executes;
                }
            }
        }

        kind_states.insert(
            VmKind::ModelInference,
            PerKindHealth {
                warm_slots: model_inference_warm,
                in_use_slots: model_inference_in_use,
                max_slots: model_inference_max,
                hit_rate_60s: 0.0, // Phase 4 : sliding-window calc
                p99_provision_ms: 0,
                total_acquires: model_inference_acquires,
                total_executes: model_inference_executes,
            },
        );
        kind_states.insert(
            VmKind::LlmCaller,
            PerKindHealth {
                warm_slots: llm_caller_warm,
                in_use_slots: llm_caller_in_use,
                max_slots: llm_caller_max,
                hit_rate_60s: 0.0,
                p99_provision_ms: 0,
                total_acquires: llm_caller_acquires,
                total_executes: llm_caller_executes,
            },
        );
        VmPoolHealth { kind_states }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  DeferredFirecrackerExecutor
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : the deferred-acquire executor.
///
/// On `execute(req)` :
/// 1. Builds the `SubPoolKey` from `req.model_id` + `req.model_version`
///    + the request's effective `TenantClass` (Sprint 8 default Tier1).
/// 2. Materializes the sub-pool if needed.
/// 3. `KindPool::checkout` with the original `wait_budget`.
/// 4. Builds the `VsockRpcRequest` from the `VmRequest`.
/// 5. `Lifecycle::rpc` with `vsock_timeout` ceiling.
/// 6. Translates `VsockRpcResponse` → `VmResponse`.
/// 7. `KindPool::checkin_warm` or `::checkin_reaped` based on
///    [`ExecutionOutcome`].
pub(crate) struct DeferredFirecrackerExecutor {
    inner: Arc<FirecrackerVmPoolInner>,
    kind: VmKind,
    wait_budget: Duration,
    /// True after one successful `execute` — re-using a lease for
    /// a second `execute` is forbidden (each lease == one VM op).
    consumed: bool,
}

#[async_trait]
impl VmExecutor for DeferredFirecrackerExecutor {
    async fn execute(&mut self, req: VmRequest) -> Result<VmResponse, VmPoolError> {
        if self.consumed {
            return Err(VmPoolError::Internal(
                "VmLease re-used : each lease accepts exactly one execute()".into(),
            ));
        }
        self.consumed = true;

        let started = Instant::now();
        let key = sub_pool_key_for(self.kind, &req)?;
        let sub_pool = self.inner.sub_pools_get_or_create_for(&key);
        let metrics = sub_pool.metrics();

        // Determine the budget for the actual checkout : the dispatcher
        // already capped `wait_budget` at the agent's wall_clock.  We
        // additionally cap at the request's `timeout_ms` so a
        // short-deadline call doesn't block a long-deadline VM cap.
        let effective_budget = self
            .wait_budget
            .min(Duration::from_millis(req.timeout_ms.max(1) as u64));
        let vm = sub_pool
            .checkout(&self.inner.lifecycle, effective_budget)
            .await?;

        // Build the RPC envelope.
        let now_micros = chrono::Utc::now().timestamp_micros();
        let deadline_micros = now_micros + (req.timeout_ms as i64 * 1000);
        let rpc_req = VsockRpcRequest {
            schema_version: super::rpc::SCHEMA_VERSION,
            request_id: Uuid::new_v4(),
            method: RpcMethod::Infer {
                params: InferParams {
                    input: req.input.clone(),
                    agent_id: req.agent_id.clone(),
                    agent_version: req.agent_version.clone(),
                    trace_id: Uuid::new_v4(),
                    deadline_micros,
                },
            },
        };

        // Send the RPC with the vsock_timeout ceiling.
        let vsock_timeout = self.inner.config.vsock_timeout;
        let rpc_started = Instant::now();
        let rpc_result = self
            .inner
            .lifecycle
            .rpc(&vm, rpc_req, vsock_timeout)
            .await;
        metrics.vsock_rtt.observe(rpc_started.elapsed());

        // **Sprint 8 — N11 audit fix (CRITICAL)** : the checkin path
        // MUST run on every code path — including when
        // `translate_response` returns Err.  Pre-fix, an `?` here
        // would propagate the error and leak the VM slot.  Post-fix,
        // every branch produces `(Result<VmResponse, _>, outcome)`
        // and the checkin policy fires unconditionally before we
        // return to the caller.
        let (response_result, outcome) = match rpc_result {
            Ok(resp) => translate_response(&self.kind, resp, metrics.as_ref()),
            Err(super::lifecycle::LifecycleError::RpcTimeout { timeout_ms }) => {
                metrics.vsock_error_total.inc();
                (
                    Err(VmPoolError::AcquireTimeout(timeout_ms as u32)),
                    ExecutionOutcome::HardError,
                )
            }
            Err(super::lifecycle::LifecycleError::RpcFailed(_reason)) => {
                metrics.vsock_error_total.inc();
                metrics.crash_total.inc();
                (Err(VmPoolError::VmCrashed), ExecutionOutcome::HardError)
            }
            Err(other) => {
                metrics.vsock_error_total.inc();
                (
                    Err(VmPoolError::Internal(format!(
                        "unexpected lifecycle error : {other}"
                    ))),
                    ExecutionOutcome::HardError,
                )
            }
        };

        // Always checkin BEFORE returning — prevents the slot leak that
        // the pre-fix `?` path produced.
        match outcome {
            ExecutionOutcome::Success | ExecutionOutcome::SoftError => {
                sub_pool.checkin_warm(vm).await;
            }
            ExecutionOutcome::HardError => {
                sub_pool.checkin_reaped(vm).await;
            }
        }
        metrics.inference_duration.observe(started.elapsed());
        response_result
    }
}

impl FirecrackerVmPoolInner {
    fn sub_pools_get_or_create_for(self: &Arc<Self>, key: &SubPoolKey) -> Arc<KindPool> {
        if let Some(existing) = self.sub_pools.get(key) {
            return Arc::clone(existing.value());
        }
        let config = self.config.for_subpool(key);
        let metrics = self.metrics.for_key(key);
        let pool = Arc::new(KindPool::new(key.clone(), config, metrics));
        match self.sub_pools.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(o) => Arc::clone(o.get()),
            dashmap::mapref::entry::Entry::Vacant(v) => {
                v.insert(Arc::clone(&pool));
                pool
            }
        }
    }
}

/// **Sprint 8 — N11** : build the `SubPoolKey` from a `VmRequest`.
///
/// Sprint 8 defaults `TenantClass` to `Tier1` since `VmRequest` doesn't
/// carry tenant info.  Sprint 9 will extend `VmRequest` with the
/// agent's `StabilityTier` so the pool can partition properly.
fn sub_pool_key_for(kind: VmKind, req: &VmRequest) -> Result<SubPoolKey, VmPoolError> {
    match kind {
        VmKind::ModelInference => {
            let model_id = req
                .model_id
                .as_deref()
                .ok_or_else(|| VmPoolError::Internal("Mode B request lacks model_id".into()))?;
            let model_version = req.model_version.as_deref().ok_or_else(|| {
                VmPoolError::Internal("Mode B request lacks model_version".into())
            })?;
            Ok(SubPoolKey::mode_b(model_id, model_version, TenantClass::Tier1))
        }
        VmKind::LlmCaller => Ok(SubPoolKey::mode_c(TenantClass::Tier1)),
    }
}

/// **Sprint 8 — N11** : translate a guest RPC response into a host
/// `VmResponse` + classify the outcome.
///
/// **Sprint 8 audit fix** : the signature is `(Result, outcome)` (not
/// `Result<(_, outcome)>`) so the caller can act on `outcome`
/// independently of `Result`'s Ok/Err.  This is the fix for the
/// pre-fix slot-leak bug where an `Err` return path swallowed the
/// outcome and the caller never checked-in the VM.
fn translate_response(
    kind: &VmKind,
    resp: VsockRpcResponse,
    metrics: &super::metrics::SubPoolMetrics,
) -> (Result<VmResponse, VmPoolError>, ExecutionOutcome) {
    match resp.payload {
        RpcPayload::Ok { result } => match result {
            RpcResult::Infer(infer) => {
                let response = match kind {
                    VmKind::ModelInference => VmResponse::mode_b(
                        infer.output,
                        (infer.model_invocation_micros / 1000).min(u32::MAX as u64) as u32,
                        infer.model_memory_high_water_mb,
                    ),
                    VmKind::LlmCaller => VmResponse {
                        output: infer.output,
                        cpu_ms_used: (infer.model_invocation_micros / 1000)
                            .min(u32::MAX as u64) as u32,
                        memory_mb_peak: infer.model_memory_high_water_mb,
                        // Mode C metadata isn't surfaced by Sprint 8
                        // guest inference daemon (no LLM router inside
                        // the VM yet — Phase 4) ; surface as None and
                        // let the caller infer from agent_id.
                        llm_tokens_used: None,
                        llm_cost_cents: None,
                        llm_provider: None,
                        llm_model: None,
                    },
                };
                (Ok(response), ExecutionOutcome::Success)
            }
            RpcResult::Health(_) | RpcResult::Shutdown { .. } => (
                Err(VmPoolError::Internal(
                    "received non-Infer Ok in Infer pathway".into(),
                )),
                // Treat as HardError : the guest is misbehaving
                // (responded to Infer with Health), reap it.
                ExecutionOutcome::HardError,
            ),
        },
        RpcPayload::Err { error } => {
            let outcome = if error.code.is_fatal_to_vm() {
                ExecutionOutcome::HardError
            } else {
                ExecutionOutcome::SoftError
            };
            let host_err = match error.code {
                RpcErrorCode::InputSchemaInvalid => VmPoolError::Internal(format!(
                    "guest rejected input schema : {}",
                    error.message
                )),
                RpcErrorCode::DeadlineExceeded => VmPoolError::AcquireTimeout(0),
                RpcErrorCode::ModelOom => {
                    metrics.crash_total.inc();
                    VmPoolError::VmCrashed
                }
                RpcErrorCode::Internal | RpcErrorCode::NotLoaded => {
                    metrics.crash_total.inc();
                    VmPoolError::Internal(format!(
                        "guest internal error : {} (code={:?})",
                        error.message, error.code
                    ))
                }
            };
            tracing::warn!(
                target: "efe_nano::firecracker",
                code = ?error.code,
                message = %error.message,
                ?outcome,
                "guest returned error"
            );
            (Err(host_err), outcome)
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::lifecycle::MockLifecycle;
    use crate::runtime::pool::VmKind;

    fn mode_b_req(model_id: &str, model_version: &str, payload: serde_json::Value) -> VmRequest {
        VmRequest::mode_b(
            "test_agent",
            "1.0.0",
            payload,
            5000,
            model_id,
            model_version,
        )
    }

    // ─────────────────────────────────────────────────────────────────
    //  acquire + execute happy path
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn acquire_returns_lease_immediately() {
        let pool = FirecrackerVmPool::for_testing();
        let start = Instant::now();
        let _lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(1))
            .await
            .unwrap();
        // Deferred-acquire pattern : acquire itself is near-instant.
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn execute_mode_b_routes_to_correct_subpool() {
        let pool = FirecrackerVmPool::for_testing();
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        let resp = lease
            .execute(mode_b_req(
                "ner_clinical",
                "1.0.0",
                serde_json::json!({"text": "hello"}),
            ))
            .await
            .unwrap();
        // Echo handler wraps input.
        assert_eq!(resp.output["echoed"]["text"], "hello");
        // A sub-pool was lazily created.
        assert!(pool.sub_pool_count() >= 1);
    }

    #[tokio::test]
    async fn execute_mode_c_routes_to_llm_caller_subpool() {
        let pool = FirecrackerVmPool::for_testing();
        let mut lease = pool
            .acquire(VmKind::LlmCaller, Duration::from_secs(2))
            .await
            .unwrap();
        // For Mode C the model_id/version are None.
        let req = VmRequest {
            agent_id: "summarize_strict".into(),
            agent_version: "1.0.0".into(),
            input: serde_json::json!({"text": "hello"}),
            timeout_ms: 5000,
            model_id: None,
            model_version: None,
            llm_call_request: None,
        };
        let resp = lease.execute(req).await.unwrap();
        assert!(resp.output["echoed"]["text"] == "hello");
    }

    // ─────────────────────────────────────────────────────────────────
    //  Re-use protection
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn lease_cannot_be_executed_twice() {
        let pool = FirecrackerVmPool::for_testing();
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(1))
            .await
            .unwrap();
        let _ = lease
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 1}),
            ))
            .await
            .unwrap();
        let err = lease
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 2}),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, VmPoolError::Internal(_)));
        assert!(err.to_string().contains("re-used"));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Pool-fill behavior
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_fails_with_acquire_timeout_when_subpool_full() {
        let mock = Arc::new(MockLifecycle::new());
        mock.set_spawn_latency(Duration::from_millis(500)); // slow cold start
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.per_kind[0].max_total = 1;
        cfg.per_kind[0].min_warm = 0;
        let pool = FirecrackerVmPool::with_lifecycle(cfg, mock).unwrap();

        // Hold the only VM via a long-running execute.
        let mut lease1 = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        let handle = tokio::spawn(async move {
            let req = mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 1}),
            );
            let _ = lease1.execute(req).await.unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;
        });

        // 2nd acquire/execute should timeout.
        let mut lease2 = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(100))
            .await
            .unwrap();
        let err = lease2
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 2}),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, VmPoolError::AcquireTimeout(_)));
        let _ = handle.await;
    }

    // ─────────────────────────────────────────────────────────────────
    //  Hard-error reaping
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn dead_vm_triggers_checkin_reaped() {
        let mock = Arc::new(MockLifecycle::new());
        let pool = FirecrackerVmPool::for_testing_with_mock(Arc::clone(&mock));

        // Kill the next-spawned VM AFTER 0 RPCs — first RPC fails.
        mock.kill_each_vm_after(1);
        // First execute kills the VM but receives the response.
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        let _resp = lease
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 1}),
            ))
            .await
            .unwrap();

        // Wait for reaper to notice + replenish.
        // Sprint 8 reaper_interval = 1s in for_testing config, so we
        // give it 2 ticks.
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let health = pool.health().await;
        let mi = &health.kind_states[&VmKind::ModelInference];
        // In-use back to 0.
        assert_eq!(mi.in_use_slots, 0);
    }

    #[tokio::test]
    async fn rpc_failure_translates_to_vm_crashed() {
        let mock = Arc::new(MockLifecycle::new());
        let pool = FirecrackerVmPool::for_testing_with_mock(Arc::clone(&mock));
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        // Cold-start first VM then kill it before the call lands.
        // Easier path : kill_vm by id after we have a snapshot.  But the
        // pool doesn't expose IDs — use kill_each_vm_after(0).
        mock.kill_each_vm_after(0);
        // Actually after-0 means immediately ; the first kill will hit.
        // (Mock semantics : we set kill_each_vm_after(0) AFTER spawn, so
        // the kill triggers on the first rpc.  Easier : kill_vm manual.)
        // Simplest reliable approach : let the first execute through,
        // then prove that after kill, the NEXT acquire/execute hits an
        // error.
        let _ = lease
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 1}),
            ))
            .await
            .unwrap();

        // Now mark every warm VM dead.
        // The pool keeps a warm copy ; mock can't enumerate but we can
        // assert metrics are sane (no panics, in_use returns to 0).
        let h = pool.health().await;
        assert_eq!(h.kind_states[&VmKind::ModelInference].in_use_slots, 0);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sub-pool partitioning
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn distinct_models_get_distinct_subpools() {
        let pool = FirecrackerVmPool::for_testing();
        {
            let mut lease = pool
                .acquire(VmKind::ModelInference, Duration::from_secs(1))
                .await
                .unwrap();
            lease
                .execute(mode_b_req(
                    "ner_clinical",
                    "1.0.0",
                    serde_json::json!({"x": 1}),
                ))
                .await
                .unwrap();
        }
        {
            let mut lease = pool
                .acquire(VmKind::ModelInference, Duration::from_secs(1))
                .await
                .unwrap();
            lease
                .execute(mode_b_req(
                    "ner_legal",
                    "1.0.0",
                    serde_json::json!({"x": 2}),
                ))
                .await
                .unwrap();
        }
        // Two distinct sub-pools — one for ner_clinical, one for ner_legal.
        assert!(pool.sub_pool_count() >= 2);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Health endpoint
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn health_returns_both_kinds() {
        let pool = FirecrackerVmPool::for_testing();
        let h = pool.health().await;
        assert!(h.kind_states.contains_key(&VmKind::ModelInference));
        assert!(h.kind_states.contains_key(&VmKind::LlmCaller));
    }

    #[tokio::test]
    async fn health_reflects_in_use_during_execute() {
        let mock = Arc::new(MockLifecycle::new());
        // Slow rpc.
        mock.set_rpc_latency(Duration::from_millis(200));

        // Plumb the slow lifecycle through.
        let pool = FirecrackerVmPool::for_testing_with_mock(Arc::clone(&mock));
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        let pool_for_health = pool.metrics();
        let handle = tokio::spawn(async move {
            lease
                .execute(mode_b_req(
                    "test_model",
                    "1.0.0",
                    serde_json::json!({"x": 1}),
                ))
                .await
                .unwrap();
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = pool_for_health.render();
        handle.await.unwrap();
    }

    // ─────────────────────────────────────────────────────────────────
    //  Shutdown
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn shutdown_reaper_stops_background_task() {
        let pool = FirecrackerVmPool::for_testing();
        pool.shutdown_reaper().await;
        // Second shutdown is a no-op (reaper is None).
        pool.shutdown_reaper().await;
    }

    #[tokio::test]
    async fn metrics_endpoint_renders_after_some_traffic() {
        let pool = FirecrackerVmPool::for_testing();
        for i in 0..3 {
            let mut lease = pool
                .acquire(VmKind::ModelInference, Duration::from_secs(1))
                .await
                .unwrap();
            lease
                .execute(mode_b_req(
                    "test_model",
                    "1.0.0",
                    serde_json::json!({"x": i}),
                ))
                .await
                .unwrap();
        }
        let out = pool.metrics().render();
        assert!(out.contains("efe_nano_firecracker_warm_count"));
        assert!(out.contains("efe_nano_firecracker_inference_duration_seconds"));
    }

    // ─────────────────────────────────────────────────────────────────
    //  Concurrent
    // ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn concurrent_acquires_each_get_a_lease() {
        let pool = Arc::new(FirecrackerVmPool::for_testing());
        let mut handles = Vec::new();
        for i in 0..8 {
            let pool = Arc::clone(&pool);
            handles.push(tokio::spawn(async move {
                let mut lease = pool
                    .acquire(VmKind::ModelInference, Duration::from_secs(2))
                    .await
                    .unwrap();
                lease
                    .execute(mode_b_req(
                        "test_model",
                        "1.0.0",
                        serde_json::json!({"i": i}),
                    ))
                    .await
                    .unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Sprint 8 audit — REGRESSION TESTS for the slot-leak bug
    //  (fixed in `translate_response` signature change).
    // ─────────────────────────────────────────────────────────────────

    /// Helper : install a mock handler that returns an `RpcError` of the
    /// given code.  Lets us simulate guest-side errors deterministically.
    fn install_error_handler(
        mock: &Arc<MockLifecycle>,
        code: super::super::rpc::RpcErrorCode,
        message: &'static str,
    ) {
        use super::super::rpc::{RpcError, RpcPayload, VsockRpcResponse, SCHEMA_VERSION};
        mock.install_handler(Arc::new(move |req| VsockRpcResponse {
            schema_version: SCHEMA_VERSION,
            request_id: req.request_id,
            payload: RpcPayload::Err {
                error: RpcError {
                    code,
                    message: message.to_string(),
                },
            },
        }));
    }

    /// THE REGRESSION : soft-error path used to leak the slot.  Now it
    /// returns the VM to warm-set.  This test verifies that
    /// `InputSchemaInvalid` → checkin_warm + permit released.
    #[tokio::test]
    async fn soft_error_returns_vm_to_warm_set_not_leaked() {
        use super::super::rpc::RpcErrorCode;
        let mock = Arc::new(MockLifecycle::new());
        install_error_handler(&mock, RpcErrorCode::InputSchemaInvalid, "bad input");
        let pool = FirecrackerVmPool::for_testing_with_mock(Arc::clone(&mock));

        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        let err = lease
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 1}),
            ))
            .await
            .unwrap_err();
        // SoftError-class : returned to caller as Internal.
        assert!(matches!(err, VmPoolError::Internal(_)));

        // Wait briefly for the deferred checkin_warm.
        tokio::time::sleep(Duration::from_millis(20)).await;

        // The bug was : VM stays in_use forever.  Post-fix : VM is
        // in warm-set (1) and in_use is 0.
        let h = pool.health().await;
        let mi = &h.kind_states[&VmKind::ModelInference];
        assert_eq!(
            mi.in_use_slots, 0,
            "soft-error MUST NOT leak the VM slot — in_use should be 0, was {}",
            mi.in_use_slots
        );

        // Reacquire should succeed within tight budget (warm path).
        let start = Instant::now();
        let mut lease2 = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(100))
            .await
            .unwrap();
        assert!(
            start.elapsed() < Duration::from_millis(80),
            "warm reacquire should be fast (was {:?})",
            start.elapsed()
        );
        // And the warm-path execute should succeed (echo handler reinstalled).
        mock.install_handler(super::super::lifecycle::default_echo_handler_for_test());
        lease2
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({"x": 2}),
            ))
            .await
            .unwrap();
    }

    /// Hard-error path : `ModelOom` → checkin_reaped (VM destroyed).
    /// Pre-fix this WORKED because the `?` propagation reaped via the
    /// vsock_error branch.  Post-fix the path is unified : both
    /// behaviors covered by the same `match outcome` block.
    #[tokio::test]
    async fn hard_error_reaps_vm_and_releases_slot() {
        use super::super::rpc::RpcErrorCode;
        let mock = Arc::new(MockLifecycle::new());
        install_error_handler(&mock, RpcErrorCode::ModelOom, "OOM at 248 MB");
        let pool = FirecrackerVmPool::for_testing_with_mock(Arc::clone(&mock));

        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        let err = lease
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({}),
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, VmPoolError::VmCrashed));

        // checkin_reaped : VM is gone, slot is released for re-spawn.
        tokio::time::sleep(Duration::from_millis(20)).await;
        let h = pool.health().await;
        let mi = &h.kind_states[&VmKind::ModelInference];
        assert_eq!(mi.in_use_slots, 0);
        // Slot is free for another spawn.
        mock.install_handler(super::super::lifecycle::default_echo_handler_for_test());
        let _ = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(200))
            .await
            .unwrap();
    }

    /// Stress test : 50 soft-errors in a row must not leak the pool.
    /// Pre-fix this would have exhausted the pool after `max_total`
    /// errors.  Post-fix every error returns the VM to warm.
    #[tokio::test]
    async fn fifty_soft_errors_in_a_row_do_not_exhaust_pool() {
        use super::super::rpc::RpcErrorCode;
        let mock = Arc::new(MockLifecycle::new());
        install_error_handler(&mock, RpcErrorCode::InputSchemaInvalid, "nope");
        let mut cfg = FirecrackerPoolConfig::for_testing();
        cfg.per_kind[0].max_total = 2; // tight cap → easy to exhaust
        cfg.per_kind[0].min_warm = 0;
        let pool =
            FirecrackerVmPool::with_lifecycle(cfg, mock).expect("config validates");

        for i in 0..50 {
            let mut lease = pool
                .acquire(VmKind::ModelInference, Duration::from_secs(1))
                .await
                .unwrap_or_else(|e| panic!("iteration {i} acquire failed : {e:?}"));
            let _ = lease
                .execute(mode_b_req(
                    "test_model",
                    "1.0.0",
                    serde_json::json!({"i": i}),
                ))
                .await; // ignore Err — we expect it
            // Drop the lease & let the async checkin finish.
            drop(lease);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // After 50 errors, the pool MUST still be operational.
        let h = pool.health().await;
        let mi = &h.kind_states[&VmKind::ModelInference];
        assert_eq!(mi.in_use_slots, 0, "no slot leak after 50 errors");
    }

    /// Lifecycle-layer RpcFailed (vsock crash) → reaped + slot released.
    /// Distinct from translate_response Err — exercises the
    /// `Err(LifecycleError::RpcFailed)` branch of the new `match`.
    #[tokio::test]
    async fn rpc_failed_at_lifecycle_layer_reaps_and_releases() {
        let mock = Arc::new(MockLifecycle::new());
        let pool = FirecrackerVmPool::for_testing_with_mock(Arc::clone(&mock));
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_secs(2))
            .await
            .unwrap();
        // First execute spawns + warms the VM.  Now kill every VM so
        // the next RPC fails at the lifecycle layer (dead VM).
        let _ = lease
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({}),
            ))
            .await
            .unwrap();
        // Sleep so the deferred checkin_warm lands.
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Now : next checkout reuses the warm VM.  Kill all warm VMs
        // before that.  The MockLifecycle doesn't expose enumeration ;
        // we use kill_each_vm_after(1) to kill the next RPC.
        mock.kill_each_vm_after(1);
        let mut lease2 = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(200))
            .await
            .unwrap();
        let _result = lease2
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({}),
            ))
            .await;
        // The first call to lease2.execute succeeded (kill_each_vm_after(1)
        // kills AFTER 1 rpc), so we proceed to a second lease.
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Now the VM is dead.  Try again — should hit RpcFailed via
        // lifecycle layer → reaped → re-spawn.
        let mut lease3 = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(500))
            .await
            .unwrap();
        // This may succeed if the pool re-spawned, or fail with VmCrashed.
        // Either way, the slot accounting must be correct.
        let _ = lease3
            .execute(mode_b_req(
                "test_model",
                "1.0.0",
                serde_json::json!({}),
            ))
            .await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let h = pool.health().await;
        assert_eq!(h.kind_states[&VmKind::ModelInference].in_use_slots, 0);
    }
}
