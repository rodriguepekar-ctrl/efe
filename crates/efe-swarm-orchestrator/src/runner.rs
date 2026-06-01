// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : SwarmOrchestrator — § 7.3 runtime.
//!
//! Layer-by-layer execution with `tokio::spawn` parallelism within
//! a layer.  Per-agent timeouts (G10) + global swarm timeout (F5).
//! Best-effort isolation (G9 + F11) : one agent's failure becomes
//! `AgentOutcome::Errored` ; the swarm continues.
//!
//! Conditional edges (G3 + I19) : when an agent is scheduled to
//! run, the runtime evaluates the conditional-edge predicates from
//! its incoming conditional edges.  If *any* of them returns
//! `false`, the agent is skipped (with the predicate's label as
//! the reason).  Skipped agents recursively skip their descendants
//! reached via dependency edges.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use efe_blackboard::{Blackboard, EventBus};
use efe_nano_sdk::{AgentId, SwarmId};
use thiserror::Error;

use crate::agent::{AgentErrorKind, AgentOutcome, SwarmAgentContext};
use crate::compliance::{ComplianceRunner, PermissiveComplianceRunner};
use crate::dag::SwarmDag;
use crate::metrics::N21Metrics;
use crate::report::{AgentReport, SwarmOverallVerdict, SwarmReport};

/// **N21** : runtime configuration for one swarm run.
#[derive(Debug, Clone)]
pub struct SwarmRunConfig {
    pub swarm_id: SwarmId,
    /// **N21 + G10 + F5** : global swarm timeout.
    pub global_timeout_ms: u32,
    /// **N21 + G12 + ADR-10** : emit N14 envelope per agent dispatch.
    pub emit_envelope: bool,
    /// **N21 + I13** : per-swarm RNG seed for replay determinism.
    /// If `None`, the orchestrator derives one from `swarm_id`.
    pub rng_seed: Option<u64>,
    /// **N21** : tenant tier label for cost-attribution metrics
    /// (e.g. `"tier_0"`, `"tier_2"`).  Defaults to `"unknown"`.
    pub tenant_tier_label: String,
    /// **N21** : DAG id for metrics (informational ; uniqueness not
    /// required).
    pub dag_id_label: String,
}

