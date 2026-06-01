//! In-process stub VM pool — Phase 2 Sprint 4 ship.
//!
//! Runs Mode B inference **in-process** while preserving the production
//! `VmPool` contract :
//!
//! - per-kind permits via `tokio::sync::Semaphore`
//! - acquire blocks within `wait_budget` then errors with `AcquireTimeout`
//! - hit/miss tracking
//! - p99 provisioning + execution latency
//! - graceful release via RAII drop of the lease's executor (returns permit)
//!
//! The execution path uses a configurable `ModelLoader` injected at
//! construction time : the stub looks up the requested `model_id`/`version`
//! in the loader and calls `model.infer(input)`.
//!
//! For Mode C (LlmCaller kind) the stub returns
//! [`VmPoolError::ProvisionFailed`] with a clear "Mode C not yet wired"
//! message — Sprint 5 swaps in a real Mode C path.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::error::VmPoolError;
use crate::runtime::llm_client::LlmClient;
use crate::runtime::model_loader::ModelLoader;

use super::{
    PerKindHealth, VmExecutor, VmKind, VmLease, VmPool, VmPoolHealth, VmRequest, VmResponse,
};

/// Configuration for [`StubVmPool`].
#[derive(Debug, Clone)]
pub struct StubVmPoolConfig {
    pub mode_b_max: u32,
    pub mode_b_warm: u32,
    pub mode_c_max: u32,
    pub mode_c_warm: u32,
    /// Synthetic per-call latency injected for testing (default 0).
    /// In production this would be the actual model inference time.
    pub synthetic_latency: Duration,
}

impl Default for StubVmPoolConfig {
    fn default() -> Self {
        Self {
            mode_b_max: 200,
            mode_b_warm: 20,
            mode_c_max: 500,
            mode_c_warm: 50,
            synthetic_latency: Duration::from_millis(0),
        }
    }
}

/// Stub VM pool.
///
/// Implements [`VmPool`] entirely in-process : there is no microVM, no
/// vsock, no kernel boundary.  The pool tracks acquire/release/hit-rate
/// semantics with real `Semaphore` permits, but execution happens inline
/// in the calling task.
///
/// This is the **production ship for Phase 2** — real Firecracker is a
/// Sprint 8+ swap-in via a sibling impl of `VmPool`.
pub struct StubVmPool {
    config: StubVmPoolConfig,
    permits: HashMap<VmKind, Arc<Semaphore>>,
    loader: Arc<dyn ModelLoader>,
    /// Mode C only : injected LLM client.  `None` causes
    /// `VmKind::LlmCaller` requests to error with `ProvisionFailed`
    /// (Sprint 4 behavior preserved when the caller doesn't wire LLM).
    llm_client: Option<Arc<dyn LlmClient>>,
    metrics: Arc<Mutex<Metrics>>,
}

impl StubVmPool {
    /// Construct a stub VM pool with `loader` as the Mode B model source
    /// and no Mode C support.  `VmKind::LlmCaller` execute calls return
    /// `ProvisionFailed("no LlmClient wired")`.
    pub fn new(config: StubVmPoolConfig, loader: Arc<dyn ModelLoader>) -> Self {
        let mut permits = HashMap::new();
        permits.insert(
            VmKind::ModelInference,
            Arc::new(Semaphore::new(config.mode_b_max as usize)),
        );
        permits.insert(
            VmKind::LlmCaller,
            Arc::new(Semaphore::new(config.mode_c_max as usize)),
        );
        Self {
            config,
            permits,
            loader,
            llm_client: None,
            metrics: Arc::new(Mutex::new(Metrics::default())),
        }
    }

    /// Construct a stub VM pool with Mode B (model loader) + Mode C
    /// (LLM client) both wired.
    pub fn with_llm(
        config: StubVmPoolConfig,
        loader: Arc<dyn ModelLoader>,
        llm_client: Arc<dyn LlmClient>,
    ) -> Self {
        let mut pool = Self::new(config, loader);
        pool.llm_client = Some(llm_client);
        pool
    }

    /// Convenience : pool with default config + Phase 2 builtin loader,
    /// **no Mode C wired**.  Use `with_phase_2_defaults_and_llm` to
    /// enable Mode C.
    pub fn with_phase_2_defaults() -> Self {
        Self::new(
            StubVmPoolConfig::default(),
            Arc::new(crate::runtime::model_loader::InProcessModelLoader::with_phase_2_builtins()),
        )
    }

