//! `markdown_extract_headings` — extract headings from markdown.
//!
//! Mode A — pure Rust (pulldown-cmark).  TIER-1.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde_json::json;

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("markdown_extract_headings").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Extract all headings from a markdown document with their level and anchor."
            .to_string(),
        author: Author::Efe,
        category: Category::Extraction,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "markdown_extract_headings::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["markdown"],
            "properties": {
                "markdown": { "type": "string" }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["headings", "count"],
            "properties": {
                "count": { "type": "integer", "minimum": 0 },
                "headings": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["level", "text", "anchor"],
                        "properties": {
                            "level": { "type": "integer", "minimum": 1, "maximum": 6 },
                            "text": { "type": "string" },
                            "anchor": { "type": "string" }
                        }
                    }
                }
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
        let md = input
            .get("markdown")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'markdown'".to_string())?;

        let parser = Parser::new(md);
        let mut headings = Vec::new();
        let mut current_level: Option<u8> = None;
        let mut current_text = String::new();

        for event in parser {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    current_level = Some(heading_level_to_u8(level));
                    current_text.clear();
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let Some(level) = current_level {
                        let text = current_text.trim().to_string();
                        let anchor = slugify(&text);
                        headings.push(json!({
                            "level": level,
                            "text": text,
                            "anchor": anchor,
                        }));
                    }
                    current_level = None;
                    current_text.clear();
                }
                Event::Text(t) => {
                    if current_level.is_some() {
                        current_text.push_str(&t);
                    }
                }
                Event::Code(c) => {
                    if current_level.is_some() {
                        current_text.push_str(&c);
                    }
                }
                _ => {}
            }
        }

        let count = headings.len();
        Ok(json!({
            "count": count,
            "headings": headings,
        }))
    }
}

fn heading_level_to_u8(level: pulldown_cmark::HeadingLevel) -> u8 {
    match level {
        pulldown_cmark::HeadingLevel::H1 => 1,
        pulldown_cmark::HeadingLevel::H2 => 2,
        pulldown_cmark::HeadingLevel::H3 => 3,
        pulldown_cmark::HeadingLevel::H4 => 4,
        pulldown_cmark::HeadingLevel::H5 => 5,
        pulldown_cmark::HeadingLevel::H6 => 6,
    }
}

/// Convert text to a URL-friendly anchor (slug).
fn slugify(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_extraction_category() {
        assert_eq!(manifest().category, Category::Extraction);
    }

    #[test]
    fn extracts_h1() {
        let h = handler();
        let out = h(json!({"markdown": "# Hello World\n\nBody text."})).unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["headings"][0]["level"], 1);
        assert_eq!(out["headings"][0]["text"], "Hello World");
        assert_eq!(out["headings"][0]["anchor"], "hello-world");
    }

    #[test]
    fn extracts_multiple_levels() {
        let h = handler();
        let md = "# H1\n\n## H2\n\n### H3\n\n#### H4";
        let out = h(json!({"markdown": md})).unwrap();
        assert_eq!(out["count"], 4);
        let levels: Vec<i64> = out["headings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["level"].as_i64().unwrap())
            .collect();
        assert_eq!(levels, vec![1, 2, 3, 4]);
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  Foo  Bar  "), "foo-bar");
        assert_eq!(slugify("a/b/c"), "a-b-c");
    }

    #[test]
    fn empty_markdown_returns_zero() {
        let h = handler();
        let out = h(json!({"markdown": ""})).unwrap();
        assert_eq!(out["count"], 0);
    }

    #[test]
    fn no_headings_in_plain_text() {
        let h = handler();
        let out = h(json!({"markdown": "Just some body text.\n\nAnother paragraph."})).unwrap();
        assert_eq!(out["count"], 0);
    }

    #[test]
    fn handles_inline_code_in_heading() {
        let h = handler();
        let out = h(json!({"markdown": "## Using `cargo test`"})).unwrap();
        assert_eq!(out["count"], 1);
        assert_eq!(out["headings"][0]["text"], "Using cargo test");
    }
}
