// SPDX-License-Identifier: UNLICENSED
//! `MultiLlmBridge` — fan-out parallèle sur N providers + collecte.

use std::sync::Arc;
use std::time::Instant;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::mpsc;

use crate::error::{CoordError, CoordResult};

use super::provider::{LlmCall, LlmProvider, LlmResponse, ProviderId, TokenChunk};
use super::registry::ProviderRegistry;

/// Wrapper léger d'un provider + cache de l'ID (évite double-clone).
pub struct ProviderHandle {
    /// Provider sous-jacent.
    pub provider: Arc<dyn LlmProvider>,
    /// ID kebab-case.
    pub id: ProviderId,
}

/// Bridge multi-LLM : appelle plusieurs providers en parallèle et
/// agrège leurs réponses.
///
/// Construit au-dessus du [`ProviderRegistry`].  Stateless (réentrant).
#[derive(Clone)]
pub struct MultiLlmBridge {
    registry: ProviderRegistry,
}

impl MultiLlmBridge {
    /// Construit le bridge à partir d'un registre.
    #[must_use]
    pub fn new(registry: ProviderRegistry) -> Self {
        Self { registry }
    }

    /// Accès au registre sous-jacent.
    #[must_use]
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Appelle UN provider unique non-streaming.
    pub async fn call_one(&self, id: &str, req: LlmCall) -> CoordResult<LlmResponse> {
        let provider = self.registry.get(id).ok_or_else(|| CoordError::LlmProvider {
            provider: id.to_string(),
            reason: "provider not registered".into(),
        })?;
        let started = Instant::now();
        let mut resp = provider.call(req).await?;
        // Sécurise la latence : si le provider la met à 0, on prend l'observée.
        if resp.latency_ms == 0 {
            resp.latency_ms = started.elapsed().as_millis() as u64;
        }
        Ok(resp)
    }

    /// Appelle UN provider unique en streaming.
    /// Renvoie la réponse agrégée ET pousse chaque chunk sur `tx`.
    pub async fn stream_one(
        &self,
        id: &str,
        req: LlmCall,
        tx: mpsc::Sender<TokenChunk>,
    ) -> CoordResult<LlmResponse> {
        let provider = self.registry.get(id).ok_or_else(|| CoordError::LlmProvider {
            provider: id.to_string(),
            reason: "provider not registered".into(),
        })?;
        provider.stream(req, tx).await
    }

    /// Fan-out sur N providers en parallèle.  Retourne TOUTES les
    /// réponses (succès ET échecs ignorés silencieusement).
    pub async fn call_many(
        &self,
        ids: &[String],
        req: LlmCall,
    ) -> Vec<CoordResult<LlmResponse>> {
        if ids.is_empty() {
            return vec![];
        }
        let mut futs = FuturesUnordered::new();
        for id in ids {
            let r = req.clone();
            let bridge = self.clone();
            let id = id.clone();
            futs.push(async move { bridge.call_one(&id, r).await });
        }
        let mut out = Vec::with_capacity(ids.len());
        while let Some(r) = futs.next().await {
            out.push(r);
        }
        out
    }

    /// Variante "succès only" : drop les erreurs.
    pub async fn call_many_ok(&self, ids: &[String], req: LlmCall) -> Vec<LlmResponse> {
        self.call_many(ids, req)
            .await
            .into_iter()
            .filter_map(Result::ok)
            .collect()
    }

