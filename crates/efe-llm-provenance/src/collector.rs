// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : Provenance collector — assembles + writes
//! `LlmProvenanceRecord`s.
//!
//! Architecture § 7.4.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use efe_nano_sdk::{
    Blake3Hash, CacheProvenance, Completeness, CostAccounting, FinishReason,
    LlmProvenanceRecord, ModelIdentity, NanoAgentId, NanoAgentVersion, PiiStrategy,
    ProvenanceError, ProvenanceId, RegionCode, SafetyFlag, SamplingParams, TenantId,
    TokenUsage, ToolCallRecord,
};
use uuid::Uuid;

use crate::metrics::N17Metrics;
use crate::query::ModelHashMismatchEvent;
use crate::registry::{ModelRegistry, ModelRegistryError};
use crate::store::LlmProvenanceStore;

// ─────────────────────────────────────────────────────────────────────
//  PreCallProvenance — values known BEFORE the LLM call
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : Provenance metadata known before the LLM
/// provider call.
#[derive(Debug, Clone)]
pub struct PreCallProvenance {
    pub trace_id: Uuid,
    pub swarm_request_id: Uuid,
    pub op_id: String,
    pub tenant_id: TenantId,
    pub agent_id: NanoAgentId,
    pub agent_version: NanoAgentVersion,
    pub model_identity: ModelIdentity,
    pub prompt_template_id: String,
    pub prompt_template_version: String,
    pub prompt_inputs_hash: Blake3Hash,
    pub system_prompt_hash: Blake3Hash,
    pub user_prompt_hash: Blake3Hash,
    pub final_prompt_hash: Blake3Hash,
    pub sampling: SamplingParams,
    pub pii_strategy_applied: PiiStrategy,
    pub redaction_token_set: Vec<String>,
    pub provider_region: Option<RegionCode>,

    /// **Sprint 10 — N18 cross-cutting** : the publisher who authored
    /// the agent making this LLM call.  `None` for EFE-shipped or
    /// pre-N18 agents.  Captured at pre-call time so the trace is
    /// faithful to the publisher state seen at dispatch (I18).
    #[allow(missing_docs)]
    pub publisher_id: Option<efe_nano_sdk::PublisherId>,

    /// **Sprint 10 — N18 cross-cutting** : KYC tier snapshot at
    /// dispatch time.  Pinned with the call so subsequent tier
    /// changes don't retroactively alter the audit trail.
    #[allow(missing_docs)]
    pub publisher_kyc_tier: Option<efe_nano_sdk::KycTier>,

    /// **Sprint 11 — N19 cross-cutting** : if this call is triggered
    /// by a gold-test case, the originating `GoldCaseId` is propagated
    /// here so every downstream `LlmProvenanceRecord` carries it (G12).
    #[allow(missing_docs)]
    pub gold_case_id: Option<efe_nano_sdk::GoldCaseId>,
}

/// **Sprint 9 — N17** : Provenance metadata captured AFTER the LLM
/// call returns.  Combined with [`PreCallProvenance`] into a final
/// `LlmProvenanceRecord`.
#[derive(Debug, Clone)]
pub struct PostCallProvenance {
    pub token_usage: TokenUsage,
    pub cost_accounting: CostAccounting,
    pub provider_request_id: Option<String>,
    pub provider_response_time_ms: u32,
    pub provider_finish_reason: FinishReason,
    pub provider_metadata: HashMap<String, serde_json::Value>,
    pub output_hash: Option<Blake3Hash>,
    pub completeness: Completeness,
    pub safety_flags: Vec<SafetyFlag>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub model_hash_verified: bool,
}

impl Default for PostCallProvenance {
    fn default() -> Self {
        Self {
            token_usage: TokenUsage::default(),
            cost_accounting: CostAccounting::default(),
            provider_request_id: None,
            provider_response_time_ms: 0,
            provider_metadata: HashMap::new(),
            output_hash: None,
            completeness: Completeness::Complete,
            safety_flags: vec![],
            tool_calls: vec![],
            provider_finish_reason: FinishReason::Unknown,
            model_hash_verified: false,
        }
    }
}

/// **Sprint 9 — N17** : Cache-hit provenance (N12 integration).
#[derive(Debug, Clone)]
pub struct CacheHitProvenance {
    pub pre_call: PreCallProvenance,
    pub cache_provenance: CacheProvenance,
}

/// **Sprint 9 — N17** : Replay provenance (N16 integration).
#[derive(Debug, Clone)]
pub struct ReplayProvenance {
    pub pre_call: PreCallProvenance,
    pub post_call: PostCallProvenance,
    pub replay_of: ProvenanceId,
}

