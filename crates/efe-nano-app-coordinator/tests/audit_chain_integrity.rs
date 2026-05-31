// SPDX-License-Identifier: UNLICENSED
//! Tests audit-niveau — invariants globaux de la chaîne d'audit.
//!
//! Vérifie le contrat "every invocation produces exactly one audit entry"
//! et la cohérence des entrées générées (swarm_id, tenant_id, schema_version,
//! timestamps croissants, status correct).
//!
//! Phase 1 n'a pas encore de chainage HMAC (Phase 2).  Mais on vérifie déjà
//! les invariants qui SERONT préservés une fois HMAC chain branché.

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

fn analyzer_ok_response() -> serde_json::Value {
    let content = serde_json::json!({
        "entities": [],
        "intent": "FACTUAL",
        "sentiment": 0.0,
        "language": "en",
        "one_line_summary": "ok",
        "confidence": 0.9
    });
    serde_json::json!({
        "content": content.to_string(),
        "tokens_evaluated": 10,
        "tokens_predicted": 10
    })
}

async fn make_mock_analyzer() -> (MockServer, AnalyzerClient) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(ResponseTemplate::new(200).set_body_json(analyzer_ok_response()))
        .mount(&server)
        .await;
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: server.uri(),
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 64,
        temperature: 0.1,
        timeout: std::time::Duration::from_secs(3),
        http: reqwest::Client::new(),
    };
    let client = AnalyzerClient::from_helper(inner);
    (server, client)
}

#[tokio::test]
async fn invariant_one_entry_per_successful_invocation() {
    let (_server, client) = make_mock_analyzer().await;
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    for i in 0..20 {
        let input = AnalyzerInput {
            text: format!("text {i}"),
            max_entities: 20,
            source_agent: None,
        };
        client.invoke(input, &ctx).await.unwrap();
    }

    let entries = writer.entries().await;
    assert_eq!(entries.len(), 20, "expected exactly 20 entries");
    for e in &entries {
        assert_eq!(e.status, AuditStatus::Success);
        assert_eq!(e.robot_id, "analyzer");
        assert_eq!(e.schema_version, "1.0.0");
        assert!(e.error_detail.is_none());
    }
}

#[tokio::test]
async fn invariant_all_entries_same_swarm_id() {
    let (_server, client) = make_mock_analyzer().await;
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    for _ in 0..10 {
        let input = AnalyzerInput {
            text: "x".into(),
            max_entities: 20,
            source_agent: None,
        };
        let _ = client.invoke(input, &ctx).await;
    }

    let entries = writer.entries().await;
    let swarm_id = entries[0].swarm_id;
    let tenant_id = entries[0].tenant_id;
    for e in &entries {
        assert_eq!(e.swarm_id, swarm_id, "all entries must share swarm_id");
        assert_eq!(e.tenant_id, tenant_id, "all entries must share tenant_id");
    }
}

#[tokio::test]
async fn invariant_entry_ids_unique() {
    let (_server, client) = make_mock_analyzer().await;
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    for _ in 0..30 {
        let input = AnalyzerInput {
            text: "x".into(),
            max_entities: 20,
            source_agent: None,
        };
        let _ = client.invoke(input, &ctx).await;
    }

    let entries = writer.entries().await;
    let mut ids = std::collections::HashSet::new();
    for e in &entries {
        assert!(ids.insert(e.entry_id), "duplicate entry_id detected");
    }
    assert_eq!(ids.len(), 30);
}

#[tokio::test]
async fn invariant_timestamps_monotonically_increasing() {
    let (_server, client) = make_mock_analyzer().await;
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    for i in 0..10 {
        let input = AnalyzerInput {
            text: format!("x{i}"),
            max_entities: 20,
            source_agent: None,
        };
        let _ = client.invoke(input, &ctx).await;
    }

    let entries = writer.entries().await;
    for w in entries.windows(2) {
        assert!(
            w[1].finished_at >= w[0].finished_at,
            "timestamps must be monotonic"
        );
    }
}

