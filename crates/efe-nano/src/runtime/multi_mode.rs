//! Multi-mode runtime — dispatches per-agent based on `execution_mode`.
//!
//! Wraps an `Arc<PureRustRuntime>` (always present, Phase 1 inheritance)
//! and an optional `Arc<LocalModelRuntime>` (Phase 2 Sprint 4 addition).
//!
//! In Sprint 5 we'll add a third optional component for Mode C
//! (`Arc<LlmAugmentedRuntime>`).  For now, requests for Mode C surface
//! [`NanoAgentError::ModeNotWired`].
//!
//! This struct is the **default executor injected into the dispatcher**
//! starting Phase 2.  Phase 1's dispatcher takes `Arc<PureRustRuntime>`
//! ; Phase 2's dispatcher takes `Arc<dyn NanoAgentExecutor>`.

use async_trait::async_trait;
use efe_nano_sdk::{ExecutionMode, NanoAgent};
use std::sync::Arc;

use crate::error::NanoAgentError;
use crate::runtime::context::CallContext;
use crate::runtime::llm_augmented::LlmAugmentedRuntime;
use crate::runtime::local_model::LocalModelRuntime;
use crate::runtime::pure_rust::PureRustRuntime;
use crate::runtime::{NanoAgentExecutor, NanoAgentResponse};

/// Multi-mode runtime composed of per-mode executors.
pub struct MultiModeRuntime {
    mode_a: Arc<PureRustRuntime>,
    mode_b: Option<Arc<LocalModelRuntime>>,
    mode_c: Option<Arc<LlmAugmentedRuntime>>,
}

impl MultiModeRuntime {
    /// Construct with only Mode A wired (Phase 1 compat mode).
    pub fn mode_a_only(mode_a: Arc<PureRustRuntime>) -> Self {
        Self {
            mode_a,
            mode_b: None,
            mode_c: None,
        }
    }

    /// Construct with Mode A + Mode B wired (Phase 2 Sprint 4 ship).
    pub fn mode_a_and_b(
        mode_a: Arc<PureRustRuntime>,
        mode_b: Arc<LocalModelRuntime>,
    ) -> Self {
        Self {
            mode_a,
            mode_b: Some(mode_b),
            mode_c: None,
        }
    }

    /// Construct with Mode A + Mode B + Mode C wired (Phase 2 Sprint 5
    /// ship — the full GA configuration).
    pub fn mode_a_b_c(
        mode_a: Arc<PureRustRuntime>,
        mode_b: Arc<LocalModelRuntime>,
        mode_c: Arc<LlmAugmentedRuntime>,
    ) -> Self {
        Self {
            mode_a,
            mode_b: Some(mode_b),
            mode_c: Some(mode_c),
        }
    }

    /// Builder : start with Mode A, add Mode B / Mode C optionally.
    pub fn builder(mode_a: Arc<PureRustRuntime>) -> MultiModeBuilder {
        MultiModeBuilder {
            mode_a,
            mode_b: None,
            mode_c: None,
        }
    }

    /// Whether Mode B is wired.
    pub fn has_mode_b(&self) -> bool {
        self.mode_b.is_some()
    }

    /// Whether Mode C is wired (Sprint 5 addition).
    pub fn has_mode_c(&self) -> bool {
        self.mode_c.is_some()
    }

    /// Health snapshot across all wired modes.
    pub async fn health(&self) -> MultiModeHealth {
        MultiModeHealth {
            mode_a_alive: true,
            mode_b_alive: self.mode_b.is_some(),
            mode_c_alive: self.mode_c.is_some(),
        }
    }
}

/// Builder pattern for [`MultiModeRuntime`].
pub struct MultiModeBuilder {
    mode_a: Arc<PureRustRuntime>,
    mode_b: Option<Arc<LocalModelRuntime>>,
    mode_c: Option<Arc<LlmAugmentedRuntime>>,
}

impl MultiModeBuilder {
    pub fn with_mode_b(mut self, mode_b: Arc<LocalModelRuntime>) -> Self {
        self.mode_b = Some(mode_b);
        self
    }

    pub fn with_mode_c(mut self, mode_c: Arc<LlmAugmentedRuntime>) -> Self {
        self.mode_c = Some(mode_c);
        self
    }

    pub fn build(self) -> MultiModeRuntime {
        MultiModeRuntime {
            mode_a: self.mode_a,
            mode_b: self.mode_b,
            mode_c: self.mode_c,
        }
    }
}

