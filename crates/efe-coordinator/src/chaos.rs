// SPDX-License-Identifier: UNLICENSED
//! **R3.S6 — Chaos engineering : injection de fautes réelles (fix L3).**
//!
//! S8-R2 a livré une graceful degradation complète (fallback chain LLM, vote
//! dégradé Mission B, drafter bypass Speculative, SLM circuit skip), mais elle
//! n'était testée que contre `MockLlmProvider::with_error()` — un échec
//! synthétique, instantané, sans réseau. **L3** : rien ne prouvait que ces
//! chemins se déclenchent contre de *vraies* fautes (429 Anthropic, timeouts
//! réseau, JSON tronqué).
//!
//! Ce module fournit un **décorateur `LlmProvider`** ([`ChaosLlmProvider`]) qui
//! enveloppe un provider (mock OU réel) et injecte des fautes selon un
//! `fault_rate ∈ [0,1]`, piloté par un [`ChaosInjector`].
//!
//! ## Propriétés de design (cf. architecture R3.S6)
//!
//! - **Zéro modif des workflows** : l'injection passe par le `ProviderRegistry`
//!   sous l'**id du provider wrappé** ([`ChaosLlmProvider::id`] délègue) → le
//!   router et la fallback chain sélectionnent le wrapper de façon
//!   transparente. Le code de prod testé est *littéralement inchangé*.
//! - **Zéro dépendance externe** : PRNG SplitMix64 interne ([`ChaosRng`]) au
//!   lieu de `rand` (cohérent avec le refus de dépendances du projet).
//! - **Déterminisme** : seedé → séquence de fautes reproductible (tests
//!   stables). `from_env` seede depuis l'entropie pour les runs live.
//! - **Injection dans `call()` ET `stream()`** : Mission A passe par `stream`,
//!   Mission B par `call`, le Speculator par les deux — décorer un seul
//!   laisserait Mission A non couverte.
//! - **`Timeout` borné** ([`CHAOS_MAX_SLEEP`]) : les workflows n'enforcent pas
//!   de hard-deadline (la deadline est advisory) ; un sleep non borné ferait
//!   hang la CI.
//! - **Fidélité d'erreur** : émet [`CoordError::LlmProvider`] — exactement ce
//!   que les vrais providers émettent → les chemins de fallback se comportent
//!   comme en prod.
//! - **`fault_rate = 0.0` ⇒ pass-through transparent** (n'épuise même pas le
//!   RNG).
//!
//! Le module est gardé par `#[cfg(feature = "chaos")]` : OFF en prod (binaire
//! propre, pas de chaos accidentel), ON en test/CI.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::{CoordError, CoordResult};
use crate::llm::{LlmCall, LlmProvider, LlmResponse, ProviderId, TokenChunk};

/// Plafond dur du sleep injecté par [`ChaosFaultKind::Timeout`].
///
/// Les workflows n'enforcent **pas** de hard-deadline (`LlmCall.deadline` est
/// advisory, aucun `tokio::time::timeout`). Un sleep injecté non borné ferait
/// donc traîner — voire hang — la mission. On plafonne ici pour garder la CI
/// fail-fast tout en exerçant le chemin « provider lent qui finit par
/// échouer ».
pub const CHAOS_MAX_SLEEP: Duration = Duration::from_secs(5);

/// Constante de gamma de SplitMix64 (2^64 / φ, impair).
const SPLITMIX64_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// Nature de la faute injectée.
///
/// `HttpStatus`/`Timeout` produisent une **erreur** (déclenche fallback) ;
/// `InvalidResponse`/`EmptyResponse` produisent un **faux succès** (un
/// `LlmResponse` dégénéré) qui teste la détection de contenu absent côté
/// consommateur (noise gate R3.S3, vote, Jaccard du Speculator).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaosFaultKind {
    /// Force un échec type HTTP (429/503…). Émet `CoordError::LlmProvider`.
    HttpStatus(u16),
    /// Dort `min(d, CHAOS_MAX_SLEEP)` puis échoue (provider lent réaliste).
    Timeout(Duration),
    /// Retourne `Ok` avec un texte non-parsable / tronqué (≠ erreur).
    InvalidResponse,
    /// Retourne `Ok` avec un texte vide (≠ erreur).
    EmptyResponse,
}

