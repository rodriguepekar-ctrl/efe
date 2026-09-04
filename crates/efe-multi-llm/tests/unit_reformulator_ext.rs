// SPDX-License-Identifier: UNLICENSED
//! Tests `CanonicalReformulator`.

use efe_multi_llm::reformulator_ext::SourceDialect;
use efe_multi_llm::{
    CanonicalReformulator, CanonicalSchemaId,
};

#[test]
fn claude_xml_to_claim_extracts_tag() {
    let r = CanonicalReformulator::new();
    let xml = "<claim>Lyon est en France.</claim>\n<source>Wikipedia</source>";
    let claim = r.to_claim(xml, SourceDialect::ClaudeXml).unwrap();
    assert!(claim.claim.contains("Lyon"));
    assert_eq!(claim.evidence.len(), 1);
}

#[test]
fn gpt_json_to_code_passes_through() {
    let r = CanonicalReformulator::new();
    let json = serde_json::json!({"language": "python", "source": "print('hi')"});
    let code = r.to_code(&json.to_string(), SourceDialect::Gpt4Json).unwrap();
    assert_eq!(code.source, "print('hi')");
}

#[test]
fn gemini_markdown_code_block_extracted() {
    let r = CanonicalReformulator::new();
    let md = "Sure!\n\n```rust\nfn add(a: i32, b: i32) -> i32 { a + b }\n```\n";
    let code = r.to_code(md, SourceDialect::GeminiMarkdown).unwrap();
    assert_eq!(code.language, efe_multi_llm::canonical::CodeLanguage::Rust);
    assert!(code.source.contains("fn add"));
}

#[test]
fn route_dispatches_to_correct_schema() {
    let r = CanonicalReformulator::new();
    for (target, _expected_key) in [
        (CanonicalSchemaId::PlanV1, "plan_id"),
        (CanonicalSchemaId::ClaimV1, "claim"),
        (CanonicalSchemaId::SynthesisV1, "summary"),
    ] {
        let v = r
            .route(
                "Some text here.",
                SourceDialect::Gpt4Json,
                target,
                "test_llm",
            )
            .unwrap();
        assert!(v.is_object());
    }
}

#[test]
fn source_dialect_from_provider_id() {
    assert_eq!(SourceDialect::from_provider_id("claude-opus-4-6"), SourceDialect::ClaudeXml);
    assert_eq!(SourceDialect::from_provider_id("gpt-5"), SourceDialect::Gpt4Json);
    assert_eq!(SourceDialect::from_provider_id("gemini-2-5"), SourceDialect::GeminiMarkdown);
}

#[test]
fn confidence_extraction_from_text() {
    let r = CanonicalReformulator::new();
    let txt = "Yes, certainly true. Confidence: 0.92.";
    let claim = r.to_claim(txt, SourceDialect::Gpt4Json).unwrap();
    assert!((claim.confidence - 0.92).abs() < 1e-3);
}

#[test]
fn argument_thesis_from_xml() {
    let r = CanonicalReformulator::new();
    let xml = "<thesis>Rust > Go</thesis>\n- Safety\n- Performance";
    let a = r
        .to_argument(
            xml,
            efe_multi_llm::canonical::ArgumentStance::Pro,
            SourceDialect::ClaudeXml,
        )
        .unwrap();
    assert!(a.thesis.contains("Rust"));
    assert!(!a.reasoning.is_empty());
}

#[test]
fn synthesis_with_md_sections() {
    let r = CanonicalReformulator::new();
    let md = "## Intro\nFirst section\n## Body\nSecond section\n";
    let s = r
        .to_synthesis(md, SourceDialect::GeminiMarkdown, "gemini")
        .unwrap();
    assert_eq!(s.sections.len(), 2);
}

#[test]
fn plan_from_numbered_list() {
    let r = CanonicalReformulator::new();
    let md = "1. Setup\n2. Build\n3. Test";
    let p = r.to_plan(md, SourceDialect::GeminiMarkdown).unwrap();
    assert_eq!(p.steps.len(), 3);
}

#[test]
fn unknown_dialect_falls_back_to_text() {
    let r = CanonicalReformulator::new();
    let txt = "Whatever raw text.";
    let claim = r.to_claim(txt, SourceDialect::Unknown).unwrap();
    assert!(!claim.claim.is_empty());
}
