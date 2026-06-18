// SPDX-License-Identifier: UNLICENSED
//! Backend **Ollama** natif (`POST /api/generate`) — keyless.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use efe_nano_sdk::ComputeAttestation;

use super::{curl_json, BackendDescriptor, BackendKind, LocalBackend, SamplingParams};
use crate::n17::{push_pending, PendingLlmCall};

/// Délai max par appel (borne le compute local — garde-fou G3).
const MAX_TIME_S: u32 = 120;

/// Backend Ollama local (keyless).
#[derive(Debug)]
pub(crate) struct OllamaBackend {
    endpoint: String,
    model: String,
    /// **RC-1** : le barreau de confiance estampillé sur chaque record (calculé par
    /// `build_local_backend` depuis la localité de l'endpoint + l'épingle).
    attestation: ComputeAttestation,
}

impl OllamaBackend {
    pub fn new(endpoint: String, model: String, attestation: ComputeAttestation) -> Arc<Self> {
        Arc::new(Self { endpoint, model, attestation })
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
        let t0 = Instant::now();
        let resp = tokio::task::spawn_blocking(move || curl_json(&url, &body, MAX_TIME_S)).await.ok().flatten();
        let latency_ms = t0.elapsed().as_millis() as u64;
        let response = resp.and_then(|r| r.get("response").and_then(|v| v.as_str()).map(|s| s.trim().to_string()));
        // Pont N17 par-appel : pousse la forensique (no-op hors d'un scope Mode C).
        push_pending(PendingLlmCall {
            engine: BackendKind::Ollama.as_str().to_string(),
            model: self.model.clone(),
            temperature: p.temperature,
            max_tokens: p.max_tokens,
            seed: p.seed,
            prompt: prompt.to_string(),
            response: response.clone(),
            latency_ms,
            retry_count: 0, // self-host local nu : le Retrier (routeur Network) écrasera si besoin.
            compute_attestation: self.attestation.clone(), // RC-1 : barreau scellé (local R2 / distant R4).
        });
        response
    }

    fn descriptor(&self) -> BackendDescriptor {
        BackendDescriptor { kind: BackendKind::Ollama, model: self.model.clone(), endpoint: self.endpoint.clone() }
    }

}
