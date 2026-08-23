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
use efe_nano_sdk::{ComputeAttestation, EndpointIdentity};
use serde_json::Value;

use crate::n17::{push_pending, scoped, PendingLlmCall};
use crate::result::SwarmError;

pub(crate) use ollama::OllamaBackend;
pub(crate) use openai_local::OpenAiLocalBackend;

/// Paramètres d'échantillonnage (seed exposé → best-effort de reproductibilité).
//
// `pub` (et non `pub(crate)`) pour que le trait `LocalBackend` — exposé sous `feature = "test-util"`
// (B0.3) — n'enfreigne pas « private-in-public ». Le module `backend` reste privé : sans la feature,
// rien de tout ceci n'est atteignable hors crate.
#[derive(Debug, Clone, Copy)]
pub struct SamplingParams {
    pub max_tokens: u32,
    pub temperature: f64,
    pub seed: u64,
}

/// Moteur local sous-jacent (pour le descripteur forensique / pont N17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
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
pub struct BackendDescriptor {
    pub kind: BackendKind,
    pub model: String,
    pub endpoint: String,
}

/// Backend LLM local **keyless**.
#[async_trait]
pub trait LocalBackend: Send + Sync {
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

    /// **ADR-014 (CP-2)** : capacité **déclarée** du backend — combien d'appels
    /// simultanés ont un sens (Ollama sérialise à `OLLAMA_NUM_PARALLEL`, 1-4 typique :
    /// pousser plus large n'achète que des files et des threads bloquants). Défaut
    /// conservateur **2**. Le SDK ne lit JAMAIS l'environnement (garde-fou G1) : une
    /// capacité supérieure se déclare par largeur EXPLICITE (builder `width(Fixed)`,
    /// DTO `width`, ou env `EFE_SWARM_WIDTH` lu côté passerelle). Déclaratif, pas
    /// mesuré — le breaker (G3) encaisse les déclarations erronées.
    fn suggested_width(&self) -> usize {
        2
    }
}

/// **B0.3 (seam CI)** — backend LOCAL **déterministe et sans réseau** : l'instrument qui rend le
/// chemin Mode C COMPLET testable en CI (un vrai `run_traced` contre des réponses scriptées), sans
/// Ollama ni GPU. [`responding`](ScriptedBackend::responding) renvoie une réponse fixe à chaque appel ;
/// [`failing`](ScriptedBackend::failing) renvoie `None` (échec injecté — le cas que B0.2 doit attraper).
/// Pousse une [`PendingLlmCall`] comme un vrai backend feuille, donc le pont N17 et le scellement Mode C
/// sont réellement exercés. Disponible sous `cfg(test)` (tests inline) ou la feature `test-util` (tests
/// d'intégration). **I-SEAM** : absent de la prod, aucun egress (il ne parle à rien).
#[cfg(any(test, feature = "test-util"))]
pub struct ScriptedBackend {
    response: Option<String>,
    /// **B1 (harnais d'inférence)** : réponses **par-tâche** — `(aiguille, réponse)`. À chaque
    /// `complete`, la PREMIÈRE règle dont l'aiguille est une sous-chaîne du prompt l'emporte. Le
    /// prompt Mode C embarque le texte de la tâche (`gen_prompt` → `Text: "<texte>"`), donc une
    /// aiguille = le texte (unique) d'une ligne de dataset cible exactement cette ligne. Vide ⇒ on
    /// retombe sur `response` (comportement `responding`/`failing` inchangé, byte-identique).
    rules: Vec<(String, String)>,
}