// ─────────────────────────────────────────────────────────────────────
//  StreamingPostCallProvenance  (Sprint 19 — N27)
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 19 — N27** : streaming-specific post-call provenance.
///
/// Composes a regular [`PostCallProvenance`] with the four N27
/// fields that are only meaningful for streamed dispatches.
/// Consumed by [`LlmProvenanceCollector::post_call_streaming`].
#[derive(Debug, Clone)]
pub struct StreamingPostCallProvenance {
    pub base: PostCallProvenance,
    /// `streaming_mode` flag stamped on the record.  Should be
    /// `true` for records emitted via the SSE wire surface ;
    /// the buffered surface uses `post_call` directly with the
    /// flag implicitly `false`.
    pub streaming_mode: bool,
    /// Retry count from [`crate::caller`] / bridge retry loop.
    pub retry_count: u8,
    /// Final retry outcome (see SDK [`RetryOutcome`]).
    pub retry_outcome: efe_nano_sdk::RetryOutcome,
    /// Time-to-first-token (turn 1 only ; see N27 § 5).
    pub time_to_first_token_ms: Option<u32>,
    /// If the stream aborted, the closed-taxonomy reason.  None
    /// for successful streams.
    pub abort_reason: Option<efe_nano_sdk::AbortReason>,
}

impl StreamingPostCallProvenance {
    /// **N27** : convenience constructor with sane defaults.
    pub fn from_base(base: PostCallProvenance) -> Self {
        Self {
            base,
            streaming_mode: true,
            retry_count: 0,
            retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
            time_to_first_token_ms: None,
            abort_reason: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  LlmProvenanceCollector trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : the collector assembles records + writes
/// them to the store + emits metrics.
#[async_trait]
pub trait LlmProvenanceCollector: Send + Sync + 'static {
    /// **N17** : Pre-call hook — verify model hash + (optionally)
    /// emit `ModelHashMismatch` event on F3 detection.
    async fn pre_call(
        &self,
        pre: &PreCallProvenance,
        actual_hash: Option<&Blake3Hash>,
    ) -> Result<(), ProvenanceError>;

    /// **N17** : Post-call hook — assemble + write the final record.
    async fn post_call(
        &self,
        pre: PreCallProvenance,
        post: PostCallProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError>;

    /// **N17** : N12 cache-hit hook.  Emits a `LlmProvenanceRecord`
    /// with `cache_hit = Some(...)` and zero token usage.
    async fn on_cache_hit(
        &self,
        hit: CacheHitProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError>;

    /// **N17** : N16 replay hook.  Emits a record with `replay_of`
    /// pointing at the original.
    async fn on_replay(
        &self,
        replay: ReplayProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError>;

    /// **Sprint 19 — N27** : streaming post-call hook.
    ///
    /// Equivalent to [`Self::post_call`] but additionally stamps
    /// the four N27 streaming fields (`streaming_mode`,
    /// `retry_count`, `retry_outcome`, `time_to_first_token_ms`)
    /// and may override `Completeness` if `abort_reason` is set.
    ///
    /// Default impl :
    /// 1. Calls `post_call(pre, base)` to write the base record.
    /// 2. Asks the implementor to patch the streaming fields via
    ///    [`Self::patch_streaming_fields`].
    ///
    /// Concrete impls SHOULD override for atomicity (in-memory
    /// store : trivial ; Postgres N28 : single UPDATE).
    async fn post_call_streaming(
        &self,
        pre: PreCallProvenance,
        streaming: StreamingPostCallProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        // The default impl is intentionally simple : write the
        // base record, then patch the N27 fields.  Implementors
        // that want atomicity should override.
        let mut base_post = streaming.base.clone();
        // If the stream aborted, override completeness to Aborted.
        if let Some(reason) = streaming.abort_reason.clone() {
            base_post.completeness = efe_nano_sdk::Completeness::Aborted {
                received_tokens: streaming.base.token_usage.canonical_output(),
                reason,
            };
        }
        let record = self.post_call(pre, base_post).await?;
        let patched = self
            .patch_streaming_fields(
                record.provenance_id,
                streaming.streaming_mode,
                streaming.retry_count,
                streaming.retry_outcome,
                streaming.time_to_first_token_ms,
            )
            .await?;
        Ok(patched.unwrap_or(record))
    }

    /// **Sprint 19 — N27** : patch the four streaming fields onto
    /// an already-written record.  Returns the updated record if
    /// the store supports read-back ; `None` is acceptable for
    /// fire-and-forget impls (the bridge's caller can re-fetch
    /// via the query API).
    ///
    /// Default impl is a no-op that returns `Ok(None)`.  The
    /// [`DefaultLlmProvenanceCollector`] overrides to write the
    /// fields into [`crate::store::InMemoryLlmProvenanceStore`].
    async fn patch_streaming_fields(
        &self,
        _id: ProvenanceId,
        _streaming_mode: bool,
        _retry_count: u8,
        _retry_outcome: efe_nano_sdk::RetryOutcome,
        _time_to_first_token_ms: Option<u32>,
    ) -> Result<Option<LlmProvenanceRecord>, ProvenanceError> {
        Ok(None)
    }
}

// ─────────────────────────────────────────────────────────────────────
//  DefaultLlmProvenanceCollector
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 9 — N17** : default collector wiring store + registry +
/// metrics.
pub struct DefaultLlmProvenanceCollector {
    store: Arc<dyn LlmProvenanceStore>,
    registry: Arc<dyn ModelRegistry>,
    metrics: Option<Arc<N17Metrics>>,
}

impl std::fmt::Debug for DefaultLlmProvenanceCollector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DefaultLlmProvenanceCollector")
            .field("has_metrics", &self.metrics.is_some())
            .finish()
    }
}

impl DefaultLlmProvenanceCollector {
    /// **N17** : Construct a collector.
    pub fn new(
        store: Arc<dyn LlmProvenanceStore>,
        registry: Arc<dyn ModelRegistry>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            registry,
            metrics: None,
        })
    }

    /// **N17** : Construct a collector with metrics.
    pub fn with_metrics(
        store: Arc<dyn LlmProvenanceStore>,
        registry: Arc<dyn ModelRegistry>,
        metrics: Arc<N17Metrics>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            registry,
            metrics: Some(metrics),
        })
    }

    /// **N17** : Access the underlying store.
    pub fn store(&self) -> &Arc<dyn LlmProvenanceStore> {
        &self.store
    }

    /// **N17** : Access the underlying registry.
    pub fn registry(&self) -> &Arc<dyn ModelRegistry> {
        &self.registry
    }

    /// **N17** : Access metrics (if installed).
    pub fn metrics(&self) -> Option<&Arc<N17Metrics>> {
        self.metrics.as_ref()
    }
}

