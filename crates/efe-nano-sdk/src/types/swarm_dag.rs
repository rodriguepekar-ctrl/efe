// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : Swarm orchestration + agentic bridge SDK types.
//!
//! Architecture § 7 + annex A3.  All types are wire-frozen Sprint 13.
//!
//! # Module surface
//!
//! - [`SwarmId`] / [`AgentId`] — opaque identifiers.
//! - [`ContextBundle`] — the structured output of the swarm pre-analysis,
//!   consumed by the agentic LLM (e.g. Claude) via tool-use.
//! - [`Entity`], [`Redaction`], [`Classification`] — the typed sub-rows.
//! - [`ContextBundleMetadata`] — agents executed, cost, size.
//! - [`MAX_CONTEXT_BUNDLE_BYTES`] — 256 KB hard cap (I10).
//!
//! # Wire compatibility
//!
//! - All types serialise via `serde` with stable JSON shape.
//! - `Option<_>` fields use `skip_serializing_if = "Option::is_none"`.
//! - Adding new enum variants is wire-compatible.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Identifiers
// ─────────────────────────────────────────────────────────────────────

/// **N21** : opaque swarm run identifier (one per `SwarmOrchestrator::run`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SwarmId(pub Uuid);

impl SwarmId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(u: Uuid) -> Self {
        Self(u)
    }
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SwarmId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SwarmId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// **N21** : opaque agent identifier inside a swarm DAG.
///
/// Lowercase ASCII alphanumeric + underscore + dash, 3..=64 chars.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    /// **N21** : Construct ; validates the format.
    pub fn new(s: impl Into<String>) -> Result<Self, AgentIdError> {
        let s = s.into();
        if s.is_empty() {
            return Err(AgentIdError::Empty);
        }
        if !(3..=64).contains(&s.len()) {
            return Err(AgentIdError::WrongLength {
                got: s.len(),
                min: 3,
                max: 64,
            });
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(AgentIdError::IllegalChar);
        }
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentIdError {
    #[error("agent id is empty")]
    Empty,

    #[error("agent id length {got} out of bounds [{min}, {max}]")]
    WrongLength { got: usize, min: usize, max: usize },

    #[error("agent id contains illegal char (only [a-z0-9_-] allowed)")]
    IllegalChar,
}

// ─────────────────────────────────────────────────────────────────────
//  ContextBundle — what the swarm gives to the agentic LLM
// ─────────────────────────────────────────────────────────────────────

/// **N21 + G8 + I10** : 256 KB hard cap on serialised `ContextBundle`.
pub const MAX_CONTEXT_BUNDLE_BYTES: usize = 256 * 1024;

/// **N21** : The structured bundle handed to the agentic LLM.
///
/// Field semantics :
/// - `entities` : list of named entities extracted by the swarm.
/// - `language` : ISO-639-1 code (e.g. "en", "fr") ; None if undetected.
/// - `redactions` : every PII redaction performed (transparent to the LLM).
/// - `summary` : short textual summary (~200 words max).
/// - `classifications` : labels + confidence.
/// - `extras` : free-form domain-specific blob (e.g. clauses for a
///   contract analyser).
/// - `metadata` : per-bundle accounting (cost, swarm_id, agents_executed).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextBundle {
    pub swarm_id: SwarmId,
    pub generated_at: SystemTime,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entities: Vec<Entity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redactions: Vec<Redaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub classifications: Vec<Classification>,
    #[serde(default)]
    pub extras: serde_json::Map<String, serde_json::Value>,
    pub metadata: ContextBundleMetadata,
}

impl ContextBundle {
    /// **N21** : Build a minimal bundle with no entities, no language, no
    /// summary — pure scaffolding for tests + early swarm runs.
    pub fn empty(swarm_id: SwarmId) -> Self {
        Self {
            swarm_id,
            generated_at: SystemTime::now(),
            entities: vec![],
            language: None,
            redactions: vec![],
            summary: None,
            classifications: vec![],
            extras: serde_json::Map::new(),
            metadata: ContextBundleMetadata {
                agents_executed: vec![],
                total_cost_microcents: 0,
                size_estimate_bytes: 0,
            },
        }
    }

