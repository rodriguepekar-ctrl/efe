// SPDX-License-Identifier: UNLICENSED
//! **R3.S4** — Provider drafter SLM **local** (llama-server, ex: Gemma 2 1.5B).
//!
//! ## Pourquoi
//!
//! L'apex du pré-mâchage économique : rapatrier le drafter Speculative en
//! local pousse son coût vers ~0 (énergie amortie, ~12-30× moins cher que
//! Gemini Flash cloud). Sur le hot path (~80 % FactualQa), l'inférence
//! devient quasi-gratuite. Bénéfices secondaires : latence (pas de
//! round-trip cloud), souveraineté (le draft ne quitte pas la machine),
//! offline-capable.
//!
//! ## Le piège (cf. architecture §0.4 + §9 R1)
//!
//! Un 1.5B local **bruite plus** → le gate R3.S3 ([`crate::speculate::
//! NoiseRejectionPolicy`]) rejette plus → plus de fallbacks verifier. Le
//! break-even est étroit (~1.9 points de hot-path-rate). R3.S4 livre
//! l'infra ET l'instrument de mesure (`spec_noise_rejected`), pas une
//! promesse aveugle.
//!
//! ## Wire
//!
//! Parle le protocole llama-server `/completion` en mode **texte libre**
//! (pas de grammar — un drafter produit de la prose, pas du JSON).
//! Implémente [`LlmProvider`] pour s'insérer comme drafter du
//! [`Speculator`](crate::speculate::Speculator).
//!
//! ## Coût (honnête)
//!
//! Le local **n'est PAS gratuit** : il consomme de l'énergie réelle +
//! amortissement hardware. Modélisé via `cost_*_microcents_per_m`
//! configurable (défaut ~1500 µ¢/M, calibrable via RAPL — cf. bench
//! framework §38). On ne met PAS 0 (cohérent « Honest Benchmarks »).

use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;

use crate::error::{CoordError, CoordResult};
use crate::llm::{
    compute_cost_microcents, microcents_to_cents, LlmCall, LlmProvider, LlmResponse,
    ProviderId,
};

/// Coût µ¢/M par défaut (énergie amortie Gemma 2 1.5B / M-series Metal).
/// ~1110 µ¢/M énergie (@ $0.15/kWh, 75 tok/s, 20 W) + ~390 amortissement.
/// Calibrable par l'opérateur (`with_cost`). Cf. architecture §4.3.
pub const DEFAULT_LOCAL_SLM_COST_MICROCENTS_PER_M: u64 = 1_500;

/// Tokens max par défaut pour un drafter local (court = haute confiance).
const DEFAULT_LOCAL_SLM_MAX_TOKENS: u32 = 64;

/// Température par défaut (drafter factuel : modérée).
const DEFAULT_LOCAL_SLM_TEMPERATURE: f32 = 0.5;

/// Timeout par défaut (local = rapide ; au-delà = llama-server en peine).
const DEFAULT_LOCAL_SLM_TIMEOUT: Duration = Duration::from_secs(3);

/// Réponse minimale du wire llama-server `/completion`.
///
/// Struct local (auto-suffisant) plutôt que réutiliser le type nano-app :
/// découple l'adaptateur du crate `efe-nano-app-coordinator` (le wire est
/// stable). On ne désérialise que les 3 champs utiles.
#[derive(Debug, Deserialize)]
struct LocalCompletion {
    /// Texte généré.
    content: String,
    /// Tokens du prompt (billing/audit).
    #[serde(default)]
    tokens_evaluated: u32,
    /// Tokens générés (billing/audit).
    #[serde(default)]
    tokens_predicted: u32,
}

/// **R3.S4** — Provider drafter SLM local (llama-server texte-libre).
pub struct LocalSlmProvider {
    /// ID kebab-case (ex: `"local_gemma-2-1.5b"`).
    id: ProviderId,
    /// URL llama-server (ex: `"http://127.0.0.1:8099"`).
    endpoint: String,
    /// Label modèle (audit + pricing hint).
    #[allow(dead_code)]
    model_label: String,
    /// System prompt court préfixé (optionnel).
    system_prompt: String,
    /// Tokens max générés.
    max_tokens: u32,
    /// Température.
    temperature: f32,
    /// Timeout end-to-end.
    timeout: Duration,
    /// Coût input µ¢/M (énergie amortie).
    cost_input_microcents_per_m: u64,
    /// Coût output µ¢/M.
    cost_output_microcents_per_m: u64,
    /// Client HTTP réutilisable (connection pooling).
    http: reqwest::Client,
}

