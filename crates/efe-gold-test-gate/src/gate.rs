// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : GoldGate orchestrator — § 7.7.
//!
//! Wires the four components (registry, runner, assertions, ledger)
//! plus metrics into the canonical `promote_to_tier_1` flow.
//!
//! Run lifecycle :
//!
//! ```text
//! 1. Resolve the GoldSet (must be Approved, hash-match the agent).
//! 2. For each case (under a per-agent in-progress mutex F15):
//!    a. Look up the expected value (Inline or N16 ReplayAnchor).
//!    b. Runner.run_case → captured output + cost + provider calls.
//!    c. AssertionEngine.evaluate → verdict.
//!    d. Tick the running cost ; if cap exceeded, abort.
//! 3. Aggregate the report.
//! 4. Append GoldRunCompleted to the ledger.
//! 5. Tick metrics.
//! ```
//!
//! Bypass shortcut : when `bypass = Some(cert)`, the runner is
//! never invoked ; instead a `GoldRunBypassed` ledger entry is
//! appended and the verdict is `Passed` (with `bypass = Some` on
//! the report so downstream consumers can mark the agent as
//! flagged).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use dashmap::DashMap;
use efe_nano_sdk::{
    BypassCertificate, GoldCase, GoldCaseEntry, GoldCaseId, GoldCaseRun, GoldCaseVerdict,
    GoldErrorKind, GoldExpectedOutput, GoldGateError, GoldOverallVerdict, GoldReport,
    GoldRunId, GoldSet, NanoAgent, NanoAgentId,
};

use crate::assertion::AssertionEngine;
use crate::ledger::{GoldRunLedger, GoldRunLedgerEntry};
use crate::metrics::N19Metrics;
use crate::registry::GoldSetRegistry;
use crate::runner::{GoldRunner, RunnerInput};

/// **N19** : tunable knobs for `GoldGate`.
#[derive(Debug, Clone)]
pub struct GoldGateConfig {
    /// Hard cap on cumulative cost (USD) ; if `None`, falls back to
    /// the GoldSet's `cost_cap_usd`.
    pub cost_cap_override_usd: Option<f64>,
    /// Stable label of the agent's tier used to tag metrics
    /// (e.g. `"tier_1"`).  Defaults to `"tier_experimental"` if not
    /// set.
    pub agent_tier_label: String,
    /// Allow `BypassCertificate.valid_until <= now`?  Default false.
    pub allow_expired_bypass: bool,
}

impl Default for GoldGateConfig {
    fn default() -> Self {
        Self {
            cost_cap_override_usd: None,
            agent_tier_label: "tier_experimental".into(),
            allow_expired_bypass: false,
        }
    }
}

/// **N19** : the orchestrator.  Held by `Arc` ; all inner components
/// are `Arc<dyn Trait>` for dependency injection.
pub struct GoldGate {
    pub registry: Arc<dyn GoldSetRegistry>,
    pub runner: Arc<dyn GoldRunner>,
    pub assertions: Arc<dyn AssertionEngine>,
    pub ledger: Arc<dyn GoldRunLedger>,
    pub metrics: Arc<N19Metrics>,
    /// Per-agent in-progress flag.  Prevents concurrent gold runs
    /// on the same agent_id (F15).  Holds the run_id of the
    /// in-flight run for diagnostic surfacing.
    in_progress: DashMap<NanoAgentId, GoldRunId>,
    /// Coordinates the in-progress check against the actual run
    /// (so the check + set + clear are atomic at the trait level).
    in_progress_lock: Mutex<()>,
}

impl std::fmt::Debug for GoldGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GoldGate")
            .field("in_progress", &self.in_progress.len())
            .finish_non_exhaustive()
    }
}

impl GoldGate {
    pub fn new(
        registry: Arc<dyn GoldSetRegistry>,
        runner: Arc<dyn GoldRunner>,
        assertions: Arc<dyn AssertionEngine>,
        ledger: Arc<dyn GoldRunLedger>,
        metrics: Arc<N19Metrics>,
    ) -> Self {
        Self {
            registry,
            runner,
            assertions,
            ledger,
            metrics,
            in_progress: DashMap::new(),
            in_progress_lock: Mutex::new(()),
        }
    }

