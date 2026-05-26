// SPDX-License-Identifier: UNLICENSED
//! Tests de parsing Claude sur fixtures réalistes (offline, $0).
//!
//! Fixtures dans `tests/fixtures/claude/*.json` — captures représentatives
//! des formats réels que retourne Anthropic Messages API v1.

#![cfg(feature = "anthropic")]

use std::path::PathBuf;

use efe_coordinator::llm::providers::common::strip_thinking_tags;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/claude")
        .join(name)
}

fn load_fixture(name: &str) -> serde_json::Value {
    let path = fixture_path(name);
    let s = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));
    serde_json::from_str(&s).unwrap_or_else(|_| panic!("invalid JSON in {}", path.display()))
}

// ─── Parsing logique (équivalent à ClaudeProvider::extract_text_and_tools) ──

fn extract_text_and_tools(
    json_resp: &serde_json::Value,
    strip_thinking: bool,
) -> String {
    let blocks = match json_resp["content"].as_array() {
        Some(arr) => arr,
        None => return String::new(),
    };
    let mut out = String::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => {
                if let Some(t) = block["text"].as_str() {
                    out.push_str(t);
                }
            }
            Some("tool_use") => {
                let name = block["name"].as_str().unwrap_or("?");
                let id = block["id"].as_str().unwrap_or("?");
                let input_str = block["input"].to_string();
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&format!(
                    "[tool_use id={id} name={name}]\n{input_str}\n[/tool_use]"
                ));
            }
            _ => {}
        }
    }
    if strip_thinking {
        strip_thinking_tags(&out)
    } else {
        out
    }
}

#[test]
fn fixture_001_factual_qa_extracts_paris() {
    let raw = load_fixture("001_factual_qa_simple.json");
    let text = extract_text_and_tools(&raw, true);
    assert_eq!(text, "Paris is the capital of France.");
    assert_eq!(raw["usage"]["input_tokens"], 12);
    assert_eq!(raw["usage"]["output_tokens"], 8);
    assert_eq!(raw["stop_reason"], "end_turn");
}

#[test]
fn fixture_002_thinking_tag_stripped() {
    let raw = load_fixture("002_with_thinking_tag.json");
    let text = extract_text_and_tools(&raw, true);
    assert!(!text.contains("<thinking>"));
    assert!(!text.contains("</thinking>"));
    assert!(text.contains("299,792,458"));
}

#[test]
fn fixture_002_thinking_kept_when_flag_off() {
    let raw = load_fixture("002_with_thinking_tag.json");
    let text = extract_text_and_tools(&raw, false);
    assert!(text.contains("<thinking>"));
    assert!(text.contains("</thinking>"));
}

#[test]
fn fixture_003_code_block_preserved() {
    let raw = load_fixture("003_code_block_rust.json");
    let text = extract_text_and_tools(&raw, true);
    assert!(text.contains("```rust"));
    assert!(text.contains("fn fib"));
    assert!(text.contains("```"));
}

#[test]
fn fixture_004_max_tokens_detected() {
    let raw = load_fixture("004_max_tokens_truncated.json");
    assert_eq!(raw["stop_reason"], "max_tokens");
    let text = extract_text_and_tools(&raw, true);
    // Le texte est tronqué mid-sentence (s'arrête après "Babbage designed the Analytical Engine in").
    assert!(text.ends_with("in"));
}

#[test]
fn fixture_005_tool_use_serialized() {
    let raw = load_fixture("005_tool_use.json");
    let text = extract_text_and_tools(&raw, true);
    assert!(text.contains("I'll look up the current weather"));
    assert!(text.contains("[tool_use id=toolu_01ABC name=get_weather]"));
    assert!(text.contains("Paris,FR"));
    assert!(text.contains("[/tool_use]"));
    assert_eq!(raw["stop_reason"], "tool_use");
}

#[test]
fn fixture_006_multi_text_blocks_concatenated() {
    let raw = load_fixture("006_multi_text_blocks.json");
    let text = extract_text_and_tools(&raw, true);
    assert_eq!(text, "First part: second part.");
}

#[test]
fn fixture_007_error_format_distinct() {
    let raw = load_fixture("007_error_rate_limit.json");
    // Pas de "content" array dans une erreur — extract retourne empty.
    let text = extract_text_and_tools(&raw, true);
    assert_eq!(text, "");
    // Mais le JSON contient bien le format erreur.
    assert_eq!(raw["type"], "error");
    assert_eq!(raw["error"]["type"], "rate_limit_error");
}

#[test]
fn fixture_008_reasoning_thinking_stripped_clean() {
    let raw = load_fixture("008_long_reasoning.json");
    let text = extract_text_and_tools(&raw, true);
    assert_eq!(text, "Yes, 2+2 equals 4.");
}

#[test]
fn all_fixtures_parse_as_valid_json() {
    for n in [
        "001_factual_qa_simple.json",
        "002_with_thinking_tag.json",
        "003_code_block_rust.json",
        "004_max_tokens_truncated.json",
        "005_tool_use.json",
        "006_multi_text_blocks.json",
        "007_error_rate_limit.json",
        "008_long_reasoning.json",
    ] {
        let _ = load_fixture(n); // panic-test
    }
}

// ─── Edge cases ──────────────────────────────────────────────────────

#[test]
fn empty_content_array_yields_empty_text() {
    let raw = serde_json::json!({"content": []});
    assert_eq!(extract_text_and_tools(&raw, true), "");
}

#[test]
fn missing_content_field_yields_empty() {
    let raw = serde_json::json!({"type": "message"});
    assert_eq!(extract_text_and_tools(&raw, true), "");
}

#[test]
fn unknown_block_type_skipped() {
    let raw = serde_json::json!({
        "content": [
            {"type": "text", "text": "keep "},
            {"type": "image", "data": "..."},
            {"type": "text", "text": "this"}
        ]
    });
    assert_eq!(extract_text_and_tools(&raw, true), "keep this");
}
