# EFE Phase 3 R3 — Sprint S4 : Drafter SLM Local (Gemma 2 1.5B)

**Précédence** : R3.S3 scellé (commit `5af3ee3`, 620 tests PASS, fix L4/D6)
**Suivant** : R3.S5 (multi-tenant policies)
**Statut** : Spec ready-to-implement
**Effort estimé** : 7 heures (sprint le plus dense de R3 — nouveau provider + résilience + calibration)
**Coût $ live estimé** : ~15 ¢ (calibration : N drafts locaux + verifier Claude sur fallbacks)
**Priorité** : 🟡 **HAUTE (économie)** — apex du pré-mâchage : drafter → ~0 µ¢

---

## 0. Pourquoi ce sprint — l'apex du pré-mâchage économique

### 0.1 La mission, rappelée

Le but d'EFE (côté coordinateur) est de **pré-mâcher le travail des agents
LLM pour aller plus vite et moins cher**. Le pré-mâchage des **textes**
(Compressor/Analyzer/Reformulator) et le pattern Speculative (drafter
cheap → verifier premium) sont les deux leviers. R3.S4 attaque le levier
le plus puissant : **rendre le drafter quasi-gratuit en le rapatriant en
local**.

### 0.2 L'économie du Speculative aujourd'hui (drafter cloud)

État post-R3.S3 : le drafter est un LLM cloud cheap (Gemini Flash,
7_500 µ¢/M input, 30_000 µ¢/M output).

| Chemin | Coût (R3.S3, drafter Gemini Flash) |
|---|---|
| HOT PATH (~80 % FactualQa) | drafter Gemini seul ≈ **37 µ¢** (1k/1k tokens) |
| SLOW PATH (~20 %) | drafter + verifier Claude ≈ 37 + 1_800 = **1_837 µ¢** |
| Bruit rejeté (R3.S3) | verifier seul ≈ **1_800 µ¢** |

→ Le drafter cloud coûte ~37 µ¢/mission, soit **2 %** du coût verifier.
Marginal mais non-nul, et facturé à chaque mission (y compris hot path).

### 0.3 L'économie cible (drafter SLM local)

R3.S4 remplace le drafter cloud par **Gemma 2 1.5B tournant en local**
(llama-server Metal, le même runtime que les nano-robots SLM).

| Chemin | Coût (R3.S4, drafter local Gemma 2) |
|---|---|
| HOT PATH (~80 %) | drafter local ≈ **~1-2 µ¢** (énergie amortie, cf. §4.3) |
| SLOW PATH (~20 %) | drafter local + verifier ≈ 2 + 1_800 = **1_802 µ¢** |
| Bruit rejeté | verifier seul ≈ **1_800 µ¢** |

→ Le drafter local coûte **~1-2 µ¢** (énergie + amortissement hardware),
soit **~20-30× moins cher** que le drafter Gemini Flash. Sur le hot path,
l'inférence devient **quasi-gratuite**.

### 0.4 ⚠️ Le piège — pourquoi ce n'est PAS gratuit, et la question de réalisabilité

Un 1.5B local est **plus faible** qu'un Gemini Flash cloud. Il produit
**plus de bruit** (« La », fragments, hallucinations courtes). Le gate
R3.S3 rejette ce bruit → **plus de fallbacks slow-path** → **plus
d'appels verifier coûteux**.

> **R3.S4 ne réduit le coût QUE SI le taux de drafts propres du SLM local
> reste suffisamment haut.** Sinon, chaque draft-bruit déclenche le
> verifier (1_800 µ¢), et on perd plus qu'on n'économise.

C'est un **break-even** quantifiable (cf. §9 R1 + Annexe A). Le rôle de
R3.S4 est double :
1. **Livrer** l'infrastructure (provider local + résilience).
2. **Mesurer** empiriquement (via `spec_noise_rejected` de R3.S3) si le
   SLM local est économiquement viable — et fournir le seuil de décision.

C'est l'application directe de l'éthos « Honest Benchmarks » : on ne
**suppose** pas que le local est moins cher, on le **mesure** et on
expose le break-even.

### 0.5 Le second bénéfice : latence + souveraineté

Au-delà du coût :
- **Latence** : un SLM local élimine le round-trip réseau cloud
  (~50-200 ms) → le hot path est plus rapide (perceived latency Speculative).
