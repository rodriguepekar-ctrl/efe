// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `MultiLlmBridge::stream_many`.

use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::{LlmCall, MockLlmProvider, MultiLlmBridge, ProviderRegistry};

fn registry() -> ProviderRegistry {
    let r = ProviderRegistry::new();
    r.register(Arc::new(
        MockLlmProvider::new("claude").with_response("alpha beta gamma"),
    ));
    r.register(Arc::new(
        MockLlmProvider::new("gpt4").with_response("delta epsilon"),
    ));
    r.register(Arc::new(
        MockLlmProvider::new("gemini").with_response("zeta"),
    ));
    r
}

#[tokio::test]
async fn stream_many_collects_chunks_from_all_providers() {
    let bridge = MultiLlmBridge::new(registry());
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let ids = vec!["claude".into(), "gpt4".into(), "gemini".into()];
    let task = tokio::spawn({
        let bridge = bridge.clone();
        async move { bridge.stream_many(&ids, LlmCall::simple("?", 10), tx).await }
    });

    let mut by_provider = std::collections::HashMap::<String, u32>::new();
    while let Some((provider, _chunk)) = rx.recv().await {
        *by_provider.entry(provider).or_insert(0) += 1;
    }
    let results = task.await.unwrap();
    assert_eq!(results.len(), 3);
    // 3 + 2 + 1 = 6 chunks attendus.
    let total: u32 = by_provider.values().sum();
    assert_eq!(total, 6);
    assert_eq!(by_provider.get("claude").copied(), Some(3));
    assert_eq!(by_provider.get("gpt4").copied(), Some(2));
    assert_eq!(by_provider.get("gemini").copied(), Some(1));
}

#[tokio::test]
async fn stream_many_empty_ids_returns_empty() {
    let bridge = MultiLlmBridge::new(registry());
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let r = bridge.stream_many(&[], LlmCall::simple("?", 10), tx).await;
    assert!(r.is_empty());
    // Nothing on rx.
    let timed = tokio::time::timeout(Duration::from_millis(20), rx.recv()).await;
    assert!(matches!(timed, Ok(None)) || timed.is_err());
}

#[tokio::test]
async fn stream_many_failed_provider_does_not_block_others() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("ok").with_response("a b c")));
    reg.register(Arc::new(MockLlmProvider::new("bad").fail_after(0)));
    let bridge = MultiLlmBridge::new(reg);
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let ids = vec!["ok".to_string(), "bad".to_string()];
    let task = tokio::spawn({
        let bridge = bridge.clone();
        async move { bridge.stream_many(&ids, LlmCall::simple("?", 10), tx).await }
    });
    let mut got = 0;
    while let Some(_c) = rx.recv().await {
        got += 1;
    }
    let results = task.await.unwrap();
    let oks = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(oks, 1);
    // "a b c" → 3 chunks from "ok".
    assert_eq!(got, 3);
}

#[tokio::test]
async fn stream_many_chunks_are_tagged_with_provider() {
    let bridge = MultiLlmBridge::new(registry());
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let ids = vec!["claude".into()];
    let task = tokio::spawn({
        let bridge = bridge.clone();
        async move { bridge.stream_many(&ids, LlmCall::simple("?", 10), tx).await }
    });
    while let Some((provider, _)) = rx.recv().await {
        assert_eq!(provider, "claude");
    }
    let _ = task.await.unwrap();
}