    /// Fan-out **streaming** sur N providers en parallèle.
    ///
    /// Pour chaque provider, on stream chaque `TokenChunk` via `chunk_tx`
    /// taggué avec le `ProviderId` source.  Quand tous les providers
    /// ont terminé, retourne le `Vec<LlmResponse>` agrégé (succès+échecs).
    ///
    /// Sémantique : les providers tournent vraiment en parallèle ;
    /// l'ordre des chunks sur `chunk_tx` est entrelacé selon l'arrivée.
    pub async fn stream_many(
        &self,
        ids: &[String],
        req: LlmCall,
        chunk_tx: mpsc::Sender<(ProviderId, TokenChunk)>,
    ) -> Vec<CoordResult<LlmResponse>> {
        if ids.is_empty() {
            return vec![];
        }
        let mut futs = FuturesUnordered::new();
        for id in ids {
            let r = req.clone();
            let bridge = self.clone();
            let id = id.clone();
            let tx = chunk_tx.clone();
            futs.push(async move {
                let (tagged_tx, mut tagged_rx) = mpsc::channel::<TokenChunk>(256);
                let id_clone = id.clone();
                // Pump thread : forward chunks vers le canal taggé.
                let pump = tokio::spawn(async move {
                    while let Some(c) = tagged_rx.recv().await {
                        if tx.send((id_clone.clone(), c)).await.is_err() {
                            break;
                        }
                    }
                });
                let r = bridge.stream_one(&id, r, tagged_tx).await;
                let _ = pump.await;
                r
            });
        }
        drop(chunk_tx); // signale aux receivers que plus rien n'arrivera après les pumps.
        let mut out = Vec::with_capacity(ids.len());
        while let Some(r) = futs.next().await {
            out.push(r);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockLlmProvider;

    fn registry_with(ids: &[(&str, u32)]) -> ProviderRegistry {
        let r = ProviderRegistry::new();
        for (id, cost) in ids {
            r.register(Arc::new(
                MockLlmProvider::new(*id)
                    .with_response(format!("response from {id}"))
                    .with_cost_cents(*cost),
            ));
        }
        r
    }

    #[tokio::test]
    async fn call_one_works() {
        let r = registry_with(&[("claude", 5), ("gpt4", 3)]);
        let b = MultiLlmBridge::new(r);
        let resp = b.call_one("claude", LlmCall::simple("?", 10)).await.unwrap();
        assert_eq!(resp.provider, "claude");
        assert_eq!(resp.cost_cents, 5);
    }

    #[tokio::test]
    async fn call_one_unknown_provider_errors() {
        let r = registry_with(&[("claude", 5)]);
        let b = MultiLlmBridge::new(r);
        let e = b.call_one("unknown", LlmCall::simple("?", 10)).await.unwrap_err();
        assert!(matches!(e, CoordError::LlmProvider { .. }));
    }

    #[tokio::test]
    async fn call_many_returns_all_results() {
        let r = registry_with(&[("claude", 5), ("gpt4", 3), ("gemini", 2)]);
        let b = MultiLlmBridge::new(r);
        let ids = vec!["claude".to_string(), "gpt4".to_string(), "gemini".to_string()];
        let results = b.call_many(&ids, LlmCall::simple("?", 10)).await;
        assert_eq!(results.len(), 3);
        let oks = results.into_iter().filter(Result::is_ok).count();
        assert_eq!(oks, 3);
    }

    #[tokio::test]
    async fn call_many_ok_drops_errors() {
        let reg = ProviderRegistry::new();
        reg.register(Arc::new(MockLlmProvider::new("claude")));
        // gpt4 echoue après 0 calls
        reg.register(Arc::new(MockLlmProvider::new("gpt4").fail_after(0)));
        let b = MultiLlmBridge::new(reg);
        let ids = vec!["claude".to_string(), "gpt4".to_string()];
        let oks = b.call_many_ok(&ids, LlmCall::simple("?", 10)).await;
        assert_eq!(oks.len(), 1);
        assert_eq!(oks[0].provider, "claude");
    }

    #[tokio::test]
    async fn stream_one_emits_chunks() {
        let r = registry_with(&[("claude", 1)]);
        let b = MultiLlmBridge::new(r);
        let (tx, mut rx) = mpsc::channel(16);
        let h = tokio::spawn({
            let b = b.clone();
            async move { b.stream_one("claude", LlmCall::simple("?", 10), tx).await }
        });
        let mut count = 0;
        while let Some(_c) = rx.recv().await {
            count += 1;
        }
        let _ = h.await.unwrap().unwrap();
        // "response from claude" → 3 mots.
        assert_eq!(count, 3);
    }

    #[tokio::test]
    async fn call_many_empty_ids_returns_empty() {
        let r = registry_with(&[("claude", 1)]);
        let b = MultiLlmBridge::new(r);
        let v = b.call_many(&[], LlmCall::simple("?", 10)).await;
        assert!(v.is_empty());
    }
}