    /// **N19 + G6** : Main entry point.
    pub async fn run(
        &self,
        agent: Arc<NanoAgent>,
        gold_set: GoldSet,
        bypass: Option<BypassCertificate>,
        config: &GoldGateConfig,
    ) -> Result<GoldReport, GoldGateError> {
        let agent_id = agent.id.clone();
        let agent_version = agent.version.clone();

        // **Post-audit fix B1** : The gold set MUST belong to this
        // agent + version.  Without this check, the caller could pass
        // any approved set and the gate would execute it, bypassing
        // the agent-specific quality bar entirely.
        if gold_set.agent_id != agent_id {
            return Err(GoldGateError::Backend {
                message: format!(
                    "gold set agent_id {:?} != agent.id {:?}",
                    gold_set.agent_id, agent_id
                ),
            });
        }
        if gold_set.agent_version != agent_version {
            return Err(GoldGateError::Backend {
                message: format!(
                    "gold set agent_version {:?} != agent.version {:?}",
                    gold_set.agent_version, agent_version
                ),
            });
        }

        // **Post-audit fix B2** : Verify the gold set's content hash
        // matches what's stored on the set (F6 protection against
        // tampered registry rows).  We compute the canonical body
        // hash from the set's fields and compare against
        // `gold_set.gold_set_hash`.  If they don't match, abort
        // before running any cases.
        let computed = gold_set.compute_canonical_hash();
        if computed != gold_set.gold_set_hash {
            return Err(GoldGateError::Backend {
                message: format!(
                    "gold set hash mismatch : computed {} != stored {}",
                    computed.to_hex(),
                    gold_set.gold_set_hash.to_hex()
                ),
            });
        }

        // F15 : per-agent serialisation.
        let run_id = GoldRunId::new();
        {
            let _guard = self
                .in_progress_lock
                .lock()
                .map_err(|e| GoldGateError::Backend {
                    message: e.to_string(),
                })?;
            if let Some(existing) = self.in_progress.get(&agent_id) {
                return Err(GoldGateError::RunAlreadyInProgress {
                    gold_run_id: *existing.value(),
                });
            }
            self.in_progress.insert(agent_id.clone(), run_id);
        }

        // Execute under a guard that always clears the flag, even
        // on early returns.
        let result = self
            .run_inner(
                run_id,
                agent.as_ref(),
                &gold_set,
                bypass,
                config,
            )
            .await;
        self.in_progress.remove(&agent_id);

        let report = result?;
        // Append the ledger entry (G3 + G14).  Bypassed runs have
        // already written their dedicated `GoldRunBypassed` row in
        // `run_bypass` ; don't double-append.
        if !report.is_bypassed() {
            self.ledger
                .append(GoldRunLedgerEntry::GoldRunCompleted {
                    report: Box::new(report.clone()),
                })
                .await
                .map_err(|e| GoldGateError::Ledger {
                    message: e.to_string(),
                })?;
        }
        // Tick aggregate metrics (G15).
        self.metrics
            .tick_gold_run(report.overall.label(), &config.agent_tier_label);
        self.metrics
            .add_cost(&config.agent_tier_label, report.total_cost_usd * 100.0);
        self.metrics.observe_run_duration(
            &config.agent_tier_label,
            report.duration_ms() as f64 / 1_000.0,
        );
        // Tier label suppression to avoid clippy unused warning on
        // future enum extension.
        let _ = &agent_version;
        Ok(report)
    }

