// SPDX-License-Identifier: UNLICENSED
//! **Sprint 15 — N23 + G12** : synthetic catalogue back-fill for
//! the six built-in pre-analysis agents.
//!
//! Each agent gets a published [`NanoAgent`] descriptor + a
//! [`NanoRunner`] shim that delegates to the existing
//! `SwarmAgent` implementation (which preserves the historical
//! blackboard pointer contract — `/entities`, `/language`,
//! `/sentiment`, `/redacted_text`+`/redactions`, `/summary`,
//! `/schema_valid`).
//!
//! Naming convention :
//! - **`NanoAgentId`** uses underscores (`ner_extractor`) because
//!   the SDK identifier rejects hyphens (`[a-z0-9_]+`).
//! - **`AgentId`** uses hyphens (`ner-extractor`) — the existing
//!   DAG topology depends on this.
//!
//! The `NanoToSwarm` adapter handles the translation
//! automatically via its `derived_agent_id()` (replaces `_` with
//! `-`).
//!
//! ## Determinism (I20)
//!
//! [`default_catalogue`] returns the same six entries in the same
//! order on every call.  `created_at` is fixed to the M50 epoch
//! (`2026-01-01T00:00:00Z`) so the canonical hash of the
//! catalogue is reproducible across builds — required for the
//! N24 replay snapshot.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use efe_nano_sdk::{
    Author, Category, DeterminismLevel, ExecutionMode, IdempotencyPolicy, NanoAgent,
    NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use efe_nano_swarm_adapter::{AdapterError, NanoRunner, NanoRunnerOutcome};
use efe_swarm_orchestrator::{AgentOutcome, SwarmAgent, SwarmAgentContext};

use crate::{
    language_detector::LanguageDetector, ner_extractor::NerExtractor,
    pii_redactor::PiiRedactor, schema_validator::SchemaValidator,
    sentiment_classifier::SentimentClassifier, summary_writer::SummaryWriter,
};

// ─────────────────────────────────────────────────────────────────────
//  Fixed reference timestamps (I20 — deterministic catalogue)
// ─────────────────────────────────────────────────────────────────────

/// **N23 + I20** : fixed catalogue epoch — `2026-01-01T00:00:00Z`.
/// Ensures `default_catalogue()` returns byte-stable entries
/// across builds.
fn catalogue_epoch() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
}

// ─────────────────────────────────────────────────────────────────────
//  Generic SwarmAgent → NanoRunner shim
// ─────────────────────────────────────────────────────────────────────

/// **N23** : a tiny shim that exposes any existing `SwarmAgent`
/// (the N21 pre-analysis agents) as a [`NanoRunner`].
///
/// The agent's own `run(ctx)` reads + writes the blackboard at
/// the canonical pointers it already uses ; the shim translates
/// the resulting `AgentOutcome` into a [`NanoRunnerOutcome`].
pub struct SwarmAgentNanoRunner<A: SwarmAgent> {
    inner: A,
}

impl<A: SwarmAgent> SwarmAgentNanoRunner<A> {
    /// **N23** : wrap a `SwarmAgent` as a `NanoRunner`.
    pub fn new(inner: A) -> Self {
        Self { inner }
    }
}

impl<A: SwarmAgent + std::fmt::Debug> std::fmt::Debug for SwarmAgentNanoRunner<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SwarmAgentNanoRunner")
            .field("inner", &self.inner)
            .finish()
    }
}

