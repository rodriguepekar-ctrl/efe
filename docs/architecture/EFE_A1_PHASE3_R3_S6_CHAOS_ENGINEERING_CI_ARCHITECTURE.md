# EFE A1 — Phase 3 R3.S6 — Chaos engineering CI (fix L3)

> **Statut** : architecture (à implémenter)
> **Sprint** : Phase 3 R3.S6
> **Priorité roadmap** : 🔴 high (résilience prouvée vs simulée)
> **Pré-requis** : S8-R2 (FailureMode + fallback chain), R3.S2 (SlmCircuitBreaker),
> R3.S4 (drafter circuit), R3.S5 (multi-tenant — orthogonal)
> **Fichiers cible** : `crates/efe-coordinator/src/chaos.rs` (NEW),
> `Cargo.toml` (feature `chaos`), `lib.rs`, `tests/chaos_offline.rs` (NEW),
> `tests/live_chaos.rs` (NEW), `.github/workflows/chaos-eng.yml` (NEW)

---

## 0. Résumé exécutif

S8-R2 a livré une **graceful degradation** complète (fallback chain LLM, vote
dégradé Mission B, drafter bypass Speculative, SLM circuit skip) — mais elle
n'a **jamais été validée contre de vraies fautes**. Toute la couche de
résilience est testée via `MockLlmProvider::with_error()` (`llm/mock.rs:67`),
qui retourne un `CoordError::LlmProvider` synthétique **déterministe à 100 %**.

**Le risque L3** : rien ne garantit que la fallback chain fonctionne quand
Anthropic part *réellement* en 429, quand un provider renvoie un JSON
tronqué, ou quand `llama-server` devient injoignable sous charge. Les chemins
de dégradation peuvent diverger subtilement entre l'erreur mockée et l'erreur
réseau réelle (codes de statut, latence, ordering, partial reads).

R3.S6 livre un **framework de chaos engineering** :

1. **`ChaosLlmProvider`** — un décorateur `LlmProvider` qui enveloppe un
   provider (mock OU réel) et injecte des fautes (`HttpStatus`, `Timeout`,
   `InvalidResponse`, `EmptyResponse`) selon un `fault_rate ∈ [0,1]`.
2. **`ChaosInjector`** — configuration par provider-id + kind, avec un PRNG
   **déterministe seedé** (zéro nouvelle dépendance) pour des tests
   reproductibles, et `from_env()` pour les runs live (`EFE_CHAOS_RATE`).
3. **Tests offline** (mock + chaos) prouvant que chaque mission dégrade
   correctement, et **tests live** (Claude/Gemini réels sous chaos) prouvant
   la résilience en condition réseau réelle.
4. **CI nightly `chaos-eng.yml`** exécutant les tests live sous injection.

**Critère central** : > 80 % des missions complètent (réponse correcte +
`failure_mode` correctement tracé) sous 30 % de fault rate, en condition
réelle.

**Principe directeur (hérité R3.S3/S4/S5)** : *zéro nouvelle dépendance non
justifiée* (PRNG interne au lieu de `rand`), *zéro dead config* (chaque
`ChaosFaultKind` exerce un chemin de dégradation réel et observable), et
*honnêteté chirurgicale* (le chaos `Timeout` révèle un gap de hard-deadline —
documenté, §11.2, pas masqué).

---

## 1. Scope / Non-scope

### 1.1 Dans le scope R3.S6

| # | Livrable | Vecteur |
|---|---|---|
| 1 | `ChaosFaultKind` (4 variants) | provider-level |
| 2 | `ChaosInjector` (fault_rates par id + PRNG seedé + `from_env`) | config |
| 3 | `ChaosLlmProvider` décorant `call()` **et** `stream()` | provider-level |
| 4 | Feature `chaos` (off en prod, on en test/CI) | build |
| 5 | 12 tests offline (4 unit injector + 4 unit provider + 4 intégration mission) | offline |
| 6 | 4 tests live (Mission A 429, Mission B timeout, Spec invalid, SLM down) | live |
| 7 | CI `chaos-eng.yml` nightly | infra |
| 8 | Chaos SLM = redirection d'endpoint (réutilise SlmCircuitBreaker R3.S2) | infra-level |

### 1.2 Hors scope (déféré, justifié §17)

- **Hard-deadline enforcement** (`tokio::time::timeout` autour de
  `stream_one`) — gap réel révélé par le chaos `Timeout` (§11.2), mais c'est
  un changement de comportement des workflows, sprint à part. S6 **borne** le
  fault `Timeout` pour ne pas hang la CI et **documente** le gap.
- **Retry avec backoff** — le bridge n'a aucun retry (§3) ; l'ajouter est
  orthogonal au chaos (S6 valide le fallback, pas le retry).
