# EFE Phase 3 R3 — Production Hardening Layer (Architecture Globale)

**Précédence** : R2 scellé le 2026-05-28 (commit `12f31bd`)
**Suivant** : Phase 4 (Frontend SSE + preview lab)
**Statut** : Spec ready-to-implement
**Durée estimée** : ~6-8 semaines (10 sprints R3.S1-S10)
**Coût $ live cumulé estimé** : ~$5-10 (chaos engineering + multi-tenant + savings v2)

---

## 0. Pourquoi R3 — adresser honnêtement les gaps R2

### 0.1 Le verdict honnête R2

> *"Phase 3 R2 = Foundation Layer solide mais non-production-ready intégral."*
> Cf. rapport honnête post-R2 (sections "Limites" de chaque sprint S1-S10).

R2 a livré **les patterns architecturaux** (pré-mâchage, routing tuné,
Speculative RAG, graceful degradation, savings report). Mais 4 limitations
critiques empêchent un déploiement production :

| # | Limitation R2 | Impact production |
|---|---|---|
| **L1** | `cost_cents: u32` → savings ×50 réel **invisible** | Rapport investisseurs montre `0 % savings` — embarrassant |
| **L2** | `SlmCircuitBreaker` livré mais **non wiré** dans workflows | F1 "SLM down" pas géré en prod |
| **L3** | S8 failure modes **simulés** seulement (MockLlmProvider) | Pas de garantie de résilience en condition réelle |
| **L4** | Speculator drafter peut produire du **bruit** ("La" accepté hot path) | Quality fail silencieux sur ~5-10 % queries |

### 0.2 Les 8 découvertes chirurgicales R2 — résolutions R3

| # | Découverte | Sprint origine | Sprint fix R3 |
|---|---|---|---|
| D1 | `target_words=100` produit résumés ≈ raw | S5 fix immédiat | — (résolu) |
| D2 | Code routing demande `cost_weight=0.2` | S6 calibration | R3.S8 (A/B test pour ré-évaluer) |
| D3 | Speculator pré-S7 appelait toujours verifier | S7 fix immédiat | — (résolu) |
| D4 | `SpeculativeResponse` manquait `cost_cents` | S7 audit post-impl fix | — (résolu) |
| D5 | `vote_with_fallback` annoncé n'existait pas | S8 archi inexacte | — (résolu, archi rectifiée) |
| D6 | `stream_many` FuturesUnordered désordonne | S8 fix immédiat | — (résolu) |
| **D7** | **`u32 cents` masque savings ×50** | **S9 limitation** | **R3.S1 — Cost microcents** |
| **D8** | **Fixtures trop strictes sur keywords** | **S9.1 patch partiel** | **R3.S9 — Régen savings report v2** |

### 0.3 Ce que R3 N'EST PAS

- ❌ Pas de **nouveau pattern architectural** (R2 a tout posé)
- ❌ Pas de **breaking change** sur l'API publique (`MissionA/B/SpecRequest/Response` shapes préservées)
- ❌ Pas de nouveaux LLM providers (Anthropic + Google + OpenAI restent les 3 wired)
- ❌ Pas de support multi-modal (texte uniquement, images en R4+)
- ❌ Pas de frontend (Phase 4 dédiée)

R3 = **Consolidation + précision + chaos + observabilité** sur les patterns
R2 existants. C'est un sprint de **maturation**, pas d'innovation.

### 0.4 Promesse R3 mesurable

À la fin de R3, le rapport `CHEWING_SAVINGS_REPORT_v2.md` (régénéré en
R3.S9) **doit montrer** :

```text
Baseline    cost = ~1500 microcents (Claude direct, 5 fixtures)
Full S4+S6  cost =   ~80 microcents (×18.7 moins cher)
Speculative cost =   ~20 microcents (×75 moins cher sur FactualQa)
                  ─────────────────
Savings cost      = -94.7 % via Full
Savings cost      = -98.7 % via Speculative
```

→ Le savings cost ×16-100 **prouvé empirically** au lieu de **projeté**.

---

## 1. Audit post-R2 — état exact (chiffres scellés)

### 1.1 Métriques R2 finales (snapshot 2026-05-28)

| Métrique | Valeur scellée |
|---|---|
| Sprints livrés | 10 (S1 → S10) |
| Tests offline cumulés | 631 (33 binaires, 0 régression) |
| Tests live PASS | ~169 assertions sur 6 sprints |
| Coût $ live cumulé R2 | ~40 ¢ |
| Architecture docs | 8 fichiers (~7900 LOC) |
| Découvertes chirurgicales | 8 (D1-D8) |
| Limitations production | 4 (L1-L4) |
| **Speedup latence Speculative** | **×5.24** (mesuré) |
| **Speedup latence Full** | **×1.59** (mesuré) |
| **Quality preservation Full** | **+20 pp** (mesuré) |
| **Savings cost (u32 ¢)** | **0 %** (résolution insuffisante) |
| **Savings cost projeté** | **×16-100** (projection 80/20) |

### 1.2 Code base R2 — surface à étendre en R3

| Module | LOC R2 | Extension R3 attendue |
|---|---|---|
| `src/llm/provider.rs` (`LlmResponse`) | ~150 | +`cost_microcents: u64` (R3.S1) |
| `src/metrics.rs` (`ChewingMetrics`) | ~320 | +`*_microcents` fields (R3.S1) |
| `src/failure.rs` | 247 | +`SlmCircuit*` variants si wired (R3.S2) |
| `src/savings_report.rs` | ~520 | +mode comparison microcents (R3.S9) |
| `src/workflows/mission_a.rs` | ~620 | +SlmCircuitBreaker guards (R3.S2) |
| `src/speculate/speculator.rs` | ~440 | +noise rejection (R3.S3) |
| **NEW R3** | — | `src/multi_tenant.rs` (R3.S5), `src/metering.rs` (R3.S7), `src/chaos.rs` (R3.S6), `src/ab_test.rs` (R3.S8) |

