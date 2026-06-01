// SPDX-License-Identifier: UNLICENSED
//! # `efe-nano` — M50 Nano Agent Swarm Runtime (commercial)
//!
//! Private runtime for the EFE M50 nano agent swarm platform.
//! **Not customer-facing** — customers use the public
//! [`efe-nano-sdk`] crate.
//!
//! Phase 1 (Sprints 1-3) modules :
//! - [`catalog`] : `InMemoryCatalog` (Sprint 1).  Postgres backend lands Sprint 2.
//! - [`runtime`] : `PureRustRuntime` (Mode A) + `CallContext` (Sprint 1).
//! - [`audit`] : Stub writers (Sprint 1).  HMAC chain (M23) integration Sprint 6.
//! - [`dispatcher`] : DAG planner + parallel executor (Sprint 3).
//! - [`idempotency`] : In-process TTL cache (Sprint 1).  Backed by efe-cache (M27) in Sprint 2.
//! - [`error`] : Runtime error hierarchy.
//! - [`starter_pack`] : 10 EFE-shipped Mode A nano agents (Sprints 2-3).
//!
//! Phase 2 Sprint 4 additions :
//! - [`runtime::local_model`] : `LocalModelRuntime` (Mode B executor).
//! - [`runtime::model_loader`] : `ModelLoader` trait + 5 builtin models.
//! - [`runtime::pool`] : `VmPool` trait + `StubVmPool` (Sprint 8 swap to Firecracker).
//! - [`runtime::multi_mode`] : `MultiModeRuntime` composing Mode A + Mode B.
//! - 5 Mode B starter agents : `ner_clinical`, `ner_legal`,
//!   `intent_classify_generic`, `lang_detect`, `embed_text`.
//!
//! Phase 2 Sprint 5 additions :
//! - [`runtime::llm_augmented`] : `LlmAugmentedRuntime` (Mode C executor).
//! - [`runtime::llm_client`] : `LlmClient` trait + `MockLlmClient` + `BuiltinLlmClient`.
//! - [`runtime::output_validator`] : strict JSON schema enforcement + retry.
//! - [`runtime::pii_redactor`] : PII redaction adapter (regex + leak detection).
//! - [`budget`] : `BudgetEnforcer` trait + `InMemoryBudgetEnforcer` stub.
//! - [`metering`] : `MeteringClient` trait + InMemory + Noop impls.
//! - 5 Mode C starter agents : `summarize_strict`, `extract_structured`,
//!   `classify_topic_llm`, `sentiment_nuanced`, `qa_grounded`.
//! - SDK extension : `PiiStrategy` enum on `ComplianceTags`.
//!
//! Phase 2 Sprint 6 additions :
//! - [`audit::hmac_chain`] : `HmacChainAuditWriter` + per-tenant HMAC chain.
//! - [`audit::chain_store`] : `ChainStore` trait + `InMemoryChainStore`.
//! - [`audit::hkdf`] : `HmacKeySource` (env / static / Vault placeholder).
//! - [`audit::verify`] : `verify_chain` + `verify_subchain`.
//! - [`replay`] : `ReplayEngine` + `DeterminismChecker` + `SwarmReconstructor` + CLI.
//! - SQL migrations v004-v007 (production deployment).
//!
//! Phase 2+ modules (not in this file yet) :
//! - `billing` (Sprint 11 — real M31 integration).
//!
//! ## See also
//!
//! - [`docs/architecture/EFE_M50_PHASE_1_ARCHITECTURE.md`]
//! - [`docs/architecture/EFE_M50_AGENTIC_ARCHITECTURE.md`]

#![doc(html_root_url = "https://docs.rs/efe-nano/0.0.1")]
#![deny(unsafe_code)]

pub mod audit;
pub mod budget;
pub mod catalog;
pub mod dispatcher;
pub mod error;
pub mod idempotency;
pub mod metering;
pub mod replay;
pub mod runtime;
/// **Sprint 8 — N15** : fairness scheduling subsystem (admission gate,
/// DRR, token bucket, inflight tracker, metrics).
pub mod scheduler;
pub mod starter_pack;