#[async_trait]
impl LlmProvenanceCollector for DefaultLlmProvenanceCollector {
    async fn pre_call(
        &self,
        pre: &PreCallProvenance,
        actual_hash: Option<&Blake3Hash>,
    ) -> Result<(), ProvenanceError> {
        let identity = &pre.model_identity;
        // **Post-audit fix — bug I** : single registry lookup ;
        // verification is performed inline below (avoids two
        // round-trips through `verify_hash` which itself called
        // `lookup`).
        let registered = self.registry.lookup(identity).await.map_err(|e| match e {
            ModelRegistryError::NotRegistered {
                provider,
                model_id,
                version,
            } => ProvenanceError::ModelNotRegistered {
                provider,
                model_id,
                version,
            },
            other => ProvenanceError::Internal {
                message: other.to_string(),
            },
        })?;

        // Dev-only escape hatch — skip all verification.
        if registered.dev_skip_hash_check {
            return Ok(());
        }

        // Hosted model without expected hash → nothing to verify
        // beyond the registry lookup.
        let Some(expected) = &registered.expected_content_hash else {
            return Ok(());
        };

        // **Post-audit fix — bug C** : if the registry has an
        // expected_content_hash but the caller didn't provide an
        // actual hash, fail-closed.  Without this, G3 is bypassable
        // by simply omitting the hash.
        let Some(actual) = actual_hash else {
            return Err(ProvenanceError::ModelIntegrityViolation {
                requested_hash: expected.to_hex(),
                actual_hash: "<missing : caller did not provide a hash>".into(),
            });
        };

        // Exact-match success path.
        if expected == actual {
            return Ok(());
        }

        // Hash mismatch — emit a drift event + tick the metric +
        // return typed integrity violation.
        let event = ModelHashMismatchEvent {
            event_id: Uuid::new_v4(),
            tenant_id: pre.tenant_id,
            model_identity: identity.clone(),
            expected_hash: *expected,
            actual_hash: *actual,
            detected_at: SystemTime::now(),
            trace_id: Some(pre.trace_id),
        };
        let _ = self.store.record_drift_event(event).await;
        if let Some(m) = &self.metrics {
            m.tick_model_hash_mismatch(&identity.model_id);
        }
        Err(ProvenanceError::ModelIntegrityViolation {
            requested_hash: expected.to_hex(),
            actual_hash: actual.to_hex(),
        })
    }

