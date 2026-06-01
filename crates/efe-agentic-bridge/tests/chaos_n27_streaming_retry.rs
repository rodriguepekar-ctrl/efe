// SPDX-License-Identifier: UNLICENSED
//! **Sprint 19 — N27** : chaos tests — F16-F25 + selected
//! adversarial scenarios.
//! See architecture § 11, § 16.3.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use efe_agentic_bridge::{
    AgenticEvent, AgenticTask, BridgeError, ClaudeBridge, ExponentialBackoff, StreamConfig,
};
use efe_llm_provenance::{
    DefaultLlmProvenanceCollector, InMemoryLlmProvenanceStore, LlmProvenanceCollector,
    LlmProvenanceStore,
};
use efe_nano_sdk::{ContextBundle, Entity, ModelIdentity, SwarmId};

fn small_bundle() -> ContextBundle {
    let mut b = ContextBundle::empty(SwarmId::new());
    b.language = Some("en".into());
    b.entities.push(Entity {
        entity_type: "person".into(),
        value: "y".into(),
        span: Some((0, 1)),
    });
    b
}

fn sse_body(events: Vec<&str>) -> Vec<u8> {
    let mut buf = Vec::new();
    for ev in events {
        buf.extend_from_slice(ev.as_bytes());
        buf.extend_from_slice(b"\n\n");
    }
    buf
}

fn sse_single_turn(input: u32, output: u32, text: &str) -> Vec<u8> {
    sse_body(vec![
        &format!(
            r#"event: message_start
data: {{"type":"message_start","message":{{"id":"msg","usage":{{"input_tokens":{input},"output_tokens":1}}}}}}"#
        ),
        r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        &format!(
            r#"event: content_block_delta
data: {{"type":"content_block_delta","index":0,"delta":{{"type":"text_delta","text":"{text}"}}}}"#
        ),
        r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}"#,
        &format!(
            r#"event: message_delta
data: {{"type":"message_delta","delta":{{"stop_reason":"end_turn","stop_sequence":null}},"usage":{{"output_tokens":{output}}}}}"#
        ),
        r#"event: message_stop
data: {"type":"message_stop"}"#,
    ])
}

fn fast_policy() -> ExponentialBackoff {
    ExponentialBackoff {
        base: Duration::from_millis(10),
        max_per_retry: Duration::from_millis(30),
        max_total_wall: Duration::from_secs(5),
        jitter_pct: 0.0,
        ..Default::default()
    }
}

fn fast_stream() -> StreamConfig {
    StreamConfig {
        read_timeout: Duration::from_millis(500),
        total_timeout: Duration::from_secs(10),
        connect_timeout: Duration::from_secs(2),
        channel_cap: 8,
    }
}

async fn bridge_with_provenance(server: &MockServer) -> ClaudeBridge {
    let store: Arc<dyn LlmProvenanceStore> = Arc::new(InMemoryLlmProvenanceStore::new());
    let registry = Arc::new(efe_llm_provenance::InMemoryModelRegistry::new());
    {
        use efe_llm_provenance::ModelRegistry;
        let mut model = efe_llm_provenance::RegisteredModel::hosted(
            ModelIdentity::hosted("anthropic", "claude-sonnet-4-6", "2023-06-01"),
            0.0,
            0.0,
            1,
        );
        model.dev_skip_hash_check = true;
        let _ = registry.register(model).await;
    }
    let collector = DefaultLlmProvenanceCollector::new(
        store,
        registry as Arc<dyn efe_llm_provenance::ModelRegistry>,
    );
    ClaudeBridge::with_endpoint("test-key", server.uri())
        .with_retry_policy(fast_policy())
        .with_stream_config(fast_stream())
        .with_provenance(collector as Arc<dyn LlmProvenanceCollector>)
}

async fn drain(stream: futures::stream::BoxStream<'_, AgenticEvent>) -> Vec<AgenticEvent> {
    futures::pin_mut!(stream);
    let mut out = Vec::new();
    while let Some(ev) = stream.next().await {
        out.push(ev);
    }
    out
}

// ═════════════════════════════════════════════════════════════════════
//  F16 — 529 Overloaded → retry succeeds
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f16_529_overloaded_retry_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(529).set_body_string(
            r#"{"error":{"type":"overloaded_error","message":"x"}}"#,
        ))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse_single_turn(5, 2, "ok"), "text/event-stream"),
        )
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    let done = events
        .iter()
        .find_map(|e| match e {
            AgenticEvent::Done { retry_count, .. } => Some(*retry_count),
            _ => None,
        });
    assert_eq!(done, Some(2), "expected 2 retries before success");
}

// ═════════════════════════════════════════════════════════════════════
//  F17 — 503 mid-deploy → retry succeeds
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f17_503_retry_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse_single_turn(5, 2, "ok"), "text/event-stream"),
        )
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    let done = events
        .iter()
        .any(|e| matches!(e, AgenticEvent::Done { retry_count: 1, .. }));
    assert!(done, "expected retry_count=1 Done");
}

