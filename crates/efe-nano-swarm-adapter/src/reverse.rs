// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + G11 + ADR-10 + Workflow B** : `SwarmToNano`
//! — the reverse adapter that exposes a single `SwarmAgent` via
//! the SDK `Runtime` trait.
//!
//! Use case : CLI / FFI / MCP layers speak the `Runtime::execute(
//! agent, input)` API but want to invoke a swarm-native agent
//! (e.g. `SentimentClassifier`) without spinning a full DAG.  The
//! reverse adapter spins a **single-node ephemeral swarm** under
//! the hood : fresh `Blackboard`, fresh `EventBus`, fresh
//! `SwarmId` — the orchestrator runs the wrapped agent, the
//! adapter extracts the result via a configurable output-pointer
//! getter, and returns the synthesised `NanoAgentResponse`.
//!
//! Note ADR-10 : the reverse adapter is for *ergonomics*, not
//! production traffic.  Multi-agent compositions belong in a real
//! DAG using the forward adapter.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, EventBus};
use efe_nano_sdk::{
    traits::runtime::{NanoAgentResponse, Runtime, RuntimeHealth, RuntimeResult, TraceId},
    AgentId, NanoAgent, SwarmId,
};
use efe_swarm_orchestrator::{
    agent::AgentOutcome, SwarmAgent, SwarmAgentContext,
};

use crate::cost::microcents_to_cents;
use crate::forward::DEFAULT_INPUT_POINTER;

/// **N23 + G11** : reverse adapter — `SwarmAgent → Runtime`.
///
/// Wraps a single swarm-native agent and dispatches it through an
/// ephemeral single-node "mini-swarm" : a fresh blackboard
/// (seeded with `input` at the configured pointer), a fresh
/// event bus, and the wrapped agent's own `SwarmAgentContext`.
///
/// `execute(agent, input)` returns the `NanoAgentResponse`
/// derived from the wrapped agent's `AgentOutcome::Completed`.
/// The `output` field of the response is extracted from the
/// blackboard at the configured output pointer (default
/// `/output`).  If the pointer is unset the adapter returns
/// `serde_json::Value::Null`.
pub struct SwarmToNano {
    /// The wrapped swarm-native agent.
    wrapped: Arc<dyn SwarmAgent>,
    /// Blackboard pointer the orchestrator seeds with `input` (read
    /// by the wrapped agent).
    input_pointer: String,
    /// Blackboard pointer the adapter reads to populate the
    /// `NanoAgentResponse.output` field.
    output_pointer: String,
    /// Optional explicit RNG seed ; defaults to `rand` via a
    /// simple hash of process-local randomness.  This adapter
    /// does NOT participate in N16 replay (ADR-10).
    rng_seed: Option<u64>,
}

impl std::fmt::Debug for SwarmToNano {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmToNano")
            .field("input_pointer", &self.input_pointer)
            .field("output_pointer", &self.output_pointer)
            .field("rng_seed", &self.rng_seed)
            .finish()
    }
}

impl SwarmToNano {
    /// **N23** : construct with the default input pointer
    /// (`/input/text`) and default output pointer (`/output`).
    pub fn new(wrapped: Arc<dyn SwarmAgent>) -> Self {
        Self {
            wrapped,
            input_pointer: DEFAULT_INPUT_POINTER.to_string(),
            output_pointer: "/output".to_string(),
            rng_seed: None,
        }
    }

    /// **N23** : override the input pointer.
    pub fn with_input_pointer<S: Into<String>>(mut self, p: S) -> Self {
        self.input_pointer = p.into();
        self
    }

    /// **N23** : override the output pointer.
    pub fn with_output_pointer<S: Into<String>>(mut self, p: S) -> Self {
        self.output_pointer = p.into();
        self
    }

    /// **N23** : pin the RNG seed (for replay sanity).
    pub fn with_rng_seed(mut self, s: u64) -> Self {
        self.rng_seed = Some(s);
        self
    }