impl LocalSlmProvider {
    /// Construit un provider local pointant sur un llama-server.
    ///
    /// `id` : identifiant registry (ex: `"local_gemma-2-1.5b"`).
    /// `endpoint` : URL base llama-server (ex: `"http://127.0.0.1:8099"`).
    /// `model_label` : nom du modèle (audit/pricing).
    pub fn new(
        id: impl Into<String>,
        endpoint: impl Into<String>,
        model_label: impl Into<String>,
    ) -> CoordResult<Self> {
        let http = reqwest::Client::builder()
            .pool_max_idle_per_host(8)
            .build()
            .map_err(|e| CoordError::Config(format!("local SLM HTTP client: {e}")))?;
        Ok(Self {
            id: id.into(),
            endpoint: endpoint.into(),
            model_label: model_label.into(),
            system_prompt: String::new(),
            max_tokens: DEFAULT_LOCAL_SLM_MAX_TOKENS,
            temperature: DEFAULT_LOCAL_SLM_TEMPERATURE,
            timeout: DEFAULT_LOCAL_SLM_TIMEOUT,
            cost_input_microcents_per_m: DEFAULT_LOCAL_SLM_COST_MICROCENTS_PER_M,
            cost_output_microcents_per_m: DEFAULT_LOCAL_SLM_COST_MICROCENTS_PER_M,
            http,
        })
    }

    /// Construit depuis l'environnement.
    ///
    /// - `EFE_LOCAL_SLM_URL` (défaut `http://127.0.0.1:8099`)
    /// - `EFE_LOCAL_SLM_MODEL` (défaut `gemma-2-1.5b`)
    /// - `EFE_LOCAL_SLM_ID` (défaut `local_<model>`)
    pub fn from_env() -> CoordResult<Self> {
        let url = std::env::var("EFE_LOCAL_SLM_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8099".to_string());
        let model = std::env::var("EFE_LOCAL_SLM_MODEL")
            .unwrap_or_else(|_| "gemma-2-1.5b".to_string());
        let id = std::env::var("EFE_LOCAL_SLM_ID")
            .unwrap_or_else(|_| format!("local_{model}"));
        Self::new(id, url, model)
    }

    /// **R3.S4** — Override le coût µ¢/M (calibration énergie locale).
    #[must_use]
    pub fn with_cost(mut self, input_mc_per_m: u64, output_mc_per_m: u64) -> Self {
        self.cost_input_microcents_per_m = input_mc_per_m;
        self.cost_output_microcents_per_m = output_mc_per_m;
        self
    }

    /// Override le system prompt.
    #[must_use]
    pub fn with_system_prompt(mut self, p: impl Into<String>) -> Self {
        self.system_prompt = p.into();
        self
    }

    /// Override le nombre max de tokens générés.
    #[must_use]
    pub fn with_max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    /// Override la température.
    #[must_use]
    pub fn with_temperature(mut self, t: f32) -> Self {
        self.temperature = t.clamp(0.0, 2.0);
        self
    }

    /// Override le timeout.
    #[must_use]
    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    /// **Pur** (testable sans réseau) — construit le prompt complet
    /// (system + user) envoyé à llama-server.
    fn build_prompt(&self, user_text: &str) -> String {
        if self.system_prompt.is_empty() {
            user_text.to_string()
        } else {
            format!("{}\n\n{}", self.system_prompt, user_text)
        }
    }

    /// **Pur** — calcule le coût µ¢ depuis les compteurs de tokens (R3.S1).
    fn cost_microcents_for(&self, tokens_in: u32, tokens_out: u32) -> u64 {
        compute_cost_microcents(
            tokens_in,
            tokens_out,
            self.cost_input_microcents_per_m,
            self.cost_output_microcents_per_m,
        )
    }
}