- **Chaos sur le bus / blackboard / DAG** — S6 cible la couche provider/SLM
  (la surface de panne externe). Le chaos interne est un autre sprint.
- **Fault injection adversariale** (réponses malveillantes, prompt injection)
  — relève de la sécurité, pas de la résilience.

### 1.3 Invariant de non-régression

> La feature `chaos` désactivée (défaut prod) ⇒ `src/chaos.rs` n'est pas
> compilé, aucun symbole exporté, zéro impact. Un `ChaosLlmProvider` avec
> `fault_rate = 0.0` ⇒ **pass-through transparent** (délègue intégralement au
> provider wrappé, byte-pour-byte).

---

## 2. Problème (limitation L3)

### L3 — Résilience simulée, jamais éprouvée

Toute la résilience S8-R2 repose sur `MockLlmProvider::with_error()`
(`mock.rs:67`), qui fait :

```rust
if let Some(reason) = self.always_fail_reason.as_ref() {
    return Err(CoordError::LlmProvider { provider: self.id.clone(), reason });
}
```

C'est un échec **instantané, déterministe, sans réseau**. Les tests
(`workflow_failure_modes.rs`) vérifient que le fallback se déclenche — mais
contre cette erreur synthétique uniquement. Aucune garantie sur :

- **Codes de statut réels** : un vrai 429 Anthropic vient avec des headers
  `retry-after`, un body JSON d'erreur, parfois après un partial read. Le
  provider réel mappe-t-il ça vers un `CoordError` que le fallback attrape ?
- **Latence réelle** : un timeout réseau n'est pas instantané. Le `deadline`
  (`LlmCall.deadline`) est-il respecté ? (Spoiler §3 : non — advisory.)
- **Réponses dégénérées** : JSON tronqué, texte vide. Le parsing provider
  les rejette-t-il proprement, ou produit-il un `LlmResponse` vide qui passe
  silencieusement (faux succès) ?
- **Comportement sous charge** : 30 % de fautes sur 20 missions consécutives
  — le circuit breaker, le fallback, le vote dégradé tiennent-ils ?

**Conséquence** : la promesse marketing « résilient, graceful degradation »
n'est pas prouvée. Un incident prod (Anthropic 429 généralisé) pourrait
révéler que le fallback ne se déclenche pas comme testé.

---

## 3. État actuel du code (inventaire ancré)

| Élément | Emplacement | Constat |
|---|---|---|
| `LlmProvider` trait (`id`, `call`, `stream`, `health`) | `llm/provider.rs:201-236` | surface à décorer |
| `stream()` default (split sur call) | `provider.rs:213-230` | chaos doit override |
| `MockLlmProvider::with_error` | `llm/mock.rs:67` | erreur synthétique déterministe |
| `MockLlmProvider` faults (`fail_after`, `always_fail_reason`) | `mock.rs:31-35` | pas de fault_rate proba |
| `FailureMode` (None/SlmDown/LlmFallbackTriggered/VoteDegraded/DrafterBypassed/Composite) | `failure.rs:40-70` | cible des assertions chaos |
| Fallback chain Mission A | `workflows/mission_a.rs:434-513` | candidates = [selected, …others] ; tout `Err`→next |
| `LlmFallbackTriggered` set + bus event | `mission_a.rs:483-503` | sur `i > 0` |
| Bridge `call_one` / `stream_one` | `llm/bridge.rs:46-75` | **AUCUN retry** (1 tentative) |
| Bridge `call_many` (fan-out Mission B) | `bridge.rs:77-99` | erreurs collectées → vote dégradé |
| Speculator drafter `stream`/`call` | `speculate/speculator.rs:369,373` | erreur → bypass verifier |
| Speculator verifier `call` | `speculator.rs:290,494` | slow path |
| Drafter circuit (R3.S4) | `speculator.rs:223` | skip drafter si circuit open |
| `SlmCircuitBreaker` (R3.S2) | `efe-nano-app-coordinator` | analyzer/compressor/verifier |
| `CoordError::LlmProvider` (retryable=true) | `error.rs:27,116` | ce que les providers émettent |
| `is_retryable()` | `error.rs:110-127` | non consommé par le bridge (pas de retry) |
| **Pas de `tokio::time::timeout`** dans workflows | grep vide | deadline **advisory** (gap §11.2) |
| **Pas de dépendance `rand`** | `Cargo.toml` | PRNG interne requis (§4.2) |
| Live test gating (`EFE_LIVE_LLM`, `s7_preconditions_satisfied`) | `tests/live_missions.rs:55-58` | modèle pour les tests live |

**Deux observations décisives** :