// Re-export the SDK so internal modules can refer to shared types via `crate::sdk::`.
pub use efe_nano_sdk as sdk;

// ─────────────────────────────────────────────────────────────────────
//  Crate constants
// ─────────────────────────────────────────────────────────────────────

/// Runtime semver — track separately from public SDK `efe-nano-sdk`.
pub const RUNTIME_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Module-id this runtime belongs to in the EFE M-number registry.
pub const MODULE_ID: &str = "M50";

#[cfg(test)]
mod sprint_1_smoke_tests {
    use super::*;
    use crate::catalog::InMemoryCatalog;
    use crate::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use crate::starter_pack::register_all_starter_pack;
    use efe_nano_sdk::{Catalog, NanoAgentId, NanoAgentRef, TenantId};

    #[tokio::test]
    async fn full_phase_1_smoke() {
        // 1. Wire catalog + registry + runtime.
        let catalog = InMemoryCatalog::new();
        let registry = HandlerRegistry::new();
        let _runtime = PureRustRuntime::new(registry.clone());

        // 2. Register the starter pack.
        register_all_starter_pack(&catalog, &registry).await.unwrap();
        assert!(registry.len().await > 0);

        // 3. Retrieve json_validate from the catalog.
        let r = NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap());
        let agent = catalog.get(&r, TenantId::system()).await.unwrap();
        assert_eq!(agent.id.as_str(), "json_validate");

        // 4. The runtime can find the handler.
        assert!(registry.lookup("json_validate::run").await.is_some());
    }

    #[test]
    fn constants_present() {
        assert!(RUNTIME_VERSION.starts_with("0.0."));
        assert_eq!(MODULE_ID, "M50");
    }
}

#[cfg(test)]
mod sprint_4_smoke_tests {
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::audit::AuditWriter;
    use crate::catalog::InMemoryCatalog;
    use crate::idempotency::IdempotencyCache;
    use crate::runtime::model_loader::InProcessModelLoader;
    use crate::runtime::pool::stub::StubVmPool;
    use crate::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use crate::runtime::{
        LocalModelRuntime, MultiModeRuntime, NanoAgentExecutor,
    };
    use crate::starter_pack::register_all_starter_pack_with_mode_b;
    use efe_nano_sdk::{Catalog, NanoAgentId, NanoAgentRef, TenantId};
    use std::sync::Arc;

    /// End-to-end Phase 2 Sprint 4 smoke test :
    /// catalog + registry + Mode A runtime + Mode B runtime (with pool +
    /// model loader) + MultiModeRuntime + Dispatcher all wired together.
    #[tokio::test]
    async fn full_phase_2_sprint_4_smoke() {
        // 1. Catalog + Mode A registry.
        let catalog = Arc::new(InMemoryCatalog::new());
        let registry = HandlerRegistry::new();
        register_all_starter_pack_with_mode_b(&*catalog, &registry)
            .await
            .unwrap();
        assert_eq!(registry.len().await, 10); // 10 Mode A handlers ; Mode B has none

        // 2. Mode A runtime.
        let mode_a = Arc::new(PureRustRuntime::new(registry));

        // 3. Mode B : model loader + VM pool + runtime.
        let loader = Arc::new(InProcessModelLoader::with_phase_2_builtins());
        let pool = Arc::new(StubVmPool::new(
            crate::runtime::pool::stub::StubVmPoolConfig::default(),
            loader,
        ));
        let mode_b = Arc::new(LocalModelRuntime::new(pool));

        // 4. Multi-mode runtime.
        let multi: Arc<dyn NanoAgentExecutor> =
            Arc::new(MultiModeRuntime::mode_a_and_b(mode_a, mode_b));

        // 5. Spot check : the multi-mode dispatcher resolves both modes.
        let mode_a_agent = catalog
            .get(
                &NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
                TenantId::system(),
            )
            .await
            .unwrap();
        let mode_b_agent = catalog
            .get(
                &NanoAgentRef::new(NanoAgentId::new("ner_clinical").unwrap()),
                TenantId::system(),
            )
            .await
            .unwrap();

        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx_a = crate::runtime::CallContext::stand_alone(
            TenantId::system(),
            audit.clone(),
            5_000,
        );
        let ctx_b = crate::runtime::CallContext::stand_alone(
            TenantId::system(),
            audit,
            5_000,
        );

        let r_a = multi
            .execute_with_context(
                &mode_a_agent,
                serde_json::json!({
                    "value": {"x": 1},
                    "schema": {"type": "object"}
                }),
                &ctx_a,
            )
            .await
            .unwrap();
        assert_eq!(r_a.output["valid"], serde_json::Value::Bool(true));

        let r_b = multi
            .execute_with_context(
                &mode_b_agent,
                serde_json::json!({"text": "metformin for diabetes phase 3 trial"}),
                &ctx_b,
            )
            .await
            .unwrap();
        assert!(r_b.output["count"].as_u64().unwrap() >= 3);

        // 6. Dispatcher wires it all.
        let _dispatcher = crate::dispatcher::Dispatcher::new(
            catalog as Arc<dyn Catalog>,
            multi,
            Arc::new(InMemoryAuditWriter::new()) as Arc<dyn AuditWriter>,
            Arc::new(IdempotencyCache::new()),
        );
    }
}

