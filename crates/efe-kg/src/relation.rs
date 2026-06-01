// SPDX-License-Identifier: UNLICENSED
//! M33 — Relation model + base predicate catalog.

use serde::{Deserialize, Serialize};

use crate::error::{KgError, Result};

/// Maximum byte length for a custom predicate slug.
pub const MAX_PREDICATE_LEN: usize = 32;

/// Base M33 predicate catalog (cf. architecture § 3.2).
pub const BASE_PREDICATES: &[&str] = &[
    "works_at",
    "located_in",
    "subsidiary_of",
    "member_of",
    "attended",
    "organized",
    "acquired",
    "released",
    "competes_with",
    "parent_of",
    "spouse_of",
    "founded",
    "ceo_of",
    "same_as",
    "mentions",
    "relates_to",
    "purchased",
    "tagged_with",
];

/// Predicate (edge label).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Predicate(pub String);

impl Predicate {
    /// Construct from a string slug. Validates against the M33 rules.
    pub fn new(slug: impl Into<String>) -> Result<Self> {
        let slug = slug.into();
        validate_predicate(&slug)?;
        Ok(Self(slug))
    }

    /// Construct a predicate known to be in [`BASE_PREDICATES`].
    /// Returns `Err(UnknownPredicate)` otherwise.
    pub fn base(slug: &str) -> Result<Self> {
        if !BASE_PREDICATES.contains(&slug) {
            return Err(KgError::UnknownPredicate(slug.to_string()));
        }
        Self::new(slug)
    }

    /// Borrow the slug.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` iff this predicate is one of the base ones.
    #[must_use]
    pub fn is_base(&self) -> bool {
        BASE_PREDICATES.contains(&self.0.as_str())
    }
}

/// Validate a predicate slug against `[a-z0-9_]{1, MAX_PREDICATE_LEN}`.
pub fn validate_predicate(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(KgError::UnknownPredicate("empty".into()));
    }
    if slug.len() > MAX_PREDICATE_LEN {
        return Err(KgError::UnknownPredicate(format!(
            "len {} > cap {MAX_PREDICATE_LEN}",
            slug.len()
        )));
    }
    if !slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(KgError::UnknownPredicate(format!(
            "invalid chars in `{slug}`"
        )));
    }
    Ok(())
}

/// One directed edge in the graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relation {
    /// Relation identifier (ULID).
    pub id: String,
    /// Subject entity id.
    pub subject_id: String,
    /// Predicate.
    pub predicate: Predicate,
    /// Object entity id.
    pub object_id: String,
    /// Aggregate confidence (0..1) — high if multiple claims agree.
    pub confidence: f32,
}

impl Relation {
    /// Construct.
    pub fn new(
        id: impl Into<String>,
        subject_id: impl Into<String>,
        predicate: Predicate,
        object_id: impl Into<String>,
        confidence: f32,
    ) -> Self {
        Self {
            id: id.into(),
            subject_id: subject_id.into(),
            predicate,
            object_id: object_id.into(),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }

    /// Validate the relation.
    pub fn validate(&self) -> Result<()> {
        if self.subject_id.is_empty() || self.object_id.is_empty() {
            return Err(KgError::Internal {
                reason: "subject_id and object_id must be non-empty".into(),
            });
        }
        if self.subject_id == self.object_id && !self.is_symmetric_self_allowed() {
            return Err(KgError::Internal {
                reason: format!(
                    "self-loop on predicate `{}` is not allowed",
                    self.predicate.as_str()
                ),
            });
        }
        if !self.confidence.is_finite() || !(0.0..=1.0).contains(&self.confidence) {
            return Err(KgError::Internal {
                reason: format!("confidence out of range: {}", self.confidence),
            });
        }
        validate_predicate(self.predicate.as_str())?;
        Ok(())
    }

    /// Whether a self-loop (subject_id == object_id) is allowed for
    /// this predicate. M33 allows it only for `same_as` (entity
    /// resolution sentinel).
    #[must_use]
    pub fn is_symmetric_self_allowed(&self) -> bool {
        self.predicate.as_str() == "same_as"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_predicates_all_validate() {
        for p in BASE_PREDICATES {
            Predicate::base(p).unwrap();
        }
    }

    #[test]
    fn predicate_validation_rejects_uppercase() {
        assert!(Predicate::new("BadPred").is_err());
    }

    #[test]
    fn predicate_validation_rejects_oversize() {
        let s = "x".repeat(MAX_PREDICATE_LEN + 1);
        assert!(Predicate::new(s).is_err());
    }

    #[test]
    fn predicate_validation_accepts_custom() {
        Predicate::new("ticket_about").unwrap();
    }

    #[test]
    fn relation_validate_rejects_empty_subject() {
        let r = Relation::new(
            "r1",
            "",
            Predicate::new("works_at").unwrap(),
            "e2",
            0.9,
        );
        assert!(r.validate().is_err());
    }

    #[test]
    fn relation_validate_rejects_self_loop_for_works_at() {
        let r = Relation::new(
            "r1",
            "e1",
            Predicate::new("works_at").unwrap(),
            "e1",
            0.9,
        );
        assert!(r.validate().is_err());
    }

    #[test]
    fn relation_validate_accepts_self_loop_for_same_as() {
        let r = Relation::new(
            "r1",
            "e1",
            Predicate::new("same_as").unwrap(),
            "e1",
            0.9,
        );
        r.validate().unwrap();
    }

    #[test]
    fn confidence_clamped_to_unit_interval() {
        let r = Relation::new("r1", "e1", Predicate::new("works_at").unwrap(), "e2", 2.5);
        assert_eq!(r.confidence, 1.0);
        let r = Relation::new("r1", "e1", Predicate::new("works_at").unwrap(), "e2", -0.5);
        assert_eq!(r.confidence, 0.0);
    }

    #[test]
    fn is_base_predicate() {
        assert!(Predicate::new("works_at").unwrap().is_base());
        assert!(!Predicate::new("ticket_about").unwrap().is_base());
    }
}