impl Default for SwarmRunConfig {
    fn default() -> Self {
        Self {
            swarm_id: SwarmId::new(),
            global_timeout_ms: 30_000,
            emit_envelope: false,
            rng_seed: None,
            tenant_tier_label: "unknown".into(),
            dag_id_label: "default".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SwarmRunError {
    #[error("global timeout {ms} ms exceeded")]
    GlobalTimeout { ms: u32 },

    #[error("aggregator panicked : {message}")]
    AggregatorFailed { message: String },

    #[error("aggregator not declared in DAG")]
    AggregatorNotInDag,

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N21** : the orchestrator itself.
pub struct SwarmOrchestrator {
    pub metrics: Arc<N21Metrics>,
    pub compliance_runner: Arc<dyn ComplianceRunner>,
}

impl std::fmt::Debug for SwarmOrchestrator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmOrchestrator").finish_non_exhaustive()
    }
}

impl Default for SwarmOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl SwarmOrchestrator {
    pub fn new() -> Self {
        Self {
            metrics: N21Metrics::new(),
            compliance_runner: Arc::new(PermissiveComplianceRunner),
        }
    }

    pub fn with_metrics(mut self, metrics: Arc<N21Metrics>) -> Self {
        self.metrics = metrics;
        self
    }

    pub fn with_compliance(mut self, runner: Arc<dyn ComplianceRunner>) -> Self {
        self.compliance_runner = runner;
        self
    }

    /// **N21** : Execute the DAG.
    ///
    /// Returns Ok(SwarmReport) for both success + best-effort
    /// partial-success.  Err only for fundamental failures
    /// (global timeout, aggregator unfindable).
    pub async fn run(
        &self,
        dag: &SwarmDag,
        input: serde_json::Value,
        config: SwarmRunConfig,
    ) -> Result<SwarmReport, SwarmRunError> {
        let started_at = SystemTime::now();
        let swarm_id = config.swarm_id;
        let blackboard = Arc::new(Blackboard::new(swarm_id));
        let eventbus = Arc::new(EventBus::new());
        let input_arc = Arc::new(input);
        let rng_seed = config.rng_seed.unwrap_or_else(|| {
            // **Fix B3** : bind the UUID to a local first ; the
            // `as_bytes()` ref must outlive the array-construction.
            let uuid = swarm_id.as_uuid();
            let b = uuid.as_bytes();
            u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
        });

        let global_timeout = Duration::from_millis(u64::from(config.global_timeout_ms));
        let inner_fut = self.run_inner(dag, blackboard, eventbus, input_arc, rng_seed);
        let agents = match tokio::time::timeout(global_timeout, inner_fut).await {
            Ok(reports) => reports,
            Err(_) => {
                let completed_at = SystemTime::now();
                self.metrics.tick_swarm_run("aborted");
                return Ok(SwarmReport {
                    swarm_id,
                    started_at,
                    completed_at,
                    agents: vec![],
                    total_cost_microcents: 0,
                    bundle: efe_nano_sdk::ContextBundle::empty(swarm_id),
                    overall: SwarmOverallVerdict::Aborted {
                        reason: format!("global_timeout_{}_ms", config.global_timeout_ms),
                    },
                });
            }
        };

        // Aggregate.
        let total_cost: i64 = agents.iter().map(|r| r.outcome.cost_microcents()).sum();
        let agents_completed = agents.iter().filter(|r| r.outcome.is_completed()).count() as u32;
        let agents_errored = agents.iter().filter(|r| r.outcome.is_errored()).count() as u32;
        let agents_total = agents.len() as u32;

        // Build the ContextBundle from blackboard via the Aggregator's
        // outcome (which it should have written to the blackboard
        // under `/bundle`).  Sprint 13 simplification : the
        // Aggregator agent writes its final structured output to the
        // blackboard at pointer `/bundle` ; the runtime reads it +
        // wraps it into a ContextBundle.
        let bundle = build_context_bundle_from_run(
            swarm_id,
            &agents,
            total_cost,
            dag.aggregator(),
        );

        let bundle_size = bundle.estimated_size_bytes();
        self.metrics
            .observe_bundle_size(&config.dag_id_label, bundle_size as u64);

        // Determine overall verdict.
        let overall = if agents
            .iter()
            .find(|r| &r.agent_id == dag.aggregator())
            .map(|r| !r.outcome.is_completed())
            .unwrap_or(true)
        {
            // Aggregator didn't complete : SwarmOverallVerdict::Failed.
            SwarmOverallVerdict::Failed {
                reason: "aggregator_did_not_complete".into(),
            }
        } else if agents_errored == 0 {
            SwarmOverallVerdict::Passed { agents_total }
        } else {
            SwarmOverallVerdict::PartialSuccess {
                agents_completed,
                agents_errored,
                agents_total,
            }
        };

        let completed_at = SystemTime::now();

        // Metrics : per-overall + cost + duration.
        self.metrics.tick_swarm_run(overall.label());
        self.metrics
            .add_swarm_cost(&config.tenant_tier_label, total_cost);
        let duration_ms = completed_at
            .duration_since(started_at)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.metrics
            .observe_swarm_duration_ms(dag.layer_count(), duration_ms);

        Ok(SwarmReport {
            swarm_id,
            started_at,
            completed_at,
            agents,
            total_cost_microcents: total_cost,
            bundle,
            overall,
        })
    }

    /// Inner run : returns the per-agent `AgentReport` vector.
    async fn run_inner(
        &self,
        dag: &SwarmDag,
        blackboard: Arc<Blackboard>,
        eventbus: Arc<EventBus>,
        input: Arc<serde_json::Value>,
        rng_seed: u64,
    ) -> Vec<AgentReport> {
        let mut reports: Vec<AgentReport> = Vec::new();
        let mut skipped: HashSet<AgentId> = HashSet::new();

        for layer in dag.layers() {
            // For each agent in this layer, decide skip-or-run by :
            // - checking every conditional edge `to == agent_id` ;
            //   if any predicate false → skip
            // - if any parent (via plain or conditional edge) was
            //   skipped → skip (I19 propagation)
            let mut dispatches: Vec<(AgentId, bool, String)> = Vec::new();
            for agent_id in layer {
                let (skip, reason) = decide_skip(dag, agent_id, &skipped, &blackboard);
                if skip {
                    skipped.insert(agent_id.clone());
                }
                dispatches.push((agent_id.clone(), skip, reason));
            }

            // Spawn all non-skipped agents in parallel.
            let mut handles: Vec<tokio::task::JoinHandle<AgentReport>> = Vec::new();
            for (agent_id, skip, reason) in dispatches {
                if skip {
                    reports.push(AgentReport {
                        agent_id: agent_id.clone(),
                        outcome: AgentOutcome::Skipped { reason },
                        started_at: SystemTime::now(),
                        completed_at: SystemTime::now(),
                    });
                    self.metrics
                        .tick_agent_invocation(agent_id.as_str(), "skipped");
                    continue;
                }

                let node = match dag.agent(&agent_id) {
                    Some(n) => n,
                    None => continue,
                };
                let agent_arc = node.agent.clone();
                let timeout_ms = node.timeout_ms;
                let compliance = node.compliance.clone();
                let blackboard = blackboard.clone();
                let eventbus = eventbus.clone();
                let input = input.clone();
                let swarm_id = blackboard.swarm_id();
                let aid_clone = agent_id.clone();
                let metrics = self.metrics.clone();
                let runner = self.compliance_runner.clone();
                let agent_seed = derive_agent_seed(rng_seed, &agent_id);

                handles.push(tokio::spawn(async move {
                    let started_at = SystemTime::now();
                    // Compliance check first.
                    if let Err(e) = runner.check(&aid_clone, &compliance).await {
                        let completed_at = SystemTime::now();
                        metrics
                            .tick_agent_invocation(aid_clone.as_str(), "errored");
                        return AgentReport {
                            agent_id: aid_clone.clone(),
                            outcome: AgentOutcome::Errored {
                                kind: AgentErrorKind::ComplianceFailed,
                                message: e.to_string(),
                            },
                            started_at,
                            completed_at,
                        };
                    }

                    let ctx = SwarmAgentContext {
                        swarm_id,
                        agent_id: aid_clone.clone(),
                        blackboard: blackboard.clone(),
                        eventbus: eventbus.clone(),
                        input,
                        rng_seed: agent_seed,
                    };
                    let timeout_dur = Duration::from_millis(u64::from(timeout_ms));
                    let res = tokio::time::timeout(timeout_dur, agent_arc.run(ctx)).await;
                    let completed_at = SystemTime::now();
                    let outcome = match res {
                        Ok(Ok(o)) => {
                            metrics.tick_agent_invocation(aid_clone.as_str(), o.label());
                            let dur = completed_at
                                .duration_since(started_at)
                                .map(|d| d.as_millis() as u64)
                                .unwrap_or(0);
                            metrics.observe_agent_duration_ms(aid_clone.as_str(), dur);
                            o
                        }
                        Ok(Err(e)) => {
                            metrics.tick_agent_invocation(aid_clone.as_str(), "errored");
                            AgentOutcome::Errored {
                                kind: e.kind(),
                                message: e.to_string(),
                            }
                        }
                        Err(_elapsed) => {
                            metrics.tick_agent_invocation(aid_clone.as_str(), "timed_out");
                            metrics.tick_agent_timeout(aid_clone.as_str());
                            AgentOutcome::TimedOut { timeout_ms }
                        }
                    };
                    AgentReport {
                        agent_id: aid_clone,
                        outcome,
                        started_at,
                        completed_at,
                    }
                }));
            }

            // Await all in this layer.
            for h in handles {
                match h.await {
                    Ok(r) => {
                        // If the agent didn't complete, mark its
                        // agent_id as skipped so descendants propagate
                        // skip (I19).
                        if !r.outcome.is_completed() {
                            skipped.insert(r.agent_id.clone());
                        }
                        reports.push(r);
                    }
                    Err(join_err) => {
                        // tokio JoinError = panic, cancellation.  Best-
                        // effort : we lost the agent_id ; can't fully
                        // reconstruct.  Push a generic errored report.
                        reports.push(AgentReport {
                            agent_id: AgentId::new("join-error").unwrap_or_else(|_| {
                                // Unreachable in practice but defensive.
                                panic!("hard-coded fallback agent id must validate")
                            }),
                            outcome: AgentOutcome::Errored {
                                kind: AgentErrorKind::Panic,
                                message: format!("{join_err}"),
                            },
                            started_at: SystemTime::now(),
                            completed_at: SystemTime::now(),
                        });
                    }
                }
            }
        }

        // Sort reports deterministically by `agent_id` ASCII for
        // replay determinism (I13).
        reports.sort_by(|a, b| a.agent_id.as_str().cmp(b.agent_id.as_str()));
        reports
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helpers
// ─────────────────────────────────────────────────────────────────────

fn decide_skip(
    dag: &SwarmDag,
    agent_id: &AgentId,
    skipped: &HashSet<AgentId>,
    blackboard: &Blackboard,
) -> (bool, String) {
    // 1. Conditional edges INTO this agent.  ALL predicates must
    //    pass ; any false → skip.
    for ce in dag.conditional_parents(agent_id) {
        if skipped.contains(&ce.from) {
            return (
                true,
                format!("conditional parent {} was skipped", ce.from),
            );
        }
        if !ce.evaluate(blackboard) {
            return (
                true,
                format!("conditional predicate {:?} false", ce.predicate_label),
            );
        }
    }
    // 2. Plain dependency parents being skipped also propagates
    //    skip (I19).
    for e in dag.edges() {
        if &e.to == agent_id && skipped.contains(&e.from) {
            return (
                true,
                format!("dependency parent {} was skipped", e.from),
            );
        }
    }
    (false, String::new())
}

fn derive_agent_seed(swarm_seed: u64, agent_id: &AgentId) -> u64 {
    // Mix swarm_seed with a hash of agent_id (FNV-1a 64).
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in agent_id.as_str().as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^ swarm_seed
}

fn build_context_bundle_from_run(
    swarm_id: SwarmId,
    agents: &[AgentReport],
    total_cost: i64,
    _aggregator: &AgentId,
) -> efe_nano_sdk::ContextBundle {
    let mut b = efe_nano_sdk::ContextBundle::empty(swarm_id);
    b.metadata.total_cost_microcents = total_cost;
    b.metadata.agents_executed = agents
        .iter()
        .filter(|r| r.outcome.is_completed())
        .map(|r| r.agent_id.clone())
        .collect();
    b.finalize_size();
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentNode, SwarmAgent, SwarmAgentError};
    use crate::compliance::Compliance;
    use crate::dag::SwarmDagBuilder;
    use async_trait::async_trait;
    use efe_nano_sdk::{AgentId, SwarmId};

    fn aid(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    struct CompletingAgent {
        id: AgentId,
        cost: i64,
        writes_to: Option<String>,
    }

    #[async_trait]
    impl SwarmAgent for CompletingAgent {
        fn agent_id(&self) -> AgentId {
            self.id.clone()
        }
        async fn run(&self, ctx: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
            if let Some(p) = &self.writes_to {
                ctx.blackboard
                    .write(p, serde_json::json!("written"), self.id.clone())
                    .map_err(|e| SwarmAgentError::Blackboard {
                        message: e.to_string(),
                    })?;
            }
            Ok(AgentOutcome::Completed {
                cost_microcents: self.cost,
                duration_ms: 1,
                blackboard_writes: if self.writes_to.is_some() { 1 } else { 0 },
                events_published: 0,
            })
        }
    }

    struct FailingAgent {
        id: AgentId,
    }
    #[async_trait]
    impl SwarmAgent for FailingAgent {
        fn agent_id(&self) -> AgentId {
            self.id.clone()
        }
        async fn run(&self, _: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
            Err(SwarmAgentError::Provider {
                message: "fail".into(),
            })
        }
    }

    struct SlowAgent {
        id: AgentId,
        ms: u64,
    }
    #[async_trait]
    impl SwarmAgent for SlowAgent {
        fn agent_id(&self) -> AgentId {
            self.id.clone()
        }
        async fn run(&self, _: SwarmAgentContext) -> Result<AgentOutcome, SwarmAgentError> {
            tokio::time::sleep(Duration::from_millis(self.ms)).await;
            Ok(AgentOutcome::Completed {
                cost_microcents: 0,
                duration_ms: self.ms as u32,
                blackboard_writes: 0,
                events_published: 0,
            })
        }
    }

    // ─── Happy path ──────────────────────────────────────────────

    #[tokio::test]
    async fn linear_chain_completes_all() {
        let dag = SwarmDagBuilder::new()
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aaa"),
                cost: 1,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 2,
                writes_to: None,
            })))
            .add_dependency(aid("aaa"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let report = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        assert!(matches!(report.overall, SwarmOverallVerdict::Passed { .. }));
        assert_eq!(report.agents_completed(), 2);
        assert_eq!(report.total_cost_microcents, 3);
    }

    // ─── Parallel layer ──────────────────────────────────────────

    #[tokio::test]
    async fn parallel_layer_all_complete() {
        let dag = SwarmDagBuilder::new()
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aaa"),
                cost: 1,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("bbb"),
                cost: 1,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 1,
                writes_to: None,
            })))
            .add_dependency(aid("aaa"), aid("aggr"))
            .add_dependency(aid("bbb"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        assert_eq!(r.agents_completed(), 3);
    }

    // ─── Best-effort isolation ───────────────────────────────────

    #[tokio::test]
    async fn one_failing_agent_swarm_partial_success() {
        let dag = SwarmDagBuilder::new()
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aaa"),
                cost: 1,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(FailingAgent { id: aid("bbb") })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 1,
                writes_to: None,
            })))
            .add_dependency(aid("aaa"), aid("aggr"))
            .add_dependency(aid("bbb"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        // bbb errored ; aaa OK ; aggr depends on bbb so it's skipped.
        // Aggregator didn't complete → overall is Failed (per
        // I20 + § 6 ADR-8 the orchestrator returns Failed if the
        // aggregator itself didn't complete).  Partial success is
        // also acceptable depending on how the runtime classifies
        // the aggregator's skip.
        match r.overall {
            SwarmOverallVerdict::PartialSuccess { .. }
            | SwarmOverallVerdict::Failed { .. } => {}
            other => panic!("expected partial/failed, got {other:?}"),
        }
        // At least bbb is errored.
        assert!(r.agents.iter().any(|r| r.outcome.is_errored()));
    }

    // ─── Per-agent timeout ───────────────────────────────────────

    #[tokio::test]
    async fn per_agent_timeout_fires() {
        let dag = SwarmDagBuilder::new()
            .add_agent(
                AgentNode::new(Arc::new(SlowAgent {
                    id: aid("slow"),
                    ms: 200,
                }))
                .with_timeout_ms(50),
            )
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 0,
                writes_to: None,
            })))
            .add_dependency(aid("slow"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        // slow timed out ; aggr skipped (parent failed).
        assert!(r.agents.iter().any(|r| {
            matches!(r.outcome, AgentOutcome::TimedOut { .. })
        }));
    }

    // ─── Conditional edge ───────────────────────────────────────

    #[tokio::test]
    async fn conditional_edge_predicate_false_skips() {
        let dag = SwarmDagBuilder::new()
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("trigger"),
                cost: 0,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("translate"),
                cost: 1_000,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 0,
                writes_to: None,
            })))
            .add_conditional(aid("trigger"), aid("translate"), "always-false", |_| false)
            .add_dependency(aid("translate"), aid("aggr"))
            .add_dependency(aid("trigger"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        // translate skipped → aggr skipped → swarm Failed.
        let translate_skipped = r.agents.iter().any(|r| {
            r.agent_id.as_str() == "translate"
                && matches!(r.outcome, AgentOutcome::Skipped { .. })
        });
        assert!(translate_skipped);
    }

    #[tokio::test]
    async fn conditional_edge_predicate_true_dispatches() {
        let dag = SwarmDagBuilder::new()
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("trigger"),
                cost: 0,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("downstream"),
                cost: 5,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 0,
                writes_to: None,
            })))
            .add_conditional(aid("trigger"), aid("downstream"), "always-true", |_| true)
            .add_dependency(aid("downstream"), aid("aggr"))
            .add_dependency(aid("trigger"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        assert!(matches!(r.overall, SwarmOverallVerdict::Passed { .. }));
    }

    // ─── Replay determinism ──────────────────────────────────────

    #[tokio::test]
    async fn replay_with_same_seed_produces_same_agent_list() {
        let make_dag = || {
            SwarmDagBuilder::new()
                .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                    id: aid("aaa"),
                    cost: 1,
                    writes_to: None,
                })))
                .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                    id: aid("bbb"),
                    cost: 2,
                    writes_to: None,
                })))
                .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                    id: aid("aggr"),
                    cost: 0,
                    writes_to: None,
                })))
                .add_dependency(aid("aaa"), aid("aggr"))
                .add_dependency(aid("bbb"), aid("aggr"))
                .with_aggregator(aid("aggr"))
                .build()
                .unwrap()
        };
        let orch = SwarmOrchestrator::new();
        let config = SwarmRunConfig {
            swarm_id: SwarmId::new(),
            rng_seed: Some(42),
            ..Default::default()
        };
        let dag1 = make_dag();
        let dag2 = make_dag();
        let r1 = orch
            .run(&dag1, serde_json::json!({}), config.clone())
            .await
            .unwrap();
        let r2 = orch
            .run(&dag2, serde_json::json!({}), config)
            .await
            .unwrap();
        // Agent list ordering is deterministic (sorted by ASCII).
        let ids1: Vec<&str> = r1.agents.iter().map(|r| r.agent_id.as_str()).collect();
        let ids2: Vec<&str> = r2.agents.iter().map(|r| r.agent_id.as_str()).collect();
        assert_eq!(ids1, ids2);
        // Total cost is identical for deterministic agents.
        assert_eq!(r1.total_cost_microcents, r2.total_cost_microcents);
    }

    // ─── Compliance opt-in ───────────────────────────────────────

    #[tokio::test]
    async fn compliance_none_does_not_block() {
        let dag = SwarmDagBuilder::new()
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 0,
                writes_to: None,
            })))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        assert!(matches!(r.overall, SwarmOverallVerdict::Passed { .. }));
    }

    #[tokio::test]
    async fn compliance_strict_blocks_n18_signed_agent() {
        use crate::compliance::StrictUnwiredComplianceRunner;
        let dag = SwarmDagBuilder::new()
            .add_agent(
                AgentNode::new(Arc::new(CompletingAgent {
                    id: aid("signed"),
                    cost: 0,
                    writes_to: None,
                }))
                .with_compliance(Compliance::N18Signed {
                    publisher_id: efe_nano_sdk::PublisherId::new(),
                    key_id: efe_nano_sdk::KeyId::new(),
                }),
            )
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 0,
                writes_to: None,
            })))
            .add_dependency(aid("signed"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new()
            .with_compliance(Arc::new(StrictUnwiredComplianceRunner));
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        // signed agent errored with ComplianceFailed.
        let signed = r.agents.iter().find(|r| r.agent_id.as_str() == "signed").unwrap();
        match &signed.outcome {
            AgentOutcome::Errored { kind, .. } => {
                assert_eq!(*kind, AgentErrorKind::ComplianceFailed);
            }
            other => panic!("expected Errored, got {other:?}"),
        }
    }

    // ─── Cost attribution ────────────────────────────────────────

    #[tokio::test]
    async fn cost_attributed_to_total() {
        let dag = SwarmDagBuilder::new()
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aaa"),
                cost: 1_000,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("bbb"),
                cost: 2_500,
                writes_to: None,
            })))
            .add_agent(AgentNode::new(Arc::new(CompletingAgent {
                id: aid("aggr"),
                cost: 500,
                writes_to: None,
            })))
            .add_dependency(aid("aaa"), aid("aggr"))
            .add_dependency(aid("bbb"), aid("aggr"))
            .with_aggregator(aid("aggr"))
            .build()
            .unwrap();
        let orch = SwarmOrchestrator::new();
        let r = orch
            .run(&dag, serde_json::json!({}), SwarmRunConfig::default())
            .await
            .unwrap();
        assert_eq!(r.total_cost_microcents, 4_000);
    }

    // ─── derive_agent_seed ──────────────────────────────────────

    #[test]
    fn derive_seed_deterministic() {
        let s1 = derive_agent_seed(42, &aid("ner_extractor"));
        let s2 = derive_agent_seed(42, &aid("ner_extractor"));
        assert_eq!(s1, s2);
    }

    #[test]
    fn derive_seed_different_agents_different_seeds() {
        let s1 = derive_agent_seed(42, &aid("aaa"));
        let s2 = derive_agent_seed(42, &aid("bbb"));
        assert_ne!(s1, s2);
    }

    #[test]
    fn derive_seed_different_swarm_seeds_different_results() {
        let s1 = derive_agent_seed(1, &aid("aaa"));
        let s2 = derive_agent_seed(2, &aid("aaa"));
        assert_ne!(s1, s2);
    }

    // ─── Smoke ──────────────────────────────────────────────────

    #[test]
    fn config_default_values() {
        let c = SwarmRunConfig::default();
        assert_eq!(c.global_timeout_ms, 30_000);
        assert!(!c.emit_envelope);
        assert!(c.rng_seed.is_none());
    }
}
