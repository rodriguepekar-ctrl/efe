// SPDX-License-Identifier: UNLICENSED
//! M33 — Raw extraction claims.
//!
//! A `Claim` is a `(subject, predicate, object, confidence,
//! provenance)` 4-tuple emitted by the relation extractor. Claims
//! that meet the promotion criteria become `Relation` rows ; the
//! rest stay as soft evidence.
//!
//! **Promotion rule** (cf. architecture § 5.2) :
//! - At least 2 supporting claims OR
//! - One claim with confidence ≥ 0.9.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::relation::{Predicate, Relation};

/// Confidence threshold above which a single claim is sufficient
/// to promote into a `Relation`.
pub const CLAIM_PROMOTE_CONFIDENCE: f32 = 0.9;

/// Minimum supporting-claim count for promotion below the
/// single-claim threshold.
pub const CLAIM_PROMOTE_MIN_SUPPORT: usize = 2;

/// One raw extraction claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    /// Claim identifier (ULID).
    pub id: String,
    /// Subject entity id.
    pub subject_id: String,
    /// Predicate slug.
    pub predicate: Predicate,
    /// Object entity id.
    pub object_id: String,
    /// Extractor confidence (0..1).
    pub confidence: f32,
    /// Document id that produced this claim.
    pub provenance_doc_id: String,
    /// (start, end) char offsets in the source text.
    pub provenance_span: (usize, usize),
}

impl Claim {
    /// Construct.
    pub fn new(
        id: impl Into<String>,
        subject_id: impl Into<String>,
        predicate: Predicate,
        object_id: impl Into<String>,
        confidence: f32,
        provenance_doc_id: impl Into<String>,
        provenance_span: (usize, usize),
    ) -> Self {
        Self {
            id: id.into(),
            subject_id: subject_id.into(),
            predicate,
            object_id: object_id.into(),
            confidence: confidence.clamp(0.0, 1.0),
            provenance_doc_id: provenance_doc_id.into(),
            provenance_span,
        }
    }

    /// Sort key into `(subject_id, predicate, object_id)`. Used by
    /// the promotion grouper.
    #[must_use]
    pub fn group_key(&self) -> (String, String, String) {
        (
            self.subject_id.clone(),
            self.predicate.as_str().to_string(),
            self.object_id.clone(),
        )
    }
}

/// Promote a set of claims into `Relation` rows according to the
/// rule documented at the top of this module.
///
/// Returns `(promoted, unpromoted)`. Each promoted `Relation` is
/// assigned an aggregate confidence = max of supporting claims.
pub fn promote_claims_to_relations(claims: &[Claim]) -> Result<(Vec<Relation>, Vec<Claim>)> {
    // Group by (subject, predicate, object).
    let mut groups: HashMap<(String, String, String), Vec<&Claim>> = HashMap::new();
    for c in claims {
        groups.entry(c.group_key()).or_default().push(c);
    }

    let mut promoted = Vec::new();
    let mut unpromoted_keys: std::collections::HashSet<(String, String, String)> =
        std::collections::HashSet::new();

    for (key, supports) in &groups {
        let max_conf = supports
            .iter()
            .map(|c| c.confidence)
            .fold(0.0_f32, f32::max);
        let promote = supports.len() >= CLAIM_PROMOTE_MIN_SUPPORT
            || max_conf >= CLAIM_PROMOTE_CONFIDENCE;
        if promote {
            // Aggregate confidence = max ; one Relation per group.
            let pred = Predicate::new(&key.1)?;
            let rel = Relation::new(
                format!(
                    "rel-{}-{}-{}",
                    key.0,
                    key.1,
                    key.2
                ),
                key.0.clone(),
                pred,
                key.2.clone(),
                max_conf,
            );
            promoted.push(rel);
        } else {
            unpromoted_keys.insert(key.clone());
        }
    }

    let unpromoted: Vec<Claim> = claims
        .iter()
        .filter(|c| unpromoted_keys.contains(&c.group_key()))
        .cloned()
        .collect();

    Ok((promoted, unpromoted))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(s: &str, p: &str, o: &str, conf: f32) -> Claim {
        Claim::new(
            format!("c-{s}-{p}-{o}"),
            s,
            Predicate::new(p).unwrap(),
            o,
            conf,
            "doc1",
            (0, 0),
        )
    }

    #[test]
    fn high_confidence_single_claim_promotes() {
        let claims = vec![claim("a", "works_at", "b", 0.95)];
        let (p, u) = promote_claims_to_relations(&claims).unwrap();
        assert_eq!(p.len(), 1);
        assert!(u.is_empty());
        assert_eq!(p[0].confidence, 0.95);
    }

    #[test]
    fn low_confidence_single_claim_does_not_promote() {
        let claims = vec![claim("a", "works_at", "b", 0.7)];
        let (p, u) = promote_claims_to_relations(&claims).unwrap();
        assert!(p.is_empty());
        assert_eq!(u.len(), 1);
    }

    #[test]
    fn two_low_confidence_claims_promote() {
        let claims = vec![
            claim("a", "works_at", "b", 0.6),
            claim("a", "works_at", "b", 0.65),
        ];
        let (p, u) = promote_claims_to_relations(&claims).unwrap();
        assert_eq!(p.len(), 1);
        assert!(u.is_empty());
        assert!((p[0].confidence - 0.65).abs() < 1e-5);
    }

    #[test]
    fn distinct_objects_are_separate_relations() {
        let claims = vec![
            claim("a", "works_at", "b", 0.95),
            claim("a", "works_at", "c", 0.95),
        ];
        let (p, _u) = promote_claims_to_relations(&claims).unwrap();
        assert_eq!(p.len(), 2);
        let objs: std::collections::HashSet<_> = p.iter().map(|r| r.object_id.clone()).collect();
        assert!(objs.contains("b"));
        assert!(objs.contains("c"));
    }

    #[test]
    fn group_key_groups_same_triple() {
        let c1 = claim("a", "works_at", "b", 0.5);
        let c2 = claim("a", "works_at", "b", 0.6);
        assert_eq!(c1.group_key(), c2.group_key());
    }
}
