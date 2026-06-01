//! `csv_normalize` — parse CSV, return headers + rows.
//!
//! Mode A — pure Rust (csv crate).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

const MAX_ROWS: usize = 100_000;       // safety cap

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("csv_normalize").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Parse a CSV string and return its headers + rows as structured data."
            .to_string(),
        author: Author::Efe,
        category: Category::Transformation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "csv_normalize::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["csv"],
            "properties": {
                "csv": { "type": "string" },
                "delimiter": { "type": "string", "maxLength": 1, "default": "," },
                "has_header": { "type": "boolean", "default": true }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["headers", "rows", "row_count"],
            "properties": {
                "headers": { "type": "array", "items": { "type": "string" } },
                "rows": {
                    "type": "array",
                    "items": { "type": "array", "items": { "type": "string" } }
                },
                "row_count": { "type": "integer", "minimum": 0 },
                "truncated": { "type": "boolean" }
            }
        }),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.005,
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

pub fn handler(
) -> impl Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync + 'static {
    |input: serde_json::Value| -> Result<serde_json::Value, String> {
        let csv_text = input
            .get("csv")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'csv'".to_string())?;
        let delimiter_str = input
            .get("delimiter")
            .and_then(|v| v.as_str())
            .unwrap_or(",");
        let delimiter = delimiter_str
            .as_bytes()
            .first()
            .copied()
            .unwrap_or(b',');
        let has_header = input
            .get("has_header")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(has_header)
            .flexible(true)
            .from_reader(csv_text.as_bytes());

        let headers: Vec<String> = if has_header {
            rdr.headers()
                .map_err(|e| format!("read headers : {e}"))?
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        };

        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut truncated = false;
        for result in rdr.records() {
            if rows.len() >= MAX_ROWS {
                truncated = true;
                break;
            }
            let record = result.map_err(|e| format!("read record : {e}"))?;
            rows.push(record.iter().map(|s| s.to_string()).collect());
        }

        let row_count = rows.len();
        Ok(json!({
            "headers": headers,
            "rows": rows,
            "row_count": row_count,
            "truncated": truncated,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_transformation_category() {
        assert_eq!(manifest().category, Category::Transformation);
    }

    #[test]
    fn parses_basic_csv() {
        let h = handler();
        let csv = "name,age\nAlice,30\nBob,25";
        let out = h(json!({"csv": csv})).unwrap();
        assert_eq!(out["headers"][0], "name");
        assert_eq!(out["headers"][1], "age");
        assert_eq!(out["row_count"], 2);
        assert_eq!(out["rows"][0][0], "Alice");
        assert_eq!(out["rows"][0][1], "30");
    }

    #[test]
    fn parses_no_header() {
        let h = handler();
        let csv = "Alice,30\nBob,25";
        let out = h(json!({"csv": csv, "has_header": false})).unwrap();
        assert_eq!(out["row_count"], 2);
        let headers = out["headers"].as_array().unwrap();
        assert_eq!(headers.len(), 0);
    }

    #[test]
    fn parses_custom_delimiter() {
        let h = handler();
        let csv = "name;age\nAlice;30";
        let out = h(json!({"csv": csv, "delimiter": ";"})).unwrap();
        assert_eq!(out["headers"][0], "name");
        assert_eq!(out["rows"][0][1], "30");
    }

    #[test]
    fn handles_quoted_fields() {
        let h = handler();
        let csv = "name,note\nAlice,\"hello, world\"";
        let out = h(json!({"csv": csv})).unwrap();
        assert_eq!(out["rows"][0][1], "hello, world");
    }

    #[test]
    fn empty_csv_returns_zero() {
        let h = handler();
        let out = h(json!({"csv": ""})).unwrap();
        assert_eq!(out["row_count"], 0);
    }
}