#[async_trait]
impl NanoAgentExecutor for MultiModeRuntime {
    async fn execute_with_context(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse, NanoAgentError> {
        match &agent.execution_mode {
            ExecutionMode::PureRust { .. } => {
                self.mode_a
                    .execute_with_context(agent, input, ctx)
                    .await
            }
            ExecutionMode::LocalModel { .. } => {
                let runtime = self.mode_b.as_ref().ok_or_else(|| {
                    NanoAgentError::ModeNotWired("local_model".to_string())
                })?;
                runtime.execute_with_context(agent, input, ctx).await
            }
            ExecutionMode::LlmAugmented { .. } => {
                let runtime = self.mode_c.as_ref().ok_or_else(|| {
                    NanoAgentError::ModeNotWired("llm_augmented".to_string())
                })?;
                runtime.execute_with_context(agent, input, ctx).await
            }
        }
    }
}

/// Health summary across wired modes.
#[derive(Debug, Clone)]
pub struct MultiModeHealth {
    pub mode_a_alive: bool,
    pub mode_b_alive: bool,
    pub mode_c_alive: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::audit::AuditWriter;
    use crate::runtime::model_loader::InProcessModelLoader;
    use crate::runtime::pool::stub::StubVmPool;
    use crate::runtime::pure_rust::HandlerRegistry;
    use efe_nano_sdk::{
        Author, AuditPolicy, Category, ComplianceTags, DeterminismLevel, IdempotencyPolicy,
        NanoAgentId, NanoAgentVersion, PiiStrategy, ResourceLimits, StabilityTier, TenantId,
    };
    use serde_json::json;

    async fn make_runtime_mode_a_only() -> Arc<MultiModeRuntime> {
        let reg = HandlerRegistry::new();
        reg.register("echo::run", Ok).await;
        Arc::new(MultiModeRuntime::mode_a_only(Arc::new(
            PureRustRuntime::new(reg),
        )))
    }

    async fn make_runtime_mode_a_and_b() -> Arc<MultiModeRuntime> {
        let reg = HandlerRegistry::new();
        reg.register("echo::run", Ok).await;
        let pool = Arc::new(StubVmPool::new(
            crate::runtime::pool::stub::StubVmPoolConfig::default(),
            Arc::new(InProcessModelLoader::with_phase_2_builtins()),
        ));
        Arc::new(MultiModeRuntime::mode_a_and_b(
            Arc::new(PureRustRuntime::new(reg)),
            Arc::new(LocalModelRuntime::new(pool)),
        ))
    }

    fn make_mode_a_agent() -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("echo_test").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "echo".into(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "echo::run".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
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
            created_at: chrono::Utc::now(),
        }
    }

    fn make_mode_b_agent() -> NanoAgent {
        let mut a = make_mode_a_agent();
        a.execution_mode = ExecutionMode::LocalModel {
            model_id: "ner_clinical_v1".into(),
            model_version: "1.0.0".into(),
        };
        a.input_schema = json!({
            "type": "object",
            "required": ["text"],
            "properties": { "text": { "type": "string" } }
        });
        a.output_schema = json!({
            "type": "object",
            "required": ["entities", "count"],
            "properties": {
                "entities": { "type": "array" },
                "count": { "type": "integer" }
            }
        });
        a.execution_mode = ExecutionMode::LocalModel {
            model_id: "ner_clinical_v1".into(),
            model_version: "1.0.0".into(),
        };
        a.limits.wall_clock_ms = 5_000;
        a
    }

    fn make_mode_c_agent() -> NanoAgent {
        let mut a = make_mode_a_agent();
        a.execution_mode = ExecutionMode::LlmAugmented {
            provider: "anthropic".into(),
            model: "claude-sonnet".into(),
            prompt_template_id: "test.v1".into(),
            max_tokens: 100,
            temperature: 0.1,
        };
        a
    }

    async fn ctx(audit: Arc<dyn AuditWriter>) -> CallContext {
        CallContext::stand_alone(TenantId::system(), audit, 5_000)
    }

    #[tokio::test]
    async fn dispatches_mode_a_to_pure_rust() {
        let runtime = make_runtime_mode_a_only().await;
        let agent = make_mode_a_agent();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let c = ctx(audit).await;

        let resp = runtime
            .execute_with_context(&agent, json!({"hello": "world"}), &c)
            .await
            .unwrap();
        assert_eq!(resp.output, json!({"hello": "world"}));
    }

    #[tokio::test]
    async fn dispatches_mode_b_to_local_model_runtime() {
        let runtime = make_runtime_mode_a_and_b().await;
        let agent = make_mode_b_agent();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let c = ctx(audit).await;

        let resp = runtime
            .execute_with_context(
                &agent,
                json!({"text": "metformin for diabetes phase 3 trial"}),
                &c,
            )
            .await
            .unwrap();
        let entities = resp.output["entities"].as_array().unwrap();
        assert!(!entities.is_empty());
    }