#[cfg(any(test, feature = "test-util"))]
impl ScriptedBackend {
    /// Renvoie `response` à **chaque** appel (le générateur Mode C produira ce candidat).
    pub fn responding(response: impl Into<String>) -> Arc<Self> {
        Arc::new(Self { response: Some(response.into()), rules: Vec::new() })
    }
    /// **Échec injecté** : `complete()` renvoie toujours `None` ⇒ la couche transport replie en
    /// sentinel vide. C'est le scénario « GPU saturé » que le fix sentinel (B0.2) doit gérer.
    pub fn failing() -> Arc<Self> {
        Arc::new(Self { response: None, rules: Vec::new() })
    }
    /// **B1 — justesse contrôlable par-tâche.** À chaque appel, renvoie la réponse de la première
    /// règle `(aiguille, réponse)` dont l'aiguille est une sous-chaîne du prompt ; aucune règle ne
    /// matche ⇒ `None` (échec injecté). Comme le prompt Mode C embarque le texte de la tâche, on
    /// scripte « juste sur ces lignes, faux sur celles-là » en clé-ant les règles sur les textes
    /// (uniques) des lignes — ce qui laisse le banc B1 **vérifier sa propre comptabilité
    /// d'exactitude sur Mac**, sans aucun modèle (I-BENCH-MAC). I-SEAM : inerte/absent en prod.
    pub fn matching(rules: Vec<(String, String)>) -> Arc<Self> {
        Arc::new(Self { response: None, rules })
    }
    /// La réponse pour ce prompt : règle gagnante si des règles existent, sinon la réponse fixe.
    fn pick(&self, prompt: &str) -> Option<String> {
        if !self.rules.is_empty() {
            return self
                .rules
                .iter()
                .find(|(needle, _)| prompt.contains(needle.as_str()))
                .map(|(_, r)| r.clone());
        }
        self.response.clone()
    }
}