    async fn post_call(
        &self,
        pre: PreCallProvenance,
        post: PostCallProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        // **Post-audit fix — bug B + I10 immutability** : a single
        // build → single insert.  No double-insert ; no overwrite.
        let model_id = pre.model_identity.model_id.clone();
        let record = build_record(pre, post, None, None);
        self.write_record(&record).await?;
        self.tick_post_call_metrics(&model_id, &record);
        Ok(record)
    }

    async fn on_cache_hit(
        &self,
        hit: CacheHitProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        let pre = hit.pre_call;
        let cache_provenance = hit.cache_provenance;
        let record = build_record(
            pre,
            PostCallProvenance {
                token_usage: TokenUsage::default(),
                cost_accounting: CostAccounting::default(),
                provider_request_id: None,
                provider_response_time_ms: 0,
                provider_finish_reason: FinishReason::Stop,
                provider_metadata: HashMap::new(),
                output_hash: None,
                completeness: Completeness::Complete,
                safety_flags: vec![],
                tool_calls: vec![],
                model_hash_verified: false,
            },
            None,
            Some(cache_provenance),
        );
        self.write_record(&record).await?;
        if let Some(m) = &self.metrics {
            m.tick_cache_hit();
        }
        Ok(record)
    }

    async fn on_replay(
        &self,
        replay: ReplayProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        // **Post-audit fix — bug B** : build the record WITH
        // replay_of set from the start, single insert (was a
        // double-insert that violated I10 read-only).
        let model_id = replay.pre_call.model_identity.model_id.clone();
        let record = build_record(
            replay.pre_call,
            replay.post_call,
            Some(replay.replay_of),
            None,
        );
        self.write_record(&record).await?;
        // **Post-audit fix — bug G** : ticking metric set is the
        // SAME as post_call (response time, safety flags, drift) +
        // additionally the replay counter.  Previously this path
        // skipped 3 of those metrics.
        self.tick_post_call_metrics(&model_id, &record);
        if let Some(m) = &self.metrics {
            m.tick_replay_call();
        }
        Ok(record)
    }

    /// **Sprint 19 — N27 + G14** : streaming post-call.  Builds
    /// the record with N27 fields stamped in one shot and writes
    /// it atomically (single store `insert`).  Overrides the
    /// trait default for atomicity.
    async fn post_call_streaming(
        &self,
        pre: PreCallProvenance,
        streaming: StreamingPostCallProvenance,
    ) -> Result<LlmProvenanceRecord, ProvenanceError> {
        let model_id = pre.model_identity.model_id.clone();
        let mut base = streaming.base.clone();
        // **ADR-N27-4** : if the stream aborted, override
        // completeness to Aborted with the closed-taxonomy reason.
        if let Some(reason) = streaming.abort_reason.clone() {
            base.completeness = efe_nano_sdk::Completeness::Aborted {
                received_tokens: streaming.base.token_usage.canonical_output(),
                reason,
            };
        }
        let mut record = build_record(pre, base, None, None);
        // **N27 + I24** : stamp the four streaming fields.
        record.streaming_mode = streaming.streaming_mode;
        record.retry_count = streaming.retry_count;
        record.retry_outcome = streaming.retry_outcome;
        record.time_to_first_token_ms = streaming.time_to_first_token_ms;
        self.write_record(&record).await?;
        self.tick_post_call_metrics(&model_id, &record);
        Ok(record)
    }

    /// **Sprint 19 — N27** : in-place patch — delegates to the
    /// underlying store.  Used when the trait default impl is
    /// in effect (we override `post_call_streaming` so this is
    /// only reached by direct callers).
    async fn patch_streaming_fields(
        &self,
        provenance_id: ProvenanceId,
        streaming_mode: bool,
        retry_count: u8,
        retry_outcome: efe_nano_sdk::RetryOutcome,
        time_to_first_token_ms: Option<u32>,
    ) -> Result<Option<LlmProvenanceRecord>, ProvenanceError> {
        self.store
            .patch_streaming_fields(
                provenance_id,
                streaming_mode,
                retry_count,
                retry_outcome,
                time_to_first_token_ms,
            )
            .await
            .map_err(|e| ProvenanceError::Store {
                message: e.to_string(),
            })
    }
}

