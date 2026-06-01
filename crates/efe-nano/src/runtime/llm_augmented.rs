//! Mode C — LLM-augmented runtime executor.
//!
//! # Pipeline (cf Sprint 5 architecture § 8.1)
//!
//! Given a `NanoAgent` with `ExecutionMode::LlmAugmented { .. }`,
//! `LlmAugmentedRuntime::execute_with_context` performs :
//!
//! 1. Validate input against `agent.input_schema`.
//! 2. Check `ctx.deadline` — early `Timeout` if past.
//! 3. PII redact (if `agent.compliance.pii_redaction_strategy.redacts_input()`).
//! 4. Estimate cost (max of `agent.cost_estimate_cents` and `llm_client.estimate`).
//! 5. Budget pre-check → `ReservationId`.
//! 6. Acquire `VmKind::LlmCaller` slot from the pool.
//! 7. Build `LlmCallRequest` from agent metadata + redacted input.
//! 8. Execute via pool lease (which internally calls `llm_client.call`).
//! 9. Validate output against `agent.output_schema`.
//!    - On `SchemaMismatch` AND `retry_policy.max_attempts > 1` :
//!      retry once with a stronger prompt (Sprint 5 Day 6).
//! 10. PII strict-mode anti-leakage check (if `PiiStrategy::Strict`).
//! 11. Budget commit with actual cost.
//! 12. Build `NanoAgentResponse` with `llm_tokens_used` populated.
//! 13. Record audit trace.
//!
//! # Error paths
//!
//! - Budget insufficient → `BudgetExceeded` → reservation NOT taken.
//! - Pool acquire timeout → `Timeout` → reservation released.
//! - LLM call error → `LlmCall(Provider | ...)` → reservation released.
//! - Output schema mismatch (after retry) → `SchemaValidationFailed`.
//! - PII leak in strict mode → `Pii(LeakDetected)`.

use chrono::Utc;
use efe_nano_sdk::{ExecutionMode, NanoAgent, PiiStrategy};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::audit::{hash_json, NanoAgentTrace};
use crate::budget::{BudgetEnforcer, ReservationId};
use crate::error::{LlmCallError, NanoAgentError, ResourceKind, SchemaSide};
use crate::metering::{ExecutionModeKind, MeteringClient, NoopMeteringClient};
use crate::runtime::context::CallContext;
use crate::runtime::llm_client::{LlmCallRequest, LlmClient, ResponseFormat};
use crate::runtime::output_validator::OutputValidator;
use crate::runtime::pii_redactor::{contains_redaction_tokens, PiiRedactor};
use crate::runtime::pool::{VmKind, VmPool, VmRequest};
use crate::runtime::NanoAgentResponse;

use jsonschema::{Draft, JSONSchema};

// ─────────────────────────────────────────────────────────────────────
//  LlmAugmentedRuntime
// ─────────────────────────────────────────────────────────────────────

/// Mode C executor.  Composed of :
/// - `pool` — for acquire/release semaphore-based parallelism gating
/// - `llm_client` — for actual LLM call (NB : carried by pool too, but
///   the runtime holds an independent ref for direct estimate calls)
/// - `output_validator` — strict JSON schema enforcement + retry policy
/// - `budget` — per-tenant pre_check/commit/release lifecycle
/// - `pii_redactor` — pre-LLM redaction (when agent compliance asks for it)
pub struct LlmAugmentedRuntime {
    pool: Arc<dyn VmPool>,
    llm_client: Arc<dyn LlmClient>,
    output_validator: Arc<OutputValidator>,
    budget: Arc<dyn BudgetEnforcer>,
    pii_redactor: Arc<PiiRedactor>,
    metering: Arc<dyn MeteringClient>,
    /// **Sprint 7 — N8** : prompt-injection filter run after PII
    /// redact, before the LLM call.  Default = [`RegexPromptInjectionFilter`].
    /// Per-agent strategy comes from
    /// `agent.compliance.prompt_injection_strategy` :
    /// - `Off`  → filter not invoked.
    /// - `Warn` → invoked, suspect input logged + metered, call proceeds.
    /// - `Block`→ invoked, suspect input ⇒ `PromptInjectionSuspected`.
    prompt_injection_filter:
        Arc<dyn crate::runtime::prompt_injection::PromptInjectionFilter>,
}

