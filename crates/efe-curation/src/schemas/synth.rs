//! Output schema for `efe_generate_synthetic_examples`.

use crate::CurationError;
use serde::{Deserialize, Serialize};
use super::ValidateOutput;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticExamplesOutput {
    pub examples: Vec<SyntheticExample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticExample {
    pub text: String,
    pub suggested_class_id: String,
    #[serde(default)]
    pub estimated_difficulty: f32,
    #[serde(default)]
    pub seed_anchor_id: Option<String>,
}

impl ValidateOutput for SyntheticExamplesOutput {
    fn validate(&self) -> Result<(), CurationError> {
        if self.examples.is_empty() {
            return Err(CurationError::LlmSchemaInvalid("no examples generated".into()));
        }
        if self.examples.len() > 200 {
            return Err(CurationError::LlmSchemaInvalid("too many examples (>200)".into()));
        }
        for ex in &self.examples {
            if ex.text.trim().is_empty() {
                return Err(CurationError::LlmSchemaInvalid("empty example text".into()));
            }
            if ex.text.len() > 4096 {
                return Err(CurationError::LlmSchemaInvalid("example text > 4096 chars".into()));
            }
            if !(0.0..=1.0).contains(&ex.estimated_difficulty) {
                return Err(CurationError::LlmSchemaInvalid(
                    format!("difficulty {} out of [0,1]", ex.estimated_difficulty)));
            }
        }
        // diversity sanity : must have ≥ 70% unique texts
        let unique: std::collections::HashSet<&str> =
            self.examples.iter().map(|e| e.text.as_str()).collect();
        let unique_ratio = unique.len() as f32 / self.examples.len() as f32;
        if unique_ratio < 0.70 {
            return Err(CurationError::LlmSchemaInvalid(
                format!("examples too repetitive (uniq ratio {:.2})", unique_ratio)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::parse_validated;

    #[test]
    fn parses_valid_synth() {
        let json = r#"{"examples":[
            {"text":"my package never arrived","suggested_class_id":"missing"},
            {"text":"where is my order","suggested_class_id":"missing"},
            {"text":"delivery delayed","suggested_class_id":"delay"}
        ]}"#;
        let out: SyntheticExamplesOutput = parse_validated(json).unwrap();
        assert_eq!(out.examples.len(), 3);
    }

    #[test]
    fn rejects_empty_examples() {
        let json = r#"{"examples": []}"#;
        let r: Result<SyntheticExamplesOutput, _> = parse_validated(json);
        assert!(r.is_err());
    }

    #[test]
    fn rejects_low_diversity() {
        let json = r#"{"examples":[
            {"text":"same","suggested_class_id":"a"},
            {"text":"same","suggested_class_id":"a"},
            {"text":"same","suggested_class_id":"a"},
            {"text":"different","suggested_class_id":"a"}
        ]}"#;
        let r: Result<SyntheticExamplesOutput, _> = parse_validated(json);
        assert!(r.is_err());
    }
}