impl DefaultLlmProvenanceCollector {
    /// **Post-audit fix — bug G** : shared metric-ticking path for
    /// `post_call` AND `on_replay` so coverage is symmetric.
    /// Previously `on_replay` skipped `observe_response_time`,
    /// `tick_safety_flag`, and `set_token_drift`.
    fn tick_post_call_metrics(&self, model_id: &str, record: &LlmProvenanceRecord) {
        let Some(m) = &self.metrics else {
            return;
        };
        let outcome = if record.completeness.is_usable() {
            "complete"
        } else {
            record.completeness.label()
        };
        m.tick_llm_call(model_id, outcome);
        m.add_tokens(
            model_id,
            record.token_usage.canonical_input(),
            record.token_usage.canonical_output(),
        );
        m.add_cost(model_id, record.cost_accounting.cost_cents);
        m.observe_response_time(
            model_id,
            f64::from(record.provider_response_time_ms) / 1_000.0,
        );
        for flag in &record.safety_flags {
            m.tick_safety_flag(flag.label());
        }
        if let Some(drift) = record.token_usage.drift_ratio() {
            m.set_token_drift(model_id, drift);
        }
    }

    /// **Post-audit fix — bug A** : centralised write path that
    /// ticks both success AND failure on the metric.
    async fn write_record(
        &self,
        record: &LlmProvenanceRecord,
    ) -> Result<(), ProvenanceError> {
        match self.store.insert(record.clone()).await {
            Ok(()) => {
                if let Some(m) = &self.metrics {
                    m.tick_provenance_write(true);
                }
                Ok(())
            }
            Err(e) => {
                if let Some(m) = &self.metrics {
                    m.tick_provenance_write(false);
                }
                Err(ProvenanceError::Store {
                    message: e.to_string(),
                })
            }
        }
    }
}

