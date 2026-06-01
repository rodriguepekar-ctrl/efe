//! [`Runtime`] trait — executes nano agents.
//!
//! The SDK defines the trait + the [`NanoAgentResponse`] type ; the
//! actual implementations (Mode A/B/C) live in the `efe-nano` runtime crate.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::NanoAgent;

/// Trace identifier for an individual nano agent call.
///
/// **Sprint 8 — N14** : now `Serialize + Deserialize` so it can be
/// embedded in `SwarmRequest.parent_envelope_trace_id` /
/// `SwarmResponse.swarm_envelope_trace_id` / `SwarmEnvelopePayload`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraceId(pub Uuid);

impl TraceId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TraceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-call response from the runtime.
#[derive(Debug, Clone)]
pub struct NanoAgentResponse {
    pub output: serde_json::Value,
    pub latency_ms: u32,
    pub cost_cents: f64,
    pub trace_id: TraceId,
    pub cache_hit: bool,
}

/// Health summary of the runtime layer.
#[derive(Debug, Clone)]
pub struct RuntimeHealth {
    /// Is Mode A executor available ?  (always true in Phase 1+)
    pub mode_a_alive: bool,
    /// Is Mode B executor available ?  (Phase 2 Sprint 4+)
    pub mode_b_alive: bool,
    /// Is Mode C executor available ?  (Phase 2 Sprint 5+)
    pub mode_c_alive: bool,
    /// Pool metrics if applicable.
    pub pool: PoolHealth,
}

#[derive(Debug, Clone)]
pub struct PoolHealth {
    pub warm_slots: u32,
    pub in_use_slots: u32,
    pub max_slots: u32,
}

/// Result type for runtime operations.
pub type RuntimeResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Executes nano agents.
///
/// Implementations :
/// - `PureRustRuntime` (Mode A, Phase 1).
/// - `LocalModelRuntime` (Mode B, Phase 2 Sprint 4).
/// - `LlmAugmentedRuntime` (Mode C, Phase 2 Sprint 5).
/// - Composite `MultiModeRuntime` that dispatches based on the agent's [`crate::types::ExecutionMode`].
#[async_trait]
pub trait Runtime: Send + Sync + 'static {
    /// Execute a single nano agent.
    ///
    /// The runtime is responsible for :
    /// - Validating input against `agent.input_schema`.
    /// - Invoking the agent implementation.
    /// - Validating output against `agent.output_schema`.
    /// - Enforcing `agent.limits`.
    /// - Recording an audit trace.
    /// - Returning a [`NanoAgentResponse`].
    async fn execute(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
    ) -> RuntimeResult<NanoAgentResponse>;

    /// Health check.
    async fn health(&self) -> RuntimeHealth;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_id_unique() {
        let a = TraceId::new();
        let b = TraceId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn trace_id_default() {
        let _: TraceId = Default::default();  // compiles
    }

    #[test]
    fn pool_health_construction() {
        let p = PoolHealth {
            warm_slots: 10,
            in_use_slots: 2,
            max_slots: 50,
        };
        assert_eq!(p.warm_slots, 10);
    }
}