1. **Le bridge ne retry pas** — chaque candidat reçoit *une* tentative ; tout
   `Err` passe au suivant. Donc `fault_rate` = probabilité par *appel
   logique*, et le fallback se déclenche dès la 1ʳᵉ faute du `selected`.
2. **La deadline est advisory** — `LlmCall.deadline` est passée mais aucun
   `tokio::time::timeout` ne l'enforce. Un provider lent peut faire traîner
   la mission. Le chaos `Timeout` doit être **borné** (§4.4).

---

## 4. Décisions chirurgicales

### 4.1 Deux vecteurs d'injection (provider vs infra)

Les fautes ne s'injectent pas au même niveau selon la cible :

| Cible | Trait | Vecteur chaos S6 |
|---|---|---|
| LLM cloud (Claude/Gemini/GPT) | `LlmProvider` | **`ChaosLlmProvider`** (décorateur) |
| Drafter Speculative (cloud OU local) | `LlmProvider` | **`ChaosLlmProvider`** |
| SLM nano-robots (analyzer/compressor/verifier) | `NanoRobotClient` (HTTP `:8090`…) | **redirection d'endpoint** (env `ANALYZER_URL` → port mort) |

**Conséquence** : le test `live_chaos_slm_analyzer_down` n'utilise PAS
`ChaosLlmProvider` (les SLM ne sont pas des `LlmProvider`). Il pointe
`ANALYZER_URL` vers un port injoignable → l'`AnalyzerClient.invoke` échoue →
le `SlmCircuitBreaker` (R3.S2) s'ouvre → analyzer skippé →
`FailureMode::SlmDown`. **Zéro nouveau code** pour ce vecteur : il valide
l'infra R3.S2 existante. C'est de l'honnêteté architecturale — on ne
sur-construit pas.

### 4.2 Déterminisme : PRNG interne seedé (zéro dépendance `rand`)

Le projet n'a **pas** `rand` (§3). Plutôt que de l'ajouter pour une poignée
de tirages, on embarque un **SplitMix64** (≈ 10 lignes, domaine public) :

```rust
struct ChaosRng { state: AtomicU64 }
impl ChaosRng {
    fn next_f32(&self) -> f32 {
        // SplitMix64 → u64 → normalisé [0,1)
        let mut z = self.state.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
                    .wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u32 << 24) as f32   // 24 bits → [0,1)
    }
}
```

**Pourquoi déterministe + seedé** : (1) reproductibilité des tests
(`fault_rate=0.5` + seed fixe ⇒ séquence de fautes identique à chaque run →
assertions stables) ; (2) cohérent avec le pattern du projet (circuit breaker
testé via `failure_count`, pas via timing) ; (3) les runs live peuvent seeder
depuis l'horloge (`from_env` + seed entropique) pour de la vraie variété.

**Sémantique `fault_rate`** : à chaque appel logique (`call`/`stream`), tirer
`r = rng.next_f32()` ; si `r < fault_rate` → injecter la faute, sinon
délégation pass-through. `fault_rate=0.0` ⇒ jamais (pass-through transparent,
invariant §1.3) ; `fault_rate=1.0` ⇒ toujours.

### 4.3 Injection dans `call()` ET `stream()`

Mission A passe par `provider.stream()` (`mission_a.rs:477` →
`bridge.stream_one` → `provider.stream`), Mission B par `provider.call()`
(`bridge.call_many` → `call_one`), le Speculator par les deux
(`speculator.rs:369/373/290`). **Décorer seulement `call()` laisserait Mission
A non couverte.** `ChaosLlmProvider` override donc les deux, avec la même
logique d'injection. `health()` est pass-through par défaut (le health check
n'est pas dans le chemin de mission).

### 4.4 Le fault `Timeout` doit être **borné** (gap hard-deadline révélé)

Comme les workflows n'enforcent pas la deadline (§3), un fault `Timeout(d)`
qui dort `d` puis échoue **ajoute `d` à la latence de la mission** sans être
interrompu. Si `d` était énorme, la CI hangerait.

**Décision** : `ChaosFaultKind::Timeout(d)` dort `min(d, CHAOS_MAX_SLEEP)`
(constante, ex. 5 s) **puis** retourne `CoordError::LlmProvider { reason:
"chaos: timeout after {d}ms" }`. Cela exerce le chemin « provider lent qui
finit par échouer » (réaliste) sans risque de hang. Le test live borne en
plus la mission via `tokio::time::timeout(harness_deadline, run_mission_a)`
côté **harness de test** (pas dans le workflow) pour fail-fast propre.

> **Finding documenté (§11.2)** : l'absence de hard-deadline dans les
> workflows est une faiblesse réelle. Le chaos `Timeout` la **rend visible**
> (latence dégradée mesurable). La corriger (`tokio::time::timeout` autour de
> `stream_one`) est hors scope S6 mais inscrite au backlog.