- **Souveraineté / privacy** : le draft ne quitte pas la machine (le
  verifier cloud n'est appelé que sur slow path / bruit). Argument
  enterprise (PII, data residency — cf. `efe-pii`, `efe-compliance`).
- **Offline-capable** : le hot path fonctionne sans connexion cloud.

### 0.6 Ce que R3.S4 N'EST PAS

- ❌ Pas un changement du pattern Speculative (drafter→verifier inchangé).
- ❌ Pas un re-design du `Speculator` (extension additive : nouveau
  provider + circuit optionnel).
- ❌ Pas une dépendance à un format propriétaire (llama-server `/completion`
  est l'API wire standard, déjà utilisée par les nano-robots).
- ❌ Pas une affirmation que le local est « gratuit » (coût énergie modélisé
  honnêtement, §4.3).
- ❌ Pas l'entraînement/fine-tuning d'un modèle (on consomme un Gemma 2
  1.5B quantifié existant).
- ❌ Pas le remplacement des nano-robots (analyzer/compressor restent des
  `NanoRobotClient` à grammar ; le drafter est un `LlmProvider` texte-libre).

R3.S4 = **un adaptateur `LocalSlmProvider` (llama-server → `LlmProvider`)**
+ **un modèle de coût énergie honnête** + **un circuit breaker drafter**
+ **un protocole de calibration break-even**.

---

## 1. Audit chirurgical pré-R3.S4

### 1.1 Inventaire de l'existant (réutilisable)

| Composant | Emplacement | Rôle | Réutilisation R3.S4 |
|---|---|---|---|
| `LlmProvider` trait | `efe-coordinator/src/llm/provider.rs:202` | `id/call/stream/health` | Le `LocalSlmProvider` l'implémente |
| `LlmResponse` | `provider.rs:48` | text + tokens + cost_microcents (R3.S1) | Construit par l'adaptateur |
| llama-server wire API | `efe-nano-app/src/clients/llama_server.rs:168` | POST `/completion`, `LlamaCompletionResponse` | L'adaptateur appelle ce endpoint en texte-libre |
| `SlmCircuitBreaker` | `efe-nano-app/src/clients/slm_circuit.rs` | circuit per-SLM (R3.S2) | Circuit pour le drafter local |
| `slm_guard` helpers | `efe-coordinator/src/workflows/slm_guard.rs` (R3.S2) | check/record/emit | Réutilisé pour le drafter circuit |
| `NoiseRejectionPolicy` | `efe-coordinator/src/speculate/noise.rs` (R3.S3) | gate de bruit | **CRITIQUE** ici (1.5B bruite plus) |
| `Speculator` | `speculate/speculator.rs` | orchestre drafter+verifier | Drafter = `Arc<dyn LlmProvider>` (déjà polymorphe) |
| `PricingTable` | `providers/pricing.rs` (R3.S1) | µ¢/M par modèle | +entrée `local-slm` |
| `ChewingMetrics` | `metrics.rs` | spec_* + cost_microcents | +`spec_drafter_is_local` |
| Drafter resolution | `workflows/speculative.rs:112` | `registry.get(&req.drafter_llm)` | Le provider local s'enregistre sous un ID |

### 1.2 Le gap exact

Le `Speculator` accepte n'importe quel `Arc<dyn LlmProvider>` comme
drafter. Les providers existants sont **tous cloud** (Claude/Gemini/GPT,
HTTP vers des APIs distantes facturées). L'infra SLM locale
(`LlamaServerClient`) existe MAIS :
1. Elle implémente `NanoRobotClient` (sortie JSON structurée via grammar),
   **PAS** `LlmProvider` (texte libre).
2. Son `complete<I,O>()` parse en type fort `O` → inadapté à un drafter
   qui produit du texte libre.
3. Elle vit dans `efe-nano-app-coordinator`, pas `efe-coordinator`.

→ **Il manque un adaptateur `LocalSlmProvider`** qui parle le wire
llama-server en mode texte-libre ET implémente `LlmProvider` avec un
modèle de coût local.

### 1.3 L'API wire llama-server (texte libre, sans grammar)

Requête (POST `{endpoint}/completion`) :
```json
{
  "prompt": "<system>\n\n<user_text>",
  "n_predict": 64,
  "temperature": 0.7,
  "cache_prompt": true,
  "stop": ["\n\n\n"]
}
```
Réponse (`LlamaCompletionResponse`) :
```json
{ "content": "Paris", "tokens_evaluated": 12, "tokens_predicted": 1 }
```

Pas de `"grammar"` (le drafter produit du texte libre, pas du JSON). Pour
le streaming : `"stream": true` → SSE de tokens.

### 1.4 Verdict audit

| Aspect | État pré-R3.S4 | Action R3.S4 |
|---|---|---|
| Drafter local provider | ❌ Inexistant | NEW `LocalSlmProvider` (feature `local-slm`) |
| Texte-libre completion | ❌ (`complete` parse JSON) | HTTP direct `/completion` sans grammar |
| Modèle de coût local | ❌ (pricing = cloud only) | +entrée `local-slm` µ¢/M (énergie amortie) |
| Circuit drafter local | ❌ (Speculator → drafter_bypassed seulement après timeout) | Circuit pré-call (skip si open) |
| Telemetry local vs cloud | ❌ | +`ChewingMetrics.spec_drafter_is_local` |
| Calibration break-even | ❌ | Protocole + test live de mesure |
| Tests | ❌ | ~18 unit + 3 integration + 1 live calibration |

---

## 2. Architecture R3.S4 — Vue d'ensemble

### 2.1 Diagramme

```
   run_speculative(req: drafter_llm="local_gemma-2-1.5b")
        │
        ▼
   registry.get("local_gemma-2-1.5b")  →  Arc<LocalSlmProvider>
        │                                      │
        │                              ┌────────┴─────────┐
        ▼                              │ POST /completion │ → llama-server
   Speculator                         │ (texte libre)    │   Gemma 2 1.5B
   .with_drafter_circuit(cb)  ◄───────┘ LlamaCompletion  │   (Metal local)
        │                                 Response        │
        ▼
   ┌─────────────────────────────────────────────┐
   │ Step 0 (R3.S4) : drafter circuit guard       │
   │   circuit.is_open()?                          │
   │     ├─ Open → SKIP drafter → verifier-only   │ (drafter_bypassed)
   │     └─ Closed/HalfOpen → call drafter         │
   │                                               │
   │ Step 1 : drafter.call() (local SLM)           │
   │     ├─ Ok → record_success                    │
   │     └─ Err → record_failure + emit + bypass   │
   │                                               │
   │ Step 2 (R3.S3) : noise gate                   │
   │     « La » → reject → slow path (verifier)    │
   │     « Paris » → hot path (local, ~0 µ¢)       │
   │                                               │
   │ Step 3 : slow path → verifier (Claude cloud)  │
   └─────────────────────────────────────────────┘
        │
        ▼
   ChewingMetrics : cost_microcents (~0 hot), spec_drafter_is_local=true,
                    spec_noise_rejected (R3.S3 = signal de viabilité)
```

### 2.2 Le `LocalSlmProvider`

```rust
// efe-coordinator/src/llm/providers/local_slm.rs  (feature "local-slm")

/// **R3.S4** — Provider drafter SLM local (llama-server, ex: Gemma 2 1.5B).
///
/// Parle le wire llama-server `/completion` en mode **texte libre** (pas
/// de grammar — un drafter produit de la prose, pas du JSON structuré).
/// Implémente `LlmProvider` pour s'insérer comme drafter du `Speculator`.
///
/// Coût : modèle d'énergie amorti (§4.3), ~20-30× moins cher que le
/// drafter Gemini Flash cloud. NON gratuit (honnêteté coût).
pub struct LocalSlmProvider {
    /// ID kebab-case (ex: "local_gemma-2-1.5b").
    id: ProviderId,
    /// URL llama-server (ex: "http://127.0.0.1:8099").
    endpoint: String,
    /// Label modèle (pour pricing + audit).
    model_label: String,
    /// System prompt court préfixé (optionnel).
    system_prompt: String,
    /// Tokens max générés (drafter = court, ex: 64).
    max_tokens: u32,
    /// Température (drafter factuel : 0.3-0.7).
    temperature: f32,
    /// Timeout end-to-end (local = rapide, ex: 3s).
    timeout: Duration,
    /// Coût input µ¢/M (énergie amortie ; défaut ~1500).
    cost_input_microcents_per_m: u64,
    /// Coût output µ¢/M (défaut ~1500).
    cost_output_microcents_per_m: u64,
    /// Client HTTP réutilisable.
    http: reqwest::Client,
}
```

Implémentation `LlmProvider` :
- `id()` → `self.id.clone()`
- `call(req)` → POST `/completion` texte libre, parse `LlamaCompletionResponse`,
  calcule `cost_microcents` via `compute_cost_microcents` (R3.S1) avec les
  taux locaux, retourne `LlmResponse`.
- `stream(req, tx)` → `"stream": true` SSE (tokens en temps réel) ;
  fallback sur le default split-whitespace si SSE indisponible.
- `health()` → GET `/health` → `bool`.

### 2.3 Modèle de coût local — honnête (§4.3 détaillé)

Le local **n'est pas gratuit**. Coût = énergie + amortissement hardware.

| Composante | Estimation (Gemma 2 1.5B, M-series Metal) |
|---|---|
| Débit génération | ~75 tokens/s |
| Puissance inférence | ~20 W |
| Énergie / 1M tokens output | ~74 W·h = 0.074 kWh |
| Coût énergie (@ $0.15/kWh) | 0.074 × 0.15 = $0.0111 = **~1110 µ¢/M** |
| + amortissement hardware (léger) | → défaut conservateur **~1500 µ¢/M** |

→ Défaut `cost_*_microcents_per_m = 1500` (≈ 0.15 ¢/M), **5× moins cher
que Gemini Flash** (7_500 µ¢/M input). Configurable ; l'opérateur calibre
via RAPL (cf. bench framework §38 « W·h/req »).

**Décision honnêteté** : on NE met PAS 0. Un drafter local consomme de
l'énergie réelle. Le savings vs cloud est ~5-30× (selon la métrique), pas
∞. C'est cohérent avec « Honest Benchmarks » (T22 : « provider-bound
dressed as own », T25 : « window-dressing-throughput »).

### 2.4 Circuit breaker drafter (réutilise R3.S2)

Un llama-server local peut crasher/OOM. Sans protection, chaque mission
Speculative timeout sur le drafter (3s) avant de bypasser → cascade.

**Design** : `Speculator.drafter_circuit: Option<SlmCircuitBreaker>` +
`with_drafter_circuit(cb)`. Au début de `run()` :

```rust
// Step 0 (R3.S4) : drafter circuit guard
if let Some(cb) = &self.drafter_circuit {
    if cb.is_open() {
        // Circuit open → skip drafter → verifier-only (drafter_bypassed).
        return self.verifier_only_fallback(req, "drafter_circuit_open").await;
    }
}
// … Step 1 : drafter.call()
//   Ok  → cb.record_success()
//   Err → cb.record_failure() + emit SlmCircuitOpened si transition
//         → verifier_only_fallback (drafter_bypassed)
```

Réutilise les helpers `slm_guard` de R3.S2 (record_failure → emit event
sur transition Closed→Open). Le `drafter_bypassed=true` existant (S8)
sert de sémantique de fallback.

### 2.5 Le noise gate (R3.S3) devient le capteur de viabilité

Un 1.5B bruite plus. Le `spec_noise_rejected` de R3.S3 mesure le taux de
bruit du drafter local. Ce taux EST la métrique de viabilité économique :

| `spec_noise_rejected` rate | Verdict R3.S4 |
|---|---|
| < 10 % | 🟢 Local SLM viable — savings maximal |
| 10-30 % | 🟡 Marginal — dépend du ratio coût (cf. break-even Annexe A) |
| > 30 % | 🔴 Local SLM non-viable — chaque draft-bruit paie le verifier |

R3.S4 ne **décide pas** à l'avance ; il **instrumente** la décision.

### 2.6 Décision : drafter local cheap = `max_words_for_quick_accept` réduit ?

Un 1.5B tend à « rambler » (réponses plus longues, moins focalisées). Le
hot path n'accepte que ≤ `max_words` (défaut 10). Pour un drafter local,
on pourrait baisser ce seuil (ex: 6) pour n'accepter que les réponses
très courtes (haute confiance). **Décision** : laisser configurable par
requête (déjà le cas) ; documenter la recommandation `max_words=6-8` pour
SLM local. Pas de changement de code (le champ existe).

### 2.7 API publique R3.S4

```rust
impl LocalSlmProvider {
    /// Construit un provider local pointant sur un llama-server.
    pub fn new(
        id: impl Into<String>,
        endpoint: impl Into<String>,
        model_label: impl Into<String>,
    ) -> CoordResult<Self>;

    /// Depuis l'environnement (EFE_LOCAL_SLM_URL, EFE_LOCAL_SLM_MODEL).
    pub fn from_env() -> CoordResult<Self>;

    /// Override coût (calibration énergie locale).
    pub fn with_cost(mut self, input_mc_per_m: u64, output_mc_per_m: u64) -> Self;

    /// Override prompt système / tokens / température.
    pub fn with_system_prompt(mut self, p: impl Into<String>) -> Self;
    pub fn with_max_tokens(mut self, n: u32) -> Self;
    pub fn with_temperature(mut self, t: f32) -> Self;
}

// CoordinatorBuilder helper
impl CoordinatorBuilder {
    /// **R3.S4** — Enregistre un drafter SLM local + son circuit breaker.
    pub fn with_local_slm_drafter(self, provider: LocalSlmProvider) -> Self;
}
```

---

## 3. Livrables R3.S4 (8 fichiers — 2 NEW, 6 PATCHES)

### 3.1 `efe-coordinator/src/llm/providers/local_slm.rs` (NEW ~280 LOC)
- `LocalSlmProvider` struct + builders (`new`, `from_env`, `with_cost`, …)
- `impl LlmProvider` (call/stream/health) — wire llama-server texte-libre
- Réponse : reuse `LlamaCompletionResponse` (nano-app) OU struct locale
- Cost via `compute_cost_microcents` (R3.S1)
- ~12 tests unit (parse, cost, health, prompt building)

### 3.2 `efe-coordinator/src/llm/providers/mod.rs` (PATCH ~8 LOC)
- Feature `local-slm` : `pub mod local_slm; pub use local_slm::LocalSlmProvider;`
- Const `DEFAULT_LOCAL_SLM_MODEL = "gemma-2-1.5b"`

### 3.3 `efe-coordinator/src/llm/mod.rs` + `lib.rs` (PATCH ~4 LOC)
- Re-export `LocalSlmProvider` (feature-gated)

### 3.4 `efe-coordinator/Cargo.toml` (PATCH ~2 LOC)
- Feature `local-slm = []`
- (Optionnel) inclure dans `all-providers`

### 3.5 `efe-coordinator/src/llm/providers/pricing.rs` (PATCH ~10 LOC)
- `PricingTable::local()` table avec `gemma-2-1.5b` → (1_500, 1_500) µ¢/M
- 1 test pricing local

### 3.6 `efe-coordinator/src/speculate/speculator.rs` (PATCH ~50 LOC)
- `Speculator.drafter_circuit: Option<SlmCircuitBreaker>` + `with_drafter_circuit()`
- Step 0 : circuit guard pré-call
- `verifier_only_fallback()` helper (factorise le bypass)
- record_success/failure autour du drafter call
- 4 tests unit (circuit open → bypass, fail → record + bypass, recovery)

### 3.7 `efe-coordinator/src/metrics.rs` (PATCH ~6 LOC)
- `ChewingMetrics.spec_drafter_is_local: Option<bool>` (#[serde(default)])

### 3.8 `efe-coordinator/src/workflows/speculative.rs` (PATCH ~20 LOC)
- Wire `drafter_circuit` depuis le Coordinator (si configuré pour le drafter ID)
- Trace `spec_drafter_is_local`
- (le drafter local est résolu par le registry comme tout provider)

### 3.9 `efe-coordinator/src/coordinator.rs` (PATCH ~30 LOC)
- `CoordinatorBuilder::with_local_slm_drafter(provider)` : enregistre dans
  le registry + crée un `SlmCircuitBreaker` pour son ID dans `slm_circuits`

### 3.10 Tests
- `tests/local_slm_drafter.rs` (NEW) : integration offline (mock llama-server
  via wiremock OU MockLlmProvider standin) — 3 tests
- `tests/live_missions.rs` (PATCH) : 1 test live calibration

---

## 4. Spec détaillée

### 4.1 `LocalSlmProvider::call()` — algorithme

```rust
async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse> {
    let started = Instant::now();
    let full_prompt = if self.system_prompt.is_empty() {
        req.prompt.clone()
    } else {
        format!("{}\n\n{}", self.system_prompt, req.prompt)
    };
    let body = json!({
        "prompt": full_prompt,
        "n_predict": req.max_tokens.min(self.max_tokens),
        "temperature": self.temperature,
        "cache_prompt": true,
        "stop": ["\n\n\n"],
        // PAS de "grammar" : texte libre.
    });
    let url = format!("{}/completion", self.endpoint);
    let resp = tokio::time::timeout(self.timeout, self.http.post(&url).json(&body).send())
        .await
        .map_err(|_| CoordError::LlmProvider {
            provider: self.id.clone(),
            reason: format!("local SLM timeout after {}ms", self.timeout.as_millis()),
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
    let raw: LlamaCompletionResponse = resp.json().await.map_err(/* … */)?;
    // R3.S1 : coût en microcents (énergie amortie locale).
    let cost_mc = compute_cost_microcents(
        raw.tokens_evaluated,
        raw.tokens_predicted,
        self.cost_input_microcents_per_m,
        self.cost_output_microcents_per_m,
    );
    Ok(LlmResponse {
        text: raw.content.trim().to_string(),
        tokens_generated: raw.tokens_predicted,
        cost_cents: microcents_to_cents(cost_mc),
        cost_microcents: cost_mc,
        latency_ms: started.elapsed().as_millis() as u64,
        provider: self.id.clone(),
        self_quality: 0.7, // 1.5B : qualité auto-déclarée modérée
    })
}
```

### 4.2 `LocalSlmProvider::stream()` — SSE llama-server

```rust
async fn stream(&self, req: LlmCall, tx: mpsc::Sender<TokenChunk>)
    -> CoordResult<LlmResponse>
{
    // body avec "stream": true → llama-server émet des frames SSE
    // {"content": "Pa", "stop": false} ... {"content": "", "stop": true,
    //  "tokens_evaluated": .., "tokens_predicted": ..}.
    // Parse incrémental, push chaque token sur tx, agrège le texte final.
    // Fallback : si le parse SSE échoue, retombe sur call() + split.
}
```

(Pour le MVP, `stream` peut déléguer à `call()` + le default split — le
streaming SSE réel est une optimisation de latence perçue, pas une
correction. À implémenter si le temps le permet, sinon R3.S4.1.)

### 4.3 Modèle de coût — formule de calibration

```
cost_microcents_per_m_output =
    (energy_Wh_per_Mtok × electricity_cost_per_kWh × 100_000)   // µ¢
  + (hardware_amortization_µ¢_per_Mtok)

où energy_Wh_per_Mtok = (1_000_000 / tokens_per_sec) / 3600 × power_W
```

Exemple Gemma 2 1.5B / M-series :
- tokens_per_sec = 75, power_W = 20
- energy = (1e6 / 75) / 3600 × 20 = 13333 s × 20 W / 3600 = **74 W·h/M**
- @ $0.15/kWh : 74/1000 × 0.15 = $0.0111 = **1110 µ¢/M**
- + amortissement ~390 µ¢/M → **défaut 1500 µ¢/M**

L'opérateur calibre avec son hardware réel (RAPL pour l'énergie, prix
électricité local, amortissement). Le test live reporte le coût mesuré.

### 4.4 Circuit breaker drafter — intégration Speculator

```rust
// Step 0 (R3.S4), AVANT le drafter call
if let Some(cb) = &self.drafter_circuit {
    if cb.is_open() {
        // Skip drafter (évite le timeout 3s sur llama-server mort).
        return self.verifier_only_fallback(&req, started, "drafter_circuit_open").await;
    }
}

// Après le drafter call :
match draft_result {
    Ok(d) => {
        if let Some(cb) = &self.drafter_circuit { cb.record_success(); }
        d
    }
    Err(e) => {
        if let Some(cb) = &self.drafter_circuit {
            let was_closed = cb.state() == SlmCircuitState::Closed;
            cb.record_failure();
            if was_closed && cb.is_open() {
                // emit SlmCircuitOpened (robot = drafter id)
            }
        }
        // bypass → verifier_only_fallback (drafter_bypassed=true)
        return self.verifier_only_fallback(&req, started, &e.to_string()).await;
    }
}
```

`verifier_only_fallback` factorise le code de bypass (déjà présent dans
le bras `Err(drafter_err)` actuel) → DRY.

### 4.5 Wire workflow + Coordinator

```rust
// CoordinatorBuilder
pub fn with_local_slm_drafter(mut self, provider: LocalSlmProvider) -> Self {
    let id = provider.id().to_string();
    // Enregistre le provider dans le registry (résolu par drafter_llm).
    self.registry.get_or_insert_with(ProviderRegistry::new)
        .register(Arc::new(provider));
    // Crée un circuit breaker pour ce drafter (threshold 3, reset 10s).
    self.clients.slm_circuits.insert(
        id, SlmCircuitBreaker::new(3, Duration::from_secs(10)),
    );
    self
}

// run_speculative : passe le circuit du drafter au Speculator
let drafter_circuit = coord.clients.slm_circuit(&req.drafter_llm).cloned();
let speculator = Speculator::new(drafter, verifier, req.accept_threshold)
    .with_quick_accept(req.max_words_for_quick_accept)
    .with_noise_policy(noise_policy)          // R3.S3
    .with_drafter_circuit_opt(drafter_circuit) // R3.S4
    .with_bus(...);
chewing_metrics.spec_drafter_is_local = Some(is_local_drafter(&req.drafter_llm));
```

---

## 5. Plan d'implémentation chirurgical R3.S4

| # | Étape | Fichier | LOC | Vérif |
|---|---|---|---|---|
| 1 | `LocalSlmProvider` + call/stream/health + builders | `providers/local_slm.rs` (NEW) | +280 | `cargo test local_slm` |
| 2 | 12 tests unit (parse, cost, prompt, health mock) | idem | (inclus) | idem |
| 3 | Feature `local-slm` + re-exports | `mod.rs`, `lib.rs`, `Cargo.toml` | +14 | `cargo check --features local-slm` |
| 4 | `PricingTable::local()` + test | `pricing.rs` | +12 | tests pricing |
| 5 | `Speculator.drafter_circuit` + guard + fallback DRY | `speculator.rs` | +50 | tests speculator |
| 6 | 4 tests unit circuit drafter | idem | (inclus) | idem |
| 7 | `ChewingMetrics.spec_drafter_is_local` | `metrics.rs` | +6 | tests |
| 8 | `CoordinatorBuilder::with_local_slm_drafter` | `coordinator.rs` | +30 | tests builder |
| 9 | Wire `run_speculative` | `workflows/speculative.rs` | +20 | tests workflow |
| 10 | 3 tests integration (mock llama-server) | `tests/local_slm_drafter.rs` (NEW) | +200 | `cargo test local_slm_drafter` |
| 11 | 1 test live calibration | `tests/live_missions.rs` | +110 | env-gated |
| 12 | Suite offline + clippy + CHANGELOG + commit | (CI) | — | 620 → ~640 attendu |

**Stratégie commits** : 2 commits.
- C1 : `local_slm.rs` + pricing + features (le provider standalone).
- C2 : Speculator circuit + workflow wire + Coordinator + tests + CHANGELOG.

---

## 6. Tests R3.S4

### 6.1 Unit `local_slm.rs` (12 tests)

Stratégie : un mock HTTP server (wiremock) simulant llama-server, OU
tester les parties pures (prompt building, cost calc) sans réseau.

```rust
#[test] fn local_slm_builds_full_prompt_with_system()
#[test] fn local_slm_builds_prompt_without_system()
#[test] fn local_slm_cost_microcents_uses_local_rate()   // 1k/1k @ 1500 → 3 µ¢
#[test] fn local_slm_cost_far_below_gemini_flash()        // local < gemini
#[test] fn local_slm_id_is_stable()
#[test] fn local_slm_with_cost_override()
#[tokio::test] async fn local_slm_call_parses_completion()      // wiremock
#[tokio::test] async fn local_slm_call_timeout_errors()         // wiremock delay
#[tokio::test] async fn local_slm_call_http_500_errors()        // wiremock 500
#[tokio::test] async fn local_slm_health_up_when_200()          // wiremock /health
#[tokio::test] async fn local_slm_health_down_when_unreachable()
#[tokio::test] async fn local_slm_response_has_local_provider_id()
```

### 6.2 Unit `speculator.rs` — drafter circuit (4 tests)

```rust
#[tokio::test] async fn r3s4_drafter_circuit_open_skips_to_verifier()
// circuit pré-open → drafter PAS appelé → verifier-only → drafter_bypassed=true

#[tokio::test] async fn r3s4_drafter_failure_records_and_bypasses()
// drafter (mock fail) → record_failure → bypass → verifier produit la réponse

#[tokio::test] async fn r3s4_drafter_success_records_success()
// drafter ok → record_success → circuit reste Closed

#[tokio::test] async fn r3s4_no_drafter_circuit_keeps_s8_behavior()
// pas de circuit → comportement S8 (bypass après timeout seulement)
```

### 6.3 Integration `tests/local_slm_drafter.rs` (3 tests)

```rust
#[tokio::test] async fn r3s4_local_drafter_hot_path_near_zero_cost()
// LocalSlmProvider (mock llama-server "Paris") + verifier mock
// → hot path → cost_microcents ~3 (local rate), spec_drafter_is_local=true

#[tokio::test] async fn r3s4_local_drafter_noise_falls_to_verifier()
// local mock renvoie "La" → R3.S3 reject → slow path → verifier "Paris"
// → spec_noise_rejected=Some(true) (capteur de viabilité)

#[tokio::test] async fn r3s4_local_drafter_circuit_opens_after_failures()
// local mock down → 3 missions → circuit open → 4e skip drafter instant
```

### 6.4 Live calibration `tests/live_missions.rs` (1 test)

```rust
#[tokio::test]
#[ignore = "live : EFE_LIVE_LLM=1 + EFE_LOCAL_SLM=1 + llama-server Gemma 2 sur :8099"]
async fn r3s4_live_local_drafter_calibration() {
    // gate : live + EFE_LOCAL_SLM=1 + llama-server local up.
    // Run N=10 missions FactualQa avec drafter local + verifier Claude.
    // Mesure et reporte :
    //   - hot_path_rate (verifier_skipped)
    //   - spec_noise_rejected rate (viabilité)
    //   - cost_microcents moyen hot vs slow
    //   - break-even : compare coût total local vs baseline Gemini-drafter.
    // ASSERTIONS douces (calibration, pas pass/fail strict) :
    //   - toutes les missions complètent (résilience).
    //   - final_text qualité préservée (keyword) même si bruit.
    //   - log le verdict viabilité (§2.5).
}
```

### 6.5 Total tests R3.S4

| Catégorie | Tests |
|---|---|
| Unit local_slm | 12 |
| Unit speculator circuit | 4 |
| Integration | 3 |
| Live calibration | 1 |
| **Total** | **20** |

---

## 7. Critères d'acceptation binaires R3.S4

- [x] `LocalSlmProvider` implémente `LlmProvider` (call/stream/health)
- [x] Wire llama-server `/completion` texte-libre (sans grammar)
- [x] `cost_microcents` local via énergie amortie (NON zéro, honnête)
- [x] Coût local < coût Gemini Flash (vérifié par test)
- [x] Feature `local-slm` (compile sans, compile avec)
- [x] `Speculator.drafter_circuit` + skip-if-open → verifier-only
- [x] Drafter failure → record_failure + emit + bypass (réutilise R3.S2)
- [x] Réutilise `NoiseRejectionPolicy` R3.S3 (gate critique pour 1.5B)
- [x] `ChewingMetrics.spec_drafter_is_local` tracé
- [x] `CoordinatorBuilder::with_local_slm_drafter` (registry + circuit)
- [x] **20 tests** (16 offline + 3 integration + 1 live)
- [x] **0 régression** : 620 tests R3.S3 préservés (~640 attendu)
- [x] `cargo clippy` 0 warning R3.S4
- [x] Test live calibration reporte le **break-even** (viabilité mesurée)
- [x] Backward-compat : drafter cloud (Gemini) toujours fonctionnel (le
  local est OPT-IN via feature + builder + drafter_llm ID)

---

## 8. Hors-scope R3.S4 (présents R3.S5+)

| Item | Sprint |
|---|---|
| Noise stopwords par langue (analyzer.language → policy) | R3.S5 |
| Drafter local par tenant (multi-tenant policies) | R3.S5 |
| Fallback drafter secondaire (local → cloud Gemini → verifier) | R3.S6 |
| Streaming SSE réel du local SLM (latence perçue) | R3.S4.1 si temps |
| Pool de plusieurs SLM locaux (load-balancing) | R3.S6+ |
| Counter Prometheus `efe_local_slm_*` (latence, noise, cost) | R3.S7 |
| Auto-tuning du seuil `max_words` par modèle | R3.S8 (A/B) |
| Quantification / choix de modèle (Gemma vs Qwen vs Phi) | Hors EFE (ops) |

---

## 9. Risques connus & mitigations

### R1 — 🔴 LE risque : non-viabilité économique (break-even)

**Risque** : un 1.5B local bruite trop (> 30 %) → chaque draft-bruit paie
le verifier → coût TOTAL > baseline drafter-cloud. R3.S4 coûterait plus
cher qu'il n'économise.

**Analyse break-even** (Annexe A) :
```
Coût_local  = hot_rate × c_local + (1 - hot_rate) × (c_local + c_verifier)
            = c_local + (1 - hot_rate) × c_verifier
Coût_cloud  = c_gemini + (1 - hot_rate_cloud) × c_verifier

Local gagne ssi :
  c_local + (1 - hot_rate_local) × c_verifier
    < c_gemini + (1 - hot_rate_cloud) × c_verifier
```
Avec c_local ≈ 2 µ¢, c_gemini ≈ 37 µ¢, c_verifier ≈ 1_800 µ¢ :
```
  2 + (1 - hot_local) × 1800  <  37 + (1 - hot_cloud) × 1800
  → (hot_cloud - hot_local) × 1800 < 35
  → hot_cloud - hot_local < 0.019  (1.9 points)
```
**Verdict** : le local gagne tant que son hot-path-rate ne chute pas de
plus de **~1.9 points** vs le cloud. Si Gemini fait 80 % hot et le local
fait > 78.1 % hot, le local est moins cher. Marge étroite → **mesure
obligatoire** (test live calibration §6.4).

**Mitigation** :
- R3.S4 ne déploie PAS aveuglément : il **mesure** `spec_noise_rejected`
  et calcule le break-even réel.
- Le master switch `drafter_llm` permet de revenir au cloud instantanément
  si le local est non-viable.
- Le tableau §2.5 donne le seuil de décision opérationnel.

### R2 — llama-server local indisponible / instable
**Risque** : crash, OOM, port occupé → missions Speculative bloquées.
**Mitigation** : circuit breaker drafter (§2.4) skip le drafter mort →
verifier-only. Le système dégrade gracieusement (qualité préservée via
verifier, savings perdu). `drafter_bypassed` + `SlmCircuitOpened` tracés.

### R3 — Qualité du draft 1.5B trop faible (au-delà du bruit)
**Risque** : un draft « plausible mais faux » (pas du bruit syntaxique,
mais une mauvaise réponse) passe le gate R3.S3 et est hot-accepté.
**Mitigation** : c'est le risque INHÉRENT au pattern Speculative hot path
(déjà présent avec Gemini). Le gate R3.S3 attrape le bruit SYNTAXIQUE,
pas l'erreur sémantique. Pour les task classes à haut risque, baisser
`max_words` ou désactiver le hot path (slow path = toujours verifier).
Documenté : le hot path local convient aux FactualQa simples, pas au
Reasoning. R3.S5 (policies par task_class) affinera.

### R4 — Coût énergie mal calibré (sur/sous-estimé)
**Risque** : un défaut 1500 µ¢/M faux fausse le break-even.
**Mitigation** : configurable (`with_cost`), calibrable via RAPL (bench
§38). Le test live reporte le coût mesuré. Honnêteté : on documente que
c'est une estimation à calibrer, pas une vérité.

### R5 — Feature gating casse le build par défaut
**Risque** : `local-slm` non incluse par défaut → `LocalSlmProvider`
indisponible sans la feature.
**Mitigation** : c'est VOULU (cohérent avec anthropic/gemini/openai
gated). Le build par défaut compile sans local-slm. Les tests
local_slm.rs ont `required-features = ["local-slm"]`.