/// PRNG déterministe seedé — **SplitMix64**, zéro dépendance externe.
///
/// Thread-safe via `AtomicU64` (`Relaxed` suffit : on ne veut qu'une séquence
/// pseudo-aléatoire reproductible, pas une garantie d'ordre inter-thread).
#[derive(Debug)]
struct ChaosRng {
    state: AtomicU64,
}

impl ChaosRng {
    fn new(seed: u64) -> Self {
        Self {
            state: AtomicU64::new(seed),
        }
    }

    /// Tirage uniforme dans `[0, 1)` (24 bits de mantisse → exact en f32).
    fn next_f32(&self) -> f32 {
        // `fetch_add` renvoie l'ancien état ; le nouvel état stocké = old + γ.
        // On reconstruit ce nouvel état pour le mixer (SplitMix64 standard).
        let mut z = self
            .state
            .fetch_add(SPLITMIX64_GAMMA, Ordering::Relaxed)
            .wrapping_add(SPLITMIX64_GAMMA);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        // Top 24 bits → [0, 2^24) → normalisé [0,1).
        ((z >> 40) as f32) / ((1u32 << 24) as f32)
    }
}

/// Configuration : quels providers fautent, à quel taux, de quelle façon.
///
/// Partagé en `Arc` : un même injecteur peut décorer plusieurs providers
/// (Mission B) tout en gardant un état RNG cohérent.
#[derive(Debug)]
pub struct ChaosInjector {
    /// provider_id → fault_rate ∈ [0,1]. Absent ⇒ 0.0 (jamais).
    fault_rates: HashMap<String, f32>,
    /// Nature de faute appliquée à tous les providers configurés.
    fault_kind: ChaosFaultKind,
    rng: ChaosRng,
}

impl ChaosInjector {
    /// Constructeur **seedé** (tests reproductibles).
    #[must_use]
    pub fn seeded(fault_kind: ChaosFaultKind, seed: u64) -> Self {
        Self {
            fault_rates: HashMap::new(),
            fault_kind,
            rng: ChaosRng::new(seed),
        }
    }

    /// Builder : ajoute un provider faillible (taux clampé à `[0,1]`).
    #[must_use]
    pub fn with_fault_rate(mut self, provider_id: impl Into<String>, rate: f32) -> Self {
        self.fault_rates
            .insert(provider_id.into(), rate.clamp(0.0, 1.0));
        self
    }

    /// **Live** : lit `EFE_CHAOS_RATE` (défaut `0.0`), seede depuis l'entropie
    /// horloge, et applique le taux à TOUS les `provider_ids` fournis.
    ///
    /// `EFE_CHAOS_RATE` absent / invalide ⇒ `0.0` ⇒ no-op (le chaos n'est
    /// jamais actif « par défaut », même feature activée).
    #[must_use]
    pub fn from_env(fault_kind: ChaosFaultKind, provider_ids: &[String]) -> Self {
        let rate = std::env::var("EFE_CHAOS_RATE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xDEAD_BEEF)
            // Mélange initial pour éviter un seed proche de 0.
            .wrapping_mul(SPLITMIX64_GAMMA)
            | 1;
        let mut inj = Self::seeded(fault_kind, seed);
        for id in provider_ids {
            inj.fault_rates.insert(id.clone(), rate);
        }
        inj
    }

    /// Taux configuré pour un provider (`0.0` si absent).
    #[must_use]
    pub fn fault_rate(&self, provider_id: &str) -> f32 {
        self.fault_rates.get(provider_id).copied().unwrap_or(0.0)
    }

    /// La nature de faute injectée.
    #[must_use]
    pub fn fault_kind(&self) -> &ChaosFaultKind {
        &self.fault_kind
    }

    /// Décide — **et consomme un tirage RNG** — si CE provider doit fauter
    /// maintenant. Court-circuite (sans tirage) quand le taux est nul :
    /// préserve l'invariant pass-through `fault_rate = 0.0`.
    ///
    /// **Déterminisme** : pleinement reproductible pour un seed donné quand les
    /// appels sont sérialisés OU quand `rate ∈ {0.0, 1.0}` (où le tirage ne
    /// change pas la décision). En Mission B, le fan-out (`call_many`) invoque
    /// les providers en parallèle : si **plusieurs** providers partagent cet
    /// injecteur avec `0 < rate < 1`, l'ordre des tirages atomiques n'est pas
    /// garanti → l'affectation des fautes devient non-déterministe (le *taux*
    /// agrégé reste correct). Les tests offline contournent ça en ne wrappant
    /// qu'un provider, ou avec `rate = 1.0`.
    #[must_use]
    pub fn should_fault(&self, provider_id: &str) -> bool {
        let rate = self.fault_rate(provider_id);
        rate > 0.0 && self.rng.next_f32() < rate
    }

    /// Enveloppe un provider réel dans un [`ChaosLlmProvider`] partageant cet
    /// injecteur.
    #[must_use]
    pub fn wrap(self: Arc<Self>, inner: Arc<dyn LlmProvider>) -> Arc<ChaosLlmProvider> {
        Arc::new(ChaosLlmProvider {
            inner,
            injector: self,
        })
    }
}

/// Décorateur `LlmProvider` : délègue à `inner`, faute selon `injector`.
///
/// **Préserve l'id du provider wrappé** → router + fallback chain
/// transparents (cf. doc module).
pub struct ChaosLlmProvider {
    inner: Arc<dyn LlmProvider>,
    injector: Arc<ChaosInjector>,
}

impl ChaosLlmProvider {
    /// Construit une réponse dégénérée selon `EmptyResponse` / `InvalidResponse`.
    /// `empty == true` → texte vide ; sinon JSON tronqué non-parsable.
    fn degenerate_response(&self, empty: bool) -> LlmResponse {
        let text = if empty {
            String::new()
        } else {
            // Texte clairement non-parsable / tronqué (mais non vide).
            "{ \"incomplete\": tru".to_string()
        };
        let tokens = if empty {
            0
        } else {
            text.split_whitespace().count() as u32
        };
        LlmResponse {
            text,
            tokens_generated: tokens,
            cost_cents: 0,
            cost_microcents: 0,
            latency_ms: 0,
            provider: self.inner.id(),
            // Qualité plancher : un consommateur quality-aware (vote) doit le
            // déprioriser.
            self_quality: 0.0,
        }
    }

