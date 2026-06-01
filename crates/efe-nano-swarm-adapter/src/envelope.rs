// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + G8 + I8** : `nano.envelope.v1` audit event
//! schema.
//!
//! Every adapter dispatch emits exactly one [`NanoEnvelopeV1`] on
//! the swarm `EventBus` (topic [`crate::ENVELOPE_TOPIC`]).  The
//! schema is **wire-stable** : Sprint 15+ MUST NOT remove or
//! rename fields.  Additions are allowed with `#[serde(default)]`.
//!
//! The envelope is the single audit primitive that links a nano
//! dispatch back to :
//! - the catalogue entry (`nano_id`, `nano_version`) for trace
//!   forensics ;
//! - the orchestrator (`swarm_id`, `agent_id`) for swarm-wide
//!   joins ;
//! - the cost reconciliation (estimate + actual + variance) for
//!   the finance SLO ;
//! - the compliance gates fired (`signature_verified`,
//!   `gold_bound`) for the EU audit officer.
//!
//! Per F9, when the EventBus is saturated (no live subscribers OR
//! all subscribers behind capacity) the adapter persists the
//! envelope to `/nano/<agent_id>/envelope` on the blackboard so
//! the report still surfaces the trail.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// **N23** : wire schema version literal.  Bump on breaking
/// schema changes (which the audit log will detect via the
/// `schema` field).
pub const SCHEMA_LITERAL: &str = "nano.envelope.v1";

/// **N23** : per-dispatch outcome label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NanoEnvelopeOutcome {
    /// Adapter dispatched the runner successfully.
    Completed,
    /// Adapter dispatched the runner and it returned a soft error.
    Errored,
    /// Adapter detected a timeout from the wrapped runner.
    TimedOut,
    /// Adapter declined to dispatch (predicate or precondition
    /// unsatisfied).  Note : N21's `AgentOutcome::Skipped` comes
    /// from the DAG predicate evaluation upstream of the adapter ;
    /// the adapter itself only emits `Skipped` if its runner
    /// returned `AdapterError::Skipped(_)`.
    Skipped,
}

impl NanoEnvelopeOutcome {
    /// **N23** : stable label for metrics + log search.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Errored => "errored",
            Self::TimedOut => "timed_out",
            Self::Skipped => "skipped",
        }
    }
}

/// **N23 + G8** : `nano.envelope.v1` — the per-dispatch audit
/// event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NanoEnvelopeV1 {
    /// Wire schema literal — always `"nano.envelope.v1"`.
    /// Sealed with `#[serde(default = "default_schema")]` so a
    /// future schema rev that drops this field still deserialises
    /// older payloads.
    #[serde(default = "default_schema")]
    pub schema: String,

    /// The `AgentId` of the wrapped node, as produced by the
    /// `SwarmAgent::agent_id()` method on `NanoToSwarm`.  Today
    /// it equals the catalogue `NanoAgentId.as_str()` (verbatim
    /// AgentId derivation per ADR-6, simplified for Sprint 15 to
    /// preserve the existing DAG topology).
    pub agent_id: String,

    /// The catalogue `NanoAgentId` as a string.
    pub nano_id: String,

    /// The catalogue `NanoAgentVersion` as a string (`"1.0.0"`).
    pub nano_version: String,

    /// The orchestrator's `SwarmId` (UUID v4).
    pub swarm_id: Uuid,

    /// W3C trace id surface (the adapter inherits or generates a
    /// fresh `Uuid::v4`).
    pub trace_id: Uuid,

    /// `ExecutionMode::mode_str()` — `"pure_rust"` /
    /// `"local_model"` / `"llm_augmented"`.
    pub mode: String,

    /// Catalogue's `cost_estimate_cents` translated to microcents.
    pub cost_estimate_microcents: i64,

    /// Runner's actual `cost_cents` translated to microcents.
    pub cost_actual_microcents: i64,

    /// `actual - estimate` ; can be negative when the runner
    /// underran the estimate.
    pub cost_variance_microcents: i64,

    /// `true` if the response came from the idempotency cache.
    pub cache_hit: bool,

    /// `true` if the signature gate fired and verified.  `false`
    /// when policy didn't require it or no signature is attached.
    pub signature_verified: bool,

    /// `true` if the gold binding gate fired and the catalogue
    /// `gold_set_ref` resolved to an `Approved` status.
    pub gold_bound: bool,

    /// Wall-clock latency of the adapter dispatch in milliseconds.
    pub latency_ms: u32,

    /// Per-outcome label.
    pub outcome: NanoEnvelopeOutcome,

    /// UTC emission timestamp.
    pub emitted_at: DateTime<Utc>,
}

