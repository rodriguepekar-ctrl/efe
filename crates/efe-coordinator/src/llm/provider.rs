// SPDX-License-Identifier: UNLICENSED
//! Trait `LlmProvider` + types I/O — surface unifiée Claude/GPT-4/Gemini.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::error::CoordResult;

/// Identifiant kebab-case d'un provider (`"claude_opus_4"`, `"gpt4_turbo"`).
pub type ProviderId = String;

/// Requête unifiée envoyée à un LLM gros.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCall {
    /// Prompt complet (peut être déjà pré-mâché par les nano-robots).
    pub prompt: String,
    /// Tokens max à générer.
    pub max_tokens: u32,
    /// Température 0.0..2.0.
    pub temperature: f32,
    /// Estampille de deadline (None = pas de cap).
    #[serde(default)]
    pub deadline: Option<Duration>,
    /// Tag de traçabilité (forwardé dans les évènements).
    #[serde(default)]
    pub trace_tag: Option<String>,
}

impl LlmCall {
    /// Construit un call simple.
    #[must_use]
    pub fn simple(prompt: impl Into<String>, max_tokens: u32) -> Self {
        Self {
            prompt: prompt.into(),
            max_tokens,
            temperature: 0.7,
            deadline: None,
            trace_tag: None,
        }
    }
}

/// Réponse complète d'un LLM (collectée après streaming).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    /// Texte concaténé.
    pub text: String,
    /// Tokens d'output produits.
    pub tokens_generated: u32,
    /// Coût observé en cents (peut être estimé si provider ne facture pas).
    pub cost_cents: u32,
    /// Latence end-to-end ms.
    pub latency_ms: u64,
    /// Provider qui a répondu.
    pub provider: ProviderId,
    /// Score qualité auto-estimé (utilisé par le vote ; defaults to 1.0).
    #[serde(default = "default_quality")]
    pub self_quality: f32,
}

const fn default_quality() -> f32 {
    1.0
}

/// Un chunk émis pendant le streaming d'un LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenChunk {
    /// Texte du chunk.
    pub token: String,
    /// Index séquentiel dans la réponse.
    pub index: u64,
    /// Estampille de production.
    pub emitted_at: chrono::DateTime<chrono::Utc>,
}

/// Trait abstrait : tout ce qui peut répondre à un `LlmCall`.
///
/// La granularité est : 1 trait impl = 1 provider en mémoire.
/// L'orchestration (round-robin, retry, fallback) se fait dans
/// [`crate::llm::MultiLlmBridge`].
#[async_trait]
pub trait LlmProvider: Send + Sync + 'static {
    /// Identifiant kebab-case.
    fn id(&self) -> ProviderId;

    /// Appel non-streaming : retourne la réponse complète.
    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse>;

    /// Appel streaming : envoie chaque token sur le canal `tx`.
    ///
    /// Default : implémentation naïve via `call()` qui découpe sur les
    /// espaces (utile pour tests ; providers réels override).
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

    /// Health check — utilisé au boot du Coordinator.
    async fn health(&self) -> CoordResult<bool> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_call_simple_defaults() {
        let c = LlmCall::simple("hi", 100);
        assert_eq!(c.prompt, "hi");
        assert_eq!(c.max_tokens, 100);
        assert!((c.temperature - 0.7).abs() < 1e-6);
        assert!(c.deadline.is_none());
    }

    #[test]
    fn llm_response_roundtrip() {
        let r = LlmResponse {
            text: "x".into(),
            tokens_generated: 10,
            cost_cents: 1,
            latency_ms: 50,
            provider: "claude".into(),
            self_quality: 0.95,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: LlmResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back.tokens_generated, 10);
    }

    #[test]
    fn token_chunk_roundtrip() {
        let c = TokenChunk {
            token: "hi".into(),
            index: 0,
            emitted_at: chrono::Utc::now(),
        };
        let s = serde_json::to_string(&c).unwrap();
        let back: TokenChunk = serde_json::from_str(&s).unwrap();
        assert_eq!(back.token, "hi");
    }
}
