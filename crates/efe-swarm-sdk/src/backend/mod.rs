// SPDX-License-Identifier: UNLICENSED
//! Backends LLM **locaux**, **tous keyless** — l'infrastructure du mode phare self-host.
//!
//! Un seul trait [`LocalBackend`] abstrait Ollama (natif) **et** les serveurs
//! **OpenAI-compatibles locaux** (llama.cpp-server, vLLM, TGI, LM Studio). **Aucune clé,
//! aucun en-tête d'auth, aucune dépendance réseau crate** : on shell-out `curl` vers
//! `127.0.0.1` (cf. `llm.rs`).
//!
//! Garde-fous (architecture §5/§7) :
//! - **G2 souveraineté** : [`build_local_backend`] **refuse** un endpoint non-local par
//!   défaut (egress interdit) → [`SwarmError::RemoteBackendDenied`].
//! - **G3 robustesse** : [`Breaker`] (circuit-breaker) coupe après N échecs consécutifs →
//!   sentinel neutre (la topologie dégrade sans saturer l'hôte).
//! - **Seeding** exposé via [`SamplingParams`] (best-effort de reproductibilité).

mod ollama;
mod openai_local;

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use crate::result::SwarmError;

pub(crate) use ollama::OllamaBackend;
pub(crate) use openai_local::OpenAiLocalBackend;

/// Paramètres d'échantillonnage (seed exposé → best-effort de reproductibilité).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SamplingParams {
    pub max_tokens: u32,
    pub temperature: f64,
    pub seed: u64,
}

/// Moteur local sous-jacent (pour le descripteur forensique / pont N17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackendKind {
    /// Ollama natif (`/api/generate`).
    Ollama,
    /// Serveur OpenAI-compatible **local** (llama.cpp-server, vLLM, TGI, LM Studio).
    OpenAiLocal,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Ollama => "ollama",
            BackendKind::OpenAiLocal => "openai_local",
        }
    }
}

/// Descripteur d'un backend (alimente le pont N17 : *quel modèle, où*).
#[derive(Debug, Clone)]
pub(crate) struct BackendDescriptor {
    pub kind: BackendKind,
    pub model: String,
    pub endpoint: String,
}

/// Backend LLM local **keyless**.
#[async_trait]
pub(crate) trait LocalBackend: Send + Sync {
    /// Complète un prompt. `None` = échec (la couche appelante renvoie un sentinel neutre).
    async fn complete(&self, prompt: &str, params: SamplingParams) -> Option<String>;
    /// Descripteur (moteur + modèle + endpoint) — forensique, jamais une clé.
    fn descriptor(&self) -> BackendDescriptor;
}

// ─────────────────────────────────────────────────────────────────────────────
//  curl partagé (keyless : aucun en-tête Authorization)
// ─────────────────────────────────────────────────────────────────────────────