fn default_schema() -> String {
    SCHEMA_LITERAL.to_string()
}

impl NanoEnvelopeV1 {
    /// **N23** : minimal constructor for tests + the adapter.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        agent_id: impl Into<String>,
        nano_id: impl Into<String>,
        nano_version: impl Into<String>,
        swarm_id: Uuid,
        trace_id: Uuid,
        mode: impl Into<String>,
        cost_estimate_microcents: i64,
        cost_actual_microcents: i64,
        cache_hit: bool,
        signature_verified: bool,
        gold_bound: bool,
        latency_ms: u32,
        outcome: NanoEnvelopeOutcome,
    ) -> Self {
        Self {
            schema: SCHEMA_LITERAL.to_string(),
            agent_id: agent_id.into(),
            nano_id: nano_id.into(),
            nano_version: nano_version.into(),
            swarm_id,
            trace_id,
            mode: mode.into(),
            cost_estimate_microcents,
            cost_actual_microcents,
            cost_variance_microcents: cost_actual_microcents
                .saturating_sub(cost_estimate_microcents),
            cache_hit,
            signature_verified,
            gold_bound,
            latency_ms,
            outcome,
            emitted_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_labels_stable() {
        assert_eq!(NanoEnvelopeOutcome::Completed.label(), "completed");
        assert_eq!(NanoEnvelopeOutcome::Errored.label(), "errored");
        assert_eq!(NanoEnvelopeOutcome::TimedOut.label(), "timed_out");
        assert_eq!(NanoEnvelopeOutcome::Skipped.label(), "skipped");
    }

    #[test]
    fn schema_field_default_round_trips() {
        // Deserialising a JSON payload that omits `schema` falls
        // back to the literal.
        let json = r#"{
            "agent_id": "ner-extractor",
            "nano_id": "ner-extractor",
            "nano_version": "1.0.0",
            "swarm_id": "00000000-0000-0000-0000-000000000000",
            "trace_id": "00000000-0000-0000-0000-000000000001",
            "mode": "pure_rust",
            "cost_estimate_microcents": 100,
            "cost_actual_microcents": 92,
            "cost_variance_microcents": -8,
            "cache_hit": false,
            "signature_verified": true,
            "gold_bound": true,
            "latency_ms": 12,
            "outcome": "completed",
            "emitted_at": "2026-05-21T12:34:56Z"
        }"#;
        let ev: NanoEnvelopeV1 = serde_json::from_str(json).unwrap();
        assert_eq!(ev.schema, SCHEMA_LITERAL);
    }

    #[test]
    fn envelope_serde_round_trip() {
        let ev = NanoEnvelopeV1::new(
            "ner-extractor",
            "ner-extractor",
            "1.0.0",
            Uuid::nil(),
            Uuid::nil(),
            "pure_rust",
            100,
            92,
            false,
            true,
            true,
            12,
            NanoEnvelopeOutcome::Completed,
        );
        let json = serde_json::to_string(&ev).unwrap();
        let back: NanoEnvelopeV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }

    #[test]
    fn constructor_computes_variance() {
        let ev = NanoEnvelopeV1::new(
            "x",
            "x",
            "1.0.0",
            Uuid::nil(),
            Uuid::nil(),
            "pure_rust",
            100,
            150,
            false,
            false,
            false,
            5,
            NanoEnvelopeOutcome::Completed,
        );
        assert_eq!(ev.cost_variance_microcents, 50);
    }

    #[test]
    fn constructor_handles_negative_variance() {
        let ev = NanoEnvelopeV1::new(
            "x",
            "x",
            "1.0.0",
            Uuid::nil(),
            Uuid::nil(),
            "pure_rust",
            100,
            80,
            false,
            false,
            false,
            5,
            NanoEnvelopeOutcome::Completed,
        );
        assert_eq!(ev.cost_variance_microcents, -20);
    }

    #[test]
    fn constructor_variance_saturates_under_max() {
        let ev = NanoEnvelopeV1::new(
            "x",
            "x",
            "1.0.0",
            Uuid::nil(),
            Uuid::nil(),
            "pure_rust",
            i64::MIN,
            i64::MAX,
            false,
            false,
            false,
            5,
            NanoEnvelopeOutcome::Completed,
        );
        // saturating_sub clamps to i64::MAX, not panics.
        assert_eq!(ev.cost_variance_microcents, i64::MAX);
    }

    #[test]
    fn schema_constant_matches_literal() {
        assert_eq!(SCHEMA_LITERAL, "nano.envelope.v1");
        assert_eq!(default_schema(), SCHEMA_LITERAL);
    }
}
