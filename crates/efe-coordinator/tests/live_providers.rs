// SPDX-License-Identifier: UNLICENSED
//! Tests live R1a+R1b — frappent les vrais Claude (Sonnet 4.6) + Gemini (Flash) + GPT (gpt-4o).
//!
//! ## Activation
//!
//! Tous les tests vérifient `EFE_LIVE_LLM=1` et early-return en `[skip]`
//! sinon.  Les clés sont attendues dans :
//! - `ANTHROPIC_API_KEY` (commence par `sk-ant-`)
//! - `GOOGLE_AI_KEY` (commence par `AIza`)
//! - `OPENAI_API_KEY` (commence par `sk-` ou `sk-proj-`)
//!
//! ```bash
//! export ANTHROPIC_API_KEY="sk-ant-..."
//! export GOOGLE_AI_KEY="AIza..."
//! export OPENAI_API_KEY="sk-proj-..."
//! EFE_LIVE_LLM=1 cargo test -p efe-coordinator --features all-providers --test live_providers -- --test-threads=1 --nocapture
//! ```
//!
//! ## Coût estimé du run complet : ~$0.10 (15+ calls cumulés sur 3 providers).

#![cfg(feature = "all-providers")]

use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::llm::{LlmCall, LlmProvider, MultiLlmBridge, ProviderRegistry};
use efe_coordinator::{ClaudeProvider, GeminiProvider, GptProvider};

fn live_enabled() -> bool {
    std::env::var("EFE_LIVE_LLM").as_deref() == Ok("1")
}

// ─── Claude (Anthropic) ──────────────────────────────────────────────

#[tokio::test]
async fn live_claude_simple_question() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + ANTHROPIC_API_KEY to enable");
        return;
    }
    let p = ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY must be set");
    let resp = p
        .call(LlmCall::simple(
            "What is 2+2? Answer with one digit only.",
            50,
        ))
        .await
        .expect("claude call");
    eprintln!("CLAUDE: text={:?} cost={} cents latency={}ms",
        resp.text, resp.cost_cents, resp.latency_ms);
    assert!(resp.text.contains('4'), "answer must contain '4', got: {}", resp.text);
    assert!(resp.cost_cents <= 5, "single short call should be ≤ 5 cents, got {}", resp.cost_cents);
    assert!(resp.latency_ms < 30_000);
    assert!(resp.tokens_generated > 0);
    assert!(resp.provider.starts_with("claude_"));
}

#[tokio::test]
async fn live_claude_streaming_works() {
    if !live_enabled() { return; }
    let p = ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY must be set");
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let task = tokio::spawn(async move {
        p.stream(
            LlmCall::simple("Count from 1 to 5. One number per line.", 100),
            tx,
        )
        .await
    });
    let mut tokens_received = Vec::new();
    while let Some(chunk) = rx.recv().await {
        tokens_received.push(chunk.token);
    }
    let resp = task.await.unwrap().expect("stream");
    eprintln!("CLAUDE STREAM: {} chunks, text={:?}", tokens_received.len(), resp.text);
    assert!(!tokens_received.is_empty(), "must receive at least 1 token chunk");
    assert!(resp.text.contains('5'));
}

#[tokio::test]
async fn live_claude_temperature_zero_deterministic() {
    if !live_enabled() { return; }
    let p = ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY");
    let mut call = LlmCall::simple("Capital of France? One word.", 30);
    call.temperature = 0.0;
    let r1 = p.call(call.clone()).await.unwrap();
    let r2 = p.call(call).await.unwrap();
    // Avec temperature=0, les réponses sont souvent (mais pas toujours) identiques.
    // On vérifie juste que les 2 contiennent "Paris".
    assert!(r1.text.to_lowercase().contains("paris"));
    assert!(r2.text.to_lowercase().contains("paris"));
}

// ─── Gemini (Google) ─────────────────────────────────────────────────

#[tokio::test]
async fn live_gemini_simple_question() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + GOOGLE_AI_KEY to enable");
        return;
    }
    let p = GeminiProvider::from_env().expect("GOOGLE_AI_KEY must be set");
    let resp = p
        .call(LlmCall::simple(
            "What is 2+2? Answer with one digit only.",
            50,
        ))
        .await
        .expect("gemini call");
    eprintln!("GEMINI: text={:?} cost={} cents latency={}ms",
        resp.text, resp.cost_cents, resp.latency_ms);
    assert!(resp.text.contains('4'), "answer must contain '4', got: {}", resp.text);
    assert!(resp.cost_cents <= 2, "Flash should be ≤ 2 cents");
    assert!(resp.latency_ms < 30_000);
    assert!(resp.tokens_generated > 0);
    assert!(resp.provider.starts_with("gemini_"));
}

