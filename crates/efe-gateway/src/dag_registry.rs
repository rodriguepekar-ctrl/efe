// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : DAG registry — § 7.3.
//! **Sprint 15 — N23 + G10 + Workflow D** : the default DAG is now
//! built on top of the N23 adapter (`NanoToSwarm`) rather than
//! wiring the six pre-analysis structs directly.  Every dispatch
//! flows through the policy gates (signature, gold, idempotency
//! cache) and emits a `nano.envelope.v1` audit event.

use std::sync::Arc;

use efe_blackboard::Blackboard;
use efe_compliance_wiring::{
    sign_catalogue_with_builtin_publisher, WiredComplianceRunner,
};
use efe_nano_preanalysis::default_catalogue;
use efe_nano_sdk::AgentId;
use efe_nano_swarm_adapter::{NanoToSwarm, NanoToSwarmPolicy};
use efe_swarm_orchestrator::{
    agent::AgentOutcome, AgentNode, SwarmAgent, SwarmAgentContext, SwarmAgentError,
    SwarmDag, SwarmDagBuilder,
};

/// **N22** : the "default" reference DAG name.
pub const DEFAULT_DAG_ID: &str = "default";

/// **N22** : DAG lookup outcome.
pub enum DagLookup {
    Found(SwarmDag),
    NotFound,
}

/// **N22** : look up a DAG by ID.  Sprint 14 only ships `"default"`
/// and (if no id passed) returns it ; everything else → NotFound.
pub fn lookup_dag(dag_id: Option<&str>) -> DagLookup {
    match dag_id.unwrap_or(DEFAULT_DAG_ID) {
        DEFAULT_DAG_ID | "" => match build_default_dag() {
            Ok(d) => DagLookup::Found(d),
            Err(_) => DagLookup::NotFound,
        },
        _ => DagLookup::NotFound,
    }
}

/// **N24** : look up a DAG by ID, using the wired compliance
/// runner's per-tier policy when present.  Used by the gateway
/// when `AppState.compliance` is `Some(...)`.
pub fn lookup_dag_wired(
    dag_id: Option<&str>,
    runner: &WiredComplianceRunner,
) -> DagLookup {
    match dag_id.unwrap_or(DEFAULT_DAG_ID) {
        DEFAULT_DAG_ID | "" => match build_default_dag_wired(runner) {
            Ok(d) => DagLookup::Found(d),
            Err(_) => DagLookup::NotFound,
        },
        _ => DagLookup::NotFound,
    }
}

