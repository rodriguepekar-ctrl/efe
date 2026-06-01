//! Dispatcher value types.
//!
//! - [`DagPlan`] : the result of topologically sorting a
//!   [`efe_nano_sdk::SwarmRequest`] into wave-ordered layers.
//! - [`Layer`] : one wave of ops that can be executed in parallel.
//! - [`OpHandle`] : a single op + its resolved metadata (catalog entry,
//!   effective timeout, retry config).
//! - [`OperationOutcome`] : intermediate per-op result the executor writes
//!   into the working state before the orchestrator aggregates everything
//!   into a [`efe_nano_sdk::SwarmResponse`].

use efe_nano_sdk::{
    NanoAgent, Operation, OperationError, OperationErrorKind, OperationResult, OpStatus,
};
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────
//  DagPlan : output of the planner stage
// ─────────────────────────────────────────────────────────────────────

/// A topologically-sorted execution plan.
///
/// `layers[i]` is independent — every op in `layers[i]` only depends on
/// ops in `layers[0..i]`.  Ops within a layer can therefore run fully
/// in parallel (subject to `max_parallelism`).
#[derive(Debug, Clone)]
pub struct DagPlan {
    pub layers: Vec<Layer>,
    /// Total op count (sum of all layers).
    pub total_ops: usize,
    /// Width of the widest layer — used to validate against `max_parallelism`.
    pub max_width: u32,
}

impl DagPlan {
    /// Iterate every op handle in execution order (layer-major).
    pub fn iter_ops(&self) -> impl Iterator<Item = &OpHandle> {
        self.layers.iter().flat_map(|l| l.ops.iter())
    }

    /// Number of layers (== longest dependency chain length).
    pub fn depth(&self) -> usize {
        self.layers.len()
    }
}

/// A single wave of independent operations.
#[derive(Debug, Clone)]
pub struct Layer {
    /// 0-based wave index.  Layer 0 has no upstream dependencies.
    pub index: u32,
    /// Ops in this wave.
    pub ops: Vec<OpHandle>,
}

impl Layer {
    pub fn width(&self) -> u32 {
        self.ops.len() as u32
    }
}

// ─────────────────────────────────────────────────────────────────────
//  OpHandle : op + resolved catalog metadata
// ─────────────────────────────────────────────────────────────────────

/// A single operation augmented with its resolved catalog entry.
///
/// The planner does the catalog lookup up-front so that the executor
/// never needs to touch the catalog during a hot loop.
#[derive(Debug, Clone)]
pub struct OpHandle {
    /// The original operation from the SwarmRequest.
    pub op: Operation,
    /// The resolved catalog entry (specific version chosen).
    pub agent: Arc<NanoAgent>,
    /// Effective wall-clock budget in ms (min of agent.limits + op.timeout_ms).
    pub effective_timeout_ms: u32,
}

impl OpHandle {
    pub fn op_id(&self) -> &str {
        &self.op.op_id
    }
}

// ─────────────────────────────────────────────────────────────────────
//  OperationOutcome : executor-side intermediate result
// ─────────────────────────────────────────────────────────────────────

/// The executor's intermediate result for a single op.
///
/// This is converted to [`OperationResult`] when assembling the final
/// [`efe_nano_sdk::SwarmResponse`].  We keep it separate so that the
/// executor can carry extra fields (e.g. [`crate::runtime::TraceId`],
/// retry-attempt count) without leaking them through the SDK wire format.
#[derive(Debug, Clone)]
pub struct OperationOutcome {
    pub op_id: String,
    pub status: OpStatus,
    pub output: Option<serde_json::Value>,
    pub error: Option<OperationError>,
    pub latency_ms: u32,
    pub cost_cents: f64,
    pub trace_id: Option<efe_nano_sdk::TraceId>,
    pub attempts: u8,
    pub cache_hit: bool,
}

impl OperationOutcome {
    /// Pending outcome — created when an op is enqueued but not yet run.
    pub fn pending(op_id: impl Into<String>) -> Self {
        Self {
            op_id: op_id.into(),
            status: OpStatus::Pending,
            output: None,
            error: None,
            latency_ms: 0,
            cost_cents: 0.0,
            trace_id: None,
            attempts: 0,
            cache_hit: false,
        }
    }

