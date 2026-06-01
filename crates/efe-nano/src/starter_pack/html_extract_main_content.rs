//! `html_extract_main_content` — extract main text content from HTML.
//!
//! Mode A — pure Rust.  TIER-1.
//!
//! Phase 1 implementation uses a lightweight regex+strip approach :
//! - Strip `<script>`, `<style>` blocks.
//! - Strip all remaining tags.
//! - Collapse whitespace.
//!
//! Phase 2 may bump to `readability-rs` for true main-content detection.

use chrono::Utc;
use efe_nano_sdk::{
    AuditPolicy, Author, Category, ComplianceTags, DeterminismLevel, ExecutionMode,
    IdempotencyPolicy, NanoAgent, NanoAgentId, NanoAgentVersion, ResourceLimits, StabilityTier,
};
use regex::Regex;
use serde_json::json;
use std::sync::OnceLock;

const SIZE_LIMIT_BYTES: usize = 10 * 1_048_576;       // 10 MB

static SCRIPT_RE: OnceLock<Regex> = OnceLock::new();
static STYLE_RE: OnceLock<Regex> = OnceLock::new();
static TAG_RE: OnceLock<Regex> = OnceLock::new();
static TITLE_RE: OnceLock<Regex> = OnceLock::new();
static WS_RE: OnceLock<Regex> = OnceLock::new();

fn script_re() -> &'static Regex {
    SCRIPT_RE.get_or_init(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap())
}
fn style_re() -> &'static Regex {
    STYLE_RE.get_or_init(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap())
}
fn tag_re() -> &'static Regex {
    TAG_RE.get_or_init(|| Regex::new(r"<[^>]+>").unwrap())
}
fn title_re() -> &'static Regex {
    TITLE_RE.get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").unwrap())
}
fn ws_re() -> &'static Regex {
    WS_RE.get_or_init(|| Regex::new(r"\s+").unwrap())
}

pub fn manifest() -> NanoAgent {
    NanoAgent {
        id: NanoAgentId::new("html_extract_main_content").expect("id is valid"),
        version: NanoAgentVersion::new("1.0.0").expect("version is valid"),
        description: "Extract the main text content from an HTML document (strips script/style/tags).".to_string(),
        author: Author::Efe,
        category: Category::Extraction,
        stability_tier: StabilityTier::Tier1,
        execution_mode: ExecutionMode::PureRust {
            entry: "html_extract_main_content::run".to_string(),
        },
        determinism: DeterminismLevel::FullDeterministic,
        idempotency: IdempotencyPolicy::auto_24h(),
        input_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["html"],
            "properties": {
                "html": { "type": "string" },
                "url": { "type": ["string", "null"] }
            }
        }),
        output_schema: json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["main_text", "char_count"],
            "properties": {
                "title": { "type": ["string", "null"] },
                "main_text": { "type": "string" },
                "char_count": { "type": "integer", "minimum": 0 }
            }
        }),
        error_schema: None,
        limits: ResourceLimits {
            memory_mb: 256,
            cpu_ms: 2_000,
            wall_clock_ms: 5_000,
            llm_tokens: None,
            network_egress_kb: 0,
        },
        cost_estimate_cents: 0.005,
        latency_p99_ms_target: 100,
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
        let html = input
            .get("html")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required field 'html'".to_string())?;

        if html.len() > SIZE_LIMIT_BYTES {
            return Err(format!(
                "html size {} exceeds limit {}",
                html.len(),
                SIZE_LIMIT_BYTES
            ));
        }

        // Extract title.
        let title = title_re()
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| decode_html_entities(m.as_str()).trim().to_string());

        // Strip scripts + styles.
        let stripped = script_re().replace_all(html, " ");
        let stripped = style_re().replace_all(&stripped, " ");
        // Strip remaining tags.
        let stripped = tag_re().replace_all(&stripped, " ");
        // Decode common HTML entities.
        let stripped = decode_html_entities(&stripped);
        // Collapse whitespace.
        let main_text = ws_re().replace_all(&stripped, " ").trim().to_string();
        let char_count = main_text.chars().count();

        Ok(json!({
            "title": title,
            "main_text": main_text,
            "char_count": char_count,
        }))
    }
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_extraction_category() {
        assert_eq!(manifest().category, Category::Extraction);
    }

    #[test]
    fn extracts_title_and_body() {
        let h = handler();
        let html = "<html><head><title>My Page</title></head><body><p>Hello World</p></body></html>";
        let out = h(json!({"html": html})).unwrap();
        assert_eq!(out["title"], "My Page");
        assert_eq!(out["main_text"], "My Page Hello World");
    }

    #[test]
    fn strips_script_block() {
        let h = handler();
        let html = "<html><body>visible<script>alert('hi')</script>after</body></html>";
        let out = h(json!({"html": html})).unwrap();
        let text = out["main_text"].as_str().unwrap();
        assert!(text.contains("visible"));
        assert!(text.contains("after"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn strips_style_block() {
        let h = handler();
        let html = "<html><body>content<style>.x{color:red}</style>more</body></html>";
        let out = h(json!({"html": html})).unwrap();
        let text = out["main_text"].as_str().unwrap();
        assert!(!text.contains("color:red"));
    }

    #[test]
    fn decodes_html_entities() {
        let h = handler();
        let html = "<p>Tom &amp; Jerry &lt;3</p>";
        let out = h(json!({"html": html})).unwrap();
        let text = out["main_text"].as_str().unwrap();
        assert!(text.contains("Tom & Jerry"));
        assert!(text.contains("<3"));
    }

    #[test]
    fn empty_html_returns_empty() {
        let h = handler();
        let out = h(json!({"html": ""})).unwrap();
        assert_eq!(out["main_text"], "");
        assert_eq!(out["char_count"], 0);
    }

    #[test]
    fn no_title_returns_null() {
        let h = handler();
        let out = h(json!({"html": "<p>no title here</p>"})).unwrap();
        assert!(out["title"].is_null());
    }

    #[test]
    fn size_limit_rejected() {
        let h = handler();
        let huge = "a".repeat(SIZE_LIMIT_BYTES + 1);
        let err = h(json!({"html": huge})).unwrap_err();
        assert!(err.contains("exceeds limit"));
    }
}
