//! Output schema for `efe_suggest_class_merge`.

use crate::{is_valid_kebab_case, CurationError};
use serde::{Deserialize, Serialize};
use super::ValidateOutput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassMergeOutput {
    pub merge_candidate: MergeCandidate,
    pub rationale_markdown: String,
    pub estimated_accuracy_impact_pp: f32,
    pub estimated_examples_re_labeled: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeCandidate {
    pub merged_class_id: String,
    pub merged_display_name: String,
    pub merged_anchors: Vec<MergedAnchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedAnchor {
    pub text: String,
    #[serde(default)]
    pub source_class_id: Option<String>,
}

impl ValidateOutput for ClassMergeOutput {
    fn validate(&self) -> Result<(), CurationError> {
        if !is_valid_kebab_case(&self.merge_candidate.merged_class_id) {
            return Err(CurationError::LlmSchemaInvalid(format!(
                "invalid merged_class_id : '{}'", self.merge_candidate.merged_class_id
            )));
        }
        if self.merge_candidate.merged_anchors.len() < 5 {
            return Err(CurationError::LlmSchemaInvalid(
                "merged class must have ≥ 5 anchors".into()));
        }
        if self.rationale_markdown.trim().is_empty() {
            return Err(CurationError::LlmSchemaInvalid("empty rationale".into()));
        }
        // accuracy delta should be in plausible range [-20pp, +5pp] for merges
        if self.estimated_accuracy_impact_pp < -20.0 || self.estimated_accuracy_impact_pp > 5.0 {
            return Err(CurationError::LlmSchemaInvalid(format!(
                "accuracy impact {} pp implausible for merge", self.estimated_accuracy_impact_pp
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::parse_validated;

    fn valid() -> &'static str { r#"{
        "merge_candidate": {
            "merged_class_id": "shipping-issue",
            "merged_display_name": "Shipping issue",
            "merged_anchors": [
                {"text":"a"},{"text":"b"},{"text":"c"},{"text":"d"},{"text":"e"}
            ]
        },
        "rationale_markdown": "Two classes are 95% similar",
        "estimated_accuracy_impact_pp": -1.5,
        "estimated_examples_re_labeled": 423
    }"# }

    #[test]
    fn parses_valid_merge() {
        let _: ClassMergeOutput = parse_validated(valid()).unwrap();
    }

    #[test]
    fn rejects_too_few_anchors() {
        let bad = r#"{
            "merge_candidate": {"merged_class_id":"x-y","merged_display_name":"X","merged_anchors":[{"text":"a"}]},
            "rationale_markdown":"x", "estimated_accuracy_impact_pp": 0, "estimated_examples_re_labeled": 0
        }"#;
        let r: Result<ClassMergeOutput, _> = parse_validated(bad);
        assert!(r.is_err());
    }
}
