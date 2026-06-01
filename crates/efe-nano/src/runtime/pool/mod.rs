//! VM pool — pool of warm execution slots for Mode B (and Mode C in Sprint 5).
//!
//! The pool abstraction exists so that Mode B/C can be migrated from
//! "in-process stub" (Phase 2 Sprint 4 ship) to "real Firecracker microVM"
//! (Sprint 8+) without touching the executor or the dispatcher.
//!
//! ## Phase 2 Sprint 4 ship : [`stub::StubVmPool`]
//!
//! Executes Mode B inference **in-process** — no microVM kernel boundary,
//! but real :
//!
//! - Acquire / release semantics with a tokio `Semaphore` per [`VmKind`].
//! - Permits respect the `max_slots` cap per kind.
//! - Hit / miss tracking (every acquire is recorded as a "warm hit" since
//!   the stub always reuses the same logical slot).
//! - Timeout via `tokio::time::timeout`.
//!
//! ## Sprint 8 swap-in : `FirecrackerVmPool`
//!
//! A future module `pool::firecracker` will impl [`VmPool`] by wrapping
//! `efe-agent-runner` (M25).  Acquire then provisions or reuses a real
//! microVM ; execute sends the inference request over vsock + reads the
//! response.  Same trait surface ; the dispatcher doesn't care.

/// **Sprint 8 — N11** : production Firecracker pool.  Same `VmPool`
/// trait surface as `StubVmPool` ; the dispatcher is unchanged.
/// See `firecracker::FirecrackerVmPool::for_testing()` for the test
/// path that exercises the deferred-acquire pattern end-to-end without
/// requiring KVM.
pub mod firecracker;
pub mod stub;

pub use firecracker::FirecrackerVmPool;
pub use stub::StubVmPool;

// ─────────────────────────────────────────────────────────────────────
//  Backend selection
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : env-var-selected pool backend.
///
/// `EFE_NANO_VM_BACKEND` :
/// - `stub` (default)  → [`StubVmPool`] (Sprint 4 ship)
/// - `firecracker`     → [`FirecrackerVmPool`] (Sprint 8 N11)
pub fn make_pool_from_env(
) -> Result<std::sync::Arc<dyn VmPool>, Box<dyn std::error::Error + Send + Sync>> {
    let backend =
        std::env::var("EFE_NANO_VM_BACKEND").unwrap_or_else(|_| "stub".to_string());
    match backend.as_str() {
        "firecracker" => {
            #[cfg(target_os = "linux")]
            {
                let config = firecracker::FirecrackerPoolConfig::from_env()?;
                let lifecycle: std::sync::Arc<dyn firecracker::Lifecycle> = std::sync::Arc::new(
                    firecracker::FirecrackerLifecycle::new(config.clone())?,
                );
                Ok(std::sync::Arc::new(
                    firecracker::FirecrackerVmPool::with_lifecycle(config, lifecycle).map_err(
                        |e| Box::<dyn std::error::Error + Send + Sync>::from(e.to_string()),
                    )?,
                ))
            }
            #[cfg(not(target_os = "linux"))]
            {
                Err(Box::from(
                    "firecracker backend requires Linux + KVM — \
                     set EFE_NANO_VM_BACKEND=stub on macOS dev",
                ))
            }
        }
        "stub" => {
            // The stub requires a ModelLoader ; the default in-process
            // loader has the Phase 2 builtins.  Mode C wiring is the
            // caller's responsibility (see `StubVmPool::with_llm`).
            Ok(std::sync::Arc::new(stub::StubVmPool::with_phase_2_defaults()))
        }
        other => Err(Box::from(format!("unknown VM backend : {other}"))),
    }
}

use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use crate::error::VmPoolError;

// ─────────────────────────────────────────────────────────────────────
//  Public traits + types
// ─────────────────────────────────────────────────────────────────────

/// Kind of VM requested. Pool keeps a separate warm-set per kind so that
/// e.g. Mode B can scale independently of Mode C.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum VmKind {
    /// Mode B : pre-loaded model microVM.
    ModelInference,
    /// Mode C : lightweight microVM with LLM HTTP client only.
    LlmCaller,
}

impl fmt::Display for VmKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelInference => f.write_str("ModelInference"),
            Self::LlmCaller => f.write_str("LlmCaller"),
        }
    }
}

/// One inference request through the pool.
#[derive(Debug, Clone)]
pub struct VmRequest {
    /// Agent id (for telemetry + lookup).
    pub agent_id: String,
    /// Agent version (for telemetry).
    pub agent_version: String,
    /// Validated input JSON (input_schema already passed).
    pub input: serde_json::Value,
    /// Wall-clock budget for this execution in milliseconds.
    pub timeout_ms: u32,
    /// Mode B only : the model id + version to invoke.  None for Mode C.
    pub model_id: Option<String>,
    pub model_version: Option<String>,
    /// Mode C only : the fully-resolved LLM call request.  None for Mode B.
    pub llm_call_request: Option<crate::runtime::llm_client::LlmCallRequest>,
}

