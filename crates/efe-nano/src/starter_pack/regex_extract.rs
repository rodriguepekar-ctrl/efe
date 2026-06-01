//! `regex_extract` — run regex against text, return matches.
//!
//! Mode A — pure Rust.  TIER-1.  DoS-protected via DFA size limit.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use regex::RegexBuilder;
use serde_json::json;

const DFA_SIZE_LIMIT_BYTES: usize = 1_048_576;        // 1 MB
const SIZE_LIMIT_BYTES: usize = 10 * 1_048_576;       // 10 MB
const MAX_MATCHES: usize = 10_000;                     // safety cap

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("regex_extract").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Run a regular expression against text and return matches with capture groups."
            .to_string(),
        author: Author::Efe,
        category: Category::Extraction,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "regex_extract::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["text", "pattern"],
            "properties": {
                "text": { "type": "string" },
                "pattern": { "type": "string" },
                "case_insensitive": { "type": "boolean", "default": false },
                "multi_line": { "type": "boolean", "default": false }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["matches", "count"],
            "properties": {
                "count": { "type": "integer", "minimum": 0 },
                "truncated": { "type": "boolean" },
                "matches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["start", "end", "groups"],
                        "properties": {
                            "start": { "type": "integer", "minimum": 0 },
                            "end": { "type": "integer", "minimum": 0 },
                            "groups": { "type": "array", "items": { "type": ["string", "null"] } }
                        }
                    }
                }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 128,
            cpu_ms: 500,
            wall_clock_ms: 2_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.005,
        latency_p99_ms_target: 30,
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
        let text = input
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'text'".to_string())?;
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'pattern'".to_string())?;

        if text.len() > SIZE_LIMIT_BYTES {
            return Err(format!(
                "text size {} exceeds limit {}",
                text.len(),
                SIZE_LIMIT_BYTES
            ));
        }

        let case_insensitive = input
            .get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let multi_line = input
            .get("multi_line")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let regex = RegexBuilder::new(pattern)
            .case_insensitive(case_insensitive)
            .multi_line(multi_line)
            .size_limit(DFA_SIZE_LIMIT_BYTES)
            .build()
            .map_err(|e| format!("invalid pattern : {e}"))?;

        let mut matches: Vec<serde_json::Value> = Vec::new();
        let mut truncated = false;

        for m in regex.captures_iter(text) {
            if matches.len() >= MAX_MATCHES {
                truncated = true;
                break;
            }
            let m_main = m.get(0).unwrap();  // capture 0 always exists
            let groups: Vec<serde_json::Value> = m
                .iter()
                .skip(1)  // skip the full match
                .map(|opt| match opt {
                    Some(g) => json!(g.as_str()),
                    None => json!(null),
                })
                .collect();
            matches.push(json!({
                "start": m_main.start(),
                "end": m_main.end(),
                "groups": groups,
            }));
        }

        let count = matches.len();
        Ok(json!({
            "count": count,
            "truncated": truncated,
            "matches": matches,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_extraction_category() {
        let m = manifest();
        assert_eq!(m.category, Category::Extraction);
    }

    #[test]
    fn finds_simple_match() {
        let h = handler();
        let out = h(json!({"text": "hello world", "pattern": "world"})).unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["matches"][0]["start"], 6);
        assert_eq!(out["matches"][0]["end"], 11);
    }

    #[test]
    fn captures_groups() {
        let h = handler();
        let out = h(json!({
            "text": "John Doe : 30 years old",
            "pattern": r"(\w+) (\w+) : (\d+)"
        }))
        .unwrap();
        assert_eq!(out["count"], 1);
        let groups = out["matches"][0]["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0], "John");
        assert_eq!(groups[1], "Doe");
        assert_eq!(groups[2], "30");
    }

    #[test]
    fn finds_multiple_matches() {
        let h = handler();
        let out = h(json!({"text": "aaa", "pattern": "a"})).unwrap();
        assert_eq!(out["count"], 3);
    }

    #[test]
    fn case_insensitive_flag() {
        let h = handler();
        let out = h(json!({"text": "HELLO", "pattern": "hello", "case_insensitive": true})).unwrap();
        assert_eq!(out["count"], 1);
        let out_strict = h(json!({"text": "HELLO", "pattern": "hello"})).unwrap();
        assert_eq!(out_strict["count"], 0);
    }

    #[test]
    fn invalid_pattern_rejected() {
        let h = handler();
        let err = h(json!({"text": "x", "pattern": "([unclosed"})).unwrap_err();
        assert!(err.contains("invalid pattern"));
    }

    #[test]
    fn size_limit_rejected() {
        let h = handler();
        let huge = "a".repeat(SIZE_LIMIT_BYTES + 1);
        let err = h(json!({"text": huge, "pattern": "a"})).unwrap_err();
        assert!(err.contains("exceeds limit"));
    }

    #[test]
    fn no_match_returns_zero() {
        let h = handler();
        let out = h(json!({"text": "abc", "pattern": "xyz"})).unwrap();
        assert_eq!(out["count"], 0);
        assert_eq!(out["matches"].as_array().unwrap().len(), 0);
    }
}