#[tokio::test]
async fn live_gemini_streaming_works() {
    if !live_enabled() { return; }
    let p = GeminiProvider::from_env().expect("GOOGLE_AI_KEY");
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let task = tokio::spawn(async move {
        p.stream(
            LlmCall::simple("Count from 1 to 5. One number per line.", 100),
            tx,
        )
        .await
    });
    let mut tokens_received = Vec::new();
    while let Some(chunk) = rx.recv().await {
        tokens_received.push(chunk.token);
    }
    let resp = task.await.unwrap().expect("stream");
    eprintln!("GEMINI STREAM: {} chunks, text={:?}", tokens_received.len(), resp.text);
    assert!(!tokens_received.is_empty());
    assert!(resp.text.contains('5'));
}

// ─── Vote 2-LLM (Claude + Gemini) ────────────────────────────────────

#[tokio::test]
async fn live_vote_2llm_claude_gemini_factual() {
    if !live_enabled() { return; }
    // Construit un registre avec les 2 vrais providers.
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(ClaudeProvider::from_env().expect("claude key")));
    reg.register(Arc::new(GeminiProvider::from_env().expect("gemini key")));

    let bridge = MultiLlmBridge::new(reg);
    let claude_id = "claude_claude-sonnet-4-6".to_string();
    let gemini_id = "gemini_gemini-2.5-flash".to_string();
    let llms = vec![claude_id.clone(), gemini_id.clone()];

    let call = LlmCall::simple(
        "What is the capital of France? Answer with one word only.",
        30,
    );
    let started = std::time::Instant::now();
    let responses = bridge.call_many_ok(&llms, call).await;
    let total = started.elapsed();

    eprintln!("=== Live 2-LLM Vote ===");
    for r in &responses {
        eprintln!("  {} → {:?} (cost {} cents, {} ms)",
            r.provider, r.text.trim(), r.cost_cents, r.latency_ms);
    }
    eprintln!("Total latency (parallel): {} ms", total.as_millis());
    let total_cost: u32 = responses.iter().map(|r| r.cost_cents).sum();
    eprintln!("Total cost: {} cents", total_cost);

    // Assertions :
    assert_eq!(responses.len(), 2, "both providers should respond");
    // Les 2 LLMs DOIVENT s'accorder sur "Paris" pour une question si factuelle.
    assert!(
        responses.iter().all(|r| r.text.to_lowercase().contains("paris")),
        "consensus check: both should say Paris"
    );
    // Coût total raisonnable.
    assert!(total_cost <= 5, "should be ≤ 5 cents for a single factual question");
    // Latence parallèle ≈ max des deux (pas somme).
    assert!(total < Duration::from_secs(30));
}

// ─── Cascade speculative 2-LLM (drafter Gemini Flash → verifier Claude) ──

#[tokio::test]
async fn live_cascade_gemini_drafter_claude_verifier() {
    if !live_enabled() { return; }
    use efe_coordinator::Speculator;

    let drafter = Arc::new(GeminiProvider::from_env().expect("gemini key"));
    let verifier = Arc::new(ClaudeProvider::from_env().expect("claude key"));
    let spec = Speculator::new(drafter, verifier, 0.5);
    let call = LlmCall::simple(
        "What is the capital of France? Answer with one word.",
        30,
    );
    let started = std::time::Instant::now();
    let outcome = spec.run(call).await.expect("speculation");
    let total = started.elapsed();
    eprintln!("=== Live Cascade (Gemini drafter + Claude verifier) ===");
    eprintln!("  draft        : {:?}", outcome.draft_text.trim());
    eprintln!("  final        : {:?}", outcome.final_text.trim());
    eprintln!("  accepted     : {}", outcome.draft_accepted);
    eprintln!("  tokens_in    : accepted={} regenerated={}",
        outcome.tokens_accepted, outcome.tokens_regenerated);
    eprintln!("  latency_saved: {} ms", outcome.latency_saved_ms);
    eprintln!("  total        : {} ms", total.as_millis());
    // Tous les deux devraient répondre "Paris" — donc draft accepté.
    assert!(outcome.final_text.to_lowercase().contains("paris"));
}

