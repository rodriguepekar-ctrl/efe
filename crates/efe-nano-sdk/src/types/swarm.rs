//! Swarm types : [`SwarmRequest`], [`Operation`], [`SwarmResponse`], etc.
//!
//! These are the types customers use to invoke a batch of nano agents
//! via the dispatcher.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

use crate::error::OperationError;
use crate::traits::TraceId;
use crate::types::{NanoAgentRef, TenantId};

// ─────────────────────────────────────────────────────────────────────
//  SwarmRequest — top-level batch invocation
// ─────────────────────────────────────────────────────────────────────

/// Top-level request to execute a batch of nano agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmRequest {
    /// Unique request ID (UUID v4).
    pub request_id: Uuid,

    /// Tenant scope.
    pub tenant_id: TenantId,

    /// The agent making this request (e.g. "claude-sonnet-4-5",
    /// "openai-gpt-4o").  Optional, used for telemetry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caller_agent_id: Option<String>,

    /// List of operations forming a DAG via `depends_on`.
    pub operations: Vec<Operation>,

    /// Maximum concurrent ops at any layer.  Safety cap, default 100.
    #[serde(default = "default_max_parallelism")]
    pub max_parallelism: u32,

    /// Overall batch timeout in milliseconds, default 60000.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u32,

    /// Customer-provided idempotency key.  Same key + same body =
    /// returns cached response for 24h.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,

    /// Stream partial results back as ops complete.
    #[serde(default)]
    pub streaming: bool,

    /// Searchable audit tags for trace lookup.
    #[serde(default)]
    pub audit_tags: Vec<String>,

    /// **Sprint 7 — N7** : Swarm-wide hard cost cap in cents.  When
    /// `Some(c)`, the dispatcher monitors the cumulative `cost_cents`
    /// across all completed ops and aborts the remaining layers as
    /// soon as the running total exceeds `c`.  Ops that were already
    /// in flight at abort time complete normally and are charged ;
    /// ops in unstarted layers are returned with
    /// `OperationErrorKind::CostCapReached`.
    ///
    /// `None` (default) = no swarm-level cap ; only per-tenant
    /// budget (M21 BudgetEnforcer) applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_cap_cents: Option<f64>,

    /// **Sprint 8 — N14** : Optional reference to the parent swarm's
    /// envelope trace.  Caller-set when this swarm is a child of a
    /// meta-swarm.  Sprint 8 : the platform doesn't validate this
    /// reference ; Sprint 9 N15 wires meta-swarm orchestration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_envelope_trace_id: Option<TraceId>,
}

fn default_max_parallelism() -> u32 {
    100
}

fn default_timeout_ms() -> u32 {
    60_000
}

impl SwarmRequest {
    /// Returns the number of operations in this request.
    pub fn op_count(&self) -> usize {
        self.operations.len()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Operation — single nano agent invocation within a swarm
// ─────────────────────────────────────────────────────────────────────

/// A single nano agent invocation within a swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    /// Customer-chosen identifier for this operation, unique within the swarm.
    /// Used for dependency references.
    pub op_id: String,

    /// Reference to the nano agent (id + version requirement).
    pub agent: NanoAgentRef,

    /// Input value for this operation.
    pub input: OperationInput,

    /// List of `op_id`s this operation depends on (must complete first).
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Per-op timeout override (in ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u32>,

    /// Retry policy if the op fails.
    #[serde(default)]
    pub retry: RetryPolicy,
}

/// How an operation gets its input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperationInput {
    /// Literal JSON value.
    Literal { value: serde_json::Value },

    /// Reference to another op's output, via JSON path.
    Ref { op_id: String, json_path: String },

    /// Merge multiple inputs into one (shallow merge for objects,
    /// concatenation for arrays).
    Merge { sources: Vec<OperationInput> },
}

/// Retry policy for a single operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Max retry attempts (1-5).  Default 1 (no retry).
    pub max_attempts: u8,

    /// Backoff between attempts in milliseconds.  Default 0.
    pub backoff_ms: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            backoff_ms: 0,
        }
    }
}

impl RetryPolicy {
    /// Convenience : up to N attempts with linear backoff in ms.
    pub fn linear(max_attempts: u8, backoff_ms: u32) -> Self {
        Self {
            max_attempts: max_attempts.clamp(1, 5),
            backoff_ms,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  SwarmResponse — aggregated results
// ─────────────────────────────────────────────────────────────────────

/// Result of executing a SwarmRequest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmResponse {
    pub request_id: Uuid,

    pub overall_status: SwarmStatus,

    /// Per-operation results, keyed by op_id.
    pub results: HashMap<String, OperationResult>,

    /// Total wall-clock time of the swarm.
    pub total_latency_ms: u32,

    /// Total cost in cents.
    pub total_cost_cents: f64,

    /// **Sprint 8 — N14** : Trace id of the envelope row written by the
    /// dispatcher at swarm completion.  Lets the caller query their
    /// audit log for the swarm anchor.  `None` if envelope writing
    /// failed (resilient mode — see N6 audit-write contract).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swarm_envelope_trace_id: Option<TraceId>,
}

/// Aggregate status of a swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwarmStatus {
    /// All ops succeeded.
    Success,
    /// At least one op failed but others succeeded.
    PartialFailure,
    /// Batch exceeded overall timeout.
    TimedOut,
    /// Batch was cancelled (by client).
    Cancelled,
}

/// Result of a single operation within the swarm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub op_id: String,
    pub status: OpStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OperationError>,

    pub latency_ms: u32,
    pub cost_cents: f64,
}