    /// Sprint 5 convenience : pool with default config + Phase 2
    /// builtin loader + Phase 2 builtin LLM client (Mode B + C ready).
    pub fn with_phase_2_defaults_and_llm() -> Self {
        Self::with_llm(
            StubVmPoolConfig::default(),
            Arc::new(crate::runtime::model_loader::InProcessModelLoader::with_phase_2_builtins()),
            Arc::new(crate::runtime::llm_client::BuiltinLlmClient::new()),
        )
    }
}

#[async_trait]
impl VmPool for StubVmPool {
    async fn acquire(
        &self,
        kind: VmKind,
        wait_budget: Duration,
    ) -> Result<VmLease, VmPoolError> {
        let started = Instant::now();
        let sem = self
            .permits
            .get(&kind)
            .ok_or_else(|| VmPoolError::Internal(format!("unknown VmKind {kind:?}")))?
            .clone();

        // Acquire a permit within budget.
        let permit = tokio::time::timeout(wait_budget, sem.acquire_owned())
            .await
            .map_err(|_| VmPoolError::AcquireTimeout(
                wait_budget.as_millis().min(u32::MAX as u128) as u32,
            ))?
            .map_err(|e| VmPoolError::Internal(format!("semaphore closed : {e}")))?;

        // For the stub, every acquire is treated as a "warm hit" because
        // we don't actually provision anything.  Sprint 8 Firecracker
        // version will distinguish warm vs cold here.
        {
            let mut m = self.metrics.lock().await;
            m.record_acquire(kind, true, started.elapsed());
        }

        Ok(VmLease {
            vm_id: uuid::Uuid::new_v4(),
            kind,
            acquired_at: Instant::now(),
            executor: Box::new(StubExecutor {
                kind,
                loader: Arc::clone(&self.loader),
                llm_client: self.llm_client.as_ref().map(Arc::clone),
                metrics: Arc::clone(&self.metrics),
                synthetic_latency: self.config.synthetic_latency,
                _permit: permit,
            }),
        })
    }

