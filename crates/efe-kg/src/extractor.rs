// SPDX-License-Identifier: UNLICENSED
//! M33 — NER + Relation extractor traits.
//!
//! Two traits abstract the extraction backends :
//!
//! - [`NerExtractor`] : text → list of (span, kind, name).
//! - [`RelationExtractor`] : sentence + entity pair → predicate +
//!   confidence.
//!
//! Concrete impls (Triton / sidecar) live in `efe-server`. This
//! crate ships an [`InMemoryNer`] + [`InMemoryRe`] for tests.

use async_trait::async_trait;

use crate::entity::EntityKind;
use crate::error::Result;
use crate::relation::Predicate;

/// Maximum input text byte length.
pub const MAX_INPUT_BYTES: usize = 100 * 1024; // 100 KB

/// One extracted entity span.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySpan {
    /// (start_char, end_char) offset in the source text.
    pub span: (usize, usize),
    /// Surface form (raw substring).
    pub surface: String,
    /// Predicted kind.
    pub kind: EntityKind,
    /// Confidence (0..1).
    pub confidence: f32,
}

/// NER extractor trait.
#[async_trait]
pub trait NerExtractor: Send + Sync {
    /// Extract every entity span from `text`. Returns spans in
    /// document order (start_char ascending).
    async fn extract(&self, text: &str) -> Result<Vec<EntitySpan>>;
}

/// Relation-extractor request.
#[derive(Debug, Clone)]
pub struct RelationRequest<'a> {
    /// Sentence containing both entities.
    pub sentence: &'a str,
    /// Subject entity span.
    pub subject: &'a EntitySpan,
    /// Object entity span.
    pub object: &'a EntitySpan,
}

/// One extracted relation hypothesis.
#[derive(Debug, Clone, PartialEq)]
pub struct RelationHypothesis {
    /// Predicted predicate.
    pub predicate: Predicate,
    /// Confidence (0..1).
    pub confidence: f32,
}

/// Relation-extractor trait.
#[async_trait]
pub trait RelationExtractor: Send + Sync {
    /// Predict the most-likely predicate for the given entity pair.
    /// `None` ⇒ no plausible relation. Implementations should
    /// internally cap the candidate-predicate set to the base
    /// catalog + the tenant's custom predicates.
    async fn predict(&self, req: RelationRequest<'_>) -> Result<Option<RelationHypothesis>>;
}

// ─────────────────────────── IN-MEMORY ───────────────────────────

/// Type alias for the in-memory NER seed map. Extracted to silence
/// the clippy `type_complexity` warning + improve readability.
pub type InMemoryNerSeeds =
    std::sync::Arc<parking_lot::RwLock<Vec<(String, Vec<EntitySpan>)>>>;

/// In-memory NER for tests : reads pre-seeded spans keyed on the
/// **exact** input text.
#[derive(Default, Clone)]
pub struct InMemoryNer {
    /// Map of `text → spans`. Linear search ; intended for tests.
    pub seeds: InMemoryNerSeeds,
}

impl InMemoryNer {
    /// Construct empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a (text → spans) mapping.
    pub fn seed(&self, text: impl Into<String>, spans: Vec<EntitySpan>) {
        self.seeds.write().push((text.into(), spans));
    }
}

#[async_trait]
impl NerExtractor for InMemoryNer {
    async fn extract(&self, text: &str) -> Result<Vec<EntitySpan>> {
        if text.len() > MAX_INPUT_BYTES {
            return Err(crate::error::KgError::InputTooLarge {
                observed: text.len(),
                cap: MAX_INPUT_BYTES,
            });
        }
        let seeds = self.seeds.read();
        for (k, v) in seeds.iter() {
            if k == text {
                return Ok(v.clone());
            }
        }
        Ok(Vec::new())
    }
}

/// In-memory RE for tests : returns a configured prediction
/// regardless of input.
#[derive(Default, Clone)]
pub struct InMemoryRe {
    /// Fixed prediction returned by every `predict` call.
    pub fixed: std::sync::Arc<parking_lot::RwLock<Option<RelationHypothesis>>>,
}

impl InMemoryRe {
    /// Construct empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure the fixed prediction.
    pub fn set(&self, hyp: Option<RelationHypothesis>) {
        *self.fixed.write() = hyp;
    }
}

#[async_trait]
impl RelationExtractor for InMemoryRe {
    async fn predict(&self, _req: RelationRequest<'_>) -> Result<Option<RelationHypothesis>> {
        Ok(self.fixed.read().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_memory_ner_returns_seeded_spans() {
        let ner = InMemoryNer::new();
        ner.seed(
            "Alice works at Acme",
            vec![
                EntitySpan {
                    span: (0, 5),
                    surface: "Alice".into(),
                    kind: EntityKind::Person,
                    confidence: 0.95,
                },
                EntitySpan {
                    span: (15, 19),
                    surface: "Acme".into(),
                    kind: EntityKind::Org,
                    confidence: 0.9,
                },
            ],
        );
        let out = ner.extract("Alice works at Acme").await.unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].kind, EntityKind::Person);
    }

    #[tokio::test]
    async fn in_memory_ner_rejects_oversize() {
        let ner = InMemoryNer::new();
        let big = "x".repeat(MAX_INPUT_BYTES + 1);
        let err = ner.extract(&big).await.unwrap_err();
        assert!(matches!(err, crate::error::KgError::InputTooLarge { .. }));
    }

    #[tokio::test]
    async fn in_memory_re_returns_configured_prediction() {
        let re = InMemoryRe::new();
        re.set(Some(RelationHypothesis {
            predicate: Predicate::new("works_at").unwrap(),
            confidence: 0.95,
        }));
        let p = EntitySpan {
            span: (0, 5),
            surface: "Alice".into(),
            kind: EntityKind::Person,
            confidence: 0.95,
        };
        let q = EntitySpan {
            span: (15, 19),
            surface: "Acme".into(),
            kind: EntityKind::Org,
            confidence: 0.9,
        };
        let out = re
            .predict(RelationRequest {
                sentence: "Alice works at Acme",
                subject: &p,
                object: &q,
            })
            .await
            .unwrap();
        assert!(out.is_some());
        assert_eq!(out.unwrap().predicate.as_str(), "works_at");
    }
}
