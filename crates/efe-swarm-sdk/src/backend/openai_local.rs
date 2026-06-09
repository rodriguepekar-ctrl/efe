// SPDX-License-Identifier: UNLICENSED
//! Backend **OpenAI-compatible LOCAL** (`POST /v1/chat/completions`) — keyless.
//!
//! Couvre llama.cpp-server, vLLM, TGI, LM Studio servis **localement**. **Aucun en-tête
//! `Authorization`** (souverain : le serveur local n'exige pas de clé). On ne transmet
//! jamais de secret.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{curl_json, BackendDescriptor, BackendKind, LocalBackend, SamplingParams};

const MAX_TIME_S: u32 = 120;

/// Backend OpenAI-compatible local (keyless).
#[derive(Debug)]
pub(crate) struct OpenAiLocalBackend {
    endpoint: String,
    model: String,
}

impl OpenAiLocalBackend {
    pub fn new(endpoint: String, model: String) -> Arc<Self> {
        Arc::new(Self { endpoint, model })
    }

    /// URL `/v1/chat/completions` (tolère un endpoint déjà suffixé `/v1`).
    fn url(&self) -> String {
        let base = self.endpoint.trim_end_matches('/');
        if base.ends_with("/v1") {
            format!("{base}/chat/completions")
        } else {
            format!("{base}/v1/chat/completions")
        }
    }
}

#[async_trait]
impl LocalBackend for OpenAiLocalBackend {
    async fn complete(&self, prompt: &str, p: SamplingParams) -> Option<String> {
        let url = self.url();
        let body = json!({
            "model": self.model,
            "messages": [{ "role": "user", "content": prompt }],
            "temperature": p.temperature,
            "max_tokens": p.max_tokens,
            "seed": p.seed,
            "stream": false
        });
        let resp = tokio::task::spawn_blocking(move || curl_json(&url, &body, MAX_TIME_S))
            .await
            .ok()
            .flatten()?;
        resp.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
    }

    fn descriptor(&self) -> BackendDescriptor {
        BackendDescriptor { kind: BackendKind::OpenAiLocal, model: self.model.clone(), endpoint: self.endpoint.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_handles_v1_suffix() {
        let a = OpenAiLocalBackend { endpoint: "http://127.0.0.1:8080".into(), model: "m".into() };
        assert_eq!(a.url(), "http://127.0.0.1:8080/v1/chat/completions");
        let b = OpenAiLocalBackend { endpoint: "http://127.0.0.1:8080/v1/".into(), model: "m".into() };
        assert_eq!(b.url(), "http://127.0.0.1:8080/v1/chat/completions");
    }
}
