// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23** : `NanoRunner` — the thin per-agent
//! execution trait the adapter wraps.
//!
//! ## Why a custom trait and not `efe_nano_sdk::traits::runtime::Runtime`
//!
//! The SDK `Runtime` trait is intentionally *one runtime, many
//! agents* — it carries dispatch logic that walks the
//! `ExecutionMode` (Mode A pure-Rust / Mode B local model / Mode
//! C LLM-augmented) and decides which inner executor handles the
//! given catalogue entry.  That model fits the existing
//! `MultiModeRuntime` in `efe-nano` but adds heavy plumbing
//! (`AuditWriter`, `CallContext`, `SwarmCostTracker`) that the
//! gateway and adapter do not need.
//!
//! The adapter needs the *opposite* shape : **one trait, one
//! agent** — a runner per catalogue entry whose only job is to
//! produce a `NanoAgentResponse` (cost + latency + output trace)
//! given a `SwarmAgentContext`.  This keeps the adapter
//! independent of `efe-nano` and lets each pre-analysis crate
//! ship trivially-thin runners.
//!
//! Authors of new runners typically :
//! 1. Already implement `SwarmAgent` for their agent struct ;
//! 2. Wrap that impl in a `NanoRunner` that reads the same
//!    blackboard pointers, dispatches the same work, and reports
//!    `(cost_cents = 0.0, blackboard_writes, events_published)`.
//!
//! Sprint 16+ may bridge `MultiModeRuntime` here via a
//! `MultiModeNanoRunner` shim so tenants who publish nanos
//! through the M50 catalogue pick up the full audit stack.

use async_trait::async_trait;
use efe_nano_sdk::{NanoAgent, TraceId};
use efe_swarm_orchestrator::SwarmAgentContext;

use crate::error::AdapterError;

/// **N23** : the per-dispatch metadata returned by a runner.
///
/// The runner is responsible for reading from + writing to the
/// blackboard ; this struct captures the audit + cost metadata
/// the adapter needs to build the `nano_envelope_v1` event and
/// the `AgentOutcome`.
#[derive(Debug, Clone)]
pub struct NanoRunnerOutcome {
    /// Runner-reported cost in cents.  Pure-Rust runners report
    /// `0.0` ; Mode B/C runners report the actual provider cost.
    pub cost_cents: f64,
    /// Number of blackboard writes the runner performed.  Adapter
    /// adds one for the `/nano/<id>/output` audit row plus a few
    /// more for `/cost`, `/trace`, etc.
    pub blackboard_writes: u32,
    /// Events the runner published on the bus.  The adapter adds
    /// one (the envelope itself).
    pub events_published: u32,
    /// Internal trace id for this dispatch.  Defaults to a fresh
    /// `TraceId::new()` if the runner does not surface one.
    pub trace_id: TraceId,
    /// `true` if the runner short-circuited via an inner cache.
    /// Note : this is the runner's own cache.  The adapter's
    /// idempotency cache (G5) is independent.
    pub cache_hit: bool,
}

impl Default for NanoRunnerOutcome {
    fn default() -> Self {
        Self {
            cost_cents: 0.0,
            blackboard_writes: 0,
            events_published: 0,
            trace_id: TraceId::new(),
            cache_hit: false,
        }
    }
}

impl NanoRunnerOutcome {
    /// **N23** : minimal builder — `cost_cents = 0.0`, no blackboard
    /// writes, no events, fresh `trace_id`, no cache hit.
    pub fn fresh() -> Self {
        Self::default()
    }

    /// **N23** : fluent helper.
    pub fn with_cost_cents(mut self, c: f64) -> Self {
        self.cost_cents = c;
        self
    }

    /// **N23** : fluent helper.
    pub fn with_blackboard_writes(mut self, n: u32) -> Self {
        self.blackboard_writes = n;
        self
    }

    /// **N23** : fluent helper.
    pub fn with_events_published(mut self, n: u32) -> Self {
        self.events_published = n;
        self
    }

    /// **N23** : fluent helper.
    pub fn with_trace_id(mut self, t: TraceId) -> Self {
        self.trace_id = t;
        self
    }

    /// **N23** : fluent helper.
    pub fn with_cache_hit(mut self, hit: bool) -> Self {
        self.cache_hit = hit;
        self
    }
}

/// **N23** : per-agent execution contract.
///
/// Implementations are Sprint 15+ thin wrappers around existing
/// pre-analysis agents (or future tenant-published nanos).  The
/// runner is responsible for :
///
/// - reading its input from the blackboard at the canonical
///   pointer (typically `/input/text` or `/input`) ;
/// - performing the work (regex extraction, model inference, LLM
///   call) ;
/// - writing the output to the blackboard at the canonical
///   pointer (e.g. `/entities`, `/language`, `/summary`) ;
/// - returning [`NanoRunnerOutcome`] with the actual cost and
///   blackboard write count.
///
/// On failure the runner returns `AdapterError`.  The adapter
/// translates this to either a soft `AgentOutcome::Errored` (for
/// signature / gold / dispatch faults) or a hard
/// `SwarmAgentError` (for blackboard write faults).
#[async_trait]
pub trait NanoRunner: Send + Sync + 'static {
    /// **N23** : execute the runner inside the given swarm context.
    ///
    /// The runner has read-write access to `ctx.blackboard` and
    /// read access to `ctx.input`, `ctx.eventbus`, and
    /// `ctx.rng_seed`.  Implementations that need randomness MUST
    /// use [`crate::current_seed`] for replay (I9).
    async fn run(
        &self,
        nano: &NanoAgent,
        ctx: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError>;
}

/// **N23** : `Arc<dyn NanoRunner>` blanket via deref forwarding so
/// callers can pass `Arc<Concrete>` where `Arc<dyn NanoRunner>` is
/// expected.
#[async_trait]
impl<T: NanoRunner + ?Sized> NanoRunner for std::sync::Arc<T> {
    async fn run(
        &self,
        nano: &NanoAgent,
        ctx: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        (**self).run(nano, ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_default_is_zero_cost() {
        let o = NanoRunnerOutcome::default();
        assert_eq!(o.cost_cents, 0.0);
        assert_eq!(o.blackboard_writes, 0);
        assert!(!o.cache_hit);
    }

    #[test]
    fn outcome_fluent_builder() {
        let t = TraceId::new();
        let o = NanoRunnerOutcome::fresh()
            .with_cost_cents(0.42)
            .with_blackboard_writes(2)
            .with_events_published(1)
            .with_trace_id(t)
            .with_cache_hit(true);
        assert!((o.cost_cents - 0.42).abs() < f64::EPSILON);
        assert_eq!(o.blackboard_writes, 2);
        assert_eq!(o.events_published, 1);
        assert_eq!(o.trace_id, t);
        assert!(o.cache_hit);
    }

    #[test]
    fn outcome_fresh_produces_unique_trace_ids() {
        let o1 = NanoRunnerOutcome::fresh();
        let o2 = NanoRunnerOutcome::fresh();
        assert_ne!(o1.trace_id, o2.trace_id);
    }
}