/// POST JSON via `curl` (corps par stdin → pas d'échappement shell ; en-tête content-type
/// pour les serveurs OpenAI-compat). **Aucun en-tête d'auth.** Bloquant (à envelopper dans
/// `spawn_blocking`).
pub(crate) fn curl_json(url: &str, body: &Value, max_time_s: u32) -> Option<Value> {
    let mut child = Command::new("curl")
        .args([
            "-s",
            "--max-time",
            &max_time_s.to_string(),
            "-H",
            "content-type: application/json",
            url,
            "-d",
            "@-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(body.to_string().as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Garde-fou souveraineté : endpoint local uniquement (par défaut)
// ─────────────────────────────────────────────────────────────────────────────

/// Extrait l'hôte d'un endpoint (`scheme://[user@]host[:port][/path]`), gère IPv6 `[::1]`.
fn host_of(endpoint: &str) -> Option<String> {
    let after_scheme = endpoint.split("://").last()?;
    let authority = after_scheme.split('/').next()?;
    let hostport = authority.rsplit('@').next()?; // ignore un éventuel userinfo
    if let Some(rest) = hostport.strip_prefix('[') {
        // IPv6 littéral : [::1]:port
        return rest.split(']').next().map(|h| h.to_string());
    }
    hostport.split(':').next().map(|h| h.to_string())
}

/// L'endpoint cible-t-il la **machine locale** (boucle locale) ? Base du garde-fou egress.
pub(crate) fn is_local_endpoint(endpoint: &str) -> bool {
    match host_of(endpoint) {
        Some(h) => h == "localhost" || h == "::1" || h.starts_with("127."),
        None => false,
    }
}

/// Construit le backend local idoine, **en appliquant le garde-fou souveraineté**.
///
/// - Endpoint non-local + `allow_remote == false` ⇒ [`SwarmError::RemoteBackendDenied`].
/// - Sélection du moteur : `model` préfixé `openai:` **ou** endpoint contenant `/v1`
///   ⇒ OpenAI-compatible local ; sinon Ollama.
/// - Le backend est enveloppé d'un [`Breaker`] (circuit-breaker).
pub(crate) fn build_local_backend(
    endpoint: &str,
    model: &str,
    allow_remote: bool,
) -> Result<Arc<dyn LocalBackend>, SwarmError> {
    if !allow_remote && !is_local_endpoint(endpoint) {
        return Err(SwarmError::RemoteBackendDenied { endpoint: endpoint.to_string() });
    }
    let inner: Arc<dyn LocalBackend> = if let Some(m) = model.strip_prefix("openai:") {
        OpenAiLocalBackend::new(endpoint.to_string(), m.to_string())
    } else if endpoint.contains("/v1") {
        OpenAiLocalBackend::new(endpoint.to_string(), model.to_string())
    } else {
        OllamaBackend::new(endpoint.to_string(), model.to_string())
    };
    Ok(Breaker::wrap(inner, DEFAULT_BREAKER_THRESHOLD, DEFAULT_BREAKER_COOLDOWN))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Circuit-breaker (protège l'hôte : coupe après N échecs consécutifs)
// ─────────────────────────────────────────────────────────────────────────────

const DEFAULT_BREAKER_THRESHOLD: u32 = 5;
const DEFAULT_BREAKER_COOLDOWN: Duration = Duration::from_secs(15);

/// Enveloppe un [`LocalBackend`] : après `threshold` échecs consécutifs, **ouvre** le
/// circuit pour `cooldown` — les appels suivants renvoient `None` **sans** toucher au
/// backend (sentinel neutre). Un succès **réarme**.
pub(crate) struct Breaker {
    inner: Arc<dyn LocalBackend>,
    fails: AtomicU32,
    threshold: u32,
    open_until: Mutex<Option<Instant>>,
    cooldown: Duration,
}

impl Breaker {
    pub fn new(inner: Arc<dyn LocalBackend>, threshold: u32, cooldown: Duration) -> Self {
        Self { inner, fails: AtomicU32::new(0), threshold: threshold.max(1), open_until: Mutex::new(None), cooldown }
    }
    pub fn wrap(inner: Arc<dyn LocalBackend>, threshold: u32, cooldown: Duration) -> Arc<dyn LocalBackend> {
        Arc::new(Self::new(inner, threshold, cooldown))
    }
    fn is_open(&self) -> bool {
        match self.open_until.lock() {
            Ok(g) => g.map(|t| Instant::now() < t).unwrap_or(false),
            Err(_) => false,
        }
    }
    fn trip(&self) {
        if let Ok(mut g) = self.open_until.lock() {
            *g = Some(Instant::now() + self.cooldown);
        }
    }
    fn reset(&self) {
        self.fails.store(0, Ordering::Relaxed);
        if let Ok(mut g) = self.open_until.lock() {
            *g = None;
        }
    }
}

#[async_trait]
impl LocalBackend for Breaker {
    async fn complete(&self, prompt: &str, params: SamplingParams) -> Option<String> {
        if self.is_open() {
            return None; // circuit ouvert : on protège l'hôte (sentinel)
        }
        match self.inner.complete(prompt, params).await {
            Some(s) => {
                self.reset();
                Some(s)
            }
            None => {
                let f = self.fails.fetch_add(1, Ordering::Relaxed) + 1;
                if f >= self.threshold {
                    self.trip();
                }
                None
            }
        }
    }
    fn descriptor(&self) -> BackendDescriptor {
        self.inner.descriptor()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_endpoints_are_recognized() {
        for e in [
            "http://127.0.0.1:11434",
            "http://localhost:8080/v1",
            "http://127.0.0.5:1234",
            "http://[::1]:11434",
        ] {
            assert!(is_local_endpoint(e), "{e} devrait être local");
        }
        for e in ["http://example.com:11434", "https://api.openai.com/v1", "http://10.0.0.5:11434", "http://192.168.1.9:8080"] {
            assert!(!is_local_endpoint(e), "{e} ne devrait PAS être local");
        }
    }

    #[test]
    fn build_denies_remote_by_default_and_allows_with_flag() {
        // garde-fou souveraineté : non-local refusé par défaut.
        let err = build_local_backend("http://example.com:11434", "gemma3:12b", false);
        assert!(matches!(err, Err(SwarmError::RemoteBackendDenied { .. })));
        // opt-in explicite → autorisé (construit, sans appel réseau).
        assert!(build_local_backend("http://example.com:11434", "gemma3:12b", true).is_ok());
        // local : toujours OK.
        assert!(build_local_backend("http://127.0.0.1:11434", "gemma3:12b", false).is_ok());
    }

    #[test]
    fn build_selects_engine_by_endpoint_and_model_prefix() {
        let ollama = build_local_backend("http://127.0.0.1:11434", "gemma3:12b", false).unwrap();
        assert_eq!(ollama.descriptor().kind, BackendKind::Ollama);
        let openai_by_path = build_local_backend("http://127.0.0.1:8080/v1", "qwen", false).unwrap();
        assert_eq!(openai_by_path.descriptor().kind, BackendKind::OpenAiLocal);
        let openai_by_prefix = build_local_backend("http://127.0.0.1:8000", "openai:llama3", false).unwrap();
        assert_eq!(openai_by_prefix.descriptor().kind, BackendKind::OpenAiLocal);
        assert_eq!(openai_by_prefix.descriptor().model, "llama3", "préfixe retiré");
    }

    // ── circuit-breaker (mock, sans réseau) ──────────────────────────────────
    struct Flaky {
        ok: bool,
        calls: AtomicU32,
    }
    #[async_trait]
    impl LocalBackend for Flaky {
        async fn complete(&self, _p: &str, _s: SamplingParams) -> Option<String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.ok {
                Some("ok".into())
            } else {
                None
            }
        }
        fn descriptor(&self) -> BackendDescriptor {
            BackendDescriptor { kind: BackendKind::Ollama, model: "mock".into(), endpoint: "mock".into() }
        }
    }
    fn params() -> SamplingParams {
        SamplingParams { max_tokens: 8, temperature: 0.0, seed: 7 }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn breaker_opens_after_threshold_and_skips_inner() {
        let mock = Arc::new(Flaky { ok: false, calls: AtomicU32::new(0) });
        let breaker = Breaker::new(mock.clone(), 2, Duration::from_secs(60));
        assert!(breaker.complete("x", params()).await.is_none());
        assert!(breaker.complete("x", params()).await.is_none()); // 2e échec → ouvre
        assert_eq!(mock.calls.load(Ordering::Relaxed), 2);
        // circuit ouvert : inner N'EST PAS appelé.
        assert!(breaker.complete("x", params()).await.is_none());
        assert_eq!(mock.calls.load(Ordering::Relaxed), 2, "circuit ouvert → inner sauté");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn breaker_passes_through_when_healthy() {
        let mock = Arc::new(Flaky { ok: true, calls: AtomicU32::new(0) });
        let breaker = Breaker::new(mock.clone(), 2, Duration::from_secs(60));
        assert_eq!(breaker.complete("x", params()).await.as_deref(), Some("ok"));
        assert_eq!(breaker.complete("x", params()).await.as_deref(), Some("ok"));
        assert_eq!(mock.calls.load(Ordering::Relaxed), 2);
    }
}
