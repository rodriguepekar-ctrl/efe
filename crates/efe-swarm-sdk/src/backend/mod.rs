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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde_json::Value;

use crate::n17::{push_pending, scoped, PendingLlmCall};
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
    /// **Health-check (S.a)** : le backend répond-il ? Sonde **rapide** (TCP/HTTP, `--max-time`
    /// court) de l'endpoint. Défaut : reachability via `curl`. Échec rapide plutôt que N timeouts.
    async fn health(&self) -> bool {
        let endpoint = self.descriptor().endpoint;
        tokio::task::spawn_blocking(move || health_probe(&endpoint)).await.unwrap_or(false)
    }
}

/// Sonde de joignabilité (connexion établie = sain ; aucun secret transmis). Bloquant.
pub(crate) fn health_probe(endpoint: &str) -> bool {
    Command::new("curl")
        .args(["-s", "-o", "/dev/null", "--max-time", "3", endpoint])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
/// - **Inc.4.b** : sur le chemin **routeur Network** (`allow_remote == true`, egress assumé),
///   le backend est enveloppé d'un [`Retrier`] (re-tentatives par-appel à back-off exponentiel
///   + jitter) **sous** le [`Breaker`]. Le self-host **souverain** (loopback) reste **inchangé**
///   (Breaker seul) : sur boucle locale, un échec est franc et rapide, le breaker suffit.
/// - L'empilement final est `Breaker(Retrier(moteur))` : le breaker ne compte qu'**une**
///   défaillance par appel logique (après épuisement des re-tentatives).
#[allow(clippy::doc_lazy_continuation)] // listes de doc multi-lignes : présentation intentionnelle
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
    // Routeur Network (egress) : re-tentatives transitoires par-appel. Souverain local : non.
    let resilient = if allow_remote {
        Retrier::wrap(inner, NETWORK_RETRY_ATTEMPTS, NETWORK_RETRY_BASE_DELAY, NETWORK_RETRY_MAX_DELAY)
    } else {
        inner
    };
    Ok(Breaker::wrap(resilient, DEFAULT_BREAKER_THRESHOLD, DEFAULT_BREAKER_COOLDOWN))
}

// ─────────────────────────────────────────────────────────────────────────────
//  Retrier (Inc.4.b) : re-tentatives par-appel — chemin routeur Network (egress)
// ─────────────────────────────────────────────────────────────────────────────

/// Nombre **total** de tentatives par appel (1 essai initial + N-1 re-tentatives).
const NETWORK_RETRY_ATTEMPTS: u32 = 3;
/// Délai de base du back-off exponentiel (doublé à chaque re-tentative).
const NETWORK_RETRY_BASE_DELAY: Duration = Duration::from_millis(200);
/// Plafond du délai (le back-off n'attend jamais plus que ça entre deux essais).
const NETWORK_RETRY_MAX_DELAY: Duration = Duration::from_secs(5);

/// Entropie bon marché (nanos d'horloge) pour le jitter — **best-effort**, sans dép `rand`.
fn jitter_nanos() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos() as u64).unwrap_or(0)
}

/// Enveloppe un [`LocalBackend`] de **re-tentatives par-appel** à back-off exponentiel +
/// jitter — pour le chemin **routeur Network** (frontière distante, hoquets transitoires).
///
/// **Correction N17 (le point délicat)** : le backend feuille pousse **une** [`PendingLlmCall`]
/// par `complete()`. Si le Retrier laissait fuiter chaque tentative dans le puits du `Meter`,
/// l'`AgentCallRecord` se lierait à la **première tentative (échouée)** et le pont N17
/// compterait des appels fantômes. Le Retrier exécute donc chaque tentative dans un **puits
/// imbriqué** (task-local) qu'il **draine** ; il ne **ré-émet qu'UN seul** record final vers
/// le puits parent — celui de l'issue retenue — en y inscrivant `retry_count`.
///
/// Limite assumée (honnête) : la couche `curl` keyless ne **distingue pas** un 4xx
/// (non-retryable) d'un 5xx/timeout (transitoire) — elle renvoie `None` dans les deux cas.
/// Les re-tentatives sont donc **transitoires-par-défaut** ; pas de variante `NonRetryable`
/// détectée ici. (Évolution possible : surfacer `%{http_code}` depuis `curl`.)
pub(crate) struct Retrier {
    inner: Arc<dyn LocalBackend>,
    attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
}

impl Retrier {
    pub fn new(inner: Arc<dyn LocalBackend>, attempts: u32, base_delay: Duration, max_delay: Duration) -> Self {
        Self { inner, attempts: attempts.max(1), base_delay, max_delay }
    }
    pub fn wrap(inner: Arc<dyn LocalBackend>, attempts: u32, base_delay: Duration, max_delay: Duration) -> Arc<dyn LocalBackend> {
        Arc::new(Self::new(inner, attempts, base_delay, max_delay))
    }

    /// Back-off **exponentiel plafonné + jitter** (equal-jitter : moitié fixe, moitié aléatoire)
    /// avant la re-tentative `attempt_index` (0-based). Borné par `max_delay`.
    fn backoff_delay(&self, attempt_index: u32) -> Duration {
        let shift = attempt_index.min(16);
        let factor = 1u64 << shift;
        let base_ms = self.base_delay.as_millis() as u64;
        let exp_ms = base_ms.saturating_mul(factor);
        let cap_ms = (self.max_delay.as_millis() as u64).max(1);
        let capped = exp_ms.min(cap_ms).max(1);
        let half = capped / 2;
        let jitter = if half > 0 { jitter_nanos() % half } else { 0 };
        Duration::from_millis(half + jitter)
    }
}

