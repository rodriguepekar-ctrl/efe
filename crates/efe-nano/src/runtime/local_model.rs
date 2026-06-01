//! Mode B — Local model runtime executor.
//!
//! Given an agent with `ExecutionMode::LocalModel { model_id, model_version }`,
//! [`LocalModelRuntime::execute_with_context`] :
//!
//! 1. Validates input against `agent.input_schema`.
//! 2. Checks the call context deadline.
//! 3. Acquires a Mode B VM slot from the pool (within deadline).
//! 4. Sends the inference request through the pool (which loads the model
//!    on first use, then invokes `LocalModel::infer`).
//! 5. Validates the output against `agent.output_schema`.
//! 6. Records an audit trace.
//! 7. Returns a `NanoAgentResponse`.
//!
//! The runtime does **not** itself manage VMs or models — it delegates
//! everything to the injected [`VmPool`].  This keeps the executor
//! ~ 200 lines, easy to reason about, and independent of the eventual
//! Firecracker swap-in in Sprint 8+.

use chrono::Utc;
use efe_nano_sdk::{ExecutionMode, NanoAgent};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::audit::{hash_json, NanoAgentTrace};
use crate::error::{NanoAgentError, ResourceKind, SchemaSide};
use crate::metering::{ExecutionModeKind, MeteringClient, NoopMeteringClient};
use crate::runtime::context::CallContext;
use crate::runtime::pool::{VmKind, VmPool, VmRequest};
use crate::runtime::NanoAgentResponse;

use jsonschema::{Draft, JSONSchema};

/// Mode B executor.
///
/// Holds an `Arc<dyn VmPool>` (production : `FirecrackerVmPool` ; Phase 2
/// ship : `StubVmPool`) + an `Arc<dyn MeteringClient>` for billing.
pub struct LocalModelRuntime {
    pool: Arc<dyn VmPool>,
    metering: Arc<dyn MeteringClient>,
}

impl LocalModelRuntime {
    pub fn new(pool: Arc<dyn VmPool>) -> Self {
        Self::with_metering(pool, Arc::new(NoopMeteringClient::new()))
    }

    /// Sprint 5 : explicit metering wiring.  Use
    /// [`crate::metering::InMemoryMeteringClient`] in tests + the
    /// production shim around `efe_metering::MeterClient` in production.
    pub fn with_metering(
        pool: Arc<dyn VmPool>,
        metering: Arc<dyn MeteringClient>,
    ) -> Self {
        Self { pool, metering }
    }

