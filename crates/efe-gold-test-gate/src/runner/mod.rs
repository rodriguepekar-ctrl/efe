// SPDX-License-Identifier: UNLICENSED
//! **Sprint 11 — N19** : GoldRunner — § 7.4.
//!
//! The runner executes a single gold case in **hermetic isolation**
//! (Architecture I11–I14) and captures the agent output + cost +
//! provider call provenance ids.
//!
//! Sprint 11 ships :
//! - the `GoldRunner` trait (transport-agnostic),
//! - `InMemoryGoldRunner` (tests + dev) — backed by a programmable
//!   `AgentRunFn` closure that simulates the agent's execution
//!   without spinning up a microVM.
//!
//! Sprint 12 wires the real `FirecrackerGoldRunner` that reuses the
//! N11 microVM pool with :
//! - network namespace disabled (I11),
//! - clock pinned to `frozen_clock_utc` (I12),
//! - root FS restricted to the gold-case bundle (I13),
//! - per-case timeout enforced by the VM watchdog (I14).
//!
//! The trait deliberately accepts `RunnerInput` instead of
//! `&NanoAgent + &GoldCase` to keep the surface flexible for the
//! Firecracker impl (which will need agent body bytes + sidecar
//! manifests).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use efe_nano_sdk::{GoldCase, GoldCaseRun, NanoAgent};
use thiserror::Error;
use uuid::Uuid;

/// **N19** : input bundle handed to a runner — agent + case + clock.
#[derive(Debug, Clone)]
pub struct RunnerInput {
    pub agent: Arc<NanoAgent>,
    pub case: GoldCase,
    /// Wall clock pinned for the case execution (I12).
    pub frozen_clock_utc: SystemTime,
}

/// **N19** : closed taxonomy of runner errors.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum GoldRunnerError {
    #[error("hermeticity violated for case {case_id}")]
    HermeticityBroken { case_id: efe_nano_sdk::GoldCaseId },

    #[error("case timed out after {seconds}s")]
    Timeout { seconds: u32 },

    #[error("microvm crash : {message}")]
    VmCrash { message: String },

    #[error("backend : {message}")]
    Backend { message: String },
}

/// **N19** : the runner trait.
#[async_trait]
pub trait GoldRunner: Send + Sync + 'static {
    async fn run_case(
        &self,
        input: RunnerInput,
    ) -> Result<GoldCaseRun, GoldRunnerError>;
}

// ─────────────────────────────────────────────────────────────────────
//  InMemoryGoldRunner
// ─────────────────────────────────────────────────────────────────────

/// **N19** : function signature for the in-memory runner's agent
/// simulator.  `(agent, case) → output_json` ; closure may also
/// inject simulated cost (US$) + provider call ids.
pub type AgentRunFn = Arc<
    dyn Fn(&NanoAgent, &GoldCase) -> RunOutcome + Send + Sync + 'static,
>;

/// **N19** : per-case outcome from an `InMemoryGoldRunner`
/// simulator.
#[derive(Debug, Clone)]
pub enum RunOutcome {
    /// Agent produced an output successfully.
    Output {
        value: serde_json::Value,
        cost_usd: f64,
        provider_calls: Vec<Uuid>,
        /// Optional override for the captured duration (default
        /// 1 ms).
        duration_ms: Option<u32>,
    },
    /// Agent crashed.
    Crashed { message: String },
    /// Agent leaked the hermeticity boundary.
    HermeticityViolation,
    /// Agent took too long.
    TimedOut,
}

/// **N19** : in-memory runner backed by a programmable simulator.
///
/// Use cases :
/// - **happy path** : `InMemoryGoldRunner::echo` returns the case's
///   expected `Inline` value as the captured output (most cases
///   pass).
/// - **regression** : `InMemoryGoldRunner::new_with(...)` lets the
///   test inject an arbitrary closure to simulate buggy / drifting
///   agents.
pub struct InMemoryGoldRunner {
    simulator: AgentRunFn,
}

impl std::fmt::Debug for InMemoryGoldRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryGoldRunner")
            .finish_non_exhaustive()
    }
}

impl InMemoryGoldRunner {
    pub fn new_with(simulator: AgentRunFn) -> Self {
        Self { simulator }
    }

    /// **N19** : Convenience constructor — every case echoes its
    /// own `Inline` expected output.  Cases backed by
    /// `ReplayAnchor` echo `null` (Sprint 11 stub ; Sprint 12 wires
    /// real N16 replay).
    pub fn echo() -> Self {
        let f: AgentRunFn = Arc::new(|_agent, case| match &case.expected {
            efe_nano_sdk::GoldExpectedOutput::Inline { value } => RunOutcome::Output {
                value: value.clone(),
                cost_usd: 0.001,
                provider_calls: vec![],
                duration_ms: Some(1),
            },
            efe_nano_sdk::GoldExpectedOutput::ReplayAnchor { .. } => RunOutcome::Output {
                value: serde_json::Value::Null,
                cost_usd: 0.001,
                provider_calls: vec![],
                duration_ms: Some(1),
            },
        });
        Self::new_with(f)
    }