---

## 2. Vue d'ensemble R3 — 10 sprints structurés

### 2.1 Plan global

| Sprint | Livrable | Priorité | Effort | Coût $ live |
|---|---|---|---|---|
| **R3.S1** | Cost microcents (`u64`) — fix L1/D7 | 🔴 critique | 4h | 0 ¢ |
| **R3.S2** | `SlmCircuitBreaker` wiring workflows — fix L2 | 🔴 critique | 6h | ~10 ¢ |
| **R3.S3** | Speculator noise rejection — fix L4/D6 | 🟡 important | 3h | ~5 ¢ |
| **R3.S4** | Drafter SLM local (`SlmDrafterProvider`) | 🟡 important | 8h | ~3 ¢ (SLM only) |
| **R3.S5** | Multi-tenant policies | 🟢 nice-to-have | 10h | ~10 ¢ |
| **R3.S6** | Chaos engineering CI — fix L3 | 🔴 critique | 12h | ~$1 ($) |
| **R3.S7** | Prometheus telemetry | 🟢 nice-to-have | 6h | 0 ¢ |
| **R3.S8** | A/B test tables de scoring | 🟡 important | 8h | ~$2 ($) |
| **R3.S9** | Régen `CHEWING_SAVINGS_REPORT_v2.md` (microcents + SLM + noise rejection) | 🔴 critique | 4h | ~$2 ($) |
| **R3.S10** | Documentation + commit + tag final R3 | 🟢 packaging | 4h | 0 ¢ |
| **TOTAL** | — | — | **65h** | **~$5-10** |

### 2.2 Ordre topologique (dépendances)

```
                    ┌─────────────────────────┐
                    │ R3.S1 Cost microcents   │ (foundation pour tout reporting)
                    └────────────┬────────────┘
                                 │
                ┌────────────────┼────────────────┐
                ▼                ▼                ▼
        ┌────────────┐  ┌────────────────┐  ┌────────────┐
        │R3.S2 SLM CB│  │R3.S3 Speculator│  │R3.S7 Prom  │
        │  wiring    │  │ noise reject   │  │ telemetry  │
        └─────┬──────┘  └────────┬───────┘  └────────────┘
              │                  │
              │                  │
              ▼                  ▼
        ┌────────────┐  ┌────────────────┐
        │R3.S4 SLM   │  │R3.S6 Chaos eng │ (utilise SLM CB wiring + noise reject)
        │  drafter   │  │      CI        │
        └─────┬──────┘  └────────┬───────┘
              │                  │
              └─────────┬────────┘
                        │
        ┌───────────────┼───────────────┐
        ▼               ▼               ▼
   ┌──────────┐  ┌──────────────┐  ┌────────────┐
   │R3.S5     │  │R3.S8 A/B test│  │R3.S9 régen │
   │ Multi-   │  │              │  │ savings v2 │
   │ tenant   │  │              │  │            │
   └──────────┘  └──────────────┘  └─────┬──────┘
                                          │
                                          ▼
                                   ┌──────────────┐
                                   │R3.S10 docs + │
                                   │   commit     │
                                   └──────────────┘
```

**Dépendances dures** :
- R3.S2 dépend de R3.S1 (microcents pour reporting du circuit overhead)
- R3.S4 dépend de R3.S2 (réutilise SlmCircuit pattern)
- R3.S6 dépend de R3.S2 + R3.S3 (chaos teste les patterns wired)
- R3.S9 dépend de R3.S1 + R3.S2 + R3.S3 + R3.S6 (tous activés pour rapport vrai)
- R3.S10 dépend de tout

---

## 3. Détail par sprint R3

### R3.S1 — Cost microcents (fix L1/D7) 🔴

**Problème** : `cost_cents: u32` arrondit le savings ×50 (Claude 0.12¢ vs
Gemini 0.0024¢) à `0¢` partout sur fixtures courtes. Rapport S9 montre
`savings_ratio = 0 %`.

**Solution** : passer en `cost_microcents: u64` (×1000 précision).

#### Livrables

- **`LlmResponse.cost_microcents: u64`** (NEW field, default 0)
- **`LlmResponse.cost_cents()` helper** : `(cost_microcents + 500) / 1000` (rounded)
  pour backward-compat
- Tous les providers (Claude, Gemini, OpenAI) calculent en microcents :
  - Claude : `input_tokens × (cost_input_per_million_microcents / 1_000_000)`
  - Gemini : idem
  - OpenAI : idem
- **`MissionAResponse.cost_microcents: u64`** + `cost_cents()` helper
- **`MissionBResponse.total_cost_microcents: u64`**
- **`SpeculativeResponse.cost_microcents: u64`**
- **`ChewingMetrics`** : `spec_savings_microcents`, `router_estimated_cost_microcents`
- **`LlmMetadata`** : `cost_input_per_million_microcents`, `cost_output_per_million_microcents`
- **`TaskClassScoringTable`** : recalibration `LlmStaticMetadata` en microcents
- **Backward-compat** :
  - Anciens JSON sans `cost_microcents` → `#[serde(default)]` → `0`
  - Helper `cost_cents() -> u32` partout pour consumers legacy
  - `SavingsReport.runs[].cost_microcents: u64` (NEW) + `.cost_cents: u32` préservé

#### Tests (12 nouveaux)

- `lib unit tests` : conversions `cost_microcents ↔ cost_cents` roundtrip stable
- `provider tests` : Claude/Gemini/OpenAI mocks retournent microcents corrects
- `metrics tests` : `cost_microcents` aggregation S4 wire
- `savings_report tests` : `SavingsReport` agrégate microcents + génère table dédiée

