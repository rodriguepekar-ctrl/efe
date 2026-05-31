// SPDX-License-Identifier: UNLICENSED
//! Tests phase — workflows nominaux via wiremock (pas de vrai llama-server).
//!
//! Couvre :
//! - Workflow Analyzer : prompt → JSON parsé → audit entry écrite
//! - Workflow Filter (pure Rust, mock embedder) end-to-end
//! - Latence audit chain : 1 entrée par invocation, exactement

use std::sync::Arc;

use efe_nano_app_coordinator::clients::{
    AnalyzerClient, FilterClient, LlamaServerClient, NanoRobotClient,
};
use efe_nano_app_coordinator::context::{AuditStatus, AuditWriter, CallContext, InMemoryAuditWriter};
use efe_nano_app_coordinator::types::*;
use std::collections::HashMap;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Helper : crée un AnalyzerClient pointé vers un MockServer.
async fn make_analyzer(mock_server: &MockServer) -> AnalyzerClient {
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: mock_server.uri(),
        grammar: String::new(),
        system_prompt: "test prompt".into(),
        max_tokens: 512,
        temperature: 0.1,
        timeout: std::time::Duration::from_secs(3),
        http: reqwest::Client::new(),
    };
    AnalyzerClient::from_helper(inner)
}

/// Payload JSON simulé par le mock llama-server.
fn analyzer_response_ok() -> serde_json::Value {
    let content = serde_json::json!({
      "entities": [
        {"text": "Apple", "kind": "ORG", "start_char": 0, "end_char": 5, "confidence": 0.98},
        {"text": "$90B", "kind": "MONEY", "start_char": 15, "end_char": 19, "confidence": 0.99}
      ],
      "intent": "FACTUAL",
      "sentiment": 0.2,
      "language": "en",
      "one_line_summary": "Apple Q3 revenue $90B",
      "confidence": 0.95
    });
    serde_json::json!({
        "content": content.to_string(),
        "tokens_evaluated": 14,
        "tokens_predicted": 50
    })
}

#[tokio::test]
async fn analyzer_happy_path_returns_typed_output() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(analyzer_response_ok()))
        .mount(&mock)
        .await;

    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());
    let client = make_analyzer(&mock).await;

    let input = AnalyzerInput {
        text: "Apple reported $90B revenue Q3.".into(),
        max_entities: 20,
        source_agent: Some("test".into()),
    };
    let out = client.invoke(input, &ctx).await.expect("invoke OK");

    assert_eq!(out.entities.len(), 2);
    assert_eq!(out.entities[0].text, "Apple");
    assert_eq!(out.entities[0].kind, EntityKind::Org);
    assert_eq!(out.intent, IntentClass::Factual);
    assert_eq!(out.language, IsoLang::En);

    // 1 entrée d'audit exactement, status Success.
    let entries = writer.entries().await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].robot_id, "analyzer");
    assert_eq!(entries[0].schema_version, "1.0.0");
    assert_eq!(entries[0].status, AuditStatus::Success);
    assert_eq!(entries[0].tokens_prompt, 14);
    assert_eq!(entries[0].tokens_completion, 50);
}

#[tokio::test]
async fn analyzer_invalid_json_returns_schema_violation() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "{ \"entities\": invalid json here",
                "tokens_evaluated": 5,
                "tokens_predicted": 5
            })),
        )
        .mount(&mock)
        .await;

    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());
    let client = make_analyzer(&mock).await;

    let input = AnalyzerInput {
        text: "any".into(),
        max_entities: 20,
        source_agent: None,
    };
    let result = client.invoke(input, &ctx).await;
    assert!(matches!(
        result,
        Err(efe_nano_app_coordinator::RobotError::SchemaViolation { .. })
    ));

    // Une entrée d'audit "failed" doit être écrite.
    let entries = writer.entries().await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, AuditStatus::Failed);
}

#[tokio::test]
async fn analyzer_http_500_returns_http_status_error() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(500).set_body_string("overloaded"))
        .mount(&mock)
        .await;

    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());
    let client = make_analyzer(&mock).await;

    let input = AnalyzerInput {
        text: "x".into(),
        max_entities: 20,
        source_agent: None,
    };
    let result = client.invoke(input, &ctx).await;
    match result {
        Err(efe_nano_app_coordinator::RobotError::HttpStatus { status, .. }) => {
            assert_eq!(status, 500);
        }
        other => panic!("expected HttpStatus, got {other:?}"),
    }
    // Audit failure recorded.
    let entries = writer.entries().await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].status, AuditStatus::Failed);
}

#[tokio::test]
async fn analyzer_invalid_input_rejected_before_call() {
    let mock = MockServer::start().await;
    // Mock pas configuré → si on l'appelle, ça crash. On veut que NON.
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());
    let client = make_analyzer(&mock).await;

    let input = AnalyzerInput {
        text: String::new(), // INVALID
        max_entities: 20,
        source_agent: None,
    };
    let result = client.invoke(input, &ctx).await;
    assert!(matches!(
        result,
        Err(efe_nano_app_coordinator::RobotError::SemanticViolation { .. })
    ));
    // Aucune entrée d'audit puisqu'on n'a pas atteint llama-server.
    let entries = writer.entries().await;
    assert_eq!(entries.len(), 0);
}

#[tokio::test]
async fn filter_workflow_end_to_end() {
    use efe_nano_app_coordinator::clients::filter::MockEmbedder;
    use efe_nano_app_coordinator::clients::EmbeddingBackend;

    let client = FilterClient::mock();
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    // Setup : capability = embedding du message → similarity max.
    let embedder = MockEmbedder::default();
    let cap_claude = embedder.embed("financial").await.unwrap();
    let cap_gemini = embedder.embed("totally other topic").await.unwrap();

    let mut caps = HashMap::new();
    caps.insert("claude".into(), cap_claude);
    caps.insert("gemini".into(), cap_gemini);

    let mut budgets = HashMap::new();
    budgets.insert("claude".into(), 10_000);
    budgets.insert("gemini".into(), 10_000);

    let input = FilterInput {
        message_summary: "financial".into(),
        available_agents: vec!["claude".into(), "gemini".into()],
        agent_capabilities: caps,
        context_budgets: budgets,
    };
    let out = client.invoke(input, &ctx).await.unwrap();
    assert_eq!(out.routing.len(), 2);

    // claude doit être Send (similarity = 1.0 avec sa propre capability)
    let claude_decision = &out
        .routing
        .iter()
        .find(|(a, _)| a == "claude")
        .unwrap()
        .1;
    assert!(matches!(claude_decision, RoutingDecision::Send { .. }));

    // Audit chain.
    let entries = writer.entries().await;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].robot_id, "filter");
}

#[tokio::test]
async fn multiple_invocations_chain_audit_entries() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(analyzer_response_ok()))
        .mount(&mock)
        .await;

    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());
    let client = make_analyzer(&mock).await;

    for i in 0..5 {
        let input = AnalyzerInput {
            text: format!("text variant {i}"),
            max_entities: 20,
            source_agent: None,
        };
        let _ = client.invoke(input, &ctx).await.unwrap();
    }

    let entries = writer.entries().await;
    assert_eq!(entries.len(), 5);
    // Tous le même tenant + swarm.
    let first = &entries[0];
    for e in &entries {
        assert_eq!(e.tenant_id, first.tenant_id);
        assert_eq!(e.swarm_id, first.swarm_id);
        assert_eq!(e.robot_id, "analyzer");
        assert_eq!(e.status, AuditStatus::Success);
    }
}