    /// **N19** : Convenience constructor — every case "drifts" by
    /// returning a perturbed output (used by regression-detection
    /// tests).  Modifies the first numeric field by adding 0.1,
    /// or appends "_drift" to the first string field.
    pub fn drifting() -> Self {
        let f: AgentRunFn = Arc::new(|_agent, case| match &case.expected {
            efe_nano_sdk::GoldExpectedOutput::Inline { value } => {
                let mut v = value.clone();
                perturb_first_value(&mut v);
                RunOutcome::Output {
                    value: v,
                    cost_usd: 0.001,
                    provider_calls: vec![],
                    duration_ms: Some(1),
                }
            }
            _ => RunOutcome::Output {
                value: serde_json::Value::Null,
                cost_usd: 0.001,
                provider_calls: vec![],
                duration_ms: Some(1),
            },
        });
        Self::new_with(f)
    }

    /// **N19** : Convenience constructor — every case crashes.
    pub fn always_crashes() -> Self {
        let f: AgentRunFn = Arc::new(|_agent, _case| RunOutcome::Crashed {
            message: "simulated runner crash".into(),
        });
        Self::new_with(f)
    }
}

#[async_trait]
impl GoldRunner for InMemoryGoldRunner {
    async fn run_case(
        &self,
        input: RunnerInput,
    ) -> Result<GoldCaseRun, GoldRunnerError> {
        let start = std::time::Instant::now();
        let outcome = (self.simulator)(&input.agent, &input.case);
        let elapsed_ms = start.elapsed().as_millis() as u32;
        // Hermeticity probe : if the simulator returns
        // `HermeticityViolation`, propagate as a runner error so the
        // gate aborts the whole run (Architecture F2).
        match outcome {
            RunOutcome::Output {
                value,
                cost_usd,
                provider_calls,
                duration_ms,
            } => Ok(GoldCaseRun {
                case_id: input.case.gold_case_id,
                trace_id: Uuid::new_v4(),
                captured_output: value,
                duration_ms: duration_ms.unwrap_or(elapsed_ms.max(1)),
                cost_usd,
                provider_calls,
                errored: None,
            }),
            RunOutcome::Crashed { message } => Ok(GoldCaseRun {
                case_id: input.case.gold_case_id,
                trace_id: Uuid::new_v4(),
                captured_output: serde_json::Value::Null,
                duration_ms: elapsed_ms.max(1),
                cost_usd: 0.0,
                provider_calls: vec![],
                errored: Some(message),
            }),
            RunOutcome::HermeticityViolation => Err(GoldRunnerError::HermeticityBroken {
                case_id: input.case.gold_case_id,
            }),
            RunOutcome::TimedOut => Err(GoldRunnerError::Timeout {
                seconds: input.case.timeout_seconds,
            }),
        }
    }
}

fn perturb_first_value(v: &mut serde_json::Value) {
    use serde_json::Value;
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                *n = serde_json::Number::from_f64(f + 0.1).unwrap_or(n.clone());
            }
        }
        Value::String(s) => {
            s.push_str("_drift");
        }
        Value::Bool(b) => {
            *b = !*b;
        }
        Value::Array(arr) => {
            if let Some(first) = arr.first_mut() {
                perturb_first_value(first);
            }
        }
        Value::Object(map) => {
            // BTreeMap iter order is insertion ; pick a value
            // deterministically (first key alphabetically).
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            if let Some(first_key) = keys.first() {
                if let Some(val) = map.get_mut(first_key) {
                    perturb_first_value(val);
                }
            }
        }
        Value::Null => {}
    }
    let _ = Duration::default(); // appease unused-import lint
}

#[cfg(test)]
mod tests {
    use super::*;
    use efe_nano_sdk::{
        Author, Category, ComparisonStrategy, DeterminismLevel, ExecutionMode,
        GoldAssertionStrategy, GoldCaseId, GoldExpectedOutput, IdempotencyPolicy,
        NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
    };

    fn agent() -> Arc<NanoAgent> {
        Arc::new(NanoAgent {
            id: NanoAgentId::new("test").unwrap(),
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
            audit: efe_nano_sdk::AuditPolicy::default(),
            compliance: efe_nano_sdk::ComplianceTags::default(),
            mcp_tool_metadata: None,
            signature: None,
            co_publishing: None,
            gold_set_ref: None,
            created_at: chrono::Utc::now(),
        })
    }