// ─── GPT (OpenAI) ─────────────────────────────────────────────────────

#[tokio::test]
async fn live_gpt_simple_question() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_LLM=1 + OPENAI_API_KEY to enable");
        return;
    }
    let p = GptProvider::from_env().expect("OPENAI_API_KEY must be set");
    let resp = p
        .call(LlmCall::simple(
            "What is 2+2? Answer with one digit only.",
            50,
        ))
        .await
        .expect("gpt call");
    eprintln!("GPT: text={:?} cost={} cents latency={}ms",
        resp.text, resp.cost_cents, resp.latency_ms);
    assert!(resp.text.contains('4'), "answer must contain '4', got: {}", resp.text);
    assert!(resp.cost_cents <= 5, "single short call should be ≤ 5 cents, got {}", resp.cost_cents);
    assert!(resp.latency_ms < 30_000);
    assert!(resp.tokens_generated > 0);
    assert!(resp.provider.starts_with("gpt_"));
}

#[tokio::test]
async fn live_gpt_streaming_works() {
    if !live_enabled() { return; }
    let p = GptProvider::from_env().expect("OPENAI_API_KEY must be set");
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let task = tokio::spawn(async move {
        p.stream(
            LlmCall::simple("Count from 1 to 5. One number per line.", 100),
            tx,
        )
        .await
    });
    let mut tokens_received = Vec::new();
    while let Some(chunk) = rx.recv().await {
        tokens_received.push(chunk.token);
    }
    let resp = task.await.unwrap().expect("stream");
    eprintln!("GPT STREAM: {} chunks, text={:?}", tokens_received.len(), resp.text);
    assert!(!tokens_received.is_empty(), "must receive at least 1 token chunk");
    assert!(resp.text.contains('5'));
}

#[tokio::test]
async fn live_gpt_temperature_zero_deterministic() {
    if !live_enabled() { return; }
    let p = GptProvider::from_env().expect("OPENAI_API_KEY");
    let mut call = LlmCall::simple("Capital of France? One word.", 30);
    call.temperature = 0.0;
    let r1 = p.call(call.clone()).await.unwrap();
    let r2 = p.call(call).await.unwrap();
    assert!(r1.text.to_lowercase().contains("paris"));
    assert!(r2.text.to_lowercase().contains("paris"));
}

#[tokio::test]
async fn live_gpt_rate_limits_observable() {
    if !live_enabled() { return; }
    let p = GptProvider::from_env().expect("OPENAI_API_KEY");
    let _r = p.call(LlmCall::simple("Hi", 20)).await.expect("call");
    let rl = p.last_rate_limits();
    eprintln!("GPT rate limits:");
    eprintln!("  requests:  {:?} / {:?} (reset {:?})",
        rl.requests_remaining, rl.requests_limit, rl.requests_reset);
    eprintln!("  tokens:    {:?} / {:?} (reset {:?})",
        rl.tokens_remaining, rl.tokens_limit, rl.tokens_reset);
    eprintln!("  org:       {:?}", rl.organization);
    // OpenAI doit exposer au moins un header `x-ratelimit-*`.
    assert!(
        rl.requests_limit.is_some() || rl.tokens_limit.is_some(),
        "expected at least one rate-limit header from OpenAI"
    );
}

// ─── Vote 3-LLM RÉEL (Claude + Gemini + GPT — R1b holy grail) ────────

#[tokio::test]
async fn live_vote_3llm_real_claude_gemini_gpt() {
    if !live_enabled() { return; }
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(ClaudeProvider::from_env().expect("claude")));
    reg.register(Arc::new(GeminiProvider::from_env().expect("gemini")));
    reg.register(Arc::new(GptProvider::from_env().expect("gpt")));
    let bridge = MultiLlmBridge::new(reg);
    let llms = vec![
        "claude_claude-sonnet-4-6".to_string(),
        "gemini_gemini-2.5-flash".to_string(),
        "gpt_gpt-4o".to_string(),
    ];
    let call = LlmCall::simple(
        "What is the capital of France? Answer with the city name only.",
        30,
    );
    let started = std::time::Instant::now();
    let results = bridge.call_many_ok(&llms, call).await;
    let total = started.elapsed();
    eprintln!("=== Live 3-LLM REAL (Claude + Gemini + GPT) ===");
    for r in &results {
        eprintln!("  {} → {:?}  ({} cents, {} ms)",
            r.provider, r.text.trim(), r.cost_cents, r.latency_ms);
    }
    let total_cost: u32 = results.iter().map(|r| r.cost_cents).sum();
    eprintln!("Total cost (all 3 real providers): {total_cost} cents");
    eprintln!("Total latency (parallel): {} ms", total.as_millis());
    assert_eq!(results.len(), 3, "all 3 real providers should respond");
    // Consensus exigé : 3/3 doivent dire "Paris".
    let consensus = results.iter().filter(|r| r.text.to_lowercase().contains("paris")).count();
    assert_eq!(consensus, 3, "factual consensus expected: 3/3 say Paris, got {consensus}/3");
    assert!(total_cost <= 10, "3 LLMs on a short factual question should cost ≤ 10 cents");
    assert!(total < Duration::from_secs(30));
}

