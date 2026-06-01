//! Mode A — pure Rust thread-pool runtime.
//!
//! Executes Rust functions registered in a closure registry.
//! Each invocation runs in `tokio::task::spawn_blocking` to avoid
//! blocking the async reactor.

use async_trait::async_trait;
use chrono::Utc;
use efe_nano_sdk::{ExecutionMode, NanoAgent};
use jsonschema::{Draft, JSONSchema};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

use crate::audit::{hash_json, NanoAgentTrace};
use crate::error::{NanoAgentError, ResourceKind, SchemaSide};
use crate::metering::{ExecutionModeKind, MeteringClient, NoopMeteringClient};
use crate::runtime::context::CallContext;
use crate::runtime::{NanoAgentResponse, PoolHealth, Runtime, RuntimeHealth, RuntimeResult, TraceId};

// ─────────────────────────────────────────────────────────────────────
//  PureRustHandler — the function trait for Mode A agents
// ─────────────────────────────────────────────────────────────────────

/// Type alias for a Mode A handler function.
///
/// Takes a JSON value, returns a Result with JSON value or string error.
/// Runs in a blocking thread pool, so handlers can use synchronous I/O.
pub type PureRustHandler =
    Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync + 'static>;

/// Registry of Mode A handler functions.
///
/// Each handler is keyed by the `entry` string from
/// [`ExecutionMode::PureRust { entry }`].
#[derive(Clone, Default)]
pub struct HandlerRegistry {
    handlers: Arc<RwLock<HashMap<String, PureRustHandler>>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a handler under the given entry name.
    pub async fn register<F>(&self, entry: impl Into<String>, handler: F)
    where
        F: Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync + 'static,
    {
        let entry = entry.into();
        let handler: PureRustHandler = Arc::new(handler);
        self.handlers.write().await.insert(entry, handler);
    }

    /// Look up a handler by entry name.
    pub async fn lookup(&self, entry: &str) -> Option<PureRustHandler> {
        self.handlers.read().await.get(entry).cloned()
    }

    /// Returns the number of registered handlers.
    pub async fn len(&self) -> usize {
        self.handlers.read().await.len()
    }

    /// Returns true if no handlers are registered.
    pub async fn is_empty(&self) -> bool {
        self.handlers.read().await.is_empty()
    }
}

impl std::fmt::Debug for HandlerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandlerRegistry").finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  PureRustRuntime
// ─────────────────────────────────────────────────────────────────────

/// Mode A executor — pure Rust functions in tokio thread pool.
///
/// Sprint 1 implementation ; Sprint 5 adds `metering` for billing.
pub struct PureRustRuntime {
    registry: HandlerRegistry,
    metering: Arc<dyn MeteringClient>,
}

impl PureRustRuntime {
    pub fn new(registry: HandlerRegistry) -> Self {
        Self::with_metering(registry, Arc::new(NoopMeteringClient::new()))
    }

    /// Sprint 5 : explicit metering wiring.
    pub fn with_metering(
        registry: HandlerRegistry,
        metering: Arc<dyn MeteringClient>,
    ) -> Self {
        Self { registry, metering }
    }

