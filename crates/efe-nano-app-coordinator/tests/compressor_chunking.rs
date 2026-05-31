// SPDX-License-Identifier: UNLICENSED
//! **Levier 3 — compression long-contexte (map-reduce)** end-to-end via wiremock.
//!
//! Prouve que `CompressorClient::invoke` :
//! - sur un input LONG (> seuil de chunking) → fait PLUSIEURS appels `/completion`
//!   (map-reduce) et renvoie un résumé valide ;
//! - sur un input COURT → fait exactement 1 appel (chemin historique) ;
//! - quand un morceau dégénère (placeholder), ne propage JAMAIS le placeholder.
//!
//! SLM factice (wiremock) → déterministe, offline, pas de vrai llama-server.

use std::sync::Arc;
use std::time::Duration;

use efe_nano_app_coordinator::clients::{CompressorClient, LlamaServerClient, NanoRobotClient};
use efe_nano_app_coordinator::context::{CallContext, InMemoryAuditWriter};
use efe_nano_app_coordinator::types::{CompressionStyle, CompressorInput};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// `CompressorClient` pointé vers un MockServer (pas de vrai SLM).
fn compressor_at(uri: String) -> CompressorClient {
    let inner = LlamaServerClient {
        id: "compressor",
        schema_version: "1.0.0",
        endpoint: uri,
        grammar: String::new(),
        system_prompt: "test compressor".into(),
        max_tokens: 256,
        temperature: 0.2,
        timeout: Duration::from_secs(5),
        http: reqwest::Client::new(),
    };
    CompressorClient::from_helper(inner)
}

/// Réponse llama-server simulée portant un `CompressorOutput` canonique.
fn canned_ok() -> serde_json::Value {
    let content = serde_json::json!({
        "summary": "Synthèse du morceau : Paris, capitale de la France.",
        "word_count": 8,
        "preserved": [],
        "omitted_for_brevity": [],
        "compression_ratio": 6.0
    });
    serde_json::json!({
        "content": content.to_string(),
        "tokens_evaluated": 80,
        "tokens_predicted": 12
    })
}

#[tokio::test]
async fn long_input_triggers_chunked_map_reduce() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_ok()))
        .mount(&mock)
        .await;

    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer);
    let client = compressor_at(mock.uri());

    // ~4400 chars ≈ 1100 tokens, bien au-dessus du seuil (300) → map-reduce.
    let long_text = "Le contexte administratif de la République française. ".repeat(80);
    let input = CompressorInput {
        text: long_text,
        target_words: 40,
        must_preserve: vec![],
        style: CompressionStyle::Extractive,
    };

    let out = client
        .invoke(input, &ctx)
        .await
        .expect("map-reduce doit réussir");
    assert!(!out.summary.trim().is_empty(), "résumé final non vide");
    assert!(
        out.summary.contains("Synthèse") || out.summary.contains("Paris"),
        "résumé final cohérent, got: {:?}",
        out.summary
    );

    let n = mock.received_requests().await.unwrap().len();
    assert!(
        n >= 2,
        "le map-reduce doit faire ≥ 2 appels SLM (chunking), got {n}"
    );
}

#[tokio::test]
async fn short_input_uses_a_single_call() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(canned_ok()))
        .mount(&mock)
        .await;

    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer);
    let client = compressor_at(mock.uri());

    // > 50 chars (contrainte validate) mais bien sous le seuil → 1 appel.
    let input = CompressorInput {
        text: "Un texte court à compresser, nettement sous le seuil de chunking.".into(),
        target_words: 20,
        must_preserve: vec![],
        style: CompressionStyle::Extractive,
    };

    client
        .invoke(input, &ctx)
        .await
        .expect("single-call doit réussir");
    let n = mock.received_requests().await.unwrap().len();
    assert_eq!(n, 1, "input court = exactement 1 appel SLM, got {n}");
}

#[tokio::test]
async fn degenerate_chunk_never_propagates_placeholder() {
    // Le mock renvoie un placeholder (mode d'échec réel du 1B sur input long).
    // Le map-reduce doit dégrader (extrait tronqué du brut) et NE JAMAIS laisser
    // "<the compressed text>" atteindre la sortie finale.
    let mock = MockServer::start().await;
    let content = serde_json::json!({
        "summary": "<the compressed text>",
        "word_count": 3,
        "preserved": [],
        "omitted_for_brevity": [],
        "compression_ratio": 5.0
    });
    let body = serde_json::json!({
        "content": content.to_string(),
        "tokens_evaluated": 50,
        "tokens_predicted": 4
    });
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&mock)
        .await;

    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer);
    let client = compressor_at(mock.uri());

    let long_text = "Phrase de contexte administratif français répétée. ".repeat(80);
    let input = CompressorInput {
        text: long_text,
        target_words: 40,
        must_preserve: vec![],
        style: CompressionStyle::Extractive,
    };

    let out = client
        .invoke(input, &ctx)
        .await
        .expect("doit dégrader proprement, pas échouer");
    assert!(
        !out.summary.contains("<the compressed text>"),
        "le placeholder dégénéré ne doit JAMAIS atteindre la sortie finale, got: {:?}",
        out.summary
    );
    assert!(!out.summary.trim().is_empty(), "résumé dégradé non vide");
}
