//! `json_validate` — validate a JSON value against a JSON Schema (draft 2020-12).
//!
//! Mode A — pure Rust.  TIER-1 from Day 1.
//!
//! Input :
//! ```json
//! {
//!   "schema": <json-schema>,
//!   "value":  <any>
//! }
//! ```
//!
//! Output :
//! ```json
//! {
//!   "valid": true | false,
//!   "errors": ["..."]
//! }
//! ```

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use jsonschema::{Draft, JSONSchema};
use serde_json::json;

/// Returns the [`NanoAgent`] catalog manifest for `json_validate`.
pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("json_validate").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Validate a JSON value against a JSON Schema (draft 2020-12).".to_string(),
        author: Author::Efe,
        category: Category::Validation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "json_validate::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["schema", "value"],
            "properties": {
                "schema": {
                    "type": "object",
                    "description": "JSON Schema to validate against."
                },
                "value": {
                    "description": "Any JSON value to validate."
                }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["valid", "errors"],
            "properties": {
                "valid": { "type": "boolean" },
                "errors": {
                    "type": "array",
                    "items": { "type": "string" }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.01,
        latency_p99_ms_target: 50,
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
        let schema_val = input
            .get("schema")
            .ok_or_else(|| "missing required field 'schema'".to_string())?;
        let value = input
            .get("value")
            .ok_or_else(|| "missing required field 'value'".to_string())?;

        // Compile the schema (caller's schema, not our input schema).
        let compiled = match JSONSchema::options()
            .with_draft(Draft::Draft202012)
            .compile(schema_val)
        {
            Ok(c) => c,
            Err(e) => {
                return Ok(json!({
                    "valid": false,
                    "errors": [format!("schema compile error : {e}")]
                }))
            }
        };

        // Validate the value.
        let errors: Vec<String> = match compiled.validate(value) {
            Ok(()) => Vec::new(),
            Err(errs) => errs.map(|e| e.to_string()).collect(),
        };
        let valid = errors.is_empty();

        Ok(json!({
            "valid": valid,
            "errors": errors,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::stub::InMemoryAuditWriter;
    use crate::audit::AuditWriter;
    use crate::runtime::context::CallContext;
    use crate::runtime::pure_rust::{HandlerRegistry, PureRustRuntime};
    use efe_nano_sdk::TenantId;
    use std::sync::Arc;

    #[test]
    fn manifest_is_valid() {
        let m = manifest();
        assert_eq!(m.id.as_str(), "json_validate");
        assert_eq!(m.version.as_str(), "1.0.0");
        assert!(m.is_efe_shipped());
        assert_eq!(m.stability_tier, StabilityTier::Tier1);
    }

    #[test]
    fn handler_returns_valid_for_matching_value() {
        let h = handler();
        let input = json!({
            "schema": {"type": "string"},
            "value": "hello"
        });
        let out = h(input).unwrap();
        assert_eq!(out["valid"], true);
        assert_eq!(out["errors"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn handler_returns_invalid_with_errors() {
        let h = handler();
        let input = json!({
            "schema": {
                "type": "object",
                "required": ["x"],
                "properties": {"x": {"type": "integer"}}
            },
            "value": {}
        });
        let out = h(input).unwrap();
        assert_eq!(out["valid"], false);
        assert!(!out["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn handler_rejects_missing_schema_field() {
        let h = handler();
        let input = json!({"value": "x"});
        let err = h(input).unwrap_err();
        assert!(err.contains("schema"));
    }

    #[test]
    fn handler_rejects_missing_value_field() {
        let h = handler();
        let input = json!({"schema": {"type": "string"}});
        let err = h(input).unwrap_err();
        assert!(err.contains("value"));
    }

    #[test]
    fn handler_reports_bad_schema() {
        let h = handler();
        let input = json!({
            "schema": {"type": "this_is_not_a_valid_type"},
            "value": "x"
        });
        let out = h(input).unwrap();
        // Schema compilation failed → returns valid=false with error
        assert_eq!(out["valid"], false);
    }

    #[tokio::test]
    async fn integration_via_runtime() {
        let reg = HandlerRegistry::new();
        reg.register("json_validate::run", handler()).await;
        let runtime = PureRustRuntime::new(reg);
        let agent = manifest();
        let audit: Arc<dyn AuditWriter> = Arc::new(InMemoryAuditWriter::new());
        let ctx = CallContext::stand_alone(TenantId::system(), audit, 5_000);
        let input = json!({
            "schema": {"type": "integer"},
            "value": 42
        });
        let resp = runtime.execute_with_context(&agent, input, &ctx).await.unwrap();
        assert_eq!(resp.output["valid"], true);
    }
}