    async fn run_inner(
        &self,
        run_id: GoldRunId,
        agent: &NanoAgent,
        gold_set: &GoldSet,
        bypass: Option<BypassCertificate>,
        config: &GoldGateConfig,
    ) -> Result<GoldReport, GoldGateError> {
        // Bypass shortcut.
        if let Some(cert) = bypass {
            return self.run_bypass(run_id, agent, gold_set, cert, config).await;
        }

        // Approval gate (G2).
        if !gold_set.is_executable() {
            return Err(GoldGateError::GoldSetNotApproved);
        }

        // Schema hash gate (F8).  The runner caller is responsible
        // for supplying the agent's current schema hashes.  Sprint
        // 11 : assume the registry's stored hashes are authoritative
        // and trust the caller to pass the right agent.

        let cost_cap = config.cost_cap_override_usd.unwrap_or(gold_set.cost_cap_usd);
        let started_at = SystemTime::now();
        let mut total_cost_usd = 0.0_f64;
        let mut entries: Vec<GoldCaseEntry> = Vec::with_capacity(gold_set.cases.len());
        let mut overall: Option<GoldOverallVerdict> = None;

        for case in &gold_set.cases {
            // Cost gate (F4).
            if total_cost_usd > cost_cap {
                overall = Some(GoldOverallVerdict::AbortedCostCap {
                    cost_actual_usd: total_cost_usd,
                    cost_cap_usd: cost_cap,
                });
                // Mark remaining cases skipped.
                entries.push(skipped_entry(case, "cost cap reached"));
                continue;
            }
            // Resolve expected output (G9 replay-anchor handling).
            let expected = match &case.expected {
                GoldExpectedOutput::Inline { value } => value.clone(),
                GoldExpectedOutput::ReplayAnchor { .. } => {
                    // Sprint 11 stub : ReplayAnchor not wired ; we
                    // surface as Errored ReplayAnchorMissing.  Sprint
                    // 12 wires the real N16 lookup.
                    self.metrics.tick_replay_lookup("miss");
                    entries.push(GoldCaseEntry {
                        case: case.clone(),
                        run: GoldCaseRun {
                            case_id: case.gold_case_id,
                            trace_id: uuid::Uuid::nil(),
                            captured_output: serde_json::Value::Null,
                            duration_ms: 0,
                            cost_usd: 0.0,
                            provider_calls: vec![],
                            errored: Some("replay anchor not yet wired".into()),
                        },
                        verdict: GoldCaseVerdict::Errored {
                            kind: GoldErrorKind::ReplayAnchorMissing,
                            message: "Sprint 11 stub : replay anchor unavailable".into(),
                        },
                    });
                    self.metrics.tick_gold_case(
                        case.assertion.label(),
                        GoldCaseVerdict::Errored {
                            kind: GoldErrorKind::ReplayAnchorMissing,
                            message: String::new(),
                        }
                        .label(),
                    );
                    self.metrics.tick_assertion(
                        case.assertion.label(),
                        "errored",
                    );
                    continue;
                }
            };
            // Execute the case.
            let runner_input = RunnerInput {
                agent: Arc::new(agent.clone()),
                case: case.clone(),
                frozen_clock_utc: started_at,
            };
            let case_run = match self.runner.run_case(runner_input).await {
                Ok(r) => r,
                Err(err) => match err {
                    crate::runner::GoldRunnerError::HermeticityBroken { case_id } => {
                        self.metrics.tick_hermeticity("network_leak");
                        overall = Some(GoldOverallVerdict::AbortedHermeticityBroken {
                            case_id,
                        });
                        entries.push(error_entry(
                            case,
                            GoldErrorKind::HermeticityBroken,
                            "network leak",
                        ));
                        break;
                    }
                    crate::runner::GoldRunnerError::Timeout { seconds } => {
                        entries.push(error_entry(
                            case,
                            GoldErrorKind::Timeout,
                            &format!("{seconds}s"),
                        ));
                        self.metrics
                            .tick_gold_case(case.assertion.label(), "errored");
                        self.metrics
                            .tick_assertion(case.assertion.label(), "errored");
                        continue;
                    }
                    crate::runner::GoldRunnerError::VmCrash { message } => {
                        entries.push(error_entry(
                            case,
                            GoldErrorKind::RunnerPanic,
                            &message,
                        ));
                        self.metrics
                            .tick_gold_case(case.assertion.label(), "errored");
                        self.metrics
                            .tick_assertion(case.assertion.label(), "errored");
                        continue;
                    }
                    crate::runner::GoldRunnerError::Backend { message } => {
                        entries.push(error_entry(
                            case,
                            GoldErrorKind::Backend,
                            &message,
                        ));
                        self.metrics
                            .tick_gold_case(case.assertion.label(), "errored");
                        self.metrics
                            .tick_assertion(case.assertion.label(), "errored");
                        continue;
                    }
                },
            };
            total_cost_usd += case_run.cost_usd;
            self.metrics.observe_case_duration(
                case.assertion.label(),
                case_run.duration_ms as f64 / 1_000.0,
            );
            // Runner-side error (case-level, not run-aborting).
            if let Some(msg) = &case_run.errored {
                entries.push(GoldCaseEntry {
                    case: case.clone(),
                    run: case_run.clone(),
                    verdict: GoldCaseVerdict::Errored {
                        kind: GoldErrorKind::RunnerPanic,
                        message: msg.clone(),
                    },
                });
                self.metrics
                    .tick_gold_case(case.assertion.label(), "errored");
                self.metrics
                    .tick_assertion(case.assertion.label(), "errored");
                continue;
            }
            // Assertion.
            let verdict = match self
                .assertions
                .evaluate(case, &case_run.captured_output, &expected)
                .await
            {
                Ok(v) => v,
                Err(e) => GoldCaseVerdict::Errored {
                    kind: GoldErrorKind::Backend,
                    message: e.to_string(),
                },
            };
            let label = verdict.label();
            self.metrics.tick_gold_case(case.assertion.label(), label);
            self.metrics.tick_assertion(case.assertion.label(), label);
            entries.push(GoldCaseEntry {
                case: case.clone(),
                run: case_run,
                verdict,
            });
        }

        // Final cost-cap check (covers the last-case-pushed-us-over
        // scenario).
        if total_cost_usd > cost_cap && overall.is_none() {
            overall = Some(GoldOverallVerdict::AbortedCostCap {
                cost_actual_usd: total_cost_usd,
                cost_cap_usd: cost_cap,
            });
        }
        if overall.is_none() {
            overall = Some(aggregate_overall(&entries));
        }
        let completed_at = SystemTime::now();
        Ok(GoldReport {
            gold_run_id: run_id,
            gold_set_id: gold_set.gold_set_id,
            agent_id: agent.id.clone(),
            agent_version: agent.version.clone(),
            gold_set_hash: gold_set.gold_set_hash,
            started_at,
            completed_at,
            cases: entries,
            overall: overall.expect("overall verdict set above"),
            total_cost_usd,
            cost_cap_usd: cost_cap,
            bypass: None,
        })
    }