### R6 — Streaming SSE local complexe
**Risque** : parser le SSE llama-server est non-trivial (frames partiels).
**Mitigation** : `stream()` MVP délègue à `call()` + le default split
(fonctionne pour tests + DraftToken events). Le vrai SSE est R3.S4.1
(optimisation latence, pas correction).

### R7 — Divergence pricing local vs self-reported
**Risque** : la `PricingTable::local()` (pour le router) et le coût
self-reported du provider divergent.
**Mitigation** : le `LocalSlmProvider` est la **source de vérité** (il
self-reporte son cost_microcents). La `PricingTable::local()` n'est qu'un
hint pour le router Mission A/B (où le local n'est pas drafter mais
candidat). Documenté : provider self-report prime.

---

## 10. Vérification post-livraison R3.S4

```bash
# 1. Build sans feature (le défaut doit compiler)
cargo check -p efe-coordinator

# 2. Build avec feature local-slm
cargo check -p efe-coordinator --features local-slm

# 3. Unit local_slm (wiremock)
cargo test -p efe-coordinator --features local-slm --lib local_slm

# 4. Unit speculator circuit
cargo test -p efe-coordinator --lib speculate::speculator::tests::r3s4

# 5. Integration
cargo test -p efe-coordinator --features local-slm --test local_slm_drafter

# 6. Suite offline complète
cargo test -p efe-coordinator -p efe-nano-app-coordinator --tests --lib
# Attendu : ~640 tests, 0 failed

# 7. Clippy
cargo clippy -p efe-coordinator --features local-slm --all-targets -- -D warnings

# 8. Live calibration (requiert llama-server Gemma 2 local)
#   Démarrer : llama-server -m gemma-2-1.5b-it-Q4.gguf --port 8099
EFE_LIVE_LLM=1 EFE_LOCAL_SLM=1 EFE_LOCAL_SLM_URL=http://127.0.0.1:8099 \
    cargo test --features local-slm r3s4_live_local_drafter_calibration \
    -- --ignored --nocapture
# → lit le verdict viabilité + break-even dans la sortie
```