    /// Direct execute entry point (for tests + dispatcher).
    pub async fn execute_with_context(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse, NanoAgentError> {
        let started_at = Utc::now();
        let started_instant = Instant::now();

        // 1. Validate input against schema.
        let input_compiled = compile_schema(&agent.input_schema)
            .map_err(|e| NanoAgentError::Internal(format!("input schema compile : {e}")))?;
        validate(&input_compiled, &input, SchemaSide::Input)?;

        // 2. Look up handler entry.
        let entry = match &agent.execution_mode {
            ExecutionMode::PureRust { entry } => entry.clone(),
            other => {
                return Err(NanoAgentError::Internal(format!(
                    "PureRustRuntime cannot execute mode '{}'",
                    other.mode_str()
                )))
            }
        };
        let handler = self
            .registry
            .lookup(&entry)
            .await
            .ok_or_else(|| NanoAgentError::Internal(format!("handler '{entry}' not registered")))?;

        // 3. Check deadline before running.
        if ctx.is_past_deadline() {
            return Err(NanoAgentError::Timeout(agent.limits.wall_clock_ms));
        }

        // 4. Run in blocking thread pool with timeout.
        let wall_clock_budget = std::time::Duration::from_millis(agent.limits.wall_clock_ms as u64);
        let input_clone = input.clone();
        let handler_clone = handler.clone();
        let join = tokio::task::spawn_blocking(move || handler_clone(input_clone));
        let result = tokio::time::timeout(wall_clock_budget, join)
            .await
            .map_err(|_| NanoAgentError::ResourceExceeded {
                kind: ResourceKind::WallClock,
            })?;

        // Handle spawn_blocking failure.
        let handler_output = result.map_err(|e| {
            NanoAgentError::Runtime {
                message: format!("blocking task join failed : {e}"),
            }
        })?;

        // Handle handler error.
        let output = handler_output.map_err(|msg| NanoAgentError::Runtime { message: msg })?;

        // 5. Validate output against schema.
        let output_compiled = compile_schema(&agent.output_schema)
            .map_err(|e| NanoAgentError::Internal(format!("output schema compile : {e}")))?;
        validate(&output_compiled, &output, SchemaSide::Output)?;

        // 6. Build response + record audit trace.
        let elapsed = started_instant.elapsed();
        let latency_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

        let input_hash_resolved = hash_json(&input);
        let trace = NanoAgentTrace {
            trace_id: ctx.trace_id,
            // Sprint 7 — N1 : propagate full parent vec.
            parent_trace_ids: ctx.parent_trace_ids.clone(),
            swarm_request_id: ctx.swarm_request_id,
            operation_id: ctx.op_id.clone(),
            tenant_id: ctx.tenant_id,
            // Sprint 7 — N2 : propagate caller_agent_id from ctx.
            caller_agent_id: ctx.caller_agent_id.clone(),
            nano_agent_id: agent.id.clone(),
            nano_agent_version: agent.version.clone(),
            execution_mode: agent.execution_mode.mode_str().to_string(),
            // Sprint 7 — N6 : prefer the dispatcher's source-hash if
            // provided (Ref / Merge ops) ; otherwise fall back to the
            // resolved hash (Literal-only stand-alone calls).
            input_hash_source: ctx
                .input_hash_source_override
                .unwrap_or(input_hash_resolved),
            input_hash_resolved,
            output_hash: Some(hash_json(&output)),
            error_kind: None,
            started_at,
            completed_at: Some(Utc::now()),
            latency_ms: Some(latency_ms),
            cost_cents: agent.cost_estimate_cents,
            llm_tokens_used: None,
            // Sprint 6 : full bodies for replay (gated by agent.audit policy).
            full_input: if agent.audit.include_input {
                Some(input.clone())
            } else {
                None
            },
            full_output: if agent.audit.include_output {
                Some(output.clone())
            } else {
                None
            },
            // Sprint 7 — N10 : new traces are always live (tombstone is
            // applied post-hoc by the GDPR / retention path).
            tombstone: None,
            swarm_envelope_trace_id: ctx.swarm_envelope_trace_id,
        };
        ctx.audit.record(trace).await;

        // Sprint 5 : emit billing meter NanoAgentCalls for Mode A.
        // Mode A does not consume LLM tokens or VM time (no pool).
        self.metering
            .record_call(ctx.tenant_id, ExecutionModeKind::ModeA)
            .await;

        Ok(NanoAgentResponse {
            output,
            latency_ms,
            cost_cents: agent.cost_estimate_cents,
            trace_id: ctx.trace_id,
            cache_hit: false,
        })
    }
}

#[async_trait]
impl crate::runtime::NanoAgentExecutor for PureRustRuntime {
    async fn execute_with_context(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse, NanoAgentError> {
        // Delegate to the concrete inherent method. We keep the concrete
        // method available too because the existing dispatcher tests use it
        // directly and because it's slightly more ergonomic for callers
        // who already have a typed `PureRustRuntime` handle.
        Self::execute_with_context(self, agent, input, ctx).await
    }
}

#[async_trait]
impl Runtime for PureRustRuntime {
    async fn execute(
        &self,
        _agent: &NanoAgent,
        _input: serde_json::Value,
    ) -> RuntimeResult<NanoAgentResponse> {
        // The Runtime trait does not give us a CallContext (that's by design — the
        // SDK trait is meant to be called from the dispatcher which constructs the
        // context).  For direct stand-alone calls, use `execute_with_context`.
        Err("PureRustRuntime requires a CallContext; use execute_with_context".into())
    }

