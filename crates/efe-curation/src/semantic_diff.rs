//! Semantic diff between two taxonomy versions.
//!
//! Detects class-level changes (added, removed, renamed, split, merged) and
//! anchor-level changes (added, removed, modified). Produces an estimated
//! impact metric (predicted accuracy delta) when a gold dataset is provided.
//!
//! See § 11.4 of `EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`.

use crate::{ClassId, CurationError, CurationResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A single anchor : a sentence + an optional pre-computed embedding centroid.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Anchor {
    pub text: String,
    #[serde(default)]
    pub embedding_centroid: Option<Vec<f32>>,
}

/// A taxonomy class (set of anchors + metadata).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomyClass {
    pub id: ClassId,
    pub display_name: String,
    pub anchors: Vec<Anchor>,
    #[serde(default)]
    pub centroid: Option<Vec<f32>>,
}

/// Whole-taxonomy snapshot used for diff input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomySnapshot {
    pub semver: String,
    pub classes: Vec<TaxonomyClass>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClassChange {
    Added { class_id: ClassId, anchors_count: usize },
    Removed { class_id: ClassId },
    Renamed { old_id: ClassId, new_id: ClassId },
    Split { from: ClassId, to: Vec<ClassId> },
    Merged { from: Vec<ClassId>, to: ClassId },
    AnchorsChanged {
        class_id: ClassId,
        added: Vec<String>,
        removed: Vec<String>,
        cosine_centroid_shift: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimatedImpact {
    pub affected_examples_count: u64,
    pub estimated_accuracy_delta_pp: f32,
    pub re_label_required: u64,
    pub confusion_matrix_delta_norm: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticDiff {
    pub from_semver: String,
    pub to_semver: String,
    pub class_changes: Vec<ClassChange>,
    pub estimated_impact: Option<EstimatedImpact>,
}

/// Compute a structural diff between two taxonomy snapshots.
///
/// Detects in priority order :
///   1. Renames (cosine similarity > 0.95 between centroids)
///   2. Splits (one class in `from` decomposed into 2-3 in `to`)
///   3. Merges (2-3 in `from` consolidated into one in `to`)
///   4. Plain adds / removes (residual)
///   5. Anchors-changed (same class id, anchors differ)
pub fn compute_diff(from: &TaxonomySnapshot, to: &TaxonomySnapshot) -> CurationResult<SemanticDiff> {
    let from_index: HashMap<&str, &TaxonomyClass> =
        from.classes.iter().map(|c| (c.id.0.as_str(), c)).collect();
    let to_index: HashMap<&str, &TaxonomyClass> =
        to.classes.iter().map(|c| (c.id.0.as_str(), c)).collect();

    let from_ids: BTreeSet<&str> = from_index.keys().copied().collect();
    let to_ids: BTreeSet<&str> = to_index.keys().copied().collect();

    let common: Vec<&str> = from_ids.intersection(&to_ids).copied().collect();
    let only_from: Vec<&str> = from_ids.difference(&to_ids).copied().collect();
    let only_to: Vec<&str> = to_ids.difference(&from_ids).copied().collect();

    let mut changes: Vec<ClassChange> = Vec::new();

    // 1. anchors-changed within common ids
    for id in &common {
        let f = from_index[id]; let t = to_index[id];
        let f_anchors: BTreeSet<&str> = f.anchors.iter().map(|a| a.text.as_str()).collect();
        let t_anchors: BTreeSet<&str> = t.anchors.iter().map(|a| a.text.as_str()).collect();
        if f_anchors != t_anchors {
            let added: Vec<String> = t_anchors.difference(&f_anchors).map(|s| s.to_string()).collect();
            let removed: Vec<String> = f_anchors.difference(&t_anchors).map(|s| s.to_string()).collect();
            let shift = match (&f.centroid, &t.centroid) {
                (Some(a), Some(b)) => 1.0 - cosine_similarity(a, b),
                _ => 0.0,
            };
            changes.push(ClassChange::AnchorsChanged {
                class_id: f.id.clone(),
                added, removed,
                cosine_centroid_shift: shift,
            });
        }
    }

    // 2. detect splits / merges / renames among only_from + only_to
    // Naive but deterministic : pair by cosine similarity of centroids.
    //
    // **Φ-D-OPS audit fix** : splits are detected BEFORE renames so
    // that a from-class with 2+ substring-matching to-classes is
    // classified as a split rather than greedily renamed to the first
    // match (= the pre-fix behaviour that hid the split signal).
    let mut paired_to: BTreeSet<&str> = BTreeSet::new();
    let mut paired_from: BTreeSet<&str> = BTreeSet::new();

    // 2a. splits : a from-class is "split" if 2+ to-classes have
    // cosine > 0.7 AND share substring of name.
    for id_from in &only_from {
        let f = from_index[id_from];
        let Some(f_cent) = &f.centroid else { continue; };
        let mut split_targets: Vec<&str> = Vec::new();
        for id_to in &only_to {
            if paired_to.contains(id_to) { continue; }
            let t = to_index[id_to];
            let Some(t_cent) = &t.centroid else { continue; };
            let sim = cosine_similarity(f_cent, t_cent);
            if sim > 0.70 && (id_to.starts_with(id_from) || id_to.contains(id_from) || id_from.contains(id_to)) {
                split_targets.push(id_to);
            }
        }
        if split_targets.len() >= 2 {
            changes.push(ClassChange::Split {
                from: f.id.clone(),
                to: split_targets.iter().map(|s| ClassId(s.to_string())).collect(),
            });
            paired_from.insert(id_from);
            for t in &split_targets { paired_to.insert(t); }
        }
    }

    // 2b. renames : ≥ 0.95 cosine similarity. Splits already claimed
    // their targets above, so we won't accidentally relabel one leg
    // of a split as a rename.
    for id_from in &only_from {
        if paired_from.contains(id_from) { continue; }
        let f = from_index[id_from];
        let Some(f_cent) = &f.centroid else { continue; };
        let mut best: Option<(&str, f32)> = None;
        for id_to in &only_to {
            if paired_to.contains(id_to) { continue; }
            let t = to_index[id_to];
            let Some(t_cent) = &t.centroid else { continue; };
            let sim = cosine_similarity(f_cent, t_cent);
            if best.map(|(_, s)| sim > s).unwrap_or(true) { best = Some((id_to, sim)); }
        }
        if let Some((id_to, sim)) = best {
            if sim > 0.95 {
                changes.push(ClassChange::Renamed {
                    old_id: f.id.clone(), new_id: ClassId(id_to.to_string()),
                });
                paired_from.insert(id_from); paired_to.insert(id_to);
            }
        }
    }

    // merges : 2+ from-classes consolidated into one to-class (two-pass too).
    let merge_candidates: Vec<&str> = only_to
        .iter()
        .filter(|id| !paired_to.contains(*id))
        .copied()
        .collect();
    for id_to in merge_candidates {
        let t = to_index[id_to];
        let Some(t_cent) = &t.centroid else { continue; };
        let mut merge_sources: Vec<&str> = Vec::new();
        let from_candidates: Vec<&str> = only_from
            .iter()
            .filter(|id| !paired_from.contains(*id))
            .copied()
            .collect();
        for id_from in from_candidates {
            let f = from_index[id_from];
            let Some(f_cent) = &f.centroid else { continue; };
            let sim = cosine_similarity(f_cent, t_cent);
            if sim > 0.70 && (id_from.starts_with(id_to) || id_from.contains(id_to) || id_to.contains(id_from)) {
                merge_sources.push(id_from);
            }
        }
        if merge_sources.len() >= 2 {
            changes.push(ClassChange::Merged {
                from: merge_sources.iter().map(|s| ClassId(s.to_string())).collect(),
                to: t.id.clone(),
            });
            for s in &merge_sources { paired_from.insert(s); }
            paired_to.insert(id_to);
        }
    }

    // Residual : plain Add / Remove
    for id_from in only_from.iter().filter(|id| !paired_from.contains(*id)) {
        changes.push(ClassChange::Removed { class_id: ClassId(id_from.to_string()) });
    }
    for id_to in only_to.iter().filter(|id| !paired_to.contains(*id)) {
        let t = to_index[id_to];
        changes.push(ClassChange::Added {
            class_id: ClassId(id_to.to_string()),
            anchors_count: t.anchors.len(),
        });
    }

    Ok(SemanticDiff {
        from_semver: from.semver.clone(),
        to_semver: to.semver.clone(),
        class_changes: changes,
        estimated_impact: None, // populated by `estimate_impact()` separately
    })
}

/// Cosine similarity between two equal-length vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() { return 0.0; }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na < f32::EPSILON || nb < f32::EPSILON { 0.0 } else { dot / (na * nb) }
}

/// Estimate impact of a diff against a gold dataset (re-classify each gold
/// example under the new taxonomy ; compare predicted vs expected).
pub fn estimate_impact(
    diff: &SemanticDiff,
    gold_examples: &[(String, ClassId)], // (text, expected_class)
    classify_fn: impl Fn(&str) -> Option<ClassId>,
) -> EstimatedImpact {
    let n = gold_examples.len() as u64;
    if n == 0 {
        return EstimatedImpact {
            affected_examples_count: 0,
            estimated_accuracy_delta_pp: 0.0,
            re_label_required: 0,
            confusion_matrix_delta_norm: 0.0,
        };
    }
    let mut correct = 0u64;
    let mut affected = 0u64;
    let mut re_label = 0u64;
    let affected_classes: BTreeSet<&ClassId> = diff.class_changes.iter()
        .flat_map(|c| match c {
            ClassChange::Added { class_id, .. } => vec![class_id],
            ClassChange::Removed { class_id } => vec![class_id],
            ClassChange::Renamed { old_id, new_id } => vec![old_id, new_id],
            ClassChange::Split { from, to } => {
                let mut v = vec![from]; v.extend(to.iter()); v
            }
            ClassChange::Merged { from, to } => {
                let mut v: Vec<&ClassId> = from.iter().collect(); v.push(to); v
            }
            ClassChange::AnchorsChanged { class_id, .. } => vec![class_id],
        })
        .collect();
    for (text, expected) in gold_examples {
        if affected_classes.contains(expected) { affected += 1; }
        let predicted = classify_fn(text);
        if predicted.as_ref() == Some(expected) { correct += 1; }
        else { re_label += 1; }
    }
    let accuracy = correct as f32 / n as f32;
    EstimatedImpact {
        affected_examples_count: affected,
        // simplified : assume baseline accuracy 0.85 ; delta vs new
        estimated_accuracy_delta_pp: (accuracy - 0.85) * 100.0,
        re_label_required: re_label,
        // crude norm : ratio of re-label to total
        confusion_matrix_delta_norm: re_label as f32 / n as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cls(id: &str, anchors: &[&str]) -> TaxonomyClass {
        TaxonomyClass {
            id: ClassId(id.into()),
            display_name: id.into(),
            anchors: anchors.iter().map(|t| Anchor { text: t.to_string(), embedding_centroid: None }).collect(),
            centroid: None,
        }
    }

    #[test]
    fn detect_added_class() {
        let from = TaxonomySnapshot { semver: "1.0.0".into(), classes: vec![cls("a", &["x"])] };
        let to = TaxonomySnapshot { semver: "1.1.0".into(), classes: vec![
            cls("a", &["x"]), cls("b", &["y", "z"]),
        ]};
        let d = compute_diff(&from, &to).unwrap();
        assert_eq!(d.class_changes.len(), 1);
        match &d.class_changes[0] {
            ClassChange::Added { class_id, anchors_count } => {
                assert_eq!(class_id.0, "b"); assert_eq!(*anchors_count, 2);
            }
            _ => panic!("expected Added, got {:?}", d.class_changes[0]),
        }
    }

    #[test]
    fn detect_removed_class() {
        let from = TaxonomySnapshot { semver: "1.0.0".into(), classes: vec![cls("a", &["x"]), cls("b", &["y"])] };
        let to = TaxonomySnapshot { semver: "1.1.0".into(), classes: vec![cls("a", &["x"])] };
        let d = compute_diff(&from, &to).unwrap();
        assert_eq!(d.class_changes.len(), 1);
        match &d.class_changes[0] {
            ClassChange::Removed { class_id } => assert_eq!(class_id.0, "b"),
            other => panic!("expected Removed, got {:?}", other),
        }
    }

    #[test]
    fn detect_anchors_changed() {
        let from = TaxonomySnapshot { semver: "1.0.0".into(), classes: vec![cls("a", &["x", "y"])] };
        let to = TaxonomySnapshot { semver: "1.1.0".into(), classes: vec![cls("a", &["x", "z"])] };
        let d = compute_diff(&from, &to).unwrap();
        assert_eq!(d.class_changes.len(), 1);
        match &d.class_changes[0] {
            ClassChange::AnchorsChanged { class_id, added, removed, .. } => {
                assert_eq!(class_id.0, "a");
                assert_eq!(added.len(), 1); assert_eq!(added[0], "z");
                assert_eq!(removed.len(), 1); assert_eq!(removed[0], "y");
            }
            other => panic!("expected AnchorsChanged, got {:?}", other),
        }
    }

    #[test]
    fn cosine_basic() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < 1e-6);
    }

    #[test]
    fn cosine_handles_zero_vector() {
        let a = vec![0.0, 0.0, 0.0]; let b = vec![1.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn detect_split_via_centroid_cosine() {
        let mut a = cls("issue", &["x"]); a.centroid = Some(vec![1.0, 0.0]);
        let mut a1 = cls("issue-bug", &["x"]); a1.centroid = Some(vec![0.95, 0.31]);
        let mut a2 = cls("issue-feature", &["y"]); a2.centroid = Some(vec![0.95, -0.31]);
        let from = TaxonomySnapshot { semver: "1.0.0".into(), classes: vec![a] };
        let to = TaxonomySnapshot { semver: "2.0.0".into(), classes: vec![a1, a2] };
        let d = compute_diff(&from, &to).unwrap();
        assert!(d.class_changes.iter().any(|c| matches!(c, ClassChange::Split { .. })),
                "expected a Split change, got {:?}", d.class_changes);
    }
}