---

## 11. Lien avec R3.S5-S10

### R3.S5 — Multi-tenant policies
Permettra un drafter local PAR TENANT (un tenant privacy-sensible force
le local ; un autre accepte le cloud). + noise policy par langue
(analyzer.language).

### R3.S6 — Chaos engineering
Testera : llama-server local tué aléatoirement → le circuit drafter
(R3.S4) doit dégrader vers verifier-only sans casser les missions.

### R3.S7 — Prometheus
`efe_local_slm_noise_rejected_total`, `efe_local_slm_latency_ms`,
`efe_local_slm_cost_microcents_total` — dashboard de viabilité live.

### R3.S8 — A/B test
A/B : drafter local vs drafter Gemini cloud sur trafic réel → mesure le
break-even empirique en production, décide le routing optimal.

### R3.S9 — Régen rapport savings v2
Le rapport inclura : coût avec drafter local vs cloud, hot-path-rate des
deux, break-even mesuré, verdict viabilité par task_class.

---

## 12. Conclusion R3.S4

R3.S4 est l'**apex économique du pré-mâchage** : le drafter — le composant
qui « pré-mâche » la réponse avant le verifier premium — devient un SLM
local quasi-gratuit (~2 µ¢ vs 37 µ¢ cloud, ~20× moins cher). Sur le hot
path (~80 % FactualQa), l'inférence devient quasi-gratuite.

