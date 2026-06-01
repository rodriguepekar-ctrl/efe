//! Mode3Engine — multi-turn iterative reasoning orchestrator.
//!
//! Pure logic — no IO. Concrete provider/swarm/router implementations
//! are injected via traits to enable unit testing without Postgres
//! or LLMRouter network round-trips.

use chrono::Utc;

use super::concept_stack::{ConceptStack, ConceptStackEntry};
use super::error::Mode3Error;
use super::types::{
    ClassifyMode3Request, ClassifyMode3Response, ConvergenceResult, StakesTier, TenantMode3Config,
};
use super::{MAX_TURNS_HARD_CAP, TEMPLATE_REFINE, TEMPLATE_TURN_0};

// Note : `LLM_REASONING_MAX_CHARS` (from super) is used inside tests
// via the `use super::*;` in the test module. The lib code itself
// uses it indirectly via `ConceptStackEntry::truncate_reasoning`.
#[cfg(test)]
use super::LLM_REASONING_MAX_CHARS;

/// Result of a swarm Mode-1 call (subset of mode1's response).
#[derive(Debug, Clone)]
pub struct SwarmResult {
    pub label: String,
    pub confidence: f32,
    pub duration_ms: u32,
}

/// Result of an LLM narrowing call (turn 0 or refine).
#[derive(Debug, Clone)]
pub struct LlmNarrowResult {
    pub candidates: Vec<String>,
    pub reasoning: Option<String>,
    pub cost_usd: f64,
    pub duration_ms: u32,
    pub provider_used: String,
}

/// Trait abstracting LLMRouter calls so the engine is unit-testable.
#[async_trait::async_trait]
pub trait Mode3Backend: Send + Sync {
    /// Embed sentence + return top-K kNN candidates from catalog.
    async fn embed_and_knn(
        &self,
        request: &ClassifyMode3Request,
        top_k: u32,
    ) -> Result<Vec<String>, Mode3Error>;

    /// Call LLM via router with a given template + candidates.
    /// Output : narrowed candidate list + reasoning trace.
    async fn llm_narrow(
        &self,
        request: &ClassifyMode3Request,
        candidates: &[String],
        template_id: &str,
        target_count: u32,
        swarm_label_attempt: Option<&str>,
        swarm_confidence: Option<f32>,
        threshold: f32,
    ) -> Result<LlmNarrowResult, Mode3Error>;

    /// Run swarm Mode-1 consensus on the given candidate set.
    async fn swarm_classify(
        &self,
        sentence: &str,
        candidates: &[String],
    ) -> Result<SwarmResult, Mode3Error>;
}

/// Engine config — how many turns, what threshold.
#[derive(Debug, Clone)]
pub struct Mode3EngineConfig {
    pub max_turns: u32,
    pub threshold: f32,
    pub top_k_anchors: u32,
}

impl Mode3EngineConfig {
    /// Resolve effective config from request + tenant defaults.
    pub fn resolve(
        request: &ClassifyMode3Request,
        tenant: &TenantMode3Config,
    ) -> Result<Self, Mode3Error> {
        let max_turns = request
            .max_turns
            .unwrap_or(tenant.default_max_turns)
            .min(tenant.max_max_turns)
            .min(MAX_TURNS_HARD_CAP);

        if max_turns < 1 || max_turns > MAX_TURNS_HARD_CAP {
            return Err(Mode3Error::InvalidMaxTurns(max_turns, MAX_TURNS_HARD_CAP));
        }

        let threshold = request.threshold.unwrap_or(tenant.default_threshold);
        if !(0.0..=1.0).contains(&threshold) {
            return Err(Mode3Error::InvalidThreshold(threshold));
        }

        let top_k = request.top_k_anchors.unwrap_or(10).clamp(3, 20);

        Ok(Self {
            max_turns,
            threshold,
            top_k_anchors: top_k,
        })
    }
}

/// Mode3Engine — pure-logic orchestrator. IO injected via Mode3Backend.
pub struct Mode3Engine<B: Mode3Backend> {
    backend: B,
}

