// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N17** : `call_with_provenance` — the glue helper
//! that wraps any LLM call with pre-call hash verification and
//! post-call provenance emission.
//!
//! Architecture § 7.5.
//!
//! # Why a helper function rather than an `LlmClient` wrapper
//!
//! The Sprint 5 `LlmClient` trait has a fixed `LlmCallRequest` /
//! `LlmCallResponse` shape that doesn't carry tenant / agent /
//! prompt-template context.  Wrapping it forces the wrapper to
//! either (a) refuse to type-check or (b) re-implement the trait
//! with a richer signature.
//!
//! Instead, N17 ships a **glue function** that the dispatcher's
//! Mode C runtime invokes around its existing LLM call.  The
//! caller is responsible for providing the rich [`PreCallProvenance`]
//! context.  The function then :
//!
//! 1. Calls [`LlmProvenanceCollector::pre_call`] — verifies the
//!    model hash (architecture G3 + I15).
//! 2. Awaits the closure that performs the underlying LLM call.
//! 3. Calls [`LlmProvenanceCollector::post_call`] — assembles the
//!    final record + writes to store + emits metrics.
//!
//! Sprint 10 will additionally ship a typed `LlmClient` wrapper
//! once the runtime team picks a stable wider call shape.

use std::future::Future;

use efe_nano_sdk::{Blake3Hash, LlmProvenanceRecord, ProvenanceError};

use crate::collector::{LlmProvenanceCollector, PostCallProvenance, PreCallProvenance};

/// **Sprint 9 — N17** : the outcome of a provenance-collected call.
#[derive(Debug)]
pub struct ProvenancedCall<T> {
    /// The opaque response returned by the underlying LLM call.
    pub response: T,
    /// The provenance record persisted to the store + audit chain.
    pub record: LlmProvenanceRecord,
}

