//! Output schema for `efe_propose_taxonomy_split`.

use crate::{is_valid_kebab_case, CurationError};
use serde::{Deserialize, Serialize};
use super::ValidateOutput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxonomySplitOutput {
    pub proposed_classes: Vec<ProposedSubclass>,
    pub rationale_markdown: String,
    pub confidence_score: f32,
    #[serde(default)]
    pub sample_assignments: Vec<SampleAssignment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedSubclass {
    pub proposed_class_id: String,
    pub display_name: String,
    pub anchor_candidates: Vec<AnchorCandidate>,
    #[serde(default)]
    pub description_markdown: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorCandidate {
    pub text: String,
    #[serde(default)]
    pub estimated_centroid_shift: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleAssignment {
    pub sample_id: String,
    pub assigned_to: String,
}

impl ValidateOutput for TaxonomySplitOutput {
    fn validate(&self) -> Result<(), CurationError> {
        if self.proposed_classes.len() < 2 || self.proposed_classes.len() > 5 {
            return Err(CurationError::LlmSchemaInvalid(format!(
                "proposed_classes count {} out of range [2,5]", self.proposed_classes.len()
            )));
        }
        if !(0.0..=1.0).contains(&self.confidence_score) {
            return Err(CurationError::LlmSchemaInvalid(format!(
                "confidence_score {} out of range [0,1]", self.confidence_score
            )));
        }
        if self.rationale_markdown.is_empty() {
            return Err(CurationError::LlmSchemaInvalid("empty rationale".into()));
        }
        let mut seen_ids = std::collections::HashSet::new();
        for sub in &self.proposed_classes {
            if !is_valid_kebab_case(&sub.proposed_class_id) {
                return Err(CurationError::LlmSchemaInvalid(format!(
                    "invalid class_id : '{}'", sub.proposed_class_id
                )));
            }
            if !seen_ids.insert(sub.proposed_class_id.clone()) {
                return Err(CurationError::LlmSchemaInvalid(format!(
                    "duplicate class_id : '{}'", sub.proposed_class_id
                )));
            }
            if sub.anchor_candidates.len() < 3 {
                return Err(CurationError::LlmSchemaInvalid(format!(
                    "class '{}' must have ≥ 3 anchor candidates (got {})",
                    sub.proposed_class_id, sub.anchor_candidates.len()
                )));
            }
            for anchor in &sub.anchor_candidates {
                if anchor.text.trim().is_empty() {
                    return Err(CurationError::LlmSchemaInvalid("anchor text empty".into()));
                }
                if anchor.text.len() > 1024 {
                    return Err(CurationError::LlmSchemaInvalid("anchor text > 1024 chars".into()));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::parse_validated;

    fn valid_json() -> &'static str { r#"{
        "proposed_classes": [
            {
                "proposed_class_id": "shipping-delay",
                "display_name": "Shipping delay",
                "anchor_candidates": [
                    {"text": "my package is late"},
                    {"text": "where is my order"},
                    {"text": "delivery delayed"}
                ],
                "description_markdown": "Issues about shipping timing"
            },
            {
                "proposed_class_id": "missing-package",
                "display_name": "Missing package",
                "anchor_candidates": [
                    {"text": "I never received my order"},
                    {"text": "package marked delivered but not arrived"},
                    {"text": "lost shipment"}
                ]
            }
        ],
        "rationale_markdown": "Splitting the catch-all class into shipping vs missing.",
        "confidence_score": 0.84,
        "sample_assignments": []
    }"# }

    #[test]
    fn parses_valid_json() {
        let out: TaxonomySplitOutput = parse_validated(valid_json()).unwrap();
        assert_eq!(out.proposed_classes.len(), 2);
    }

    #[test]
    fn rejects_too_few_classes() {
        let bad = r#"{
            "proposed_classes": [{"proposed_class_id":"a","display_name":"a","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]}],
            "rationale_markdown": "x", "confidence_score": 0.5
        }"#;
        let r: Result<TaxonomySplitOutput, _> = parse_validated(bad);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_invalid_class_id() {
        let bad = r#"{
            "proposed_classes": [
                {"proposed_class_id":"BadID","display_name":"a","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]},
                {"proposed_class_id":"ok","display_name":"b","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]}
            ],
            "rationale_markdown": "x", "confidence_score": 0.5
        }"#;
        let r: Result<TaxonomySplitOutput, _> = parse_validated(bad);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_too_few_anchors() {
        let bad = r#"{
            "proposed_classes": [
                {"proposed_class_id":"a-b","display_name":"a","anchor_candidates":[{"text":"x"},{"text":"y"}]},
                {"proposed_class_id":"c-d","display_name":"b","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]}
            ],
            "rationale_markdown": "x", "confidence_score": 0.5
        }"#;
        let r: Result<TaxonomySplitOutput, _> = parse_validated(bad);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_confidence_out_of_range() {
        let bad = r#"{
            "proposed_classes": [
                {"proposed_class_id":"a-b","display_name":"a","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]},
                {"proposed_class_id":"c-d","display_name":"b","anchor_candidates":[{"text":"x"},{"text":"y"},{"text":"z"}]}
            ],
            "rationale_markdown": "x", "confidence_score": 1.5
        }"#;
        let r: Result<TaxonomySplitOutput, _> = parse_validated(bad);
        assert!(r.is_err());
    }
}