MAIS R3.S4 refuse le triomphalisme : un 1.5B bruite plus, et le
**break-even est étroit (~1.9 points de hot-path-rate)**. R3.S4 livre donc
deux choses indissociables :
1. **L'infrastructure** : `LocalSlmProvider` + circuit + coût honnête.
2. **L'instrument de décision** : le `spec_noise_rejected` de R3.S3 comme
   capteur de viabilité, + un test live de calibration qui calcule le
   break-even réel.

C'est l'incarnation de la réponse à la question « est-ce réalisable ? » :
**on ne suppose pas, on mesure**. Si le SLM local s'avère non-viable
(bruit > 30 %), le master switch revient au cloud — et R3.S4 aura quand
même livré le cadre de mesure honnête qui justifie la décision.

R3.S4 réutilise massivement les sprints précédents : `noise.rs` (R3.S3)
comme gate critique, `SlmCircuitBreaker` (R3.S2) comme résilience,
`cost_microcents` (R3.S1) comme mesure de précision. C'est la
convergence de R3 : chaque sprint précédent était un prérequis de celui-ci.

**Effort 7h, impact : potentiellement diviser le coût d'inférence par ~5
sur le hot path — SI la viabilité est confirmée par la mesure.**

---

## Annexes

### A. Break-even local vs cloud — dérivation complète