    /// **N21 + I10 + F14** : Estimate the serialised size in bytes.
    /// Used by the aggregator to enforce the 256 KB cap.
    pub fn estimated_size_bytes(&self) -> usize {
        serde_json::to_vec(self).map(|v| v.len()).unwrap_or(0)
    }

    /// **N21 + I10** : True if the bundle fits within the
    /// `MAX_CONTEXT_BUNDLE_BYTES` cap.
    pub fn fits_size_cap(&self) -> bool {
        self.estimated_size_bytes() <= MAX_CONTEXT_BUNDLE_BYTES
    }

    /// **N21** : Set the metadata size estimate to the actual
    /// serialised size (called by the aggregator).
    pub fn finalize_size(&mut self) {
        let size = self.estimated_size_bytes();
        self.metadata.size_estimate_bytes = size;
    }
}

/// **N21** : per-bundle accounting metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBundleMetadata {
    pub agents_executed: Vec<AgentId>,
    pub total_cost_microcents: i64,
    pub size_estimate_bytes: usize,
}

// ─────────────────────────────────────────────────────────────────────
//  Entity / Redaction / Classification
// ─────────────────────────────────────────────────────────────────────

/// **N21** : a named entity extracted from the input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    /// Entity type (e.g. "person", "organisation", "location",
    /// "date", "money").
    #[serde(rename = "type")]
    pub entity_type: String,
    pub value: String,
    /// Character span `[start, end)` in the input text (None if the
    /// agent works on non-text inputs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<(u32, u32)>,
}

/// **N21** : a single PII redaction event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Redaction {
    /// PII category (e.g. "email", "phone", "ssn", "credit_card",
    /// "person_name").
    #[serde(rename = "type")]
    pub redaction_type: String,
    /// Character span in the original text.
    pub original_span: (u32, u32),
    /// The token that replaced the PII in the redacted text (e.g.
    /// "<EMAIL_0>", "<PHONE_3>").
    pub redaction_token: String,
}

