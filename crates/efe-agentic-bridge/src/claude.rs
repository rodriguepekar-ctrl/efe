// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : ClaudeBridge — § 7.4 + ADR-5 + ADR-6.
//!
//! Two-turn tool-use protocol :
//! 1. POST `/v1/messages` with `tools = [swarm_context]` + user
//!    message.  Claude responds with `tool_use` block requesting
//!    the swarm context.
//! 2. POST `/v1/messages` again with the prior assistant turn + a
//!    `tool_result` block carrying the serialised ContextBundle.
//!    Claude returns the final answer.
//!
//! Sprint 13 ships sync (non-streaming) ; Sprint 14 wires streaming
//! via Server-Sent Events.

use async_trait::async_trait;
use efe_llm_provenance::{
    hash::{hash_bytes, hash_str, prompt_hashes_for},
    LlmProvenanceCollector, PostCallProvenance, PreCallProvenance,
};
use efe_nano_sdk::{
    Completeness, ContextBundle, CostAccounting, FinishReason, ModelIdentity,
    SamplingParams, ToolCallRecord, ToolCallResultKind, MAX_CONTEXT_BUNDLE_BYTES,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bridge::AgenticBridge;
use crate::error::BridgeError;
use crate::pricing::{compute_cost_cents, compute_cost_microcents, PRICES_TABLE_VERSION};
use crate::task::{AgenticResponse, AgenticTask, StopReason, TokenUsage};
use crate::tool_use::{ToolDefinition, ToolUseBlock, SWARM_CONTEXT_TOOL_NAME};

pub const DEFAULT_CLAUDE_MODEL: &str = "claude-sonnet-4-6";
pub const DEFAULT_ANTHROPIC_ENDPOINT: &str = "https://api.anthropic.com";
pub const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

// ─────────────────────────────────────────────────────────────────────
//  N27 streaming knobs (architecture § 17.1)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 19 — N27** : default per-event SSE read timeout.
/// Anthropic emits `ping` events every ~10-15 s during long
/// completions, so 60 s is a safe upper bound.
pub const N27_DEFAULT_STREAM_READ_TIMEOUT_MS: u64 = 60_000;
/// **N27** : total stream wall-clock cap.  5 min covers the
/// slowest two-turn flows with retries.
pub const N27_DEFAULT_STREAM_TOTAL_TIMEOUT_MS: u64 = 300_000;
/// **N27** : initial TCP+TLS connect timeout (retry-eligible).
pub const N27_DEFAULT_STREAM_CONNECT_TIMEOUT_MS: u64 = 10_000;
/// **N27 + ADR-N27-5** : internal mpsc channel capacity between
/// the bridge dispatcher and the downstream SSE encoder.
pub const N27_DEFAULT_CHANNEL_CAP: usize = 64;

/// **Sprint 19 — N27** : streaming configuration for
/// [`ClaudeBridge`].  See architecture § 17.1.
#[derive(Debug, Clone, Copy)]
pub struct StreamConfig {
    pub read_timeout: Duration,
    pub total_timeout: Duration,
    pub connect_timeout: Duration,
    pub channel_cap: usize,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            read_timeout: Duration::from_millis(N27_DEFAULT_STREAM_READ_TIMEOUT_MS),
            total_timeout: Duration::from_millis(N27_DEFAULT_STREAM_TOTAL_TIMEOUT_MS),
            connect_timeout: Duration::from_millis(N27_DEFAULT_STREAM_CONNECT_TIMEOUT_MS),
            channel_cap: N27_DEFAULT_CHANNEL_CAP,
        }
    }
}

impl StreamConfig {
    /// **N27** : build from env vars (see architecture § 17.1).
    pub fn from_env() -> Self {
        fn env_u64(k: &str, d: u64) -> u64 {
            std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
        }
        fn env_usize(k: &str, d: usize) -> usize {
            std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
        }
        Self {
            read_timeout: Duration::from_millis(env_u64(
                "EFE_BRIDGE_STREAM_READ_TIMEOUT_MS",
                N27_DEFAULT_STREAM_READ_TIMEOUT_MS,
            )),
            total_timeout: Duration::from_millis(env_u64(
                "EFE_BRIDGE_STREAM_TOTAL_TIMEOUT_MS",
                N27_DEFAULT_STREAM_TOTAL_TIMEOUT_MS,
            )),
            connect_timeout: Duration::from_millis(env_u64(
                "EFE_BRIDGE_STREAM_CONNECT_TIMEOUT_MS",
                N27_DEFAULT_STREAM_CONNECT_TIMEOUT_MS,
            )),
            channel_cap: env_usize("EFE_BRIDGE_STREAM_CHANNEL_CAP", N27_DEFAULT_CHANNEL_CAP),
        }
    }
}

/// **N21 + N26** : the ClaudeBridge.
///
/// Sprint 18 N26 adds the optional N17 provenance collector :
/// when wired via [`ClaudeBridge::with_provenance`] AND the
/// caller-supplied [`AgenticTask`] carries an
/// [`AgenticAuditContext`], every two-turn dispatch produces
/// exactly one `LlmProvenanceRecord` via the existing
/// `call_with_provenance` glue.
pub struct ClaudeBridge {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    model: String,
    anthropic_version: String,
    // ─── Counters ─────────────────────────────────────────────
    //
    // **Sprint 19 — N27 audit-fix** : all atomics are
    // `Arc<AtomicU64>` (not plain `AtomicU64`) so the spawned
    // streaming-dispatch task and the bridge handle CAN share
    // the same counter.  Pre-audit, `clone_for_task` was
    // snapshotting the value into a fresh `AtomicU64` —
    // counters incremented in the spawned task never made it
    // back to the bridge handle's view.
    call_count: Arc<AtomicU64>,
    /// **N26 + G2** : optional N17 collector.  `None` preserves
    /// Sprint 13 behaviour ; `Some` enables full provenance
    /// emission.
    provenance: Option<Arc<dyn LlmProvenanceCollector>>,
    /// **N26** : counter ticked whenever the bridge skipped
    /// provenance emission (no collector or no audit context).
    provenance_skipped: Arc<AtomicU64>,
    /// **N26 + I8** : counter ticked when the collector's
    /// `post_call` returned an error (logged + ignored).
    provenance_emit_failed: Arc<AtomicU64>,
    /// **N26 + I6 + F5** : counter ticked when the collector's
    /// `post_call` exceeded the [`PROVENANCE_POST_CALL_TIMEOUT`].
    provenance_post_timeout: Arc<AtomicU64>,
    /// **Sprint 19 — N27** : retry policy used by the streaming
    /// dispatch path.  Default = [`ExponentialBackoff::default`]
    /// (500 ms base, 3 retries, ±25 % jitter).
    retry_policy: crate::retry::ExponentialBackoff,
    /// **Sprint 19 — N27** : streaming timeouts + internal
    /// channel cap.
    stream_config: StreamConfig,
    /// **Sprint 19 — N27** : counter ticked on each retry attempt
    /// (across all dispatches).  Useful for ops dashboards.
    stream_retry_attempts: Arc<AtomicU64>,
    /// **Sprint 19 — N27** : counter ticked on stream aborts
    /// (client disconnect, upstream error post-first-byte,
    /// read timeout, total timeout).
    stream_aborts: Arc<AtomicU64>,
}

/// **N26 + ADR-6 + I6** : ceiling on collector `post_call`
/// latency.  Hosted in-memory stores complete in ~1 ms ;
/// Postgres-backed in ~50 ms ; this 500 ms ceiling absorbs
/// occasional slow writes without breaking the customer-visible
/// latency SLO.
pub const PROVENANCE_POST_CALL_TIMEOUT: Duration = Duration::from_millis(500);

/// **N26 + ADR-6** : ceiling on collector `pre_call` latency.
/// The pre-call performs a registry lookup + (optionally) hash
/// verification.  An in-memory registry returns in ~1 µs ; a
/// future Postgres-backed one might be slower.  A 200 ms
/// ceiling keeps the customer-visible HTTP latency bounded
/// even when the registry is slow.  On timeout, the bridge
/// fails CLOSED — no HTTP call to Anthropic is made.
pub const PROVENANCE_PRE_CALL_TIMEOUT: Duration = Duration::from_millis(200);