    /// Execute a Mode B agent.
    ///
    /// # Errors
    ///
    /// - [`NanoAgentError::Internal`] if the agent is not Mode B.
    /// - [`NanoAgentError::SchemaValidation`] if input or output fails schema.
    /// - [`NanoAgentError::ResourceExceeded`] on wall-clock budget exhausted.
    /// - [`NanoAgentError::VmPool`] on acquire / execute pool errors.
    /// - [`NanoAgentError::LocalModel`] on inference errors.
    pub async fn execute_with_context(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse, NanoAgentError> {
        let started_at = Utc::now();
        let started_instant = Instant::now();

        // 1. Verify it's a Mode B agent + extract model coordinates.
        let (model_id, model_version) = match &agent.execution_mode {
            ExecutionMode::LocalModel { model_id, model_version } => {
                (model_id.clone(), model_version.clone())
            }
            other => {
                return Err(NanoAgentError::Internal(format!(
                    "LocalModelRuntime cannot execute mode '{}'",
                    other.mode_str()
                )))
            }
        };

        // 2. Validate input against schema (cached compile in Phase 4).
        let input_compiled = compile_schema(&agent.input_schema)
            .map_err(|e| NanoAgentError::Internal(format!("input schema compile : {e}")))?;
        validate_value(&input_compiled, &input, SchemaSide::Input)?;

        // 3. Check call-context deadline.
        let remaining = match ctx.remaining_time() {
            Some(d) => d,
            None => {
                return Err(NanoAgentError::Timeout(agent.limits.wall_clock_ms));
            }
        };

        // 4. Acquire a Mode B slot from the pool, capping wait to remaining time.
        let wait_budget = remaining.min(Duration::from_millis(
            agent.limits.wall_clock_ms.max(1) as u64,
        ));
        let mut lease = self
            .pool
            .acquire(VmKind::ModelInference, wait_budget)
            .await?;

        // 5. Build VmRequest + execute.
        let req = VmRequest::mode_b(
            agent.id.as_str(),
            agent.version.as_str(),
            input.clone(),
            agent.limits.wall_clock_ms,
            model_id,
            model_version,
        );
        let vm_resp = lease.execute(req).await?;
        // Lease released on drop at end of scope.

        // 6. Validate output against schema.
        let output_compiled = compile_schema(&agent.output_schema)
            .map_err(|e| NanoAgentError::Internal(format!("output schema compile : {e}")))?;
        validate_value(&output_compiled, &vm_resp.output, SchemaSide::Output)?;

        // 7. Build response + record audit trace.
        let elapsed = started_instant.elapsed();
        let latency_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;

        let input_hash_resolved = hash_json(&input);
        let trace = NanoAgentTrace {
            trace_id: ctx.trace_id,
            // Sprint 7 — N1 / N2 : DAG parents + caller from ctx.
            parent_trace_ids: ctx.parent_trace_ids.clone(),
            swarm_request_id: ctx.swarm_request_id,
            operation_id: ctx.op_id.clone(),
            tenant_id: ctx.tenant_id,
            caller_agent_id: ctx.caller_agent_id.clone(),
            nano_agent_id: agent.id.clone(),
            nano_agent_version: agent.version.clone(),
            execution_mode: agent.execution_mode.mode_str().to_string(),
            // Sprint 7 — N6 : prefer dispatcher-provided source hash.
            input_hash_source: ctx
                .input_hash_source_override
                .unwrap_or(input_hash_resolved),
            input_hash_resolved,
            output_hash: Some(hash_json(&vm_resp.output)),
            error_kind: None,
            started_at,
            completed_at: Some(Utc::now()),
            latency_ms: Some(latency_ms),
            cost_cents: agent.cost_estimate_cents,
            llm_tokens_used: None,
            full_input: if agent.audit.include_input {
                Some(input.clone())
            } else {
                None
            },
            full_output: if agent.audit.include_output {
                Some(vm_resp.output.clone())
            } else {
                None
            },
            // Sprint 7 — N10 : new trace is live.
            tombstone: None,
            swarm_envelope_trace_id: ctx.swarm_envelope_trace_id,
        };
        ctx.audit.record(trace).await;

        // 8. Memory cap check (best-effort — stub reports 0 ; real Firecracker
        //    swap-in will populate `vm_resp.memory_mb_peak`).
        if vm_resp.memory_mb_peak > agent.limits.memory_mb {
            return Err(NanoAgentError::ResourceExceeded {
                kind: ResourceKind::Memory,
            });
        }

        // 9. Emit billing meters (Sprint 5) :
        //    NanoAgentCalls (Mode B) + NanoAgentVmSeconds (no LLM tokens).
        self.metering
            .record_call(ctx.tenant_id, ExecutionModeKind::ModeB)
            .await;
        self.metering
            .record_vm_seconds(ctx.tenant_id, elapsed.as_secs_f64())
            .await;

        Ok(NanoAgentResponse {
            output: vm_resp.output,
            latency_ms,
            cost_cents: agent.cost_estimate_cents,
            trace_id: ctx.trace_id,
            cache_hit: false,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Internals : schema compile + validate (same as Mode A path)
// ─────────────────────────────────────────────────────────────────────

fn compile_schema(schema: &serde_json::Value) -> Result<JSONSchema, String> {
    JSONSchema::options()
        .with_draft(Draft::Draft202012)
        .compile(schema)
        .map_err(|e| e.to_string())
}

fn validate_value(
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

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::audit::AuditWriter;
    use crate::runtime::model_loader::InProcessModelLoader;
    use crate::runtime::pool::stub::{StubVmPool, StubVmPoolConfig};
    use efe_nano_sdk::{
        Author, AuditPolicy, Category, ComplianceTags, DeterminismLevel, IdempotencyPolicy,
        NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier, TenantId,
    };
    use serde_json::json;

    fn make_agent_mode_b(model_id: &str, version: &str) -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("ner_test").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "test mode B agent".to_string(),
            author: Author::Efe,
            category: Category::EntityRecognition,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::LocalModel {
                model_id: model_id.to_string(),
                model_version: version.to_string(),
            },
            determinism: DeterminismLevel::Seeded,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: json!({
                "type": "object",
                "required": ["text"],
                "properties": { "text": { "type": "string" } }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["entities", "count"],
                "properties": {
                    "entities": { "type": "array" },
                    "count": { "type": "integer" }
                }
            }),
            error_schema: None,
            limits: ResourceLimits {
                memory_mb: 2048,
                cpu_ms: 5_000,
                wall_clock_ms: 5_000,
                llm_tokens: None,
                network_egress_kb: 0,
            },
            cost_estimate_cents: 0.1,
            latency_p99_ms_target: 200,
            audit: AuditPolicy::default(),
            compliance: ComplianceTags::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: Utc::now(),
        }
    }

    fn make_pool() -> Arc<StubVmPool> {
        Arc::new(StubVmPool::with_phase_2_defaults())
    }

    async fn make_ctx(audit: Arc<dyn AuditWriter>) -> CallContext {
        CallContext::stand_alone(TenantId::system(), audit, 5_000)
    }

    #[tokio::test]
    async fn executes_mode_b_clinical_ner() {
        let pool = make_pool();
        let runtime = LocalModelRuntime::new(pool);
        let agent = make_agent_mode_b("ner_clinical_v1", "1.0.0");
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let resp = runtime
            .execute_with_context(
                &agent,
                json!({"text": "metformin for diabetes"}),
                &ctx,
            )
            .await
            .unwrap();

        let entities = resp.output["entities"].as_array().unwrap();
        assert!(!entities.is_empty());
        assert!(resp.latency_ms < 1_000);
    }

    #[tokio::test]
    async fn records_audit_trace() {
        let pool = make_pool();
        let runtime = LocalModelRuntime::new(pool);
        let agent = make_agent_mode_b("ner_clinical_v1", "1.0.0");
        let mem = Arc::new(InMemoryAuditWriter::new());
        let audit: Arc<dyn AuditWriter> = mem.clone();
        let ctx = make_ctx(audit).await;

        let _ = runtime
            .execute_with_context(&agent, json!({"text": "phase 3 trial"}), &ctx)
            .await
            .unwrap();
        assert_eq!(mem.count().await, 1);
        let traces = mem.traces().await;
        assert_eq!(traces[0].execution_mode, "local_model");
    }

    #[tokio::test]
    async fn rejects_non_mode_b_agent() {
        let pool = make_pool();
        let runtime = LocalModelRuntime::new(pool);
        // Make a Mode A agent and try to feed it to the Mode B runtime.
        let mut agent = make_agent_mode_b("ner_clinical_v1", "1.0.0");
        agent.execution_mode = ExecutionMode::PureRust {
            entry: "json_validate::run".to_string(),
        };
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;
        let err = runtime
            .execute_with_context(&agent, json!({"text": "x"}), &ctx)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::Internal(msg) => {
                assert!(msg.contains("pure_rust"));
                assert!(msg.contains("LocalModelRuntime"));
            }
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn input_schema_violation_is_caught() {
        let pool = make_pool();
        let runtime = LocalModelRuntime::new(pool);
        let agent = make_agent_mode_b("ner_clinical_v1", "1.0.0");
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        // Missing required 'text' field.
        let err = runtime
            .execute_with_context(&agent, json!({}), &ctx)
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
    async fn unknown_model_id_returns_pool_error() {
        let pool = make_pool();
        let runtime = LocalModelRuntime::new(pool);
        let agent = make_agent_mode_b("ghost_model_v1", "1.0.0");
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let err = runtime
            .execute_with_context(&agent, json!({"text": "x"}), &ctx)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::VmPool(crate::error::VmPoolError::ProvisionFailed(msg)) => {
                assert!(msg.contains("model loader"));
            }
            other => panic!("expected VmPool/ProvisionFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn past_deadline_returns_timeout_immediately() {
        let pool = make_pool();
        let runtime = LocalModelRuntime::new(pool);
        let agent = make_agent_mode_b("ner_clinical_v1", "1.0.0");
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        // Past deadline.
        let ctx = CallContext::stand_alone(TenantId::system(), audit, 0);
        tokio::time::sleep(Duration::from_millis(2)).await;

        let err = runtime
            .execute_with_context(&agent, json!({"text": "x"}), &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, NanoAgentError::Timeout(_)));
    }

    #[tokio::test]
    async fn synthetic_latency_within_budget_succeeds() {
        let pool = Arc::new(StubVmPool::new(
            StubVmPoolConfig {
                mode_b_max: 4,
                mode_b_warm: 0,
                mode_c_max: 4,
                mode_c_warm: 0,
                synthetic_latency: Duration::from_millis(20),
            },
            Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        ));
        let runtime = LocalModelRuntime::new(pool);
        let agent = make_agent_mode_b("ner_clinical_v1", "1.0.0");
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let resp = runtime
            .execute_with_context(&agent, json!({"text": "aspirin"}), &ctx)
            .await
            .unwrap();
        // Latency includes the synthetic 20 ms.
        assert!(resp.latency_ms >= 20);
    }

    #[tokio::test]
    async fn parallel_calls_share_pool() {
        let pool = make_pool();
        let runtime = Arc::new(LocalModelRuntime::new(pool));
        let agent = Arc::new(make_agent_mode_b("ner_clinical_v1", "1.0.0"));
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());

        let mut handles = Vec::new();
        for _ in 0..16 {
            let runtime = Arc::clone(&runtime);
            let agent = Arc::clone(&agent);
            let audit = Arc::clone(&audit);
            handles.push(tokio::spawn(async move {
                let ctx = CallContext::stand_alone(TenantId::system(), audit, 5_000);
                runtime
                    .execute_with_context(
                        &agent,
                        json!({"text": "aspirin for diabetes"}),
                        &ctx,
                    )
                    .await
            }));
        }

        for h in handles {
            let r = h.await.unwrap();
            assert!(r.is_ok());
        }
    }
}
