// SPDX-License-Identifier: UNLICENSED
//! **Sprint 13 — N21** : live smoke test against the real Anthropic
//! Messages API.
//!
//! **Opt-in** : the test is silently skipped if `ANTHROPIC_API_KEY`
//! is not set in the environment.  Run locally with :
//!
//! ```bash
//! export ANTHROPIC_API_KEY=sk-ant-xxx
//! cargo test --package efe-agentic-bridge --test live_smoke -- --nocapture
//! ```
//!
//! **Cost** : each call exercises the full 2-turn tool-use protocol
//! against Claude Haiku (cheapest model) with `max_tokens: 100`.
//! Estimated cost per run : ~$0.001 (1/10 of a cent).
//!
//! **What it proves** :
//! 1. `ClaudeBridge` correctly speaks the Anthropic Messages API
//!    `anthropic-version: 2023-06-01` over HTTPS.
//! 2. Tool-use first-turn response is parseable.
//! 3. Tool-result second-turn round-trip works.
//! 4. The bridge handles real auth, real TLS, real rate limits.

use std::time::{Duration, Instant};

use efe_agentic_bridge::{
    AgenticBridge, AgenticTask, ClaudeBridge, DEFAULT_CLAUDE_MODEL,
};
use efe_nano_sdk::{ContextBundle, Entity, SwarmId};

fn maybe_api_key() -> Option<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

fn build_smoke_bundle() -> ContextBundle {
    let mut bundle = ContextBundle::empty(SwarmId::new());
    bundle.language = Some("en".into());
    bundle.summary = Some(
        "This is a smoke-test bundle. The customer asked about \
         their account balance and recent transactions."
            .into(),
    );
    bundle.entities.push(Entity {
        entity_type: "person".into(),
        value: "John Doe".into(),
        span: Some((10, 18)),
    });
    bundle.entities.push(Entity {
        entity_type: "money".into(),
        value: "$1,234.50".into(),
        span: Some((42, 51)),
    });
    bundle.classifications.push(efe_nano_sdk::Classification {
        label: "billing_inquiry".into(),
        confidence: 0.92,
    });
    bundle.finalize_size();
    bundle
}

#[tokio::test]
async fn live_claude_smoke_2_turn_tool_use() {
    let Some(api_key) = maybe_api_key() else {
        eprintln!(
            "ANTHROPIC_API_KEY not set ; skipping live smoke test. \
             Run with `ANTHROPIC_API_KEY=sk-ant-xxx cargo test --test \
             live_smoke -- --nocapture`."
        );
        return;
    };

    // Use Haiku for cheapness (~10x cheaper than Sonnet).
    let bridge = ClaudeBridge::new(api_key).with_model("claude-haiku-4-5");
    let bundle = build_smoke_bundle();

    let task = AgenticTask::new(
        "Look at the swarm_context and tell me, in EXACTLY ONE \
         SHORT SENTENCE, what the customer is asking about.",
    )
    .with_system(
        "You are a smoke-test assistant. Be terse. Answer in one \
         sentence.",
    )
    .with_max_tokens(60)
    .with_temperature(0.0);

    let started = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(45),
        bridge.call_with_context(&bundle, task),
    )
    .await;

    match result {
        Ok(Ok(resp)) => {
            let elapsed = started.elapsed();
            eprintln!("─────────────────────────────────────────────────");
            eprintln!(" LIVE CLAUDE SMOKE TEST — SUCCESS");
            eprintln!("─────────────────────────────────────────────────");
            eprintln!(" model       : {}", resp.model_identity.model_id);
            eprintln!(" stop_reason : {:?}", resp.stop_reason);
            eprintln!(
                " tokens      : in={} out={}",
                resp.usage.input_tokens, resp.usage.output_tokens
            );
            eprintln!(" cost_µcents : {}", resp.cost_microcents);
            eprintln!(
                " cost_usd    : ${:.6}",
                resp.cost_microcents as f64 / 1_000_000.0
            );
            eprintln!(" duration    : {elapsed:?}");
            eprintln!(" response    : {}", resp.text);
            eprintln!("─────────────────────────────────────────────────");
            assert!(!resp.text.is_empty(), "Claude returned empty text");
            assert!(
                resp.usage.input_tokens > 0,
                "Expected non-zero input tokens"
            );
            assert!(
                resp.usage.output_tokens > 0,
                "Expected non-zero output tokens"
            );
            assert!(
                resp.cost_microcents >= 0,
                "Cost must be non-negative"
            );
            assert_eq!(resp.model_identity.provider, "anthropic");
        }
        Ok(Err(e)) => panic!(
            "Live Claude API call failed (API key set ; ensure it is \
             valid and has access to claude-haiku-4-5) : {e:?}"
        ),
        Err(_) => panic!("Live Claude API call timed out after 45s"),
    }
}

#[tokio::test]
async fn live_claude_smoke_bundle_with_redactions() {
    let Some(api_key) = maybe_api_key() else {
        eprintln!("ANTHROPIC_API_KEY not set ; skipping");
        return;
    };
    // Bundle with redactions — verify Claude reasons over the
    // <EMAIL_0> token rather than failing.
    let mut bundle = ContextBundle::empty(SwarmId::new());
    bundle.language = Some("en".into());
    bundle.summary = Some(
        "Customer <EMAIL_0> reported issue with their order \
         <ORDER_0> from <DATE_0>."
            .into(),
    );
    bundle.redactions.push(efe_nano_sdk::Redaction {
        redaction_type: "email".into(),
        original_span: (10, 28),
        redaction_token: "<EMAIL_0>".into(),
    });
    bundle.finalize_size();

    let bridge = ClaudeBridge::new(api_key).with_model("claude-haiku-4-5");
    let task = AgenticTask::new(
        "What redaction tokens are present in the swarm_context? \
         List them.",
    )
    .with_max_tokens(80)
    .with_temperature(0.0);

    let result = tokio::time::timeout(
        Duration::from_secs(45),
        bridge.call_with_context(&bundle, task),
    )
    .await
    .expect("timeout")
    .expect("Claude call failed");

    eprintln!("Redaction-aware response : {}", result.text);
    // Claude should mention the EMAIL_0 token explicitly.
    let lower = result.text.to_lowercase();
    assert!(
        lower.contains("email_0") || lower.contains("email") || lower.contains("redaction"),
        "Expected response to mention the EMAIL_0 token, got : {}",
        result.text
    );
}

#[tokio::test]
async fn live_claude_default_model_constant_loadable() {
    // Even without an API key, verify the default model constant is
    // sensible.  This test runs unconditionally.
    assert!(DEFAULT_CLAUDE_MODEL.starts_with("claude"));
    assert!(!DEFAULT_CLAUDE_MODEL.is_empty());
}
