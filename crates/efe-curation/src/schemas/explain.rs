//! Output schema for `efe_explain_misclassification`.

use crate::CurationError;
use serde::{Deserialize, Serialize};
use super::ValidateOutput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainOutput {
    pub root_cause_markdown: String,
    pub suggested_fixes: Vec<SuggestedFix>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SuggestedFix {
    AddAnchor {
        target_class_id: String,
        anchor_text: String,
        rationale: String,
    },
    RemoveAmbiguousAnchor {
        from_class_id: String,
        anchor_text: String,
        rationale: String,
    },
    SplitClass {
        target_class_id: String,
        rationale: String,
    },
    GenerateExamples {
        target_class_id: String,
        n_examples: u32,
        rationale: String,
    },
}

impl ValidateOutput for ExplainOutput {
    fn validate(&self) -> Result<(), CurationError> {
        if self.root_cause_markdown.trim().is_empty() {
            return Err(CurationError::LlmSchemaInvalid("empty root_cause".into()));
        }
        if self.suggested_fixes.is_empty() {
            return Err(CurationError::LlmSchemaInvalid("no suggested_fixes".into()));
        }
        if self.suggested_fixes.len() > 10 {
            return Err(CurationError::LlmSchemaInvalid("too many fixes (>10)".into()));
        }
        for fix in &self.suggested_fixes {
            match fix {
                SuggestedFix::GenerateExamples { n_examples, .. } => {
                    if !(5..=200).contains(n_examples) {
                        return Err(CurationError::LlmSchemaInvalid(
                            "n_examples must be in [5,200]".into()));
                    }
                }
                _ => {}
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
    fn parses_valid_explain() {
        let json = r#"{
            "root_cause_markdown": "Anchor 'crash' belongs to both classes",
            "suggested_fixes": [
                {"type": "REMOVE_AMBIGUOUS_ANCHOR", "from_class_id": "bug", "anchor_text": "crash", "rationale": "..."}
            ]
        }"#;
        let out: ExplainOutput = parse_validated(json).unwrap();
        assert_eq!(out.suggested_fixes.len(), 1);
    }

    #[test]
    fn rejects_empty_fixes() {
        let json = r#"{"root_cause_markdown": "x", "suggested_fixes": []}"#;
        let r: Result<ExplainOutput, _> = parse_validated(json);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_invalid_n_examples() {
        let json = r#"{
            "root_cause_markdown": "x",
            "suggested_fixes": [
                {"type": "GENERATE_EXAMPLES", "target_class_id": "a", "n_examples": 1000, "rationale": "x"}
            ]
        }"#;
        let r: Result<ExplainOutput, _> = parse_validated(json);
        assert!(r.is_err());
    }
}
