// SPDX-License-Identifier: UNLICENSED
//! `MockLlmProvider` — provider déterministe pour tests + benches.

use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::error::{CoordError, CoordResult};

use super::provider::{LlmCall, LlmProvider, LlmResponse, ProviderId, TokenChunk};

/// Provider de test entièrement déterministe.
///
/// Configurable :
/// - le texte qu'il "génère",
/// - sa latence simulée,
/// - son coût simulé,
/// - sa probabilité d'échec (déterministe : `fail_after_n` calls).
pub struct MockLlmProvider {
    id: ProviderId,
    response_text: String,
    cost_cents_per_call: u32,
    simulated_latency: Duration,
    self_quality: f32,
    /// Si Some(n), échoue à partir de l'appel `n+1`.
    fail_after_n: Option<u64>,
    /// **Sprint S8 R2** — Si Some(reason), échoue immédiatement à chaque
    /// appel avec ce message (utile pour tester graceful degradation
    /// dans les workflows : fallback chain, vote dégradé, drafter bypass…).
    always_fail_reason: Option<String>,
    call_count: Mutex<u64>,
}

impl MockLlmProvider {
    /// Builder simple.
    #[must_use]
    pub fn new(id: impl Into<ProviderId>) -> Self {
        Self {
            id: id.into(),
            response_text: "mock response".into(),
            cost_cents_per_call: 1,
            simulated_latency: Duration::from_millis(1),
            self_quality: 0.9,
            fail_after_n: None,
            always_fail_reason: None,
            call_count: Mutex::new(0),
        }
    }

    /// **Sprint S8 R2** — Le provider échoue à chaque appel avec ce
    /// message (formaté en `CoordError::LlmProvider { reason }`).
    ///
    /// Pratique pour tester :
    /// - `with_error("HTTP 429: rate limited")` → déclenche le path retry
    /// - `with_error("simulated outage")` → déclenche le fallback chain
    ///
    /// Distinct de `fail_after(0)` qui aurait le même effet mais avec
    /// un message générique. `with_error` permet d'asserter sur le
    /// `reason` exact dans les tests de classification d'erreur.
    #[must_use]
    pub fn with_error(mut self, reason: impl Into<String>) -> Self {
        self.always_fail_reason = Some(reason.into());
        self
    }

    /// Override du texte de réponse.
    #[must_use]
    pub fn with_response(mut self, text: impl Into<String>) -> Self {
        self.response_text = text.into();
        self
    }

    /// Override du coût simulé.
    #[must_use]
    pub fn with_cost_cents(mut self, c: u32) -> Self {
        self.cost_cents_per_call = c;
        self
    }

    /// Override de la latence simulée.
    #[must_use]
    pub fn with_latency(mut self, d: Duration) -> Self {
        self.simulated_latency = d;
        self
    }

    /// Override du score de self-quality (utilisé par le `VoteEngine`).
    #[must_use]
    pub fn with_quality(mut self, q: f32) -> Self {
        self.self_quality = q.clamp(0.0, 1.0);
        self
    }

    /// Le provider échoue à partir de l'appel `n+1`.
    #[must_use]
    pub fn fail_after(mut self, n: u64) -> Self {
        self.fail_after_n = Some(n);
        self
    }

    /// Nombre d'appels reçus depuis création.
    #[must_use]
    pub fn call_count(&self) -> u64 {
        *self.call_count.lock()
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    async fn call(&self, _req: LlmCall) -> CoordResult<LlmResponse> {
        let n = {
            let mut g = self.call_count.lock();
            *g += 1;
            *g
        };
        // S8 R2 : always_fail_reason a priorité (un test qui fail
        // toujours avec un message précis).
        if let Some(reason) = self.always_fail_reason.as_ref() {
            return Err(CoordError::LlmProvider {
                provider: self.id.clone(),
                reason: reason.clone(),
            });
        }
        if let Some(threshold) = self.fail_after_n {
            if n > threshold {
                return Err(CoordError::LlmProvider {
                    provider: self.id.clone(),
                    reason: format!("simulated failure at call #{n}"),
                });
            }
        }
        tokio::time::sleep(self.simulated_latency).await;
        let tokens = self.response_text.split_whitespace().count() as u32;
        Ok(LlmResponse {
            text: self.response_text.clone(),
            tokens_generated: tokens,
            cost_cents: self.cost_cents_per_call,
            latency_ms: self.simulated_latency.as_millis() as u64,
            provider: self.id.clone(),
            self_quality: self.self_quality,
        })
    }

    async fn stream(
        &self,
        req: LlmCall,
        tx: mpsc::Sender<TokenChunk>,
    ) -> CoordResult<LlmResponse> {
        let resp = self.call(req).await?;
        for (i, tok) in resp.text.split_whitespace().enumerate() {
            let chunk = TokenChunk {
                token: format!("{tok} "),
                index: i as u64,
                emitted_at: chrono::Utc::now(),
            };
            if tx.send(chunk).await.is_err() {
                break;
            }
        }
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_configured_text() {
        let p = MockLlmProvider::new("claude").with_response("hello world from claude");
        let r = p.call(LlmCall::simple("?", 10)).await.unwrap();
        assert_eq!(r.text, "hello world from claude");
        assert_eq!(r.provider, "claude");
        assert_eq!(r.tokens_generated, 4);
    }

    #[tokio::test]
    async fn mock_fail_after_triggers() {
        let p = MockLlmProvider::new("g").fail_after(2);
        assert!(p.call(LlmCall::simple("?", 10)).await.is_ok());
        assert!(p.call(LlmCall::simple("?", 10)).await.is_ok());
        let r = p.call(LlmCall::simple("?", 10)).await;
        assert!(matches!(r, Err(CoordError::LlmProvider { .. })));
    }

    #[tokio::test]
    async fn mock_stream_emits_one_chunk_per_word() {
        let p = MockLlmProvider::new("g").with_response("a b c d");
        let (tx, mut rx) = mpsc::channel(16);
        let h = tokio::spawn({
            let p = std::sync::Arc::new(p);
            async move { p.stream(LlmCall::simple("?", 10), tx).await }
        });
        let mut got = 0;
        while let Some(_c) = rx.recv().await {
            got += 1;
        }
        let _ = h.await.unwrap().unwrap();
        assert_eq!(got, 4);
    }

    #[tokio::test]
    async fn call_count_increments() {
        let p = MockLlmProvider::new("g");
        let p = std::sync::Arc::new(p);
        for _ in 0..3 {
            p.call(LlmCall::simple("?", 10)).await.unwrap();
        }
        assert_eq!(p.call_count(), 3);
    }
}