    async fn run_bypass(
        &self,
        run_id: GoldRunId,
        agent: &NanoAgent,
        gold_set: &GoldSet,
        cert: BypassCertificate,
        config: &GoldGateConfig,
    ) -> Result<GoldReport, GoldGateError> {
        // Validate the certificate shape first (cheap rejects).
        cert.validate_shape()?;
        // Validate the lifetime window.
        let now = SystemTime::now();
        let max_lifetime = std::time::Duration::from_secs(
            efe_nano_sdk::BYPASS_MAX_LIFETIME_DAYS * 86_400,
        );
        if !config.allow_expired_bypass {
            if cert.valid_until <= now {
                return Err(GoldGateError::BypassInvalidLifetime);
            }
            if cert.valid_until > now + max_lifetime {
                return Err(GoldGateError::BypassInvalidLifetime);
            }
        }
        // (Sprint 12 wires real cryptographic verification of each
        // OperatorSignature via N18's SignatureVerifier.  Sprint 11
        // accepts the shape-validated certificate as-is.)

        // Build a "trivially passed" report tagged with the bypass.
        let started_at = SystemTime::now();
        let completed_at = started_at;
        let report = GoldReport {
            gold_run_id: run_id,
            gold_set_id: gold_set.gold_set_id,
            agent_id: agent.id.clone(),
            agent_version: agent.version.clone(),
            gold_set_hash: gold_set.gold_set_hash,
            started_at,
            completed_at,
            cases: vec![],
            overall: GoldOverallVerdict::Passed {
                cases_passed: 0,
                cases_total: 0,
            },
            total_cost_usd: 0.0,
            cost_cap_usd: 0.0,
            bypass: Some(cert.clone()),
        };
        // Append a dedicated bypass ledger row (audit trail).
        self.ledger
            .append(GoldRunLedgerEntry::GoldRunBypassed {
                gold_run_id: run_id,
                agent_id: agent.id.clone(),
                agent_version: agent.version.clone(),
                certificate: cert.clone(),
            })
            .await
            .map_err(|e| GoldGateError::Ledger {
                message: e.to_string(),
            })?;
        let ticket_class = ticket_class(&cert.ticket_id);
        self.metrics
            .tick_bypass(&config.agent_tier_label, ticket_class);
        Ok(report)
    }
}

fn ticket_class(ticket_id: &str) -> &'static str {
    if ticket_id.starts_with("INC-") {
        "INC"
    } else if ticket_id.starts_with("SEC-") {
        "SEC"
    } else if ticket_id.starts_with("GOLD-") {
        "GOLD"
    } else {
        "other"
    }
}