    async fn health(&self) -> VmPoolHealth {
        let metrics = self.metrics.lock().await;
        let mut kind_states = HashMap::new();
        for &kind in &[VmKind::ModelInference, VmKind::LlmCaller] {
            let max = match kind {
                VmKind::ModelInference => self.config.mode_b_max,
                VmKind::LlmCaller => self.config.mode_c_max,
            };
            let sem = self.permits.get(&kind).unwrap();
            let in_use = max as usize - sem.available_permits();
            kind_states.insert(
                kind,
                PerKindHealth {
                    warm_slots: sem.available_permits().min(max as usize) as u32,
                    in_use_slots: in_use as u32,
                    max_slots: max,
                    hit_rate_60s: metrics.hit_rate(kind),
                    p99_provision_ms: metrics.p99_provision_ms(kind),
                    total_acquires: metrics.total_acquires(kind),
                    total_executes: metrics.total_executes(kind),
                },
            );
        }
        VmPoolHealth { kind_states }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Internal executor
// ─────────────────────────────────────────────────────────────────────

struct StubExecutor {
    kind: VmKind,
    loader: Arc<dyn ModelLoader>,
    /// Mode C executor : injected on `with_llm` constructors.
    llm_client: Option<Arc<dyn LlmClient>>,
    metrics: Arc<Mutex<Metrics>>,
    synthetic_latency: Duration,
    _permit: OwnedSemaphorePermit,
}

#[async_trait]
impl VmExecutor for StubExecutor {
    async fn execute(&mut self, req: VmRequest) -> Result<VmResponse, VmPoolError> {
        let started = Instant::now();
        let timeout = Duration::from_millis(req.timeout_ms.max(1) as u64);

        // Mode B path : look up model + infer.
        let result = match self.kind {
            VmKind::ModelInference => {
                let model_id = req.model_id.as_deref().ok_or_else(|| {
                    VmPoolError::Internal(
                        "ModelInference request missing model_id".into(),
                    )
                })?;
                let model_version = req.model_version.as_deref().unwrap_or("1.0.0");

                let loader = Arc::clone(&self.loader);
                let model_id_owned = model_id.to_string();
                let model_version_owned = model_version.to_string();
                let input_clone = req.input.clone();
                let synthetic_latency = self.synthetic_latency;

                // Wrap the (optional synthetic delay + model lookup + infer)
                // in a single timeout.  Synthetic latency must be INSIDE the
                // timeout so a slow inference correctly trips it.
                let fut = async move {
                    if !synthetic_latency.is_zero() {
                        tokio::time::sleep(synthetic_latency).await;
                    }
                    let model = loader
                        .get_or_load(&model_id_owned, &model_version_owned)
                        .await
                        .map_err(|e| VmPoolError::ProvisionFailed(format!(
                            "model loader : {e}"
                        )))?;
                    let out = model.infer(input_clone).await.map_err(|e| {
                        VmPoolError::Internal(format!("inference : {e}"))
                    })?;
                    Ok::<_, VmPoolError>(out)
                };

                match tokio::time::timeout(timeout, fut).await {
                    Ok(Ok(output)) => Ok(VmResponse::mode_b(
                        output,
                        started.elapsed().as_millis().min(u32::MAX as u128) as u32,
                        0,
                    )),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(VmPoolError::AcquireTimeout(req.timeout_ms)),
                }
            }

            VmKind::LlmCaller => {
                let client = self.llm_client.clone().ok_or_else(|| {
                    VmPoolError::ProvisionFailed(
                        "Mode C requested but no LlmClient wired in pool".into(),
                    )
                })?;
                let llm_req = req.llm_call_request.clone().ok_or_else(|| {
                    VmPoolError::Internal(
                        "LlmCaller request missing llm_call_request".into(),
                    )
                })?;
                let synthetic_latency = self.synthetic_latency;
                let fut = async move {
                    if !synthetic_latency.is_zero() {
                        tokio::time::sleep(synthetic_latency).await;
                    }
                    client.call(llm_req).await.map_err(|e| {
                        VmPoolError::Internal(format!("LLM call : {e}"))
                    })
                };
                match tokio::time::timeout(timeout, fut).await {
                    Ok(Ok(llm_resp)) => Ok(VmResponse {
                        output: llm_resp.content,
                        cpu_ms_used: started.elapsed().as_millis().min(u32::MAX as u128) as u32,
                        memory_mb_peak: 0,
                        llm_tokens_used: Some(llm_resp.tokens_in + llm_resp.tokens_out),
                        llm_cost_cents: Some(llm_resp.cost_cents),
                        llm_provider: Some(llm_resp.provider_name),
                        llm_model: Some(llm_resp.model_name),
                    }),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(VmPoolError::AcquireTimeout(req.timeout_ms)),
                }
            }
        };

        if result.is_ok() {
            self.metrics.lock().await.record_execute(self.kind);
        }
        result
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Metrics tracking (60s-windowed hit rate + p99)
// ─────────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Metrics {
    /// Total acquires per kind (since pool start).
    acquires: HashMap<VmKind, u64>,
    /// Total executes (success) per kind.
    executes: HashMap<VmKind, u64>,
    /// Per-acquire (kind, instant, was_hit) — pruned to last 60s.
    recent: Vec<(VmKind, Instant, bool, Duration)>,
}

impl Metrics {
    fn record_acquire(&mut self, kind: VmKind, was_hit: bool, wait: Duration) {
        *self.acquires.entry(kind).or_insert(0) += 1;
        self.recent.push((kind, Instant::now(), was_hit, wait));
        self.prune();
    }

    fn record_execute(&mut self, kind: VmKind) {
        *self.executes.entry(kind).or_insert(0) += 1;
    }

    fn prune(&mut self) {
        let cutoff = Instant::now() - Duration::from_secs(60);
        self.recent.retain(|(_, t, _, _)| *t >= cutoff);
    }

    fn hit_rate(&self, kind: VmKind) -> f32 {
        let cutoff = Instant::now() - Duration::from_secs(60);
        let (hits, misses) = self
            .recent
            .iter()
            .filter(|(k, t, _, _)| *k == kind && *t >= cutoff)
            .fold((0u32, 0u32), |(h, m), (_, _, was_hit, _)| {
                if *was_hit { (h + 1, m) } else { (h, m + 1) }
            });
        let total = hits + misses;
        if total == 0 { 1.0 } else { hits as f32 / total as f32 }
    }

    fn p99_provision_ms(&self, kind: VmKind) -> u32 {
        let cutoff = Instant::now() - Duration::from_secs(60);
        let mut times: Vec<u32> = self
            .recent
            .iter()
            .filter(|(k, t, _, _)| *k == kind && *t >= cutoff)
            .map(|(_, _, _, wait)| wait.as_millis().min(u32::MAX as u128) as u32)
            .collect();
        if times.is_empty() {
            return 0;
        }
        times.sort_unstable();
        let idx = ((times.len() as f32) * 0.99) as usize;
        times[idx.min(times.len() - 1)]
    }

    fn total_acquires(&self, kind: VmKind) -> u64 {
        *self.acquires.get(&kind).unwrap_or(&0)
    }

    fn total_executes(&self, kind: VmKind) -> u64 {
        *self.executes.get(&kind).unwrap_or(&0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::model_loader::InProcessModelLoader;
    use serde_json::json;

    fn make_pool() -> StubVmPool {
        StubVmPool::with_phase_2_defaults()
    }

    fn make_request_clinical() -> VmRequest {
        VmRequest::mode_b(
            "ner_clinical",
            "1.0.0",
            json!({"text": "metformin for diabetes"}),
            5_000,
            "ner_clinical_v1",
            "1.0.0",
        )
    }

    #[tokio::test]
    async fn acquire_and_execute_succeeds() {
        let pool = make_pool();
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(500))
            .await
            .unwrap();
        let resp = lease.execute(make_request_clinical()).await.unwrap();
        let entities = resp.output["entities"].as_array().unwrap();
        assert!(!entities.is_empty());
    }

    #[tokio::test]
    async fn acquire_timeout_when_pool_exhausted() {
        let pool = Arc::new(StubVmPool::new(
            StubVmPoolConfig {
                mode_b_max: 1,
                mode_b_warm: 0,
                mode_c_max: 1,
                mode_c_warm: 0,
                synthetic_latency: Duration::from_millis(0),
            },
            Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        ));

        // Hold the only permit.
        let _held = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap();
        // Next acquire times out.
        let err = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap_err();
        match err {
            VmPoolError::AcquireTimeout(_) => {}
            other => panic!("expected AcquireTimeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn lease_drop_releases_permit() {
        let pool = Arc::new(StubVmPool::new(
            StubVmPoolConfig {
                mode_b_max: 1,
                mode_b_warm: 0,
                mode_c_max: 1,
                mode_c_warm: 0,
                synthetic_latency: Duration::from_millis(0),
            },
            Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        ));

        {
            let _lease = pool
                .acquire(VmKind::ModelInference, Duration::from_millis(50))
                .await
                .unwrap();
            // permit is held by _lease
        } // lease drops here → permit returns

        // Should now be able to acquire again.
        let _again = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn kind_isolation_separate_permits() {
        let pool = Arc::new(StubVmPool::new(
            StubVmPoolConfig {
                mode_b_max: 1,
                mode_b_warm: 0,
                mode_c_max: 1,
                mode_c_warm: 0,
                synthetic_latency: Duration::from_millis(0),
            },
            Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        ));

        // Hold both permits.
        let _b = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap();
        let _c = pool
            .acquire(VmKind::LlmCaller, Duration::from_millis(50))
            .await
            .unwrap();
        // Both held without exhausting each other.
    }

    #[tokio::test]
    async fn llm_caller_without_client_wired_returns_provision_failed() {
        // make_pool() uses with_phase_2_defaults() which does NOT wire an LLM client.
        let pool = make_pool();
        let mut lease = pool
            .acquire(VmKind::LlmCaller, Duration::from_millis(50))
            .await
            .unwrap();
        let req = VmRequest {
            agent_id: "summarize".into(),
            agent_version: "1.0.0".into(),
            input: json!({"text": "foo"}),
            timeout_ms: 1_000,
            model_id: None,
            model_version: None,
            llm_call_request: None,
        };
        let err = lease.execute(req).await.unwrap_err();
        match err {
            VmPoolError::ProvisionFailed(msg) => assert!(msg.contains("no LlmClient")),
            other => panic!("expected ProvisionFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn llm_caller_with_client_wired_executes_llm_call() {
        use crate::runtime::llm_client::{LlmCallRequest, ResponseFormat};

        // Use the Sprint 5 builtin pool : Mode B loader + Mode C LLM client.
        let pool = StubVmPool::with_phase_2_defaults_and_llm();
        let mut lease = pool
            .acquire(VmKind::LlmCaller, Duration::from_millis(500))
            .await
            .unwrap();
        let llm_req = LlmCallRequest::builder(
            efe_nano_sdk::TenantId::system(),
            "m50.classify_topic.v1",
        )
        .template_inputs(json!({"text": "stock market today"}))
        .max_tokens(200)
        .temperature(0.0)
        .response_format(ResponseFormat::json_schema(json!({"type": "object"})))
        .build();
        let req = VmRequest::mode_c(
            "classify_topic_llm",
            "1.0.0",
            json!({"text": "stock market today"}),
            5_000,
            llm_req,
        );
        let resp = lease.execute(req).await.unwrap();
        assert_eq!(resp.output["topic"], "finance");
        assert!(resp.llm_tokens_used.is_some());
        assert!(resp.llm_cost_cents.is_some());
        assert_eq!(resp.llm_provider.as_deref(), Some("builtin"));
    }

    #[tokio::test]
    async fn llm_caller_with_missing_llm_call_request_is_internal_error() {
        let pool = StubVmPool::with_phase_2_defaults_and_llm();
        let mut lease = pool
            .acquire(VmKind::LlmCaller, Duration::from_millis(50))
            .await
            .unwrap();
        let req = VmRequest {
            agent_id: "x".into(),
            agent_version: "1.0.0".into(),
            input: json!({}),
            timeout_ms: 1_000,
            model_id: None,
            model_version: None,
            llm_call_request: None,
        };
        let err = lease.execute(req).await.unwrap_err();
        match err {
            VmPoolError::Internal(msg) => assert!(msg.contains("missing llm_call_request")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_timeout_surfaces_via_pool_error() {
        // Set a synthetic latency higher than the request timeout.
        let pool = Arc::new(StubVmPool::new(
            StubVmPoolConfig {
                mode_b_max: 1,
                mode_b_warm: 0,
                mode_c_max: 1,
                mode_c_warm: 0,
                synthetic_latency: Duration::from_millis(100),
            },
            Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        ));
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap();
        let mut req = make_request_clinical();
        req.timeout_ms = 10;  // < synthetic_latency 100 ms
        let err = lease.execute(req).await.unwrap_err();
        assert!(matches!(err, VmPoolError::AcquireTimeout(_)));
    }

    #[tokio::test]
    async fn health_reports_metrics() {
        let pool = make_pool();
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap();
        let _ = lease.execute(make_request_clinical()).await.unwrap();
        drop(lease);

        let h = pool.health().await;
        let mode_b = h.kind_states.get(&VmKind::ModelInference).unwrap();
        assert_eq!(mode_b.total_acquires, 1);
        assert_eq!(mode_b.total_executes, 1);
        assert_eq!(mode_b.max_slots, 200);
    }

    #[tokio::test]
    async fn parallel_acquires_succeed_up_to_max() {
        // Pool with capacity 10. Acquire 10 in parallel — all succeed.
        let pool = Arc::new(StubVmPool::new(
            StubVmPoolConfig {
                mode_b_max: 10,
                mode_b_warm: 0,
                mode_c_max: 10,
                mode_c_warm: 0,
                synthetic_latency: Duration::from_millis(0),
            },
            Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        ));

        let mut handles = Vec::new();
        for _ in 0..10 {
            let pool = Arc::clone(&pool);
            handles.push(tokio::spawn(async move {
                pool.acquire(VmKind::ModelInference, Duration::from_millis(500))
                    .await
                    .unwrap()
            }));
        }
        let mut leases = Vec::new();
        for h in handles {
            leases.push(h.await.unwrap());
        }
        assert_eq!(leases.len(), 10);
    }

    #[tokio::test]
    async fn unknown_model_id_surfaces_as_provision_failed() {
        let pool = make_pool();
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap();
        let req = VmRequest::mode_b(
            "ghost",
            "1.0.0",
            json!({"text": "hi"}),
            1_000,
            "ghost_model_v1",
            "1.0.0",
        );
        let err = lease.execute(req).await.unwrap_err();
        match err {
            VmPoolError::ProvisionFailed(msg) => assert!(msg.contains("model loader")),
            other => panic!("expected ProvisionFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn missing_model_id_for_mode_b_returns_internal() {
        let pool = make_pool();
        let mut lease = pool
            .acquire(VmKind::ModelInference, Duration::from_millis(50))
            .await
            .unwrap();
        let req = VmRequest {
            agent_id: "x".into(),
            agent_version: "1.0.0".into(),
            input: json!({}),
            timeout_ms: 1_000,
            model_id: None,
            model_version: None,
            llm_call_request: None,
        };
        let err = lease.execute(req).await.unwrap_err();
        assert!(matches!(err, VmPoolError::Internal(_)));
    }
}
