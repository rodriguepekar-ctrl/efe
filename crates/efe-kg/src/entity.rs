// SPDX-License-Identifier: UNLICENSED
//! M33 — Entity model.

use serde::{Deserialize, Serialize};

use crate::error::{KgError, Result};

/// Maximum byte length for an entity's canonical name. Above the
/// cap we refuse the ingest.
pub const MAX_ENTITY_NAME_LEN: usize = 200;

/// Enumerated NER label set. The 6 categories below cover the
/// vast majority of enterprise NER use cases. Custom labels are
/// out of scope for M33 (deferred to M33.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// A human individual.
    Person,
    /// An organization, company, or institution.
    Org,
    /// A geographic location.
    Location,
    /// A commercial product or service.
    Product,
    /// A named event (conference, incident, release, …).
    Event,
    /// Miscellaneous — fallback bucket.
    Misc,
}

impl EntityKind {
    /// Static name (used as a metric label).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Org => "org",
            Self::Location => "location",
            Self::Product => "product",
            Self::Event => "event",
            Self::Misc => "misc",
        }
    }

    /// Parse from name. Returns `None` if unknown.
    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "person" => Some(Self::Person),
            "org" => Some(Self::Org),
            "location" => Some(Self::Location),
            "product" => Some(Self::Product),
            "event" => Some(Self::Event),
            "misc" => Some(Self::Misc),
            _ => None,
        }
    }

    /// All variants in stable enum order.
    pub fn all() -> &'static [EntityKind] {
        &[
            Self::Person,
            Self::Org,
            Self::Location,
            Self::Product,
            Self::Event,
            Self::Misc,
        ]
    }
}

/// One entity in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// ULID-shaped identifier.
    pub id: String,
    /// Entity kind.
    pub kind: EntityKind,
    /// Canonical name (lowercased + normalized).
    pub canonical_name: String,
    /// Raw surface forms observed in source text (deduplicated).
    #[serde(default)]
    pub mentions: Vec<String>,
    /// Optional dense vector (768-d by default) used by GraphRAG
    /// fusion. `None` when no embedder has run yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    /// Confidence score from the NER extractor (0..1).
    pub confidence: f32,
}

impl Entity {
    /// Construct from `(kind, canonical_name)`.
    pub fn new(id: impl Into<String>, kind: EntityKind, canonical_name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind,
            canonical_name: canonical_name.into(),
            mentions: Vec::new(),
            embedding: None,
            confidence: 1.0,
        }
    }

    /// Builder-style mention setter.
    #[must_use]
    pub fn with_mention(mut self, mention: impl Into<String>) -> Self {
        self.mentions.push(mention.into());
        self
    }

    /// Builder-style embedding setter.
    #[must_use]
    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    /// Builder-style confidence setter.
    #[must_use]
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// Validate the entity. Returns `Err` for malformed inputs.
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.canonical_name)?;
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(KgError::Internal {
                reason: format!("confidence out of range: {}", self.confidence),
            });
        }
        Ok(())
    }
}

/// Validate an entity name (also used by direct CRUD ingress).
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(KgError::InvalidEntityName {
            reason: "name is empty".into(),
        });
    }
    if name.len() > MAX_ENTITY_NAME_LEN {
        return Err(KgError::InvalidEntityName {
            reason: format!(
                "name length {} > cap {MAX_ENTITY_NAME_LEN}",
                name.len()
            ),
        });
    }
    Ok(())
}

/// Normalize a raw surface form into a canonical name :
///
/// 1. trim whitespace
/// 2. lowercase (Unicode-aware)
/// 3. collapse multi-space runs
///
/// Diacritics are **preserved** (`naïve` ≠ `naive` ⇒ kept distinct)
/// — M33.1 will add an optional diacritic-strip mode.
#[must_use]
pub fn canonicalize_name(s: &str) -> String {
    let trimmed = s.trim().to_lowercase();
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_space = false;
    for c in trimmed.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_name_roundtrip() {
        for k in EntityKind::all() {
            assert_eq!(EntityKind::from_name(k.name()), Some(*k));
        }
    }

    #[test]
    fn from_name_rejects_unknown() {
        assert_eq!(EntityKind::from_name("not-a-kind"), None);
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(matches!(
            validate_name(""),
            Err(KgError::InvalidEntityName { .. })
        ));
    }

    #[test]
    fn validate_name_rejects_oversize() {
        let s = "x".repeat(MAX_ENTITY_NAME_LEN + 1);
        assert!(matches!(
            validate_name(&s),
            Err(KgError::InvalidEntityName { .. })
        ));
    }

    #[test]
    fn validate_name_accepts_unicode() {
        validate_name("Société Générale").unwrap();
    }

    #[test]
    fn canonicalize_lowercases_and_collapses_spaces() {
        assert_eq!(canonicalize_name("  Acme   Corp  "), "acme corp");
    }

    #[test]
    fn canonicalize_preserves_diacritics() {
        assert_eq!(canonicalize_name("Société"), "société");
    }

    #[test]
    fn entity_builder_sets_fields() {
        let e = Entity::new("e1", EntityKind::Person, "alice")
            .with_mention("Alice")
            .with_mention("A. Smith")
            .with_embedding(vec![1.0, 0.0])
            .with_confidence(0.9);
        assert_eq!(e.mentions.len(), 2);
        assert!(e.embedding.is_some());
        assert_eq!(e.confidence, 0.9);
    }

    #[test]
    fn entity_validate_rejects_invalid_confidence() {
        let mut e = Entity::new("e1", EntityKind::Person, "alice");
        e.confidence = f32::NAN;
        assert!(e.validate().is_err());
    }
}