    /// Construit l'erreur injectée (HttpStatus / Timeout), après sleep borné
    /// le cas échéant. Suppose que le kind est bien l'un des deux variants
    /// "erreur" (les variants "faux succès" sont gérés en amont).
    async fn injected_error(&self) -> CoordError {
        let provider = self.inner.id();
        match &self.injector.fault_kind {
            ChaosFaultKind::HttpStatus(code) => CoordError::LlmProvider {
                provider,
                reason: format!("chaos: HTTP {code}"),
            },
            ChaosFaultKind::Timeout(d) => {
                let bounded = (*d).min(CHAOS_MAX_SLEEP);
                tokio::time::sleep(bounded).await;
                CoordError::LlmProvider {
                    provider,
                    reason: format!("chaos: timeout after {}ms", d.as_millis()),
                }
            }
            // Les faux succès ne devraient pas atteindre ce helper.
            ChaosFaultKind::InvalidResponse | ChaosFaultKind::EmptyResponse => {
                CoordError::LlmProvider {
                    provider,
                    reason: "chaos: unexpected degenerate kind in error path".to_string(),
                }
            }
        }
    }
}

#[async_trait]
impl LlmProvider for ChaosLlmProvider {
    fn id(&self) -> ProviderId {
        // Transparence : le router/fallback voient l'id réel du provider wrappé.
        self.inner.id()
    }

    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
        let id = self.inner.id();
        if self.injector.should_fault(&id) {
            return match self.injector.fault_kind {
                ChaosFaultKind::EmptyResponse => Ok(self.degenerate_response(true)),
                ChaosFaultKind::InvalidResponse => Ok(self.degenerate_response(false)),
                ChaosFaultKind::HttpStatus(_) | ChaosFaultKind::Timeout(_) => {
                    Err(self.injected_error().await)
                }
            };
        }
        self.inner.call(req).await
    }