impl std::fmt::Debug for ClaudeBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeBridge")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("anthropic_version", &self.anthropic_version)
            .field("api_key_present", &!self.api_key.is_empty())
            .field("call_count", &self.call_count.load(Ordering::Relaxed))
            .field("provenance_enabled", &self.provenance.is_some())
            .field(
                "provenance_skipped",
                &self.provenance_skipped.load(Ordering::Relaxed),
            )
            .field(
                "provenance_emit_failed",
                &self.provenance_emit_failed.load(Ordering::Relaxed),
            )
            .field(
                "provenance_post_timeout",
                &self.provenance_post_timeout.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl ClaudeBridge {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_endpoint(api_key, DEFAULT_ANTHROPIC_ENDPOINT)
    }

    pub fn with_endpoint(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("reqwest client"),
            api_key: api_key.into(),
            base_url: base_url.into(),
            model: DEFAULT_CLAUDE_MODEL.into(),
            anthropic_version: DEFAULT_ANTHROPIC_VERSION.into(),
            call_count: Arc::new(AtomicU64::new(0)),
            provenance: None,
            provenance_skipped: Arc::new(AtomicU64::new(0)),
            provenance_emit_failed: Arc::new(AtomicU64::new(0)),
            provenance_post_timeout: Arc::new(AtomicU64::new(0)),
            retry_policy: crate::retry::ExponentialBackoff::default(),
            stream_config: StreamConfig::default(),
            stream_retry_attempts: Arc::new(AtomicU64::new(0)),
            stream_aborts: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_anthropic_version(mut self, version: impl Into<String>) -> Self {
        self.anthropic_version = version.into();
        self
    }

    /// **N26 + G2** : inject an N17 provenance collector.  Once
    /// wired, every dispatch with a populated
    /// `AgenticTask.audit_context` emits exactly one
    /// `LlmProvenanceRecord`.
    pub fn with_provenance(
        mut self,
        collector: Arc<dyn LlmProvenanceCollector>,
    ) -> Self {
        self.provenance = Some(collector);
        self
    }

    /// **Sprint 19 — N27** : inject a retry policy.  Defaults to
    /// [`ExponentialBackoff::default`] (500 ms base / 3 retries /
    /// ±25 % jitter / 30 s total wall cap).
    ///
    /// Pass [`ExponentialBackoff::disabled`] to opt out of retry
    /// (e.g. for bench builds).
    pub fn with_retry_policy(mut self, policy: crate::retry::ExponentialBackoff) -> Self {
        self.retry_policy = policy;
        self
    }

    /// **Sprint 19 — N27** : inject a streaming config.  Defaults
    /// to the architecture § 17.1 values.
    pub fn with_stream_config(mut self, cfg: StreamConfig) -> Self {
        self.stream_config = cfg;
        self
    }

    /// **N27** : number of retry attempts performed by this
    /// bridge instance (cumulative).
    pub fn stream_retry_attempts_count(&self) -> u64 {
        self.stream_retry_attempts.load(Ordering::Relaxed)
    }

    /// **N27** : number of streams aborted (any
    /// [`AbortReason`](efe_nano_sdk::AbortReason) variant).
    pub fn stream_aborts_count(&self) -> u64 {
        self.stream_aborts.load(Ordering::Relaxed)
    }

    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// **N26** : number of dispatches where provenance was skipped
    /// (either no collector wired or no audit_context).
    pub fn provenance_skipped_count(&self) -> u64 {
        self.provenance_skipped.load(Ordering::Relaxed)
    }

    /// **N26** : number of dispatches where the collector's
    /// `post_call` returned an error (logged + ignored).
    pub fn provenance_emit_failed_count(&self) -> u64 {
        self.provenance_emit_failed.load(Ordering::Relaxed)
    }

    /// **N26** : number of dispatches where the collector's
    /// `post_call` exceeded [`PROVENANCE_POST_CALL_TIMEOUT`].
    pub fn provenance_post_timeout_count(&self) -> u64 {
        self.provenance_post_timeout.load(Ordering::Relaxed)
    }

    /// **N26** : true if provenance emission is wired.
    pub fn provenance_enabled(&self) -> bool {
        self.provenance.is_some()
    }
}

/// **N26** : internal artefact of a two-turn dispatch.  Returned
/// by [`ClaudeBridge::dispatch_two_turn`] and consumed by both
/// the provenance-on and provenance-off paths.
struct TwoTurnResult {
    /// Final assistant text.
    text: String,
    /// Summed token usage across both turns.
    usage: AnthropicUsage,
    /// Stop reason from turn 2.
    stop_reason: StopReason,
    /// Anthropic response id from turn 2 (for `provider_request_id`).
    provider_request_id: Option<String>,
    /// Wall-clock latency : turn 1 + turn 2.
    total_latency_ms: u32,
    /// The serialised turn-1 `tool_use` block for the provenance
    /// `tool_calls` field.
    tool_use_block: ToolUseBlock,
    /// The serialised bundle JSON the bridge handed Claude as
    /// `tool_result` content.  Used to compute the
    /// `prompt_inputs_hash`.
    bundle_json_bytes: Vec<u8>,
}

#[async_trait]
impl AgenticBridge for ClaudeBridge {
    /// **Sprint 19 — N27** : streaming surface.  Delegates to
    /// [`ClaudeBridge::dispatch_stream`] which drives the SSE
    /// wire end-to-end + emits the N27 provenance record.
    async fn call_with_context_stream(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<futures::stream::BoxStream<'static, AgenticEvent>, BridgeError> {
        self.dispatch_stream(bundle, task).await
    }

    async fn call_with_context(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        // I10 + F14 : reject oversized bundles upstream of any HTTP
        // call — defends against the customer-bill blast.
        let bundle_size = bundle.estimated_size_bytes();
        if bundle_size > MAX_CONTEXT_BUNDLE_BYTES {
            return Err(BridgeError::BundleTooLarge {
                size_bytes: bundle_size,
                max_bytes: MAX_CONTEXT_BUNDLE_BYTES,
            });
        }

        // **N26 + G3 + I1 + I2** : decide the provenance path.
        let provenance_path = matches!(
            (&self.provenance, &task.audit_context),
            (Some(_), Some(_))
        );

        if provenance_path {
            self.dispatch_with_provenance(bundle, task).await
        } else {
            // Sprint 13 fallback.
            self.provenance_skipped.fetch_add(1, Ordering::Relaxed);
            if self.provenance.is_none() {
                tracing::debug!(
                    "ClaudeBridge dispatch without provenance (no collector wired)"
                );
            } else {
                tracing::debug!(
                    "ClaudeBridge dispatch without provenance (no audit_context)"
                );
            }
            self.dispatch_without_provenance(bundle, task).await
        }
    }
}

impl ClaudeBridge {
    /// **N26** : Sprint 13 dispatch path — no provenance.
    /// Kept verbatim for backward compatibility.
    async fn dispatch_without_provenance(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        let outcome = self.dispatch_two_turn(bundle, &task).await?;
        Ok(self.build_agentic_response(outcome))
    }

    /// **N26 + G3 + G10 + I1 + I3 + I4** : provenance-enabled
    /// dispatch path.  Wraps the two-turn HTTP call in
    /// `call_with_provenance` so a single `LlmProvenanceRecord`
    /// is emitted per logical call.
    async fn dispatch_with_provenance(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticResponse, BridgeError> {
        let ctx = task
            .audit_context
            .as_ref()
            .expect("dispatch_with_provenance requires audit_context");
        let collector = self
            .provenance
            .as_ref()
            .expect("dispatch_with_provenance requires collector");

        // Pre-call hashing (G4 + I7).
        let bundle_json_bytes = serde_json::to_vec(bundle).map_err(|e| {
            BridgeError::Backend {
                message: format!("bundle serialise (pre-hash) : {e}"),
            }
        })?;
        let inputs_json_str =
            std::str::from_utf8(&bundle_json_bytes).unwrap_or("<bundle-non-utf8>");
        let system_prompt = build_system_prompt(&task);
        let final_prompt = format!("{system_prompt}\n\n{}", task.user_message);
        let hashes = prompt_hashes_for(
            inputs_json_str,
            &system_prompt,
            &task.user_message,
            &final_prompt,
        );

        let pre = PreCallProvenance {
            trace_id: ctx.trace_id,
            swarm_request_id: ctx.swarm_request_id,
            op_id: ctx.op_id.clone(),
            tenant_id: ctx.tenant_id,
            agent_id: ctx.agent_id.clone(),
            agent_version: ctx.agent_version.clone(),
            model_identity: ModelIdentity::hosted(
                "anthropic",
                &self.model,
                &self.anthropic_version,
            ),
            prompt_template_id: ctx.prompt_template_id.clone(),
            prompt_template_version: ctx.prompt_template_version.clone(),
            prompt_inputs_hash: hashes.inputs_hash,
            system_prompt_hash: hashes.system_hash,
            user_prompt_hash: hashes.user_hash,
            final_prompt_hash: hashes.final_hash,
            sampling: SamplingParams {
                temperature: task.temperature,
                top_p: None,
                max_tokens: task.max_tokens,
                seed: None,
                stop_sequences: Vec::new(),
            },
            pii_strategy_applied: ctx.pii_strategy_applied,
            redaction_token_set: ctx.redaction_token_set.clone(),
            provider_region: None,
            publisher_id: ctx.publisher_id,
            publisher_kyc_tier: ctx.publisher_kyc_tier,
            gold_case_id: ctx.gold_case_id,
        };

        // **G10 + I4 + I5** : pre-call verification (model
        // registry check).  Failure OR timeout → fail closed
        // BEFORE any HTTP call to Anthropic.  The
        // [`PROVENANCE_PRE_CALL_TIMEOUT`] bound (200 ms) keeps
        // the customer-visible HTTP latency bounded even when
        // the registry is slow.
        let pre_call_fut = collector.pre_call(&pre, None);
        match tokio::time::timeout(PROVENANCE_PRE_CALL_TIMEOUT, pre_call_fut).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(BridgeError::ProvenanceVerifyFailed {
                    reason: e.to_string(),
                });
            }
            Err(_elapsed) => {
                return Err(BridgeError::ProvenanceVerifyFailed {
                    reason: format!(
                        "pre_call timed out after {} ms",
                        PROVENANCE_PRE_CALL_TIMEOUT.as_millis()
                    ),
                });
            }
        }

        // Two-turn dispatch.
        let outcome = self.dispatch_two_turn(bundle, &task).await?;
        let cost_microcents = compute_cost_microcents(
            &self.model,
            outcome.usage.input_tokens,
            outcome.usage.output_tokens,
        );
        let output_hash = hash_str(&outcome.text);

        // **G6 + I17** : record the turn-1 tool_use block as a
        // ToolCallRecord so the auditor can see the agent asked
        // for the bundle.
        let args_hash =
            hash_bytes(outcome.tool_use_block.input.to_string().as_bytes());
        let bundle_hash = hash_bytes(&outcome.bundle_json_bytes);
        let tool_call = ToolCallRecord {
            ordinal: 0,
            tool_name: outcome.tool_use_block.name.clone(),
            arguments_hash: args_hash,
            result_hash: Some(bundle_hash),
            result_kind: ToolCallResultKind::Ok,
        };

        let post = PostCallProvenance {
            token_usage: efe_nano_sdk::TokenUsage {
                client_estimated_input: 0,
                client_estimated_output: 0,
                provider_reported_input: Some(outcome.usage.input_tokens),
                provider_reported_output: Some(outcome.usage.output_tokens),
                is_estimated: false,
            },
            cost_accounting: CostAccounting {
                cost_cents: compute_cost_cents(
                    &self.model,
                    outcome.usage.input_tokens,
                    outcome.usage.output_tokens,
                ),
                prices_table_version: PRICES_TABLE_VERSION,
                provider_quoted_cost_cents: None,
                overflow_capped: false,
            },
            provider_request_id: outcome.provider_request_id.clone(),
            provider_response_time_ms: outcome.total_latency_ms,
            provider_finish_reason: map_finish_reason(outcome.stop_reason),
            provider_metadata: std::collections::HashMap::new(),
            output_hash: Some(output_hash),
            completeness: derive_completeness(
                outcome.stop_reason,
                outcome.usage.output_tokens,
                task.max_tokens,
            ),
            safety_flags: Vec::new(),
            tool_calls: vec![tool_call],
            model_hash_verified: false, // hosted Anthropic
        };

        // **G3 + I3 + I6 + I8 + ADR-5 + ADR-6** : non-blocking
        // post-call emission with a 500 ms ceiling.  Errors and
        // timeouts are LOGGED + metric-ticked but do NOT
        // propagate as BridgeError ; the customer-visible
        // response is the LLM answer regardless.
        let post_call_fut = collector.post_call(pre, post);
        match tokio::time::timeout(PROVENANCE_POST_CALL_TIMEOUT, post_call_fut).await {
            Ok(Ok(_record)) => {
                tracing::debug!("N26 provenance record emitted");
            }
            Ok(Err(e)) => {
                self.provenance_emit_failed
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    error = %e,
                    trace_id = %ctx.trace_id,
                    "N26 collector post_call failed ; response still returned to caller"
                );
            }
            Err(_elapsed) => {
                self.provenance_post_timeout
                    .fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    timeout_ms = PROVENANCE_POST_CALL_TIMEOUT.as_millis() as u64,
                    trace_id = %ctx.trace_id,
                    "N26 collector post_call timed out ; response still returned to caller"
                );
            }
        }

        // **G6** : build the AgenticResponse with the exact
        // microcents we recorded (consistent with provenance).
        Ok(AgenticResponse {
            text: outcome.text,
            stop_reason: outcome.stop_reason,
            usage: TokenUsage {
                input_tokens: outcome.usage.input_tokens,
                output_tokens: outcome.usage.output_tokens,
            },
            model_identity: ModelIdentity::hosted(
                "anthropic",
                &self.model,
                &self.anthropic_version,
            ),
            cost_microcents,
        })
    }

    /// **N26** : the two-turn Anthropic protocol, factored out
    /// of `call_with_context` so both the provenance-on and
    /// provenance-off paths share it.
    async fn dispatch_two_turn(
        &self,
        bundle: &ContextBundle,
        task: &AgenticTask,
    ) -> Result<TwoTurnResult, BridgeError> {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));

        // Turn 1 : tell Claude there's a swarm_context tool.
        let system_prompt = build_system_prompt(task);
        let req1 = serde_json::json!({
            "model": &self.model,
            "max_tokens": task.max_tokens,
            "temperature": task.temperature,
            "system": system_prompt,
            "tools": [ToolDefinition::swarm_context()],
            "messages": [
                {"role": "user", "content": &task.user_message},
            ],
        });
        let t1_start = Instant::now();
        let resp1 = self.send(&url, &req1).await?;
        let parsed1 = self.parse_response(resp1).await?;
        let t1_ms = t1_start.elapsed().as_millis() as u32;

        // If Claude doesn't tool-use first, treat as protocol violation.
        let (tool_block, assistant_blocks) = extract_tool_use(&parsed1)?;
        if !tool_block.is_swarm_context() {
            return Err(BridgeError::ProtocolViolation {
                message: format!(
                    "Claude called unexpected tool {:?} (expected {})",
                    tool_block.name, SWARM_CONTEXT_TOOL_NAME
                ),
            });
        }

        // Turn 2 : provide the bundle as tool_result.
        let bundle_json = serde_json::to_value(bundle).map_err(|e| {
            BridgeError::Backend {
                message: format!("bundle serialise : {e}"),
            }
        })?;
        let bundle_json_bytes = serde_json::to_vec(bundle).unwrap_or_default();
        let req2 = serde_json::json!({
            "model": &self.model,
            "max_tokens": task.max_tokens,
            "temperature": task.temperature,
            "system": system_prompt,
            "tools": [ToolDefinition::swarm_context()],
            "messages": [
                {"role": "user", "content": &task.user_message},
                {"role": "assistant", "content": assistant_blocks},
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_block.id,
                        "content": bundle_json.to_string(),
                    }]
                },
            ],
        });
        let t2_start = Instant::now();
        let resp2 = self.send(&url, &req2).await?;
        let parsed2 = self.parse_response(resp2).await?;
        let t2_ms = t2_start.elapsed().as_millis() as u32;

        let text = extract_text(&parsed2)?;
        let stop_reason = parsed2
            .stop_reason
            .as_deref()
            .map(StopReason::from_anthropic)
            .unwrap_or(StopReason::Other);
        // **G6** : sum usage across both turns.
        let usage1 = parsed1.usage.unwrap_or_default();
        let usage2 = parsed2.usage.unwrap_or_default();
        let total_usage = AnthropicUsage {
            input_tokens: usage1.input_tokens.saturating_add(usage2.input_tokens),
            output_tokens: usage1.output_tokens.saturating_add(usage2.output_tokens),
        };

        Ok(TwoTurnResult {
            text,
            usage: total_usage,
            stop_reason,
            provider_request_id: parsed2.id.clone(),
            total_latency_ms: t1_ms.saturating_add(t2_ms),
            tool_use_block: tool_block,
            bundle_json_bytes,
        })
    }

    /// **N26** : pack a `TwoTurnResult` into an `AgenticResponse`
    /// (no provenance — Sprint 13 path).
    fn build_agentic_response(&self, outcome: TwoTurnResult) -> AgenticResponse {
        let cost = estimate_cost_microcents(&self.model, &outcome.usage);
        AgenticResponse {
            text: outcome.text,
            stop_reason: outcome.stop_reason,
            usage: TokenUsage {
                input_tokens: outcome.usage.input_tokens,
                output_tokens: outcome.usage.output_tokens,
            },
            model_identity: ModelIdentity::hosted(
                "anthropic",
                &self.model,
                &self.anthropic_version,
            ),
            cost_microcents: cost,
        }
    }
}