    #[tokio::test]
    async fn mode_b_not_wired_returns_mode_not_wired() {
        let runtime = make_runtime_mode_a_only().await;
        let agent = make_mode_b_agent();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let c = ctx(audit).await;

        let err = runtime
            .execute_with_context(&agent, json!({"text": "x"}), &c)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::ModeNotWired(m) => assert_eq!(m, "local_model"),
            other => panic!("expected ModeNotWired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mode_c_not_wired_returns_mode_not_wired() {
        // mode_a_and_b runtime doesn't wire Mode C → ModeNotWired.
        let runtime = make_runtime_mode_a_and_b().await;
        let agent = make_mode_c_agent();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let c = ctx(audit).await;

        let err = runtime
            .execute_with_context(&agent, json!({}), &c)
            .await
            .unwrap_err();
        match err {
            NanoAgentError::ModeNotWired(m) => assert_eq!(m, "llm_augmented"),
            other => panic!("expected ModeNotWired, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatches_mode_c_to_llm_augmented_runtime_when_wired() {
        let reg = HandlerRegistry::new();
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());
        let mode_b = Arc::new(LocalModelRuntime::new(Arc::clone(&pool) as Arc<dyn crate::runtime::pool::VmPool>));
        let mode_c = Arc::new(LlmAugmentedRuntime::with_phase_2_defaults(
            Arc::clone(&pool) as Arc<dyn crate::runtime::pool::VmPool>,
        ));
        let runtime: Arc<dyn NanoAgentExecutor> = Arc::new(MultiModeRuntime::mode_a_b_c(
            Arc::new(PureRustRuntime::new(reg)),
            mode_b,
            mode_c,
        ));
        let agent = make_mode_c_agent_real();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let c = ctx(audit).await;

        let resp = runtime
            .execute_with_context(
                &agent,
                json!({"text": "Hello. World.", "max_sentences": 1}),
                &c,
            )
            .await
            .unwrap();
        // Builtin LLM returns at least one sentence summary.
        assert!(resp.output["summary"].is_string());
    }

    /// Real Mode C agent with valid input/output schemas (for the
    /// dispatches_mode_c_to_llm_augmented_runtime_when_wired test).
    fn make_mode_c_agent_real() -> NanoAgent {
        let mut a = make_mode_a_agent();
        a.execution_mode = ExecutionMode::LlmAugmented {
            provider: "builtin".into(),
            model: "deterministic-v1".into(),
            prompt_template_id: "m50.summarize_strict.v1".into(),
            max_tokens: 200,
            temperature: 0.1,
        };
        a.input_schema = json!({
            "type": "object",
            "required": ["text", "max_sentences"],
            "properties": {
                "text": {"type": "string"},
                "max_sentences": {"type": "integer"}
            }
        });
        a.output_schema = json!({
            "type": "object",
            "required": ["summary", "sentence_count"],
            "properties": {
                "summary": {"type": "string"},
                "sentence_count": {"type": "integer"}
            }
        });
        a.limits.wall_clock_ms = 10_000;
        a.compliance.pii_redaction_strategy = PiiStrategy::PreLlmRedact;
        a
    }

    #[tokio::test]
    async fn builder_with_mode_c_works() {
        let reg = HandlerRegistry::new();
        let pool = Arc::new(StubVmPool::with_phase_2_defaults_and_llm());
        let mode_b = Arc::new(LocalModelRuntime::new(
            Arc::clone(&pool) as Arc<dyn crate::runtime::pool::VmPool>,
        ));
        let mode_c = Arc::new(LlmAugmentedRuntime::with_phase_2_defaults(
            Arc::clone(&pool) as Arc<dyn crate::runtime::pool::VmPool>,
        ));
        let runtime = MultiModeRuntime::builder(Arc::new(PureRustRuntime::new(reg)))
            .with_mode_b(mode_b)
            .with_mode_c(mode_c)
            .build();
        assert!(runtime.has_mode_b());
        assert!(runtime.has_mode_c());
        let h = runtime.health().await;
        assert!(h.mode_a_alive);
        assert!(h.mode_b_alive);
        assert!(h.mode_c_alive);
    }

    #[tokio::test]
    async fn builder_pattern_works() {
        let reg = HandlerRegistry::new();
        let pool = Arc::new(StubVmPool::with_phase_2_defaults());
        let mode_b = Arc::new(LocalModelRuntime::new(pool));
        let runtime = MultiModeRuntime::builder(Arc::new(PureRustRuntime::new(reg)))
            .with_mode_b(mode_b)
            .build();
        assert!(runtime.has_mode_b());
    }

    #[tokio::test]
    async fn health_reports_correct_modes() {
        let r = make_runtime_mode_a_only().await;
        let h = r.health().await;
        assert!(h.mode_a_alive);
        assert!(!h.mode_b_alive);
        assert!(!h.mode_c_alive);

        let r2 = make_runtime_mode_a_and_b().await;
        let h2 = r2.health().await;
        assert!(h2.mode_a_alive);
        assert!(h2.mode_b_alive);
        assert!(!h2.mode_c_alive);
    }
}
