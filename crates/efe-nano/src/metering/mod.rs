//! Per-call usage metering for billable nano agent events.
//!
//! # Why a trait abstraction
//!
//! The architecture promises that every nano agent call increments
//! three `Meter` counters in M31 :
//! - `NanoAgentCalls` — every Mode A/B/C call
//! - `NanoAgentLlmTokens` — tokens consumed by Mode C
//! - `NanoAgentVmSeconds` — Mode B/C VM wall-clock seconds
//!
//! Coupling `efe-nano` directly to `efe-metering::MeterClient` is
//! avoided because :
//! 1. Constructing a `MeterClient` requires a full `Counter` backend
//!    (Postgres-backed in prod) — not testable in unit tests.
//! 2. The set of meters an integrator wants to track may grow (audit
//!    export GB, regional egress, etc.) — a trait scales.
//! 3. M31 may refactor its internal storage (batched writes, Redis
//!    counters) without touching M50.
//!
//! Sprint 5 ships three impls :
//! - [`NoopMeteringClient`] — production default when no billing wired.
//! - [`InMemoryMeteringClient`] — for tests, captures counts per meter.
//! - `EfeMeteringClient` (deployment-time) — wraps
//!   `efe_metering::MeterClient` and translates trait calls to
//!   `Meter::NanoAgentCalls / NanoAgentLlmTokens / NanoAgentVmSeconds`.
//!   The variants exist in `efe-metering::Meter` per decision G.5 ; the
//!   wiring is a 30-line deployment-time shim.

pub mod inmem;
pub mod noop;

pub use inmem::InMemoryMeteringClient;
pub use noop::NoopMeteringClient;

use async_trait::async_trait;
use efe_nano_sdk::TenantId;

// ─────────────────────────────────────────────────────────────────────
//  Trait
// ─────────────────────────────────────────────────────────────────────

/// Records usage events for billing.
///
/// Implementations must be `Send + Sync + 'static` so they can be shared
/// across the multi-task runtime.  All methods are async + `&self` so
/// they don't block the caller.
#[async_trait]
pub trait MeteringClient: Send + Sync + 'static {
    /// Record one nano agent call for the given `mode` (Mode A/B/C).
    /// Increments `Meter::NanoAgentCalls`.
    async fn record_call(&self, tenant: TenantId, mode: ExecutionModeKind);

    /// Record LLM tokens consumed (Mode C only).
    /// Increments `Meter::NanoAgentLlmTokens` by `tokens`.
    async fn record_llm_tokens(&self, tenant: TenantId, tokens: u64);

    /// Record VM wall-clock seconds (Mode B/C).
    /// Increments `Meter::NanoAgentVmSeconds` by `seconds`.  Use
    /// fractional seconds (e.g. 0.05 for 50 ms).
    async fn record_vm_seconds(&self, tenant: TenantId, seconds: f64);
}

/// Discriminator for [`MeteringClient::record_call`] — mirrors
/// [`efe_nano_sdk::ExecutionMode`] but is `Copy` + has no fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionModeKind {
    ModeA,
    ModeB,
    ModeC,
}

impl ExecutionModeKind {
    /// Convert from the SDK's full `ExecutionMode` enum.
    pub fn from_execution_mode(em: &efe_nano_sdk::ExecutionMode) -> Self {
        match em {
            efe_nano_sdk::ExecutionMode::PureRust { .. } => Self::ModeA,
            efe_nano_sdk::ExecutionMode::LocalModel { .. } => Self::ModeB,
            efe_nano_sdk::ExecutionMode::LlmAugmented { .. } => Self::ModeC,
        }
    }

    /// Static label for metric tags + logs.
    pub fn label(self) -> &'static str {
        match self {
            Self::ModeA => "mode_a",
            Self::ModeB => "mode_b",
            Self::ModeC => "mode_c",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn execution_mode_kind_from_pure_rust() {
        let em = efe_nano_sdk::ExecutionMode::PureRust {
            entry: "x".into(),
        };
        assert_eq!(ExecutionModeKind::from_execution_mode(&em), ExecutionModeKind::ModeA);
    }

    #[test]
    fn execution_mode_kind_from_local_model() {
        let em = efe_nano_sdk::ExecutionMode::LocalModel {
            model_id: "x".into(),
            model_version: "1.0.0".into(),
        };
        assert_eq!(ExecutionModeKind::from_execution_mode(&em), ExecutionModeKind::ModeB);
    }

    #[test]
    fn execution_mode_kind_from_llm_augmented() {
        let em = efe_nano_sdk::ExecutionMode::LlmAugmented {
            provider: "x".into(),
            model: "y".into(),
            prompt_template_id: "z".into(),
            max_tokens: 100,
            temperature: 0.1,
        };
        assert_eq!(ExecutionModeKind::from_execution_mode(&em), ExecutionModeKind::ModeC);
    }

    #[test]
    fn execution_mode_kind_label_distinct() {
        assert_eq!(ExecutionModeKind::ModeA.label(), "mode_a");
        assert_eq!(ExecutionModeKind::ModeB.label(), "mode_b");
        assert_eq!(ExecutionModeKind::ModeC.label(), "mode_c");
    }
}