    async fn stream(
        &self,
        req: LlmCall,
        tx: mpsc::Sender<TokenChunk>,
    ) -> CoordResult<LlmResponse> {
        let id = self.inner.id();
        if self.injector.should_fault(&id) {
            return match self.injector.fault_kind {
                ChaosFaultKind::EmptyResponse => {
                    // N'émet aucun chunk → réponse vide.
                    Ok(self.degenerate_response(true))
                }
                ChaosFaultKind::InvalidResponse => {
                    let resp = self.degenerate_response(false);
                    // Émet le texte dégénéré comme un unique chunk.
                    let chunk = TokenChunk {
                        token: resp.text.clone(),
                        index: 0,
                        emitted_at: chrono::Utc::now(),
                    };
                    let _ = tx.send(chunk).await;
                    Ok(resp)
                }
                ChaosFaultKind::HttpStatus(_) | ChaosFaultKind::Timeout(_) => {
                    // N'émet aucun chunk, échoue (éventuellement après sleep borné).
                    Err(self.injected_error().await)
                }
            };
        }
        self.inner.stream(req, tx).await
    }

    async fn health(&self) -> CoordResult<bool> {
        // Pass-through : le health check n'est pas dans le chemin de mission.
        self.inner.health().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::MockLlmProvider;
    use serial_test::serial;

    // ================================================================
    // 1-4 : ChaosRng + ChaosInjector
    // ================================================================

    #[test]
    fn chaos_rng_deterministic_for_seed() {
        let a = ChaosRng::new(42);
        let b = ChaosRng::new(42);
        let seq_a: Vec<f32> = (0..16).map(|_| a.next_f32()).collect();
        let seq_b: Vec<f32> = (0..16).map(|_| b.next_f32()).collect();
        assert_eq!(seq_a, seq_b, "même seed → même séquence");
        // Toutes les valeurs dans [0,1).
        assert!(seq_a.iter().all(|&x| (0.0..1.0).contains(&x)));
        // Un seed différent diverge (extrêmement improbable d'être identique).
        let c = ChaosRng::new(43);
        let seq_c: Vec<f32> = (0..16).map(|_| c.next_f32()).collect();
        assert_ne!(seq_a, seq_c, "seed différent → séquence différente");
    }

    #[test]
    fn chaos_injector_zero_rate_never_faults() {
        let inj = ChaosInjector::seeded(ChaosFaultKind::HttpStatus(429), 7)
            .with_fault_rate("claude", 0.0);
        for _ in 0..1_000 {
            assert!(!inj.should_fault("claude"), "taux 0 → jamais de faute");
        }
        // Provider non configuré → 0 aussi.
        assert!(!inj.should_fault("unknown"));
    }

    #[test]
    fn chaos_injector_full_rate_always_faults() {
        let inj = ChaosInjector::seeded(ChaosFaultKind::HttpStatus(503), 99)
            .with_fault_rate("gemini", 1.0);
        for _ in 0..1_000 {
            assert!(inj.should_fault("gemini"), "taux 1 → toujours");
        }
    }

    #[test]
    fn chaos_injector_basic_fault_rate() {
        let inj = ChaosInjector::seeded(ChaosFaultKind::HttpStatus(429), 0xC0FFEE)
            .with_fault_rate("p", 0.3);
        let n = 10_000;
        let faults = (0..n).filter(|_| inj.should_fault("p")).count();
        let freq = faults as f32 / n as f32;
        // Loi des grands nombres + seed fixe → fréquence proche de 0.30.
        assert!(
            (0.27..=0.33).contains(&freq),
            "fréquence observée {freq} hors [0.27, 0.33]"
        );
    }

    // `from_env` lit `EFE_CHAOS_RATE` : verrouille le contrat d'environnement
    // câblé par le workflow CI `chaos-eng.yml`. `#[serial]` car la variable est
    // un nom global partagé (pas de collision parallèle).
    #[test]
    #[serial]
    fn chaos_injector_from_env_reads_rate() {
        std::env::set_var("EFE_CHAOS_RATE", "1.0");
        let inj =
            ChaosInjector::from_env(ChaosFaultKind::HttpStatus(429), &["p".to_string()]);
        assert_eq!(inj.fault_rate("p"), 1.0, "EFE_CHAOS_RATE=1.0 appliqué à 'p'");
        assert!(inj.should_fault("p"), "rate 1.0 → toujours faute (seed-indépendant)");
        // Un provider non listé reste à 0.
        assert_eq!(inj.fault_rate("other"), 0.0);
        std::env::remove_var("EFE_CHAOS_RATE");
    }

    #[test]
    #[serial]
    fn chaos_injector_from_env_absent_is_noop() {
        std::env::remove_var("EFE_CHAOS_RATE");
        let inj =
            ChaosInjector::from_env(ChaosFaultKind::HttpStatus(429), &["p".to_string()]);
        assert_eq!(inj.fault_rate("p"), 0.0, "var absente ⇒ 0.0 (no-op fail-safe)");
        for _ in 0..100 {
            assert!(!inj.should_fault("p"), "rate 0 → jamais de faute");
        }
    }

    // ================================================================
    // 5-8 : ChaosLlmProvider
    // ================================================================

    #[test]
    fn chaos_provider_preserves_provider_id() {
        let inner = Arc::new(MockLlmProvider::new("claude_real"));
        let inj = Arc::new(ChaosInjector::seeded(ChaosFaultKind::HttpStatus(429), 1));
        let wrapped = inj.wrap(inner);
        assert_eq!(
            wrapped.id(),
            "claude_real",
            "le wrapper préserve l'id du provider wrappé (transparence router)"
        );
    }

    #[tokio::test]
    async fn chaos_provider_zero_rate_passthrough() {
        let inner = Arc::new(
            MockLlmProvider::new("p")
                .with_response("hello world")
                .with_cost_microcents(123),
        );
        // Pas de fault_rate configuré → 0 → pass-through pur.
        let inj = Arc::new(ChaosInjector::seeded(ChaosFaultKind::HttpStatus(429), 5));
        let wrapped = inj.wrap(inner);
        let r = wrapped.call(LlmCall::simple("?", 10)).await.unwrap();
        assert_eq!(r.text, "hello world", "réponse identique au wrappé");
        assert_eq!(r.cost_microcents, 123);
        assert_eq!(r.provider, "p");
    }

    #[tokio::test]
    async fn chaos_provider_emits_http_429_at_rate() {
        let inner = Arc::new(MockLlmProvider::new("claude").with_response("ok"));
        let inj = Arc::new(
            ChaosInjector::seeded(ChaosFaultKind::HttpStatus(429), 3).with_fault_rate("claude", 1.0),
        );
        let wrapped = inj.wrap(inner);

        // call() faute.
        let err = wrapped.call(LlmCall::simple("?", 10)).await.unwrap_err();
        match err {
            CoordError::LlmProvider { reason, .. } => {
                assert!(reason.contains("429"), "reason doit porter le code 429: {reason}");
            }
            other => panic!("attendu LlmProvider, got {other:?}"),
        }

        // stream() faute aussi (sinon Mission A non couverte).
        let (tx, mut rx) = mpsc::channel(16);
        let res = wrapped.stream(LlmCall::simple("?", 10), tx).await;
        assert!(res.is_err(), "stream doit aussi fauter");
        // Aucun chunk émis sur une faute HTTP.
        assert!(rx.try_recv().is_err(), "pas de chunk sur faute 429");
    }

    #[tokio::test]
    async fn chaos_provider_emits_timeout_bounded() {
        let inner = Arc::new(MockLlmProvider::new("slow").with_response("ok"));
        // Timeout 1h demandé → doit être borné à CHAOS_MAX_SLEEP.
        let inj = Arc::new(
            ChaosInjector::seeded(ChaosFaultKind::Timeout(Duration::from_secs(3600)), 11)
                .with_fault_rate("slow", 1.0),
        );
        let wrapped = inj.wrap(inner);
        let started = std::time::Instant::now();
        let err = wrapped.call(LlmCall::simple("?", 10)).await.unwrap_err();
        let elapsed = started.elapsed();
        assert!(
            elapsed <= CHAOS_MAX_SLEEP + Duration::from_millis(500),
            "le sleep doit être borné à CHAOS_MAX_SLEEP (+ε), got {elapsed:?}"
        );
        match err {
            CoordError::LlmProvider { reason, .. } => {
                assert!(reason.contains("timeout"), "reason: {reason}");
            }
            other => panic!("attendu LlmProvider, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn chaos_provider_empty_response_is_ok_but_empty() {
        let inner = Arc::new(MockLlmProvider::new("e").with_response("real answer"));
        let inj = Arc::new(
            ChaosInjector::seeded(ChaosFaultKind::EmptyResponse, 2).with_fault_rate("e", 1.0),
        );
        let wrapped = inj.wrap(inner);
        let r = wrapped.call(LlmCall::simple("?", 10)).await.unwrap();
        assert_eq!(r.text, "", "EmptyResponse → texte vide (faux succès)");
        assert_eq!(r.tokens_generated, 0);
        assert_eq!(r.self_quality, 0.0);
    }
}