impl ClaudeBridge {
    async fn send(
        &self,
        url: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response, BridgeError> {
        let resp = self
            .client
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.anthropic_version)
            .header("content-type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| BridgeError::Backend {
                message: format!("request : {e}"),
            })?;
        let status = resp.status();
        if status == 429 {
            return Err(BridgeError::RateLimited);
        }
        if status == 401 || status == 403 {
            return Err(BridgeError::Authentication);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(BridgeError::Backend {
                message: format!("status {status} : {body}"),
            });
        }
        Ok(resp)
    }

    async fn parse_response(
        &self,
        resp: reqwest::Response,
    ) -> Result<ClaudeMessagesResponse, BridgeError> {
        resp.json::<ClaudeMessagesResponse>()
            .await
            .map_err(|e| BridgeError::ProtocolViolation {
                message: format!("parse : {e}"),
            })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Anthropic Messages API response shape
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
struct ClaudeMessagesResponse {
    /// **N26 + I11** : Anthropic response message id (`msg_...`).
    /// Captured for the `provider_request_id` field of the
    /// emitted provenance record.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    content: Vec<ClaudeContentBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClaudeContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: serde_json::Value },
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

fn extract_tool_use(
    resp: &ClaudeMessagesResponse,
) -> Result<(ToolUseBlock, Vec<serde_json::Value>), BridgeError> {
    let mut assistant_blocks: Vec<serde_json::Value> = Vec::new();
    let mut tool_block: Option<ToolUseBlock> = None;
    for c in &resp.content {
        assistant_blocks.push(serde_json::to_value(c).map_err(|e| {
            BridgeError::Backend {
                message: format!("serialise block : {e}"),
            }
        })?);
        if let ClaudeContentBlock::ToolUse { id, name, input } = c {
            tool_block = Some(ToolUseBlock {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            });
        }
    }
    let tool = tool_block.ok_or_else(|| BridgeError::ProtocolViolation {
        message: "no tool_use block in first turn".into(),
    })?;
    Ok((tool, assistant_blocks))
}

fn extract_text(resp: &ClaudeMessagesResponse) -> Result<String, BridgeError> {
    for c in &resp.content {
        if let ClaudeContentBlock::Text { text } = c {
            return Ok(text.clone());
        }
    }
    Err(BridgeError::ProtocolViolation {
        message: "no text block in second turn".into(),
    })
}

fn build_system_prompt(task: &AgenticTask) -> String {
    let base = format!(
        "You have a tool `{SWARM_CONTEXT_TOOL_NAME}` that returns structured \
         pre-analysed context. Call it first ; then answer using the structured \
         data it provides."
    );
    if task.system_prompt.is_empty() {
        base
    } else {
        format!("{base}\n\n{}", task.system_prompt)
    }
}

/// **N21 → N26** : rough cost estimator in microcents.  Sprint
/// 13 ships a hard-coded table ; Sprint 18 N26 routes through
/// the new `pricing` module so cost numbers match the
/// `LlmProvenanceRecord` exactly.
fn estimate_cost_microcents(model: &str, usage: &AnthropicUsage) -> i64 {
    compute_cost_microcents(model, usage.input_tokens, usage.output_tokens)
}

/// **N26 + I14** : map an Anthropic `stop_reason` to the N17
/// `FinishReason` taxonomy.
fn map_finish_reason(stop: StopReason) -> FinishReason {
    match stop {
        StopReason::EndTurn => FinishReason::Stop,
        StopReason::MaxTokens => FinishReason::Length,
        StopReason::ToolUse => FinishReason::ToolCalls,
        StopReason::StopSequence => FinishReason::Stop,
        StopReason::Other => FinishReason::Unknown,
    }
}

/// **N26** : derive the N17 `Completeness` from the bridge
/// stop reason.  Sprint 18 maps `MaxTokens` to `Partial` and
/// carries the actual received/expected token counts so the
/// audit row faithfully captures the truncation delta.
fn derive_completeness(
    stop: StopReason,
    received_output_tokens: u32,
    expected_max_tokens: u32,
) -> Completeness {
    match stop {
        StopReason::MaxTokens => Completeness::Partial {
            received_tokens: received_output_tokens,
            expected_tokens: expected_max_tokens,
        },
        _ => Completeness::Complete,
    }
}

// ═════════════════════════════════════════════════════════════════════
//  Sprint 19 — N27 streaming dispatch
// ═════════════════════════════════════════════════════════════════════

use crate::claude_stream::{parse_sse_stream, StreamState};
use crate::retry::{AttemptOutcome, NetworkErrorClass, RetryLoop, RetryOutcomeProbe};
use crate::task::AgenticEvent;
use efe_nano_sdk::{AbortReason, RetryOutcome};
use futures::stream::{BoxStream, StreamExt};
use std::sync::atomic::AtomicBool;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// **N27** : the streaming output channel — produces
/// [`AgenticEvent`] forwarded to the gateway SSE encoder.
pub type AgenticEventStream = BoxStream<'static, AgenticEvent>;

impl ClaudeBridge {
    /// **Sprint 19 — N27** : open a streaming dispatch.
    ///
    /// Returns a `BoxStream<AgenticEvent>` that emits :
    /// * `MessageStart`, `TextDelta`, `ToolUseStart/Delta/Complete`,
    ///   `TurnUsage` events (internal + downstream-visible mix)
    /// * exactly one terminal `Done` (or `Error`) event
    ///
    /// Behaviour highlights :
    /// * Two-turn `tool_use` protocol : turn 1 streams ; on
    ///   `stop_reason=tool_use` the bridge executes the tool
    ///   in-process and streams turn 2 ; both turns counted in
    ///   the final usage.
    /// * Retry : turn 1 is wrapped in a [`RetryLoop`] driven by
    ///   the configured policy.  Turn 2 has its OWN budget.
    /// * Cancellation : the returned stream is wrapped with a
    ///   take-until on the internal [`CancellationToken`] — when
    ///   the downstream drops the receiver, the upstream HTTP/2
    ///   stream is RST_STREAM-cancelled.
    /// * Provenance : when a collector + audit_context are wired,
    ///   exactly one `LlmProvenanceRecord` is emitted with
    ///   `streaming_mode = true`, `retry_count`, `retry_outcome`,
    ///   `time_to_first_token_ms` populated.  Aborts produce a
    ///   record with `Completeness::Aborted`.
    pub async fn dispatch_stream(
        &self,
        bundle: &ContextBundle,
        task: AgenticTask,
    ) -> Result<AgenticEventStream, BridgeError> {
        // I10 + F14 : reject oversized bundles upstream of any HTTP call.
        let bundle_size = bundle.estimated_size_bytes();
        if bundle_size > MAX_CONTEXT_BUNDLE_BYTES {
            return Err(BridgeError::BundleTooLarge {
                size_bytes: bundle_size,
                max_bytes: MAX_CONTEXT_BUNDLE_BYTES,
            });
        }

        // **G3 + I1 + I2** : decide the provenance path.
        let prov_wired = self.provenance.is_some();
        let audit_present = task.audit_context.is_some();
        let pre_for_record: Option<PreCallProvenance> = if prov_wired && audit_present {
            Some(self.pre_call_for_stream(bundle, &task).await?)
        } else {
            self.provenance_skipped.fetch_add(1, Ordering::Relaxed);
            None
        };

        // Cancellation + first-byte coordination.
        let cancel = CancellationToken::new();
        let downstream_first =
            std::sync::Arc::new(AtomicBool::new(false));

        // Bounded internal channel (ADR-N27-5).
        let (tx, rx) = mpsc::channel::<AgenticEvent>(self.stream_config.channel_cap);

        // Snapshot the pieces the spawned task needs.
        let this = self.clone_for_task();
        let cancel_task = cancel.clone();
        let first_task = downstream_first.clone();
        let bundle_owned = bundle.clone();
        let task_owned = task.clone();

        // Spawn the dispatcher.
        tokio::spawn(async move {
            let result = this
                .clone()
                .dispatch_stream_inner(
                    bundle_owned.clone(),
                    task_owned.clone(),
                    tx.clone(),
                    cancel_task.clone(),
                    first_task.clone(),
                )
                .await;

            // **Audit-pass-3 fix Bug #9** : the abort counter
            // must be incremented BEFORE the receiver's
            // end-of-stream signal so callers that consume the
            // stream-to-end and then inspect the counter (e.g.
            // chaos tests) observe a consistent state.  Order is :
            //   1. classify the outcome's abort status
            //   2. increment counter
            //   3. drop tx (rx sees end-of-stream)
            //   4. write provenance (up to 500 ms, no longer
            //      blocks the consumer thanks to step 3)
            //
            // **Bug #1+#2** : abort counter increments whenever
            // any turn's snapshot carries `abort_reason: Some(...)`.
            // That covers ReadTimeout / TotalTimeout /
            // ClientDisconnect / UpstreamError surfaces via the
            // `Failed` variant.  The explicit `Aborted` variant
            // is reserved for future explicit-abort paths (e.g.
            // pre-call provenance rejection — not used in N27 v1).
            let aborted = match &result {
                StreamDispatchOutcome::Aborted { .. } => true,
                StreamDispatchOutcome::Failed {
                    turn1_state,
                    turn2_state,
                    ..
                } => {
                    turn1_state.as_ref().and_then(|s| s.abort_reason.as_ref()).is_some()
                        || turn2_state
                            .as_ref()
                            .and_then(|s| s.abort_reason.as_ref())
                            .is_some()
                }
                StreamDispatchOutcome::Success { .. } => false,
            };
            if aborted {
                this.stream_aborts.fetch_add(1, Ordering::Relaxed);
            }

            // **Audit-pass-3 fix Bug #8** : the dispatcher held its
            // own tx clone, but the spawn-closure-level `tx` is
            // also alive until the closure ends.  Drop it
            // explicitly here so the downstream `ReceiverStream`
            // sees end-of-stream IMMEDIATELY after the `Done`
            // event (rather than waiting up to 500 ms for the
            // post-call provenance write to complete).
            drop(tx);

            // Always emit provenance (G14) when wired.
            if let (Some(c), Some(pre)) = (&this.provenance, pre_for_record) {
                let streaming = build_streaming_post_call(
                    &this,
                    &task_owned,
                    &result,
                );
                let fut = c.post_call_streaming(pre, streaming);
                match tokio::time::timeout(PROVENANCE_POST_CALL_TIMEOUT, fut).await {
                    Ok(Ok(_)) => {
                        tracing::debug!("N27 streaming provenance record emitted");
                    }
                    Ok(Err(e)) => {
                        this.provenance_emit_failed.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            error = %e,
                            "N27 streaming collector post_call failed"
                        );
                    }
                    Err(_) => {
                        this.provenance_post_timeout.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!("N27 streaming collector post_call timed out");
                    }
                }
            }
        });

        // Build the downstream stream : wrap rx + cancel.
        let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        // We do NOT take_until cancel — the inner dispatcher
        // listens to it explicitly.  Returning the bare receiver
        // lets the gateway see the `Error` or `Done` terminator
        // even after a cancel.
        Ok(Box::pin(rx_stream))
    }

    /// **N27** : pre-call provenance for the streaming path.
    /// Returns the [`PreCallProvenance`] for the dispatcher to
    /// hand back to `post_call_streaming` at the end.
    async fn pre_call_for_stream(
        &self,
        bundle: &ContextBundle,
        task: &AgenticTask,
    ) -> Result<PreCallProvenance, BridgeError> {
        let ctx = task.audit_context.as_ref().expect("audit_context required");
        let bundle_json_bytes = serde_json::to_vec(bundle).map_err(|e| {
            BridgeError::Backend {
                message: format!("bundle serialise : {e}"),
            }
        })?;
        let inputs_json_str =
            std::str::from_utf8(&bundle_json_bytes).unwrap_or("<bundle-non-utf8>");
        let system_prompt = build_system_prompt(task);
        let final_prompt = format!("{system_prompt}\n\n{}", task.user_message);
        let hashes = prompt_hashes_for(
            inputs_json_str,
            &system_prompt,
            &task.user_message,
            &final_prompt,
        );

        let pre = PreCallProvenance {
            trace_id: ctx.trace_id,
            swarm_request_id: ctx.swarm_request_id,
            op_id: ctx.op_id.clone(),
            tenant_id: ctx.tenant_id,
            agent_id: ctx.agent_id.clone(),
            agent_version: ctx.agent_version.clone(),
            model_identity: ModelIdentity::hosted(
                "anthropic",
                &self.model,
                &self.anthropic_version,
            ),
            prompt_template_id: ctx.prompt_template_id.clone(),
            prompt_template_version: ctx.prompt_template_version.clone(),
            prompt_inputs_hash: hashes.inputs_hash,
            system_prompt_hash: hashes.system_hash,
            user_prompt_hash: hashes.user_hash,
            final_prompt_hash: hashes.final_hash,
            sampling: SamplingParams {
                temperature: task.temperature,
                top_p: None,
                max_tokens: task.max_tokens,
                seed: None,
                stop_sequences: Vec::new(),
            },
            pii_strategy_applied: ctx.pii_strategy_applied,
            redaction_token_set: ctx.redaction_token_set.clone(),
            provider_region: None,
            publisher_id: ctx.publisher_id,
            publisher_kyc_tier: ctx.publisher_kyc_tier,
            gold_case_id: ctx.gold_case_id,
        };

        let collector = self.provenance.as_ref().expect("collector required");
        // Pre-call verify : model registry lookup + (optional) hash.
        // For hosted Anthropic models the registry entry has
        // `dev_skip_hash_check = true` so we pass `None`.
        let fut = collector.pre_call(&pre, None);
        match tokio::time::timeout(PROVENANCE_PRE_CALL_TIMEOUT, fut).await {
            Ok(Ok(())) => Ok(pre),
            Ok(Err(e)) => Err(BridgeError::ProvenanceVerifyFailed {
                reason: e.to_string(),
            }),
            Err(_) => Err(BridgeError::ProvenanceVerifyFailed {
                reason: format!(
                    "pre_call timed out after {} ms",
                    PROVENANCE_PRE_CALL_TIMEOUT.as_millis()
                ),
            }),
        }
    }

    /// **N27** : the inner dispatcher.  Drives turn-1 retry,
    /// optional tool execution + turn-2 retry, emits events to
    /// `tx`, returns the final outcome for provenance.
    async fn dispatch_stream_inner(
        self: std::sync::Arc<Self>,
        bundle: ContextBundle,
        task: AgenticTask,
        tx: mpsc::Sender<AgenticEvent>,
        cancel: CancellationToken,
        downstream_first: std::sync::Arc<AtomicBool>,
    ) -> StreamDispatchOutcome {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let system_prompt = build_system_prompt(&task);

        // ─── Turn 1 ──────────────────────────────────────────────
        let req1 = serde_json::json!({
            "model": &self.model,
            "max_tokens": task.max_tokens,
            "temperature": task.temperature,
            "system": system_prompt,
            "tools": [crate::tool_use::ToolDefinition::swarm_context()],
            "messages": [{"role": "user", "content": &task.user_message}],
            "stream": true,
        });

        let retry_loop = RetryLoop::with_first_byte_flag(
            self.retry_policy.clone(),
            downstream_first.clone(),
        );
        let cancel_t1 = cancel.clone();
        let tx_t1 = tx.clone();
        let this_t1 = self.clone();
        let url_t1 = url.clone();
        let stream_cfg = self.stream_config;
        let retry_attempt_counter = self.stream_retry_attempts.clone();

        let (turn1_r, turn1_partial, turn1_summary) = retry_loop
            .execute(|attempt| {
                if attempt > 0 {
                    retry_attempt_counter.fetch_add(1, Ordering::Relaxed);
                }
                let url = url_t1.clone();
                let tx = tx_t1.clone();
                let cancel = cancel_t1.clone();
                let first = downstream_first.clone();
                let body = req1.clone();
                let this = this_t1.clone();
                async move {
                    this.stream_one_turn(1, &url, &body, tx, cancel, first, stream_cfg)
                        .await
                }
            })
            .await;

        let turn1_state = match turn1_r {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(AgenticEvent::Error(e.clone())).await;
                // **Audit-fix Bug #2** : surface the partial state
                // from the LAST failed attempt so build_streaming_post_call
                // can stamp Completeness::Aborted with the right reason +
                // partial token counts.
                let ttft_from_partial =
                    turn1_partial.as_ref().and_then(|p| p.ttft_ms);
                return StreamDispatchOutcome::Failed {
                    error: e,
                    turn1_summary,
                    turn2_summary: None,
                    turn1_state: turn1_partial,
                    turn2_state: None,
                    ttft_ms: ttft_from_partial,
                };
            }
        };

        // If no tool-use, emit Done now.
        if !matches!(turn1_state.final_stop_reason, Some(StopReason::ToolUse)) {
            let usage = turn1_state.token_usage();
            let cost = compute_cost_microcents(
                &self.model,
                usage.input_tokens,
                usage.output_tokens,
            );
            let done = AgenticEvent::Done {
                stop_reason: turn1_state.final_stop_reason.unwrap_or(StopReason::Other),
                total_usage: usage,
                model_identity: ModelIdentity::hosted(
                    "anthropic",
                    &self.model,
                    &self.anthropic_version,
                ),
                cost_microcents: cost,
                time_to_first_token_ms: turn1_state.ttft_ms(),
                provider_response_time_ms: turn1_state.elapsed_ms(),
                retry_count: turn1_summary.retry_count,
                retry_outcome: turn1_summary.outcome,
            };
            let _ = tx.send(done).await;
            return StreamDispatchOutcome::Success {
                turn1_summary,
                turn2_summary: None,
                turn1_state: turn1_state.clone_for_outcome(),
                turn2_state: None,
                ttft_ms: turn1_state.ttft_ms(),
            };
        }

        // ─── Turn 2 ──────────────────────────────────────────────
        let tool_uses = turn1_state.tool_uses();
        let tool = match tool_uses.first() {
            Some(t) => t.clone(),
            None => {
                let err = BridgeError::ProtocolViolation {
                    message: "stop_reason=tool_use but no tool_use block".into(),
                };
                let _ = tx.send(AgenticEvent::Error(err.clone())).await;
                return StreamDispatchOutcome::Failed {
                    error: err,
                    turn1_summary,
                    turn2_summary: None,
                    turn1_state: Some(turn1_state.clone_for_outcome()),
                    turn2_state: None,
                    ttft_ms: turn1_state.ttft_ms(),
                };
            }
        };
        if tool.1 != crate::tool_use::SWARM_CONTEXT_TOOL_NAME {
            let err = BridgeError::ProtocolViolation {
                message: format!(
                    "unexpected tool : {} (expected {})",
                    tool.1,
                    crate::tool_use::SWARM_CONTEXT_TOOL_NAME
                ),
            };
            let _ = tx.send(AgenticEvent::Error(err.clone())).await;
            return StreamDispatchOutcome::Failed {
                error: err,
                turn1_summary,
                turn2_summary: None,
                turn1_state: Some(turn1_state.clone_for_outcome()),
                turn2_state: None,
                ttft_ms: turn1_state.ttft_ms(),
            };
        }

        let bundle_json = serde_json::to_value(&bundle).unwrap_or(serde_json::json!({}));
        // Reconstruct the assistant turn content from accumulated state.
        let mut assistant_blocks: Vec<serde_json::Value> = Vec::new();
        let acc_text = turn1_state.accumulated_text();
        if !acc_text.is_empty() {
            assistant_blocks
                .push(serde_json::json!({"type": "text", "text": acc_text}));
        }
        assistant_blocks.push(serde_json::json!({
            "type": "tool_use",
            "id": &tool.0,
            "name": &tool.1,
            "input": &tool.2,
        }));

        let req2 = serde_json::json!({
            "model": &self.model,
            "max_tokens": task.max_tokens,
            "temperature": task.temperature,
            "system": system_prompt,
            "tools": [crate::tool_use::ToolDefinition::swarm_context()],
            "messages": [
                {"role": "user", "content": &task.user_message},
                {"role": "assistant", "content": assistant_blocks},
                {
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": &tool.0,
                        "content": bundle_json.to_string(),
                    }]
                },
            ],
            "stream": true,
        });

        let retry_loop2 = RetryLoop::with_first_byte_flag(
            self.retry_policy.clone(),
            downstream_first.clone(),
        );
        let cancel_t2 = cancel.clone();
        let tx_t2 = tx.clone();
        let this_t2 = self.clone();
        let url_t2 = url.clone();
        let stream_cfg2 = self.stream_config;
        let retry_attempt_counter2 = self.stream_retry_attempts.clone();

        let (turn2_r, turn2_partial, turn2_summary) = retry_loop2
            .execute(|attempt| {
                if attempt > 0 {
                    retry_attempt_counter2.fetch_add(1, Ordering::Relaxed);
                }
                let url = url_t2.clone();
                let tx = tx_t2.clone();
                let cancel = cancel_t2.clone();
                let first = downstream_first.clone();
                let body = req2.clone();
                let this = this_t2.clone();
                async move {
                    this.stream_one_turn(2, &url, &body, tx, cancel, first, stream_cfg2)
                        .await
                }
            })
            .await;

        let turn2_state = match turn2_r {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(AgenticEvent::Error(e.clone())).await;
                return StreamDispatchOutcome::Failed {
                    error: e,
                    turn1_summary,
                    turn2_summary: Some(turn2_summary),
                    turn1_state: Some(turn1_state.clone_for_outcome()),
                    turn2_state: turn2_partial,
                    ttft_ms: turn1_state.ttft_ms(),
                };
            }
        };

        // Final summary.
        let total_input = turn1_state.input_tokens.saturating_add(turn2_state.input_tokens);
        let total_output = turn1_state
            .final_output_tokens
            .saturating_add(turn2_state.final_output_tokens);
        let usage = TokenUsage {
            input_tokens: total_input,
            output_tokens: total_output,
        };
        let cost = compute_cost_microcents(&self.model, total_input, total_output);
        let provider_response_time_ms = turn1_state.elapsed_ms() + turn2_state.elapsed_ms();
        let total_retry_count =
            turn1_summary.retry_count.saturating_add(turn2_summary.retry_count);
        let retry_outcome = combine_retry_outcomes(
            turn1_summary.outcome,
            turn2_summary.outcome,
        );

        let done = AgenticEvent::Done {
            stop_reason: turn2_state.final_stop_reason.unwrap_or(StopReason::Other),
            total_usage: usage,
            model_identity: ModelIdentity::hosted(
                "anthropic",
                &self.model,
                &self.anthropic_version,
            ),
            cost_microcents: cost,
            time_to_first_token_ms: turn1_state.ttft_ms(),
            provider_response_time_ms,
            retry_count: total_retry_count,
            retry_outcome,
        };
        let _ = tx.send(done).await;

        StreamDispatchOutcome::Success {
            turn1_summary,
            turn2_summary: Some(turn2_summary),
            turn1_state: turn1_state.clone_for_outcome(),
            turn2_state: Some(turn2_state.clone_for_outcome()),
            ttft_ms: turn1_state.ttft_ms(),
        }
    }

    /// **N27** : stream a single Anthropic turn.  Returns
    /// `AttemptOutcome::Success(state)` on `message_stop`,
    /// `Failure { error, probe, partial }` otherwise.  The
    /// `partial` carries the LAST attempt's `StreamStateSnapshot`
    /// (audit-fix Bug #2 — abort_reason preservation across the
    /// retry boundary).
    #[allow(clippy::too_many_arguments)]
    async fn stream_one_turn(
        self: std::sync::Arc<Self>,
        turn: u8,
        url: &str,
        body: &serde_json::Value,
        tx: mpsc::Sender<AgenticEvent>,
        cancel: CancellationToken,
        downstream_first: std::sync::Arc<AtomicBool>,
        cfg: StreamConfig,
    ) -> AttemptOutcome<StreamState, BridgeError, StreamStateSnapshot> {
        self.call_count.fetch_add(1, Ordering::Relaxed);

        // **Audit-fix Bug #2** : a fresh empty state lives from
        // the very start of the attempt so even pre-HTTP failures
        // surface a (mostly-empty) snapshot to the dispatcher.
        // The state's `started_at` (set in `StreamState::new`) is
        // the per-attempt wall-clock anchor — no separate Instant
        // is needed at this scope.
        let mut state = StreamState::new(turn);

        // POST + check status.
        let send_fut = self
            .client
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", &self.anthropic_version)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(body)
            .send();
        let resp = tokio::select! {
            r = send_fut => r,
            _ = cancel.cancelled() => {
                state.abort_reason = Some(AbortReason::ClientDisconnect);
                return AttemptOutcome::fail_with(
                    BridgeError::Cancelled,
                    probe_cancelled(downstream_first.load(Ordering::SeqCst)),
                    state.clone_for_outcome(),
                );
            }
        };
        let response = match resp {
            Ok(r) => r,
            Err(e) => {
                let class = classify_reqwest_err(&e);
                return AttemptOutcome::fail_with(
                    BridgeError::Backend {
                        message: format!("send : {e}"),
                    },
                    RetryOutcomeProbe {
                        http_status: None,
                        anthropic_error_type: None,
                        retry_after: None,
                        network_class: class,
                        post_first_byte_downstream: downstream_first.load(Ordering::SeqCst),
                    },
                    state.clone_for_outcome(),
                );
            }
        };
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(crate::retry::parse_retry_after);
            let body_text = response.text().await.unwrap_or_default();
            let anth_type = extract_anthropic_error_type(&body_text);
            return AttemptOutcome::fail_with(
                BridgeError::HttpStatus {
                    status: status.as_u16(),
                    body: truncate(&body_text, 1024),
                },
                RetryOutcomeProbe {
                    http_status: Some(status.as_u16()),
                    anthropic_error_type: anth_type,
                    retry_after,
                    network_class: NetworkErrorClass::None,
                    post_first_byte_downstream: downstream_first.load(Ordering::SeqCst),
                },
                state.clone_for_outcome(),
            );
        }
        // Content-type check.
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if !ct.contains("text/event-stream") {
            return AttemptOutcome::fail_with(
                BridgeError::InvalidResponseType {
                    content_type: ct.clone(),
                },
                RetryOutcomeProbe {
                    http_status: Some(200),
                    anthropic_error_type: None,
                    retry_after: None,
                    network_class: NetworkErrorClass::Other,
                    post_first_byte_downstream: false,
                },
                state.clone_for_outcome(),
            );
        }

        // Stream the body.
        let body_stream = response.bytes_stream();
        let mut sse_stream = parse_sse_stream(body_stream);
        let started = std::time::Instant::now();

        loop {
            let next_fut = sse_stream.next();
            let next_ev = tokio::select! {
                ev = tokio::time::timeout(cfg.read_timeout, next_fut) => ev,
                _ = cancel.cancelled() => {
                    state.abort_reason = Some(AbortReason::ClientDisconnect);
                    return AttemptOutcome::fail_with(
                        BridgeError::Cancelled,
                        probe_cancelled(downstream_first.load(Ordering::SeqCst)),
                        state.clone_for_outcome(),
                    );
                }
                _ = tokio::time::sleep(cfg.total_timeout.saturating_sub(started.elapsed())) => {
                    state.abort_reason = Some(AbortReason::TotalTimeout {
                        ms: cfg.total_timeout.as_millis() as u32,
                    });
                    return AttemptOutcome::fail_with(
                        BridgeError::TotalTimeout,
                        probe_post_byte(downstream_first.load(Ordering::SeqCst)),
                        state.clone_for_outcome(),
                    );
                }
            };
            let ev = match next_ev {
                Ok(Some(Ok(e))) => e,
                Ok(Some(Err(e))) => {
                    // **Audit-fix Bug #4** : SSE parse error mid-stream
                    // is an upstream-truncated stream — stamp it as
                    // `UpstreamError { error_type: "sse_parse_error" }`
                    // so the record carries Completeness::Aborted.
                    state.abort_reason = Some(AbortReason::UpstreamError {
                        error_type: "sse_parse_error".to_string(),
                    });
                    return AttemptOutcome::fail_with(
                        BridgeError::SseParseError {
                            reason: e.to_string(),
                        },
                        probe_post_byte(downstream_first.load(Ordering::SeqCst)),
                        state.clone_for_outcome(),
                    );
                }
                Ok(None) => {
                    // **Audit-fix Bug #4** : body ended without
                    // message_stop — semantically an upstream
                    // truncation.
                    state.abort_reason = Some(AbortReason::UpstreamError {
                        error_type: "premature_eof".to_string(),
                    });
                    return AttemptOutcome::fail_with(
                        BridgeError::PrematureEof,
                        probe_post_byte(downstream_first.load(Ordering::SeqCst)),
                        state.clone_for_outcome(),
                    );
                }
                Err(_elapsed) => {
                    state.abort_reason = Some(AbortReason::ReadTimeout {
                        ms: cfg.read_timeout.as_millis() as u32,
                    });
                    return AttemptOutcome::fail_with(
                        BridgeError::ReadTimeout,
                        probe_post_byte(downstream_first.load(Ordering::SeqCst)),
                        state.clone_for_outcome(),
                    );
                }
            };

            // Apply.
            let downstream_event = match state.apply(ev) {
                Ok(d) => d,
                Err(e) => {
                    return AttemptOutcome::fail_with(
                        e,
                        probe_post_byte(downstream_first.load(Ordering::SeqCst)),
                        state.clone_for_outcome(),
                    );
                }
            };

            if let Some(downstream_ev) = downstream_event {
                if downstream_ev.is_downstream_visible() {
                    // Flip the first-byte flag *before* sending so
                    // a concurrent retry classification sees the
                    // updated value.
                    downstream_first.store(true, Ordering::SeqCst);
                }
                if tx.send(downstream_ev).await.is_err() {
                    // Downstream gone — treat as client disconnect.
                    state.abort_reason = Some(AbortReason::ClientDisconnect);
                    return AttemptOutcome::fail_with(
                        BridgeError::Cancelled,
                        probe_post_byte(true),
                        state.clone_for_outcome(),
                    );
                }
            }

            if state.finalized {
                break;
            }
        }

        AttemptOutcome::Success(state)
    }

    /// **N27 audit-fix** : produce a fully-shared `Arc<Self>` for
    /// the spawned dispatch task.  ALL atomics are shared via
    /// `Arc::clone` (not value-snapshot) so the bridge handle
    /// sees increments performed inside the spawned task — see
    /// audit pass 2 Bug #1.
    fn clone_for_task(&self) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            client: self.client.clone(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            anthropic_version: self.anthropic_version.clone(),
            call_count: self.call_count.clone(),
            provenance: self.provenance.clone(),
            provenance_skipped: self.provenance_skipped.clone(),
            provenance_emit_failed: self.provenance_emit_failed.clone(),
            provenance_post_timeout: self.provenance_post_timeout.clone(),
            retry_policy: self.retry_policy.clone(),
            stream_config: self.stream_config,
            stream_retry_attempts: self.stream_retry_attempts.clone(),
            stream_aborts: self.stream_aborts.clone(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Streaming dispatch helpers
// ─────────────────────────────────────────────────────────────────────

/// **N27** : compact summary returned from `dispatch_stream_inner`
/// to drive provenance emission.
enum StreamDispatchOutcome {
    Success {
        turn1_summary: crate::retry::RetrySummary,
        turn2_summary: Option<crate::retry::RetrySummary>,
        turn1_state: StreamStateSnapshot,
        turn2_state: Option<StreamStateSnapshot>,
        ttft_ms: Option<u32>,
    },
    Failed {
        #[allow(dead_code)]
        error: BridgeError,
        turn1_summary: crate::retry::RetrySummary,
        turn2_summary: Option<crate::retry::RetrySummary>,
        turn1_state: Option<StreamStateSnapshot>,
        turn2_state: Option<StreamStateSnapshot>,
        ttft_ms: Option<u32>,
    },
    /// Reserved for explicit abort-without-error code paths (e.g.
    /// pre-call provenance fail).
    #[allow(dead_code)]
    Aborted {
        reason: AbortReason,
        ttft_ms: Option<u32>,
    },
}

/// **N27** : trimmed snapshot of [`StreamState`] (no `Instant`
/// fields) so we can hold it across the post-dispatch provenance
/// emission without lifetime headaches.
///
/// **Audit-fix Bug #2** : added `ttft_ms` so the dispatcher's
/// failure path can recover the time-to-first-token from the
/// last failed attempt's partial state (was previously lost
/// because the function returning the snapshot dropped the
/// `Instant` fields).
#[derive(Clone)]
struct StreamStateSnapshot {
    text: String,
    input_tokens: u32,
    output_tokens: u32,
    stop_reason: StopReason,
    latency_ms: u32,
    /// **Audit-fix Bug #2** : preserved across the retry boundary.
    ttft_ms: Option<u32>,
    provider_request_id: Option<String>,
    abort_reason: Option<AbortReason>,
    #[allow(dead_code)]
    tool_uses: Vec<crate::claude_stream::finalize::ToolUseFinalized>,
}

impl StreamState {
    fn clone_for_outcome(&self) -> StreamStateSnapshot {
        StreamStateSnapshot {
            text: self.accumulated_text(),
            input_tokens: self.input_tokens,
            output_tokens: self.final_output_tokens,
            stop_reason: self.final_stop_reason.unwrap_or(StopReason::Other),
            latency_ms: self.elapsed_ms(),
            ttft_ms: self.ttft_ms(),
            provider_request_id: self.message_id.clone(),
            abort_reason: self.abort_reason.clone(),
            tool_uses: self
                .tool_uses()
                .into_iter()
                .map(|(id, name, input)| {
                    crate::claude_stream::finalize::ToolUseFinalized {
                        id,
                        name,
                        input,
                    }
                })
                .collect(),
        }
    }
}

/// **N27** : assemble the streaming post-call provenance from
/// the dispatch outcome.
fn build_streaming_post_call(
    bridge: &ClaudeBridge,
    task: &AgenticTask,
    outcome: &StreamDispatchOutcome,
) -> efe_llm_provenance::StreamingPostCallProvenance {
    use efe_llm_provenance::StreamingPostCallProvenance;

    let (
        text,
        usage_in,
        usage_out,
        stop,
        latency_ms,
        prid,
        abort,
        turn1_summary,
        turn2_summary,
        ttft_ms,
        succeeded,
    ) = match outcome {
        StreamDispatchOutcome::Success {
            turn1_summary,
            turn2_summary,
            turn1_state,
            turn2_state,
            ttft_ms,
        } => {
            let (t2_text, t2_in, t2_out, t2_lat, t2_prid, t2_stop) = match turn2_state {
                Some(s) => (
                    s.text.clone(),
                    s.input_tokens,
                    s.output_tokens,
                    s.latency_ms,
                    s.provider_request_id.clone(),
                    s.stop_reason,
                ),
                None => (
                    String::new(),
                    0,
                    0,
                    0,
                    None,
                    turn1_state.stop_reason,
                ),
            };
            (
                if t2_text.is_empty() {
                    turn1_state.text.clone()
                } else {
                    t2_text
                },
                turn1_state.input_tokens.saturating_add(t2_in),
                turn1_state.output_tokens.saturating_add(t2_out),
                if turn2_state.is_some() {
                    t2_stop
                } else {
                    turn1_state.stop_reason
                },
                turn1_state.latency_ms.saturating_add(t2_lat),
                t2_prid.or_else(|| turn1_state.provider_request_id.clone()),
                None,
                *turn1_summary,
                *turn2_summary,
                *ttft_ms,
                true,
            )
        }
        StreamDispatchOutcome::Failed {
            error: _,
            turn1_summary,
            turn2_summary,
            turn1_state,
            turn2_state,
            ttft_ms,
        } => {
            let s1 = turn1_state.as_ref();
            let s2 = turn2_state.as_ref();
            let text = s2
                .map(|s| s.text.clone())
                .or_else(|| s1.map(|s| s.text.clone()))
                .unwrap_or_default();
            let in_tok = s1.map(|s| s.input_tokens).unwrap_or(0)
                + s2.map(|s| s.input_tokens).unwrap_or(0);
            let out_tok = s1.map(|s| s.output_tokens).unwrap_or(0)
                + s2.map(|s| s.output_tokens).unwrap_or(0);
            let stop = s2
                .map(|s| s.stop_reason)
                .or_else(|| s1.map(|s| s.stop_reason))
                .unwrap_or(StopReason::Other);
            let lat = s1.map(|s| s.latency_ms).unwrap_or(0)
                + s2.map(|s| s.latency_ms).unwrap_or(0);
            let prid = s2
                .and_then(|s| s.provider_request_id.clone())
                .or_else(|| s1.and_then(|s| s.provider_request_id.clone()));
            let abort = s2
                .and_then(|s| s.abort_reason.clone())
                .or_else(|| s1.and_then(|s| s.abort_reason.clone()));
            (
                text,
                in_tok,
                out_tok,
                stop,
                lat,
                prid,
                abort,
                *turn1_summary,
                *turn2_summary,
                *ttft_ms,
                false,
            )
        }
        StreamDispatchOutcome::Aborted { reason, ttft_ms } => (
            String::new(),
            0,
            0,
            StopReason::Other,
            0,
            None,
            Some(reason.clone()),
            crate::retry::RetrySummary::no_retry(),
            None,
            *ttft_ms,
            false,
        ),
    };

    // **Audit-fix #6** : the record uses `cost_cents` (decimal
    // cents stored in `CostAccounting`).  The microcent value is
    // surfaced separately in the `Done` event for downstream
    // UIs ; it's computed inside `dispatch_stream_inner` where
    // the Done event is built.  Here we ONLY compute `cost_cents`
    // for the record.
    let output_hash = hash_str(&text);
    let completeness = match (succeeded, abort.clone()) {
        (true, _) => derive_completeness(stop, usage_out, task.max_tokens),
        (false, Some(reason)) => Completeness::Aborted {
            received_tokens: usage_out,
            reason,
        },
        (false, None) => Completeness::Errored {
            error_kind: "stream_failed".into(),
        },
    };

    let total_retry_count = turn1_summary
        .retry_count
        .saturating_add(turn2_summary.map(|s| s.retry_count).unwrap_or(0));
    let retry_outcome = combine_retry_outcomes(
        turn1_summary.outcome,
        turn2_summary.map(|s| s.outcome).unwrap_or(RetryOutcome::NoRetry),
    );

    let post = PostCallProvenance {
        token_usage: efe_nano_sdk::TokenUsage {
            client_estimated_input: 0,
            client_estimated_output: 0,
            provider_reported_input: Some(usage_in),
            provider_reported_output: Some(usage_out),
            is_estimated: false,
        },
        cost_accounting: CostAccounting {
            cost_cents: compute_cost_cents(&bridge.model, usage_in, usage_out),
            prices_table_version: PRICES_TABLE_VERSION,
            provider_quoted_cost_cents: None,
            overflow_capped: false,
        },
        provider_request_id: prid,
        provider_response_time_ms: latency_ms,
        provider_finish_reason: map_finish_reason(stop),
        provider_metadata: std::collections::HashMap::new(),
        output_hash: Some(output_hash),
        completeness,
        safety_flags: Vec::new(),
        tool_calls: Vec::new(),
        model_hash_verified: false,
    };

    StreamingPostCallProvenance {
        base: post,
        streaming_mode: true,
        retry_count: total_retry_count,
        retry_outcome,
        time_to_first_token_ms: ttft_ms,
        abort_reason: abort,
    }
}

/// **N27** : combine two turn-level [`RetryOutcome`]s into the
/// record-level outcome.  Rule of thumb : if any turn was
/// retried, the record reflects that ; if any turn exhausted,
/// the record is "exhausted".
fn combine_retry_outcomes(t1: RetryOutcome, t2: RetryOutcome) -> RetryOutcome {
    use RetryOutcome::*;
    match (t1, t2) {
        (NonRetryable, _) | (_, NonRetryable) => NonRetryable,
        (RetriedExhausted, _) | (_, RetriedExhausted) => RetriedExhausted,
        (RetriedSucceeded, _) | (_, RetriedSucceeded) => RetriedSucceeded,
        _ => NoRetry,
    }
}

fn probe_post_byte(post: bool) -> RetryOutcomeProbe {
    RetryOutcomeProbe {
        http_status: None,
        anthropic_error_type: None,
        retry_after: None,
        network_class: NetworkErrorClass::BodyMidStream,
        post_first_byte_downstream: post,
    }
}

fn probe_cancelled(post: bool) -> RetryOutcomeProbe {
    RetryOutcomeProbe {
        http_status: None,
        anthropic_error_type: None,
        retry_after: None,
        network_class: NetworkErrorClass::Other,
        post_first_byte_downstream: post,
    }
}

fn classify_reqwest_err(e: &reqwest::Error) -> NetworkErrorClass {
    if e.is_timeout() {
        NetworkErrorClass::Timeout
    } else if e.is_connect() {
        NetworkErrorClass::Connect
    } else if e.is_body() {
        NetworkErrorClass::BodyMidStream
    } else if e.is_decode() {
        NetworkErrorClass::Decode
    } else {
        NetworkErrorClass::Other
    }
}

fn extract_anthropic_error_type(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.get("type")).and_then(|t| t.as_str()).map(String::from))
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        let mut t = s[..n].to_string();
        t.push('…');
        t
    }
}

