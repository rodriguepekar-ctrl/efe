//! LLM output schema validators for the 5 curation tools.
//!
//! Each tool's LLM output is parsed as JSON then validated against the
//! corresponding Rust struct. Invalid output triggers a retry (up to 2x)
//! and ultimately surfaces as `CurationError::LlmSchemaInvalid`.

pub mod split;
pub mod explain;
pub mod synth;
pub mod merge;
pub mod audit_scorecard;

pub use split::TaxonomySplitOutput;
pub use explain::ExplainOutput;
pub use synth::SyntheticExamplesOutput;
pub use merge::ClassMergeOutput;
pub use audit_scorecard::AuditScorecardOutput;

use crate::CurationError;

/// Parse + validate LLM output JSON. Common entry point for all schemas.
pub fn parse_validated<T: serde::de::DeserializeOwned + ValidateOutput>(
    raw: &str,
) -> Result<T, CurationError> {
    let parsed: T = serde_json::from_str(raw)
        .map_err(|e| CurationError::LlmSchemaInvalid(format!("JSON parse : {}", e)))?;
    parsed.validate()?;
    Ok(parsed)
}

/// Trait : a schema struct can validate its own invariants beyond what
/// serde catches.
pub trait ValidateOutput {
    fn validate(&self) -> Result<(), CurationError>;
}