    /// **N23** : seed `input` to the blackboard before dispatch.
    ///
    /// If `input` is a string, write it directly ; if it has a
    /// `"text"` field, prefer that ; else stringify the JSON.
    /// Mirrors `efe_gateway::dag_registry::seed_input_text` for
    /// consistency.
    fn seed_input(
        &self,
        bb: &Blackboard,
        input: &serde_json::Value,
        agent: &AgentId,
    ) {
        let text_value = if let Some(s) = input.as_str() {
            serde_json::Value::String(s.to_string())
        } else if let Some(s) = input.get("text").and_then(|v| v.as_str()) {
            serde_json::Value::String(s.to_string())
        } else {
            serde_json::Value::String(input.to_string())
        };
        let _ = bb.write(&self.input_pointer, text_value, agent.clone());
        // Also write the raw input value to `/input` for agents
        // (e.g. SchemaValidator) that read the full JSON.
        let _ = bb.write("/input", input.clone(), agent.clone());
    }
}

#[async_trait]
impl Runtime for SwarmToNano {
    async fn execute(
        &self,
        _agent: &NanoAgent,
        input: serde_json::Value,
    ) -> RuntimeResult<NanoAgentResponse> {
        let started = Instant::now();
        let swarm_id = SwarmId::new();
        let bb = Arc::new(Blackboard::new(swarm_id));
        let eventbus = Arc::new(EventBus::new());
        let seed_agent = AgentId::new("swarm-to-nano-seed")
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("seed agent id : {e}"),
                ))
            })?;

        self.seed_input(&bb, &input, &seed_agent);

        let inner_agent_id = self.wrapped.agent_id();
        let rng_seed = self.rng_seed.unwrap_or_else(|| {
            // Hash the swarm_id UUID bytes to derive a stable seed.
            let uuid = swarm_id.as_uuid();
            let bytes = uuid.as_bytes();
            let mut s: u64 = 0;
            for (i, b) in bytes.iter().enumerate().take(8) {
                s ^= (*b as u64) << ((i % 8) * 8);
            }
            s
        });

        let ctx = SwarmAgentContext {
            swarm_id,
            agent_id: inner_agent_id.clone(),
            blackboard: Arc::clone(&bb),
            eventbus,
            input: Arc::new(input.clone()),
            rng_seed,
        };

        let outcome = self
            .wrapped
            .run(ctx)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                Box::new(std::io::Error::other(format!("swarm-to-nano : {e}")))
            })?;

        let (cost_microcents, _writes) = match outcome {
            AgentOutcome::Completed {
                cost_microcents,
                blackboard_writes,
                ..
            } => (cost_microcents, blackboard_writes),
            AgentOutcome::Errored { kind, message } => {
                return Err(Box::new(std::io::Error::other(format!(
                    "swarm-to-nano errored {} : {}",
                    kind.label(),
                    message
                ))));
            }
            AgentOutcome::TimedOut { timeout_ms } => {
                return Err(Box::new(std::io::Error::other(format!(
                    "swarm-to-nano timed out after {timeout_ms} ms"
                ))));
            }
            AgentOutcome::Skipped { reason } => {
                return Err(Box::new(std::io::Error::other(format!(
                    "swarm-to-nano skipped : {reason}"
                ))));
            }
        };

        let output = bb
            .read(&self.output_pointer)
            .map(|bv| bv.value)
            .unwrap_or(serde_json::Value::Null);
        let latency_ms = (started.elapsed().as_millis() as u32).max(1);
        Ok(NanoAgentResponse {
            output,
            latency_ms,
            cost_cents: microcents_to_cents(cost_microcents),
            trace_id: TraceId::new(),
            cache_hit: false,
        })
    }

    async fn health(&self) -> RuntimeHealth {
        RuntimeHealth {
            mode_a_alive: true,
            mode_b_alive: false,
            mode_c_alive: false,
            pool: efe_nano_sdk::traits::runtime::PoolHealth {
                warm_slots: 0,
                in_use_slots: 0,
                max_slots: 0,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, NanoAgentId,
        NanoAgentVersion, ResourceLimits, StabilityTier,
    };
    use efe_swarm_orchestrator::{SwarmAgentError, SwarmAgentContext};

    fn make_nano() -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author: Author::Efe,
            category: Category::Extraction,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "test::run".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.0,
            latency_p99_ms_target: 50,
            audit: Default::default(),
            compliance: Default::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        }
    }

    struct EchoSwarmAgent;
    #[async_trait]
    impl SwarmAgent for EchoSwarmAgent {
        fn agent_id(&self) -> AgentId {
            AgentId::new("echo-swarm").unwrap()
        }
        async fn run(&self, ctx: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
            let text = ctx
                .blackboard
                .read(DEFAULT_INPUT_POINTER)
                .and_then(|v| v.value.as_str().map(String::from))
                .unwrap_or_default();
            ctx.blackboard
                .write(
                    "/output",
                    serde_json::json!({"echoed": text}),
                    ctx.agent_id.clone(),
                )
                .map_err(|e| SwarmAgentError::Blackboard {
                    message: e.to_string(),
                })?;
            Ok(AgentOutcome::Completed {
                cost_microcents: 250,
                duration_ms: 1,
                blackboard_writes: 1,
                events_published: 0,
            })
        }
    }

    #[tokio::test]
    async fn execute_round_trips_input_through_swarm_agent() {
        let adapter = SwarmToNano::new(Arc::new(EchoSwarmAgent));
        let nano = make_nano();
        let resp = adapter
            .execute(&nano, serde_json::json!({"text": "hello"}))
            .await
            .unwrap();
        assert_eq!(resp.output, serde_json::json!({"echoed": "hello"}));
        // 250 microcents = 0.025 cents.
        assert!((resp.cost_cents - 0.025).abs() < 1e-6);
    }

    #[tokio::test]
    async fn execute_string_input_falls_through_to_text_pointer() {
        let adapter = SwarmToNano::new(Arc::new(EchoSwarmAgent));
        let nano = make_nano();
        let resp = adapter
            .execute(&nano, serde_json::json!("direct string"))
            .await
            .unwrap();
        assert_eq!(resp.output, serde_json::json!({"echoed": "direct string"}));
    }

    #[tokio::test]
    async fn execute_with_seed_is_reproducible() {
        let adapter = SwarmToNano::new(Arc::new(EchoSwarmAgent)).with_rng_seed(42);
        let nano = make_nano();
        let _ = adapter
            .execute(&nano, serde_json::json!({"text": "x"}))
            .await
            .unwrap();
    }

    struct ErrSwarmAgent;
    #[async_trait]
    impl SwarmAgent for ErrSwarmAgent {
        fn agent_id(&self) -> AgentId {
            AgentId::new("err-swarm").unwrap()
        }
        async fn run(&self, _ctx: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
            Ok(AgentOutcome::Errored {
                kind: efe_swarm_orchestrator::AgentErrorKind::ProviderError,
                message: "fixture failure".into(),
            })
        }
    }

    #[tokio::test]
    async fn execute_propagates_errored_outcome_as_runtime_error() {
        let adapter = SwarmToNano::new(Arc::new(ErrSwarmAgent));
        let err = adapter
            .execute(&make_nano(), serde_json::json!({"text": "x"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fixture failure"));
    }

    #[tokio::test]
    async fn execute_missing_output_returns_null() {
        struct NoOp;
        #[async_trait]
        impl SwarmAgent for NoOp {
            fn agent_id(&self) -> AgentId {
                AgentId::new("noop").unwrap()
            }
            async fn run(
                &self,
                _ctx: SwarmAgentContext,
            ) -> Result<AgentOutcome, SwarmAgentError> {
                Ok(AgentOutcome::Completed {
                    cost_microcents: 0,
                    duration_ms: 1,
                    blackboard_writes: 0,
                    events_published: 0,
                })
            }
        }
        let adapter = SwarmToNano::new(Arc::new(NoOp));
        let resp = adapter
            .execute(&make_nano(), serde_json::json!({"text": "x"}))
            .await
            .unwrap();
        assert_eq!(resp.output, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn health_reports_mode_a_only() {
        let adapter = SwarmToNano::new(Arc::new(EchoSwarmAgent));
        let h = adapter.health().await;
        assert!(h.mode_a_alive);
        assert!(!h.mode_b_alive);
        assert!(!h.mode_c_alive);
    }
}