impl<B: Mode3Backend> Mode3Engine<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Main entry — orchestrate Mode-3 multi-turn classification.
    /// Returns concept_stack + final response (without persisting yet).
    /// Caller is responsible for audit chain + review queue insertion.
    #[tracing::instrument(skip_all, fields(tenant_id = %request.tenant_id, catalog = %request.catalog))]
    pub async fn classify(
        &self,
        request: ClassifyMode3Request,
        tenant: TenantMode3Config,
        stakes_tier: StakesTier,
    ) -> Result<(ConceptStack, ClassifyMode3Response), Mode3Error> {
        let config = Mode3EngineConfig::resolve(&request, &tenant)?;
        let mut stack = ConceptStack::new();

        // ─── Turn 0 — initial narrowing 10 → 5 via Mode-2 template ───
        let candidates_in = self
            .backend
            .embed_and_knn(&request, config.top_k_anchors)
            .await?;
        let turn0 = self
            .backend
            .llm_narrow(
                &request,
                &candidates_in,
                TEMPLATE_TURN_0,
                5,
                None,
                None,
                config.threshold,
            )
            .await?;

        validate_subset(&turn0.candidates, &candidates_in, 0)?;

        stack.append(ConceptStackEntry {
            turn: 0,
            candidates_in: candidates_in.clone(),
            candidates_out: turn0.candidates.clone(),
            llm_reasoning: ConceptStackEntry::truncate_reasoning(turn0.reasoning),
            swarm_label_attempt: None,
            swarm_confidence: None,
            cost_usd: turn0.cost_usd,
            duration_ms: turn0.duration_ms,
            provider_used: Some(turn0.provider_used),
            timestamp: Utc::now(),
        });

        let mut current_candidates = turn0.candidates;

        // ─── Turns 1+ — partial swarm + LLM refine ───────────────────
        for turn in 1..=config.max_turns {
            let swarm_result = self
                .backend
                .swarm_classify(&request.sentence_redacted, &current_candidates)
                .await?;

            let convergence = ConvergenceResult::check(
                Some(swarm_result.confidence),
                config.threshold,
                turn,
                config.max_turns,
            );

            match convergence {
                ConvergenceResult::Converged => {
                    stack.append(ConceptStackEntry {
                        turn,
                        candidates_in: current_candidates,
                        candidates_out: vec![],
                        llm_reasoning: None,
                        swarm_label_attempt: Some(swarm_result.label.clone()),
                        swarm_confidence: Some(swarm_result.confidence),
                        cost_usd: 0.0,
                        duration_ms: swarm_result.duration_ms,
                        provider_used: None,
                        timestamp: Utc::now(),
                    });
                    stack.finalize(swarm_result.label.clone(), swarm_result.confidence, true);
                    let resp = ClassifyMode3Response {
                        final_label: Some(swarm_result.label),
                        final_confidence: Some(swarm_result.confidence),
                        mode: "MODE_3".into(),
                        total_turns: stack.total_turns(),
                        total_cost_usd: stack.total_cost_usd,
                        converged: true,
                        requires_human_review: false,
                        review_token: None,
                        degraded_reason: None,
                        audit_id: None,
                    };
                    return Ok((stack, resp));
                }
                ConvergenceResult::MaxTurnsReached => {
                    stack.append(ConceptStackEntry {
                        turn,
                        candidates_in: current_candidates,
                        candidates_out: vec![],
                        llm_reasoning: None,
                        swarm_label_attempt: Some(swarm_result.label.clone()),
                        swarm_confidence: Some(swarm_result.confidence),
                        cost_usd: 0.0,
                        duration_ms: swarm_result.duration_ms,
                        provider_used: None,
                        timestamp: Utc::now(),
                    });
                    stack.finalize(swarm_result.label.clone(), swarm_result.confidence, false);

                    // Check if review queue insertion required.
                    let requires_review = stakes_tier.requires_review_when_unconverged()
                        && tenant.human_review_required;
                    let resp = ClassifyMode3Response {
                        final_label: Some(swarm_result.label),
                        final_confidence: Some(swarm_result.confidence),
                        mode: "MODE_3".into(),
                        total_turns: stack.total_turns(),
                        total_cost_usd: stack.total_cost_usd,
                        converged: false,
                        requires_human_review: requires_review,
                        review_token: None, // populated by caller after DB insert
                        degraded_reason: None,
                        audit_id: None,
                    };
                    return Ok((stack, resp));
                }
                ConvergenceResult::Continue => {
                    // Refine for next turn
                    let refined = self
                        .backend
                        .llm_narrow(
                            &request,
                            &current_candidates,
                            TEMPLATE_REFINE,
                            2,
                            Some(&swarm_result.label),
                            Some(swarm_result.confidence),
                            config.threshold,
                        )
                        .await?;

                    validate_subset(&refined.candidates, &current_candidates, turn)?;

                    stack.append(ConceptStackEntry {
                        turn,
                        candidates_in: current_candidates,
                        candidates_out: refined.candidates.clone(),
                        llm_reasoning: ConceptStackEntry::truncate_reasoning(refined.reasoning),
                        swarm_label_attempt: Some(swarm_result.label),
                        swarm_confidence: Some(swarm_result.confidence),
                        cost_usd: refined.cost_usd,
                        duration_ms: swarm_result.duration_ms + refined.duration_ms,
                        provider_used: Some(refined.provider_used),
                        timestamp: Utc::now(),
                    });
                    current_candidates = refined.candidates;
                }
            }
        }

        // Should never reach here — loop exits via Converged or MaxTurnsReached.
        unreachable!("Mode3Engine loop must exit via convergence check")
    }
}