#[async_trait]
impl<A: SwarmAgent> NanoRunner for SwarmAgentNanoRunner<A> {
    async fn run(
        &self,
        _nano: &NanoAgent,
        ctx: &SwarmAgentContext,
    ) -> Result<NanoRunnerOutcome, AdapterError> {
        let outcome = self
            .inner
            .run(ctx.clone())
            .await
            .map_err(|e| AdapterError::DispatchFailed(e.to_string()))?;

        match outcome {
            AgentOutcome::Completed {
                cost_microcents,
                blackboard_writes,
                events_published,
                ..
            } => {
                let cost_cents = efe_nano_swarm_adapter::microcents_to_cents(cost_microcents);
                Ok(NanoRunnerOutcome::fresh()
                    .with_cost_cents(cost_cents)
                    .with_blackboard_writes(blackboard_writes)
                    .with_events_published(events_published))
            }
            AgentOutcome::Errored { kind, message } => Err(AdapterError::DispatchFailed(
                format!("{} : {}", kind.label(), message),
            )),
            AgentOutcome::TimedOut { timeout_ms } => Err(AdapterError::DispatchFailed(format!(
                "swarm-agent timed out after {timeout_ms} ms"
            ))),
            AgentOutcome::Skipped { reason } => Err(AdapterError::Skipped(reason)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Helper for building a Mode-A reference descriptor
// ─────────────────────────────────────────────────────────────────────

fn base_descriptor_mode_a(
    id: &str,
    description: &str,
    category: Category,
    stability: StabilityTier,
    output_schema: serde_json::Value,
) -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new(id).expect("static catalogue id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("static version"),
        description: description.to_string(),
        author: Author::Efe,
        category,
        stability_tier: stability,
        execution_mode: ExecutionMode::PureRust {
            entry: format!("{id}::run"),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: serde_json::json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": { "type": "string", "minLength": 0, "maxLength": 1_000_000 }
            }
        }),
        output_schema,
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.0001,
        latency_p99_ms_target: 50,
        audit: Default::default(),
        compliance: Default::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: catalogue_epoch(),
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Six descriptors
// ─────────────────────────────────────────────────────────────────────

/// **N23 + G12** : `ner_extractor` descriptor.
///
/// Mode A, FullDeterministic.  Reads `/input/text`, writes
/// `/entities : Vec<Entity>`.
pub fn ner_extractor_descriptor() -> NanoAgent {
    base_descriptor_mode_a(
        "ner_extractor",
        "Pure-Rust regex-based entity extractor : email, phone, url, money, ISO date.",
        Category::EntityRecognition,
        StabilityTier::Tier1,
        serde_json::json!({
            "type": "object",
            "properties": {
                "entities": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["entity_type", "value"],
                        "properties": {
                            "entity_type": { "type": "string" },
                            "value": { "type": "string" },
                            "span": { "type": "array", "items": { "type": "integer" } }
                        }
                    }
                }
            }
        }),
    )
}

/// **N23 + G12** : `ner_extractor` runner.
pub fn ner_extractor_runner() -> Arc<dyn NanoRunner> {
    Arc::new(SwarmAgentNanoRunner::new(NerExtractor::new()))
}

/// **N23 + G12** : `language_detector` descriptor.
pub fn language_detector_descriptor() -> NanoAgent {
    base_descriptor_mode_a(
        "language_detector",
        "Pure-Rust ISO-639-1 language detector (stopword count, 6 European languages).",
        Category::Classification,
        StabilityTier::Tier1,
        serde_json::json!({
            "type": "object",
            "properties": {
                "language": { "type": "string", "minLength": 2, "maxLength": 2 }
            }
        }),
    )
}

/// **N23 + G12** : `language_detector` runner.
pub fn language_detector_runner() -> Arc<dyn NanoRunner> {
    Arc::new(SwarmAgentNanoRunner::new(LanguageDetector::new()))
}

/// **N23 + G12** : `schema_validator` descriptor.
///
/// Reads `/input` (the full JSON, not `/input/text`).
pub fn schema_validator_descriptor() -> NanoAgent {
    base_descriptor_mode_a(
        "schema_validator",
        "Pure-Rust subset JSON-Schema validator (type, required, properties, min/max length, min/max number).",
        Category::Validation,
        StabilityTier::Tier1,
        serde_json::json!({
            "type": "object",
            "required": ["valid"],
            "properties": {
                "valid": { "type": "boolean" },
                "errors": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        }),
    )
}

/// **N23 + G12** : `schema_validator` runner.
///
/// The schema arg is provided at construction time via
/// `SchemaValidator::new(schema)`.  The catalogue back-fill ships
/// an "empty schema" (no constraints) so the gateway can re-wire
/// with the actual schema per request later.
pub fn schema_validator_runner() -> Arc<dyn NanoRunner> {
    Arc::new(SwarmAgentNanoRunner::new(SchemaValidator::new(
        serde_json::json!({}),
    )))
}

/// **N23 + G12** : `pii_redactor` descriptor (TierAudit per the
/// architecture doc — PII is regulatory-critical).
pub fn pii_redactor_descriptor() -> NanoAgent {
    let mut base = base_descriptor_mode_a(
        "pii_redactor",
        "Pure-Rust PII redactor : emails, phones, SSNs, credit cards.  Replaces with typed tokens.",
        Category::Transformation,
        StabilityTier::TierAudit,
        serde_json::json!({
            "type": "object",
            "properties": {
                "redacted_text": { "type": "string" },
                "redactions": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["redaction_type", "redaction_token"],
                        "properties": {
                            "redaction_type": { "type": "string" },
                            "redaction_token": { "type": "string" },
                            "original_span": { "type": "array", "items": { "type": "integer" } }
                        }
                    }
                }
            }
        }),
    );
    base.compliance.dpia_required = true;
    base
}