#### Critère d'acceptation

- [x] `LlmResponse.cost_microcents > 0` sur run live (Claude fixture 01)
- [x] Différence Claude vs Gemini visible en microcents (×50)
- [x] Backward-compat : tous les tests R2 préservés (631+12 = 643 attendu)
- [x] `cargo clippy` 0 warning

---

### R3.S2 — `SlmCircuitBreaker` wiring workflows (fix L2) 🔴

**Problème** : `SlmCircuitBreaker` livré en S8 mais **PAS wiré** dans
`run_mission_a/b/speculative`. Donc en cas de SLM crash répété, le
workflow continue d'appeler le SLM en boucle (pas de circuit ouvert).

**Solution** : injecter `SlmCircuitBreaker` per-robot dans `Coordinator`,
guards autour des `*Client::invoke()` dans workflows.

#### Livrables

- **`CoordinatorClients.slm_circuits: HashMap<String, SlmCircuitBreaker>`**
  (3 entries : analyzer, compressor, verifier)
- **`Coordinator::builder().with_slm_circuits()`** helper
- **`run_mission_a`** : guard avant chaque `*Client::invoke()` :
  ```rust
  if let Some(circuit) = coord.clients.slm_circuits.get("analyzer") {
      if circuit.is_open() {
          // skip analyzer, emit StreamEvent::SlmCircuitOpened
          chewing_metrics.failure_mode = FailureMode::SlmDown("analyzer".into());
          chewing_metrics.failed_components.push("analyzer".into());
      } else {
          match analyzer.invoke(...).await {
              Ok(r) => { circuit.record_success(); /* ... */ }
              Err(e) => { circuit.record_failure(); /* skip + trace */ }
          }
      }
  }
  ```
