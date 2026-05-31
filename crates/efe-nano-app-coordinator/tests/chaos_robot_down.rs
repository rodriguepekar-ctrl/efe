// SPDX-License-Identifier: UNLICENSED
//! Tests chaos — comportement sous panne.
//!
//! Couvre :
//! - Robot down (connection refused) → erreur typée
//! - Timeout → erreur typée
//! - Réponse tronquée mid-JSON → SchemaViolation
//! - Health check sur robot down
//! - 4xx vs 5xx → retry hints différents

use std::sync::Arc;
use std::time::Duration;

use efe_nano_app_coordinator::clients::{
    AnalyzerClient, LlamaServerClient, NanoRobotClient, RobotHealth,
};
use efe_nano_app_coordinator::context::{AuditStatus, AuditWriter, CallContext, InMemoryAuditWriter};
use efe_nano_app_coordinator::types::AnalyzerInput;
use efe_nano_app_coordinator::RobotError;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Robot down = port inexistant.  On choisit un port très improbable.
fn make_dead_client() -> AnalyzerClient {
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: "http://127.0.0.1:1".into(), // port 1 = reserved, refusera
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 10,
        temperature: 0.1,
        timeout: Duration::from_secs(2),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap(),
    };
    AnalyzerClient::from_helper(inner)
}

#[tokio::test]
async fn robot_down_returns_transport_error() {
    let client = make_dead_client();
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    let input = AnalyzerInput {
        text: "x".into(),
        max_entities: 20,
        source_agent: None,
    };
    let result = client.invoke(input, &ctx).await;
    assert!(
        matches!(result, Err(RobotError::Transport { .. }) | Err(RobotError::Timeout { .. })),
        "expected Transport or Timeout, got {result:?}"
    );

    // Une entrée d'audit "failed" doit être écrite.
    let entries = writer.entries().await;
    assert_eq!(entries.len(), 1);
    assert!(matches!(
        entries[0].status,
        AuditStatus::Failed | AuditStatus::Timeout
    ));
    assert!(entries[0].error_detail.is_some());
}

#[tokio::test]
async fn health_check_on_dead_robot_returns_down() {
    let client = make_dead_client();
    let health = client.health().await.unwrap();
    assert_eq!(health, RobotHealth::Down);
}

#[tokio::test]
async fn response_truncated_mid_json_returns_schema_violation() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "{ \"entities\": [{\"text\": \"App",  // tronqué
                "tokens_evaluated": 5,
                "tokens_predicted": 5
            })),
        )
        .mount(&mock)
        .await;

    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: mock.uri(),
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 10,
        temperature: 0.1,
        timeout: Duration::from_secs(3),
        http: reqwest::Client::new(),
    };
    let client = AnalyzerClient::from_helper(inner);
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    let input = AnalyzerInput {
        text: "x".into(),
        max_entities: 20,
        source_agent: None,
    };
    let result = client.invoke(input, &ctx).await;
    assert!(matches!(result, Err(RobotError::SchemaViolation { .. })));

    let entries = writer.entries().await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, AuditStatus::Failed);
}

#[tokio::test]
async fn http_4xx_should_not_retry() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad input"))
        .mount(&mock)
        .await;
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: mock.uri(),
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 10,
        temperature: 0.1,
        timeout: Duration::from_secs(2),
        http: reqwest::Client::new(),
    };
    let client = AnalyzerClient::from_helper(inner);
    let ctx = CallContext::mock();
    let input = AnalyzerInput {
        text: "x".into(),
        max_entities: 20,
        source_agent: None,
    };
    let err = client.invoke(input, &ctx).await.unwrap_err();
    assert!(!err.should_retry(), "4xx must not be retried");
}

#[tokio::test]
async fn http_5xx_should_retry() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(503).set_body_string("overloaded"))
        .mount(&mock)
        .await;
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: mock.uri(),
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 10,
        temperature: 0.1,
        timeout: Duration::from_secs(2),
        http: reqwest::Client::new(),
    };
    let client = AnalyzerClient::from_helper(inner);
    let ctx = CallContext::mock();
    let input = AnalyzerInput {
        text: "x".into(),
        max_entities: 20,
        source_agent: None,
    };
    let err = client.invoke(input, &ctx).await.unwrap_err();
    assert!(err.should_retry(), "5xx should be retried");
    // backoff doit être > 0
    assert!(err.suggested_backoff(0).as_millis() > 0);
}

#[tokio::test]
async fn audit_chain_preserved_under_mixed_success_failure() {
    let mock = MockServer::start().await;
    let ok = serde_json::json!({
      "entities": [],
      "intent": "FACTUAL",
      "sentiment": 0.0,
      "language": "en",
      "one_line_summary": "x",
      "confidence": 0.5
    });
    // Alterne success/fail via 2 mocks distincts sur paths différents — ici on
    // utilise un mock unique qui répond toujours OK, et on simule échec via
    // input invalide pour 1 sur 3.
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "content": ok.to_string(),
            "tokens_evaluated": 10,
            "tokens_predicted": 10
        })))
        .mount(&mock)
        .await;
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: mock.uri(),
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 10,
        temperature: 0.1,
        timeout: Duration::from_secs(3),
        http: reqwest::Client::new(),
    };
    let client = AnalyzerClient::from_helper(inner);
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    // 3 invocations : 2 succès, 1 échec (input invalide).
    let _ = client
        .invoke(
            AnalyzerInput {
                text: "x".into(),
                max_entities: 20,
                source_agent: None,
            },
            &ctx,
        )
        .await;
    let _ = client
        .invoke(
            AnalyzerInput {
                text: String::new(), // INVALID
                max_entities: 20,
                source_agent: None,
            },
            &ctx,
        )
        .await;
    let _ = client
        .invoke(
            AnalyzerInput {
                text: "y".into(),
                max_entities: 20,
                source_agent: None,
            },
            &ctx,
        )
        .await;

    let entries = writer.entries().await;
    // 2 (succès) — l'input invalide est rejeté AVANT l'appel donc 0 audit
    // entry pour celui-là.
    assert_eq!(entries.len(), 2);
    for e in &entries {
        assert_eq!(e.status, AuditStatus::Success);
    }
}
