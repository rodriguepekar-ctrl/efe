// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + G1 + Workflow A** : `NanoToSwarm` — the
//! forward adapter that turns a `(NanoAgent, NanoRunner)` pair
//! into a `SwarmAgent`.
//!
//! See `docs/architecture/EFE_M50_N23_NANO_SWARM_ADAPTER_ARCHITECTURE.md`
//! § 10.1 for the annotated dispatch flow.

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use efe_blackboard::SwarmEvent;
use efe_nano_sdk::{
    traits::runtime::{NanoAgentResponse, TraceId},
    AgentId, IdempotencyPolicy, NanoAgent,
};
use efe_swarm_orchestrator::{
    AgentOutcome, SwarmAgent, SwarmAgentContext, SwarmAgentError,
};
use crate::cost::cents_to_microcents;
use crate::determinism::SeedGuard;
use crate::envelope::{NanoEnvelopeOutcome, NanoEnvelopeV1};
use crate::error::AdapterError;
use crate::policy::{compute_cache_key, NanoToSwarmPolicy};
use crate::runner::{NanoRunner, NanoRunnerOutcome};
use crate::ENVELOPE_TOPIC;

/// **N23** : default blackboard input pointer.
///
/// Each preanalysis agent has its own canonical input pointer ;
/// `NerExtractor`, `LanguageDetector`, `PiiRedactor`,
/// `SentimentClassifier`, and `SummaryWriter` all read
/// `/input/text` and `SchemaValidator` reads `/input`.  The
/// adapter defaults to `/input/text` and exposes
/// [`NanoToSwarm::with_input_pointer`] for overrides.
pub const DEFAULT_INPUT_POINTER: &str = "/input/text";

/// **N23** : blackboard key prefix the adapter writes to.
pub const NANO_OUTPUT_PREFIX: &str = "/nano";

/// **N23 + G1** : the forward adapter.
///
/// Wraps a `(Arc<NanoAgent>, Arc<dyn NanoRunner>,
/// NanoToSwarmPolicy)` tuple and implements `SwarmAgent`.
pub struct NanoToSwarm {
    /// The catalogue entry (signed, gold-bound, schema-defined).
    nano: Arc<NanoAgent>,
    /// The wrapped runner.
    runner: Arc<dyn NanoRunner>,
    /// Policy gates.
    policy: NanoToSwarmPolicy,
    /// Override for the swarm `AgentId` ; defaults to
    /// `nano.id.as_str()` (verbatim — preserves the existing DAG
    /// topology where `ner-extractor` etc. are hard-coded).
    agent_id_override: Option<AgentId>,
    /// Override for the blackboard input pointer.  Defaults to
    /// [`DEFAULT_INPUT_POINTER`].
    input_pointer: String,
}

impl std::fmt::Debug for NanoToSwarm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NanoToSwarm")
            .field("nano_id", &self.nano.id.as_str())
            .field("nano_version", &self.nano.version.as_str())
            .field("policy", &self.policy)
            .field("agent_id_override", &self.agent_id_override)
            .field("input_pointer", &self.input_pointer)
            .finish()
    }
}

impl NanoToSwarm {
    /// **N23** : construct a forward adapter with the default
    /// policy ([`NanoToSwarmPolicy::experimental_fast`]) and
    /// default input pointer.
    pub fn new(nano: Arc<NanoAgent>, runner: Arc<dyn NanoRunner>) -> Self {
        Self {
            nano,
            runner,
            policy: NanoToSwarmPolicy::experimental_fast(),
            agent_id_override: None,
            input_pointer: DEFAULT_INPUT_POINTER.to_string(),
        }
    }

    /// **N23 + ADR-3** : configure the policy.
    pub fn with_policy(mut self, p: NanoToSwarmPolicy) -> Self {
        self.policy = p;
        self
    }

    /// **N23** : override the derived `AgentId`.
    pub fn with_agent_id(mut self, id: AgentId) -> Self {
        self.agent_id_override = Some(id);
        self
    }

    /// **N23** : override the blackboard input pointer.
    pub fn with_input_pointer<S: Into<String>>(mut self, p: S) -> Self {
        self.input_pointer = p.into();
        self
    }

    /// **N23** : canonical view of the wrapped catalogue entry.
    pub fn nano(&self) -> &NanoAgent {
        &self.nano
    }