impl LlmAugmentedRuntime {
    pub fn new(
        pool: Arc<dyn VmPool>,
        llm_client: Arc<dyn LlmClient>,
        output_validator: Arc<OutputValidator>,
        budget: Arc<dyn BudgetEnforcer>,
        pii_redactor: Arc<PiiRedactor>,
    ) -> Self {
        Self::with_metering(
            pool,
            llm_client,
            output_validator,
            budget,
            pii_redactor,
            Arc::new(NoopMeteringClient::new()),
        )
    }

    /// Sprint 5 : explicit metering wiring.  Use [`crate::metering::InMemoryMeteringClient`]
    /// for tests + the production-deployment shim around `efe_metering::MeterClient` for
    /// real billing.
    ///
    /// **Sprint 7 — N8** : wires the default
    /// [`crate::runtime::prompt_injection::RegexPromptInjectionFilter`] ;
    /// use [`with_filter`] to override.
    pub fn with_metering(
        pool: Arc<dyn VmPool>,
        llm_client: Arc<dyn LlmClient>,
        output_validator: Arc<OutputValidator>,
        budget: Arc<dyn BudgetEnforcer>,
        pii_redactor: Arc<PiiRedactor>,
        metering: Arc<dyn MeteringClient>,
    ) -> Self {
        Self {
            pool,
            llm_client,
            output_validator,
            budget,
            pii_redactor,
            metering,
            prompt_injection_filter: Arc::new(
                crate::runtime::prompt_injection::RegexPromptInjectionFilter::new(),
            ),
        }
    }

    /// **Sprint 7 — N8** : install a custom prompt-injection filter.
    /// Phase 3 will replace the default regex filter with an ML
    /// classifier ; this seam lets the swap happen without touching
    /// the runtime.
    pub fn with_filter(
        mut self,
        filter: Arc<dyn crate::runtime::prompt_injection::PromptInjectionFilter>,
    ) -> Self {
        self.prompt_injection_filter = filter;
        self
    }

    /// Sprint 5 convenience : wire builtin defaults (BuiltinLlmClient +
    /// in-memory budget + regex PII + Noop metering).  Used by tests + demos.
    pub fn with_phase_2_defaults(pool: Arc<dyn VmPool>) -> Self {
        Self::new(
            pool,
            Arc::new(crate::runtime::llm_client::BuiltinLlmClient::new()),
            Arc::new(OutputValidator::new()),
            Arc::new(crate::budget::InMemoryBudgetEnforcer::new()),
            Arc::new(PiiRedactor::with_default_regex_backend()),
        )
    }

    /// Pre-warm the `OutputValidator` cache with the agents' output
    /// schemas.  Called at dispatcher construction to avoid first-call
    /// schema-compile latency.
    pub fn warm_output_validator(
        &self,
        schemas: &[serde_json::Value],
    ) -> Result<usize, NanoAgentError> {
        self.output_validator
            .warm_with(schemas)
            .map_err(NanoAgentError::OutputValidation)
    }