#[async_trait]
impl LlmProvider for LocalSlmProvider {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }

    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
        let started = Instant::now();
        let full_prompt = self.build_prompt(&req.prompt);
        // n_predict borné par le min(req, config) — un drafter local reste court.
        let n_predict = req.max_tokens.min(self.max_tokens).max(1);
        let body = serde_json::json!({
            "prompt": full_prompt,
            "n_predict": n_predict,
            "temperature": self.temperature,
            "cache_prompt": true,
            "stop": ["\n\n\n"],
            // PAS de "grammar" : un drafter produit du texte libre.
        });
        let url = format!("{}/completion", self.endpoint);

        let resp = tokio::time::timeout(
            self.timeout,
            self.http.post(&url).json(&body).send(),
        )
        .await
        .map_err(|_| CoordError::LlmProvider {
            provider: self.id.clone(),
            reason: format!(
                "local SLM timeout after {}ms",
                self.timeout.as_millis()
            ),
        })?
        .map_err(|e| CoordError::LlmProvider {
            provider: self.id.clone(),
            reason: format!("local SLM transport: {e}"),
        })?;

        if !resp.status().is_success() {
            return Err(CoordError::LlmProvider {
                provider: self.id.clone(),
                reason: format!("local SLM HTTP {}", resp.status().as_u16()),
            });
        }

        let raw: LocalCompletion =
            resp.json().await.map_err(|e| CoordError::LlmProvider {
                provider: self.id.clone(),
                reason: format!("local SLM parse: {e}"),
            })?;

        // R3.S1 : coût µ¢ (énergie amortie locale).
        let cost_mc = self.cost_microcents_for(raw.tokens_evaluated, raw.tokens_predicted);

        Ok(LlmResponse {
            text: raw.content.trim().to_string(),
            tokens_generated: raw.tokens_predicted,
            cost_cents: microcents_to_cents(cost_mc),
            cost_microcents: cost_mc,
            latency_ms: started.elapsed().as_millis() as u64,
            provider: self.id.clone(),
            // 1.5B : qualité auto-déclarée modérée (le vote/Jaccard arbitre).
            self_quality: 0.7,
        })
    }

    // `stream` : MVP délègue au default (call + split-whitespace). Le vrai
    // SSE llama-server (`"stream": true`) est R3.S4.1 (optimisation latence
    // perçue, pas correction). Le default émet bien les DraftToken events.

    async fn health(&self) -> CoordResult<bool> {
        let url = format!("{}/health", self.endpoint);
        let r = tokio::time::timeout(
            Duration::from_secs(2),
            self.http.get(&url).send(),
        )
        .await;
        match r {
            Ok(Ok(resp)) => Ok(resp.status().is_success()),
            _ => Ok(false),
        }
    }
}