// (The `AtomicArcTarget` helper trait was removed in the
// audit-pass-2 fix : counters are now `Arc<AtomicU64>` in the
// struct itself so a simple `Arc::clone` shares state correctly.)

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{Entity, SwarmId};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn small_bundle() -> ContextBundle {
        let mut b = ContextBundle::empty(SwarmId::new());
        b.language = Some("en".into());
        b.entities.push(Entity {
            entity_type: "person".into(),
            value: "Alice".into(),
            span: Some((0, 5)),
        });
        b
    }

    // ─── Happy path : 2-turn tool-use round-trip ─────────────────

    #[tokio::test]
    async fn happy_path_two_turn_tool_use() {
        let server = MockServer::start().await;
        // Turn 1 : tool_use response.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{
                    "type": "tool_use",
                    "id": "tu_01",
                    "name": "swarm_context",
                    "input": {}
                }],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 20, "output_tokens": 5}
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Turn 2 : final text.
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "Risk score 7/10."}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 50, "output_tokens": 12}
            })))
            .mount(&server)
            .await;
        let bridge = ClaudeBridge::with_endpoint("test-key", server.uri());
        let bundle = small_bundle();
        let task = AgenticTask::new("Analyse this");
        let resp = bridge.call_with_context(&bundle, task).await.unwrap();
        assert_eq!(resp.text, "Risk score 7/10.");
        assert_eq!(resp.stop_reason, StopReason::EndTurn);
        // **N26 + G6** : token usage is now the SUM of both
        // turns (Sprint 13 reported only turn 2).  Turn 1 :
        // input 20, output 5 ; turn 2 : input 50, output 12.
        assert_eq!(resp.usage.input_tokens, 70);
        assert_eq!(resp.usage.output_tokens, 17);
        assert!(resp.cost_microcents > 0);
    }

    // ─── Reject oversized bundle ────────────────────────────────

    #[tokio::test]
    async fn bundle_over_cap_rejected_before_http() {
        let bridge = ClaudeBridge::with_endpoint("x", "http://localhost:1");
        let mut huge = ContextBundle::empty(SwarmId::new());
        huge.summary = Some("y".repeat(300 * 1024));
        let task = AgenticTask::new("x");
        let err = bridge.call_with_context(&huge, task).await.unwrap_err();
        assert!(matches!(err, BridgeError::BundleTooLarge { .. }));
    }

    // ─── 429 → RateLimited ──────────────────────────────────────

    #[tokio::test]
    async fn rate_limited_429() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&server)
            .await;
        let bridge = ClaudeBridge::with_endpoint("k", server.uri());
        let task = AgenticTask::new("x");
        let err = bridge
            .call_with_context(&small_bundle(), task)
            .await
            .unwrap_err();
        assert!(matches!(err, BridgeError::RateLimited));
    }

    // ─── 401 → Authentication ───────────────────────────────────

    #[tokio::test]
    async fn auth_401() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let bridge = ClaudeBridge::with_endpoint("k", server.uri());
        let err = bridge
            .call_with_context(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap_err();
        assert!(matches!(err, BridgeError::Authentication));
    }

    // ─── Protocol violation : no tool_use in turn 1 ─────────────

    #[tokio::test]
    async fn protocol_violation_no_tool_use_in_turn1() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "I cannot help."}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 5, "output_tokens": 3}
            })))
            .mount(&server)
            .await;
        let bridge = ClaudeBridge::with_endpoint("k", server.uri());
        let err = bridge
            .call_with_context(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap_err();
        match err {
            BridgeError::ProtocolViolation { message } => {
                assert!(message.contains("no tool_use"));
            }
            other => panic!("expected ProtocolViolation, got {other:?}"),
        }
    }

    // ─── Protocol violation : wrong tool name ───────────────────

    #[tokio::test]
    async fn protocol_violation_wrong_tool_name() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{
                    "type": "tool_use",
                    "id": "tu_01",
                    "name": "other_tool",
                    "input": {}
                }],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 5, "output_tokens": 3}
            })))
            .mount(&server)
            .await;
        let bridge = ClaudeBridge::with_endpoint("k", server.uri());
        let err = bridge
            .call_with_context(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap_err();
        match err {
            BridgeError::ProtocolViolation { message } => {
                assert!(message.contains("unexpected tool"));
            }
            other => panic!("expected ProtocolViolation, got {other:?}"),
        }
    }

    // ─── 500 → Backend ──────────────────────────────────────────

    #[tokio::test]
    async fn backend_500() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(500).set_body_string("oops"))
            .mount(&server)
            .await;
        let bridge = ClaudeBridge::with_endpoint("k", server.uri());
        let err = bridge
            .call_with_context(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap_err();
        assert!(matches!(err, BridgeError::Backend { .. }));
    }

    // ─── Cost estimator per model family ────────────────────────

    #[test]
    fn cost_estimator_haiku_cheaper_than_sonnet() {
        let usage = AnthropicUsage {
            input_tokens: 1_000,
            output_tokens: 1_000,
        };
        // **N26 + ADR-4** : use the exact model id from the
        // baked Anthropic pricing table.  Sprint 13's
        // `.contains("haiku")` heuristic is gone — Sprint 18
        // uses exact matches against the published table.
        let haiku = estimate_cost_microcents("claude-haiku-3-5", &usage);
        let sonnet = estimate_cost_microcents("claude-sonnet-4-6", &usage);
        let opus = estimate_cost_microcents("claude-opus-4", &usage);
        assert!(haiku < sonnet);
        assert!(sonnet < opus);
    }

    #[test]
    fn cost_estimator_default_uses_sonnet() {
        let usage = AnthropicUsage {
            input_tokens: 1_000,
            output_tokens: 1_000,
        };
        let unknown = estimate_cost_microcents("unknown-model", &usage);
        let sonnet = estimate_cost_microcents("claude-sonnet-4-6", &usage);
        // **N26** : unknown models fall back to Sonnet pricing.
        assert_eq!(unknown, sonnet);
    }

    // ─── Builder + secret hygiene ────────────────────────────────

    #[test]
    fn builder_with_model_overrides_default() {
        let b = ClaudeBridge::new("k").with_model("claude-haiku-4-5");
        let dbg = format!("{b:?}");
        assert!(dbg.contains("claude-haiku-4-5"));
    }

    #[test]
    fn debug_format_does_not_leak_api_key() {
        let b = ClaudeBridge::new("ultra-secret-anthropic-key");
        let dbg = format!("{b:?}");
        assert!(!dbg.contains("ultra-secret-anthropic-key"));
        assert!(dbg.contains("api_key_present: true"));
    }

    // ─── System prompt augmentation ──────────────────────────────

    #[test]
    fn build_system_prompt_default_includes_tool_hint() {
        let t = AgenticTask::new("x");
        let s = build_system_prompt(&t);
        assert!(s.contains("swarm_context"));
    }

    #[test]
    fn build_system_prompt_appends_user_system() {
        let t = AgenticTask::new("x").with_system("You are a juriste.");
        let s = build_system_prompt(&t);
        assert!(s.contains("swarm_context"));
        assert!(s.contains("juriste"));
    }

    // ─── Call count tracking ─────────────────────────────────────

    #[tokio::test]
    async fn call_count_increments_on_send() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{
                    "type": "tool_use",
                    "id": "tu_01",
                    "name": "swarm_context",
                    "input": {}
                }],
                "stop_reason": "tool_use",
                "usage": {"input_tokens": 1, "output_tokens": 0}
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": [{"type": "text", "text": "x"}],
                "stop_reason": "end_turn",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            })))
            .mount(&server)
            .await;
        let b = ClaudeBridge::with_endpoint("k", server.uri());
        assert_eq!(b.call_count(), 0);
        let _ = b
            .call_with_context(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap();
        // call_count counts top-level entries to call_with_context, not
        // HTTP requests.
        assert_eq!(b.call_count(), 1);
    }
}