/// **N22** : build the reference pre-analysis DAG.
///
/// Layer 1 (parallel) :
///   - ner-extractor
///   - language-detector
///   - schema-validator
///   - sentiment-classifier
///
/// Layer 2 :
///   - pii-redactor (deps ner + language)
///
/// Layer 3 :
///   - summary-writer (deps pii)
///
/// Layer 4 (Aggregator) :
///   - context-aggregator (no-op ; runtime builds the bundle from
///     the per-agent outcomes)
fn build_default_dag() -> Result<SwarmDag, efe_swarm_orchestrator::SwarmDagError> {
    // **N23 + G10 + Workflow D** : pull every node from the
    // catalogue and wrap with `NanoToSwarm`.  Default policy is
    // `experimental_fast` (no compliance gates) — when the gateway
    // carries a `WiredComplianceRunner` it uses
    // [`build_default_dag_wired`] instead.
    let policy = NanoToSwarmPolicy::experimental_fast();
    let mut builder = SwarmDagBuilder::new();
    for (nano, runner) in default_catalogue() {
        let adapter = NanoToSwarm::new(nano, runner).with_policy(policy.clone());
        builder = builder.add_agent(AgentNode::new(Arc::new(adapter)));
    }
    builder
        // Aggregator
        .add_agent(AgentNode::new(Arc::new(NoOpAggregator)))
        // Dependencies
        .add_dependency(
            AgentId::new("ner-extractor").unwrap(),
            AgentId::new("pii-redactor").unwrap(),
        )
        .add_dependency(
            AgentId::new("language-detector").unwrap(),
            AgentId::new("pii-redactor").unwrap(),
        )
        .add_dependency(
            AgentId::new("pii-redactor").unwrap(),
            AgentId::new("summary-writer").unwrap(),
        )
        // Aggregator dependencies
        .add_dependency(
            AgentId::new("ner-extractor").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("language-detector").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("schema-validator").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("sentiment-classifier").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("summary-writer").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .with_aggregator(AgentId::new("context-aggregator").unwrap())
        .build()
}

/// **N24 + Annex A3** : per-tier policy build for the default DAG.
///
/// Built-in catalogue entries are first signed via
/// [`sign_catalogue_with_builtin_publisher`] so the wired
/// signature gate has something to verify.  Each entry's policy
/// is chosen via [`WiredComplianceRunner::select_policy_for`]
/// (Tier1+ → strict, TierExperimental → relaxed).
fn build_default_dag_wired(
    runner: &WiredComplianceRunner,
) -> Result<SwarmDag, efe_swarm_orchestrator::SwarmDagError> {
    let signed = sign_catalogue_with_builtin_publisher(default_catalogue(), None);
    let mut builder = SwarmDagBuilder::new();
    for (nano, nano_runner) in signed {
        let policy = runner.select_policy_for(&nano);
        let adapter = NanoToSwarm::new(nano, nano_runner).with_policy(policy);
        builder = builder.add_agent(AgentNode::new(Arc::new(adapter)));
    }
    builder
        // Aggregator
        .add_agent(AgentNode::new(Arc::new(NoOpAggregator)))
        // Dependencies (identical to the unwired DAG).
        .add_dependency(
            AgentId::new("ner-extractor").unwrap(),
            AgentId::new("pii-redactor").unwrap(),
        )
        .add_dependency(
            AgentId::new("language-detector").unwrap(),
            AgentId::new("pii-redactor").unwrap(),
        )
        .add_dependency(
            AgentId::new("pii-redactor").unwrap(),
            AgentId::new("summary-writer").unwrap(),
        )
        .add_dependency(
            AgentId::new("ner-extractor").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("language-detector").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("schema-validator").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("sentiment-classifier").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .add_dependency(
            AgentId::new("summary-writer").unwrap(),
            AgentId::new("context-aggregator").unwrap(),
        )
        .with_aggregator(AgentId::new("context-aggregator").unwrap())
        .build()
}

/// **N22** : a trivial aggregator that completes without writing.
/// The SwarmOrchestrator builds the actual ContextBundle from the
/// per-agent outcomes ; the aggregator's role here is to be the
/// "sink" node satisfying the DAG invariant.
struct NoOpAggregator;

#[async_trait::async_trait]
impl SwarmAgent for NoOpAggregator {
    fn agent_id(&self) -> AgentId {
        AgentId::new("context-aggregator").unwrap()
    }

    async fn run(
        &self,
        ctx: SwarmAgentContext,
    ) -> Result<AgentOutcome, SwarmAgentError> {
        let _ = ctx;
        Ok(AgentOutcome::Completed {
            cost_microcents: 0,
            duration_ms: 1,
            blackboard_writes: 0,
            events_published: 0,
        })
    }
}

/// **N22** : seed the blackboard with the swarm input under the
/// canonical pointer `/input/text` (consumed by every pre-analysis
/// agent).  Returns the pointer for downstream agents.
pub fn seed_input_text(
    blackboard: &Blackboard,
    input: &serde_json::Value,
    agent_id: &AgentId,
) -> Result<(), efe_blackboard::BlackboardError> {
    // If the input is a string, write directly ; if a JSON object,
    // look for a `"text"` field ; else serialise.
    let text_value = if let Some(s) = input.as_str() {
        serde_json::Value::String(s.to_string())
    } else if let Some(s) = input.get("text").and_then(|v| v.as_str()) {
        serde_json::Value::String(s.to_string())
    } else {
        serde_json::Value::String(input.to_string())
    };
    blackboard.write("/input/text", text_value, agent_id.clone())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dag_builds() {
        let r = lookup_dag(Some(DEFAULT_DAG_ID));
        assert!(matches!(r, DagLookup::Found(_)));
    }

    #[test]
    fn empty_dag_id_returns_default() {
        let r = lookup_dag(Some(""));
        assert!(matches!(r, DagLookup::Found(_)));
    }

    #[test]
    fn none_dag_id_returns_default() {
        let r = lookup_dag(None);
        assert!(matches!(r, DagLookup::Found(_)));
    }

    #[test]
    fn unknown_dag_id_not_found() {
        let r = lookup_dag(Some("custom-dag-xyz"));
        assert!(matches!(r, DagLookup::NotFound));
    }

    #[test]
    fn default_dag_node_count_is_7() {
        match lookup_dag(None) {
            DagLookup::Found(d) => assert_eq!(d.node_count(), 7),
            _ => panic!("expected default DAG"),
        }
    }

    #[test]
    fn seed_input_text_from_string() {
        use efe_nano_sdk::SwarmId;
        let bb = Blackboard::new(SwarmId::new());
        let input = serde_json::json!("hello world");
        let agent = AgentId::new("seed").unwrap();
        seed_input_text(&bb, &input, &agent).unwrap();
        let v = bb.read("/input/text").unwrap();
        assert_eq!(v.value, serde_json::json!("hello world"));
    }

    #[test]
    fn seed_input_text_from_object_text_field() {
        use efe_nano_sdk::SwarmId;
        let bb = Blackboard::new(SwarmId::new());
        let input = serde_json::json!({"text": "hello", "extra": 1});
        let agent = AgentId::new("seed").unwrap();
        seed_input_text(&bb, &input, &agent).unwrap();
        assert_eq!(
            bb.read("/input/text").unwrap().value,
            serde_json::json!("hello")
        );
    }

    #[test]
    fn seed_input_text_from_unknown_object_serialises() {
        use efe_nano_sdk::SwarmId;
        let bb = Blackboard::new(SwarmId::new());
        let input = serde_json::json!({"foo": "bar"});
        let agent = AgentId::new("seed").unwrap();
        seed_input_text(&bb, &input, &agent).unwrap();
        // Falls back to stringified JSON.
        let v = bb.read("/input/text").unwrap();
        assert!(v.value.as_str().unwrap().contains("foo"));
    }
}
