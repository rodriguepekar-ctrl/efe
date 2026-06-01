//! Runtime executors.
//!
//! Phase 1 ships [`pure_rust::PureRustRuntime`] (Mode A).
//! Phase 2 Sprint 4 adds [`local_model::LocalModelRuntime`] (Mode B) and
//! the multi-mode dispatcher [`multi_mode::MultiModeRuntime`].
//! Phase 2 Sprint 5 adds [`llm_augmented::LlmAugmentedRuntime`] (Mode C) +
//! [`llm_client`], [`output_validator`], [`pii_redactor`] supporting modules.
//! All three runtime executors emit billing meters via the
//! [`crate::metering::MeteringClient`] trait.
//!
//! The dispatcher (and any other caller) interacts with a runtime via the
//! [`NanoAgentExecutor`] trait — this is the per-mode dispatch abstraction
//! that decouples [`crate::dispatcher::Dispatcher`] from any specific runtime
//! type.  Both [`PureRustRuntime`] and [`multi_mode::MultiModeRuntime`]
//! implement it.

pub mod context;
/// **Sprint 8 — N13** : dual-ledger budget router for shadow / canary cost.
pub mod cost_attribution;
pub mod llm_augmented;
pub mod llm_client;
pub mod local_model;
pub mod model_loader;
pub mod multi_mode;
pub mod output_validator;
pub mod pii_redactor;
pub mod pool;
/// **Sprint 7 — N8** : prompt-injection detection for Mode C inputs.
pub mod prompt_injection;
pub mod pure_rust;

pub use context::CallContext;
pub use llm_client::{
    BuiltinLlmClient, LlmCallRequest, LlmCallResponse, LlmClient, MockLlmClient, MockResponse,
    ResponseFormat,
};
pub use llm_augmented::LlmAugmentedRuntime;
pub use local_model::LocalModelRuntime;
pub use model_loader::{
    builtin, InProcessModelLoader, LocalModel, ModelFamily, ModelLoader, ModelLoaderHealth,
};
pub use multi_mode::MultiModeRuntime;
pub use output_validator::{OutputValidator, RetryPolicy};
pub use pii_redactor::{
    contains_redaction_tokens, MockPiiBackend, PiiBackend, PiiRedactor, RegexPiiBackend,
    SharedPiiBackend,
};
pub use pool::{StubVmPool, VmKind, VmLease, VmPool, VmPoolHealth, VmRequest, VmResponse};
pub use prompt_injection::{
    InjectionSeverity, InjectionSignal, InjectionVerdict, PromptInjectionFilter,
    RegexPromptInjectionFilter,
};
pub use pure_rust::PureRustRuntime;

// Re-export the SDK trait for convenience.
pub use efe_nano_sdk::traits::runtime::{
    NanoAgentResponse, PoolHealth, Runtime, RuntimeHealth, RuntimeResult, TraceId,
};

use async_trait::async_trait;
use efe_nano_sdk::NanoAgent;

use crate::error::NanoAgentError;

/// Internal multi-mode execution trait.
///
/// The dispatcher and tests interact with runtimes via this trait so that
/// Mode A only, Mode A+B, or Mode A+B+C can all be plugged in interchangeably.
///
/// Phase 1's `PureRustRuntime` impls this directly (Mode A only path).
/// Phase 2 Sprint 4's `MultiModeRuntime` impls this by composition of an
/// inner `PureRustRuntime` + an inner `LocalModelRuntime`.
#[async_trait]
pub trait NanoAgentExecutor: Send + Sync + 'static {
    /// Execute a single nano agent given its catalog entry, input value,
    /// and call context.
    ///
    /// Implementations dispatch internally based on `agent.execution_mode`.
    async fn execute_with_context(
        &self,
        agent: &NanoAgent,
        input: serde_json::Value,
        ctx: &CallContext,
    ) -> Result<NanoAgentResponse, NanoAgentError>;
}