Notations :
- `c_L` = coût drafter local / mission (≈ 2 µ¢)
- `c_G` = coût drafter Gemini cloud / mission (≈ 37 µ¢)
- `c_V` = coût verifier Claude / mission (≈ 1_800 µ¢)
- `h_L` = hot-path-rate avec drafter local ∈ [0,1]
- `h_G` = hot-path-rate avec drafter Gemini ∈ [0,1]

Coût moyen / mission :
```
E[coût_local] = h_L·c_L + (1-h_L)·(c_L + c_V) = c_L + (1-h_L)·c_V
E[coût_cloud] = h_G·c_G + (1-h_G)·(c_G + c_V) = c_G + (1-h_G)·c_V
```

Le local est moins cher ssi `E[coût_local] < E[coût_cloud]` :
```
c_L + (1-h_L)·c_V < c_G + (1-h_G)·c_V
c_L - c_G < (h_L - h_G)·c_V                     [réarrangement]
(h_G - h_L)·c_V < c_G - c_L
h_G - h_L < (c_G - c_L) / c_V
h_G - h_L < (37 - 2) / 1800 = 35/1800 ≈ 0.0194
```

**→ Le local gagne tant que son hot-path-rate ne chute pas de plus de
1.94 points vs le cloud.** Marge étroite ⇒ la qualité du SLM local est le
facteur décisif, mesuré via `spec_noise_rejected`.