fn aggregate_overall(entries: &[GoldCaseEntry]) -> GoldOverallVerdict {
    let total = entries.len() as u32;
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errored = 0u32;
    for e in entries {
        match &e.verdict {
            GoldCaseVerdict::Passed { .. } => passed += 1,
            GoldCaseVerdict::Failed { .. } => failed += 1,
            GoldCaseVerdict::Errored { .. } => errored += 1,
            GoldCaseVerdict::Skipped { .. } => {}
        }
    }
    if errored > 0 && errored as f64 / total.max(1) as f64 >= 0.5 {
        return GoldOverallVerdict::Errored {
            error_count: errored,
            cases_total: total,
        };
    }
    if failed > 0 || errored > 0 {
        return GoldOverallVerdict::Failed {
            cases_passed: passed,
            cases_failed: failed + errored,
            cases_total: total,
        };
    }
    GoldOverallVerdict::Passed {
        cases_passed: passed,
        cases_total: total,
    }
}

fn skipped_entry(case: &GoldCase, reason: &str) -> GoldCaseEntry {
    GoldCaseEntry {
        case: case.clone(),
        run: GoldCaseRun {
            case_id: case.gold_case_id,
            trace_id: uuid::Uuid::nil(),
            captured_output: serde_json::Value::Null,
            duration_ms: 0,
            cost_usd: 0.0,
            provider_calls: vec![],
            errored: None,
        },
        verdict: GoldCaseVerdict::Skipped {
            reason: reason.into(),
        },
    }
}

fn error_entry(case: &GoldCase, kind: GoldErrorKind, message: &str) -> GoldCaseEntry {
    GoldCaseEntry {
        case: case.clone(),
        run: GoldCaseRun {
            case_id: case.gold_case_id,
            trace_id: uuid::Uuid::nil(),
            captured_output: serde_json::Value::Null,
            duration_ms: 0,
            cost_usd: 0.0,
            provider_calls: vec![],
            errored: Some(message.into()),
        },
        verdict: GoldCaseVerdict::Errored {
            kind,
            message: message.into(),
        },
    }
}

// Compiler hint to keep `HashMap` import live (used in cross-cutting
// tests below).
#[allow(dead_code)]
fn _hashmap_keep_alive() -> HashMap<(), ()> {
    HashMap::new()
}

