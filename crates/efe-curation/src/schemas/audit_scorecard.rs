//! Output schema for `efe_audit_bundle_vs_gold` (LLM-augmented portion).
//!
//! The structural scorecard (accuracy, F1, per-class metrics) is computed
//! by the bench harness directly. The LLM-augmented part is the
//! `weak_spots_explanation_markdown` (annotation of pre-computed weak spots).

use crate::CurationError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use super::ValidateOutput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditScorecardOutput {
    pub bundle_id: String,
    pub gold_dataset_id: String,
    pub accuracy: f32,
    pub macro_f1: f32,
    pub weighted_f1: f32,
    pub per_class: HashMap<String, ClassMetrics>,
    pub weak_spots: Vec<WeakSpot>,
    #[serde(default)]
    pub llm_explanation_markdown: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassMetrics {
    pub precision: f32,
    pub recall: f32,
    pub f1: f32,
    pub support: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WeakSpot {
    pub class_id: String,
    pub description: String,
    pub sample_sentences: Vec<String>,
    pub severity: f32,
}

impl ValidateOutput for AuditScorecardOutput {
    fn validate(&self) -> Result<(), CurationError> {
        if !(0.0..=1.0).contains(&self.accuracy) {
            return Err(CurationError::LlmSchemaInvalid(format!(
                "accuracy {} out of [0,1]", self.accuracy)));
        }
        if !(0.0..=1.0).contains(&self.macro_f1) {
            return Err(CurationError::LlmSchemaInvalid(format!(
                "macro_f1 {} out of [0,1]", self.macro_f1)));
        }
        if !(0.0..=1.0).contains(&self.weighted_f1) {
            return Err(CurationError::LlmSchemaInvalid(format!(
                "weighted_f1 {} out of [0,1]", self.weighted_f1)));
        }
        for (cid, m) in &self.per_class {
            for (name, val) in [("precision", m.precision), ("recall", m.recall), ("f1", m.f1)] {
                if !(0.0..=1.0).contains(&val) {
                    return Err(CurationError::LlmSchemaInvalid(format!(
                        "class '{}' {} {} out of [0,1]", cid, name, val)));
                }
            }
        }
        for ws in &self.weak_spots {
            if !(0.0..=1.0).contains(&ws.severity) {
                return Err(CurationError::LlmSchemaInvalid(format!(
                    "weak_spot severity {} out of [0,1]", ws.severity)));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::parse_validated;

    #[test]
    fn parses_valid_scorecard() {
        let json = r#"{
            "bundle_id": "b1", "gold_dataset_id": "g1",
            "accuracy": 0.87, "macro_f1": 0.85, "weighted_f1": 0.88,
            "per_class": {
                "support": {"precision": 0.9, "recall": 0.85, "f1": 0.875, "support": 100}
            },
            "weak_spots": [
                {"class_id":"support","description":"...","sample_sentences":["a","b"],"severity":0.7}
            ]
        }"#;
        let _: AuditScorecardOutput = parse_validated(json).unwrap();
    }

    #[test]
    fn rejects_oob_accuracy() {
        let json = r#"{
            "bundle_id":"b","gold_dataset_id":"g",
            "accuracy":1.5,"macro_f1":0.5,"weighted_f1":0.5,
            "per_class":{},"weak_spots":[]
        }"#;
        let r: Result<AuditScorecardOutput, _> = parse_validated(json);
        assert!(r.is_err());
    }
}