#[async_trait]
impl LocalBackend for Retrier {
    async fn complete(&self, prompt: &str, params: SamplingParams) -> Option<String> {
        let mut last_call: Option<PendingLlmCall> = None;
        for attempt in 0..self.attempts {
            // Puits IMBRIQUÉ : capte la forensique de CETTE tentative sans polluer le parent.
            let sink = Arc::new(Mutex::new(Vec::<PendingLlmCall>::new()));
            let out = scoped(sink.clone(), self.inner.complete(prompt, params)).await;
            let mut captured = sink.lock().ok().and_then(|mut v| v.pop());
            if let Some(c) = captured.as_mut() {
                c.retry_count = attempt;
            }
            match out {
                Some(s) => {
                    // Succès : n'émettre QU'UN record final (lien N17 correct).
                    if let Some(c) = captured {
                        push_pending(c);
                    }
                    return Some(s);
                }
                None => {
                    last_call = captured;
                    if attempt + 1 < self.attempts {
                        tokio::time::sleep(self.backoff_delay(attempt)).await;
                    }
                }
            }
        }
        // Épuisé : un seul record final (échec), `retry_count = attempts - 1`.
        if let Some(mut c) = last_call {
            c.retry_count = self.attempts.saturating_sub(1);
            push_pending(c);
        }
        None
    }

    fn descriptor(&self) -> BackendDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> bool {
        self.inner.health().await
    }
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

    // ── Retrier (Inc.4.b) : mock qui simule un backend feuille (pousse 1 PendingLlmCall) ──
    struct Recovering {
        fail_until: u32, // nombre d'échecs initiaux avant succès
        calls: AtomicU32,
    }
    #[async_trait]
    impl LocalBackend for Recovering {
        async fn complete(&self, _p: &str, s: SamplingParams) -> Option<String> {
            let n = self.calls.fetch_add(1, Ordering::Relaxed);
            let response = if n >= self.fail_until { Some("ok".to_string()) } else { None };
            // simule le backend feuille : pousse la forensique par-appel (retry_count brut = 0).
            push_pending(PendingLlmCall {
                engine: BackendKind::OpenAiLocal.as_str().to_string(),
                model: "router-model".into(),
                temperature: s.temperature,
                max_tokens: s.max_tokens,
                seed: s.seed,
                prompt: "x".into(),
                response: response.clone(),
                latency_ms: 1,
                retry_count: 0,
            });
            response
        }
        fn descriptor(&self) -> BackendDescriptor {
            BackendDescriptor { kind: BackendKind::OpenAiLocal, model: "router-model".into(), endpoint: "http://router/v1".into() }
        }
    }
    fn fast_retrier(inner: Arc<dyn LocalBackend>, attempts: u32) -> Retrier {
        // délais minuscules : tests rapides (le back-off n'est pas l'objet du test).
        Retrier::new(inner, attempts, Duration::from_millis(1), Duration::from_millis(4))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retrier_emits_single_record_with_retry_count_on_eventual_success() {
        let mock = Arc::new(Recovering { fail_until: 2, calls: AtomicU32::new(0) });
        let retrier = fast_retrier(mock.clone(), 3);
        let sink = Arc::new(Mutex::new(Vec::new()));
        let out = scoped(sink.clone(), retrier.complete("x", params())).await;
        assert_eq!(out.as_deref(), Some("ok"));
        assert_eq!(mock.calls.load(Ordering::Relaxed), 3, "2 échecs + 1 succès");
        let calls = std::mem::take(&mut *sink.lock().unwrap());
        assert_eq!(calls.len(), 1, "UN seul record final (le pont N17 ne voit pas les tentatives)");
        assert_eq!(calls[0].retry_count, 2, "2 re-tentatives consommées");
        assert!(calls[0].response.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retrier_exhausts_and_emits_single_errored_record() {
        let mock = Arc::new(Recovering { fail_until: 99, calls: AtomicU32::new(0) });
        let retrier = fast_retrier(mock.clone(), 3);
        let sink = Arc::new(Mutex::new(Vec::new()));
        let out = scoped(sink.clone(), retrier.complete("x", params())).await;
        assert!(out.is_none(), "toutes les tentatives échouent");
        assert_eq!(mock.calls.load(Ordering::Relaxed), 3, "épuise exactement les 3 tentatives");
        let calls = std::mem::take(&mut *sink.lock().unwrap());
        assert_eq!(calls.len(), 1, "un seul record final, même en échec");
        assert_eq!(calls[0].retry_count, 2);
        assert!(calls[0].response.is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn retrier_first_try_success_is_no_retry() {
        let mock = Arc::new(Recovering { fail_until: 0, calls: AtomicU32::new(0) });
        let retrier = fast_retrier(mock.clone(), 3);
        let sink = Arc::new(Mutex::new(Vec::new()));
        let out = scoped(sink.clone(), retrier.complete("x", params())).await;
        assert_eq!(out.as_deref(), Some("ok"));
        assert_eq!(mock.calls.load(Ordering::Relaxed), 1, "succès immédiat : aucune re-tentative");
        let calls = std::mem::take(&mut *sink.lock().unwrap());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].retry_count, 0, "premier coup → NoRetry");
    }

    #[test]
    fn build_remote_wraps_retrier_and_delegates_descriptor() {
        // allow_remote=true (routeur Network) : Breaker(Retrier(OpenAiLocal)). Le descripteur
        // doit traverser les deux enveloppes intactes (forensique N17 fidèle).
        let b = build_local_backend("http://router.internal:8080/v1", "anthropic/claude-3-5-sonnet", true).unwrap();
        assert_eq!(b.descriptor().kind, BackendKind::OpenAiLocal);
        assert_eq!(b.descriptor().model, "anthropic/claude-3-5-sonnet");
    }
}
