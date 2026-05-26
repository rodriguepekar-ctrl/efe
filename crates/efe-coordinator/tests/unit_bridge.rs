// SPDX-License-Identifier: UNLICENSED
//! Tests d'intégration `MultiLlmBridge` + `ProviderRegistry`.

use std::sync::Arc;

use efe_coordinator::{LlmCall, MockLlmProvider, MultiLlmBridge, ProviderRegistry};

fn registry(names: &[&str]) -> ProviderRegistry {
    let r = ProviderRegistry::new();
    for n in names {
        r.register(Arc::new(MockLlmProvider::new(*n).with_response(format!("from {n}"))));
    }
    r
}

#[tokio::test]
async fn call_one_returns_correct_provider() {
    let r = registry(&["claude", "gpt4"]);
    let b = MultiLlmBridge::new(r);
    let resp = b.call_one("claude", LlmCall::simple("?", 10)).await.unwrap();
    assert_eq!(resp.provider, "claude");
    assert_eq!(resp.text, "from claude");
}

#[tokio::test]
async fn call_many_returns_in_request_order_count() {
    let r = registry(&["claude", "gpt4", "gemini"]);
    let b = MultiLlmBridge::new(r);
    let ids = vec!["claude".into(), "gpt4".into(), "gemini".into()];
    let results = b.call_many(&ids, LlmCall::simple("?", 10)).await;
    assert_eq!(results.len(), 3);
    let providers: std::collections::HashSet<_> = results
        .into_iter()
        .filter_map(Result::ok)
        .map(|r| r.provider)
        .collect();
    assert_eq!(providers.len(), 3);
}

#[tokio::test]
async fn registry_overwrites_keep_one_per_id() {
    let r = ProviderRegistry::new();
    r.register(Arc::new(MockLlmProvider::new("x").with_response("a")));
    r.register(Arc::new(MockLlmProvider::new("x").with_response("b")));
    assert_eq!(r.len(), 1);
    let b = MultiLlmBridge::new(r);
    let resp = b.call_one("x", LlmCall::simple("?", 10)).await.unwrap();
    assert_eq!(resp.text, "b");
}

#[tokio::test]
async fn empty_registry_call_errors() {
    let b = MultiLlmBridge::new(ProviderRegistry::new());
    let r = b.call_one("anyone", LlmCall::simple("?", 10)).await;
    assert!(r.is_err());
}

#[tokio::test]
async fn call_many_with_some_failing_returns_partial() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("ok")));
    reg.register(Arc::new(MockLlmProvider::new("bad").fail_after(0)));
    let b = MultiLlmBridge::new(reg);
    let ids = vec!["ok".to_string(), "bad".to_string()];
    let oks = b.call_many_ok(&ids, LlmCall::simple("?", 10)).await;
    assert_eq!(oks.len(), 1);
    assert_eq!(oks[0].provider, "ok");
}

#[tokio::test]
async fn stream_one_chunks_match_words() {
    let reg = registry(&["claude"]);
    let b = MultiLlmBridge::new(reg);
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let resp = tokio::spawn({
        let b = b.clone();
        async move { b.stream_one("claude", LlmCall::simple("?", 10), tx).await }
    });
    let mut count = 0u32;
    while let Some(_chunk) = rx.recv().await {
        count += 1;
    }
    let resp = resp.await.unwrap().unwrap();
    // "from claude" → 2 mots.
    assert_eq!(count, 2);
    assert_eq!(resp.tokens_generated, 2);
}