    /// Execute a Mode C agent end-to-end.
    pub async fn execute_with_context(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse, NanoAgentError> {
        let started_at = Utc::now();
        let started_instant = Instant::now();

        // 1. Verify Mode C agent.
        let llm_spec = match &agent.execution_mode {
            ExecutionMode::LlmAugmented {
                provider,
                model,
                prompt_template_id,
                max_tokens,
                temperature,
            } => LlmAugmentedSpec {
                provider: provider.clone(),
                model: model.clone(),
                prompt_template_id: prompt_template_id.clone(),
                max_tokens: *max_tokens,
                temperature: *temperature,
            },
            other => {
                return Err(NanoAgentError::Internal(format!(
                    "LlmAugmentedRuntime cannot execute mode '{}'",
                    other.mode_str()
                )))
            }
        };

        // 2. Validate input against schema.
        let input_compiled = compile_schema(&agent.input_schema)
            .map_err(|e| NanoAgentError::Internal(format!("input schema compile : {e}")))?;
        validate_value(&input_compiled, &input, SchemaSide::Input)?;

        // 3. Deadline check.
        let remaining = ctx
            .remaining_time()
            .ok_or(NanoAgentError::Timeout(agent.limits.wall_clock_ms))?;

        // 4. PII redaction (if strategy != None).
        let strategy = agent.compliance.pii_redaction_strategy;
        let processed_input = if strategy.redacts_input() {
            self.pii_redactor.redact_json(input.clone()).await?
        } else {
            input.clone()
        };

        // 4.5 — **Sprint 7 — N8** : prompt-injection filter.
        // Runs AFTER PII redaction so PII tokens don't get confused
        // with injection patterns ; runs BEFORE the LLM call so a
        // malicious input never reaches a provider.  The check is
        // gated on the agent's `prompt_injection_strategy` to avoid
        // wasting CPU on Mode A/B agents.
        let pi_strategy = agent.compliance.prompt_injection_strategy;
        if !matches!(
            pi_strategy,
            efe_nano_sdk::PromptInjectionStrategy::Off
        ) {
            let verdict = self.prompt_injection_filter.scan(&processed_input).await;
            if verdict.is_suspect() {
                let pattern_ids: Vec<&str> = verdict
                    .signals
                    .iter()
                    .map(|s| s.pattern_id.as_str())
                    .collect();
                match pi_strategy {
                    efe_nano_sdk::PromptInjectionStrategy::Block => {
                        // Emit a structured warning trace before bailing
                        // (no LLM cost was incurred, so we don't bill).
                        tracing::warn!(
                            target: "efe_nano::prompt_injection",
                            agent_id = %agent.id.as_str(),
                            tenant_id = %ctx.tenant_id,
                            op_id = %ctx.op_id,
                            score = verdict.score,
                            patterns = ?pattern_ids,
                            "prompt injection blocked (strategy=Block)"
                        );
                        return Err(NanoAgentError::PromptInjection {
                            score: verdict.score,
                            patterns: pattern_ids
                                .iter()
                                .map(|s| s.to_string())
                                .collect(),
                        });
                    }
                    efe_nano_sdk::PromptInjectionStrategy::Warn => {
                        tracing::warn!(
                            target: "efe_nano::prompt_injection",
                            agent_id = %agent.id.as_str(),
                            tenant_id = %ctx.tenant_id,
                            op_id = %ctx.op_id,
                            score = verdict.score,
                            patterns = ?pattern_ids,
                            "prompt injection detected (strategy=Warn, proceeding)"
                        );
                        // Continue ; metering layer will see the
                        // `prompt_injection_warn` counter via observability.
                    }
                    efe_nano_sdk::PromptInjectionStrategy::Off => unreachable!(),
                }
            }
        }

        // 5. Build LlmCallRequest from agent metadata + processed input.
        let llm_req = LlmCallRequest::builder(ctx.tenant_id, &llm_spec.prompt_template_id)
            .template_inputs(processed_input.clone())
            .max_tokens(llm_spec.max_tokens)
            .temperature(llm_spec.temperature)
            .providers_allow_list(vec![format!("{}:{}", llm_spec.provider, llm_spec.model)])
            .response_format(ResponseFormat::json_schema(agent.output_schema.clone()))
            .timeout_ms(remaining.as_millis().min(agent.limits.wall_clock_ms as u128) as u32)
            .build();

        // 6. Estimate cost + budget pre-check.
        let estimated_cost = self
            .llm_client
            .estimate_cost_cents(&llm_req)
            .max(agent.cost_estimate_cents);
        let reservation = match self.budget.pre_check(ctx.tenant_id, estimated_cost).await {
            Ok(r) => r,
            Err(e) => return Err(NanoAgentError::Budget(e)),
        };

        // From here on, any error path must release the reservation.
        let result = self
            .execute_after_reservation(
                agent,
                &input,
                &processed_input,
                llm_req,
                &llm_spec,
                ctx,
                reservation,
                strategy,
                started_at,
                started_instant,
            )
            .await;

        // Centralized cleanup : on error, release reservation.
        match result {
            Ok(resp) => Ok(resp),
            Err(e) => {
                let _ = self.budget.release(reservation).await;
                Err(e)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_after_reservation(
        &self,
        agent: &NanoAgent,
        original_input: &serde_json::Value,
        processed_input: &serde_json::Value,
        llm_req: LlmCallRequest,
        llm_spec: &LlmAugmentedSpec,
        ctx: &CallContext,
        reservation: ReservationId,
        strategy: PiiStrategy,
        started_at: chrono::DateTime<Utc>,
        started_instant: Instant,
    ) -> Result<NanoAgentResponse, NanoAgentError> {
        // 7. Acquire pool slot.
        let pool_wait = ctx
            .remaining_time()
            .unwrap_or_else(|| Duration::from_millis(agent.limits.wall_clock_ms as u64));
        let mut lease = self.pool.acquire(VmKind::LlmCaller, pool_wait).await?;

        // 8. Execute first attempt.
        let vm_req = VmRequest::mode_c(
            agent.id.as_str(),
            agent.version.as_str(),
            processed_input.clone(),
            agent.limits.wall_clock_ms,
            llm_req.clone(),
        );
        let mut vm_resp = lease.execute(vm_req).await?;
        let mut retry_attempts: u32 = 0;

        // Accumulate tokens + cost across retries — customers paid the
        // LLM provider for both attempts, so metering + budget must
        // reflect total cost.
        let mut total_tokens: u64 = vm_resp.llm_tokens_used.unwrap_or(0) as u64;
        let mut total_cost_cents: f64 = vm_resp
            .llm_cost_cents
            .unwrap_or(agent.cost_estimate_cents);

        // 9. Output validation + retry path.
        let output_schema = &agent.output_schema;
        match self.output_validator.validate(&vm_resp.output, output_schema) {
            Ok(()) => {}
            Err(first_err) => {
                let policy = self.output_validator.retry_policy();
                if policy.max_attempts > 1 {
                    // Retry once with stronger prompt.
                    let stronger_req =
                        inject_schema_reminder(&llm_req, output_schema);
                    let retry_vm_req = VmRequest::mode_c(
                        agent.id.as_str(),
                        agent.version.as_str(),
                        processed_input.clone(),
                        agent.limits.wall_clock_ms,
                        stronger_req,
                    );
                    let retry_resp = lease.execute(retry_vm_req).await?;
                    retry_attempts = 1;

                    // Accumulate retry's tokens + cost on top of the first
                    // attempt — both calls were billed by the provider.
                    total_tokens += retry_resp.llm_tokens_used.unwrap_or(0) as u64;
                    total_cost_cents += retry_resp
                        .llm_cost_cents
                        .unwrap_or(agent.cost_estimate_cents);

                    self.output_validator
                        .validate(&retry_resp.output, output_schema)
                        .map_err(NanoAgentError::OutputValidation)?;
                    vm_resp = retry_resp;
                } else {
                    return Err(NanoAgentError::OutputValidation(first_err));
                }
            }
        }

        // 10. PII leak check (strict mode).
        if strategy.checks_output_for_leaks() {
            if let Some(token) = contains_redaction_tokens(&vm_resp.output) {
                return Err(NanoAgentError::LlmCall(LlmCallError::PiiLeakDetected(token)));
            }
        }

        // 11. Memory cap check (best-effort).
        if vm_resp.memory_mb_peak > agent.limits.memory_mb {
            return Err(NanoAgentError::ResourceExceeded {
                kind: ResourceKind::Memory,
            });
        }

        // 12. Budget commit with TOTAL actual cost (incl. retry).
        //     `actual_cost` covers all attempts so the customer is
        //     correctly billed for both LLM calls when retry happened.
        let actual_cost = total_cost_cents;
        self.budget.commit(reservation, actual_cost).await?;

        // 13. Build response.
        let elapsed = started_instant.elapsed();
        let latency_ms = elapsed.as_millis().min(u32::MAX as u128) as u32;
        // `llm_tokens_used` carries the cumulative tokens across attempts.
        let llm_tokens_used = if total_tokens > 0 {
            Some(total_tokens.min(u32::MAX as u64) as u32)
        } else {
            None
        };

        // 14. Record audit trace.
        let input_hash_resolved = hash_json(original_input);
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
            // Sprint 6 : hash the ORIGINAL (pre-redaction) input — this
            // is the agent input the customer sent.  The redacted form
            // was used internally for the LLM call but isn't the audit
            // identity.
            //
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
            cost_cents: actual_cost,
            llm_tokens_used,
            // Sprint 6 : full bodies for replay.
            // For Mode C with PII redaction strategy != None, we store
            // the REDACTED input (post-redaction) when include_input is
            // true.  This avoids storing raw PII in the audit chain.
            full_input: if agent.audit.include_input {
                if strategy.redacts_input() {
                    Some(processed_input.clone())
                } else {
                    Some(original_input.clone())
                }
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

        // 15. Emit billing meters (Sprint 5 § 8 step 14) :
        //
        //   - NanoAgentCalls    += 1
        //   - NanoAgentLlmTokens += tokens_in + tokens_out
        //   - NanoAgentVmSeconds += elapsed
        //
        // Metering only fires on the success path : on error we already
        // released the budget reservation and returned early — no meter
        // increment in that case (matches Phase 4 M31 billing semantics).
        self.metering
            .record_call(ctx.tenant_id, ExecutionModeKind::ModeC)
            .await;
        if let Some(tokens) = llm_tokens_used {
            self.metering
                .record_llm_tokens(ctx.tenant_id, tokens as u64)
                .await;
        }
        self.metering
            .record_vm_seconds(ctx.tenant_id, elapsed.as_secs_f64())
            .await;

        // Honour `agent.audit.redact_pii` flag : we already used the
        // processed (redacted) input in `template_inputs`, so the trace
        // body in M50 audit chain (Sprint 6) doesn't see raw PII even
        // when redact_pii is true.

        if retry_attempts > 0 {
            tracing::debug!(
                target: "efe_nano::mode_c",
                agent = agent.id.as_str(),
                provider = llm_spec.provider.as_str(),
                model = llm_spec.model.as_str(),
                retry_attempts = retry_attempts,
                total_tokens = total_tokens,
                total_cost_cents = total_cost_cents,
                "Mode C call succeeded after retry"
            );
        }

        Ok(NanoAgentResponse {
            output: vm_resp.output,
            latency_ms,
            cost_cents: actual_cost,
            trace_id: ctx.trace_id,
            cache_hit: false,
        })
    }
}

impl std::fmt::Debug for LlmAugmentedRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmAugmentedRuntime").finish_non_exhaustive()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────

/// Cached snapshot of `agent.execution_mode.LlmAugmented` fields.
#[derive(Debug, Clone)]
struct LlmAugmentedSpec {
    provider: String,
    model: String,
    prompt_template_id: String,
    max_tokens: u32,
    temperature: f32,
}

/// Inject the agent's `output_schema` body into the retry call's
/// template inputs under a special `_retry_reminder` key, so that prompt
/// templates can render a reinforcement instruction on the second attempt.
fn inject_schema_reminder(
    base: &LlmCallRequest,
    output_schema: &serde_json::Value,
) -> LlmCallRequest {
    let mut new_req = base.clone();
    let mut inputs = base.template_inputs.clone();
    if let Some(obj) = inputs.as_object_mut() {
        obj.insert(
            "_retry_reminder".to_string(),
            serde_json::json!({
                "previous_attempt_failed": true,
                "required_schema": output_schema,
            }),
        );
    }
    new_req.template_inputs = inputs;
    new_req
}

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
    use crate::budget::InMemoryBudgetEnforcer;
    use crate::runtime::llm_client::{BuiltinLlmClient, MockLlmClient, MockResponse};
    use crate::runtime::pool::stub::StubVmPool;
    use efe_nano_sdk::{
        Author, AuditPolicy, Category, ComplianceTags, DeterminismLevel, IdempotencyPolicy,
        NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier, TenantId,
    };
    use serde_json::json;

    fn make_mode_c_agent_summarize() -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("summarize_strict").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "test summarize_strict".into(),
            author: Author::Efe,
            category: Category::Summarization,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::LlmAugmented {
                provider: "builtin".into(),
                model: "deterministic-v1".into(),
                prompt_template_id: "m50.summarize_strict.v1".into(),
                max_tokens: 200,
                temperature: 0.1,
            },
            determinism: DeterminismLevel::LlmReplayable,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: json!({
                "type": "object",
                "required": ["text", "max_sentences"],
                "properties": {
                    "text": {"type": "string"},
                    "max_sentences": {"type": "integer"}
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["summary", "sentence_count"],
                "properties": {
                    "summary": {"type": "string"},
                    "sentence_count": {"type": "integer"}
                }
            }),
            error_schema: None,
            limits: ResourceLimits {
                memory_mb: 512,
                cpu_ms: 10_000,
                wall_clock_ms: 10_000,
                llm_tokens: Some(200),
                network_egress_kb: 50,
            },
            cost_estimate_cents: 1.0,
            latency_p99_ms_target: 3_000,
            audit: AuditPolicy::default(),
            compliance: ComplianceTags {
                gdpr: true,
                pii_redaction_strategy: PiiStrategy::PreLlmRedact,
                ..ComplianceTags::default()
            },
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: Utc::now(),
        }
    }

    fn make_runtime_builtin() -> Arc<LlmAugmentedRuntime> {
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());
        Arc::new(LlmAugmentedRuntime::with_phase_2_defaults(pool))
    }

    async fn make_ctx(audit: Arc<dyn AuditWriter>) -> CallContext {
        CallContext::stand_alone(TenantId::system(), audit, 10_000)
    }

    // ── E2E with BuiltinLlmClient ────────────────────────────────────

    #[tokio::test]
    async fn mode_c_executes_summarize_strict_end_to_end() {
        let runtime = make_runtime_builtin();
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let input = json!({
            "text": "First. Second. Third.",
            "max_sentences": 2
        });
        let resp = runtime.execute_with_context(&agent, input, &ctx).await.unwrap();
        assert!(resp.output["summary"].is_string());
        assert_eq!(resp.output["sentence_count"], 2);
        assert!(resp.cost_cents > 0.0);
    }

    #[tokio::test]
    async fn mode_c_records_audit_trace() {
        let runtime = make_runtime_builtin();
        let agent = make_mode_c_agent_summarize();
        let mem = Arc::new(InMemoryAuditWriter::new());
        let audit: Arc<dyn AuditWriter> = mem.clone();
        let ctx = make_ctx(audit).await;

        let _ = runtime
            .execute_with_context(
                &agent,
                json!({"text": "Single.", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(mem.count().await, 1);
        let traces = mem.traces().await;
        assert_eq!(traces[0].execution_mode, "llm_augmented");
        assert!(traces[0].llm_tokens_used.is_some());
    }

    // ── Mode mismatch ───────────────────────────────────────────────

    #[tokio::test]
    async fn rejects_non_mode_c_agent() {
        let runtime = make_runtime_builtin();
        let mut agent = make_mode_c_agent_summarize();
        agent.execution_mode = ExecutionMode::PureRust {
            entry: "echo".into(),
        };
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;
        let err = runtime
            .execute_with_context(&agent, json!({"text": "x", "max_sentences": 1}), &ctx)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::Internal(msg) => assert!(msg.contains("pure_rust")),
            other => panic!("expected Internal, got {other:?}"),
        }
    }

    // ── Input schema failure ────────────────────────────────────────

    #[tokio::test]
    async fn input_schema_violation_is_caught() {
        let runtime = make_runtime_builtin();
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;
        let err = runtime
            .execute_with_context(&agent, json!({}), &ctx)     // missing text + max_sentences
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

    // ── Budget pre-check failure ────────────────────────────────────

    #[tokio::test]
    async fn budget_insufficient_returns_budget_error() {
        // Construct a runtime with budget cap 0.
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());
        let runtime = LlmAugmentedRuntime::new(
            pool,
            Arc::new(BuiltinLlmClient::new()),
            Arc::new(OutputValidator::new()),
            Arc::new(InMemoryBudgetEnforcer::with_cap(0.0)),
            Arc::new(PiiRedactor::with_default_regex_backend()),
        );
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let err = runtime
            .execute_with_context(
                &agent,
                json!({"text": "x", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap_err();
        match err {
            NanoAgentError::Budget(crate::error::BudgetError::Insufficient { .. }) => {}
            other => panic!("expected Budget(Insufficient), got {other:?}"),
        }
    }

    // ── PII redaction is invoked ────────────────────────────────────

    #[tokio::test]
    async fn pii_redactor_is_called_for_mode_c_pre_llm_redact() {
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());
        let pii_mock = Arc::new(crate::runtime::pii_redactor::MockPiiBackend::passthrough());
        let runtime = LlmAugmentedRuntime::new(
            pool,
            Arc::new(BuiltinLlmClient::new()),
            Arc::new(OutputValidator::new()),
            Arc::new(InMemoryBudgetEnforcer::new()),
            Arc::new(PiiRedactor::new(pii_mock.clone())),
        );
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let _ = runtime
            .execute_with_context(
                &agent,
                json!({"text": "Email me at user@host.com", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap();
        // PII redactor should have been called for the "text" field at minimum.
        assert!(pii_mock.call_count() >= 1, "PII redactor never called");
    }

    #[tokio::test]
    async fn pii_redactor_skipped_when_strategy_is_none() {
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());
        let pii_mock = Arc::new(crate::runtime::pii_redactor::MockPiiBackend::passthrough());
        let runtime = LlmAugmentedRuntime::new(
            pool,
            Arc::new(BuiltinLlmClient::new()),
            Arc::new(OutputValidator::new()),
            Arc::new(InMemoryBudgetEnforcer::new()),
            Arc::new(PiiRedactor::new(pii_mock.clone())),
        );
        let mut agent = make_mode_c_agent_summarize();
        agent.compliance.pii_redaction_strategy = PiiStrategy::None;
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let _ = runtime
            .execute_with_context(
                &agent,
                json!({"text": "no redaction wanted", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(pii_mock.call_count(), 0);
    }

    // ── Output validation failure → retry path ──────────────────────

    #[tokio::test]
    async fn output_schema_mismatch_retries_once_then_succeeds_on_retry_response() {
        // Build a MockLlmClient that returns bad-shape JSON first, then
        // good-shape on retry (we can detect retry via _retry_reminder
        // in the inputs).
        struct RetryAwareMock {
            count: tokio::sync::Mutex<u32>,
        }
        #[async_trait::async_trait]
        impl LlmClient for RetryAwareMock {
            async fn call(
                &self,
                req: LlmCallRequest,
            ) -> Result<crate::runtime::llm_client::LlmCallResponse, LlmCallError> {
                let mut c = self.count.lock().await;
                *c += 1;
                let attempt = *c;
                let content = if attempt == 1 {
                    // First attempt : missing required `sentence_count` → fails validation.
                    json!({"summary": "wrong shape"})
                } else {
                    // Second attempt detects _retry_reminder + returns good shape.
                    assert!(
                        req.template_inputs.get("_retry_reminder").is_some(),
                        "retry didn't inject _retry_reminder"
                    );
                    json!({"summary": "good", "sentence_count": 1})
                };
                Ok(crate::runtime::llm_client::LlmCallResponse {
                    content,
                    tokens_in: 10,
                    tokens_out: 5,
                    cost_cents: 0.5,
                    provider_name: "mock".into(),
                    model_name: "retry-mock".into(),
                    cache_hit: false,
                    duration_ms: 1,
                    retry_attempts: 0,
                })
            }
            fn estimate_cost_cents(&self, _: &LlmCallRequest) -> f64 {
                0.5
            }
        }

        let llm = Arc::new(RetryAwareMock {
            count: tokio::sync::Mutex::new(0),
        });
        let pool = Arc::new(StubVmPool::with_llm(
            crate::runtime::pool::stub::StubVmPoolConfig::default(),
            Arc::new(crate::runtime::model_loader::InProcessModelLoader::with_phase_2_builtins()),
            llm.clone(),
        ));
        let runtime = LlmAugmentedRuntime::new(
            pool,
            llm.clone(),
            Arc::new(OutputValidator::new()),
            Arc::new(InMemoryBudgetEnforcer::new()),
            Arc::new(PiiRedactor::with_default_regex_backend()),
        );
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let resp = runtime
            .execute_with_context(
                &agent,
                json!({"text": "Hello.", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(resp.output["summary"], "good");
    }

    // ── Retry exhausted → SchemaValidationFailed ────────────────────

    #[tokio::test]
    async fn output_schema_persistent_failure_after_retry_surfaces() {
        // Always-bad-shape LLM client.
        let mock_llm = Arc::new(MockLlmClient::new());
        mock_llm
            .set_canned(
                "m50.summarize_strict.v1",
                MockResponse::Ok(crate::runtime::llm_client::LlmCallResponse {
                    content: json!({"unrelated": "field"}),
                    tokens_in: 10,
                    tokens_out: 5,
                    cost_cents: 0.5,
                    provider_name: "mock".into(),
                    model_name: "always-bad".into(),
                    cache_hit: false,
                    duration_ms: 1,
                    retry_attempts: 0,
                }),
            )
            .await;
        let pool = Arc::new(StubVmPool::with_llm(
            crate::runtime::pool::stub::StubVmPoolConfig::default(),
            Arc::new(crate::runtime::model_loader::InProcessModelLoader::with_phase_2_builtins()),
            mock_llm.clone(),
        ));
        let runtime = LlmAugmentedRuntime::new(
            pool,
            mock_llm.clone(),
            Arc::new(OutputValidator::new()),
            Arc::new(InMemoryBudgetEnforcer::new()),
            Arc::new(PiiRedactor::with_default_regex_backend()),
        );
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let err = runtime
            .execute_with_context(
                &agent,
                json!({"text": "x", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap_err();
        match err {
            NanoAgentError::OutputValidation(crate::error::ValidationError::SchemaMismatch(_)) => {}
            other => panic!("expected OutputValidation/SchemaMismatch, got {other:?}"),
        }
        // 2 attempts made.
        assert_eq!(mock_llm.call_count(), 2);
    }

    // ── Past deadline ───────────────────────────────────────────────

    #[tokio::test]
    async fn past_deadline_returns_timeout() {
        let runtime = make_runtime_builtin();
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = CallContext::stand_alone(TenantId::system(), audit, 0);
        tokio::time::sleep(Duration::from_millis(5)).await;
        let err = runtime
            .execute_with_context(
                &agent,
                json!({"text": "x", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, NanoAgentError::Timeout(_)));
    }

    // ── Provider error path → release reservation ───────────────────

    #[tokio::test]
    async fn provider_error_releases_reservation() {
        let llm = Arc::new(BuiltinLlmClient::new());
        llm.force_error("simulated").await;
        let pool = Arc::new(StubVmPool::with_llm(
            crate::runtime::pool::stub::StubVmPoolConfig::default(),
            Arc::new(crate::runtime::model_loader::InProcessModelLoader::with_phase_2_builtins()),
            llm.clone(),
        ));
        let budget = Arc::new(InMemoryBudgetEnforcer::with_cap(100.0));
        let runtime = LlmAugmentedRuntime::new(
            pool,
            llm,
            Arc::new(OutputValidator::new()),
            budget.clone(),
            Arc::new(PiiRedactor::with_default_regex_backend()),
        );
        let agent = make_mode_c_agent_summarize();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let _ = runtime
            .execute_with_context(
                &agent,
                json!({"text": "x", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap_err();
        // Reservation must be released (snapshot shows 0 reserved).
        let snap = budget.snapshot(ctx.tenant_id).await.unwrap();
        assert_eq!(snap.reserved_cents, 0.0);
        // No budget was charged (no commit happened).
        assert_eq!(snap.used_cents, 0.0);
    }

    // ── PII Strict mode leak detection ──────────────────────────────

    #[tokio::test]
    async fn strict_mode_rejects_output_containing_redaction_token() {
        // MockLlmClient that echoes a redaction token in output.
        let mock_llm = Arc::new(MockLlmClient::new());
        mock_llm
            .set_canned(
                "m50.summarize_strict.v1",
                MockResponse::Ok(crate::runtime::llm_client::LlmCallResponse {
                    content: json!({
                        "summary": "Sent email to [EMAIL_1] today",
                        "sentence_count": 1
                    }),
                    tokens_in: 10,
                    tokens_out: 5,
                    cost_cents: 0.5,
                    provider_name: "mock".into(),
                    model_name: "leak-mock".into(),
                    cache_hit: false,
                    duration_ms: 1,
                    retry_attempts: 0,
                }),
            )
            .await;
        let pool = Arc::new(StubVmPool::with_llm(
            crate::runtime::pool::stub::StubVmPoolConfig::default(),
            Arc::new(crate::runtime::model_loader::InProcessModelLoader::with_phase_2_builtins()),
            mock_llm.clone(),
        ));
        let runtime = LlmAugmentedRuntime::new(
            pool,
            mock_llm,
            Arc::new(OutputValidator::new()),
            Arc::new(InMemoryBudgetEnforcer::new()),
            Arc::new(PiiRedactor::with_default_regex_backend()),
        );
        let mut agent = make_mode_c_agent_summarize();
        agent.compliance.pii_redaction_strategy = PiiStrategy::Strict;
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = make_ctx(audit).await;

        let err = runtime
            .execute_with_context(
                &agent,
                json!({"text": "Email me at user@host.com", "max_sentences": 1}),
                &ctx,
            )
            .await
            .unwrap_err();
        match err {
            NanoAgentError::LlmCall(LlmCallError::PiiLeakDetected(token)) => {
                assert!(token.contains("[EMAIL_"));
            }
            other => panic!("expected PiiLeakDetected, got {other:?}"),
        }
    }

    // ── Warm output validator ───────────────────────────────────────

    #[tokio::test]
    async fn warm_output_validator_pre_compiles_schemas() {
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());
        let runtime = LlmAugmentedRuntime::with_phase_2_defaults(pool);
        let n = runtime
            .warm_output_validator(&[
                json!({"type": "string"}),
                json!({"type": "integer"}),
            ])
            .unwrap();
        assert_eq!(n, 2);
    }
}
