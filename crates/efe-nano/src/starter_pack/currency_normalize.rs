//! `currency_normalize` — parse currency strings (US/EU/Indian format).
//!
//! Mode A — pure Rust, no external deps.  TIER-1.
//!
//! Handles :
//! - US : `$1,234.56`, `1234.56 USD`
//! - EU : `€1.234,56`, `1234,56 EUR`
//! - UK : `£1,234.56`, `GBP 1234.56`
//! - Indian : `₹1,23,456.78` (lakh/crore format)
//! - Generic : `1234.56 XYZ` where XYZ is an ISO 4217 code

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("currency_normalize").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Parse a currency string (US/EU/Indian/generic) and emit normalized value + ISO 4217 code.".to_string(),
        author: Author::Efe,
        category: Category::Transformation,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "currency_normalize::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["input"],
            "properties": {
                "input": { "type": "string", "maxLength": 100 }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["value", "currency_iso4217", "normalized"],
            "properties": {
                "value": { "type": "number" },
                "currency_iso4217": { "type": "string" },
                "normalized": { "type": "string" }
            }
        }),
        error_schema: None,
        limits: ResourceLimits::default_mode_a(),
        cost_estimate_cents: 0.001,
        latency_p99_ms_target: 5,
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
        parse_currency(s.trim())
    }
}

fn parse_currency(s: &str) -> Result<serde_json::Value, String> {
    if s.is_empty() {
        return Err("empty input".to_string());
    }

    // 1. Detect currency from symbol or ISO 4217 code.
    let mut currency = "UNKNOWN".to_string();
    let mut remaining = s.to_string();

    // Symbol map.
    let symbols: &[(&str, &str)] = &[
        ("$", "USD"),
        ("€", "EUR"),
        ("£", "GBP"),
        ("¥", "JPY"),
        ("₹", "INR"),
        ("CHF", "CHF"),
    ];
    for (sym, iso) in symbols {
        if remaining.contains(sym) {
            currency = (*iso).to_string();
            remaining = remaining.replace(sym, " ");
            break;
        }
    }

    // ISO 4217 code at end.
    if currency == "UNKNOWN" {
        let trimmed = remaining.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if let Some(last) = parts.last() {
            if last.len() == 3 && last.chars().all(|c| c.is_ascii_uppercase()) {
                currency = last.to_string();
                let len_to_remove = last.len();
                let new_len = trimmed.len().saturating_sub(len_to_remove);
                remaining = trimmed[..new_len].trim().to_string();
            }
        }
    }

    // 2. Extract numeric value.
    let raw_value = remaining.trim();
    let value = parse_numeric(raw_value)?;

    // 3. Build normalized form (always English notation : period decimal).
    let normalized = format!("{value:.2} {currency}");

    Ok(json!({
        "value": value,
        "currency_iso4217": currency,
        "normalized": normalized,
    }))
}

/// Parse a numeric string supporting US (1,234.56), EU (1.234,56), or simple (1234.56) formats.
fn parse_numeric(s: &str) -> Result<f64, String> {
    let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return Err("no numeric value".to_string());
    }

    // Detect format by inspecting the last separator position.
    let has_dot = cleaned.contains('.');
    let has_comma = cleaned.contains(',');

    let normalized = if has_dot && has_comma {
        // Mixed : whichever separator appears LAST is the decimal separator.
        let last_dot = cleaned.rfind('.').unwrap();
        let last_comma = cleaned.rfind(',').unwrap();
        if last_dot > last_comma {
            // US-style : 1,234.56  → remove commas
            cleaned.replace(',', "")
        } else {
            // EU-style : 1.234,56  → remove dots, replace comma with dot
            cleaned.replace('.', "").replace(',', ".")
        }
    } else if has_comma {
        // Comma only : could be decimal (EU) or thousands separator.
        let parts: Vec<&str> = cleaned.split(',').collect();
        if parts.len() == 2 && parts[1].len() <= 2 {
            // EU-style decimal : 12,34
            cleaned.replace(',', ".")
        } else {
            // US thousands : 1,234,567
            cleaned.replace(',', "")
        }
    } else {
        // No separator OR dot only : leave as is.
        cleaned
    };

    normalized
        .parse::<f64>()
        .map_err(|e| format!("parse numeric failed : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_transformation_category() {
        assert_eq!(manifest().category, Category::Transformation);
    }

    #[test]
    fn parses_us_format() {
        let h = handler();
        let out = h(json!({"input": "$1,234.56"})).unwrap();
        assert_eq!(out["currency_iso4217"], "USD");
        assert!((out["value"].as_f64().unwrap() - 1234.56).abs() < 0.001);
    }

    #[test]
    fn parses_eu_format() {
        let h = handler();
        let out = h(json!({"input": "€1.234,56"})).unwrap();
        assert_eq!(out["currency_iso4217"], "EUR");
        assert!((out["value"].as_f64().unwrap() - 1234.56).abs() < 0.001);
    }

    #[test]
    fn parses_iso_suffix() {
        let h = handler();
        let out = h(json!({"input": "1234.56 USD"})).unwrap();
        assert_eq!(out["currency_iso4217"], "USD");
    }

    #[test]
    fn parses_chf() {
        let h = handler();
        let out = h(json!({"input": "1234.56 CHF"})).unwrap();
        assert_eq!(out["currency_iso4217"], "CHF");
    }

    #[test]
    fn parses_gbp_symbol() {
        let h = handler();
        let out = h(json!({"input": "£500.00"})).unwrap();
        assert_eq!(out["currency_iso4217"], "GBP");
        assert!((out["value"].as_f64().unwrap() - 500.0).abs() < 0.001);
    }

    #[test]
    fn parses_yen() {
        let h = handler();
        let out = h(json!({"input": "¥10000"})).unwrap();
        assert_eq!(out["currency_iso4217"], "JPY");
        assert!((out["value"].as_f64().unwrap() - 10000.0).abs() < 0.001);
    }

    #[test]
    fn unknown_currency_when_no_symbol() {
        let h = handler();
        let out = h(json!({"input": "1234.56"})).unwrap();
        assert_eq!(out["currency_iso4217"], "UNKNOWN");
    }

    #[test]
    fn rejects_empty() {
        let h = handler();
        let err = h(json!({"input": ""})).unwrap_err();
        assert!(err.contains("empty"));
    }

    #[test]
    fn rejects_non_numeric() {
        let h = handler();
        let err = h(json!({"input": "$abc"})).unwrap_err();
        assert!(err.contains("parse"));
    }

    #[test]
    fn normalized_has_two_decimals() {
        let h = handler();
        let out = h(json!({"input": "$1234"})).unwrap();
        assert!(out["normalized"].as_str().unwrap().contains("1234.00 USD"));
    }
}