/// **N21** : a classification label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    pub label: String,
    /// Confidence in `[0.0, 1.0]`.
    pub confidence: f64,
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    #[test]
    fn swarm_id_round_trip() {
        let id = SwarmId::new();
        let json = serde_json::to_string(&id).unwrap();
        let back: SwarmId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn agent_id_accepts_valid() {
        assert!(AgentId::new("ner_extractor").is_ok());
        assert!(AgentId::new("lang-detect").is_ok());
        assert!(AgentId::new("a1b2c3").is_ok());
    }

    #[test]
    fn agent_id_rejects_empty() {
        assert!(matches!(AgentId::new(""), Err(AgentIdError::Empty)));
    }

    #[test]
    fn agent_id_rejects_too_short() {
        assert!(matches!(
            AgentId::new("ab"),
            Err(AgentIdError::WrongLength { .. })
        ));
    }

    #[test]
    fn agent_id_rejects_too_long() {
        let long = "a".repeat(65);
        assert!(matches!(
            AgentId::new(long),
            Err(AgentIdError::WrongLength { .. })
        ));
    }

    #[test]
    fn agent_id_rejects_uppercase() {
        assert!(matches!(
            AgentId::new("NerExtractor"),
            Err(AgentIdError::IllegalChar)
        ));
    }

    #[test]
    fn agent_id_rejects_dot_or_slash() {
        assert!(matches!(
            AgentId::new("foo.bar"),
            Err(AgentIdError::IllegalChar)
        ));
        assert!(matches!(
            AgentId::new("foo/bar"),
            Err(AgentIdError::IllegalChar)
        ));
    }

    #[test]
    fn agent_id_serde_round_trip() {
        let id = agent("ner_extractor");
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"ner_extractor\"");
        let back: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(back.as_str(), "ner_extractor");
    }

    #[test]
    fn context_bundle_empty_constructible() {
        let b = ContextBundle::empty(SwarmId::new());
        assert!(b.entities.is_empty());
        assert!(b.language.is_none());
        assert_eq!(b.metadata.total_cost_microcents, 0);
    }

    #[test]
    fn context_bundle_serde_round_trip() {
        let mut b = ContextBundle::empty(SwarmId::new());
        b.language = Some("en".into());
        b.entities.push(Entity {
            entity_type: "person".into(),
            value: "John Doe".into(),
            span: Some((120, 128)),
        });
        b.redactions.push(Redaction {
            redaction_type: "email".into(),
            original_span: (400, 420),
            redaction_token: "<EMAIL_0>".into(),
        });
        b.classifications.push(Classification {
            label: "consulting".into(),
            confidence: 0.92,
        });
        b.metadata.agents_executed.push(agent("ner_extractor"));
        b.metadata.total_cost_microcents = 42;
        let json = serde_json::to_string(&b).unwrap();
        let back: ContextBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn context_bundle_estimated_size_grows_with_content() {
        let mut b = ContextBundle::empty(SwarmId::new());
        let s1 = b.estimated_size_bytes();
        b.summary = Some("x".repeat(1000));
        let s2 = b.estimated_size_bytes();
        assert!(s2 > s1);
    }

    #[test]
    fn context_bundle_fits_size_cap_default() {
        let b = ContextBundle::empty(SwarmId::new());
        assert!(b.fits_size_cap());
    }

    #[test]
    fn context_bundle_huge_summary_busts_cap() {
        let mut b = ContextBundle::empty(SwarmId::new());
        // 300 KB > 256 KB.
        b.summary = Some("x".repeat(300 * 1024));
        assert!(!b.fits_size_cap());
    }

    #[test]
    fn context_bundle_finalize_size_updates_metadata() {
        let mut b = ContextBundle::empty(SwarmId::new());
        b.summary = Some("hello".into());
        b.finalize_size();
        assert!(b.metadata.size_estimate_bytes > 0);
    }

    #[test]
    fn entity_serde_round_trip() {
        let e = Entity {
            entity_type: "person".into(),
            value: "Alice".into(),
            span: Some((0, 5)),
        };
        let json = serde_json::to_string(&e).unwrap();
        // Field renamed from `entity_type` to `type` on the wire.
        assert!(json.contains("\"type\":\"person\""));
        let back: Entity = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn entity_span_optional() {
        let e = Entity {
            entity_type: "url".into(),
            value: "https://example.com".into(),
            span: None,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(!json.contains("\"span\""));
    }

    #[test]
    fn redaction_serde_round_trip() {
        let r = Redaction {
            redaction_type: "email".into(),
            original_span: (10, 30),
            redaction_token: "<EMAIL_0>".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"type\":\"email\""));
        let back: Redaction = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn classification_confidence_range_preserved() {
        let c = Classification {
            label: "high_risk".into(),
            confidence: 0.85,
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Classification = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn max_context_bundle_bytes_is_256k() {
        assert_eq!(MAX_CONTEXT_BUNDLE_BYTES, 262_144);
    }

    #[test]
    fn agent_id_display_matches_as_str() {
        let a = agent("ner_extractor");
        assert_eq!(format!("{a}"), "ner_extractor");
    }

    #[test]
    fn swarm_id_display_uuid() {
        let id = SwarmId::from_uuid(Uuid::nil());
        assert_eq!(format!("{id}"), "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn agent_id_error_display_non_empty() {
        let e = AgentIdError::WrongLength {
            got: 2,
            min: 3,
            max: 64,
        };
        assert!(!format!("{e}").is_empty());
    }
}