#[cfg(any(test, feature = "test-util"))]
#[async_trait]
impl LocalBackend for ScriptedBackend {
    async fn complete(&self, prompt: &str, p: SamplingParams) -> Option<String> {
        // B1 : règle par-tâche si présente (justesse contrôlable), sinon la réponse fixe historique.
        let response = self.pick(prompt);
        // Pont N17 par-appel — fidèle au backend feuille réel (le scellement Mode C est exercé).
        push_pending(PendingLlmCall {
            engine: BackendKind::OpenAiLocal.as_str().to_string(),
            model: "scripted".into(),
            temperature: p.temperature,
            max_tokens: p.max_tokens,
            seed: p.seed,
            prompt: prompt.to_string(),
            response: response.clone(),
            latency_ms: 0,
            retry_count: 0,
            compute_attestation: ComputeAttestation::R2LocalWitnessed,
            model_hash_verified: false,
        });
        response
    }
    fn descriptor(&self) -> BackendDescriptor {
        BackendDescriptor { kind: BackendKind::OpenAiLocal, model: "scripted".into(), endpoint: "scripted://test".into() }
    }
    async fn health(&self) -> bool {
        true // déterministe, sans réseau : toujours sain.
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

// **M1.** Ce module portait sa propre analyse d'autorité — `host_of`, `is_loopback_ipv4`,
// `is_local_endpoint` — déclarée « miroir EXACT de provenance-capture::guard », maintenue d'accord
// par une discipline écrite en commentaire (« les deux copies durcies ENSEMBLE »). Elle a tenu deux
// fois, puis échoué trois fois ailleurs dans le dépôt : une vigilance ne se compile pas.
//
// L'analyse vient désormais de `scella-authority`, une FEUILLE sans dépendance. Le SDK ne gagne
// aucun chemin vers le périmètre preuve, et réciproquement — c'était la seule raison sérieuse
// d'avoir dupliqué.
//
// Ce module a DEUX consommateurs de l'hôte, pas un, et c'est pourquoi il ne suffisait pas
// d'extraire `is_local_endpoint` : `matched_pin` compare l'hôte d'un endpoint à celui d'une
// épingle, ce qui exige que les deux viennent du MÊME analyseur — garantie que le type `Authority`
// rend structurelle au lieu de la laisser au hasard.
use scella_authority::{host_of, is_local_endpoint};

// ─────────────────────────────────────────────────────────────────────────────
//  RC-1 — épinglage d'endpoint distant (G2-remote) + barreau de confiance
// ─────────────────────────────────────────────────────────────────────────────

/// **RC-1** — une **épingle** d'endpoint distant : l'opérateur déclare, hors-bande, le couple
/// (hôte, empreinte cert/SPKI attendue). On épingle l'**empreinte**, pas le hostname (TOCTOU,
/// architecture §5.2) ; `host` ne sert qu'à **apparier** un endpoint à son épingle, et c'est
/// `spki_fingerprint` — la racine de confiance du canal — qui est **enregistrée** dans le record.
#[derive(Debug, Clone)]
pub(crate) struct EndpointPin {
    pub host: String,
    pub spki_fingerprint: String,
}

impl EndpointPin {
    pub fn new(host: impl Into<String>, spki_fingerprint: impl Into<String>) -> Self {
        Self { host: host.into(), spki_fingerprint: spki_fingerprint.into() }
    }
}

/// **RC-1** — l'empreinte épinglée pour `endpoint`, si son hôte figure dans `pins`. `None` ⇒
/// **non épinglé** (le gate G2-remote refusera). Apparie sur l'hôte ; renvoie l'empreinte à
/// enregistrer. (La vérification mTLS réelle — l'empreinte présentée par le serveur == celle
/// épinglée — est l'infra de canal, hors de cet incrément ; ici on pose le schéma + le gate.)
pub(crate) fn matched_pin(endpoint: &str, pins: &[EndpointPin]) -> Option<String> {
    let host = host_of(endpoint)?;
    pins.iter().find(|p| p.host.as_str() == host).map(|p| p.spki_fingerprint.clone())
}

/// **RC-1** — le **barreau de confiance de computation** d'un endpoint, calculé *avant* l'appel
/// et estampillé (scellé) sur chaque record N17 que ce backend produira.
///
/// - **Loopback** ⇒ [`ComputeAttestation::R2LocalWitnessed`] : le récepteur de provenance est
///   co-localisé avec l'inférence, il témoigne de la computation (I-RC3 : R2 exige loopback).
/// - **Distant épinglé** ⇒ [`ComputeAttestation::R4RemoteChannel`] : on atteste la *transaction*
///   sur un canal pinné, jamais la computation. `keyed` distingue le routeur à credential.
///
/// R3 (co-signature T-WITNESS) est un palier **additif** (cf. `NodeCoSignature`) : un nœud pinné
/// hisse R4→R3 en co-signant sa computation. Sa production/vérification ed25519 est l'incrément suivant.
fn attestation_for(endpoint: &str, matched_fingerprint: Option<String>, keyed: bool) -> ComputeAttestation {
    if is_local_endpoint(endpoint) {
        ComputeAttestation::R2LocalWitnessed
    } else {
        ComputeAttestation::R4RemoteChannel {
            endpoint: EndpointIdentity::new(
                // `None` (autorité inanalysable) ⟹ identité VIDE plutôt qu'une chaîne qui ne
                // désigne pas l'hôte réel : ce champ part dans un record scellé.
                host_of(endpoint).map(str::to_string).unwrap_or_default(),
                matched_fingerprint.unwrap_or_default(),
            ),
            keyed,
        }
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
    pins: &[EndpointPin],
) -> Result<Arc<dyn LocalBackend>, SwarmError> {
    let local = is_local_endpoint(endpoint);
    // G2 souveraineté : non-local refusé par défaut (egress interdit).
    if !allow_remote && !local {
        return Err(SwarmError::RemoteBackendDenied { endpoint: endpoint.to_string() });
    }
    // G2-remote (RC-1) : un distant autorisé DOIT être épinglé — sinon refus. Aucun distant
    // non pinné ne peut produire un record (la racine de confiance du canal serait inconnue).
    let matched_fingerprint = if local {
        None
    } else {
        match matched_pin(endpoint, pins) {
            Some(fp) => Some(fp),
            None => return Err(SwarmError::RemoteEndpointNotPinned { endpoint: endpoint.to_string() }),
        }
    };
    // RC-1 : le barreau de confiance, scellé sur chaque record. `keyed = false` ici — le chemin
    // SDK est self-host (zéro-clé) ; le routeur keyé (Network) est résolu côté passerelle, où
    // l'estampillage `keyed = true` est un incrément ultérieur.
    let attestation = attestation_for(endpoint, matched_fingerprint, false);
    let inner: Arc<dyn LocalBackend> = if let Some(m) = model.strip_prefix("openai:") {
        OpenAiLocalBackend::new(endpoint.to_string(), m.to_string(), attestation)
    } else if endpoint.contains("/v1") {
        OpenAiLocalBackend::new(endpoint.to_string(), model.to_string(), attestation)
    } else {
        OllamaBackend::new(endpoint.to_string(), model.to_string(), attestation)
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

    fn suggested_width(&self) -> usize {
        self.inner.suggested_width() // la capacité traverse l'enveloppe (comme le descripteur)
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
    fn suggested_width(&self) -> usize {
        self.inner.suggested_width()
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

    /// **Régression SSRF/egress (G2)** — la garde de souveraineté DOIT s'accorder avec l'hôte que
    /// curl/reqwest contactent réellement. Ces endpoints exhibaient un faux loopback via un différentiel
    /// d'analyseur (fragment/query + `userinfo@` pris au dernier `@`) ou un `starts_with("127.")` naïf :
    /// tous doivent être refusés (sinon un agent exfiltre le prompt hors frontière). Cf. `provenance-capture::guard`.
    #[test]
    fn ssrf_egress_bypasses_are_closed() {
        for e in [
            "http://evil.com#@127.0.0.1",     // fragment : l'autorité est evil.com, pas 127.0.0.1
            "http://evil.com?x@127.0.0.1",    // query : idem
            "http://evil.com/x@127.0.0.1",    // path : idem
            "http://127.evil.com",            // NOM préfixé 127. (résout externe) ≠ loopback IPv4
            "http://127.0.0.1@evil.com",      // userinfo = 127.0.0.1, hôte RÉEL = evil.com
            "http://user:127.0.0.1@evil.com", // userinfo = user:127.0.0.1, hôte = evil.com
            "http://evil.com\\@127.0.0.1/v1", // §3.2 (F3) backslash : reqwest normalise \→/ → evil.com
            "http://127.0.0.1 @evil.com",     // §3.2 (F3) espace dans l'autorité
        ] {
            assert!(!is_local_endpoint(e), "bypass egress doit être refusé : {e}");
        }
        // L'hôte extrait est bien evil.com (pas le leurre loopback) : l'extracteur s'accorde avec curl.
        assert_eq!(host_of("http://evil.com#@127.0.0.1"), Some("evil.com"));
        assert_eq!(host_of("http://evil.com?x@127.0.0.1"), Some("evil.com"));
        assert_eq!(host_of("http://127.0.0.1@evil.com"), Some("evil.com"));
        assert_eq!(host_of("http://"), None, "autorité vide → pas d'hôte (fail-closed)");
        // §3.2 (F3) : les autorités WHATWG-divergentes ⟹ None (fail-closed), jamais faussement local.
        assert_eq!(host_of("http://evil.com\\@127.0.0.1/v1"), None, "backslash ⟹ None");
    }

    /// La normalisation (casse + point final de FQDN) corrige des FAUX NÉGATIFS (hôtes légitimement
    /// locaux) sans ouvrir de porte : un nom externe préfixé localhost/127 reste refusé.
    #[test]
    fn local_forms_are_normalized_without_opening_egress() {
        for e in [
            "http://127.0.0.1:11434",
            "http://[::1]:8080",
            "http://LOCALHOST:8000",   // casse
            "http://localhost.:8000",  // point final de FQDN
            "http://127.0.0.1.:11434", // IPv4 + point final
            "http://127.255.0.1:9",    // 127.0.0.0/8, pas seulement 127.0.0.1
        ] {
            assert!(is_local_endpoint(e), "hôte local légitime, devrait passer : {e}");
        }
        for e in ["http://localhost.evil.com", "http://127.0.0.1.evil.com"] {
            assert!(!is_local_endpoint(e), "nom externe, ne doit PAS passer : {e}");
        }
    }

    /// **Interaction RC-1 (G2-remote)** : un endpoint qui usurpait le loopback via le différentiel
    /// d'analyseur est désormais classé DISTANT → il retombe sous le gate d'épinglage. Sans `allow_remote`
    /// il est refusé (egress) ; avec `allow_remote` mais SANS épingle correspondante il est refusé (non
    /// pinné) ; l'épingle s'apparie sur l'hôte RÉEL (evil.com), jamais sur le leurre. Le fix RENFORCE
    /// l'épinglage, il ne l'affaiblit pas.
    #[test]
    fn ssrf_bypass_endpoint_is_routed_through_remote_pin_gate() {
        let bypass = "http://evil.com#@127.0.0.1";
        // egress refusé par défaut (plus de faux « local » qui sautait toute épingle).
        assert!(matches!(
            build_local_backend(bypass, "gemma3:12b", false, &[]),
            Err(SwarmError::RemoteBackendDenied { .. })
        ));
        // egress autorisé mais non épinglé → refusé (G2-remote).
        assert!(matches!(
            build_local_backend(bypass, "gemma3:12b", true, &[]),
            Err(SwarmError::RemoteEndpointNotPinned { .. })
        ));
        // l'épingle s'apparie sur l'hôte RÉEL (evil.com), pas sur le leurre 127.0.0.1.
        assert!(matched_pin(bypass, &[EndpointPin::new("evil.com", "sha256/PIN")]).is_some(), "apparié sur evil.com");
        assert!(matched_pin(bypass, &[EndpointPin::new("127.0.0.1", "sha256/PIN")]).is_none(), "jamais sur le leurre");
        let pins = [EndpointPin::new("evil.com", "sha256/PIN")];
        assert!(build_local_backend(bypass, "gemma3:12b", true, &pins).is_ok(), "distant épinglé (sur l'hôte réel) → construit");
    }

    #[test]
    fn build_denies_remote_by_default_and_allows_with_flag() {
        // garde-fou souveraineté (G2) : non-local refusé par défaut.
        let err = build_local_backend("http://example.com:11434", "gemma3:12b", false, &[]);
        assert!(matches!(err, Err(SwarmError::RemoteBackendDenied { .. })));
        // G2-remote (RC-1) : egress autorisé MAIS endpoint non épinglé → refusé.
        let unpinned = build_local_backend("http://example.com:11434", "gemma3:12b", true, &[]);
        assert!(
            matches!(unpinned, Err(SwarmError::RemoteEndpointNotPinned { .. })),
            "un distant non pinné doit être refusé (aucun distant en clair)"
        );
        // egress autorisé + endpoint ÉPINGLÉ → autorisé (construit, sans appel réseau).
        let pins = [EndpointPin::new("example.com", "sha256/PINTEST")];
        assert!(
            build_local_backend("http://example.com:11434", "gemma3:12b", true, &pins).is_ok(),
            "un distant épinglé doit être autorisé"
        );
        // local : toujours OK, sans épingle.
        assert!(build_local_backend("http://127.0.0.1:11434", "gemma3:12b", false, &[]).is_ok());
    }

    #[test]
    fn build_selects_engine_by_endpoint_and_model_prefix() {
        let ollama = build_local_backend("http://127.0.0.1:11434", "gemma3:12b", false, &[]).unwrap();
        assert_eq!(ollama.descriptor().kind, BackendKind::Ollama);
        let openai_by_path = build_local_backend("http://127.0.0.1:8080/v1", "qwen", false, &[]).unwrap();
        assert_eq!(openai_by_path.descriptor().kind, BackendKind::OpenAiLocal);
        let openai_by_prefix = build_local_backend("http://127.0.0.1:8000", "openai:llama3", false, &[]).unwrap();
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
                compute_attestation: ComputeAttestation::R2LocalWitnessed,
                model_hash_verified: false,
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
        // doit traverser les deux enveloppes intactes (forensique N17 fidèle). Endpoint ÉPINGLÉ.
        let pins = [EndpointPin::new("router.internal", "sha256/ROUTERPIN")];
        let b = build_local_backend("http://router.internal:8080/v1", "anthropic/claude-3-5-sonnet", true, &pins).unwrap();
        assert_eq!(b.descriptor().kind, BackendKind::OpenAiLocal);
        assert_eq!(b.descriptor().model, "anthropic/claude-3-5-sonnet");
    }
}