/// **Post-audit fix — bug B** : shared builder, single source of
/// truth for assembling a [`LlmProvenanceRecord`] from
/// `PreCallProvenance` + `PostCallProvenance` + optional
/// replay / cache flags.
fn build_record(
    pre: PreCallProvenance,
    post: PostCallProvenance,
    replay_of: Option<ProvenanceId>,
    cache_hit: Option<efe_nano_sdk::CacheProvenance>,
) -> LlmProvenanceRecord {
    LlmProvenanceRecord {
        provenance_id: ProvenanceId::new(),
        trace_id: pre.trace_id,
        swarm_request_id: pre.swarm_request_id,
        op_id: pre.op_id,
        tenant_id: pre.tenant_id,
        agent_id: pre.agent_id,
        agent_version: pre.agent_version,
        model_identity: pre.model_identity,
        model_hash_verified: post.model_hash_verified,
        prompt_template_id: pre.prompt_template_id,
        prompt_template_version: pre.prompt_template_version,
        prompt_inputs_hash: pre.prompt_inputs_hash,
        system_prompt_hash: pre.system_prompt_hash,
        user_prompt_hash: pre.user_prompt_hash,
        final_prompt_hash: pre.final_prompt_hash,
        sampling: pre.sampling,
        token_usage: post.token_usage,
        cost_accounting: post.cost_accounting,
        provider_request_id: post.provider_request_id,
        provider_response_time_ms: post.provider_response_time_ms,
        provider_finish_reason: post.provider_finish_reason,
        provider_region: pre.provider_region,
        provider_metadata: post.provider_metadata,
        output_hash: post.output_hash,
        completeness: post.completeness,
        safety_flags: post.safety_flags,
        tool_calls: post.tool_calls,
        pii_strategy_applied: pre.pii_strategy_applied,
        redaction_token_set: pre.redaction_token_set,
        replay_of,
        cache_hit,
        recorded_at: SystemTime::now(),
        publisher_id: pre.publisher_id,
        publisher_kyc_tier: pre.publisher_kyc_tier,
        gold_case_id: pre.gold_case_id,
        // **Sprint 19 — N27** : default to "no streaming, no retry".
        // The streaming dispatch path patches these after the
        // buffered record was built (see `patch_streaming_fields`).
        streaming_mode: false,
        retry_count: 0,
        retry_outcome: efe_nano_sdk::RetryOutcome::NoRetry,
        time_to_first_token_ms: None,
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{InMemoryModelRegistry, RegisteredModel};
    use crate::store::InMemoryLlmProvenanceStore;

    fn pre(tenant: TenantId, identity: ModelIdentity) -> PreCallProvenance {
        PreCallProvenance {
            trace_id: Uuid::new_v4(),
            swarm_request_id: Uuid::new_v4(),
            op_id: "op_1".into(),
            tenant_id: tenant,
            agent_id: NanoAgentId::new("agent").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            model_identity: identity,
            prompt_template_id: "t".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::from_bytes([1u8; 32]),
            system_prompt_hash: Blake3Hash::from_bytes([2u8; 32]),
            user_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
            final_prompt_hash: Blake3Hash::from_bytes([4u8; 32]),
            sampling: SamplingParams::default(),
            pii_strategy_applied: PiiStrategy::Strict,
            redaction_token_set: vec![],
            provider_region: Some(RegionCode::UsEast1),
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
        }
    }

    async fn fresh_collector() -> Arc<DefaultLlmProvenanceCollector> {
        let store: Arc<dyn LlmProvenanceStore> = Arc::new(InMemoryLlmProvenanceStore::new());
        let registry = Arc::new(InMemoryModelRegistry::new());
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        registry
            .register(RegisteredModel::hosted(identity, 1000.0, 3000.0, 1))
            .await
            .unwrap();
        DefaultLlmProvenanceCollector::with_metrics(
            store,
            registry as Arc<dyn ModelRegistry>,
            N17Metrics::new(),
        )
    }

    #[tokio::test]
    async fn post_call_writes_record() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let p = pre(tenant, identity.clone());
        let trace_id = p.trace_id;
        let record = c.post_call(p, PostCallProvenance::default()).await.unwrap();
        let got = c.store.get_by_trace(trace_id, tenant).await.unwrap();
        assert_eq!(got, record);
    }

    #[tokio::test]
    async fn pre_call_no_actual_hash_lookup_ok() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let p = pre(tenant, identity);
        c.pre_call(&p, None).await.unwrap();
    }

    #[tokio::test]
    async fn pre_call_hash_mismatch_records_drift_event() {
        let store: Arc<InMemoryLlmProvenanceStore> = Arc::new(InMemoryLlmProvenanceStore::new());
        let registry = Arc::new(InMemoryModelRegistry::new());
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let mut m = RegisteredModel::hosted(identity.clone(), 1000.0, 3000.0, 1);
        m.expected_content_hash = Some(Blake3Hash::from_bytes([0xAAu8; 32]));
        registry.register(m).await.unwrap();
        let c = DefaultLlmProvenanceCollector::with_metrics(
            store.clone() as Arc<dyn LlmProvenanceStore>,
            registry as Arc<dyn ModelRegistry>,
            N17Metrics::new(),
        );
        let tenant = TenantId::new();
        let p = pre(tenant, identity);
        let actual = Blake3Hash::from_bytes([0xBBu8; 32]);
        let err = c.pre_call(&p, Some(&actual)).await.unwrap_err();
        assert!(matches!(err, ProvenanceError::ModelIntegrityViolation { .. }));
        assert_eq!(store.drift_event_count(), 1);
    }

    #[tokio::test]
    async fn pre_call_hash_match_passes() {
        let store: Arc<dyn LlmProvenanceStore> = Arc::new(InMemoryLlmProvenanceStore::new());
        let registry = Arc::new(InMemoryModelRegistry::new());
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let mut m = RegisteredModel::hosted(identity.clone(), 1000.0, 3000.0, 1);
        m.expected_content_hash = Some(Blake3Hash::from_bytes([0xCCu8; 32]));
        registry.register(m).await.unwrap();
        let c = DefaultLlmProvenanceCollector::new(
            store,
            registry as Arc<dyn ModelRegistry>,
        );
        let tenant = TenantId::new();
        let p = pre(tenant, identity);
        let actual = Blake3Hash::from_bytes([0xCCu8; 32]);
        c.pre_call(&p, Some(&actual)).await.unwrap();
    }

    #[tokio::test]
    async fn on_cache_hit_emits_record_with_cache_provenance() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let p = pre(tenant, identity);
        let trace_id = p.trace_id;
        let hit = CacheHitProvenance {
            pre_call: p,
            cache_provenance: CacheProvenance {
                original_trace_id: Uuid::new_v4(),
                original_provenance_id: ProvenanceId::new(),
                cache_key_version: 1,
            },
        };
        let record = c.on_cache_hit(hit).await.unwrap();
        assert!(record.is_cache_hit());
        let got = c.store.get_by_trace(trace_id, tenant).await.unwrap();
        assert!(got.is_cache_hit());
    }

    #[tokio::test]
    async fn on_replay_emits_record_with_replay_of() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let original_id = ProvenanceId::new();
        let p = pre(tenant, identity);
        let trace_id = p.trace_id;
        let record = c
            .on_replay(ReplayProvenance {
                pre_call: p,
                post_call: PostCallProvenance::default(),
                replay_of: original_id,
            })
            .await
            .unwrap();
        assert!(record.is_replay());
        assert_eq!(record.replay_of, Some(original_id));
        let got = c.store.get_by_trace(trace_id, tenant).await.unwrap();
        assert_eq!(got.replay_of, Some(original_id));
    }

    #[tokio::test]
    async fn post_call_ticks_metrics() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let p = pre(tenant, identity);
        let mut post = PostCallProvenance::default();
        post.token_usage.client_estimated_input = 100;
        post.token_usage.client_estimated_output = 50;
        post.token_usage.provider_reported_input = Some(110);
        post.token_usage.provider_reported_output = Some(55);
        post.cost_accounting.cost_cents = 0.5;
        c.post_call(p, post).await.unwrap();
        let metrics = c.metrics().unwrap();
        assert!(metrics.llm_call_total("gpt-4", "complete") >= 1);
    }

    #[tokio::test]
    async fn debug_format_does_not_panic() {
        let c = fresh_collector().await;
        let _ = format!("{c:?}");
    }

    // ─────────────────────────────────────────────────────────────────
    //  Post-audit regression tests
    // ─────────────────────────────────────────────────────────────────

    /// **Bug A regression** : when the store write fails, the
    /// `tick_provenance_write(false)` metric MUST be ticked.
    #[tokio::test]
    async fn store_failure_ticks_provenance_failure_metric() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct FailStore {
            inner: Arc<crate::store::InMemoryLlmProvenanceStore>,
            fail: AtomicBool,
        }
        #[async_trait::async_trait]
        impl crate::store::LlmProvenanceStore for FailStore {
            async fn insert(
                &self,
                _record: LlmProvenanceRecord,
            ) -> Result<(), crate::store::ProvenanceStoreError> {
                if self.fail.load(Ordering::SeqCst) {
                    return Err(crate::store::ProvenanceStoreError::Backend {
                        message: "simulated".into(),
                    });
                }
                Ok(())
            }
            async fn get_by_trace(
                &self,
                trace_id: Uuid,
                tenant: TenantId,
            ) -> Result<LlmProvenanceRecord, crate::store::ProvenanceStoreError> {
                self.inner.get_by_trace(trace_id, tenant).await
            }
            async fn get_by_provenance(
                &self,
                pid: ProvenanceId,
                tenant: TenantId,
            ) -> Result<LlmProvenanceRecord, crate::store::ProvenanceStoreError> {
                self.inner.get_by_provenance(pid, tenant).await
            }
            async fn list_by_swarm(
                &self,
                swarm_id: Uuid,
                tenant: TenantId,
            ) -> Result<Vec<LlmProvenanceRecord>, crate::store::ProvenanceStoreError> {
                self.inner.list_by_swarm(swarm_id, tenant).await
            }
            async fn list_by_model(
                &self,
                model: &ModelIdentity,
                tenant: TenantId,
                window: crate::query::DateRange,
            ) -> Result<Vec<LlmProvenanceRecord>, crate::store::ProvenanceStoreError> {
                self.inner.list_by_model(model, tenant, window).await
            }
            async fn cost_summary(
                &self,
                tenant: TenantId,
                window: crate::query::DateRange,
            ) -> Result<crate::query::CostSummary, crate::store::ProvenanceStoreError> {
                self.inner.cost_summary(tenant, window).await
            }
            async fn list_drift_events(
                &self,
                tenant: TenantId,
                window: crate::query::DateRange,
            ) -> Result<Vec<crate::query::ModelHashMismatchEvent>, crate::store::ProvenanceStoreError> {
                self.inner.list_drift_events(tenant, window).await
            }
            async fn record_drift_event(
                &self,
                event: crate::query::ModelHashMismatchEvent,
            ) -> Result<(), crate::store::ProvenanceStoreError> {
                self.inner.record_drift_event(event).await
            }
        }

        let inner = Arc::new(crate::store::InMemoryLlmProvenanceStore::new());
        let store: Arc<dyn crate::store::LlmProvenanceStore> = Arc::new(FailStore {
            inner,
            fail: AtomicBool::new(true),
        });
        let registry = Arc::new(InMemoryModelRegistry::new());
        registry
            .register(RegisteredModel::hosted(
                ModelIdentity::hosted("openai", "gpt-4", "2024"),
                1000.0,
                3000.0,
                1,
            ))
            .await
            .unwrap();
        let metrics = N17Metrics::new();
        let collector = DefaultLlmProvenanceCollector::with_metrics(
            store,
            registry as Arc<dyn ModelRegistry>,
            metrics.clone(),
        );
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let p = pre(tenant, identity);
        let err = collector
            .post_call(p, PostCallProvenance::default())
            .await
            .unwrap_err();
        assert!(matches!(err, ProvenanceError::Store { .. }));
        // The failure metric MUST have been ticked.
        assert_eq!(metrics.provenance_writes_failure_total(), 1);
        assert_eq!(metrics.provenance_writes_success_total(), 0);
    }

    /// **Bug B regression** : `on_replay` MUST insert the record
    /// exactly once with `replay_of` set from the start (no
    /// double-insert).
    #[tokio::test]
    async fn on_replay_inserts_record_once_with_replay_of_set() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let original = ProvenanceId::new();
        let p = pre(tenant, identity);
        let trace_id = p.trace_id;
        let r = c
            .on_replay(ReplayProvenance {
                pre_call: p,
                post_call: PostCallProvenance::default(),
                replay_of: original,
            })
            .await
            .unwrap();
        assert_eq!(r.replay_of, Some(original));
        // The stored record MUST already have replay_of set — no
        // window where a replay record exists without the link.
        let got = c.store.get_by_trace(trace_id, tenant).await.unwrap();
        assert_eq!(got.replay_of, Some(original));
    }

    /// **Bug C regression** : if caller passes `actual_hash = None`
    /// but registry has an `expected_content_hash`, pre_call MUST
    /// fail-closed.
    #[tokio::test]
    async fn pre_call_with_missing_actual_hash_fails_closed() {
        let store: Arc<dyn crate::store::LlmProvenanceStore> =
            Arc::new(crate::store::InMemoryLlmProvenanceStore::new());
        let registry = Arc::new(InMemoryModelRegistry::new());
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let mut m = RegisteredModel::hosted(identity.clone(), 1000.0, 3000.0, 1);
        m.expected_content_hash = Some(Blake3Hash::from_bytes([0xAAu8; 32]));
        registry.register(m).await.unwrap();
        let c = DefaultLlmProvenanceCollector::new(store, registry as Arc<dyn ModelRegistry>);
        let tenant = TenantId::new();
        let p = pre(tenant, identity);
        // Caller forgets to provide actual_hash — must NOT silently pass.
        let err = c.pre_call(&p, None).await.unwrap_err();
        assert!(matches!(err, ProvenanceError::ModelIntegrityViolation { .. }));
    }

    /// **Bug C — defensive complement** : if caller passes
    /// `actual_hash = None` AND registry has NO expected hash (hosted
    /// model), pre_call must pass.
    #[tokio::test]
    async fn pre_call_with_no_actual_and_no_expected_passes() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let p = pre(tenant, identity);
        c.pre_call(&p, None).await.unwrap();
    }

    /// **Bug E regression** : looking up an unknown model returns
    /// the typed `ModelNotRegistered`, NOT `Internal`.
    #[tokio::test]
    async fn pre_call_unknown_model_returns_typed_error() {
        let store: Arc<dyn crate::store::LlmProvenanceStore> =
            Arc::new(crate::store::InMemoryLlmProvenanceStore::new());
        let registry = Arc::new(InMemoryModelRegistry::new());
        let c = DefaultLlmProvenanceCollector::new(store, registry as Arc<dyn ModelRegistry>);
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("unknown-provider", "ghost", "1.0");
        let p = pre(tenant, identity);
        let err = c.pre_call(&p, None).await.unwrap_err();
        assert!(matches!(err, ProvenanceError::ModelNotRegistered { .. }));
    }

    /// **Bug B + I10 combined** : on_replay's record's
    /// `provenance_id` is the same in store and in returned record.
    #[tokio::test]
    async fn on_replay_record_id_matches_stored_id() {
        let c = fresh_collector().await;
        let tenant = TenantId::new();
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let p = pre(tenant, identity);
        let trace_id = p.trace_id;
        let returned = c
            .on_replay(ReplayProvenance {
                pre_call: p,
                post_call: PostCallProvenance::default(),
                replay_of: ProvenanceId::new(),
            })
            .await
            .unwrap();
        let stored = c.store.get_by_trace(trace_id, tenant).await.unwrap();
        assert_eq!(returned.provenance_id, stored.provenance_id);
    }
}
