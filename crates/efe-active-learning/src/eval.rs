//! Two-stage eval engine : offline holdout + canary (M23 reused).
//!
//! See § 15 of EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md.

use crate::{ActiveLearningResult, holdout::TrainingExample};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassMetrics {
    pub precision: f32,
    pub recall: f32,
    pub f1: f32,
    pub support: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoldoutEvalResult {
    pub holdout_size: u64,
    pub accuracy: f32,
    pub macro_f1: f32,
    pub weighted_f1: f32,
    pub per_class: HashMap<String, ClassMetrics>,
    pub computed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalDelta {
    pub accuracy_old: f32,
    pub accuracy_new: f32,
    pub delta_pp: f32,
    pub macro_f1_old: f32,
    pub macro_f1_new: f32,
    pub macro_f1_delta_pp: f32,
    pub per_class_old: HashMap<String, ClassMetrics>,
    pub per_class_new: HashMap<String, ClassMetrics>,
}

/// Trait : a classifier that can predict a class given a sentence and
/// a centroid table (Mode-1 cosine sim).
pub trait Classifier: Send + Sync {
    fn classify(&self, text: &str) -> Option<String>;
}

/// Eval a classifier on a holdout : compute accuracy + per-class P/R/F1.
pub fn eval_on_holdout<C: Classifier>(
    classifier: &C, holdout: &[TrainingExample],
) -> HoldoutEvalResult {
    if holdout.is_empty() {
        return HoldoutEvalResult {
            holdout_size: 0, accuracy: 0.0, macro_f1: 0.0, weighted_f1: 0.0,
            per_class: HashMap::new(), computed_at: Utc::now(),
        };
    }

    // Per-class TP/FP/FN counters
    let mut tp: HashMap<String, u32> = HashMap::new();
    let mut fp: HashMap<String, u32> = HashMap::new();
    let mut fn_: HashMap<String, u32> = HashMap::new();
    let mut support: HashMap<String, u32> = HashMap::new();

    let mut correct = 0u32;
    for ex in holdout {
        *support.entry(ex.class_id.clone()).or_insert(0) += 1;
        let predicted = classifier.classify(&ex.text);
        match predicted {
            Some(p) if p == ex.class_id => {
                *tp.entry(ex.class_id.clone()).or_insert(0) += 1;
                correct += 1;
            }
            Some(p) => {
                *fp.entry(p).or_insert(0) += 1;
                *fn_.entry(ex.class_id.clone()).or_insert(0) += 1;
            }
            None => {
                *fn_.entry(ex.class_id.clone()).or_insert(0) += 1;
            }
        }
    }

    let mut per_class = HashMap::new();
    let mut classes: std::collections::HashSet<String> = HashSet::new();
    classes.extend(tp.keys().cloned());
    classes.extend(fp.keys().cloned());
    classes.extend(fn_.keys().cloned());
    classes.extend(support.keys().cloned());

    let mut macro_f1_sum = 0.0_f32;
    let mut weighted_f1_sum = 0.0_f32;
    let total_support = holdout.len() as f32;
    for class_id in &classes {
        let t = *tp.get(class_id).unwrap_or(&0) as f32;
        let f = *fp.get(class_id).unwrap_or(&0) as f32;
        let n = *fn_.get(class_id).unwrap_or(&0) as f32;
        let s = *support.get(class_id).unwrap_or(&0);
        let precision = if t + f > 0.0 { t / (t + f) } else { 0.0 };
        let recall = if t + n > 0.0 { t / (t + n) } else { 0.0 };
        let f1 = if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else { 0.0 };
        per_class.insert(class_id.clone(), ClassMetrics {
            precision, recall, f1, support: s,
        });
        macro_f1_sum += f1;
        weighted_f1_sum += f1 * (s as f32) / total_support;
    }
    let macro_f1 = if classes.is_empty() { 0.0 }
                    else { macro_f1_sum / (classes.len() as f32) };

    HoldoutEvalResult {
        holdout_size: holdout.len() as u64,
        accuracy: correct as f32 / total_support,
        macro_f1, weighted_f1: weighted_f1_sum,
        per_class, computed_at: Utc::now(),
    }
}

use std::collections::HashSet;

/// Compute the delta between old and new eval results.
pub fn compute_delta(old: HoldoutEvalResult, new: HoldoutEvalResult) -> EvalDelta {
    EvalDelta {
        accuracy_old: old.accuracy,
        accuracy_new: new.accuracy,
        delta_pp: (new.accuracy - old.accuracy) * 100.0,
        macro_f1_old: old.macro_f1,
        macro_f1_new: new.macro_f1,
        macro_f1_delta_pp: (new.macro_f1 - old.macro_f1) * 100.0,
        per_class_old: old.per_class,
        per_class_new: new.per_class,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PerfectClassifier;
    impl Classifier for PerfectClassifier {
        fn classify(&self, text: &str) -> Option<String> {
            // Trivial : take first word as class
            text.split_whitespace().next().map(|s| s.to_string())
        }
    }

    fn ex(text: &str, class: &str) -> TrainingExample {
        TrainingExample {
            id: uuid::Uuid::new_v4(),
            text: text.into(), class_id: class.into(),
        }
    }

    #[test]
    fn empty_holdout_returns_zero() {
        let r = eval_on_holdout(&PerfectClassifier, &[]);
        assert_eq!(r.holdout_size, 0);
        assert_eq!(r.accuracy, 0.0);
    }

    #[test]
    fn perfect_classifier_returns_100pct_accuracy() {
        let h = vec![
            ex("a foo", "a"), ex("a bar", "a"), ex("b baz", "b"),
        ];
        let r = eval_on_holdout(&PerfectClassifier, &h);
        assert!((r.accuracy - 1.0).abs() < 1e-6);
        assert_eq!(r.holdout_size, 3);
    }

    #[test]
    fn delta_computed_correctly() {
        let old = HoldoutEvalResult {
            holdout_size: 100, accuracy: 0.80, macro_f1: 0.78, weighted_f1: 0.79,
            per_class: HashMap::new(), computed_at: Utc::now(),
        };
        let new = HoldoutEvalResult {
            holdout_size: 100, accuracy: 0.83, macro_f1: 0.81, weighted_f1: 0.82,
            per_class: HashMap::new(), computed_at: Utc::now(),
        };
        let d = compute_delta(old, new);
        assert!((d.delta_pp - 3.0).abs() < 0.001);
        assert!((d.macro_f1_delta_pp - 3.0).abs() < 0.001);
    }
}