    async fn health(&self) -> RuntimeHealth {
        RuntimeHealth {
            mode_a_alive: true,
            mode_b_alive: false,
            mode_c_alive: false,
            pool: PoolHealth {
                warm_slots: 0,
                in_use_slots: 0,
                max_slots: 0,
            },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Schema compilation + validation helpers
// ─────────────────────────────────────────────────────────────────────

fn compile_schema(schema: &serde_json::Value) -> Result<JSONSchema, String> {
    // jsonschema 0.17 + feature "draft202012" enables 2020-12 support.
    JSONSchema::options()
        .with_draft(Draft::Draft202012)
        .compile(schema)
        .map_err(|e| e.to_string())
}

fn validate(
    schema: &JSONSchema,
    value: &serde_json::Value,
    side: SchemaSide,
) -> Result<(), NanoAgentError> {
    if let Err(errors) = schema.validate(value) {
        let msgs: Vec<String> = errors.map(|e| e.to_string()).collect();
        Err(NanoAgentError::SchemaValidation {
            what: side,
            message: msgs.join("; "),
        })
    } else {
        Ok(())
    }
}

// Reference to TraceId import for completeness — used in struct fields.
#[allow(dead_code)]
fn _trace_id_marker(_: TraceId) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::audit::AuditWriter;
    use efe_nano_sdk::{
        Author, AuditPolicy, Category, ComplianceTags, DeterminismLevel, IdempotencyPolicy,
        NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier, TenantId,
    };

    fn make_agent(entry: &str, input_schema: serde_json::Value, output_schema: serde_json::Value) -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("test_agent").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "test".to_string(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: entry.to_string(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema,
            output_schema,
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.001,
            latency_p99_ms_target: 50,
            audit: AuditPolicy::default(),
            compliance: ComplianceTags::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: Utc::now(),
        }
    }

    async fn make_ctx(audit: Arc<dyn AuditWriter>) -> CallContext {
        CallContext::stand_alone(TenantId::system(), audit, 5_000)
    }

    #[tokio::test]
    async fn registry_register_and_lookup() {
        let reg = HandlerRegistry::new();
        assert_eq!(reg.len().await, 0);
        reg.register("echo", Ok).await;
        assert_eq!(reg.len().await, 1);
        let h = reg.lookup("echo").await.unwrap();
        let out = h(serde_json::json!({"hello": "world"})).unwrap();
        assert_eq!(out["hello"], "world");
    }

    #[tokio::test]
    async fn execute_simple_echo_agent() {
        let reg = HandlerRegistry::new();
        reg.register("echo", Ok).await;
        let runtime = PureRustRuntime::new(reg);

        let agent = make_agent(
            "echo",
            serde_json::json!({"type": "object"}),
            serde_json::json!({"type": "object"}),
        );
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit.clone()).await;

        let resp = runtime
            .execute_with_context(&agent, serde_json::json!({"x": 1}), &ctx)
            .await
            .unwrap();
        assert_eq!(resp.output, serde_json::json!({"x": 1}));
        assert!(resp.latency_ms < 100);
        assert_eq!(resp.cost_cents, 0.001);
    }

    #[tokio::test]
    async fn execute_records_audit_trace() {
        let reg = HandlerRegistry::new();
        reg.register("echo", Ok).await;
        let runtime = PureRustRuntime::new(reg);

        let agent = make_agent(
            "echo",
            serde_json::json!({"type": "object"}),
            serde_json::json!({"type": "object"}),
        );
        let mem = Arc::new(InMemoryAuditWriter::new());
        let audit: Arc<dyn AuditWriter> = mem.clone();
        let ctx = make_ctx(audit).await;

        let _ = runtime
            .execute_with_context(&agent, serde_json::json!({}), &ctx)
            .await
            .unwrap();
        assert_eq!(mem.count().await, 1);
        let traces = mem.traces().await;
        assert_eq!(traces[0].nano_agent_id.as_str(), "test_agent");
        assert!(traces[0].error_kind.is_none());
    }

    #[tokio::test]
    async fn input_schema_validation_rejects_invalid_input() {
        let reg = HandlerRegistry::new();
        reg.register("echo", Ok).await;
        let runtime = PureRustRuntime::new(reg);

        // Schema requires "x" field
        let agent = make_agent(
            "echo",
            serde_json::json!({
                "type": "object",
                "required": ["x"],
                "properties": {"x": {"type": "integer"}}
            }),
            serde_json::json!({"type": "object"}),
        );
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        // Input missing "x" → SchemaValidation error
        let err = runtime
            .execute_with_context(&agent, serde_json::json!({}), &ctx)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::SchemaValidation {
                what: SchemaSide::Input,
                ..
            } => {}
            other => panic!("expected SchemaValidation/Input, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn output_schema_validation_rejects_invalid_output() {
        let reg = HandlerRegistry::new();
        // Handler returns wrong shape
        reg.register("bad_output", |_| Ok(serde_json::json!({"bogus": true}))).await;
        let runtime = PureRustRuntime::new(reg);

        let agent = make_agent(
            "bad_output",
            serde_json::json!({"type": "object"}),
            serde_json::json!({
                "type": "object",
                "required": ["valid"],
                "properties": {"valid": {"type": "boolean"}}
            }),
        );
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let err = runtime
            .execute_with_context(&agent, serde_json::json!({}), &ctx)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::SchemaValidation {
                what: SchemaSide::Output,
                ..
            } => {}
            other => panic!("expected SchemaValidation/Output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn handler_error_surfaces_as_runtime_error() {
        let reg = HandlerRegistry::new();
        reg.register("err", |_| Err("oops".to_string())).await;
        let runtime = PureRustRuntime::new(reg);

        let agent = make_agent(
            "err",
            serde_json::json!({"type": "object"}),
            serde_json::json!({"type": "object"}),
        );
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let err = runtime
            .execute_with_context(&agent, serde_json::json!({}), &ctx)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::Runtime { message } => assert!(message.contains("oops")),
            other => panic!("expected Runtime error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn unknown_entry_is_internal_error() {
        let reg = HandlerRegistry::new();
        let runtime = PureRustRuntime::new(reg);
        let agent = make_agent(
            "missing",
            serde_json::json!({"type": "object"}),
            serde_json::json!({"type": "object"}),
        );
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let err = runtime
            .execute_with_context(&agent, serde_json::json!({}), &ctx)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::Internal(msg) => assert!(msg.contains("missing")),
            other => panic!("expected Internal error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn health_reports_mode_a_only() {
        let reg = HandlerRegistry::new();
        let runtime = PureRustRuntime::new(reg);
        let h = runtime.health().await;
        assert!(h.mode_a_alive);
        assert!(!h.mode_b_alive);
        assert!(!h.mode_c_alive);
    }
}