    fn case_with_expected(expected: GoldExpectedOutput) -> GoldCase {
        GoldCase {
            gold_case_id: GoldCaseId::new(),
            name: "c".into(),
            input: serde_json::json!({"x": 1}),
            expected,
            assertion: GoldAssertionStrategy::StrictEquality,
            timeout_seconds: 30,
            tags: vec![],
        }
    }

    fn input(case: GoldCase) -> RunnerInput {
        RunnerInput {
            agent: agent(),
            case,
            frozen_clock_utc: SystemTime::now(),
        }
    }

    #[tokio::test]
    async fn echo_runner_returns_expected_inline_value() {
        let r = InMemoryGoldRunner::echo();
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({"y": 42}),
        });
        let out = r.run_case(input(case)).await.unwrap();
        assert_eq!(out.captured_output, serde_json::json!({"y": 42}));
        assert!(out.errored.is_none());
    }

    #[tokio::test]
    async fn echo_runner_replay_anchor_returns_null() {
        let r = InMemoryGoldRunner::echo();
        let case = case_with_expected(GoldExpectedOutput::ReplayAnchor {
            trace_id: Uuid::nil(),
            span_id: Uuid::nil(),
        });
        let out = r.run_case(input(case)).await.unwrap();
        assert_eq!(out.captured_output, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn drifting_runner_perturbs_string() {
        let r = InMemoryGoldRunner::drifting();
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({"summary": "hello"}),
        });
        let out = r.run_case(input(case)).await.unwrap();
        assert_eq!(
            out.captured_output,
            serde_json::json!({"summary": "hello_drift"})
        );
    }

    #[tokio::test]
    async fn drifting_runner_perturbs_number() {
        let r = InMemoryGoldRunner::drifting();
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({"score": 0.5}),
        });
        let out = r.run_case(input(case)).await.unwrap();
        // Score should be ~0.6 (0.5 + 0.1).
        let score = out.captured_output.get("score").unwrap().as_f64().unwrap();
        assert!((score - 0.6).abs() < 1e-9);
    }

    #[tokio::test]
    async fn drifting_runner_perturbs_bool() {
        let r = InMemoryGoldRunner::drifting();
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({"flag": true}),
        });
        let out = r.run_case(input(case)).await.unwrap();
        assert_eq!(out.captured_output, serde_json::json!({"flag": false}));
    }

    #[tokio::test]
    async fn always_crashes_runner_records_error_string() {
        let r = InMemoryGoldRunner::always_crashes();
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({"y": 1}),
        });
        let out = r.run_case(input(case)).await.unwrap();
        assert!(out.errored.is_some());
        assert_eq!(out.captured_output, serde_json::Value::Null);
    }

    #[tokio::test]
    async fn custom_simulator_hermeticity_violation_propagates_as_error() {
        let f: AgentRunFn = Arc::new(|_a, _c| RunOutcome::HermeticityViolation);
        let r = InMemoryGoldRunner::new_with(f);
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({}),
        });
        let err = r.run_case(input(case)).await.unwrap_err();
        assert!(matches!(err, GoldRunnerError::HermeticityBroken { .. }));
    }

    #[tokio::test]
    async fn custom_simulator_timeout_propagates_as_error() {
        let f: AgentRunFn = Arc::new(|_a, _c| RunOutcome::TimedOut);
        let r = InMemoryGoldRunner::new_with(f);
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({}),
        });
        let err = r.run_case(input(case)).await.unwrap_err();
        assert!(matches!(err, GoldRunnerError::Timeout { .. }));
        // Suppress unused-import warning : we don't use
        // ComparisonStrategy directly in this test.
        let _ = ComparisonStrategy::Exact;
    }

    #[tokio::test]
    async fn run_case_records_provider_calls() {
        let f: AgentRunFn = Arc::new(|_a, _c| RunOutcome::Output {
            value: serde_json::json!({}),
            cost_usd: 0.5,
            provider_calls: vec![Uuid::new_v4(), Uuid::new_v4()],
            duration_ms: Some(50),
        });
        let r = InMemoryGoldRunner::new_with(f);
        let case = case_with_expected(GoldExpectedOutput::Inline {
            value: serde_json::json!({}),
        });
        let out = r.run_case(input(case)).await.unwrap();
        assert_eq!(out.provider_calls.len(), 2);
        assert_eq!(out.cost_usd, 0.5);
        assert_eq!(out.duration_ms, 50);
    }

    #[test]
    fn debug_format_no_panic() {
        let r = InMemoryGoldRunner::echo();
        let _ = format!("{r:?}");
    }
}