/// Anti-hallucination guard : LLM-returned candidates must be a subset
/// of the input candidates (verbatim, case-sensitive).
fn validate_subset(output: &[String], input: &[String], turn: u32) -> Result<(), Mode3Error> {
    for c in output {
        if !input.iter().any(|i| i == c) {
            return Err(Mode3Error::OutputValidation {
                turn,
                reason: format!("hallucinated anchor not in candidate set: {:?}", c),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_subset_ok() {
        let input = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let output = vec!["a".to_string(), "b".to_string()];
        assert!(validate_subset(&output, &input, 0).is_ok());
    }

    #[test]
    fn test_validate_subset_rejects_hallucinated() {
        let input = vec!["a".to_string(), "b".to_string()];
        let output = vec!["a".to_string(), "x_hallucinated".to_string()];
        let err = validate_subset(&output, &input, 1).unwrap_err();
        assert!(matches!(err, Mode3Error::OutputValidation { turn: 1, .. }));
    }

    #[test]
    fn test_validate_subset_empty_output_ok() {
        let input = vec!["a".to_string()];
        let output: Vec<String> = vec![];
        assert!(validate_subset(&output, &input, 0).is_ok());
    }

    #[test]
    fn test_truncate_reasoning_short_kept() {
        let s = "short text".to_string();
        let result = ConceptStackEntry::truncate_reasoning(Some(s.clone()));
        assert_eq!(result, Some(s));
    }

    #[test]
    fn test_truncate_reasoning_long_truncated() {
        let s = "x".repeat(LLM_REASONING_MAX_CHARS + 100);
        let result = ConceptStackEntry::truncate_reasoning(Some(s)).unwrap();
        assert!(result.ends_with("...[truncated]"));
        assert!(result.len() <= LLM_REASONING_MAX_CHARS + 16); // +14 for marker
    }

    #[test]
    fn test_engine_config_resolves_defaults() {
        let req = ClassifyMode3Request {
            tenant_id: "t".into(),
            catalog: "c".into(),
            bundle_version: "v1.0.0".into(),
            sentence: "s".into(),
            sentence_redacted: "s".into(),
            max_turns: None,
            threshold: None,
            top_k_anchors: None,
        };
        let tenant = TenantMode3Config::default_for("t");
        // tenant.enabled = false but resolve() doesn't check enablement
        let config = Mode3EngineConfig::resolve(&req, &tenant).unwrap();
        assert_eq!(config.max_turns, 3);
        assert_eq!(config.threshold, 0.85);
        assert_eq!(config.top_k_anchors, 10);
    }

    #[test]
    fn test_engine_config_clamps_max_turns_to_hard_cap() {
        let req = ClassifyMode3Request {
            tenant_id: "t".into(),
            catalog: "c".into(),
            bundle_version: "v1.0.0".into(),
            sentence: "s".into(),
            sentence_redacted: "s".into(),
            max_turns: Some(99), // > MAX_TURNS_HARD_CAP
            threshold: None,
            top_k_anchors: None,
        };
        let tenant = TenantMode3Config::default_for("t");
        let config = Mode3EngineConfig::resolve(&req, &tenant).unwrap();
        assert_eq!(config.max_turns, MAX_TURNS_HARD_CAP);
    }

    #[test]
    fn test_engine_config_rejects_invalid_threshold() {
        let req = ClassifyMode3Request {
            tenant_id: "t".into(),
            catalog: "c".into(),
            bundle_version: "v1.0.0".into(),
            sentence: "s".into(),
            sentence_redacted: "s".into(),
            max_turns: None,
            threshold: Some(1.5),
            top_k_anchors: None,
        };
        let tenant = TenantMode3Config::default_for("t");
        let err = Mode3EngineConfig::resolve(&req, &tenant).unwrap_err();
        assert!(matches!(err, Mode3Error::InvalidThreshold(_)));
    }

    #[test]
    fn test_engine_config_clamps_top_k() {
        let req = ClassifyMode3Request {
            tenant_id: "t".into(),
            catalog: "c".into(),
            bundle_version: "v1.0.0".into(),
            sentence: "s".into(),
            sentence_redacted: "s".into(),
            max_turns: None,
            threshold: None,
            top_k_anchors: Some(100),
        };
        let tenant = TenantMode3Config::default_for("t");
        let config = Mode3EngineConfig::resolve(&req, &tenant).unwrap();
        assert_eq!(config.top_k_anchors, 20);
    }

    #[test]
    fn test_convergence_check_converged() {
        let r = ConvergenceResult::check(Some(0.95), 0.85, 1, 3);
        assert_eq!(r, ConvergenceResult::Converged);
    }

    #[test]
    fn test_convergence_check_continue() {
        let r = ConvergenceResult::check(Some(0.5), 0.85, 1, 3);
        assert_eq!(r, ConvergenceResult::Continue);
    }

    #[test]
    fn test_convergence_check_max_turns() {
        let r = ConvergenceResult::check(Some(0.5), 0.85, 3, 3);
        assert_eq!(r, ConvergenceResult::MaxTurnsReached);
    }

    #[test]
    fn test_concept_stack_total_cost_aggregates() {
        let mut stack = ConceptStack::new();
        stack.append(ConceptStackEntry {
            turn: 0,
            candidates_in: vec!["a".into()],
            candidates_out: vec![],
            llm_reasoning: None,
            swarm_label_attempt: None,
            swarm_confidence: None,
            cost_usd: 0.001,
            duration_ms: 100,
            provider_used: None,
            timestamp: Utc::now(),
        });
        stack.append(ConceptStackEntry {
            turn: 1,
            candidates_in: vec![],
            candidates_out: vec![],
            llm_reasoning: None,
            swarm_label_attempt: None,
            swarm_confidence: None,
            cost_usd: 0.002,
            duration_ms: 200,
            provider_used: None,
            timestamp: Utc::now(),
        });
        assert!((stack.total_cost_usd - 0.003).abs() < 1e-9);
        assert_eq!(stack.total_turns(), 2);
        assert_eq!(stack.total_duration_ms(), 300);
    }

    #[test]
    fn test_concept_stack_finalize_sets_flags() {
        let mut stack = ConceptStack::new();
        stack.finalize("billing".into(), 0.91, true);
        assert_eq!(stack.final_label.as_deref(), Some("billing"));
        assert_eq!(stack.final_confidence, Some(0.91));
        assert!(stack.converged);
    }

    #[test]
    fn test_concept_stack_llm_calls_count() {
        let mut stack = ConceptStack::new();
        stack.append(ConceptStackEntry {
            turn: 0,
            candidates_in: vec![],
            candidates_out: vec![],
            llm_reasoning: Some("turn 0 reasoning".into()),
            swarm_label_attempt: None,
            swarm_confidence: None,
            cost_usd: 0.001,
            duration_ms: 100,
            provider_used: Some("anthropic".into()),
            timestamp: Utc::now(),
        });
        stack.append(ConceptStackEntry {
            turn: 1,
            candidates_in: vec![],
            candidates_out: vec![],
            llm_reasoning: None, // swarm-only
            swarm_label_attempt: Some("billing".into()),
            swarm_confidence: Some(0.91),
            cost_usd: 0.0,
            duration_ms: 50,
            provider_used: None,
            timestamp: Utc::now(),
        });
        assert_eq!(stack.llm_calls_count(), 1);
    }

    #[test]
    fn test_stakes_tier_parse() {
        assert_eq!(StakesTier::parse("low"), StakesTier::Low);
        assert_eq!(StakesTier::parse("MEDIUM"), StakesTier::Medium);
        assert_eq!(StakesTier::parse("high"), StakesTier::High);
        assert_eq!(StakesTier::parse("unknown"), StakesTier::Low);
    }

    #[test]
    fn test_stakes_tier_requires_review() {
        assert!(StakesTier::High.requires_review_when_unconverged());
        assert!(!StakesTier::Medium.requires_review_when_unconverged());
        assert!(!StakesTier::Low.requires_review_when_unconverged());
    }
}
