// SPDX-License-Identifier: UNLICENSED
//! Backend **Ollama** natif (`POST /api/generate`) — keyless.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{curl_json, BackendDescriptor, BackendKind, LocalBackend, SamplingParams};

/// Délai max par appel (borne le compute local — garde-fou G3).
const MAX_TIME_S: u32 = 120;

/// Backend Ollama local (keyless).
#[derive(Debug)]
pub(crate) struct OllamaBackend {
    endpoint: String,
    model: String,
}

impl OllamaBackend {
    pub fn new(endpoint: String, model: String) -> Arc<Self> {
        Arc::new(Self { endpoint, model })
    }
}

#[async_trait]
impl LocalBackend for OllamaBackend {
    async fn complete(&self, prompt: &str, p: SamplingParams) -> Option<String> {
        let url = format!("{}/api/generate", self.endpoint.trim_end_matches('/'));
        let body = json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "options": { "temperature": p.temperature, "num_predict": p.max_tokens, "seed": p.seed }
        });
        let resp = tokio::task::spawn_blocking(move || curl_json(&url, &body, MAX_TIME_S))
            .await
            .ok()
            .flatten()?;
        resp.get("response").and_then(|v| v.as_str()).map(|s| s.trim().to_string())
    }

    fn descriptor(&self) -> BackendDescriptor {
        BackendDescriptor { kind: BackendKind::Ollama, model: self.model.clone(), endpoint: self.endpoint.clone() }
    }
}