/// **N23 + G12** : `pii_redactor` runner.
pub fn pii_redactor_runner() -> Arc<dyn NanoRunner> {
    Arc::new(SwarmAgentNanoRunner::new(PiiRedactor::new()))
}

/// **N23 + G12** : `sentiment_classifier` descriptor — Mode B
/// stub (lexicon-based ; Sprint 14+ promotes to fine-tuned BERT).
pub fn sentiment_classifier_descriptor() -> NanoAgent {
    let mut base = base_descriptor_mode_a(
        "sentiment_classifier",
        "Lexicon-based positive/negative sentiment classifier with log-odds confidence.",
        Category::Classification,
        StabilityTier::TierExperimental,
        serde_json::json!({
            "type": "object",
            "required": ["label", "confidence"],
            "properties": {
                "label": { "type": "string" },
                "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                "positive_hits": { "type": "integer" },
                "negative_hits": { "type": "integer" }
            }
        }),
    );
    // Override the execution mode for Mode B per the design doc.
    base.execution_mode = ExecutionMode::LocalModel {
        model_id: "sentiment-lexicon".to_string(),
        model_version: "v0".to_string(),
    };
    base.determinism = DeterminismLevel::Seeded;
    base
}

/// **N23 + G12** : `sentiment_classifier` runner.
pub fn sentiment_classifier_runner() -> Arc<dyn NanoRunner> {
    Arc::new(SwarmAgentNanoRunner::new(SentimentClassifier::new()))
}

/// **N23 + G12** : `summary_writer` descriptor — Mode C stub.
pub fn summary_writer_descriptor() -> NanoAgent {
    let mut base = base_descriptor_mode_a(
        "summary_writer",
        "Truncation-based summariser (first N sentences, char-capped).  Sprint 14+ wires Claude Haiku.",
        Category::Summarization,
        StabilityTier::TierExperimental,
        serde_json::json!({
            "type": "object",
            "properties": {
                "summary": { "type": "string" }
            }
        }),
    );
    base.execution_mode = ExecutionMode::LlmAugmented {
        provider: "anthropic-stub".to_string(),
        model: "stub-v0".to_string(),
        prompt_template_id: "summarise_first_n".to_string(),
        max_tokens: 256,
        temperature: 0.0,
    };
    base.determinism = DeterminismLevel::LlmReplayable;
    base.idempotency = IdempotencyPolicy::Manual { ttl_sec: 3_600 };
    // Mode C typically does have a non-zero cost ; the stub keeps
    // 0.001 cent so the variance test in the audit still has a
    // non-zero estimate to compare against.
    base.cost_estimate_cents = 0.001;
    base.latency_p99_ms_target = 500;
    base
}

/// **N23 + G12** : `summary_writer` runner.
pub fn summary_writer_runner() -> Arc<dyn NanoRunner> {
    Arc::new(SwarmAgentNanoRunner::new(SummaryWriter::new()))
}

// ─────────────────────────────────────────────────────────────────────
//  default_catalogue
// ─────────────────────────────────────────────────────────────────────

/// **N23** : type alias for a catalogue entry — the catalogue
/// pairs each `NanoAgent` descriptor with its `NanoRunner`.
pub type CatalogueEntry = (Arc<NanoAgent>, Arc<dyn NanoRunner>);

/// **N23 + G12 + I20** : returns the six built-in catalogue
/// entries paired with their runners.
///
/// Order is **deterministic** : NER, language, schema, PII,
/// sentiment, summary.  The order matches the layered DAG :
/// layer 1 (parallel) gets NER + language + schema + sentiment ;
/// layer 2 the redactor ; layer 3 the summariser.
pub fn default_catalogue() -> Vec<CatalogueEntry> {
    vec![
        (Arc::new(ner_extractor_descriptor()), ner_extractor_runner()),
        (
            Arc::new(language_detector_descriptor()),
            language_detector_runner(),
        ),
        (
            Arc::new(schema_validator_descriptor()),
            schema_validator_runner(),
        ),
        (
            Arc::new(sentiment_classifier_descriptor()),
            sentiment_classifier_runner(),
        ),
        (Arc::new(pii_redactor_descriptor()), pii_redactor_runner()),
        (
            Arc::new(summary_writer_descriptor()),
            summary_writer_runner(),
        ),
    ]
}

