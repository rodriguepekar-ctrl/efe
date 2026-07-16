// SPDX-License-Identifier: UNLICENSED
//! **Fil B.1 — tests LIVE du reformulateur SLM** (ignorés par défaut).
//!
//! Prérequis : un `llama-server` servant Qwen3-0.6B (profil multi-llm) sur
//! `EFE_SLM_REFORMULATOR_URL` (défaut `http://127.0.0.1:8091`). Lancer :
//!
//! ```text
//! EFE_SLM_REFORMULATOR_URL=http://127.0.0.1:8091 \
//!   cargo test -p efe-multi-llm --test live_slm_reformulator -- --ignored --nocapture
//! ```
//!
//! C'est ICI (et seulement ici) que la revendication « un petit modèle traduit
//! réellement les dialectes entre Claude/GPT/Gemini » est exercée de bout en
//! bout : chaque hop doit revenir estampillé `Slm` (pas `SlmDegradedToRule`)
//! et désérialiser dans le type canonique attendu. Hors ligne, cette garantie
//! n'existe PAS — ne jamais la revendiquer sans avoir fait tourner ces tests.

use std::sync::Arc;

use efe_multi_llm::canonical::{
    ArgumentStance, CanonicalArgument, CanonicalClaim, CanonicalCode, CanonicalPlan,
    CanonicalSchemaId, CanonicalSynthesis,
};
use efe_multi_llm::reformulator_ext::SourceDialect;
use efe_multi_llm::{Reformulator, ReformulatorEngine, SlmReformulator};
use efe_nano_app_coordinator::ReformulatorClient;

fn slm() -> SlmReformulator {
    let url = std::env::var("EFE_SLM_REFORMULATOR_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8091".to_string());
    let client = ReformulatorClient::from_assets_multillm(url).expect("assets multillm");
    SlmReformulator::from_client(Arc::new(client))
}

#[tokio::test]
#[ignore = "exige un llama-server Qwen3-0.6B (EFE_SLM_REFORMULATOR_URL)"]
async fn live_plan_from_claude_xml_is_slm_stamped() {
    let raw = "<plan><step>Parse the invoice PDF</step><step>Extract line items</step>\
               <step>Reconcile totals</step></plan>";
    let out = slm()
        .to_canonical(raw, SourceDialect::ClaudeXml, CanonicalSchemaId::PlanV1, "claude_live")
        .await
        .expect("hop plan");
    assert_eq!(out.engine, ReformulatorEngine::Slm, "doit être le SLM, pas la dégradation");
    let plan: CanonicalPlan = serde_json::from_value(out.value).expect("CanonicalPlan");
    assert!(!plan.steps.is_empty());
    println!("LIVE plan_v1 OK — {} steps, title={:?}", plan.steps.len(), plan.title);
}

#[tokio::test]
#[ignore = "exige un llama-server Qwen3-0.6B (EFE_SLM_REFORMULATOR_URL)"]
async fn live_code_from_gemini_markdown_is_slm_stamped() {
    let raw = "Here is the implementation:\n```rust\nfn add(a: u32, b: u32) -> u32 { a + b }\n```";
    let out = slm()
        .to_canonical(raw, SourceDialect::GeminiMarkdown, CanonicalSchemaId::CodeV1, "gemini_live")
        .await
        .expect("hop code");
    assert_eq!(out.engine, ReformulatorEngine::Slm);
    let code: CanonicalCode = serde_json::from_value(out.value).expect("CanonicalCode");
    assert!(code.source.contains("fn add"), "le code doit venir du raw_text : {}", code.source);
    println!("LIVE code_v1 OK — lang={:?}", code.language);
}

#[tokio::test]
#[ignore = "exige un llama-server Qwen3-0.6B (EFE_SLM_REFORMULATOR_URL)"]
async fn live_claim_from_gemini_markdown_is_slm_stamped() {
    let raw = "The Fed held rates steady.[^1]\n\n[^1]: FOMC statement, June.";
    let out = slm()
        .to_canonical(raw, SourceDialect::GeminiMarkdown, CanonicalSchemaId::ClaimV1, "gemini_live")
        .await
        .expect("hop claim");
    assert_eq!(out.engine, ReformulatorEngine::Slm);
    let claim: CanonicalClaim = serde_json::from_value(out.value).expect("CanonicalClaim");
    assert!(!claim.claim.is_empty());
    assert!((0.0..=1.0).contains(&claim.confidence));
    println!("LIVE claim_v1 OK — claim={:?} conf={}", claim.claim, claim.confidence);
}

#[tokio::test]
#[ignore = "exige un llama-server Qwen3-0.6B (EFE_SLM_REFORMULATOR_URL)"]
async fn live_argument_carries_stance_hint() {
    let raw = "<thesis>Rates should stay unchanged</thesis>\n- inflation is sticky\n- growth on target";
    let out = slm()
        .to_argument_value(raw, ArgumentStance::Pro, SourceDialect::ClaudeXml)
        .await
        .expect("hop argument");
    assert_eq!(out.engine, ReformulatorEngine::Slm);
    let arg: CanonicalArgument = serde_json::from_value(out.value).expect("CanonicalArgument");
    assert!(!arg.reasoning.is_empty());
    println!("LIVE argument_v1 OK — stance={:?} thesis={:?}", arg.stance, arg.thesis);
}

#[tokio::test]
#[ignore = "exige un llama-server Qwen3-0.6B (EFE_SLM_REFORMULATOR_URL)"]
async fn live_synthesis_from_gpt_json_is_slm_stamped() {
    let raw = r#"{"summary": "Both agree the plan works; timelines differ.",
                  "notes": "claude says 2 sprints, gemini says 3"}"#;
    let out = slm()
        .to_canonical(raw, SourceDialect::Gpt4Json, CanonicalSchemaId::SynthesisV1, "gpt_live")
        .await
        .expect("hop synthesis");
    assert_eq!(out.engine, ReformulatorEngine::Slm);
    let s: CanonicalSynthesis = serde_json::from_value(out.value).expect("CanonicalSynthesis");
    assert!(!s.summary.is_empty());
    println!("LIVE synthesis_v1 OK — summary={:?}", s.summary);
}
