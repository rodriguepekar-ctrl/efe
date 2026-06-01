//! `xml_validate` — validate XML well-formedness.
//!
//! Mode A — pure Rust.  TIER-1 from Day 1.
//!
//! Note : full XSD validation requires schema processing which is
//! out of scope for Phase 1.  This agent validates **well-formedness**
//! (Phase 2 may add XSD via libxml2 binding).
//!
//! Input :
//! ```json
//! { "xml": "<root>...</root>" }
//! ```
//!
//! Output :
//! ```json
//! {
//!   "well_formed": true | false,
//!   "errors": ["..."],
//!   "root_element": "root"   // null if not well-formed
//! }
//! ```

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use serde_json::json;

/// Returns the [`NanoAgent`] catalog manifest for `xml_validate`.
pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("xml_validate").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Validate XML well-formedness and extract root element.".to_string(),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "xml_validate::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["xml"],
            "properties": {
                "xml": { "type": "string" }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["well_formed", "errors"],
            "properties": {
                "well_formed": { "type": "boolean" },
                "errors": { "type": "array", "items": { "type": "string" } },
                "root_element": { "type": ["string", "null"] }
            }
        }),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.01,
        latency_p99_ms_target: 80,
        audit: AuditPolicy::default(),
        compliance: ComplianceTags::default(),
        mcp_tool_metadata: None,
        signature: None,
        co_publishing: None,
        gold_set_ref: None,
        created_at: Utc::now(),
    }
}

/// Returns the Mode A handler closure.
pub fn handler(
) -> impl Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync + 'static {
    |input: serde_json::Value| -> Result<serde_json::Value, String> {
        let xml = input
            .get("xml")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing or non-string field 'xml'".to_string())?;

        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut errors: Vec<String> = Vec::new();
        let mut root_element: Option<String> = None;
        let mut buf = Vec::new();
        let mut depth: i32 = 0;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    if root_element.is_none() {
                        let name_bytes = e.name().0.to_vec();
                        let name = String::from_utf8(name_bytes)
                            .map_err(|err| format!("invalid UTF-8 in element name : {err}"))?;
                        root_element = Some(name);
                    }
                    depth += 1;
                }
                Ok(Event::End(_)) => {
                    depth -= 1;
                }
                Ok(Event::Empty(e)) => {
                    if root_element.is_none() {
                        let name_bytes = e.name().0.to_vec();
                        let name = String::from_utf8(name_bytes).unwrap_or_default();
                        root_element = Some(name);
                    }
                }
                Ok(Event::Eof) => break,
                Ok(_) => {}
                Err(e) => {
                    errors.push(format!("parse error at position {}: {}", reader.buffer_position(), e));
                    break;
                }
            }
            buf.clear();
        }

        if depth != 0 {
            errors.push(format!("unbalanced tags : depth ended at {depth}"));
        }
        if root_element.is_none() && errors.is_empty() {
            errors.push("no root element".to_string());
        }

        let well_formed = errors.is_empty();
        Ok(json!({
            "well_formed": well_formed,
            "errors": errors,
            "root_element": root_element,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_valid() {
        let m = manifest();
        assert_eq!(m.id.as_str(), "xml_validate");
        assert_eq!(m.stability_tier, StabilityTier::Tier1);
    }

    #[test]
    fn well_formed_xml_passes() {
        let h = handler();
        let out = h(json!({"xml": "<root><a>1</a><b>2</b></root>"})).unwrap();
        assert_eq!(out["well_formed"], true);
        assert_eq!(out["root_element"], "root");
        assert!(out["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn unbalanced_tags_rejected() {
        let h = handler();
        let out = h(json!({"xml": "<root><a></b></root>"})).unwrap();
        assert_eq!(out["well_formed"], false);
        assert!(!out["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn empty_root_element_works() {
        let h = handler();
        let out = h(json!({"xml": "<root/>"})).unwrap();
        assert_eq!(out["well_formed"], true);
        assert_eq!(out["root_element"], "root");
    }

    #[test]
    fn missing_xml_field_errors() {
        let h = handler();
        let err = h(json!({})).unwrap_err();
        assert!(err.contains("xml"));
    }

    #[test]
    fn malformed_xml_rejected() {
        let h = handler();
        let out = h(json!({"xml": "<root><<<"})).unwrap();
        assert_eq!(out["well_formed"], false);
    }
}
