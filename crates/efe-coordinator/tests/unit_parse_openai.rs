// SPDX-License-Identifier: UNLICENSED
//! Tests de parsing OpenAI sur fixtures réalistes (offline, $0).
//!
//! Fixtures dans `tests/fixtures/openai/*.json` — captures représentatives
//! des formats que retourne `POST /v1/chat/completions`.

#![cfg(feature = "openai")]

use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/openai")
        .join(name)
}

fn load_fixture(name: &str) -> serde_json::Value {
    let path = fixture_path(name);
    let s = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture not found: {}", path.display()));
    serde_json::from_str(&s).unwrap_or_else(|_| panic!("invalid JSON in {}", path.display()))
}

// Reproduction locale de l'extraction (équivalent à
// `GptProvider::extract_text_and_tools`) pour tests sans dépendre du trait.
fn extract_text_and_tools(json_resp: &serde_json::Value) -> String {
    let message = &json_resp["choices"][0]["message"];
    let mut out = String::new();
    if let Some(t) = message["content"].as_str() {
        out.push_str(t);
    }
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("?");
            let func = &tc["function"];
            let name = func["name"].as_str().unwrap_or("?");
            let args = func["arguments"].as_str().unwrap_or("{}");
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!(
                "[tool_call id={id} name={name}]\n{args}\n[/tool_call]"
            ));
        }
    }
    out
}

#[test]
fn fixture_001_factual_qa_extracts_paris() {
    let raw = load_fixture("001_factual_qa_simple.json");
    let text = extract_text_and_tools(&raw);
    assert_eq!(text, "Paris is the capital of France.");
    assert_eq!(raw["usage"]["prompt_tokens"], 14);
    assert_eq!(raw["usage"]["completion_tokens"], 8);
    assert_eq!(raw["choices"][0]["finish_reason"], "stop");
}

#[test]
fn fixture_002_code_block_preserved() {
    let raw = load_fixture("002_code_block_rust.json");
    let text = extract_text_and_tools(&raw);
    assert!(text.contains("```rust"));
    assert!(text.contains("fn add(a: i32, b: i32) -> i32"));
    assert!(text.contains("```\n"));
}

#[test]
fn fixture_003_max_tokens_truncated_finish_reason_length() {
    let raw = load_fixture("003_max_tokens_truncated.json");
    let text = extract_text_and_tools(&raw);
    assert!(text.starts_with("Once upon"));
    assert_eq!(raw["choices"][0]["finish_reason"], "length");
    // Content tronqué (pas de ponctuation finale).
    assert!(!text.ends_with('.'));
}

#[test]
fn fixture_004_tool_call_single_serialized() {
    let raw = load_fixture("004_tool_call_single.json");
    let text = extract_text_and_tools(&raw);
    assert!(text.contains("[tool_call id=call_1aZxQpKn3LfP8w4mRT5jVbYu name=get_weather]"));
    assert!(text.contains("Paris, France"));
    assert!(text.contains("celsius"));
    assert!(text.contains("[/tool_call]"));
    assert_eq!(raw["choices"][0]["finish_reason"], "tool_calls");
}

#[test]
fn fixture_005_tool_call_parallel_both_emitted() {
    let raw = load_fixture("005_tool_call_parallel.json");
    let text = extract_text_and_tools(&raw);
    // Texte initial préservé.
    assert!(text.starts_with("I'll check both cities for you."));
    // Les deux tool_calls sont sérialisés.
    assert!(text.contains("[tool_call id=call_paris001 name=get_weather]"));
    assert!(text.contains("[tool_call id=call_lyon002 name=get_weather]"));
    assert!(text.contains("Paris"));
    assert!(text.contains("Lyon"));
}

#[test]
fn fixture_006_long_reasoning_preserved() {
    let raw = load_fixture("006_long_reasoning.json");
    let text = extract_text_and_tools(&raw);
    assert!(text.contains("step by step"));
    assert!(text.contains("42"));
    assert!(text.contains("1. First"));
    assert!(text.contains("4. Finally"));
}

#[test]
fn fixture_007_error_rate_limit_payload_shape() {
    let raw = load_fixture("007_error_rate_limit.json");
    // Pas de `choices` → extraction vide.
    let text = extract_text_and_tools(&raw);
    assert_eq!(text, "");
    assert_eq!(raw["error"]["type"], "rate_limit_exceeded");
    assert_eq!(raw["error"]["code"], "rate_limit_exceeded");
    let msg = raw["error"]["message"].as_str().unwrap();
    assert!(msg.contains("Rate limit reached"));
}

#[test]
fn fixture_008_refusal_content_null() {
    let raw = load_fixture("008_refusal.json");
    let text = extract_text_and_tools(&raw);
    // `content: null` → extracted text vide.
    assert_eq!(text, "");
    // Mais le champ `refusal` est lisible séparément.
    let refusal = raw["choices"][0]["message"]["refusal"].as_str().unwrap();
    assert!(refusal.contains("cannot help"));
}

#[test]
fn pricing_lookup_gpt4o_known() {
    use efe_coordinator::llm::providers::PricingTable;
    let p = PricingTable::openai();
    // gpt-4o : (250 in, 1000 out) cents/M.
    // 14 prompt + 8 completion ≈ (14×250 + 8×1000) / 1M = 0.0115 → arrondi 1
    let cost = p.calc("gpt-4o", 14, 8);
    assert_eq!(cost, 1);
}

#[test]
fn pricing_lookup_gpt4o_mini_cheaper() {
    use efe_coordinator::llm::providers::PricingTable;
    let p = PricingTable::openai();
    // gpt-4o-mini : (15 in, 60 out) cents/M.
    // 14 + 8 ≈ (14×15 + 8×60) / 1M = 0.00069 → arrondi 1
    let mini_cost = p.calc("gpt-4o-mini", 14, 8);
    let std_cost = p.calc("gpt-4o", 14, 8);
    assert!(mini_cost <= std_cost, "mini={mini_cost} std={std_cost}");
}

#[test]
fn pricing_o1_reasoning_premium() {
    use efe_coordinator::llm::providers::PricingTable;
    let p = PricingTable::openai();
    // o1 (1500 in, 6000 out) cents/M vs gpt-4o-mini (15, 60) cents/M → ratio 100×.
    // À 100k tokens chacun :
    //   o1   : (100_000 × 1500 + 100_000 × 6000) / 1M = 750 cents
    //   mini : (100_000 × 15   + 100_000 × 60)   / 1M = 7.5 → 8 cents
    let o1_cost = p.calc("o1", 100_000, 100_000);
    let mini_cost = p.calc("gpt-4o-mini", 100_000, 100_000);
    assert_eq!(o1_cost, 750);
    assert_eq!(mini_cost, 8);
    assert!(o1_cost > 50 * mini_cost, "o1={o1_cost} mini={mini_cost}");
}

#[test]
fn unknown_model_pricing_uses_fallback() {
    use efe_coordinator::llm::providers::PricingTable;
    let p = PricingTable::openai();
    let cost = p.calc("gpt-x-experimental-2099", 1000, 1000);
    // Fallback (1000, 5000) → 6 cents
    assert_eq!(cost, 6);
}