impl VmRequest {
    /// Convenience : Mode B request constructor.
    pub fn mode_b(
        agent_id: impl Into<String>,
        agent_version: impl Into<String>,
        input: serde_json::Value,
        timeout_ms: u32,
        model_id: impl Into<String>,
        model_version: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            agent_version: agent_version.into(),
            input,
            timeout_ms,
            model_id: Some(model_id.into()),
            model_version: Some(model_version.into()),
            llm_call_request: None,
        }
    }

    /// Convenience : Mode C request constructor.
    pub fn mode_c(
        agent_id: impl Into<String>,
        agent_version: impl Into<String>,
        input: serde_json::Value,
        timeout_ms: u32,
        llm_call_request: crate::runtime::llm_client::LlmCallRequest,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            agent_version: agent_version.into(),
            input,
            timeout_ms,
            model_id: None,
            model_version: None,
            llm_call_request: Some(llm_call_request),
        }
    }
}

/// One inference response from the pool.
#[derive(Debug, Clone)]
pub struct VmResponse {
    /// Output JSON (caller will validate against output_schema).
    pub output: serde_json::Value,
    /// CPU time consumed (ms) — best-effort measurement.
    pub cpu_ms_used: u32,
    /// Peak memory usage (MB) — best-effort measurement.
    pub memory_mb_peak: u32,
    /// Mode C only : tokens consumed by the LLM call (in + out).  None
    /// for Mode B.
    pub llm_tokens_used: Option<u32>,
    /// Mode C only : provider-reported cost in cents.  None for Mode B.
    pub llm_cost_cents: Option<f64>,
    /// Mode C only : provider name (e.g. "anthropic").  None for Mode B.
    pub llm_provider: Option<String>,
    /// Mode C only : model name (e.g. "claude-sonnet-4-5").  None for
    /// Mode B.
    pub llm_model: Option<String>,
}

impl VmResponse {
    /// Construct a Mode B response (no LLM metadata).
    pub fn mode_b(output: serde_json::Value, cpu_ms_used: u32, memory_mb_peak: u32) -> Self {
        Self {
            output,
            cpu_ms_used,
            memory_mb_peak,
            llm_tokens_used: None,
            llm_cost_cents: None,
            llm_provider: None,
            llm_model: None,
        }
    }
}

/// VmPool trait. See module-level doc.
#[async_trait]
pub trait VmPool: Send + Sync + 'static {
    /// Acquire a warm VM. Blocks up to `wait_budget` waiting for capacity
    /// (returns [`VmPoolError::AcquireTimeout`] if budget exceeded).
    /// Cold-path provisioning happens within `wait_budget`.
    async fn acquire(
        &self,
        kind: VmKind,
        wait_budget: Duration,
    ) -> Result<VmLease, VmPoolError>;

    /// Pool health snapshot for telemetry.
    async fn health(&self) -> VmPoolHealth;
}

/// RAII lease for an acquired VM.
///
/// Drop releases the permit + returns the VM to the warm-set (if the pool
/// is below `max_slots` for that kind), or destroys the VM otherwise.
pub struct VmLease {
    /// Stable identifier for telemetry + audit.
    pub vm_id: uuid::Uuid,
    pub kind: VmKind,
    /// When `acquire()` succeeded — used for "queue wait time" telemetry.
    pub acquired_at: Instant,

    /// The actual executor — boxed so the pool implementation can swap
    /// between stub / Firecracker / etc.
    pub(crate) executor: Box<dyn VmExecutor>,
}

impl VmLease {
    /// Execute a request inside this leased VM.
    pub async fn execute(
        &mut self,
        req: VmRequest,
    ) -> Result<VmResponse, VmPoolError> {
        self.executor.execute(req).await
    }

    /// How long this lease has been held (since `acquire` returned).
    pub fn hold_duration(&self) -> Duration {
        self.acquired_at.elapsed()
    }
}

impl fmt::Debug for VmLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VmLease")
            .field("vm_id", &self.vm_id)
            .field("kind", &self.kind)
            .field("acquired_at", &self.acquired_at)
            .finish_non_exhaustive()
    }
}

/// Internal trait : per-kind execution.  Implemented by both `StubVmPool`'s
/// internal executor and (Sprint 8+) `FirecrackerVmPool`'s vsock-based
/// executor.
#[async_trait]
pub(crate) trait VmExecutor: Send + Sync {
    async fn execute(&mut self, req: VmRequest) -> Result<VmResponse, VmPoolError>;
}

/// Pool health snapshot.
#[derive(Debug, Clone)]
pub struct VmPoolHealth {
    pub kind_states: HashMap<VmKind, PerKindHealth>,
}

#[derive(Debug, Clone)]
pub struct PerKindHealth {
    /// Number of warm (idle) slots currently available.
    pub warm_slots: u32,
    /// Number of slots currently in use (leased out).
    pub in_use_slots: u32,
    /// Maximum slots configured for this kind.
    pub max_slots: u32,
    /// Hit rate over the last 60s window ([0.0, 1.0]).
    pub hit_rate_60s: f32,
    /// p99 provisioning time (ms) — relevant only on cold path.
    pub p99_provision_ms: u32,
    /// Total acquires since pool start.
    pub total_acquires: u64,
    /// Total successful executes since pool start.
    pub total_executes: u64,
}
