//! `date_normalize` — parse a date string, emit ISO 8601 + timestamp.
//!
//! Mode A — pure Rust (chrono only).  TIER-1.
//!
//! Phase 1 supports the most common formats; Phase 2 may add `dateparser` crate
//! for broader format support.

use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("date_normalize").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Parse a date string and emit ISO 8601 + UNIX timestamp.".to_string(),
        author: Author::Efe,
        category: Category::Transformation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "date_normalize::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["input"],
            "properties": {
                "input": { "type": "string", "maxLength": 100 },
                "input_format": {
                    "type": ["string", "null"],
                    "description": "Optional strftime format hint (e.g. '%Y-%m-%d')"
                }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["iso8601", "timestamp_ms"],
            "properties": {
                "iso8601": { "type": "string" },
                "timestamp_ms": { "type": "integer" },
                "tz_offset": { "type": "string" }
            }
        }),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.001,
        latency_p99_ms_target: 10,
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
        let s = input
            .get("input")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'input'".to_string())?;

        // If explicit format provided, try it first.
        if let Some(fmt) = input.get("input_format").and_then(|v| v.as_str()) {
            if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
                return Ok(format_response(dt.with_timezone(&Utc)));
            }
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
                let dt = Utc.from_utc_datetime(&ndt);
                return Ok(format_response(dt));
            }
            if let Ok(nd) = NaiveDate::parse_from_str(s, fmt) {
                let dt = Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap());
                return Ok(format_response(dt));
            }
        }

        // Try a sequence of common formats (heuristic).
        // 1. ISO 8601 / RFC 3339 with timezone.
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Ok(format_response(dt.with_timezone(&Utc)));
        }
        // 2. RFC 2822 (email).
        if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
            return Ok(format_response(dt.with_timezone(&Utc)));
        }
        // 3. Common naive datetime formats.
        for fmt in [
            "%Y-%m-%d %H:%M:%S",
            "%Y-%m-%dT%H:%M:%S",
            "%Y/%m/%d %H:%M:%S",
        ] {
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
                let dt = Utc.from_utc_datetime(&ndt);
                return Ok(format_response(dt));
            }
        }
        // 4. Common date-only formats.
        for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%d/%m/%Y", "%d-%m-%Y", "%m/%d/%Y"] {
            if let Ok(nd) = NaiveDate::parse_from_str(s, fmt) {
                let dt = Utc.from_utc_datetime(&nd.and_hms_opt(0, 0, 0).unwrap());
                return Ok(format_response(dt));
            }
        }

        Err(format!("could not parse date : '{s}'"))
    }
}

fn format_response(dt: DateTime<Utc>) -> serde_json::Value {
    json!({
        "iso8601": dt.to_rfc3339(),
        "timestamp_ms": dt.timestamp_millis(),
        "tz_offset": "+00:00",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_transformation_category() {
        assert_eq!(manifest().category, Category::Transformation);
    }

    #[test]
    fn parses_iso8601() {
        let h = handler();
        let out = h(json!({"input": "2026-05-18T14:30:00Z"})).unwrap();
        assert_eq!(out["iso8601"], "2026-05-18T14:30:00+00:00");
        assert!(out["timestamp_ms"].as_i64().unwrap() > 0);
    }

    #[test]
    fn parses_date_only() {
        let h = handler();
        let out = h(json!({"input": "2026-05-18"})).unwrap();
        assert!(out["iso8601"].as_str().unwrap().starts_with("2026-05-18T00:00:00"));
    }

    #[test]
    fn parses_european_date() {
        let h = handler();
        let out = h(json!({"input": "18/05/2026"})).unwrap();
        assert!(out["iso8601"].as_str().unwrap().starts_with("2026-05-18"));
    }

    #[test]
    fn parses_rfc2822() {
        let h = handler();
        let out = h(json!({"input": "Mon, 18 May 2026 14:30:00 +0000"})).unwrap();
        assert!(out["iso8601"].as_str().unwrap().starts_with("2026-05-18T14:30:00"));
    }

    #[test]
    fn parses_with_explicit_format() {
        let h = handler();
        let out = h(json!({"input": "2026.05.18", "input_format": "%Y.%m.%d"})).unwrap();
        assert!(out["iso8601"].as_str().unwrap().starts_with("2026-05-18"));
    }

    #[test]
    fn rejects_garbage() {
        let h = handler();
        let err = h(json!({"input": "not a date at all"})).unwrap_err();
        assert!(err.contains("could not parse"));
    }
}