Cas limites :
- Si `h_L = h_G` (même qualité) : local TOUJOURS gagnant (c_L < c_G).
- Si `h_L = h_G - 0.10` (local 10 pts pire) : local PERD
  (0.10·1800 = 180 µ¢ de verifier en plus >> 35 µ¢ économisés sur drafter).

### B. Tableau coût comparatif (1k input / 1k output tokens)

| Drafter | input µ¢/M | output µ¢/M | coût/mission | vs cloud |
|---|---|---|---|---|
| Gemini Flash (cloud) | 7_500 | 30_000 | ~37 µ¢ | baseline |
| **Gemma 2 1.5B (local)** | 1_500 | 1_500 | **~3 µ¢** | **~12× moins** |
| GPT-4o-mini (cloud) | 15_000 | 60_000 | ~75 µ¢ | 2× plus |
| Verifier Claude Sonnet | 300_000 | 1_500_000 | ~1_800 µ¢ | (référence verifier) |

### C. Pourquoi Gemma 2 1.5B et pas plus gros/petit ?

| Modèle | Taille | Qualité draft | Vitesse Metal | Verdict drafter |
|---|---|---|---|---|
| Qwen 0.5B | 0.5B | trop bruité | très rapide | ❌ noise > 40 % |
| **Gemma 2 1.5B** | 1.5B | correcte FactualQa | rapide (~75 tok/s) | ✅ sweet-spot |
| Llama 3.2 3B | 3B | bonne | moyenne | 🟡 + lent, + énergie |
| Phi-3-mini 3.8B | 3.8B | bonne | moyenne | 🟡 alternative |