// ─── Rate-limit observability (Claude) ──

#[tokio::test]
async fn live_claude_rate_limits_observable() {
    if !live_enabled() { return; }
    let p = ClaudeProvider::from_env().expect("claude key");
    let _r = p.call(LlmCall::simple("Hi", 20)).await.expect("call");
    let rl = p.last_rate_limits();
    eprintln!("CLAUDE rate limits:");
    eprintln!("  requests:  {:?} / {:?} (reset {:?})",
        rl.requests_remaining, rl.requests_limit, rl.requests_reset);
    eprintln!("  tokens:    {:?} / {:?} (reset {:?})",
        rl.tokens_remaining, rl.tokens_limit, rl.tokens_reset);
    // Anthropic devrait avoir renvoyé ces headers — au moins requests_limit existe.
    assert!(rl.requests_limit.is_some() || rl.tokens_limit.is_some(),
        "expected at least one rate-limit header from Anthropic");
}

// ─── Observability Gemini (safety + finish_reason) ──

#[tokio::test]
async fn live_gemini_observability_captured() {
    if !live_enabled() { return; }
    let p = GeminiProvider::from_env().expect("gemini key");
    let _r = p.call(LlmCall::simple("Say hi.", 20)).await.expect("call");
    let obs = p.last_observability();
    eprintln!("GEMINI obs: finish={:?} prompt_tokens={:?} candidates_tokens={:?} safety_blocked={:?}",
        obs.last_finish_reason, obs.last_prompt_tokens,
        obs.last_candidates_tokens, obs.last_safety_blocked);
    assert!(obs.last_finish_reason.is_some());
    assert!(obs.last_prompt_tokens.is_some());
    // Pas de safety block sur "Say hi."
    assert!(obs.last_safety_blocked.is_none());
}

// ─── PII redactor (sans API call — vérifie l'intégration côté provider) ──

#[tokio::test]
async fn live_claude_pii_redactor_applied_before_send() {
    if !live_enabled() { return; }
    use std::sync::Arc as StdArc;
    let redactor: efe_coordinator::PiiRedactor = StdArc::new(|s: &str| {
        // Remplace tous les @gmail.com par [REDACTED-EMAIL].
        s.replace("@gmail.com", "[REDACTED-EMAIL]")
    });
    let p = ClaudeProvider::from_env().expect("claude").with_redactor(redactor);
    // Le call doit envoyer "alice[REDACTED-EMAIL]" au lieu de "alice@gmail.com".
    // On vérifie juste qu'il termine sans erreur — la redaction est silencieuse côté provider.
    let r = p.call(LlmCall::simple(
        "Just say OK if you can read this: alice@gmail.com",
        10,
    )).await.expect("call");
    assert!(!r.text.is_empty());
}

// ─── Health checks ───────────────────────────────────────────────────

#[tokio::test]
async fn live_claude_health() {
    if !live_enabled() { return; }
    let p = ClaudeProvider::from_env().expect("ANTHROPIC_API_KEY");
    let ok = p.health().await.expect("health call");
    assert!(ok, "Claude /messages should respond OK on minimal ping");
}

#[tokio::test]
async fn live_gemini_health() {
    if !live_enabled() { return; }
    let p = GeminiProvider::from_env().expect("GOOGLE_AI_KEY");
    let ok = p.health().await.expect("health call");
    assert!(ok, "Gemini /models should respond OK");
}

#[tokio::test]
async fn live_gpt_health() {
    if !live_enabled() { return; }
    let p = GptProvider::from_env().expect("OPENAI_API_KEY");
    let ok = p.health().await.expect("health call");
    assert!(ok, "OpenAI /v1/models should respond OK");
}