// ═════════════════════════════════════════════════════════════════════════
// Tests
// ═════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;

    fn provider(endpoint: &str) -> LocalSlmProvider {
        LocalSlmProvider::new("local_gemma-2-1.5b", endpoint, "gemma-2-1.5b")
            .expect("provider builds")
    }

    // ── Mini serveur HTTP llama-server (zéro dépendance externe) ──
    //
    // Accepte 1 connexion, ignore la requête, renvoie une réponse HTTP/1.1
    // minimale avec `Content-Length` + `Connection: close` (reqwest lit le
    // body correctement). Retourne l'URL base à passer au provider.
    async fn spawn_mock_llama(
        status_line: &'static str,
        json_body: String,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let handle = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                // Drain la requête (best-effort, non-bloquant après 1 read).
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{json_body}",
                    json_body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        (url, handle)
    }

    // ── Tests purs (sans réseau) ──

    #[test]
    fn r3s4_local_slm_builds_full_prompt_with_system() {
        let p = provider("http://x").with_system_prompt("You are a drafter.");
        assert_eq!(
            p.build_prompt("Capital of France?"),
            "You are a drafter.\n\nCapital of France?"
        );
    }

    #[test]
    fn r3s4_local_slm_builds_prompt_without_system() {
        let p = provider("http://x");
        assert_eq!(p.build_prompt("Capital of France?"), "Capital of France?");
    }

    #[test]
    fn r3s4_local_slm_cost_microcents_uses_local_rate() {
        // 1000 in @ 1500 µ¢/M = floor(1_500_000/1M) = 1 ; idem out = 1 → 2 µ¢.
        // (chaque terme est floored séparément avant somme — cf.
        // compute_cost_microcents R3.S1).
        let p = provider("http://x");
        assert_eq!(p.cost_microcents_for(1_000, 1_000), 2);
    }

    #[test]
    fn r3s4_local_slm_cost_far_below_gemini_flash() {
        // Gemini Flash : 1000 in @ 7500 + 1000 out @ 30000 = 7 + 30 = 37 µ¢.
        // Local @ 1500/1500 = 3 µ¢ → ~12× moins cher.
        let p = provider("http://x");
        let local = p.cost_microcents_for(1_000, 1_000);
        let gemini = compute_cost_microcents(1_000, 1_000, 7_500, 30_000);
        assert!(local * 10 < gemini, "local ({local}) doit être ≪ gemini ({gemini})");
    }

    #[test]
    fn r3s4_local_slm_id_is_stable() {
        assert_eq!(provider("http://x").id(), "local_gemma-2-1.5b");
    }

    #[test]
    fn r3s4_local_slm_with_cost_override() {
        let p = provider("http://x").with_cost(5_000, 5_000);
        // 1000 in @ 5000 µ¢/M = floor(5_000_000/1M) = 5 ; idem out → 10 µ¢.
        assert_eq!(p.cost_microcents_for(1_000, 1_000), 10);
    }

    #[test]
    fn r3s4_local_slm_temperature_clamped() {
        let p = provider("http://x").with_temperature(5.0);
        assert!((p.temperature - 2.0).abs() < f32::EPSILON);
    }

    // ── Tests HTTP (mini serveur tokio) ──

    #[tokio::test]
    async fn r3s4_local_slm_call_parses_completion() {
        let body = r#"{"content":"Paris","tokens_evaluated":12,"tokens_predicted":1}"#;
        let (url, h) = spawn_mock_llama("200 OK", body.to_string()).await;
        let p = provider(&url);
        let resp = p.call(LlmCall::simple("Capital of France?", 64)).await.unwrap();
        assert_eq!(resp.text, "Paris");
        assert_eq!(resp.tokens_generated, 1);
        assert_eq!(resp.provider, "local_gemma-2-1.5b");
        // 12 in + 1 out @ 1500 µ¢/M = 0 + 0 (integer floor) = 0 µ¢ sur si peu
        // de tokens — c'est correct (sub-µ¢ arrondi à 0, comme attendu).
        assert_eq!(resp.cost_microcents, 0);
        let _ = h.await;
    }

    #[tokio::test]
    async fn r3s4_local_slm_call_cost_nonzero_on_larger_output() {
        let body = r#"{"content":"x","tokens_evaluated":1000,"tokens_predicted":1000}"#;
        let (url, h) = spawn_mock_llama("200 OK", body.to_string()).await;
        let p = provider(&url);
        let resp = p.call(LlmCall::simple("q", 2000)).await.unwrap();
        assert_eq!(resp.cost_microcents, 2, "1000/1000 @ 1500 = 1+1 = 2 µ¢ (floor par terme)");
        let _ = h.await;
    }

    #[tokio::test]
    async fn r3s4_local_slm_call_http_500_errors() {
        let (url, h) = spawn_mock_llama("500 Internal Server Error", "{}".to_string()).await;
        let p = provider(&url);
        let r = p.call(LlmCall::simple("q", 64)).await;
        assert!(matches!(r, Err(CoordError::LlmProvider { .. })));
        let _ = h.await;
    }

    #[tokio::test]
    async fn r3s4_local_slm_call_unreachable_errors() {
        // Port fermé → transport error.
        let p = provider("http://127.0.0.1:1");
        let r = p.call(LlmCall::simple("q", 64)).await;
        assert!(matches!(r, Err(CoordError::LlmProvider { .. })));
    }

    #[tokio::test]
    async fn r3s4_local_slm_health_up_when_200() {
        let (url, h) = spawn_mock_llama("200 OK", "ok".to_string()).await;
        let p = provider(&url);
        assert!(p.health().await.unwrap());
        let _ = h.await;
    }

    #[tokio::test]
    async fn r3s4_local_slm_health_down_when_unreachable() {
        let p = provider("http://127.0.0.1:1");
        assert!(!p.health().await.unwrap());
    }

    #[tokio::test]
    async fn r3s4_local_slm_stream_default_emits_tokens() {
        let body = r#"{"content":"Paris is capital","tokens_evaluated":5,"tokens_predicted":3}"#;
        let (url, h) = spawn_mock_llama("200 OK", body.to_string()).await;
        let p = Arc::new(provider(&url));
        let (tx, mut rx) = mpsc::channel(16);
        let pc = Arc::clone(&p);
        let task = tokio::spawn(async move {
            pc.stream(LlmCall::simple("q", 64), tx).await
        });
        let mut count = 0;
        while rx.recv().await.is_some() {
            count += 1;
        }
        let _ = task.await.unwrap().unwrap();
        assert_eq!(count, 3, "3 mots → 3 DraftToken via le default split");
        let _ = h.await;
    }
}