    /// Skipped because a dependency failed.
    pub fn skipped_due_to_dependency(
        op_id: impl Into<String>,
        failed_dep: impl Into<String>,
    ) -> Self {
        Self {
            op_id: op_id.into(),
            status: OpStatus::Skipped,
            output: None,
            error: Some(
                OperationError::new(
                    OperationErrorKind::DependencyFailed,
                    format!("upstream dependency '{}' failed", failed_dep.into()),
                )
                .with_source(""),
            ),
            latency_ms: 0,
            cost_cents: 0.0,
            trace_id: None,
            attempts: 0,
            cache_hit: false,
        }
    }

    /// Convert to the SDK's wire-format result.
    pub fn into_operation_result(self) -> OperationResult {
        OperationResult {
            op_id: self.op_id,
            status: self.status,
            output: self.output,
            error: self.error,
            latency_ms: self.latency_ms,
            cost_cents: self.cost_cents,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{NanoAgentId, NanoAgentRef, OperationInput, RetryPolicy};

    fn dummy_op(id: &str, deps: Vec<&str>) -> Operation {
        Operation {
            op_id: id.to_string(),
            agent: NanoAgentRef::new(NanoAgentId::new("json_validate").unwrap()),
            input: OperationInput::Literal {
                value: serde_json::json!({}),
            },
            depends_on: deps.into_iter().map(String::from).collect(),
            timeout_ms: None,
            retry: RetryPolicy::default(),
        }
    }

    #[test]
    fn pending_outcome_starts_in_pending_state() {
        let o = OperationOutcome::pending("op-1");
        assert_eq!(o.op_id, "op-1");
        assert_eq!(o.status, OpStatus::Pending);
        assert!(o.error.is_none());
        assert_eq!(o.attempts, 0);
    }

    #[test]
    fn skipped_outcome_carries_dependency_message() {
        let o = OperationOutcome::skipped_due_to_dependency("op-2", "op-1");
        assert_eq!(o.status, OpStatus::Skipped);
        let err = o.error.as_ref().unwrap();
        assert_eq!(err.kind, OperationErrorKind::DependencyFailed);
        assert!(err.message.contains("op-1"));
    }

    #[test]
    fn into_operation_result_drops_internal_fields() {
        let o = OperationOutcome {
            op_id: "op-1".to_string(),
            status: OpStatus::Success,
            output: Some(serde_json::json!({"x": 1})),
            error: None,
            latency_ms: 5,
            cost_cents: 0.001,
            trace_id: None,
            attempts: 1,
            cache_hit: false,
        };
        let r = o.into_operation_result();
        assert_eq!(r.op_id, "op-1");
        assert_eq!(r.status, OpStatus::Success);
        assert_eq!(r.latency_ms, 5);
        assert_eq!(r.cost_cents, 0.001);
        // attempts + trace_id + cache_hit are gone — internal only.
    }

    #[test]
    fn dag_plan_depth_equals_layer_count() {
        let plan = DagPlan {
            layers: vec![
                Layer { index: 0, ops: vec![] },
                Layer { index: 1, ops: vec![] },
                Layer { index: 2, ops: vec![] },
            ],
            total_ops: 0,
            max_width: 0,
        };
        assert_eq!(plan.depth(), 3);
    }

    #[test]
    fn op_handle_carries_id_through() {
        let op = dummy_op("test-op", vec![]);
        let agent = make_test_nano_agent();
        let h = OpHandle {
            op,
            agent: Arc::new(agent),
            effective_timeout_ms: 5_000,
        };
        assert_eq!(h.op_id(), "test-op");
    }

    fn make_test_nano_agent() -> NanoAgent {
        use chrono::Utc;
        use efe_nano_sdk::{
            AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
            IdempotencyPolicy, NanoAgentVersion, ResourceLimits, StabilityTier,
        };

        NanoAgent {
            id: NanoAgentId::new("json_validate").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "test".to_string(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "json_validate::run".to_string(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
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
            created_at: Utc::now(),
        }
    }
}