/// Status of a single operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpStatus {
    /// Not yet started.
    Pending,
    /// Currently running.
    Running,
    /// Completed successfully.
    Success,
    /// Failed (see `error`).
    Failed,
    /// Skipped because a dependency failed.
    Skipped,
    /// Cancelled (batch-wide).
    Cancelled,
}

impl OpStatus {
    /// Did this op produce useful output ?
    pub fn is_success(&self) -> bool {
        matches!(self, OpStatus::Success)
    }

    /// Is this op in a terminal state ?
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OpStatus::Success | OpStatus::Failed | OpStatus::Skipped | OpStatus::Cancelled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NanoAgentId;

    fn sample_request() -> SwarmRequest {
        SwarmRequest {
            request_id: Uuid::new_v4(),
            tenant_id: TenantId::new(),
            caller_agent_id: Some("claude-sonnet-4-5".to_string()),
            operations: vec![Operation {
                op_id: "extract-1".to_string(),
                agent: NanoAgentRef::new(NanoAgentId::new("pdf_extract_text").unwrap()),
                input: OperationInput::Literal {
                    value: serde_json::json!({"pdf_bytes_base64": "..."}),
                },
                depends_on: vec![],
                timeout_ms: Some(10_000),
                retry: RetryPolicy::default(),
            }],
            max_parallelism: 50,
            timeout_ms: 30_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec!["test".to_string()],
            cost_cap_cents: None,
            parent_envelope_trace_id: None,
        }
    }

    #[test]
    fn swarm_request_round_trip() {
        let r = sample_request();
        let s = serde_json::to_string(&r).unwrap();
        let r2: SwarmRequest = serde_json::from_str(&s).unwrap();
        assert_eq!(r.request_id, r2.request_id);
        assert_eq!(r.tenant_id, r2.tenant_id);
        assert_eq!(r.operations.len(), r2.operations.len());
        assert_eq!(r.max_parallelism, r2.max_parallelism);
    }

    #[test]
    fn operation_input_literal() {
        let i = OperationInput::Literal {
            value: serde_json::json!({"x": 1}),
        };
        let s = serde_json::to_string(&i).unwrap();
        assert!(s.contains(r#""kind":"literal""#));
        let i2: OperationInput = serde_json::from_str(&s).unwrap();
        match i2 {
            OperationInput::Literal { value } => {
                assert_eq!(value, serde_json::json!({"x": 1}));
            }
            _ => panic!("expected Literal"),
        }
    }

    #[test]
    fn operation_input_ref() {
        let i = OperationInput::Ref {
            op_id: "extract-1".to_string(),
            json_path: "$.text".to_string(),
        };
        let s = serde_json::to_string(&i).unwrap();
        assert!(s.contains(r#""kind":"ref""#));
        let i2: OperationInput = serde_json::from_str(&s).unwrap();
        if let OperationInput::Ref { op_id, json_path } = i2 {
            assert_eq!(op_id, "extract-1");
            assert_eq!(json_path, "$.text");
        } else {
            panic!("expected Ref");
        }
    }

    #[test]
    fn retry_policy_default() {
        let p = RetryPolicy::default();
        assert_eq!(p.max_attempts, 1);
        assert_eq!(p.backoff_ms, 0);
    }

    #[test]
    fn retry_policy_clamps_max_attempts() {
        let p = RetryPolicy::linear(10, 100);
        assert_eq!(p.max_attempts, 5);  // clamped to 5
        let p2 = RetryPolicy::linear(0, 100);
        assert_eq!(p2.max_attempts, 1);  // clamped to 1
    }

    #[test]
    fn op_status_helpers() {
        assert!(OpStatus::Success.is_success());
        assert!(!OpStatus::Failed.is_success());
        assert!(!OpStatus::Pending.is_success());

        assert!(OpStatus::Success.is_terminal());
        assert!(OpStatus::Failed.is_terminal());
        assert!(OpStatus::Skipped.is_terminal());
        assert!(OpStatus::Cancelled.is_terminal());
        assert!(!OpStatus::Pending.is_terminal());
        assert!(!OpStatus::Running.is_terminal());
    }

    #[test]
    fn swarm_request_defaults() {
        // Test that omitting fields uses defaults.
        let json = format!(
            r#"{{
                "request_id": "{}",
                "tenant_id": "{}",
                "operations": []
            }}"#,
            Uuid::new_v4(),
            TenantId::new()
        );
        let r: SwarmRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(r.max_parallelism, 100);
        assert_eq!(r.timeout_ms, 60_000);
        assert!(!r.streaming);
        assert!(r.audit_tags.is_empty());
    }

    #[test]
    fn swarm_status_round_trip() {
        for status in [
            SwarmStatus::Success,
            SwarmStatus::PartialFailure,
            SwarmStatus::TimedOut,
            SwarmStatus::Cancelled,
        ] {
            let s = serde_json::to_string(&status).unwrap();
            let s2: SwarmStatus = serde_json::from_str(&s).unwrap();
            assert_eq!(status, s2);
        }
    }

    #[test]
    fn operation_input_merge() {
        let i = OperationInput::Merge {
            sources: vec![
                OperationInput::Literal {
                    value: serde_json::json!({"a": 1}),
                },
                OperationInput::Literal {
                    value: serde_json::json!({"b": 2}),
                },
            ],
        };
        let s = serde_json::to_string(&i).unwrap();
        let i2: OperationInput = serde_json::from_str(&s).unwrap();
        if let OperationInput::Merge { sources } = i2 {
            assert_eq!(sources.len(), 2);
        } else {
            panic!("expected Merge");
        }
    }

    #[test]
    fn swarm_request_op_count() {
        let r = sample_request();
        assert_eq!(r.op_count(), 1);
    }
}