### 4.5 Fidélité d'erreur : le chaos émet ce que les vrais providers émettent

Les providers réels échouent en `CoordError::LlmProvider { provider, reason }`
(retryable). Le chaos **mirrore** ce type, avec un `reason` préfixé `chaos:`
porteur du kind (`"chaos: HTTP 429"`, `"chaos: invalid JSON response"`). Ainsi
les chemins de fallback/classification se comportent exactement comme en prod.
`InvalidResponse`/`EmptyResponse` sont une nuance : ils ne produisent PAS une
erreur mais un `LlmResponse` dégénéré (texte vide / non-parsable) — ce qui
teste si le **consommateur** (vote, Jaccard du Speculator, noise gate R3.S3)
détecte le contenu absent. Voir la table §6.

### 4.6 Feature `chaos`

`src/chaos.rs` est gardé par `#[cfg(feature = "chaos")]` (comme `local-slm`).
Off en prod (binaire propre, invariant §1.3) ; les tests/CI compilent avec
`--features chaos` (+ `all-providers` pour le live). `ChaosLlmProvider` étant
générique sur `Arc<dyn LlmProvider>`, il n'a **aucune** dépendance provider :
les tests offline tournent avec `--features chaos` seul (wrap de
`MockLlmProvider`).

---

## 5. Modèle de données (`src/chaos.rs`)

```rust
// SPDX-License-Identifier: UNLICENSED
//! R3.S6 — Chaos engineering : injection de fautes réelles (fix L3).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::error::{CoordError, CoordResult};
use crate::llm::{LlmCall, LlmProvider, LlmResponse, ProviderId, TokenChunk};

/// Plafond dur du sleep injecté (évite tout hang CI — cf. §4.4).
pub const CHAOS_MAX_SLEEP: Duration = Duration::from_secs(5);

/// Nature de la faute injectée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChaosFaultKind {
    /// Force un échec type HTTP (429/503…). Émet `CoordError::LlmProvider`.
    HttpStatus(u16),
    /// Dort `min(d, CHAOS_MAX_SLEEP)` puis échoue (provider lent réaliste).
    Timeout(Duration),
    /// Retourne un `LlmResponse` au texte non-parsable / tronqué (≠ erreur).
    InvalidResponse,
    /// Retourne un `LlmResponse` au texte vide (≠ erreur).
    EmptyResponse,
}

/// PRNG déterministe seedé (SplitMix64) — zéro dépendance externe (§4.2).
#[derive(Debug)]
struct ChaosRng { state: AtomicU64 }
impl ChaosRng {
    fn new(seed: u64) -> Self { Self { state: AtomicU64::new(seed) } }
    /// Tirage uniforme [0,1). Thread-safe (atomic).
    fn next_f32(&self) -> f32 { /* SplitMix64, cf. §4.2 */ }
}

/// Configuration : quels providers fautent, à quel taux, de quelle façon.
#[derive(Debug)]
pub struct ChaosInjector {
    /// provider_id → fault_rate ∈ [0,1]. Absent ⇒ 0 (jamais).
    fault_rates: HashMap<String, f32>,
    /// Nature de faute appliquée (uniforme pour tous les providers configurés).
    fault_kind: ChaosFaultKind,
    rng: ChaosRng,
}

impl ChaosInjector {
    /// Constructeur seedé (tests reproductibles).
    #[must_use]
    pub fn seeded(fault_kind: ChaosFaultKind, seed: u64) -> Self { /* … */ }

    /// Builder : ajoute un provider faillible.
    #[must_use]
    pub fn with_fault_rate(mut self, provider_id: impl Into<String>, rate: f32) -> Self { /* clamp [0,1] */ }

    /// Live : lit `EFE_CHAOS_RATE` (défaut 0.0) + seed entropique. Applique le
    /// taux à TOUS les providers passés en argument.
    #[must_use]
    pub fn from_env(fault_kind: ChaosFaultKind, provider_ids: &[String]) -> Self { /* … */ }

    /// Décide (et consomme un tirage) si CE provider doit fauter maintenant.
    #[must_use]
    pub fn should_fault(&self, provider_id: &str) -> bool {
        let rate = self.fault_rates.get(provider_id).copied().unwrap_or(0.0);
        rate > 0.0 && self.rng.next_f32() < rate
    }

    /// Enveloppe un provider réel dans un `ChaosLlmProvider`.
    #[must_use]
    pub fn wrap(self: Arc<Self>, inner: Arc<dyn LlmProvider>) -> Arc<ChaosLlmProvider> { /* … */ }
}

/// Décorateur `LlmProvider` : délègue à `inner`, faute selon `injector`.
pub struct ChaosLlmProvider {
    inner: Arc<dyn LlmProvider>,
    injector: Arc<ChaosInjector>,
}

#[async_trait]
impl LlmProvider for ChaosLlmProvider {
    fn id(&self) -> ProviderId { self.inner.id() } // §preserve id → router/fallback transparents

    async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
        if self.injector.should_fault(&self.inner.id()) {
            return self.apply_fault(/* via call path */).await;
        }
        self.inner.call(req).await
    }

    async fn stream(&self, req: LlmCall, tx: mpsc::Sender<TokenChunk>) -> CoordResult<LlmResponse> {
        if self.injector.should_fault(&self.inner.id()) {
            return self.apply_fault_stream(tx).await; // n'émet rien OU émet le dégénéré
        }
        self.inner.stream(req, tx).await
    }

    async fn health(&self) -> CoordResult<bool> { self.inner.health().await } // pass-through
}
```

