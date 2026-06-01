//! DAG validation + topological layering for a [`SwarmRequest`].
//!
//! Pipeline :
//! 1. Validate op_id uniqueness + dependency existence.
//! 2. Resolve every `NanoAgentRef` against the catalog (gives a concrete
//!    `Arc<NanoAgent>`).
//! 3. Run a layered Kahn topological sort.
//! 4. Cycle detection : if any op remains with in-degree > 0 after the
//!    sort, the graph has a cycle.

use efe_nano_sdk::{Catalog, NanoAgent, Operation, SwarmRequest};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::dispatcher::types::{DagPlan, Layer, OpHandle};
use crate::error::DispatcherError;

/// Plan a SwarmRequest into a layered DAG.
///
/// Returns a [`DagPlan`] with topologically-sorted layers or a
/// [`DispatcherError`] if the request is malformed.
pub async fn plan(
    request: &SwarmRequest,
    catalog: &dyn Catalog,
) -> Result<DagPlan, DispatcherError> {
    // ─────────────────────────────────────────────────────────────────
    //  Phase 1 — structural validation
    // ─────────────────────────────────────────────────────────────────
    if request.operations.is_empty() {
        return Err(DispatcherError::InvalidDag {
            reason: "swarm request has 0 operations".into(),
        });
    }

    let mut seen: HashSet<&str> = HashSet::with_capacity(request.operations.len());
    for op in &request.operations {
        if op.op_id.is_empty() {
            return Err(DispatcherError::InvalidDag {
                reason: "operation has empty op_id".into(),
            });
        }
        if !seen.insert(op.op_id.as_str()) {
            return Err(DispatcherError::DuplicateOpId(op.op_id.clone()));
        }
    }

    // Every dependency must reference a known op_id.
    for op in &request.operations {
        for dep in &op.depends_on {
            if !seen.contains(dep.as_str()) {
                return Err(DispatcherError::UnknownOpId(dep.clone()));
            }
            if dep == &op.op_id {
                return Err(DispatcherError::InvalidDag {
                    reason: format!("op '{}' depends on itself", op.op_id),
                });
            }
        }
    }

    // Every op_id referenced via OperationInput::Ref or ::Merge must be
    // listed in this op's `depends_on` — otherwise the executor might try
    // to read the upstream output before it's been produced (race).
    for op in &request.operations {
        for referenced in referenced_op_ids(&op.input) {
            if referenced == op.op_id {
                return Err(DispatcherError::InvalidDag {
                    reason: format!("op '{}' input refers to itself", op.op_id),
                });
            }
            if !seen.contains(referenced.as_str()) {
                return Err(DispatcherError::UnknownOpId(referenced));
            }
            if !op.depends_on.contains(&referenced) {
                return Err(DispatcherError::InvalidDag {
                    reason: format!(
                        "op '{}' input references '{}' but does not list it in depends_on",
                        op.op_id, referenced
                    ),
                });
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Phase 2 — resolve catalog refs in parallel
    // ─────────────────────────────────────────────────────────────────
    let agents = resolve_agents(request, catalog).await?;

    // ─────────────────────────────────────────────────────────────────
    //  Phase 3 — Kahn's algorithm with layering
    // ─────────────────────────────────────────────────────────────────
    let layers = topological_layers(&request.operations)?;

    // ─────────────────────────────────────────────────────────────────
    //  Phase 4 — assemble OpHandles
    // ─────────────────────────────────────────────────────────────────
    let by_id: HashMap<&str, &Operation> = request
        .operations
        .iter()
        .map(|o| (o.op_id.as_str(), o))
        .collect();

    let mut plan_layers: Vec<Layer> = Vec::with_capacity(layers.len());
    let mut max_width: u32 = 0;
    let mut total_ops: usize = 0;
    for (layer_idx, op_ids) in layers.iter().enumerate() {
        let mut ops: Vec<OpHandle> = Vec::with_capacity(op_ids.len());
        for op_id in op_ids {
            let op = (*by_id.get(op_id.as_str()).expect("op exists")).clone();
            let agent = Arc::clone(
                agents
                    .get(op_id.as_str())
                    .expect("agent resolved for every op_id"),
            );
            let effective = compute_effective_timeout(&op, &agent, request.timeout_ms);
            ops.push(OpHandle {
                op,
                agent,
                effective_timeout_ms: effective,
            });
        }
        let width = ops.len() as u32;
        if width > max_width {
            max_width = width;
        }
        total_ops += ops.len();
        plan_layers.push(Layer {
            index: layer_idx as u32,
            ops,
        });
    }

    // Width vs max_parallelism check.
    if max_width > request.max_parallelism {
        return Err(DispatcherError::TooParallel {
            actual: max_width,
            max: request.max_parallelism,
        });
    }

    Ok(DagPlan {
        layers: plan_layers,
        total_ops,
        max_width,
    })
}

/// Resolve each op's `NanoAgentRef` against the catalog.
///
/// Returns a map keyed by op_id → resolved `Arc<NanoAgent>`.
async fn resolve_agents(
    request: &SwarmRequest,
    catalog: &dyn Catalog,
) -> Result<HashMap<String, Arc<NanoAgent>>, DispatcherError> {
    let mut out: HashMap<String, Arc<NanoAgent>> =
        HashMap::with_capacity(request.operations.len());
    for op in &request.operations {
        let agent = catalog
            .get(&op.agent, request.tenant_id)
            .await
            .map_err(|e| DispatcherError::InvalidDag {
                reason: format!(
                    "op '{}' : catalog resolution failed for '{}': {}",
                    op.op_id,
                    op.agent.id.as_str(),
                    e
                ),
            })?;
        out.insert(op.op_id.clone(), agent);
    }
    Ok(out)
}

/// Run a layered Kahn topological sort.
///
/// Returns `Vec<Vec<op_id>>` where each inner vec is one wave.
/// Returns `CycleDetected` if any op remains unscheduled.
fn topological_layers(
    operations: &[Operation],
) -> Result<Vec<Vec<String>>, DispatcherError> {
    // Build adjacency : dep_id → set of ops that depend on it.
    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    let mut indegree: HashMap<String, usize> = HashMap::new();

    for op in operations {
        // Make sure every op has an entry even if indegree == 0.
        indegree.entry(op.op_id.clone()).or_insert(0);
    }

    for op in operations {
        for dep in &op.depends_on {
            adjacency
                .entry(dep.clone())
                .or_default()
                .push(op.op_id.clone());
            *indegree.entry(op.op_id.clone()).or_insert(0) += 1;
        }
    }

    // Sort op_ids deterministically when introducing them into the queue so
    // that layer ordering is reproducible (and tests stay stable).
    let mut queue: VecDeque<String> = {
        let zero: BTreeSet<&String> = indegree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(k, _)| k)
            .collect();
        zero.into_iter().cloned().collect()
    };

    let mut layers: Vec<Vec<String>> = Vec::new();
    let mut scheduled: HashSet<String> = HashSet::new();

    while !queue.is_empty() {
        // Take everything currently at indegree-0 as this layer.
        let layer: Vec<String> = queue.drain(..).collect();

        // Deterministic ordering : sort the layer alphabetically.
        let mut layer_sorted = layer;
        layer_sorted.sort();
        for op_id in &layer_sorted {
            scheduled.insert(op_id.clone());
        }

        // Decrement indegree of each successor, queue any that hit 0.
        let mut next_layer: BTreeSet<String> = BTreeSet::new();
        for op_id in &layer_sorted {
            if let Some(successors) = adjacency.get(op_id) {
                for succ in successors {
                    let deg = indegree.get_mut(succ).expect("indegree present");
                    *deg -= 1;
                    if *deg == 0 && !scheduled.contains(succ) {
                        next_layer.insert(succ.clone());
                    }
                }
            }
        }

        layers.push(layer_sorted);
        queue.extend(next_layer);
    }

    // Cycle check : any op not scheduled?
    if scheduled.len() != operations.len() {
        let unscheduled: Vec<String> = operations
            .iter()
            .filter(|o| !scheduled.contains(&o.op_id))
            .map(|o| o.op_id.clone())
            .collect();
        return Err(DispatcherError::CycleDetected { ops: unscheduled });
    }

    Ok(layers)
}

/// Effective timeout = min(op.timeout_ms, agent.limits.wall_clock_ms, request.timeout_ms).
fn compute_effective_timeout(op: &Operation, agent: &NanoAgent, swarm_budget_ms: u32) -> u32 {
    let mut t = agent.limits.wall_clock_ms;
    if let Some(per_op) = op.timeout_ms {
        t = t.min(per_op);
    }
    t.min(swarm_budget_ms)
}

/// Walk an [`OperationInput`] tree and collect every `op_id` it references
/// (via `Ref` or nested `Merge`).  Used to validate that input references
/// align with `depends_on`.
fn referenced_op_ids(input: &efe_nano_sdk::OperationInput) -> Vec<String> {
    let mut out = Vec::new();
    walk_referenced(input, &mut out);
    out
}

fn walk_referenced(input: &efe_nano_sdk::OperationInput, out: &mut Vec<String>) {
    match input {
        efe_nano_sdk::OperationInput::Literal { .. } => {}
        efe_nano_sdk::OperationInput::Ref { op_id, .. } => {
            out.push(op_id.clone());
        }
        efe_nano_sdk::OperationInput::Merge { sources } => {
            for s in sources {
                walk_referenced(s, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::InMemoryCatalog;
    use efe_nano_sdk::{
        AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
        IdempotencyPolicy, NanoAgentId, NanoAgentRef, NanoAgentVersion, OperationInput,
        ResourceLimits, RetryPolicy, StabilityTier, TenantId,
    };
    use uuid::Uuid;

    fn op(id: &str, deps: &[&str]) -> Operation {
        Operation {
            op_id: id.to_string(),
            agent: NanoAgentRef::new(NanoAgentId::new("agent_a").unwrap()),
            input: OperationInput::Literal {
                value: serde_json::json!({}),
            },
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            timeout_ms: None,
            retry: RetryPolicy::default(),
        }
    }

    fn req(ops: Vec<Operation>) -> SwarmRequest {
        SwarmRequest {
            request_id: Uuid::new_v4(),
            tenant_id: TenantId::system(),
            caller_agent_id: None,
            operations: ops,
            max_parallelism: 100,
            timeout_ms: 30_000,
            idempotency_key: None,
            streaming: false,
            audit_tags: vec![],
            cost_cap_cents: None,
            parent_envelope_trace_id: None,
        }
    }

    fn agent_a() -> NanoAgent {
        NanoAgent {
            id: NanoAgentId::new("agent_a").unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "test".to_string(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::Tier1,
            execution_mode: ExecutionMode::PureRust {
                entry: "agent_a::run".to_string(),
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
            created_at: chrono::Utc::now(),
        }
    }

    async fn make_catalog() -> InMemoryCatalog {
        let c = InMemoryCatalog::new();
        c.publish(TenantId::system(), agent_a()).await.unwrap();
        c
    }

    #[tokio::test]
    async fn linear_chain_is_layered_one_per_layer() {
        let catalog = make_catalog().await;
        let request = req(vec![
            op("a", &[]),
            op("b", &["a"]),
            op("c", &["b"]),
        ]);
        let plan = plan(&request, &catalog).await.unwrap();
        assert_eq!(plan.depth(), 3);
        assert_eq!(plan.layers[0].ops[0].op_id(), "a");
        assert_eq!(plan.layers[1].ops[0].op_id(), "b");
        assert_eq!(plan.layers[2].ops[0].op_id(), "c");
        assert_eq!(plan.total_ops, 3);
        assert_eq!(plan.max_width, 1);
    }

    #[tokio::test]
    async fn diamond_dag_has_2_layers_with_2_widths() {
        // a → b, a → c, b → d, c → d
        let catalog = make_catalog().await;
        let request = req(vec![
            op("a", &[]),
            op("b", &["a"]),
            op("c", &["a"]),
            op("d", &["b", "c"]),
        ]);
        let plan = plan(&request, &catalog).await.unwrap();
        assert_eq!(plan.depth(), 3);
        assert_eq!(plan.layers[0].width(), 1);  // a
        assert_eq!(plan.layers[1].width(), 2);  // b, c
        assert_eq!(plan.layers[2].width(), 1);  // d
        assert_eq!(plan.max_width, 2);
    }

    #[tokio::test]
    async fn parallel_ops_in_one_layer() {
        // a, b, c, d all independent
        let catalog = make_catalog().await;
        let request = req(vec![op("a", &[]), op("b", &[]), op("c", &[]), op("d", &[])]);
        let plan = plan(&request, &catalog).await.unwrap();
        assert_eq!(plan.depth(), 1);
        assert_eq!(plan.layers[0].width(), 4);
    }

    #[tokio::test]
    async fn cycle_detected() {
        let catalog = make_catalog().await;
        // a → b → c → a (cycle)
        let request = req(vec![
            op("a", &["c"]),
            op("b", &["a"]),
            op("c", &["b"]),
        ]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::CycleDetected { ops } => {
                assert_eq!(ops.len(), 3);
            }
            other => panic!("expected CycleDetected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn self_loop_rejected() {
        let catalog = make_catalog().await;
        let request = req(vec![op("a", &["a"])]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::InvalidDag { reason } => assert!(reason.contains("depends on itself")),
            other => panic!("expected InvalidDag, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn duplicate_op_id_rejected() {
        let catalog = make_catalog().await;
        let request = req(vec![op("a", &[]), op("a", &[])]);
        let err = plan(&request, &catalog).await.unwrap_err();
        assert!(matches!(err, DispatcherError::DuplicateOpId(_)));
    }

    #[tokio::test]
    async fn unknown_dependency_rejected() {
        let catalog = make_catalog().await;
        let request = req(vec![op("a", &["ghost"])]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::UnknownOpId(id) => assert_eq!(id, "ghost"),
            other => panic!("expected UnknownOpId, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_swarm_rejected() {
        let catalog = make_catalog().await;
        let request = req(vec![]);
        let err = plan(&request, &catalog).await.unwrap_err();
        assert!(matches!(err, DispatcherError::InvalidDag { .. }));
    }

    #[tokio::test]
    async fn agent_not_in_catalog_yields_invalid_dag() {
        let catalog = InMemoryCatalog::new();  // empty
        let request = req(vec![op("a", &[])]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::InvalidDag { reason } => {
                assert!(reason.contains("catalog resolution failed"));
            }
            other => panic!("expected InvalidDag, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn max_parallelism_exceeded() {
        let catalog = make_catalog().await;
        let mut request = req(vec![op("a", &[]), op("b", &[]), op("c", &[])]);
        request.max_parallelism = 2;
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::TooParallel { actual, max } => {
                assert_eq!(actual, 3);
                assert_eq!(max, 2);
            }
            other => panic!("expected TooParallel, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn deterministic_ordering_within_layer() {
        let catalog = make_catalog().await;
        // 5 parallel ops in any order — sort must be alphabetical.
        let request = req(vec![op("z", &[]), op("b", &[]), op("a", &[]), op("c", &[]), op("y", &[])]);
        let plan = plan(&request, &catalog).await.unwrap();
        let ids: Vec<&str> = plan.layers[0].ops.iter().map(|o| o.op_id()).collect();
        assert_eq!(ids, vec!["a", "b", "c", "y", "z"]);
    }

    #[tokio::test]
    async fn effective_timeout_takes_min_of_three_sources() {
        let catalog = make_catalog().await;
        let mut o = op("a", &[]);
        o.timeout_ms = Some(1000);

        // Agent default wall_clock_ms is 5000 (ResourceLimits::default_mode_a).
        // Swarm timeout 10_000.  Op timeout 1000.  Effective = 1000.
        let mut request = req(vec![o]);
        request.timeout_ms = 10_000;
        let plan = plan(&request, &catalog).await.unwrap();
        assert_eq!(plan.layers[0].ops[0].effective_timeout_ms, 1000);
    }

    #[tokio::test]
    async fn effective_timeout_capped_by_swarm_budget() {
        let catalog = make_catalog().await;
        let mut request = req(vec![op("a", &[])]);
        request.timeout_ms = 500;  // tighter than agent
        let plan = plan(&request, &catalog).await.unwrap();
        assert_eq!(plan.layers[0].ops[0].effective_timeout_ms, 500);
    }

    #[tokio::test]
    async fn empty_op_id_rejected() {
        let catalog = make_catalog().await;
        let request = req(vec![op("", &[])]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::InvalidDag { reason } => assert!(reason.contains("empty op_id")),
            other => panic!("expected InvalidDag, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ref_input_without_matching_depends_on_rejected() {
        // op_b's input references op_a but op_b.depends_on is empty.
        // The planner should catch this as an InvalidDag before execution.
        let catalog = make_catalog().await;
        let request = req(vec![
            op("a", &[]),
            Operation {
                op_id: "b".to_string(),
                agent: NanoAgentRef::new(NanoAgentId::new("agent_a").unwrap()),
                input: OperationInput::Ref {
                    op_id: "a".to_string(),
                    json_path: "$".to_string(),
                },
                depends_on: vec![],  // intentionally empty
                timeout_ms: None,
                retry: RetryPolicy::default(),
            },
        ]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::InvalidDag { reason } => {
                assert!(reason.contains("depends_on"));
                assert!(reason.contains("'a'"));
            }
            other => panic!("expected InvalidDag, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ref_input_pointing_to_self_rejected() {
        // op_a's input references op_a — the planner must reject before
        // execution would deadlock.
        let catalog = make_catalog().await;
        let request = req(vec![Operation {
            op_id: "a".to_string(),
            agent: NanoAgentRef::new(NanoAgentId::new("agent_a").unwrap()),
            input: OperationInput::Ref {
                op_id: "a".to_string(),
                json_path: "$".to_string(),
            },
            depends_on: vec![],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        }]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::InvalidDag { reason } => {
                assert!(reason.contains("refers to itself"));
            }
            other => panic!("expected InvalidDag, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn merge_input_traversed_for_dependency_validation() {
        // Nested Merge → Ref → unknown op_id must be caught.
        let catalog = make_catalog().await;
        let request = req(vec![Operation {
            op_id: "a".to_string(),
            agent: NanoAgentRef::new(NanoAgentId::new("agent_a").unwrap()),
            input: OperationInput::Merge {
                sources: vec![
                    OperationInput::Literal {
                        value: serde_json::json!({}),
                    },
                    OperationInput::Merge {
                        sources: vec![OperationInput::Ref {
                            op_id: "ghost".to_string(),
                            json_path: "$".to_string(),
                        }],
                    },
                ],
            },
            depends_on: vec![],
            timeout_ms: None,
            retry: RetryPolicy::default(),
        }]);
        let err = plan(&request, &catalog).await.unwrap_err();
        match err {
            DispatcherError::UnknownOpId(id) => assert_eq!(id, "ghost"),
            other => panic!("expected UnknownOpId, got {other:?}"),
        }
    }
}