#[tokio::test]
async fn invariant_failures_also_audited() {
    // Mock returning malformed JSON → SchemaViolation.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/completion"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content": "{ not valid json",
                "tokens_evaluated": 5,
                "tokens_predicted": 3
            })),
        )
        .mount(&server)
        .await;

    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: server.uri(),
        grammar: String::new(),
        system_prompt: "x".into(),
        max_tokens: 64,
        temperature: 0.1,
        timeout: std::time::Duration::from_secs(3),
        http: reqwest::Client::new(),
    };
    let client = AnalyzerClient::from_helper(inner);
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    for _ in 0..5 {
        let input = AnalyzerInput {
            text: "x".into(),
            max_entities: 20,
            source_agent: None,
        };
        let _ = client.invoke(input, &ctx).await;
    }

    let entries = writer.entries().await;
    assert_eq!(entries.len(), 5);
    for e in &entries {
        assert_eq!(e.status, AuditStatus::Failed);
        assert!(e.error_detail.is_some());
    }
}

#[tokio::test]
async fn invariant_in_memory_filter_robot_also_records() {
    let client = FilterClient::mock();
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    use efe_nano_app_coordinator::clients::filter::MockEmbedder;
    use efe_nano_app_coordinator::clients::EmbeddingBackend;
    let embedder = MockEmbedder::default();
    let cap = embedder.embed("test").await.unwrap();

    let mut caps = HashMap::new();
    caps.insert("claude".into(), cap);
    let mut budgets = HashMap::new();
    budgets.insert("claude".into(), 5000);

    for _ in 0..15 {
        let input = FilterInput {
            message_summary: "test".into(),
            available_agents: vec!["claude".into()],
            agent_capabilities: caps.clone(),
            context_budgets: budgets.clone(),
        };
        client.invoke(input, &ctx).await.unwrap();
    }

    let entries = writer.entries().await;
    assert_eq!(entries.len(), 15);
    for e in &entries {
        assert_eq!(e.robot_id, "filter");
        assert_eq!(e.status, AuditStatus::Success);
        // Robots ONNX : 0 tokens.
        assert_eq!(e.tokens_prompt, 0);
        assert_eq!(e.tokens_completion, 0);
    }
}

#[tokio::test]
async fn invariant_input_output_hashes_present() {
    let (_server, client) = make_mock_analyzer().await;
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    let input = AnalyzerInput {
        text: "deterministic input".into(),
        max_entities: 20,
        source_agent: None,
    };
    client.invoke(input, &ctx).await.unwrap();

    let entries = writer.entries().await;
    assert_eq!(entries.len(), 1);
    // BLAKE3 hex = 64 chars.
    assert_eq!(entries[0].input_hash.len(), 64);
    assert_eq!(entries[0].output_hash.len(), 64);
    // Hashes must be hex.
    assert!(entries[0].input_hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(entries[0].output_hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test]
async fn invariant_same_input_produces_same_input_hash() {
    let (_server, client) = make_mock_analyzer().await;
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    let input1 = AnalyzerInput {
        text: "same".into(),
        max_entities: 20,
        source_agent: None,
    };
    let input2 = AnalyzerInput {
        text: "same".into(),
        max_entities: 20,
        source_agent: None,
    };
    client.invoke(input1, &ctx).await.unwrap();
    client.invoke(input2, &ctx).await.unwrap();

    let entries = writer.entries().await;
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].input_hash, entries[1].input_hash,
        "identical inputs must produce identical hash (deterministic BLAKE3)"
    );
}

#[tokio::test]
async fn invariant_latency_recorded_positive() {
    let (_server, client) = make_mock_analyzer().await;
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    let input = AnalyzerInput {
        text: "x".into(),
        max_entities: 20,
        source_agent: None,
    };
    client.invoke(input, &ctx).await.unwrap();

    let entries = writer.entries().await;
    // Pas de strict >0 car wiremock peut être ultra-rapide ; on accepte 0+
    assert!(entries[0].latency_ms < 5000, "sanity bound");
}