- Idem pour `run_mission_b` (3 SLM × N fixtures = 3N guards)
- Idem pour `run_speculative` (verifier SLM uniquement, drafter c'est un LLM cloud)
- **Bus event `SlmCircuitOpened`** émis quand le circuit s'ouvre

#### Tests (8 nouveaux)

- `mission_a_skips_analyzer_after_3_failures_with_real_circuit` (1 test offline)
- `mission_a_skips_compressor_after_threshold` (idem)
- `mission_a_emits_slm_circuit_opened_event` (validation bus)
- `mission_b_continues_when_analyzer_circuit_open` (vote mode dégradé partial S8)
- `slm_circuit_recovers_after_reset_timeout` (test temporel)
- `failure_mode_slm_down_set_correctly` (chewing_metrics)
- `multiple_slm_circuits_open_composite_failure_mode` (validation FailureMode::Composite réel)
- **Test live** : `live_mission_a_resilient_when_analyzer_unreachable`
  - Setup : SLM analyzer pas démarré (port 8090 down)
  - Mission A doit fonctionner sans analyzer (skip silencieux)
  - Bus event `SLM_CIRCUIT_OPENED` émis après 3 missions

#### Critère d'acceptation

- [x] 8 tests offline PASS
- [x] 1 test live PASS (analyzer down)
- [x] R2 tests préservés (631+8 = 639 attendu offline)

---

### R3.S3 — Speculator noise rejection (fix L4/D6) 🟡

**Problème** : drafter Gemini peut produire `"La"` (1 mot) sur query longue.
Le hot path actuel accepte tout draft ≤ `max_words_for_quick_accept`
(10 par défaut). 1 mot ≤ 10 → accepté aveuglément.

Observé S9 fixture 03 : `Run 10/22 : speculative... final_text="La"` → fail quality.

**Solution** : ajouter un seuil minimum + validation sémantique légère.

#### Livrables

- **`Speculator.min_words_for_quick_accept: usize`** (default 2)
- **`Speculator::with_min_words(n)`** builder
- **`Speculator::run`** logique étendue :
  ```rust
  let draft_words = draft.text.split_whitespace().count();
  let hot_path = self.max_words_for_quick_accept > 0
      && draft_words >= self.min_words_for_quick_accept  // ← NEW
      && draft_words <= self.max_words_for_quick_accept;
  ```
- **`SpeculativeRequest.min_words_for_quick_accept: usize`** (default 2,
  `#[serde(default = "default_min_words_for_quick_accept")]`)
- Wire dans `run_speculative` workflow

#### Tests (5 nouveaux)

- `speculator_rejects_draft_of_1_word_when_min_2`
- `speculator_accepts_draft_of_2_words_at_threshold`
- `speculator_min_words_default_is_2`
- `speculative_request_default_min_words_serde`
- **Test live** : `live_mission_speculative_rejects_la_noise`
  - Setup : reproduire fixture 03 + drafter Gemini (peut répondre "La")
  - Avec `min_words=2` : le draft "La" est REJECTED → slow path → Claude appelé → "Paris" correct
  - Quality pass rate doit passer de 50% (R2) à 100% (R3)

#### Critère d'acceptation

- [x] 5 tests offline PASS
- [x] 1 test live PASS (noise rejected)
- [x] Régression S9 (Speculative quality) : 50 % → ≥ 80 % attendu

---

### R3.S4 — Drafter SLM local (alternative à Gemini cloud) 🟡

**Problème** : Speculator S7 utilise Gemini Flash comme drafter — dépendance
cloud + coût ~$0.075/M. Pour cas où la **confidentialité importe** (santé,
légal, défense), tout doit rester local.

**Solution** : `SlmDrafterProvider` qui implémente `LlmProvider` en parlant
à un SLM llama-server local (Qwen3-0.6B sur port 8091 par exemple).

#### Livrables

- **`SlmDrafterProvider`** nouvelle struct + impl `LlmProvider` :
  ```rust
  pub struct SlmDrafterProvider {
      endpoint: String,         // e.g. "http://127.0.0.1:8091"
      model_name: String,       // e.g. "Qwen3-0.6B"
      timeout: Duration,
      breaker: SlmCircuitBreaker,  // réutilise R3.S2
  }

  impl LlmProvider for SlmDrafterProvider {
      fn id(&self) -> ProviderId { format!("slm_drafter_{}", self.model_name) }
      async fn call(&self, req: LlmCall) -> CoordResult<LlmResponse>;
      async fn stream(&self, req: LlmCall, tx: Sender<TokenChunk>) -> CoordResult<LlmResponse>;
  }
  ```
- **Pricing** : `cost_microcents = 0` (local, GPU coûts négligeables)
- **`SpeculativeRequest::with_slm_drafter_and_cloud_verifier(...)`** helper
- **Scripts** : `./scripts/slm/start.sh` étendu pour démarrer reformulator port 8091 comme drafter Qwen3-0.6B
- **Fixture S9 V2** : `08_speculative_local_only.json` (confidentialité scenario)

#### Tests (6 nouveaux)

- `slm_drafter_provider_id_format` (unit)
- `slm_drafter_call_returns_response` (mock llama-server)
- `slm_drafter_stream_emits_tokens` (mock)
- `slm_drafter_handles_endpoint_unreachable` (graceful degradation S8 réutilisé)
- `speculative_with_slm_drafter_uses_cost_microcents_0` (intégration)
- **Test live** : `live_speculative_local_drafter_qwen3_no_cloud`
  - Setup : Qwen3-0.6B SLM démarré + Claude verifier
  - Mission Speculative drafter = SLM local
  - cost_microcents drafter = 0
  - savings massif vs cloud-cloud setup

#### Critère d'acceptation

- [x] 6 tests offline PASS
- [x] 1 test live PASS (SLM drafter)
- [x] `cost_microcents` SLM = 0 démontré
- [x] Confidentialité : aucune donnée envoyée au cloud sur hot path

---

### R3.S5 — Multi-tenant policies 🟢

**Problème** : la `TaskClassScoringTable` est singleton global. En SaaS
multi-tenant, chaque client doit pouvoir avoir sa propre table calibrée
(client A privilégie qualité, client B privilégie coût).

**Solution** : `TenantContext` + table per-tenant.

#### Livrables

- **`TenantId: Uuid`** (déjà existant dans `CallContext.tenant_id`)
- **`TenantPolicies` struct** :
  ```rust
  pub struct TenantPolicies {
      pub tenant_id: TenantId,
      pub scoring_table: TaskClassScoringTable,
      pub retry_policy: RetryPolicy,
      pub slm_circuit_threshold: u32,
      pub max_cost_microcents_per_query: u64,
      pub allowed_llms: Vec<String>,            // whitelist
  }
  ```
- **`PolicyRegistry`** : `HashMap<TenantId, TenantPolicies>`, accessed via
  `Coordinator::with_policies(...)`
- **`MissionA/B/SpecRequest.tenant_id: Option<TenantId>`** (optionnel,
  backward-compat default = `Uuid::nil()`)
- **`run_mission_a`** : lookup `coord.policies.get(req.tenant_id)` →
  override table de scoring + retry + budget
- **Audit chain** : `AuditEntry.tenant_id` (existant) propagé partout

#### Tests (10 nouveaux)

- `tenant_policies_default_falls_back_to_global` (backward-compat)
- `tenant_policies_lookup_by_id`
- `tenant_a_uses_quality_weighted_table` vs `tenant_b_uses_cost_weighted_table`
- `tenant_max_cost_microcents_caps_mission`
- `tenant_allowed_llms_filters_available_llms`
- 4 tests live (skip si pas de scenario multi-tenant en CI)

#### Critère d'acceptation

- [x] 10 tests offline PASS
- [x] 2 tenants distincts produisent des routings différents sur même query
- [x] Audit chain trace `tenant_id` par mission

---

### R3.S6 — Chaos engineering CI (fix L3) 🔴

**Problème** : S8 failure modes simulés via `MockLlmProvider::with_error()`,
**jamais validés en condition réelle**. Pas de garantie que la fallback chain
marche quand Anthropic part réellement en 429.

**Solution** : framework de chaos testing qui injecte de vraies fautes
(timeouts, 429 simulés via proxy, latence artificielle) en condition live.

#### Livrables

- **Nouveau module `src/chaos.rs`** :
  ```rust
  pub struct ChaosInjector {
      // Map provider_id → fault_rate ∈ [0,1]
      fault_rates: HashMap<String, f32>,
      fault_kind: ChaosFaultKind,
  }

  pub enum ChaosFaultKind {
      HttpStatus(u16),      // forcer HTTP 429/503
      Timeout(Duration),    // bloquer N ms puis fail
      InvalidResponse,      // retourner JSON mal formé
      EmptyResponse,        // retourner texte vide
  }
  ```
- **Décoration `LlmProvider`** : `ChaosLlmProvider` wraps un provider réel
  et injecte la faute aléatoirement selon `fault_rate`
- **Test live** `live_chaos_mission_a_resilience` :
  - Setup : ChaosLlmProvider(Claude, fault_rate=0.5, fault=HttpStatus(429))
  - 20 missions consécutives
  - Asserte : > 80 % réussissent via fallback chain Gemini
  - Asserte : `failure_mode` correctement tracé
- **CI workflow** : `chaos-eng.yml` GitHub Action nightly :
  ```yaml
  - run: cargo test --test live_chaos_* -- --test-threads=1
    env:
      ANTHROPIC_API_KEY: ${{ secrets.ANTH_KEY }}
      EFE_LIVE_LLM: 1
      EFE_CHAOS_RATE: 0.3
  ```

#### Tests (12 nouveaux + 1 CI workflow)

- `chaos_injector_basic_fault_rate` (unit)
- `chaos_provider_emits_http_429_at_rate`
- `chaos_provider_preserves_provider_id`
- `chaos_provider_emits_timeout`
- 4 tests offline d'intégration via `MissionA/B/Spec` + ChaosInjector
- **4 tests live** :
  - `live_chaos_mission_a_429_at_30pct` — 30 % faults Claude → fallback Gemini
  - `live_chaos_mission_b_one_llm_timeout` — 1/3 LLM timeout → vote dégradé
  - `live_chaos_speculative_drafter_invalid_response` — drafter Gemini invalid → bypass verifier
  - `live_chaos_slm_analyzer_down` — analyzer 8090 unreachable → skip + bus event

#### Critère d'acceptation

- [x] 12 tests offline PASS
- [x] 4 tests live PASS sous chaos injection
- [x] CI workflow `chaos-eng.yml` opérationnel
- [x] **Validation empirique** : > 80 % missions complètent sous 30 % fault rate
- [x] **L3 résolu** : résilience prouvée en condition réelle (plus simulée)

---

### R3.S7 — Prometheus telemetry 🟢

**Problème** : R2 émet des `StreamEvent` SSE mais aucune métrique
Prometheus-compatible pour monitoring production.

**Solution** : exposer `/metrics` HTTP endpoint avec les compteurs R2.

#### Livrables

- **`src/metering.rs`** module + `MetricsRegistry`
- Métriques exposées :
  ```
  efe_missions_total{type="A"|"B"|"Speculative", outcome="success"|"error"}
  efe_cost_microcents_total{provider="claude_…"|"gemini_…"|"gpt_…"}
  efe_latency_ms_histogram{phase="analyzer"|"compressor"|"verifier"|"llm"}
  efe_failure_mode_total{kind="LLM_FALLBACK_TRIGGERED"|"SLM_DOWN"|...}
  efe_slm_circuit_state{robot="analyzer", state="closed"|"open"|"half_open"}
  efe_savings_microcents_total{mode="full"|"speculative"}
  efe_quality_pass_rate_gauge{mode="full"|"baseline"}
  ```
- **HTTP server** `metrics_server.rs` (Tokio + Hyper, port `:9090`)
- **Hook dans `Coordinator`** : chaque mission incrémente les compteurs
- **CHANGELOG** : `[Unreleased]` → `cost_microcents` exposed via Prometheus

#### Tests (8 nouveaux)

- `metrics_registry_increments_counter`
- `metrics_registry_observes_histogram`
- `metrics_registry_renders_prometheus_format`
- `coordinator_records_mission_metrics_after_run` (intégration)
- 4 tests HTTP endpoint

#### Critère d'acceptation

- [x] `curl http://localhost:9090/metrics` retourne format Prometheus valide
- [x] 8 métriques R2 visibles (table ci-dessus)
- [x] Aucun overhead > 1 ms par mission

---

### R3.S8 — A/B test entre tables de scoring 🟡

**Problème** : Comment savoir si une nouvelle table de scoring est meilleure
qu'une ancienne ? La calibration S6 est figée nov-2025 ; en mars 2026 si
GPT-5 sort, on veut tester la nouvelle table sans risquer la prod.

**Solution** : framework A/B test qui route X % du trafic vers `table_v2`
et compare empirically.

#### Livrables

- **`ABTestConfig`** struct :
  ```rust
  pub struct ABTestConfig {
      pub control_table: TaskClassScoringTable,    // v1
      pub experiment_table: TaskClassScoringTable, // v2
      pub experiment_traffic_ratio: f32,           // ∈ [0,1]
      pub tenant_filter: Option<TenantId>,
      pub fixture_filter: Option<Vec<String>>,
  }
  ```
- **Hash-based bucketing** : `hash(swarm_id) % 100 < experiment_ratio * 100`
  → assigne au bucket control ou experiment
- **`SavingsReport` v2** : capture `ab_test_arm: ControlOrExperiment` par run
- **`live_ab_test_savings_report`** : run 30 missions, 50 % chaque bucket,
  comparaison stat des deux

#### Tests (7 nouveaux)

- `ab_test_config_serializes_stable`
- `ab_test_assigns_buckets_deterministically`
- `ab_test_savings_report_segregates_control_vs_experiment`
- `ab_test_traffic_ratio_zero_skips_experiment`
- `ab_test_with_tenant_filter` (intégration multi-tenant R3.S5)
- 2 tests live (30 missions A/B sur 5 fixtures × 2 tables)

#### Critère d'acceptation

- [x] 7 tests offline PASS
- [x] 2 tests live PASS produisent rapport segmenté
- [x] Hash bucketing reproductible (même `swarm_id` → même bucket)

---

### R3.S9 — Régen `CHEWING_SAVINGS_REPORT_v2.md` (le sprint le plus visible) 🔴

**Problème** : `CHEWING_SAVINGS_REPORT.md` R2 montre `savings_ratio = 0 %`
sur cost — embarrassant. Régénérer après R3.S1 (microcents) + R3.S2 (SLM
wired) + R3.S3 (noise rejection) → vraie image.

**Solution** : test live identique à S9 mais avec SLM daemons ON, microcents
activé, Speculator noise rejection.

#### Livrables

- **Test live `live_chewing_savings_report_v2`** :
  - Setup : 3 LLMs cloud + 3 SLMs locaux (analyzer/compressor/verifier)
  - 5 fixtures S9 patchées (R2) + 1 fixture NEW R3.S4 (`08_speculative_local_only`)
  - 5 modes : Baseline / S4Chewing (avec SLM vrai) / S6Routing / Full (S4+S6 wired) / Speculative (avec noise rejection R3.S3)
  - Total : 5 fixtures × 5 modes + 2 fixtures × Speculative + 1 fixture × LocalSpec = ~26-30 runs
- **`SavingsReport` v2** : `to_markdown_v2()` utilise `cost_microcents`
  partout, affiche table cost en `μ¢`
- **`CHEWING_SAVINGS_REPORT_v2.md`** généré, **`CHEWING_SAVINGS_REPORT.md`**
  préservé comme baseline historique
- **Assertions H1-H15** (5 nouvelles) :
  - H1-H10 : identiques S9
  - **H11** : `savings_microcents_ratio > 0.50` (vrai savings via microcents)
  - **H12** : SLM circuit jamais ouvert (validation R3.S2 wiring effective)
  - **H13** : Speculative quality ≥ 80 % (validation R3.S3 noise rejection)
  - **H14** : Local SLM drafter cost_microcents = 0 (R3.S4)
  - **H15** : Aucun `final_text` < 2 mots accepté en hot path

#### Critère d'acceptation

- [x] Test live PASS H1-H15 (15/15)
- [x] **`savings_microcents_ratio > 50 %` mesuré** (vs 0 % R2)
- [x] `CHEWING_SAVINGS_REPORT_v2.md` produit (~200 lignes)
- [x] **L1 résolu** : argument économique défensible quantitativement

---

### R3.S10 — Documentation + commit + tag final R3 🟢

**Problème** : R3 livré sans packaging cohérent → R3 incomplet.

**Solution** : `R3_LIVE_VALIDATION.md` méta-rapport + CHANGELOG `[0.2.0]`
+ tag git `phase-3-r3-complete`.

#### Livrables

- **`crates/efe-coordinator/R3_LIVE_VALIDATION.md`** méta-rapport agrégé
  (cohérent avec `R2_LIVE_VALIDATION.md`)
- **`crates/efe-coordinator/CHANGELOG.md`** entrée `[0.2.0]` Phase 3 R3 :
  ```
  ## [0.2.0] — 2026-XX-XX — Phase 3 R3 BOUCLÉE (production hardening)

  ### Added
  - `cost_microcents: u64` exposed via LlmResponse, MissionA/B/SpecResponse,
    ChewingMetrics, SavingsReport (R3.S1 fix D7)
  - SlmCircuitBreaker wired dans run_mission_a/b/speculative (R3.S2 fix L2)
  - Speculator min_words_for_quick_accept=2 (R3.S3 fix L4/D6)
  - SlmDrafterProvider (R3.S4 — local Qwen3-0.6B drafter)
  - Multi-tenant policies (R3.S5 — TenantPolicies + PolicyRegistry)
  - Chaos engineering live tests (R3.S6 fix L3)
  - Prometheus /metrics endpoint (R3.S7)
  - A/B test framework entre tables de scoring (R3.S8)

  ### Changed
  - CHEWING_SAVINGS_REPORT_v2.md produit avec microcents + SLM wired
    (savings cost mesuré > 50 % vs 0 % R2)
  ```
- **Tag git annoté** : `phase-3-r3-complete`
- **Bump version `Cargo.toml`** : 0.1.0 → 0.2.0 (cf. CHANGELOG)
- **2 commits chirurgicaux** :
  1. `feat(phase-3-r3): sprints S1-S9 — production hardening + chaos + AB test`
  2. `docs(phase-3-r3-s10): R3_LIVE_VALIDATION + CHANGELOG [0.2.0] + tag`

#### Critère d'acceptation

- [x] `R3_LIVE_VALIDATION.md` produit (~700 LOC)
- [x] CHANGELOG `[0.2.0]` complet
- [x] Tag git `phase-3-r3-complete` pointant vers le commit final
- [x] 0 régression cumulée R2+R3 (~720+ tests offline attendus)

---

## 4. Architecture cible post-R3

### 4.1 Pipeline Mission A complet R3

```
                ┌────────────────────────────┐
                │ MissionARequest            │
                │ + task_class               │
                │ + scoring_table            │
                │ + tenant_id (R3.S5)        │
                │ + cost_microcents budget   │
                └──────────────┬─────────────┘
                               ▼
              ┌─────────────────────────────────┐
              │ PolicyRegistry lookup           │ (R3.S5)
              │ → override scoring_table        │
              │ → retry_policy custom           │
              └──────────────┬──────────────────┘
                             │
              ┌──────────────▼──────────────────┐
              │ SlmCircuit guard analyzer       │ (R3.S2)
              │ if !is_open() → analyzer.invoke │
              │ → record_success/failure        │
              └──────────────┬──────────────────┘
                             │
              ┌──────────────▼──────────────────┐
              │ Router S6 tuned                 │
              │ (microcents-aware R3.S1)        │
              └──────────────┬──────────────────┘
                             │
              ┌──────────────▼──────────────────┐
              │ SlmCircuit guard compressor     │ (R3.S2)
              └──────────────┬──────────────────┘
                             │
              ┌──────────────▼──────────────────┐
              │ LLM call + fallback chain S8    │
              │ + ChaosInjector wrap (R3.S6)    │
              │ + Prometheus counter (R3.S7)    │
              └──────────────┬──────────────────┘
                             │
              ┌──────────────▼──────────────────┐
              │ SlmCircuit guard verifier       │ (R3.S2)
              └──────────────┬──────────────────┘
                             ▼
                  ┌─────────────────────┐
                  │ MissionAResponse    │
                  │ + cost_microcents   │ (R3.S1)
                  │ + failure_mode      │
                  │ + ab_test_arm       │ (R3.S8)
                  └─────────────────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │ Prometheus metrics  │ (R3.S7)
                  │ updated             │
                  └─────────────────────┘
```

### 4.2 Modules NEW R3 (5)

| Module | Sprint | Rôle |
|---|---|---|
| `src/multi_tenant.rs` | R3.S5 | `TenantPolicies`, `PolicyRegistry` |
| `src/chaos.rs` | R3.S6 | `ChaosInjector`, `ChaosLlmProvider` |
| `src/metering.rs` | R3.S7 | Prometheus `MetricsRegistry`, HTTP server |
| `src/ab_test.rs` | R3.S8 | `ABTestConfig`, hash bucketing |
| `crates/efe-coordinator/src/llm/providers/slm_drafter.rs` | R3.S4 | `SlmDrafterProvider` |

### 4.3 Modules étendus R3

| Module | Changements R3 |
|---|---|
| `src/llm/provider.rs` | `+cost_microcents: u64`, `+cost_cents() helper` |
| `src/metrics.rs` | `+*_microcents fields`, `+cost_cents() helper` |
| `src/savings_report.rs` | `+RunRecord.cost_microcents`, `+to_markdown_v2()` |
| `src/failure.rs` | (unchanged — déjà extensible) |
| `src/workflows/mission_a.rs` | +SlmCircuit guards (3×), +Prometheus hooks |
| `src/workflows/mission_b.rs` | idem |
| `src/workflows/speculative.rs` | +min_words rejection, +SLM drafter alt |
| `src/speculate/speculator.rs` | +min_words_for_quick_accept |
| `src/coordinator.rs` | +`policies: Arc<PolicyRegistry>`, +`slm_circuits` |
| `src/request.rs` | +`tenant_id: Option<TenantId>` partout |

---

## 5. Effort estimé + coût $ live

| Sprint | Effort dev | Coût $ live | Bloque |
|---|---|---|---|
| R3.S1 | 4h | 0 ¢ | (foundation) |
| R3.S2 | 6h | ~10 ¢ | R3.S6, R3.S9 |
| R3.S3 | 3h | ~5 ¢ | R3.S6, R3.S9 |
| R3.S4 | 8h | ~3 ¢ | R3.S9 |
| R3.S5 | 10h | ~10 ¢ | — |
| R3.S6 | 12h | ~$1 | R3.S9 |
| R3.S7 | 6h | 0 ¢ | — |
| R3.S8 | 8h | ~$2 | — |
| R3.S9 | 4h | ~$2 | R3.S10 |
| R3.S10 | 4h | 0 ¢ | — |
| **TOTAL** | **65h** | **~$5-10** | — |

À raison de 2h/jour soutenu : **~8 semaines** (= 2 mois).
À raison de 4h/jour intensif : **~4 semaines** (= 1 mois).

---

## 6. Critères d'acceptation R3 (binaires)

### 6.1 Foundations (R3.S1-S2)

- [x] `LlmResponse.cost_microcents > 0` mesuré live sur ≥ 5 fixtures
- [x] `SlmCircuitBreaker` wired dans 3 workflows + 1 test live "SLM down"
- [x] Backward-compat 100 % : tous les tests R2 préservés (631+)

### 6.2 Quality (R3.S3-S4)

- [x] Speculator noise rejection `min_words=2` PASS
- [x] Speculative quality ≥ 80 % sur 5 runs FactualQa (vs 50 % R2 sur 2 runs)
- [x] SlmDrafterProvider live PASS avec Qwen3-0.6B local

### 6.3 Production (R3.S5-S7)

- [x] Multi-tenant : 2 tenants distincts → 2 routings différents observés
- [x] Chaos engineering : > 80 % missions complètent sous 30 % fault rate
- [x] Prometheus `/metrics` : 8 métriques R2 exposées + scrape OK

### 6.4 Empirical proof (R3.S8-S9)

- [x] A/B test 2 tables → diff savings statistiquement mesurable
- [x] **`CHEWING_SAVINGS_REPORT_v2.md` montre `savings_cost > 50 %`** ← L1 résolu
- [x] Test live H1-H15 PASS (15/15)

### 6.5 Packaging (R3.S10)

- [x] `R3_LIVE_VALIDATION.md` méta-rapport
- [x] CHANGELOG `[0.2.0]` complet
- [x] Tag git `phase-3-r3-complete`
- [x] Cargo.toml 0.1.0 → 0.2.0

---

## 7. Hors-scope R3 (Phase 4+)

| Item | Phase |
|---|---|
| Frontend SSE (preview lab UI) | Phase 4 |
| Multi-modal (images, audio) | Phase 5 |
| LLM providers additionnels (Cohere, Mistral, Together) | Phase 6 |
| Tests live NATS | Phase 7 |
| Auto-tuning des tables (ML-based) | Phase 8 |
| Inference batching (groupe N queries) | Phase 9 |

---

## 8. Risques R3 connus & mitigations

### R1 — Cost microcents demande bump version workspace

**Risque** : `Cargo.toml` workspace bump 0.0.1 → 0.2.0 touche 30+ crates
non-R3. Risque de casser des dépendances inter-crate.

**Mitigation** :
- Bump uniquement `crates/efe-coordinator/Cargo.toml` (override `version = "0.2.0"` localement)
- Workspace package version inchangée
- CHANGELOG documente clairement la portée du bump

### R2 — Chaos engineering live coûte cher en CI

**Risque** : 30 missions × 5 ¢ × 4 tests live = $6 par run nightly.
Sur 30 jours = $180/mois en CI.

**Mitigation** :
- Tests live chaos NIGHTLY uniquement (1 run/jour)
- Mocks + ChaosInjector pour 95 % des tests offline
- Budget cap dans CI (timeout + max requests)

### R3 — SLM Qwen3 local non disponible cross-platform

**Risque** : Mac M-series uniquement. Linux GPU / Windows non testés.

**Mitigation** :
- R3.S4 tests live opt-in (`EFE_LIVE_SLM=1`)
- Gemini Flash reste fallback drafter cloud
- Phase 4+ peut ajouter Linux GPU support

### R4 — A/B test demande gros volume pour signifiance stat

**Risque** : 30 missions/bucket = N=30, faible puissance statistique.
Détection de différence < 5 % impossible.

**Mitigation** :
- R3.S8 produit comparaison **directionnelle**, pas significativité
- Sur ≥ 100 missions/bucket en prod réelle → significativité mesurable
- Phase 5+ : intégration avec Bayesian A/B test framework

### R5 — Régen savings report v2 peut révéler que le savings réel est < projection

**Risque** : Si après microcents la mesure montre savings = 30 % au lieu
des ×16 projetés, l'argument économique est érodé.

**Mitigation** :
- Soyez préparé à des chiffres moins glorieux mais **honnêtes**
- 30 % réel mesuré > 0 % apparent R2
- Honnêteté chirurgicale > marketing optimiste

---

## 9. Vérification post-R3

```bash
# 1. Cost microcents partout
cargo test -p efe-coordinator --features all-providers --lib | \
    grep "cost_microcents\|microcents"

# 2. SlmCircuitBreaker wired
cargo test -p efe-coordinator --features all-providers \
    --test workflow_failure_modes mission_a_skips_analyzer

# 3. Chaos engineering live (opt-in)
EFE_LIVE_LLM=1 EFE_CHAOS_RATE=0.3 cargo test \
    --test live_chaos_* -- --test-threads=1 --nocapture

# 4. Régen CHEWING_SAVINGS_REPORT_v2.md
EFE_LIVE_LLM=1 EFE_LIVE_SLM=1 cargo test \
    --test live_chewing_savings_report_v2 -- --nocapture

# 5. Prometheus endpoint
cargo run --bin efe-metrics-server &
curl http://localhost:9090/metrics | head -50

# 6. Suite offline complète
cargo test -p efe-coordinator -p efe-nano-app-coordinator \
    --features all-providers --tests --lib

# 7. Tag git
git log phase-3-r3-complete --oneline -1
```

---

## 10. Lien avec Phase 4

R3 livre la **production hardening** ; Phase 4 livre le **frontend SSE**
(preview lab UI) qui consomme :

- `StreamEvent` (déjà stable R2)
- `MissionA/B/SpecResponse` (extended R3 avec cost_microcents)
- `/metrics` Prometheus (R3.S7) → dashboards Grafana
- `CHEWING_SAVINGS_REPORT_v2.json` (R3.S9) → visualisation

Architecture frontend = Next.js + Tailwind + SSE EventSource, hors-scope R3.

---

## 11. Conclusion R3

R3 est le sprint qui **transforme R2 d'une démo en un produit** :

| Avant R3 (post-R2) | Après R3 |
|---|---|
| `savings_cost = 0 %` (résolution u32) | `savings_cost > 50 %` mesuré empirically |
| `SlmCircuitBreaker` inactif | Wiré, validé live |
| Failure modes simulés | Validés via chaos engineering live |
| Speculator accepte le bruit | `min_words=2` rejette le bruit |
| Cloud uniquement | Local SLM drafter alternative |
| Tables figées | A/B test entre versions |
| Pas de telemetry | Prometheus `/metrics` exposé |
| Sgl tenant | Multi-tenant policies |

**Phase 3 R3 = production-ready intégral.**

Effort total estimé : 65h dev + ~$10 live + 10 sprints chirurgicaux.

---

## Annexes

### A. Mapping découvertes R2 → fixes R3

| Découverte R2 | Fix R3 | Sprint |
|---|---|---|
| **L1** : u32 cents masque savings | Cost microcents | R3.S1 |
| **L2** : SlmCircuitBreaker non wiré | Wiring workflows | R3.S2 |
| **L3** : Failure modes simulés | Chaos engineering | R3.S6 |
| **L4** : Speculator accepte bruit | min_words rejection | R3.S3 |
| **D7** : u32 résolution | Microcents (= L1) | R3.S1 |
| **D8** : Fixtures keywords trop stricts | (déjà patché S9.1) | — |

### B. Comparaison V1 (R2) vs V2 (R3)

| Métrique | R2 final | R3 attendu | Delta |
|---|---|---|---|
| Tests offline | 631 | ~720 | +89 (~14 %) |
| Tests live PASS | ~169 assertions | ~225 assertions | +56 (~33 %) |
| Architecture docs | 8 (~7900 LOC) | 9 (R3 archi + 1 R4 transition) | +1 |
| Modules src | ~25 modules | +5 (R3 new) | +5 |
| **Savings cost mesuré** | **0 %** (u32) | **>50 %** (microcents) | **+ ∞** |
| **Speculative quality** | **50 %** (2 runs) | **≥80 %** (5+ runs) | **+30 pp** |
| **Failure modes empirically** | **0** validation | **≥4** live tests | nouveau |

### C. Pourquoi 10 sprints (et pas plus/moins) ?

- **Moins de 10** : sprints groupés trop denses, risque bugs latents
  (cf. découvertes R2 post-impl)
- **Plus de 10** : sprints trop granulaires, packaging overhead > content
- **10 = sweet spot** : 4-12h par sprint, chacun a une thèse atomique
  testable + livrable scellable

### D. Stratégie de release semver post-R3

| Version | Date | Contenu |
|---|---|---|
| 0.0.1-0.0.5 | R1, R1b | Provider scaffolding |
| 0.1.0 | R2 | Foundation Layer (S1-S9 R2) |
| **0.2.0** | **R3** | **Production Hardening** |
| 0.3.0 | Phase 4 | Frontend SSE |
| 1.0.0 | Phase 5+ | Multi-modal + GA release |

Convention : MINOR bump par phase, PATCH par sprint patch chirurgical
(comme S9.1 R2).
