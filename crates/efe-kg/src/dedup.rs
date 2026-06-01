// SPDX-License-Identifier: UNLICENSED
//! M33 — Entity resolution / deduplication.
//!
//! Two-stage resolution :
//!
//! 1. **Name-similarity** : Levenshtein distance on canonical names
//!    ≤ `MAX_LEV_DISTANCE` (default 3).
//! 2. **Vector-similarity** : cosine similarity ≥ `COSINE_THRESHOLD`
//!    (default 0.85) on entity embeddings.
//!
//! Both conditions must hold for a merge. If only the name matches
//! we surface a `Candidate` (suggested but not merged).

use crate::entity::Entity;

/// Default cosine threshold for vector-similarity match.
pub const COSINE_THRESHOLD: f32 = 0.85;

/// Default Levenshtein distance cap for name-similarity match.
pub const MAX_LEV_DISTANCE: usize = 3;

/// Result of comparing two entities.
#[derive(Debug, Clone, PartialEq)]
pub enum DedupVerdict {
    /// Both name and vector are close ⇒ merge automatically.
    Merge,
    /// Names match but vectors disagree ⇒ surface to operator.
    Candidate,
    /// Distinct entities.
    Distinct,
}

/// Compute Levenshtein distance between `a` and `b`. Capped at
/// `cap`. Returns `cap + 1` if the actual distance exceeds the cap.
#[must_use]
pub fn levenshtein_capped(a: &str, b: &str, cap: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if (a.len().abs_diff(b.len())) > cap {
        return cap + 1;
    }
    if a.is_empty() {
        return b.len().min(cap + 1);
    }
    if b.is_empty() {
        return a.len().min(cap + 1);
    }
    // Two-row DP.
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0_usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (cur[j] + 1)
                .min(prev[j + 1] + 1)
                .min(prev[j] + cost);
            if cur[j + 1] < row_min {
                row_min = cur[j + 1];
            }
        }
        if row_min > cap {
            return cap + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Cosine similarity between two vectors. Returns `None` if dim
/// mismatch or zero norm.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    let na = na.sqrt();
    let nb = nb.sqrt();
    if na < 1e-12 || nb < 1e-12 {
        return None;
    }
    Some(dot / (na * nb))
}

/// Compare two entities and produce a verdict.
#[must_use]
pub fn classify(a: &Entity, b: &Entity) -> DedupVerdict {
    // 1. Same id ⇒ trivially merge (no-op).
    if a.id == b.id {
        return DedupVerdict::Merge;
    }
    // 2. Different kinds ⇒ never merge.
    if a.kind != b.kind {
        return DedupVerdict::Distinct;
    }
    // 3. Name distance.
    let lev = levenshtein_capped(&a.canonical_name, &b.canonical_name, MAX_LEV_DISTANCE);
    let name_match = lev <= MAX_LEV_DISTANCE;
    if !name_match {
        return DedupVerdict::Distinct;
    }
    // 4. Vector similarity (only when both have embeddings).
    let cos = match (a.embedding.as_ref(), b.embedding.as_ref()) {
        (Some(va), Some(vb)) => cosine(va, vb),
        _ => None,
    };
    match cos {
        Some(c) if c >= COSINE_THRESHOLD => DedupVerdict::Merge,
        Some(_) => DedupVerdict::Candidate, // name OK, vectors disagree
        None => DedupVerdict::Candidate,    // name OK, no vectors → manual review
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;

    fn ent(id: &str, kind: EntityKind, name: &str) -> Entity {
        Entity::new(id, kind, name)
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein_capped("kitten", "sitting", 5), 3);
        assert_eq!(levenshtein_capped("", "abc", 5), 3);
        assert_eq!(levenshtein_capped("abc", "", 5), 3);
        assert_eq!(levenshtein_capped("abc", "abc", 5), 0);
    }

    #[test]
    fn levenshtein_cap_short_circuit() {
        // Strings differ by way more than the cap.
        assert_eq!(levenshtein_capped("a", "abcdefghij", 2), 3);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        assert_eq!(cosine(&[1.0, 0.0], &[0.0, 1.0]), Some(0.0));
    }

    #[test]
    fn cosine_identical_is_one() {
        assert!((cosine(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]).unwrap() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_zero_vector_is_none() {
        assert!(cosine(&[0.0, 0.0], &[1.0, 1.0]).is_none());
    }

    #[test]
    fn classify_different_kind_never_merges() {
        let a = ent("e1", EntityKind::Person, "alice");
        let b = ent("e2", EntityKind::Org, "alice");
        assert_eq!(classify(&a, &b), DedupVerdict::Distinct);
    }

    #[test]
    fn classify_distant_names_are_distinct() {
        let a = ent("e1", EntityKind::Person, "alice");
        let b = ent("e2", EntityKind::Person, "bob smith");
        assert_eq!(classify(&a, &b), DedupVerdict::Distinct);
    }

    #[test]
    fn classify_close_names_no_vector_is_candidate() {
        let a = ent("e1", EntityKind::Person, "alice");
        let b = ent("e2", EntityKind::Person, "alicee"); // distance 1
        assert_eq!(classify(&a, &b), DedupVerdict::Candidate);
    }

    #[test]
    fn classify_close_names_and_close_vector_merges() {
        let a = Entity::new("e1", EntityKind::Person, "alice")
            .with_embedding(vec![1.0, 0.0, 0.0]);
        let b = Entity::new("e2", EntityKind::Person, "alic")
            .with_embedding(vec![0.99, 0.01, 0.0]);
        assert_eq!(classify(&a, &b), DedupVerdict::Merge);
    }

    #[test]
    fn classify_close_names_but_distant_vector_is_candidate() {
        let a = Entity::new("e1", EntityKind::Person, "alice")
            .with_embedding(vec![1.0, 0.0, 0.0]);
        let b = Entity::new("e2", EntityKind::Person, "alic")
            .with_embedding(vec![0.0, 1.0, 0.0]);
        assert_eq!(classify(&a, &b), DedupVerdict::Candidate);
    }

    #[test]
    fn same_id_always_merges() {
        let a = ent("e1", EntityKind::Person, "alice");
        let b = ent("e1", EntityKind::Org, "completely different");
        assert_eq!(classify(&a, &b), DedupVerdict::Merge);
    }
}