// Suppress an unused-import lint on `GoldCaseId` if Sprint 12 widens
// the bypass certificate.
#[allow(dead_code)]
fn _gold_case_id_keep_alive() -> Option<GoldCaseId> {
    None
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DefaultAssertionEngine, InMemoryGoldRunLedger, InMemoryGoldRunner,
        InMemoryGoldSetRegistry, StubEmbeddingProvider,
    };
    use efe_nano_sdk::{
        Author, Category, DeterminismLevel, ExecutionMode, GoldAssertionStrategy,
        GoldCaseId, GoldExpectedOutput, GoldSetId, GoldSetStatus, IdempotencyPolicy,
        ModelIdentity, NanoAgentVersion, OperatorSignature, Provenance, ResourceLimits,
        Sha256Hash, StabilityTier,
    };
    use std::sync::Arc;

    fn agent(name: &str) -> Arc<NanoAgent> {
        Arc::new(NanoAgent {
            id: NanoAgentId::new(name).unwrap(),
            version: NanoAgentVersion::new("1.0.0").unwrap(),
            description: "x".into(),
            author: Author::Efe,
            category: Category::Validation,
            stability_tier: StabilityTier::TierExperimental,
            execution_mode: ExecutionMode::PureRust {
                entry: "x::run".into(),
            },
            determinism: DeterminismLevel::FullDeterministic,
            idempotency: IdempotencyPolicy::auto_24h(),
            input_schema: serde_json::json!({}),
            output_schema: serde_json::json!({}),
            error_schema: None,
            limits: ResourceLimits::default_mode_a(),
            cost_estimate_cents: 0.01,
            latency_p99_ms_target: 50,
            audit: Default::default(),
            compliance: Default::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        })
    }

    fn case_strict(label: &str, expected_value: serde_json::Value) -> GoldCase {
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: label.into(),
            input: serde_json::json!({}),
            expected: GoldExpectedOutput::Inline {
                value: expected_value,
            },
            assertion: GoldAssertionStrategy::StrictEquality,
            timeout_seconds: 30,
            tags: vec![],
        }
    }

    fn gold_set_5(agent_id: &NanoAgentId) -> GoldSet {
        GoldSet {
            gold_set_id: GoldSetId::new(),
            agent_id: agent_id.clone(),
            agent_version: NanoAgentVersion::new("1.0.0").unwrap(),
            agent_input_schema_hash: Sha256Hash::from_bytes([1u8; 32]),
            agent_output_schema_hash: Sha256Hash::from_bytes([2u8; 32]),
            cases: vec![
                case_strict("c1", serde_json::json!({"o": 1})),
                case_strict("c2", serde_json::json!({"o": 2})),
                case_strict("c3", serde_json::json!({"o": 3})),
                case_strict("c4", serde_json::json!({"o": 4})),
                case_strict("c5", serde_json::json!({"o": 5})),
            ],
            cost_cap_usd: 100.0,
            provenance: Provenance::Manual,
            status: GoldSetStatus::Approved,
            approvers: vec![op_sig(), op_sig()],
            created_at: SystemTime::now(),
            approved_at: Some(SystemTime::now()),
            gold_set_hash: Sha256Hash::from_bytes([0u8; 32]),
        }
        .with_computed_hash()
    }

    fn op_sig() -> OperatorSignature {
        OperatorSignature {
            operator_id: uuid::Uuid::new_v4(),
            signed_at: SystemTime::now(),
            ticket_id: "GOLD-1".into(),
            signature: vec![0u8; 64],
        }
    }

    fn gate(runner: Arc<dyn GoldRunner>) -> GoldGate {
        GoldGate::new(
            Arc::new(InMemoryGoldSetRegistry::new()),
            runner,
            Arc::new(DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))),
            Arc::new(InMemoryGoldRunLedger::new([42u8; 32])),
            N19Metrics::new(),
        )
    }

    // ─── Happy path : echo runner makes every case pass ──────────

    #[tokio::test]
    async fn happy_path_all_cases_pass() {
        let agent = agent("happy");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let report = g
            .run(agent.clone(), set, None, &GoldGateConfig::default())
            .await
            .unwrap();
        assert!(matches!(
            report.overall,
            GoldOverallVerdict::Passed {
                cases_passed: 5,
                cases_total: 5
            }
        ));
        assert!(report.permits_promote());
    }

    // ─── Sad path : drifting runner fails every case ─────────────

    #[tokio::test]
    async fn drifting_runner_fails_all_cases() {
        let agent = agent("drift");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::drifting()));
        let report = g
            .run(agent.clone(), set, None, &GoldGateConfig::default())
            .await
            .unwrap();
        match report.overall {
            GoldOverallVerdict::Failed { cases_failed, .. } => {
                assert_eq!(cases_failed, 5);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(!report.permits_promote());
    }

    // ─── Gold set not approved → error ──────────────────────────

    #[tokio::test]
    async fn unapproved_gold_set_rejected() {
        let agent = agent("a");
        let mut set = gold_set_5(&agent.id);
        set.status = GoldSetStatus::Draft;
        set.approvers.clear();
        // No re-hash needed : `status` + `approvers` are excluded
        // from the canonical hash.
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let err = g
            .run(agent, set, None, &GoldGateConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, GoldGateError::GoldSetNotApproved));
    }

    // ─── Cost cap ────────────────────────────────────────────────

    #[tokio::test]
    async fn cost_cap_aborts_run() {
        use crate::runner::{AgentRunFn, InMemoryGoldRunner, RunOutcome};
        let expensive: AgentRunFn = Arc::new(|_a, c| match &c.expected {
            GoldExpectedOutput::Inline { value } => RunOutcome::Output {
                value: value.clone(),
                cost_usd: 50.0,
                provider_calls: vec![],
                duration_ms: Some(1),
            },
            _ => RunOutcome::Output {
                value: serde_json::Value::Null,
                cost_usd: 50.0,
                provider_calls: vec![],
                duration_ms: Some(1),
            },
        });
        let agent = agent("expensive");
        let mut set = gold_set_5(&agent.id);
        set.cost_cap_usd = 40.0;
        set.refresh_hash();
        let g = gate(Arc::new(InMemoryGoldRunner::new_with(expensive)));
        let report = g
            .run(agent, set, None, &GoldGateConfig::default())
            .await
            .unwrap();
        assert!(matches!(
            report.overall,
            GoldOverallVerdict::AbortedCostCap { .. }
        ));
        assert!(!report.permits_promote());
    }

    // ─── Bypass ──────────────────────────────────────────────────

    #[tokio::test]
    async fn bypass_with_valid_certificate_passes_without_running_cases() {
        let agent = agent("bypass");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::always_crashes()));
        let cert = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "INC-9117".into(),
            rationale: "x".repeat(64),
            valid_until: SystemTime::now() + std::time::Duration::from_secs(86_400),
        };
        let report = g
            .run(agent, set, Some(cert), &GoldGateConfig::default())
            .await
            .unwrap();
        assert!(report.permits_promote());
        assert!(report.is_bypassed());
    }

    #[tokio::test]
    async fn bypass_with_short_rationale_rejected() {
        let agent = agent("short");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let cert = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "INC-1".into(),
            rationale: "too short".into(),
            valid_until: SystemTime::now() + std::time::Duration::from_secs(86_400),
        };
        let err = g
            .run(agent, set, Some(cert), &GoldGateConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            GoldGateError::BypassRationaleTooShort { .. }
        ));
    }

    #[tokio::test]
    async fn bypass_with_single_approver_rejected() {
        let agent = agent("single");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let cert = BypassCertificate {
            approvers: vec![op_sig()],
            ticket_id: "INC-1".into(),
            rationale: "x".repeat(64),
            valid_until: SystemTime::now() + std::time::Duration::from_secs(86_400),
        };
        let err = g
            .run(agent, set, Some(cert), &GoldGateConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            GoldGateError::BypassInsufficientApprovers { .. }
        ));
    }

    #[tokio::test]
    async fn bypass_with_expired_certificate_rejected() {
        let agent = agent("expired");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let cert = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "INC-1".into(),
            rationale: "x".repeat(64),
            valid_until: SystemTime::now() - std::time::Duration::from_secs(60),
        };
        let err = g
            .run(agent, set, Some(cert), &GoldGateConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, GoldGateError::BypassInvalidLifetime));
    }

    #[tokio::test]
    async fn bypass_with_too_long_lifetime_rejected() {
        let agent = agent("long");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let cert = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "INC-1".into(),
            rationale: "x".repeat(64),
            valid_until: SystemTime::now() + std::time::Duration::from_secs(86_400 * 30),
        };
        let err = g
            .run(agent, set, Some(cert), &GoldGateConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, GoldGateError::BypassInvalidLifetime));
    }

    #[tokio::test]
    async fn bypass_with_bad_ticket_rejected() {
        let agent = agent("badticket");
        let set = gold_set_5(&agent.id);
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let cert = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "FREE-FORM".into(),
            rationale: "x".repeat(64),
            valid_until: SystemTime::now() + std::time::Duration::from_secs(86_400),
        };
        let err = g
            .run(agent, set, Some(cert), &GoldGateConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, GoldGateError::BypassInvalidTicket { .. }));
    }

    // ─── Replay anchor stub ──────────────────────────────────────

    #[tokio::test]
    async fn replay_anchor_marks_case_errored_sprint_11_stub() {
        let agent = agent("replay");
        let mut set = gold_set_5(&agent.id);
        set.cases[0].expected = GoldExpectedOutput::ReplayAnchor {
            trace_id: uuid::Uuid::new_v4(),
            span_id: uuid::Uuid::new_v4(),
        };
        set.refresh_hash();
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let report = g
            .run(agent, set, None, &GoldGateConfig::default())
            .await
            .unwrap();
        // The first case errored ; the other 4 passed.  Aggregate
        // => Failed (1 errored treated as failed/errored).
        match report.overall {
            GoldOverallVerdict::Failed {
                cases_passed,
                cases_failed,
                cases_total,
            } => {
                assert_eq!(cases_passed, 4);
                assert_eq!(cases_failed, 1);
                assert_eq!(cases_total, 5);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    // ─── Concurrent runs serialised ──────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_runs_serialised_per_agent() {
        let agent = agent("concurrent");
        let set = gold_set_5(&agent.id);
        let g = Arc::new(gate(Arc::new(InMemoryGoldRunner::echo())));
        let agent_clone = agent.clone();
        let set_clone = set.clone();
        let g_clone = g.clone();
        // Manually claim the in-progress slot to simulate an
        // ongoing run.  This deterministically reproduces F15.
        g_clone.in_progress.insert(agent.id.clone(), GoldRunId::new());
        let err = g_clone
            .run(agent_clone, set_clone, None, &GoldGateConfig::default())
            .await
            .unwrap_err();
        assert!(matches!(err, GoldGateError::RunAlreadyInProgress { .. }));
    }

    // ─── Ledger row appended on completion ───────────────────────

    #[tokio::test]
    async fn ledger_row_appended_on_run() {
        let agent = agent("ledger");
        let set = gold_set_5(&agent.id);
        let registry = Arc::new(InMemoryGoldSetRegistry::new());
        let ledger = Arc::new(InMemoryGoldRunLedger::new([7u8; 32]));
        let g = GoldGate::new(
            registry,
            Arc::new(InMemoryGoldRunner::echo()),
            Arc::new(DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))),
            ledger.clone(),
            N19Metrics::new(),
        );
        let _ = g
            .run(agent.clone(), set, None, &GoldGateConfig::default())
            .await
            .unwrap();
        let history = ledger.history(&agent.id, 10).await.unwrap();
        assert!(history
            .iter()
            .any(|r| matches!(r.entry, GoldRunLedgerEntry::GoldRunCompleted { .. })));
    }

    // ─── Bypass ledger row ───────────────────────────────────────

    #[tokio::test]
    async fn bypass_writes_dedicated_ledger_row() {
        let agent = agent("bypass_ledger");
        let set = gold_set_5(&agent.id);
        let ledger = Arc::new(InMemoryGoldRunLedger::new([8u8; 32]));
        let g = GoldGate::new(
            Arc::new(InMemoryGoldSetRegistry::new()),
            Arc::new(InMemoryGoldRunner::echo()),
            Arc::new(DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))),
            ledger.clone(),
            N19Metrics::new(),
        );
        let cert = BypassCertificate {
            approvers: vec![op_sig(), op_sig()],
            ticket_id: "INC-9117".into(),
            rationale: "x".repeat(64),
            valid_until: SystemTime::now() + std::time::Duration::from_secs(86_400),
        };
        let _ = g
            .run(
                agent.clone(),
                set,
                Some(cert),
                &GoldGateConfig::default(),
            )
            .await
            .unwrap();
        let history = ledger.history(&agent.id, 10).await.unwrap();
        assert!(history
            .iter()
            .any(|r| matches!(r.entry, GoldRunLedgerEntry::GoldRunBypassed { .. })));
        // GoldRunCompleted is NOT appended on bypass (the only
        // ledger row is the bypass record).
        assert!(!history
            .iter()
            .any(|r| matches!(r.entry, GoldRunLedgerEntry::GoldRunCompleted { .. })));
    }

    // ─── Strategy mix triggers all three labels ──────────────────

    #[tokio::test]
    async fn metric_labels_cover_all_three_strategies() {
        let mut set = gold_set_5(&NanoAgentId::new("mix").unwrap());
        // First case → tolerance.
        let mut fields = std::collections::BTreeMap::new();
        fields.insert("/o".into(), 0.0);
        set.cases[0].assertion = GoldAssertionStrategy::NumericTolerance { fields };
        // Second case → similarity.
        set.cases[1].assertion = GoldAssertionStrategy::SemanticSimilarity {
            embedding_model: ModelIdentity::hosted("openai", "text-embedding-3-small", "1"),
            cosine_threshold: 0.5,
            field_pointer: None,
        };
        set.refresh_hash();
        let metrics = N19Metrics::new();
        let g = GoldGate::new(
            Arc::new(InMemoryGoldSetRegistry::new()),
            Arc::new(InMemoryGoldRunner::echo()),
            Arc::new(DefaultAssertionEngine::new(Arc::new(StubEmbeddingProvider::new()))),
            Arc::new(InMemoryGoldRunLedger::new([9u8; 32])),
            metrics.clone(),
        );
        let agent_arc = agent("mix");
        let _ = g
            .run(
                agent_arc,
                set,
                None,
                &GoldGateConfig {
                    agent_tier_label: "tier_1".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(metrics.gold_cases_total("strict_equality", "passed") >= 1);
        assert!(metrics.gold_cases_total("numeric_tolerance", "passed") >= 1);
        assert!(metrics.gold_cases_total("semantic_similarity", "passed") >= 1);
    }

    #[test]
    fn debug_format_no_panic() {
        let g = gate(Arc::new(InMemoryGoldRunner::echo()));
        let _ = format!("{g:?}");
    }
}