#[cfg(test)]
mod sprint_5_smoke_tests {
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::audit::AuditWriter;
    use crate::catalog::InMemoryCatalog;
    use crate::idempotency::IdempotencyCache;
    use crate::runtime::pool::stub::StubVmPool;
    use crate::runtime::pool::VmPool;
    use crate::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use crate::runtime::{
        LlmAugmentedRuntime, LocalModelRuntime, MultiModeRuntime, NanoAgentExecutor,
    };
    use crate::starter_pack::{
        mode_c_output_schemas, register_all_starter_pack_with_mode_b_and_c,
    };
    use efe_nano_sdk::{Catalog, NanoAgentId, NanoAgentRef, TenantId};
    use std::sync::Arc;

    /// End-to-end Phase 2 Sprint 5 smoke test :
    /// catalog + Mode A handlers + Mode B model loader + Mode C LLM client
    /// + MultiModeRuntime A+B+C + Dispatcher.
    #[tokio::test]
    async fn full_phase_2_sprint_5_smoke() {
        // 1. Catalog with 20 agents.
        let catalog = Arc::new(InMemoryCatalog::new());
        let registry = HandlerRegistry::new();
        let count = register_all_starter_pack_with_mode_b_and_c(&*catalog, &registry)
            .await
            .unwrap();
        assert_eq!(count, 20);

        // 2. Pool with both loader + LLM client.
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());

        // 3. Three mode runtimes.
        let mode_a = Arc::new(PureRustRuntime::new(registry));
        let mode_b = Arc::new(LocalModelRuntime::new(
            Arc::clone(&pool) as Arc<dyn VmPool>,
        ));
        let mode_c = Arc::new(LlmAugmentedRuntime::with_phase_2_defaults(
            Arc::clone(&pool) as Arc<dyn VmPool>,
        ));

        // 4. Pre-warm output validator.
        mode_c
            .warm_output_validator(&mode_c_output_schemas())
            .unwrap();

        // 5. Multi-mode runtime composing all three.
        let multi: Arc<dyn NanoAgentExecutor> = Arc::new(
            MultiModeRuntime::mode_a_b_c(mode_a, mode_b, mode_c),
        );

