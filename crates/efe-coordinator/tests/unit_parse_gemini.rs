// SPDX-License-Identifier: UNLICENSED
//! Tests de parsing Gemini sur fixtures réalistes (offline, $0).

#![cfg(feature = "gemini")]

use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/gemini")
        .join(name)
}

fn load_fixture(name: &str) -> serde_json::Value {
    let path = fixture_path(name);
    let s = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));
    serde_json::from_str(&s).unwrap_or_else(|_| panic!("invalid JSON in {}", path.display()))
}

// ─── Helper de parsing (équivalent code dans GeminiProvider::call) ──

fn extract_text(json_resp: &serde_json::Value) -> String {
    json_resp["candidates"][0]["content"]["parts"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn extract_finish_reason(json_resp: &serde_json::Value) -> Option<String> {
    json_resp["candidates"][0]["finishReason"]
        .as_str()
        .map(String::from)
}

fn extract_safety_block(json_resp: &serde_json::Value) -> Option<String> {
    json_resp["candidates"][0]["safetyRatings"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|r| r["blocked"].as_bool() == Some(true))
                .and_then(|r| r["category"].as_str().map(String::from))
        })
        .or_else(|| {
            json_resp["promptFeedback"]["blockReason"]
                .as_str()
                .map(String::from)
        })
}

#[test]
fn fixture_001_factual_qa_extracts_paris() {
    let raw = load_fixture("001_factual_qa_simple.json");
    let text = extract_text(&raw);
    assert_eq!(text, "Paris is the capital of France.");
    assert_eq!(raw["usageMetadata"]["promptTokenCount"], 12);
    assert_eq!(raw["usageMetadata"]["candidatesTokenCount"], 8);
    assert_eq!(extract_finish_reason(&raw).as_deref(), Some("STOP"));
}

#[test]
fn fixture_002_markdown_footnotes_preserved() {
    let raw = load_fixture("002_markdown_with_footnote.json");
    let text = extract_text(&raw);
    assert!(text.contains("**Paris**"));
    assert!(text.contains("[^1]"));
    assert!(text.contains("CIA World Factbook"));
}

#[test]
fn fixture_003_multi_parts_concatenated_in_order() {
    let raw = load_fixture("003_multi_parts_concatenation.json");
    let text = extract_text(&raw);
    assert_eq!(text, "Part one. Part two. Part three.");
}

#[test]
fn fixture_004_max_tokens_finish_reason() {
    let raw = load_fixture("004_max_tokens_truncated.json");
    assert_eq!(extract_finish_reason(&raw).as_deref(), Some("MAX_TOKENS"));
    let text = extract_text(&raw);
    assert!(text.contains("Babbage"));
}

#[test]
fn fixture_005_python_code_block() {
    let raw = load_fixture("005_code_block_python.json");
    let text = extract_text(&raw);
    assert!(text.contains("```python"));
    assert!(text.contains("def fib"));
}

#[test]
fn fixture_006_error_format() {
    let raw = load_fixture("006_error_quota.json");
    assert_eq!(raw["error"]["code"], 429);
    assert_eq!(raw["error"]["status"], "RESOURCE_EXHAUSTED");
    // Pas de candidates → text vide.
    assert_eq!(extract_text(&raw), "");
}

#[test]
fn fixture_007_safety_blocked_detected() {
    let raw = load_fixture("007_safety_blocked.json");
    assert_eq!(extract_finish_reason(&raw).as_deref(), Some("SAFETY"));
    let block = extract_safety_block(&raw);
    assert_eq!(block.as_deref(), Some("HARM_CATEGORY_HATE_SPEECH"));
    // Text est vide (parts: []).
    assert_eq!(extract_text(&raw), "");
}

#[test]
fn fixture_008_empty_candidates_prompt_block() {
    let raw = load_fixture("008_empty_candidates.json");
    // Pas de candidates[0] → extract_text retourne empty.
    assert_eq!(extract_text(&raw), "");
    // promptFeedback.blockReason est exposé via extract_safety_block.
    let block = extract_safety_block(&raw);
    assert_eq!(block.as_deref(), Some("OTHER"));
}

#[test]
fn all_fixtures_parse_as_valid_json() {
    for n in [
        "001_factual_qa_simple.json",
        "002_markdown_with_footnote.json",
        "003_multi_parts_concatenation.json",
        "004_max_tokens_truncated.json",
        "005_code_block_python.json",
        "006_error_quota.json",
        "007_safety_blocked.json",
        "008_empty_candidates.json",
    ] {
        let _ = load_fixture(n);
    }
}

// ─── Edge cases ──────────────────────────────────────────────────────

#[test]
fn missing_candidates_field_empty_text() {
    let raw = serde_json::json!({"usageMetadata": {}});
    assert_eq!(extract_text(&raw), "");
}

#[test]
fn candidates_without_content_yields_empty() {
    let raw = serde_json::json!({"candidates": [{}]});
    assert_eq!(extract_text(&raw), "");
}

#[test]
fn parts_with_non_text_skipped() {
    let raw = serde_json::json!({
        "candidates": [{
            "content": {
                "parts": [
                    {"text": "A"},
                    {"inlineData": {"mimeType": "image/png", "data": "..."}},
                    {"text": "B"}
                ]
            }
        }]
    });
    assert_eq!(extract_text(&raw), "AB");
}