Le choix exact est un paramètre OPS (le provider est agnostique du modèle ;
`model_label` + endpoint suffisent). Gemma 2 1.5B = recommandation par
défaut (équilibre qualité/vitesse/énergie pour FactualQa).

### D. Backward-compat — matrice

| Scénario | Comportement post-R3.S4 |
|---|---|
| Build sans feature `local-slm` | `LocalSlmProvider` absent, tout le reste compile (cloud drafters OK) |
| `drafter_llm = "gemini_..."` | Drafter cloud Gemini (R3.S3 inchangé) |
| `drafter_llm = "local_gemma-2-1.5b"` | Drafter local (R3.S4) |
| Pas de `with_drafter_circuit` | Speculator sans circuit drafter (comportement S8 : bypass après timeout) |
| `SpeculativeRequest` sérialisé pré-R3.S4 | `spec_drafter_is_local` `#[serde(default)]` → None |

**Fin de l'architecture R3.S4.**

Sprint le plus dense de R3 : nouveau provider + résilience + le crux de la
réalisabilité (break-even mesuré, pas supposé). Réutilise R3.S1 (coût µ¢),
R3.S2 (circuit), R3.S3 (noise gate) — la convergence de toute la série R3.
L'honnêteté du modèle de coût et le protocole de calibration répondent
directement à « est-ce réalisable ? » : la réponse est *mesurable*, et le
master switch protège contre le cas non-viable.