        // 6. Spot-check Mode C invocation via multi-mode dispatcher.
        let mode_c_agent = catalog
            .get(
                &NanoAgentRef::new(NanoAgentId::new("summarize_strict").unwrap()),
                TenantId::system(),
            )
            .await
            .unwrap();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = crate::runtime::CallContext::stand_alone(
            TenantId::system(),
            audit,
            10_000,
        );
        let resp = multi
            .execute_with_context(
                &mode_c_agent,
                serde_json::json!({
                    "text": "First sentence. Second sentence.",
                    "max_sentences": 1
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert!(resp.output["summary"].is_string());
        assert_eq!(resp.output["sentence_count"], 1);

        // 7. Dispatcher wires it.
        let _dispatcher = crate::dispatcher::Dispatcher::new(
            catalog as Arc<dyn Catalog>,
            multi,
            Arc::new(InMemoryAuditWriter::new()) as Arc<dyn AuditWriter>,
            Arc::new(IdempotencyCache::new()),
        );
    }
}

#[cfg(test)]
mod sprint_6_smoke_tests {
    use crate::audit::chain_store::InMemoryChainStore;
    use crate::audit::hkdf::HmacKeySource;
    use crate::audit::hmac_chain::InMemoryHmacChainAuditWriter;
    use crate::audit::verify::verify_chain;
    use crate::audit::AuditWriter;
    use crate::catalog::InMemoryCatalog;
    use crate::idempotency::IdempotencyCache;
    use crate::replay::ReplayEngine;
    use crate::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use crate::runtime::NanoAgentExecutor;
    use crate::starter_pack::register_all_starter_pack;
    use efe_nano_sdk::{
        Catalog, NanoAgentId, NanoAgentRef, Operation, OperationInput, RetryPolicy, SwarmRequest,
        SwarmStatus, TenantId,
    };
    use std::sync::Arc;
    use uuid::Uuid;

    /// Sprint 6 end-to-end : dispatch → HMAC chain → verify → replay → match.
    #[tokio::test]
    async fn full_phase_2_sprint_6_smoke() {
        // 1. Catalog + Mode A runtime.
        let catalog = Arc::new(InMemoryCatalog::new());
        let registry = HandlerRegistry::new();
        register_all_starter_pack(&*catalog, &registry).await.unwrap();
        let runtime: Arc<dyn NanoAgentExecutor> = Arc::new(PureRustRuntime::new(registry));

        // 2. HMAC chain audit writer + chain store.
        let chain_store = Arc::new(InMemoryChainStore::new());
        let key_source = HmacKeySource::test_key();
        let writer = Arc::new(InMemoryHmacChainAuditWriter::new(
            Arc::clone(&chain_store),
            key_source.clone(),
        ));

        // 3. Dispatcher wires it all together.
        let dispatcher = Arc::new(crate::dispatcher::Dispatcher::new(
            Arc::clone(&catalog) as Arc<dyn Catalog>,
            runtime,
            Arc::clone(&writer) as Arc<dyn AuditWriter>,
            Arc::new(IdempotencyCache::new()),
        ));

        // 4. Dispatch a small Mode A swarm to populate the chain.
        //    All starter-pack agents use `AuditPolicy::default()` which
        //    already has `include_input = include_output = true`, so
        //    `full_input` / `full_output` flow into the body store
        //    automatically — no per-test override needed.
        let tenant = TenantId::system();
        let request_id = Uuid::new_v4();

        let request = SwarmRequest {
            request_id,
            tenant_id: tenant,
            caller_agent_id: Some("smoke".into()),
            operations: vec![Operation {
                op_id: "v".into(),
                agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
                input: OperationInput::Literal {
                    value: serde_json::json!({
                        "value": {"x": 42},
                        "schema": {"type": "object"}
                    }),
                },
                depends_on: vec![],
                timeout_ms: None,
                retry: RetryPolicy::default(),
            }],
            max_parallelism: 1,
            timeout_ms: 30_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec![],
            cost_cap_cents: None,
            parent_envelope_trace_id: None,
        };
        let resp = dispatcher.dispatch(request).await.unwrap();
        assert_eq!(resp.overall_status, SwarmStatus::Success);

        // 5. Verify the chain integrity.
        let report = verify_chain(&*chain_store, &key_source, tenant, None)
            .await
            .unwrap();
        assert!(report.entries_verified >= 1);

        // 6. Replay the swarm.
        let engine = ReplayEngine::new(
            Arc::clone(&chain_store),
            key_source,
            Arc::clone(&dispatcher),
            catalog as Arc<dyn Catalog>,
        );
        let replay_report = engine.replay(tenant, request_id).await.unwrap();
        assert!(replay_report.chain_verified);
        assert_eq!(replay_report.total_ops, 1);
    }
}