// ═════════════════════════════════════════════════════════════════════
//  F19 — 400 → no retry, immediate error
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f19_400_no_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_string(r#"{"error":{"type":"bad_request","message":"x"}}"#),
        )
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let started = std::time::Instant::now();
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    let elapsed = started.elapsed();
    // Should be fast — no retry sleep.
    assert!(elapsed < Duration::from_millis(500), "400 took {elapsed:?}");
    // Error event surfaced.
    assert!(events.iter().any(|e| matches!(e, AgenticEvent::Error(_))));
    // No retry attempts.
    assert_eq!(bridge.stream_retry_attempts_count(), 0);
}

// ═════════════════════════════════════════════════════════════════════
//  F (renamed) — truncated body before message_stop → PrematureEof
//
//  **Audit-pass-3 fix Bug #7** : this test was originally labeled
//  `chaos_n27_f23_total_timeout_exceeded` but it actually exercises
//  the PrematureEof code path, not TotalTimeout.  Wiremock cannot
//  trivially simulate a body-mid-hang ; testing TotalTimeout
//  realistically requires a hand-rolled TCP listener.  See
//  `chaos_n27_f_truncated_body_aborts` for the renamed test.
//  A future test (`chaos_n27_f23_total_timeout_with_hanging_server`)
//  would spin a `tokio::net::TcpListener` that writes headers then
//  hangs.  Deferred to N28+ (architecture § 16.4).
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f_truncated_body_aborts_with_premature_eof() {
    let server = MockServer::start().await;
    let truncated = sse_body(vec![
        r#"event: message_start
data: {"type":"message_start","message":{"id":"m","usage":{"input_tokens":5,"output_tokens":1}}}"#,
        r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(truncated, "text/event-stream"),
        )
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    let n_errors = events
        .iter()
        .filter(|e| matches!(e, AgenticEvent::Error(_)))
        .count();
    assert_eq!(n_errors, 1, "expected one terminal Error");
    // **Audit-pass-3** : verify the error kind is actually PrematureEof.
    let prem = events.iter().any(|e| {
        matches!(
            e,
            AgenticEvent::Error(BridgeError::PrematureEof)
        )
    });
    assert!(prem, "expected PrematureEof error kind specifically");
    // **Audit-pass-3 + Bug #4 follow-up** : abort_reason set on
    // the snapshot so stream_aborts counter increments.
    assert!(
        bridge.stream_aborts_count() >= 1,
        "expected stream_aborts >= 1, got {}",
        bridge.stream_aborts_count()
    );
}

// ═════════════════════════════════════════════════════════════════════
//  F24 — corrupt tool-use JSON
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f24_corrupt_tool_use_json() {
    let server = MockServer::start().await;
    // Turn 1 with broken tool-use JSON.
    let bad_body = sse_body(vec![
        r#"event: message_start
data: {"type":"message_start","message":{"id":"m","usage":{"input_tokens":5,"output_tokens":1}}}"#,
        r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tu","name":"swarm_context","input":{}}}"#,
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{not valid"}}"#,
        r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}"#,
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(bad_body, "text/event-stream"),
        )
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    // Error surfaced.
    let error_kind = events.iter().find_map(|e| match e {
        AgenticEvent::Error(BridgeError::ToolUseJsonInvalid { .. }) => Some("tool_use_json_invalid"),
        AgenticEvent::Error(_) => Some("other_error"),
        _ => None,
    });
    assert_eq!(error_kind, Some("tool_use_json_invalid"));
}

// ═════════════════════════════════════════════════════════════════════
//  Network RST pre-stream → retries
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f_total_wall_exceeded_with_5xx_loop() {
    let server = MockServer::start().await;
    // Always 503 → exhaust retries (MAX_RETRIES = 3).
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    assert!(events.iter().any(|e| matches!(e, AgenticEvent::Error(_))));
    assert!(bridge.stream_retry_attempts_count() <= 3);
}

// ═════════════════════════════════════════════════════════════════════
//  Wrong content-type → error (no streaming wire)
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f_wrong_content_type_no_streaming() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"unexpected":"shape"}"#),
        )
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    let invalid = events.iter().any(|e| {
        matches!(
            e,
            AgenticEvent::Error(BridgeError::InvalidResponseType { .. })
        )
    });
    assert!(invalid, "expected InvalidResponseType error");
}

// ═════════════════════════════════════════════════════════════════════
//  Premature EOF (no message_stop) → error
// ═════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn chaos_n27_f_premature_eof_records_error() {
    let server = MockServer::start().await;
    // Stop right after message_delta — no message_stop event.
    let body = sse_body(vec![
        r#"event: message_start
data: {"type":"message_start","message":{"id":"m","usage":{"input_tokens":5,"output_tokens":1}}}"#,
        r#"event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        r#"event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#,
        r#"event: content_block_stop
data: {"type":"content_block_stop","index":0}"#,
        r#"event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}"#,
        // No message_stop.
    ]);
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(body, "text/event-stream"),
        )
        .mount(&server)
        .await;
    let bridge = bridge_with_provenance(&server).await;
    let events = drain(
        bridge
            .dispatch_stream(&small_bundle(), AgenticTask::new("x"))
            .await
            .unwrap(),
    )
    .await;
    // PrematureEof yielded as error (Note : downstream already
    // saw the text delta so retry is forbidden, the error is
    // surfaced).
    let prem = events.iter().any(|e| {
        matches!(e, AgenticEvent::Error(BridgeError::PrematureEof))
    });
    assert!(prem, "expected PrematureEof error");
}