    fn derived_agent_id(&self) -> AgentId {
        if let Some(id) = &self.agent_id_override {
            return id.clone();
        }
        // The catalogue `NanoAgentId` allows `[a-z0-9_]` only (no
        // hyphens) ; the swarm `AgentId` historically uses hyphens
        // (`ner-extractor`, `language-detector`, ...).  To preserve
        // the existing DAG topology we translate the catalogue id
        // to its hyphenated form by default.  Override via
        // [`Self::with_agent_id`] when this convention does not
        // apply (e.g. tenant-published nanos).
        let hyphenated = self.nano.id.as_str().replace('_', "-");
        AgentId::new(&hyphenated)
            .unwrap_or_else(|_| AgentId::new("nano-adapted").expect("static fallback"))
    }
}

#[async_trait]
impl SwarmAgent for NanoToSwarm {
    fn agent_id(&self) -> AgentId {
        self.derived_agent_id()
    }

    async fn run(
        &self,
        ctx: SwarmAgentContext,
    ) -> Result<AgentOutcome, SwarmAgentError> {
        let started = Instant::now();
        let trace_id = TraceId::new();
        let nano = Arc::clone(&self.nano);
        let runner = Arc::clone(&self.runner);
        let agent_id_for_writes = ctx.agent_id.clone();
        let bb_prefix = format!("{NANO_OUTPUT_PREFIX}/{}", nano.id.as_str());

        // (0) **N24 + I12** : publisher status gate (cheap-first).
        //     Runs before signature + gold so a revoked /
        //     suspended publisher fails fast without paying
        //     crypto cost.
        if let Err(e) = run_status_gate(&self.policy, &nano).await {
            return finalise_errored(
                &ctx,
                &nano,
                &bb_prefix,
                &agent_id_for_writes,
                started,
                trace_id,
                &e,
                NanoEnvelopeOutcome::Errored,
            )
            .await;
        }

        // (1) Signature gate (G2 / I4).
        let signature_verified = match run_signature_gate(&self.policy, &nano).await {
            Ok(verified) => verified,
            Err(e) => {
                return finalise_errored(
                    &ctx,
                    &nano,
                    &bb_prefix,
                    &agent_id_for_writes,
                    started,
                    trace_id,
                    &e,
                    NanoEnvelopeOutcome::Errored,
                )
                .await;
            }
        };

        // (2) Gold binding gate (G4 / I5).
        let gold_bound = match run_gold_gate(&self.policy, &nano).await {
            Ok(bound) => bound,
            Err(e) => {
                return finalise_errored(
                    &ctx,
                    &nano,
                    &bb_prefix,
                    &agent_id_for_writes,
                    started,
                    trace_id,
                    &e,
                    NanoEnvelopeOutcome::Errored,
                )
                .await;
            }
        };

        // (3) Idempotency cache lookup (G5).
        let input_value = ctx
            .blackboard
            .read(&self.input_pointer)
            .map(|bv| bv.value)
            .unwrap_or(serde_json::Value::Null);

        let cache_key = compute_cache_key(&nano, &input_value);
        let mut cache_hit = false;
        let mut cached_response: Option<NanoAgentResponse> = None;
        if let Some(cache) = &self.policy.idempotency_cache {
            if !self.policy.caching_disabled_for(&nano)
                && matches!(
                    nano.idempotency,
                    IdempotencyPolicy::Auto { .. } | IdempotencyPolicy::Manual { .. }
                )
            {
                match cache.get(&cache_key).await {
                    Ok(Some(r)) => {
                        cache_hit = true;
                        cached_response = Some(r);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // Cache errors are soft : log + skip cache.
                        tracing::warn!(
                            adapter = "nano-to-swarm",
                            error = %e,
                            "idempotency cache get failed ; continuing without cache"
                        );
                    }
                }
            }
        }

        // (4) Dispatch (or short-circuit on cache hit).
        let (runner_outcome, response_for_cache): (NanoRunnerOutcome, Option<NanoAgentResponse>) =
            if let Some(resp) = cached_response.clone() {
                let outcome = NanoRunnerOutcome::fresh()
                    .with_cost_cents(resp.cost_cents)
                    .with_trace_id(resp.trace_id)
                    .with_cache_hit(true);
                (outcome, None)
            } else {
                // Seed forwarding (G7 / I9).
                let _seed_guard = SeedGuard::new(ctx.rng_seed);

                // Enforce limits (G9) if configured.
                let dispatch_fut = runner.run(&nano, &ctx);
                let dispatch_result = if self.policy.enforce_limits {
                    let budget = Duration::from_millis(
                        nano.limits.wall_clock_ms.max(1) as u64,
                    );
                    match tokio::time::timeout(budget, dispatch_fut).await {
                        Ok(r) => r,
                        Err(_) => {
                            return finalise_timed_out(
                                &ctx,
                                &nano,
                                &bb_prefix,
                                &agent_id_for_writes,
                                started,
                                trace_id,
                                nano.limits.wall_clock_ms,
                            )
                            .await;
                        }
                    }
                } else {
                    dispatch_fut.await
                };
                drop(_seed_guard);

                let outcome = match dispatch_result {
                    Ok(o) => o,
                    Err(e) => {
                        return finalise_errored(
                            &ctx,
                            &nano,
                            &bb_prefix,
                            &agent_id_for_writes,
                            started,
                            trace_id,
                            &e,
                            NanoEnvelopeOutcome::Errored,
                        )
                        .await;
                    }
                };

                let synthetic_response = NanoAgentResponse {
                    output: serde_json::json!(null),
                    latency_ms: started.elapsed().as_millis() as u32,
                    cost_cents: outcome.cost_cents,
                    trace_id: outcome.trace_id,
                    cache_hit: false,
                };
                (outcome, Some(synthetic_response))
            };

        // (5) Cache insert.
        if !cache_hit {
            if let (Some(cache), Some(resp)) = (
                &self.policy.idempotency_cache,
                response_for_cache.as_ref(),
            ) {
                if !self.policy.caching_disabled_for(&nano) {
                    let ttl = self.policy.effective_ttl(&nano);
                    if let Err(e) = cache.put(&cache_key, resp.clone(), ttl).await {
                        tracing::warn!(
                            adapter = "nano-to-swarm",
                            error = %e,
                            "idempotency cache put failed ; envelope still emitted"
                        );
                    }
                }
            }
        }

        // (6) Cost translation (G6 / I6).
        let estimate_microcents = cents_to_microcents(nano.cost_estimate_cents);
        let actual_microcents = cents_to_microcents(runner_outcome.cost_cents);

        // (7) Blackboard writes (G6 / I7).
        let adapter_writes = write_adapter_audit(
            &ctx,
            &bb_prefix,
            &agent_id_for_writes,
            estimate_microcents,
            actual_microcents,
            cache_hit,
            signature_verified,
            gold_bound,
            runner_outcome.trace_id,
            &nano,
            started,
        )
        .await?;

        let latency_ms = (started.elapsed().as_millis() as u32).max(1);

        // (8) Envelope emission (G8 / I8 / F9).
        let envelope = NanoEnvelopeV1::new(
            ctx.agent_id.as_str(),
            nano.id.as_str(),
            nano.version.as_str(),
            ctx.swarm_id.as_uuid(),
            trace_id.0,
            nano.execution_mode.mode_str(),
            estimate_microcents,
            actual_microcents,
            cache_hit,
            signature_verified,
            gold_bound,
            latency_ms,
            NanoEnvelopeOutcome::Completed,
        );
        let events_published = publish_envelope(&ctx, &envelope, &bb_prefix, &agent_id_for_writes)?;

        // (9) Final outcome.
        let total_writes = runner_outcome.blackboard_writes.saturating_add(adapter_writes);
        let total_events = runner_outcome.events_published.saturating_add(events_published);
        Ok(AgentOutcome::Completed {
            cost_microcents: actual_microcents,
            duration_ms: latency_ms,
            blackboard_writes: total_writes,
            events_published: total_events,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Gate helpers
// ─────────────────────────────────────────────────────────────────────

/// **N24 + G4 + I12** : publisher status gate dispatch.
///
/// Cheap-first : runs BEFORE the signature + gold gates so a
/// revoked / suspended publisher fails without paying crypto
/// cost.  No-op when `policy.require_publisher_status == false`
/// (the default).
async fn run_status_gate(
    policy: &NanoToSwarmPolicy,
    nano: &NanoAgent,
) -> Result<(), AdapterError> {
    if !policy.require_publisher_status {
        return Ok(());
    }
    let gate = policy
        .publisher_status_gate
        .as_ref()
        .ok_or_else(|| AdapterError::DispatchFailed(
            "policy.require_publisher_status=true but no publisher_status_gate configured".into(),
        ))?;
    gate.assert_active(nano)
        .await
        .map_err(AdapterError::DispatchFailed)?;
    Ok(())
}

async fn run_signature_gate(
    policy: &NanoToSwarmPolicy,
    nano: &NanoAgent,
) -> Result<bool, AdapterError> {
    if !policy.verify_signature {
        return Ok(false);
    }
    let gate = policy
        .signature_gate
        .as_ref()
        .ok_or_else(|| AdapterError::SignatureInvalid(
            "policy.verify_signature=true but no signature_gate configured".into(),
        ))?;
    let signature = nano.signature.as_ref().ok_or_else(|| {
        AdapterError::SignatureInvalid(format!(
            "catalogue entry {}.{} has no signature",
            nano.id.as_str(),
            nano.version.as_str()
        ))
    })?;
    gate.verify(nano, signature)
        .await
        .map_err(AdapterError::SignatureInvalid)?;
    Ok(true)
}

async fn run_gold_gate(
    policy: &NanoToSwarmPolicy,
    nano: &NanoAgent,
) -> Result<bool, AdapterError> {
    if !policy.require_gold_binding {
        return Ok(false);
    }
    let gate = policy
        .gold_gate
        .as_ref()
        .ok_or_else(|| AdapterError::GoldBindingMissing(
            "policy.require_gold_binding=true but no gold_gate configured".into(),
        ))?;
    if nano.gold_set_ref.is_none() {
        return Err(AdapterError::GoldBindingMissing(format!(
            "catalogue entry {}.{} has no gold_set_ref",
            nano.id.as_str(),
            nano.version.as_str()
        )));
    }
    gate.assert_approved(nano)
        .await
        .map_err(AdapterError::GoldBindingMissing)?;
    Ok(true)
}

// ─────────────────────────────────────────────────────────────────────
//  Audit writes
// ─────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn write_adapter_audit(
    ctx: &SwarmAgentContext,
    prefix: &str,
    written_by: &AgentId,
    estimate_microcents: i64,
    actual_microcents: i64,
    cache_hit: bool,
    signature_verified: bool,
    gold_bound: bool,
    trace_id: TraceId,
    nano: &NanoAgent,
    started: Instant,
) -> Result<u32, SwarmAgentError> {
    let mut writes = 0u32;

    let cost_value = serde_json::json!({
        "estimate_microcents": estimate_microcents,
        "actual_microcents": actual_microcents,
        "variance_microcents": actual_microcents.saturating_sub(estimate_microcents),
    });
    ctx.blackboard
        .write(
            &format!("{prefix}/cost"),
            cost_value,
            written_by.clone(),
        )
        .map_err(|e| SwarmAgentError::Blackboard {
            message: format!("write {prefix}/cost : {e}"),
        })?;
    writes += 1;

    ctx.blackboard
        .write(
            &format!("{prefix}/trace"),
            serde_json::json!(trace_id.0.to_string()),
            written_by.clone(),
        )
        .map_err(|e| SwarmAgentError::Blackboard {
            message: format!("write {prefix}/trace : {e}"),
        })?;
    writes += 1;

    ctx.blackboard
        .write(
            &format!("{prefix}/mode"),
            serde_json::json!(nano.execution_mode.mode_str()),
            written_by.clone(),
        )
        .map_err(|e| SwarmAgentError::Blackboard {
            message: format!("write {prefix}/mode : {e}"),
        })?;
    writes += 1;

    ctx.blackboard
        .write(
            &format!("{prefix}/cache_hit"),
            serde_json::Value::Bool(cache_hit),
            written_by.clone(),
        )
        .map_err(|e| SwarmAgentError::Blackboard {
            message: format!("write {prefix}/cache_hit : {e}"),
        })?;
    writes += 1;

    ctx.blackboard
        .write(
            &format!("{prefix}/signature_verified"),
            serde_json::Value::Bool(signature_verified),
            written_by.clone(),
        )
        .map_err(|e| SwarmAgentError::Blackboard {
            message: format!("write {prefix}/signature_verified : {e}"),
        })?;
    writes += 1;

    ctx.blackboard
        .write(
            &format!("{prefix}/gold_bound"),
            serde_json::Value::Bool(gold_bound),
            written_by.clone(),
        )
        .map_err(|e| SwarmAgentError::Blackboard {
            message: format!("write {prefix}/gold_bound : {e}"),
        })?;
    writes += 1;

    let latency_ms = (started.elapsed().as_millis() as u64).min(u32::MAX as u64) as u32;
    ctx.blackboard
        .write(
            &format!("{prefix}/latency_ms"),
            serde_json::json!(latency_ms),
            written_by.clone(),
        )
        .map_err(|e| SwarmAgentError::Blackboard {
            message: format!("write {prefix}/latency_ms : {e}"),
        })?;
    writes += 1;

    Ok(writes)
}

fn publish_envelope(
    ctx: &SwarmAgentContext,
    envelope: &NanoEnvelopeV1,
    prefix: &str,
    written_by: &AgentId,
) -> Result<u32, SwarmAgentError> {
    let payload = serde_json::to_value(envelope).unwrap_or(serde_json::Value::Null);
    let event = SwarmEvent {
        swarm_id: ctx.swarm_id,
        published_by: ctx.agent_id.clone(),
        topic: ENVELOPE_TOPIC.to_string(),
        payload: payload.clone(),
        published_at: std::time::SystemTime::now(),
    };
    let publish_result = ctx.eventbus.publish(event);
    let subscriber_count = publish_result.unwrap_or(0);
    if subscriber_count == 0 {
        // F9 : persist to blackboard as fallback.
        let _ = ctx.blackboard.write(
            &format!("{prefix}/envelope"),
            payload,
            written_by.clone(),
        );
    }
    Ok(1)
}

// ─────────────────────────────────────────────────────────────────────
//  Errored / timed-out finalisers
// ─────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn finalise_errored(
    ctx: &SwarmAgentContext,
    nano: &NanoAgent,
    prefix: &str,
    written_by: &AgentId,
    started: Instant,
    trace_id: TraceId,
    err: &AdapterError,
    envelope_outcome: NanoEnvelopeOutcome,
) -> Result<AgentOutcome, SwarmAgentError> {
    let latency_ms = (started.elapsed().as_millis() as u32).max(1);
    let estimate_microcents = cents_to_microcents(nano.cost_estimate_cents);
    let signature_verified = matches!(err, AdapterError::SignatureInvalid(_)).then_some(false);
    let gold_bound = matches!(err, AdapterError::GoldBindingMissing(_)).then_some(false);
    let envelope = NanoEnvelopeV1::new(
        ctx.agent_id.as_str(),
        nano.id.as_str(),
        nano.version.as_str(),
        ctx.swarm_id.as_uuid(),
        trace_id.0,
        nano.execution_mode.mode_str(),
        estimate_microcents,
        0,
        false,
        signature_verified.unwrap_or(false),
        gold_bound.unwrap_or(false),
        latency_ms,
        envelope_outcome,
    );
    let _ = publish_envelope(ctx, &envelope, prefix, written_by);

    // Hard errors propagate.
    if !err.is_soft() {
        return Err(err.clone().into());
    }

    Ok(AgentOutcome::Errored {
        kind: err.agent_error_kind(),
        message: err.to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn finalise_timed_out(
    ctx: &SwarmAgentContext,
    nano: &NanoAgent,
    prefix: &str,
    written_by: &AgentId,
    started: Instant,
    trace_id: TraceId,
    timeout_ms: u32,
) -> Result<AgentOutcome, SwarmAgentError> {
    let latency_ms = (started.elapsed().as_millis() as u32).max(1);
    let estimate_microcents = cents_to_microcents(nano.cost_estimate_cents);
    let envelope = NanoEnvelopeV1::new(
        ctx.agent_id.as_str(),
        nano.id.as_str(),
        nano.version.as_str(),
        ctx.swarm_id.as_uuid(),
        trace_id.0,
        nano.execution_mode.mode_str(),
        estimate_microcents,
        0,
        false,
        false,
        false,
        latency_ms,
        NanoEnvelopeOutcome::TimedOut,
    );
    let _ = publish_envelope(ctx, &envelope, prefix, written_by);
    Ok(AgentOutcome::TimedOut { timeout_ms })
}

// ─────────────────────────────────────────────────────────────────────
//  Unit tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::NanoToSwarmPolicy;
    use crate::runner::NanoRunnerOutcome;
    use efe_blackboard::{Blackboard, EventBus};
    use efe_nano_sdk::{
        Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, NanoAgentId,
        NanoAgentVersion, ResourceLimits, StabilityTier, SwarmId,
    };
    use efe_swarm_orchestrator::AgentErrorKind;

    fn make_nano(id: &str) -> Arc<NanoAgent> {
        Arc::new(NanoAgent {
            id: NanoAgentId::new(id).unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author: Author::Efe,
            category: Category::Extraction,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "x::run".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.01,
            latency_p99_ms_target: 50,
            audit: Default::default(),
            compliance: Default::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        })
    }

    fn make_ctx(input_text: &str, agent_id: &str) -> SwarmAgentContext {
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        bb.write(
            DEFAULT_INPUT_POINTER,
            serde_json::json!(input_text),
            AgentId::new("seed").unwrap(),
        )
        .unwrap();
        SwarmAgentContext {
            swarm_id: bb.swarm_id(),
            agent_id: AgentId::new(agent_id).unwrap(),
            blackboard: bb,
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(serde_json::json!({"text": input_text})),
            rng_seed: 0,
        }
    }

    struct EchoRunner;

    #[async_trait]
    impl NanoRunner for EchoRunner {
        async fn run(
            &self,
            _nano: &NanoAgent,
            ctx: &SwarmAgentContext,
        ) -> Result<NanoRunnerOutcome, AdapterError> {
            let text = ctx
                .blackboard
                .read(DEFAULT_INPUT_POINTER)
                .and_then(|v| v.value.as_str().map(String::from))
                .unwrap_or_default();
            ctx.blackboard
                .write(
                    "/echo",
                    serde_json::json!(text),
                    ctx.agent_id.clone(),
                )
                .map_err(|e| AdapterError::BlackboardWriteFailed {
                    pointer: "/echo".into(),
                    message: e.to_string(),
                })?;
            Ok(NanoRunnerOutcome::fresh()
                .with_blackboard_writes(1)
                .with_cost_cents(0.0))
        }
    }

    #[tokio::test]
    async fn happy_path_completes_with_envelope_persisted_to_blackboard() {
        let nano = make_nano("echo");
        let runner = Arc::new(EchoRunner);
        let adapter = NanoToSwarm::new(nano, runner);
        let ctx = make_ctx("hello", "echo");
        let bb = ctx.blackboard.clone();

        let outcome = adapter.run(ctx).await.unwrap();
        assert!(matches!(outcome, AgentOutcome::Completed { .. }));

        // Runner wrote its own output.
        assert_eq!(bb.read("/echo").unwrap().value, serde_json::json!("hello"));

        // Adapter wrote audit fields.
        assert!(bb.read("/nano/echo/cost").is_some());
        assert!(bb.read("/nano/echo/trace").is_some());
        assert!(bb.read("/nano/echo/mode").is_some());
        assert_eq!(
            bb.read("/nano/echo/cache_hit").unwrap().value,
            serde_json::Value::Bool(false)
        );

        // F9 fallback : the envelope is persisted (no subscribers).
        assert!(bb.read("/nano/echo/envelope").is_some());
    }

    #[tokio::test]
    async fn agent_id_uses_nano_id_by_default() {
        let nano = make_nano("ner_extractor");
        let runner = Arc::new(EchoRunner);
        let adapter = NanoToSwarm::new(nano, runner);
        assert_eq!(adapter.agent_id().as_str(), "ner-extractor");
    }

    #[tokio::test]
    async fn agent_id_override_applies() {
        let nano = make_nano("ner_extractor");
        let runner = Arc::new(EchoRunner);
        let adapter = NanoToSwarm::new(nano, runner)
            .with_agent_id(AgentId::new("ner-v2").unwrap());
        assert_eq!(adapter.agent_id().as_str(), "ner-v2");
    }

    #[tokio::test]
    async fn input_pointer_override_applies() {
        let nano = make_nano("echo");
        let runner = Arc::new(EchoRunner);
        // Build adapter with a custom input pointer.
        let adapter = NanoToSwarm::new(nano, runner).with_input_pointer("/custom/input");
        let bb = Arc::new(Blackboard::new(SwarmId::new()));
        bb.write(
            "/custom/input",
            serde_json::json!("custom hello"),
            AgentId::new("seed").unwrap(),
        )
        .unwrap();
        let ctx = SwarmAgentContext {
            swarm_id: bb.swarm_id(),
            agent_id: AgentId::new("echo").unwrap(),
            blackboard: bb.clone(),
            eventbus: Arc::new(EventBus::new()),
            input: Arc::new(serde_json::json!({})),
            rng_seed: 0,
        };
        let outcome = adapter.run(ctx).await.unwrap();
        assert!(matches!(outcome, AgentOutcome::Completed { .. }));
        // EchoRunner reads DEFAULT_INPUT_POINTER unconditionally
        // (its own contract) so it sees nothing — the test
        // confirms the adapter doesn't crash on a different
        // input_pointer.
        assert!(bb.read("/nano/echo/cost").is_some());
    }

    #[tokio::test]
    async fn signature_gate_required_but_missing_emits_errored() {
        struct AlwaysOkSig;
        #[async_trait]
        impl crate::policy::SignatureGate for AlwaysOkSig {
            async fn verify(
                &self,
                _nano: &NanoAgent,
                _sig: &efe_nano_sdk::PublisherSignature,
            ) -> Result<(), String> {
                Ok(())
            }
        }
        let nano = make_nano("ner_extractor");
        // No signature attached to nano.
        let mut policy = NanoToSwarmPolicy::experimental_fast();
        policy.verify_signature = true;
        policy.signature_gate = Some(Arc::new(AlwaysOkSig));
        let adapter = NanoToSwarm::new(nano, Arc::new(EchoRunner)).with_policy(policy);
        let ctx = make_ctx("hello", "ner-extractor");
        let outcome = adapter.run(ctx).await.unwrap();
        match outcome {
            AgentOutcome::Errored { kind, message } => {
                assert_eq!(kind, AgentErrorKind::ComplianceFailed);
                assert!(message.to_lowercase().contains("signature"));
            }
            o => panic!("expected Errored, got {o:?}"),
        }
    }

    #[tokio::test]
    async fn gold_gate_required_but_missing_emits_errored() {
        struct AlwaysOkGold;
        #[async_trait]
        impl crate::policy::GoldBindingGate for AlwaysOkGold {
            async fn assert_approved(&self, _: &NanoAgent) -> Result<(), String> {
                Ok(())
            }
        }
        let nano = make_nano("ner_extractor");
        let mut policy = NanoToSwarmPolicy::experimental_fast();
        policy.require_gold_binding = true;
        policy.gold_gate = Some(Arc::new(AlwaysOkGold));
        let adapter = NanoToSwarm::new(nano, Arc::new(EchoRunner)).with_policy(policy);
        let ctx = make_ctx("hello", "ner-extractor");
        let outcome = adapter.run(ctx).await.unwrap();
        match outcome {
            AgentOutcome::Errored { kind, .. } => {
                assert_eq!(kind, AgentErrorKind::ComplianceFailed);
            }
            o => panic!("expected Errored, got {o:?}"),
        }
    }

    #[tokio::test]
    async fn idempotency_cache_hit_skips_dispatch() {
        use crate::policy::InMemoryIdempotencyCache;
        let cache = Arc::new(InMemoryIdempotencyCache::new());
        let nano = make_nano("echo");
        let policy = NanoToSwarmPolicy::cache_only(cache.clone());
        let adapter1 = NanoToSwarm::new(Arc::clone(&nano), Arc::new(EchoRunner))
            .with_policy(policy.clone());
        let adapter2 = NanoToSwarm::new(nano, Arc::new(EchoRunner)).with_policy(policy);

        // First call : miss → dispatch → put.
        let ctx1 = make_ctx("hello", "echo");
        let _ = adapter1.run(ctx1).await.unwrap();
        assert_eq!(cache.len(), 1);

        // Second call (same input) : hit → skip dispatch.
        let ctx2 = make_ctx("hello", "echo");
        let bb2 = ctx2.blackboard.clone();
        let outcome = adapter2.run(ctx2).await.unwrap();
        assert!(matches!(outcome, AgentOutcome::Completed { .. }));
        // /nano/echo/cache_hit should be true.
        let hit = bb2.read("/nano/echo/cache_hit").unwrap().value;
        assert_eq!(hit, serde_json::Value::Bool(true));
        // EchoRunner was NOT invoked the second time → /echo absent.
        assert!(bb2.read("/echo").is_none());
    }

    struct FailingRunner;
    #[async_trait]
    impl NanoRunner for FailingRunner {
        async fn run(
            &self,
            _nano: &NanoAgent,
            _ctx: &SwarmAgentContext,
        ) -> Result<NanoRunnerOutcome, AdapterError> {
            Err(AdapterError::DispatchFailed("boom".into()))
        }
    }

    #[tokio::test]
    async fn dispatch_failure_yields_errored_outcome() {
        let nano = make_nano("echo");
        let adapter = NanoToSwarm::new(nano, Arc::new(FailingRunner));
        let ctx = make_ctx("hello", "echo");
        let outcome = adapter.run(ctx).await.unwrap();
        match outcome {
            AgentOutcome::Errored { kind, message } => {
                assert_eq!(kind, AgentErrorKind::ProviderError);
                assert!(message.contains("boom"));
            }
            o => panic!("expected Errored, got {o:?}"),
        }
    }

    struct SlowRunner;
    #[async_trait]
    impl NanoRunner for SlowRunner {
        async fn run(
            &self,
            _nano: &NanoAgent,
            _ctx: &SwarmAgentContext,
        ) -> Result<NanoRunnerOutcome, AdapterError> {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok(NanoRunnerOutcome::fresh())
        }
    }

    #[tokio::test]
    async fn limits_timeout_yields_timed_out_outcome() {
        // Set the catalogue limit to 50 ms ; SlowRunner sleeps 500 ms.
        let mut na = make_nano("echo");
        Arc::get_mut(&mut na).unwrap().limits.wall_clock_ms = 50;
        let adapter = NanoToSwarm::new(na, Arc::new(SlowRunner));
        let ctx = make_ctx("hello", "echo");
        let outcome = adapter.run(ctx).await.unwrap();
        match outcome {
            AgentOutcome::TimedOut { timeout_ms } => assert_eq!(timeout_ms, 50),
            o => panic!("expected TimedOut, got {o:?}"),
        }
    }

    #[tokio::test]
    async fn idempotency_disabled_skips_cache_entirely() {
        use crate::policy::InMemoryIdempotencyCache;
        let cache = Arc::new(InMemoryIdempotencyCache::new());
        let mut na = make_nano("echo");
        Arc::get_mut(&mut na).unwrap().idempotency = IdempotencyPolicy::Disabled;
        let policy = NanoToSwarmPolicy::cache_only(cache.clone());
        let adapter1 = NanoToSwarm::new(Arc::clone(&na), Arc::new(EchoRunner))
            .with_policy(policy.clone());
        let adapter2 = NanoToSwarm::new(na, Arc::new(EchoRunner)).with_policy(policy);

        let _ = adapter1.run(make_ctx("hello", "echo")).await.unwrap();
        assert_eq!(cache.len(), 0); // no put.

        // Second call : runner still invoked (no cache).
        let ctx2 = make_ctx("hello", "echo");
        let bb2 = ctx2.blackboard.clone();
        let _ = adapter2.run(ctx2).await.unwrap();
        assert_eq!(
            bb2.read("/echo").unwrap().value,
            serde_json::json!("hello")
        );
    }

    #[tokio::test]
    async fn envelope_emits_to_event_bus_when_subscribed() {
        let nano = make_nano("echo");
        let adapter = NanoToSwarm::new(nano, Arc::new(EchoRunner));
        let ctx = make_ctx("hello", "echo");
        let mut sub = ctx.eventbus.subscribe();
        let _ = adapter.run(ctx).await.unwrap();
        let ev = sub.try_recv().expect("envelope on bus");
        assert_eq!(ev.topic, ENVELOPE_TOPIC);
        let env: NanoEnvelopeV1 = serde_json::from_value(ev.payload).unwrap();
        assert_eq!(env.nano_id, "echo");
        assert_eq!(env.outcome, NanoEnvelopeOutcome::Completed);
    }

    #[tokio::test]
    async fn cost_translation_exact() {
        let mut na = make_nano("echo");
        Arc::get_mut(&mut na).unwrap().cost_estimate_cents = 0.05;
        struct C5;
        #[async_trait]
        impl NanoRunner for C5 {
            async fn run(
                &self,
                _: &NanoAgent,
                _: &SwarmAgentContext,
            ) -> Result<NanoRunnerOutcome, AdapterError> {
                Ok(NanoRunnerOutcome::fresh().with_cost_cents(0.12))
            }
        }
        let adapter = NanoToSwarm::new(na, Arc::new(C5));
        let ctx = make_ctx("hello", "echo");
        let outcome = adapter.run(ctx).await.unwrap();
        match outcome {
            AgentOutcome::Completed { cost_microcents, .. } => {
                assert_eq!(cost_microcents, 1_200);
            }
            o => panic!("expected Completed, got {o:?}"),
        }
    }
}