/// **Sprint 9 — N17** : invoke an LLM call with full provenance
/// collection.
///
/// `collector` — the collector that will verify + persist.
/// `pre` — the fully-populated pre-call context.
/// `pre_call_hash` — optional content-hash of the model the
///                   provider is about to serve (for hosted models
///                   this is typically `None`).
/// `make_call` — a closure that performs the underlying LLM call
///               and returns `(response, post)` — the response and
///               the post-call provenance.
///
/// Returns either the [`ProvenancedCall`] tuple or a typed
/// [`ProvenanceError`] (model integrity violation, store fault,
/// etc.).
pub async fn call_with_provenance<F, Fut, T>(
    collector: &(dyn LlmProvenanceCollector),
    pre: PreCallProvenance,
    pre_call_hash: Option<&Blake3Hash>,
    make_call: F,
) -> Result<ProvenancedCall<T>, ProvenanceError>
where
    F: FnOnce(PreCallProvenance) -> Fut,
    Fut: Future<Output = Result<(T, PostCallProvenance), ProvenanceError>>,
{
    // Step 1 : pre-call verification (G3 + I15 + F3).
    collector.pre_call(&pre, pre_call_hash).await?;

    // Step 2 : underlying LLM call.  The closure consumes `pre` and
    // returns it as part of the post.  This avoids cloning the
    // potentially-large `pre` while keeping the API ergonomic.
    let pre_clone = pre.clone();
    let (response, post) = make_call(pre).await?;

    // Step 3 : post-call provenance emission.
    let record = collector.post_call(pre_clone, post).await?;

    Ok(ProvenancedCall { response, record })
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{InMemoryModelRegistry, RegisteredModel};
    use crate::store::{InMemoryLlmProvenanceStore, LlmProvenanceStore};
    use crate::collector::DefaultLlmProvenanceCollector;
    use crate::registry::ModelRegistry;
    use crate::metrics::N17Metrics;
    use efe_nano_sdk::{
        Completeness, CostAccounting, FinishReason, ModelIdentity, NanoAgentId,
        NanoAgentVersion, PiiStrategy, RegionCode, SamplingParams, TenantId, TokenUsage,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    async fn fresh_collector() -> Arc<DefaultLlmProvenanceCollector> {
        let store: Arc<dyn LlmProvenanceStore> =
            Arc::new(InMemoryLlmProvenanceStore::new());
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
        DefaultLlmProvenanceCollector::with_metrics(
            store,
            registry as Arc<dyn ModelRegistry>,
            N17Metrics::new(),
        )
    }

    fn sample_pre() -> PreCallProvenance {
        PreCallProvenance {
            trace_id: Uuid::new_v4(),
            swarm_request_id: Uuid::new_v4(),
            op_id: "test_op".into(),
            tenant_id: TenantId::new(),
            agent_id: NanoAgentId::new("test_agent").unwrap(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            model_identity: ModelIdentity::hosted("openai", "gpt-4", "2024"),
            prompt_template_id: "summarize".into(),
            prompt_template_version: "1.0.0".into(),
            prompt_inputs_hash: Blake3Hash::from_bytes([1u8; 32]),
            system_prompt_hash: Blake3Hash::from_bytes([2u8; 32]),
            user_prompt_hash: Blake3Hash::from_bytes([3u8; 32]),
            final_prompt_hash: Blake3Hash::from_bytes([4u8; 32]),
            sampling: SamplingParams {
                temperature: 0.0,
                top_p: None,
                max_tokens: 500,
                seed: Some(42),
                stop_sequences: vec![],
            },
            pii_strategy_applied: PiiStrategy::Strict,
            redaction_token_set: vec![],
            provider_region: Some(RegionCode::UsEast1),
            publisher_id: None,
            publisher_kyc_tier: None,
            gold_case_id: None,
        }
    }

    #[tokio::test]
    async fn call_with_provenance_happy_path_emits_record() {
        let collector = fresh_collector().await;
        let pre = sample_pre();
        let trace_id = pre.trace_id;
        let tenant = pre.tenant_id;

        let outcome = call_with_provenance::<_, _, &str>(
            collector.as_ref(),
            pre,
            None,
            |_pre| async move {
                // Simulate an LLM call returning "Hello, world!"
                let post = PostCallProvenance {
                    token_usage: TokenUsage {
                        client_estimated_input: 10,
                        client_estimated_output: 5,
                        provider_reported_input: Some(11),
                        provider_reported_output: Some(5),
                        is_estimated: false,
                    },
                    cost_accounting: CostAccounting {
                        cost_cents: 0.05,
                        prices_table_version: 1,
                        provider_quoted_cost_cents: None,
                        overflow_capped: false,
                    },
                    provider_request_id: Some("req_xyz".into()),
                    provider_response_time_ms: 234,
                    provider_finish_reason: FinishReason::Stop,
                    provider_metadata: Default::default(),
                    output_hash: Some(Blake3Hash::from_bytes([9u8; 32])),
                    completeness: Completeness::Complete,
                    safety_flags: vec![],
                    tool_calls: vec![],
                    model_hash_verified: false,
                };
                Ok(("Hello, world!", post))
            },
        )
        .await
        .unwrap();

        assert_eq!(outcome.response, "Hello, world!");
        assert_eq!(outcome.record.trace_id, trace_id);
        assert_eq!(outcome.record.tenant_id, tenant);
        // Verify the record landed in the store.
        let got = collector
            .store()
            .get_by_trace(trace_id, tenant)
            .await
            .unwrap();
        assert_eq!(got, outcome.record);
    }

    #[tokio::test]
    async fn call_with_provenance_short_circuits_on_hash_mismatch() {
        let store: Arc<dyn LlmProvenanceStore> =
            Arc::new(InMemoryLlmProvenanceStore::new());
        let registry = Arc::new(InMemoryModelRegistry::new());
        let identity = ModelIdentity::hosted("openai", "gpt-4", "2024");
        let mut m = RegisteredModel::hosted(identity.clone(), 1000.0, 3000.0, 1);
        m.expected_content_hash = Some(Blake3Hash::from_bytes([0xAAu8; 32]));
        registry.register(m).await.unwrap();
        let collector = DefaultLlmProvenanceCollector::with_metrics(
            store.clone(),
            registry as Arc<dyn ModelRegistry>,
            N17Metrics::new(),
        );
        let pre = sample_pre();
        let actual = Blake3Hash::from_bytes([0xBBu8; 32]); // mismatch
        let closure_called = std::sync::atomic::AtomicBool::new(false);
        let result = call_with_provenance::<_, _, &str>(
            collector.as_ref(),
            pre,
            Some(&actual),
            |_pre| async {
                closure_called.store(true, std::sync::atomic::Ordering::SeqCst);
                let post = PostCallProvenance {
                    token_usage: TokenUsage::default(),
                    cost_accounting: CostAccounting::default(),
                    provider_request_id: None,
                    provider_response_time_ms: 0,
                    provider_finish_reason: FinishReason::Unknown,
                    provider_metadata: Default::default(),
                    output_hash: None,
                    completeness: Completeness::Complete,
                    safety_flags: vec![],
                    tool_calls: vec![],
                    model_hash_verified: false,
                };
                Ok(("unused", post))
            },
        )
        .await;
        // The closure must NOT have been invoked — pre_call short-circuited.
        assert!(!closure_called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(matches!(
            result.unwrap_err(),
            ProvenanceError::ModelIntegrityViolation { .. }
        ));
    }

    #[tokio::test]
    async fn call_with_provenance_propagates_closure_errors() {
        let collector = fresh_collector().await;
        let pre = sample_pre();
        let result = call_with_provenance::<_, _, &str>(
            collector.as_ref(),
            pre,
            None,
            |_pre| async {
                Err(ProvenanceError::Internal {
                    message: "simulated".into(),
                })
            },
        )
        .await;
        assert!(matches!(
            result.unwrap_err(),
            ProvenanceError::Internal { .. }
        ));
    }

    #[tokio::test]
    async fn call_with_provenance_preserves_tenant_in_record() {
        let collector = fresh_collector().await;
        let pre = sample_pre();
        let tenant = pre.tenant_id;
        let outcome = call_with_provenance::<_, _, ()>(
            collector.as_ref(),
            pre,
            None,
            |_pre| async {
                Ok((
                    (),
                    PostCallProvenance {
                        token_usage: TokenUsage::default(),
                        cost_accounting: CostAccounting::default(),
                        provider_request_id: None,
                        provider_response_time_ms: 0,
                        provider_finish_reason: FinishReason::Unknown,
                        provider_metadata: Default::default(),
                        output_hash: None,
                        completeness: Completeness::Complete,
                        safety_flags: vec![],
                        tool_calls: vec![],
                        model_hash_verified: false,
                    },
                ))
            },
        )
        .await
        .unwrap();
        assert_eq!(outcome.record.tenant_id, tenant);
    }

    #[tokio::test]
    async fn call_with_provenance_carries_sampling_seed_through() {
        let collector = fresh_collector().await;
        let mut pre = sample_pre();
        pre.sampling.seed = Some(123_456_789);
        let outcome = call_with_provenance::<_, _, ()>(
            collector.as_ref(),
            pre,
            None,
            |_pre| async {
                Ok((
                    (),
                    PostCallProvenance {
                        token_usage: TokenUsage::default(),
                        cost_accounting: CostAccounting::default(),
                        provider_request_id: None,
                        provider_response_time_ms: 0,
                        provider_finish_reason: FinishReason::Unknown,
                        provider_metadata: Default::default(),
                        output_hash: None,
                        completeness: Completeness::Complete,
                        safety_flags: vec![],
                        tool_calls: vec![],
                        model_hash_verified: false,
                    },
                ))
            },
        )
        .await
        .unwrap();
        assert_eq!(outcome.record.sampling.seed, Some(123_456_789));
    }
}