**Notes de conception**

- `id()` **préserve** l'id du provider wrappé → le router (`mission_a.rs:218`)
  et la fallback chain (`:434`) sélectionnent le chaos-wrapper de façon
  transparente, sans connaître son existence.
- `fault_rate = 0.0` ⇒ `should_fault` court-circuite avant le tirage
  (`rate > 0.0`) ⇒ pass-through pur (invariant §1.3, et n'épuise pas le RNG).
- `ChaosInjector` en `Arc` partagé : un seul injecteur peut wrapper plusieurs
  providers et garder un état RNG cohérent (utile pour Mission B où on wrap
  potentiellement plusieurs LLMs).

---

## 6. Sémantique d'injection (table de décision)

| `ChaosFaultKind` | `call()` retourne | `stream()` fait | Chemin de dégradation exercé |
|---|---|---|---|
| `HttpStatus(429)` | `Err(LlmProvider{reason:"chaos: HTTP 429"})` | n'émet aucun chunk, `Err(...)` | Mission A fallback (`:483`) ; Mission B fail→`call_many` collecte→`VoteDegraded` ; Spec drafter→`DrafterBypassed` |
| `Timeout(d)` | sleep `min(d,5s)` puis `Err(...)` | sleep puis `Err(...)` | latence dégradée + même fallback ; révèle gap hard-deadline (§11.2) |
| `InvalidResponse` | `Ok(LlmResponse{text:"{ malformed", …})` | émet 1 chunk dégénéré puis `Ok` | teste parsing aval / Jaccard Spec / noise gate R3.S3 |
| `EmptyResponse` | `Ok(LlmResponse{text:"", tokens:0, …})` | n'émet rien, `Ok(empty)` | teste détection de contenu absent (noise gate R3.S3 ; vote sur texte vide) |

**Subtilité `InvalidResponse`/`EmptyResponse`** : ce sont des **faux succès**
(`Ok`), pas des erreurs → ils ne déclenchent PAS le fallback. Ils testent que
le *consommateur* gère le contenu dégénéré : en Speculative, le `NoiseRejectionPolicy`
(R3.S3) doit rejeter un draft vide → `spec_noise_rejected = Some(true)` → slow
path. C'est précisément le genre de défaut subtil que le mock ne capturait pas.

---

## 7. Intégration dans les workflows

**Aucune modification des workflows** n'est requise : le chaos s'injecte au
niveau du `ProviderRegistry`, en enregistrant le `ChaosLlmProvider` **sous
l'id du provider wrappé**.

```rust
// Setup d'un test/CI chaos (Mission A) :
let injector = Arc::new(
    ChaosInjector::seeded(ChaosFaultKind::HttpStatus(429), seed)
        .with_fault_rate("claude_claude-sonnet-4-6", 0.3),
);
let reg = ProviderRegistry::new();
reg.register(injector.clone().wrap(Arc::new(claude_real))); // id = claude_…
reg.register(Arc::new(gemini_real));                         // fallback sain
let coord = Coordinator::builder().registry(reg).build()?;
// run_mission_a избира "claude_…" → chaos faute à 30 % → fallback Gemini.
```

Réaction attendue par mission :

- **Mission A** : `selected = claude` fault → `attempt = Err` →
  `failed_components.push("llm:claude…")` → candidat suivant (gemini réel) Ok
  → `failure_mode = LlmFallbackTriggered{from:claude,to:gemini}` + bus event
  (`mission_a.rs:485-503`).
- **Mission B** : `call_many` sur [claude_chaos, gemini, gpt] → claude faute →
  collecté dans `failed_llms` → vote sur 2 survivants → `VoteDegraded{2,3}`.
- **Speculative** : drafter (chaos) faute → drafter bypass → verifier (sain)
  → `DrafterBypassed`. Si drafter `EmptyResponse` → noise gate R3.S3 → slow
  path.
- **SLM down** (infra-level, §4.1) : `ANALYZER_URL` → port mort → circuit
  s'ouvre après seuil → `SlmDown("analyzer")`, mission continue (texte brut).

---

## 8. Plan de tests (12 offline + 4 live)

### 8.1 Unitaires injector/provider (`src/chaos.rs`, 8 tests)

1. `chaos_rng_deterministic_for_seed` — même seed → même séquence `next_f32`.
2. `chaos_injector_zero_rate_never_faults` — `should_fault`==false ∀ tirages.
3. `chaos_injector_full_rate_always_faults` — `fault_rate=1.0` → toujours true.
4. `chaos_injector_basic_fault_rate` — sur 10 000 tirages à 0.3, fréquence
   ∈ [0.27, 0.33] (loi des grands nombres, seed fixe → borne stable).
5. `chaos_provider_preserves_provider_id` — `wrap(p).id() == p.id()`.
6. `chaos_provider_zero_rate_passthrough` — réponse identique au wrappé
   (texte, cost, tokens) — invariant §1.3.
7. `chaos_provider_emits_http_429_at_rate` — `HttpStatus(429)` + rate 1.0 →
   `Err(LlmProvider)` avec reason contenant "429", sur `call` ET `stream`.
8. `chaos_provider_emits_timeout_bounded` — `Timeout(1h)` → échoue en
   ≤ `CHAOS_MAX_SLEEP` + ε (borne §4.4), `Err`.

### 8.2 Intégration offline (mock + chaos via missions, 4 tests, `tests/chaos_offline.rs`)

9. `chaos_mission_a_429_triggers_fallback` — claude_chaos rate 1.0 (429),
   gemini sain → `final_text` == gemini, `failure_mode == LlmFallbackTriggered`,
   bus event émis.
10. `chaos_mission_b_one_llm_fault_degrades_vote` — 1/3 LLM chaos rate 1.0 →
    `vote_degraded == true`, `failed_llms.len()==1`, réponse produite.
11. `chaos_speculative_drafter_empty_response_forces_slow_path` — drafter
    `EmptyResponse` → noise gate R3.S3 → `verifier_skipped==false`,
    `spec_noise_rejected==Some(true)`, `final_text` correct.
12. `chaos_speculative_drafter_429_bypasses_to_verifier` — drafter
    `HttpStatus(503)` → `DrafterBypassed`, verifier produit la réponse.

### 8.3 Live (gate `EFE_LIVE_LLM=1` + `chaos` feature, `tests/live_chaos.rs`)

- `live_chaos_mission_a_429_at_30pct` — Claude réel wrap chaos
  `HttpStatus(429)` rate 0.3, Gemini réel fallback. **20 missions** ⇒
  asserte **> 80 %** complètent ; chaque échec Claude ⇒ `LlmFallbackTriggered`
  tracé. Harness borné par `tokio::time::timeout`.
- `live_chaos_mission_b_one_llm_timeout` — 1/3 LLM `Timeout` rate 1.0 → vote
  dégradé observé sur réseau réel.
- `live_chaos_speculative_drafter_invalid_response` — drafter Gemini réel
  `InvalidResponse` → bypass/slow path → qualité préservée (`contains("paris")`).
- `live_chaos_slm_analyzer_down` — `ANALYZER_URL=http://127.0.0.1:1` (port
  mort) + circuits R3.S2 → `SlmDown("analyzer")` + bus event, mission complète.

**Cible empirique** : agrégé sur les runs, ≥ 80 % de complétion sous 30 %
fault, `failure_mode` toujours cohérent avec la faute injectée.

---

## 9. CI workflow `chaos-eng.yml` (nightly)

```yaml
name: chaos-eng
on:
  schedule: [{ cron: "0 4 * * *" }]   # nightly 04:00 UTC
  workflow_dispatch: {}
jobs:
  chaos:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run chaos live suite
        run: cargo test -p efe-coordinator --features "chaos,all-providers" --test live_chaos -- --test-threads=1
        env:
          ANTHROPIC_API_KEY: ${{ secrets.ANTH_KEY }}
          GOOGLE_AI_KEY: ${{ secrets.GOOGLE_AI_KEY }}
          EFE_LIVE_LLM: "1"
          EFE_CHAOS_RATE: "0.3"
```

- `--test-threads=1` : sérialise (chaos partage l'état RNG + budget API).
- `workflow_dispatch` : déclenchable à la main pour reproduire un incident.
- Offline (`chaos_offline`, `chaos.rs` unit) tournent dans la CI normale
  (`cargo test --features chaos`) — pas besoin de secrets.

---

## 10. Observabilité

Le chaos **réutilise** la traçabilité S8-R2 (aucun nouvel event) :

- `StreamEvent::LlmFallbackTriggered { from, to, reason }` — `reason` contient
  `"chaos: HTTP 429"` (preuve que la faute injectée a bien déclenché le path).
- `ChewingMetrics.failure_mode` / `failed_components` — assertions de test.
- `MissionBResponse.{vote_degraded, failed_llms}`.
- `SpeculativeResponse` + `spec_noise_rejected` (R3.S3).

Optionnel (si R3.S7 Prometheus est livré avant) : `efe_failure_mode_total`
incrémente sous chaos → dashboard de résilience. Hors scope S6 (dépend de S7).

---

## 11. Résilience : ce que le chaos prouve, et ce qu'il révèle

### 11.1 Prouvé (objectif L3)

- La fallback chain Mission A se déclenche sur **vraies** 429/timeouts réseau,
  pas seulement sur l'erreur mockée.
- Le vote dégradé Mission B tolère un LLM réellement défaillant.
- Le drafter bypass + noise gate Speculative gèrent les réponses dégénérées.
- Le `SlmCircuitBreaker` (R3.S2) s'ouvre sur un SLM réellement injoignable.
- Sous 30 % de fautes réelles, > 80 % des missions servent l'utilisateur.

### 11.2 Révélé (findings honnêtes, backlog)

- **Pas de hard-deadline workflow** : le chaos `Timeout` montre que la latence
  d'une mission peut dépasser `max_total_latency` (deadline advisory, §3). Le
  fix (`tokio::time::timeout` autour de `stream_one`) est un sprint à part ;
  S6 le documente et borne le fault pour la CI.
- **Pas de retry/backoff** : le bridge échoue après 1 tentative. Sous chaos,
  cela signifie qu'un blip transitoire = fallback immédiat (pas de retry du
  provider primaire). C'est un choix conscient (le fallback est le filet),
  mais le chaos quantifie le taux de bascule — utile pour décider si un retry
  léger vaut le coup (backlog).

---

## 12. Backward-compat & sécurité

- **Feature off (prod)** : `src/chaos.rs` non compilé, surface publique
  inchangée, zéro symbole `Chaos*` exporté. Le binaire prod ne peut pas
  injecter de faute (sécurité : pas de chaos accidentel en prod).
- **`fault_rate = 0.0`** : pass-through byte-pour-byte (invariant §1.3, testé).
- **`EFE_CHAOS_RATE` absent** : `from_env` ⇒ 0.0 ⇒ no-op. Le chaos n'est jamais
  actif « par défaut », même feature activée — il faut un taux explicite.
- **Pas de fuite** : le chaos ne modifie jamais le contenu d'une vraie réponse
  réussie (il fault OU délègue ; `InvalidResponse`/`EmptyResponse` sont des
  réponses *fabriquées*, jamais une corruption de la vraie sortie provider).

---

## 13. Plan de modification fichier-par-fichier

| Fichier | Action | Détail |
|---|---|---|
| `src/chaos.rs` | **NEW** | `ChaosFaultKind`, `ChaosRng`, `ChaosInjector`, `ChaosLlmProvider`, `CHAOS_MAX_SLEEP` + 8 tests unit |
| `Cargo.toml` | patch | `[features] chaos = []` |
| `src/lib.rs` | patch | `#[cfg(feature="chaos")] pub mod chaos;` + re-exports `Chaos*` |
| `tests/chaos_offline.rs` | **NEW** | 4 tests intégration (mock + chaos) |
| `tests/live_chaos.rs` | **NEW** | 4 tests live (gate `EFE_LIVE_LLM`) |
| `.github/workflows/chaos-eng.yml` | **NEW** | CI nightly |
| `CHANGELOG.md` | patch | section R3.S6 |

**Note** : aucune modification de `mission_a/b.rs`, `speculator.rs`, `bridge.rs`
— l'injection est transparente via le registry (§7). C'est la force du design :
le chaos teste le code de prod **inchangé**.

---

## 14. Phasage d'implémentation (12 phases)

1. **Phase 1** — `Cargo.toml` feature `chaos`. `cargo check --features chaos`.
2. **Phase 2** — `src/chaos.rs` : `ChaosRng` (SplitMix64) + 1 test
   déterminisme. Valider la distribution.
3. **Phase 3** — `ChaosFaultKind` + `ChaosInjector` (`seeded`, `with_fault_rate`,
   `should_fault`) + 3 tests.
4. **Phase 4** — `ChaosLlmProvider` (`call`/`stream`/`id`/`health`) +
   `apply_fault` (4 kinds, bornage timeout) + `wrap`.
5. **Phase 5** — `from_env` (`EFE_CHAOS_RATE`) + 4 tests provider.
6. **Phase 6** — `lib.rs` re-exports gated. `cargo check --features chaos`.
7. **Phase 7** — `tests/chaos_offline.rs` : Mission A fallback.
8. **Phase 8** — `tests/chaos_offline.rs` : Mission B vote dégradé + Spec
   bypass + Spec empty→noise.
9. **Phase 9** — `tests/live_chaos.rs` : 4 tests live (compile + skip sans clés).
10. **Phase 10** — `.github/workflows/chaos-eng.yml`.
11. **Phase 11** — suite complète `--features chaos,all-providers` +
    `clippy --tests` exit 0 ; vérifier les invariants (pass-through, bornage).
12. **Phase 12** — CHANGELOG + commit (fichiers explicites).

---

## 15. Risques & mitigations

| Risque | Prob. | Impact | Mitigation |
|---|---|---|---|
| Hang CI sur `Timeout` non borné | moyen | élevé | `CHAOS_MAX_SLEEP` + harness `tokio::time::timeout` (§4.4) |
| Tests live flaky (proba) | moyen | moyen | PRNG **seedé** déterministe (§4.2) + bornes statistiques larges |
| Coût API live (20 missions × N) | faible | bas | gate `EFE_LIVE_LLM`, nightly only, requêtes courtes (~2-5 ¢) |
| Nouvelle dépendance `rand` | — | bas | **évitée** : SplitMix64 interne |
| Chaos compilé en prod | faible | élevé | feature `chaos` off par défaut (§12) |
| `InvalidResponse` passe en faux succès non détecté | moyen | moyen | test dédié #11 (noise gate R3.S3 doit l'attraper) |
| Fault sur `call` mais pas `stream` (Mission A non couverte) | — | élevé | injection dans **les deux** (§4.3) + test #7 |

---

## 16. Critères d'acceptation

- [ ] 12 tests offline PASS (8 unit + 4 intégration).
- [ ] 4 tests live PASS sous injection (compile + skip propre sans clés).
- [ ] `chaos-eng.yml` opérationnel (nightly + dispatch).
- [ ] **Validation empirique** : > 80 % des missions complètent sous 30 %
      fault rate (live).
- [ ] `failure_mode` toujours cohérent avec la faute injectée (tous tests).
- [ ] **Invariant pass-through** : `fault_rate=0.0` ⇒ réponse identique au
      wrappé (test #6).
- [ ] **Bornage** : aucun test ne dépasse `CHAOS_MAX_SLEEP` + ε sur `Timeout`.
- [ ] Feature off ⇒ zéro symbole chaos, binaire prod inchangé.
- [ ] `cargo test --features chaos,all-providers` exit 0 ; `clippy --tests` 0.
- [ ] **L3 résolu** : résilience prouvée en condition réelle.

---

## 17. Hors scope / suites futures

| Item | Sprint candidat | Raison du report |
|---|---|---|
| Hard-deadline workflow (`tokio::time::timeout`) | post-S6 | change le comportement des workflows ; révélé par S6 (§11.2) |
| Retry + backoff du provider primaire | post-S6 | orthogonal au fallback ; S6 quantifie d'abord le taux de bascule |
| Chaos bus/blackboard/DAG | sprint dédié | surface interne, pas la panne externe |
| Prometheus `efe_failure_mode_total` sous chaos | R3.S7 | dépend du module metering |
| Fault injection adversariale (sécurité) | sprint sécurité | résilience ≠ sécurité |
| Chaos multi-tenant (R3.S5 × S6) | post-S6 | matrice tenant × fault, après stabilisation S6 |

---

## 18. Annexe — pourquoi ce design est chirurgical

- **Zéro modif des workflows** : l'injection passe par le registry sous l'id
  wrappé → le code de prod testé est *littéralement inchangé* (le chaos ne
  teste pas une copie). C'est la propriété qui rend la preuve L3 valide.
- **Zéro nouvelle dépendance** : SplitMix64 interne (§4.2) — cohérent avec le
  refus de `rand` du projet.
- **Honnêteté sur les gaps** : le chaos `Timeout` *révèle* l'absence de
  hard-deadline (§11.2) au lieu de la masquer — on borne pour la CI et on
  inscrit le fix au backlog, sans le bâcler dans S6.
- **Deux vecteurs distincts** : provider-level (`ChaosLlmProvider`) vs
  infra-level (endpoint mort pour les SLM) — on ne force pas un SLM
  (`NanoRobotClient`) dans le moule `LlmProvider` ; on réutilise le
  `SlmCircuitBreaker` R3.S2 déjà éprouvé.
- **Fidélité d'erreur** : le chaos émet exactement `CoordError::LlmProvider`
  (ce que les vrais providers émettent) → les chemins de fallback se
  comportent comme en prod, pas comme avec une erreur synthétique d'un type
  différent.