/// **N23 + I20** : returns the canonical SHA-256 hex hash of the
/// catalogue's serialised descriptors.  Sprint 16+ replay uses
/// this as the catalogue anchor.
pub fn canonical_hash() -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"nano-catalogue-v1\x00");
    for (nano, _) in default_catalogue() {
        let bytes = serde_json::to_vec(&*nano).unwrap_or_default();
        h.update(&bytes);
        h.update(b"\x00");
    }
    let d = h.finalize();
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalogue_has_six_entries() {
        let c = default_catalogue();
        assert_eq!(c.len(), 6);
    }

    #[test]
    fn catalogue_ids_are_valid_nano_ids() {
        for (nano, _) in default_catalogue() {
            // The constructor would have panicked otherwise ; this
            // assertion is symbolic.
            assert!(!nano.id.as_str().is_empty());
            assert!(nano.id.as_str().chars().all(|c| c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '_'));
        }
    }

    #[test]
    fn catalogue_versions_are_semver() {
        for (nano, _) in default_catalogue() {
            assert!(nano.version.as_str().contains('.'));
        }
    }

    #[test]
    fn catalogue_canonical_hash_stable() {
        let h1 = canonical_hash();
        let h2 = canonical_hash();
        assert_eq!(h1, h2);
        // SHA-256 hex = 64 chars.
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn catalogue_categories_match_purpose() {
        let c = default_catalogue();
        assert_eq!(c[0].0.category, Category::EntityRecognition); // NER
        assert_eq!(c[1].0.category, Category::Classification); // language
        assert_eq!(c[2].0.category, Category::Validation); // schema
        assert_eq!(c[3].0.category, Category::Classification); // sentiment
        assert_eq!(c[4].0.category, Category::Transformation); // pii
        assert_eq!(c[5].0.category, Category::Summarization); // summary
    }

    #[test]
    fn pii_redactor_is_audit_tier_and_requires_dpia() {
        let nano = pii_redactor_descriptor();
        assert_eq!(nano.stability_tier, StabilityTier::TierAudit);
        assert!(nano.requires_dpia());
    }

    #[test]
    fn sentiment_classifier_is_mode_b() {
        let nano = sentiment_classifier_descriptor();
        assert_eq!(nano.execution_mode.mode_str(), "local_model");
        assert_eq!(nano.determinism, DeterminismLevel::Seeded);
    }

    #[test]
    fn summary_writer_is_mode_c() {
        let nano = summary_writer_descriptor();
        assert_eq!(nano.execution_mode.mode_str(), "llm_augmented");
        assert_eq!(nano.determinism, DeterminismLevel::LlmReplayable);
        assert!(matches!(nano.idempotency, IdempotencyPolicy::Manual { .. }));
    }

    #[test]
    fn catalogue_created_at_uses_fixed_epoch() {
        // I20 : determinism.  Every entry has the same
        // catalogue_epoch.
        for (nano, _) in default_catalogue() {
            assert_eq!(nano.created_at, catalogue_epoch());
        }
    }

    #[test]
    fn ner_extractor_descriptor_serde_round_trips() {
        let n = ner_extractor_descriptor();
        let json = serde_json::to_string(&n).unwrap();
        let back: NanoAgent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id.as_str(), "ner_extractor");
    }

    #[tokio::test]
    async fn ner_extractor_runner_dispatches_through_shim() {
        use efe_blackboard::{Blackboard, EventBus};
        use efe_nano_sdk::{AgentId, SwarmId};
        use std::sync::Arc as StdArc;

        let bb = StdArc::new(Blackboard::new(SwarmId::new()));
        bb.write(
            crate::INPUT_TEXT_POINTER,
            serde_json::json!("contact alice@example.com"),
            AgentId::new("seed").unwrap(),
        )
        .unwrap();
        let ctx = SwarmAgentContext {
            swarm_id: bb.swarm_id(),
            agent_id: AgentId::new("ner-extractor").unwrap(),
            blackboard: bb.clone(),
            eventbus: StdArc::new(EventBus::new()),
            input: StdArc::new(serde_json::json!({"text": "x"})),
            rng_seed: 0,
        };
        let runner = ner_extractor_runner();
        let nano = ner_extractor_descriptor();
        let outcome = runner.run(&nano, &ctx).await.unwrap();
        assert!(outcome.blackboard_writes >= 1);
        // The underlying NerExtractor writes /entities.
        assert!(bb.read("/entities").is_some());
    }
}
