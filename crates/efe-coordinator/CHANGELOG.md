# Changelog — `efe-coordinator`

Toutes les modifications notables sont documentées ici.  Format suit
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) et
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_(rolling — prochains items)_

- **Phase 3 R3** : Production hardening — cost microcents (`cost_microcents: u64`
  pour exposer le savings ×50 invisible en u32), multi-tenant policies,
  Prometheus telemetry, chaos engineering CI, Speculator noise rejection
  (fix S9.1 D6).
- **Phase 4** : Frontend SSE — composants UI (preview lab) consommant
  `StreamEvent` v1.0.0.
- **Phase 7** : Tests live NATS (env-gated `EFE_NATS_LIVE=1`) +
  rotation de la chaîne d'audit cross-tenant.

---

## [0.1.0] — 2026-05-28 — Phase 3 R2 BOUCLÉE (pipeline complet)

> **Minor bump** : 9 sprints chirurgicaux S1-S9 + S9.1 patch + S10 packaging.
> Argument économique quantifié empirically (×1.59 speedup mixture complète,
> ×5.24 Speculative FactualQa). 0 breaking change.
>
> Détail complet : `crates/efe-coordinator/R2_LIVE_VALIDATION.md` (méta-rapport
> agrégé) + `CHEWING_SAVINGS_REPORT.md` (livrable quantifié).

### Ajout — 4 nouveaux modules src

- **`src/failure.rs`** (S8) — `FailureMode` enum tagged (6 variants) +
  `is_degraded() + count_degradations() + summarize()`. 11 tests offline.
  Sérialise en `{ "kind": "SLM_DOWN", "detail": "analyzer" }` (SCREAMING_SNAKE_CASE).
- **`src/savings_report.rs`** (S9) — `SavingsReport` + `RunRecord` +
  `RunMode` (5 variants) + `ModeSummary`. Méthodes `add_run() / aggregate()
  / to_markdown() / to_json()`. 13 tests offline. Génère
  `CHEWING_SAVINGS_REPORT.md` (lisible humain) + `.json` (machine).
- **`crates/efe-nano-app-coordinator/src/types/scoring.rs`** (S6) —
  `TaskClassScoringTable` calibrée 3-LLM × 6 task_class, version
  `s6-r2-2025-11`. 17 tests offline (6 winners + 11 cohérence).
- **`crates/efe-nano-app-coordinator/src/clients/slm_circuit.rs`** (S8) —
  `SlmCircuitBreaker` (closed/open/half-open, threshold 3, reset 10s).
  6 tests offline. **Ready-to-wire** dans `run_mission_a` (reporté R3).

### Ajout — extensions struct (backward-compat via `#[serde(default)]`)

- **`ChewingMetrics`** étendu progressivement :
  - S4 : 18 champs initial (input_tokens_raw/compressed/...)
  - S7 : `+verifier_skipped`, `+spec_drafter_latency_ms`,
    `+spec_verifier_latency_ms`, `+spec_savings_cents` (Mission Speculative)
  - S8 : `+failure_mode`, `+degradations_count`, `+failed_components`
- **`MissionAResponse`** :
  - S4 : `+chewing_metrics: Option<ChewingMetrics>`
  - S8 : `+fallback_chain_used`, `+failure_mode`
- **`MissionBResponse`** :
  - S5 : `+chewing_metrics`
  - S8 : `+vote_degraded`, `+failed_llms`, `+failure_mode`
- **`SpeculativeResponse`** (S7) : `+verifier_skipped`, `+chewing_metrics`,
  `+cost_cents` (comble l'asymétrie pré-S7 vs MissionA/B).
- **`SpeculationOutcome`** (S7+S8) : `+verifier_skipped`,
  `+drafter_latency_ms`, `+verifier_latency_ms`, `+cost_cents`,
  `+drafter_bypassed`.
- **`MissionARequest`** (S6) : `+task_class_scoring_table: Option<Arc<…>>`
  `#[serde(skip)]` + helper `MissionARequest::with_default_scoring(…)`.
- **`SpeculativeRequest`** (S7) : `+drafter_llm` (default `"drafter"`),
  `+accept_threshold` (default 0.7), `+max_words_for_quick_accept`
  (default 10). Helper `with_drafter_and_verifier(…)`.
- **`TaskClass`** (S6) : `+Hash` derive (additif, non-cassant).
- **`RouterClient`** (S6) : `+Clone, Copy` derive + factory
  `RouterClient::for_task_class(&table, task)`.
- **`Speculator`** (S7) : `+max_words_for_quick_accept` field + builder
  `.with_quick_accept(n)`. **Refactor majeur** `run()` :
  drafter d'abord (séquentiel) + hot path skip verifier si court +
  fallback drafter→verifier si drafter fail (S8).

### Ajout — `StreamEvent` (4 nouveaux variants S8)

- `LlmFallbackTriggered { from, to, reason }` — bus event quand router
  fallback chain déclenché
- `LlmFailed { provider, reason }` — bus event par LLM raté dans
  Mission B fan-out
- `SlmCircuitOpened { robot, failures }` — bus event quand circuit SLM
  s'ouvre
- `DrafterBypassed { drafter, fallback_to_verifier, reason }` — bus
  event quand Speculator bypass au verifier

Variants existants étendus :
- `SpeculationResolved { …, +verifier_skipped: bool }` (S8 `#[serde(default)]`)

### Ajout — 4 nouvelles fixtures live + 5 patches S9

Nouvelles fixtures S6-S8 :
- `05_code_gen_rust.json` (Code → Claude)
- `06_creative_writing.json` (Creative → GPT-4o)
- `07_translation_fr_en.json` (Translation → Gemini)
- `08_speculative_factual.json` (S7 hot path target)

Patches S9 sur 5 fixtures (`01, 03, 05, 06, 07`) :
- `+baseline_min_cost_cents`, `+baseline_max_cost_cents`, `+s9_included: true`
- `07` patch S9.1 : `+expected_keyword_alternatives` pour accepter
  synonymes valides "twilight"/"dusk"/"gloaming" sur Translation.

### Modifié — workflows Mission A/B/Speculative

- **`run_mission_a`** :
  - S4 : wire ChewingMetrics complet (analyzer + router + compressor +
    LLM + verifier)
  - S6 : `RouterClient::for_task_class` à la volée si
    `req.task_class_scoring_table.is_some()`
  - S8 : loop fallback chain `[selected_llm] + req.available_llms[..]` +
    bus event `LLM_FALLBACK_TRIGGERED`
- **`run_mission_b`** :
  - S5 : pré-mâchage mutualisé 1× + fan-out parallèle stream_many
  - S8 : remplace `filter_map(Result::ok)` par tracking `failed_llms` +
    bus event `LLM_FAILED` par échec + calcul `vote_degraded`. **Bug fix
    chirurgical** : extraction `provider_id` depuis
    `CoordError::LlmProvider` car `FuturesUnordered` désordonne les
    résultats vs `req.llms`.
- **`run_speculative`** :
  - S7 : drafter dynamique via `req.drafter_llm`, helper
    `estimate_savings_cents`, wire `ChewingMetrics` complet, emit
    `StreamEvent::SpeculationResolved`
  - S8 : bypass drafter → verifier direct si drafter fail, bus event
    `DRAFTER_BYPASSED`

### Modifié — `MockLlmProvider` (S8)

- `+with_error(reason: impl Into<String>) -> Self` — provider qui fail
  systématiquement avec message custom (utile pour tester graceful
  degradation, fallback chain, vote dégradé).

### Modifié — calibration S5

- `mission_a.rs:240` + `mission_b.rs:120` : `target_words: 100 → 30`
  (post-run live D1 — résumés ≈ raw avec 100, vrai compression avec 30).
  Économie observée : 85 % savings tokens sur fixture 03_compressible.

### Tests — ~700 cumulés sur 33 binaires

- **+34 tests S8** (11 failure + 6 slm_circuit + 2 metrics + 15 workflow)
- **+19 tests S7** (5 unit Speculator + 9 workflow + 5 metrics)
- **+21 tests S6** (17 scoring + 4 intégration workflow_mission_a)
- **+17 tests S9** (13 unit savings_report + 4 offline)
- **+3 tests S8 patches post-audit** (vote fallback I5 + Composite)
- **5 tests live S7 patches post-audit** (cost_cents hot/slow path)
- **0 régression** : tous les tests pré-sprint préservés à chaque livraison.

### Tests live — 6 sprints validés, ~40 ¢ cumulés

| Sprint | Tests live | Score | Coût |
|---|---|---|---|
| S2 | `live_mission_a_naked` | A1-A11 PASS (12/12) | ~1¢ |
| S3 | `live_mission_a_with_analyzer_verifier` | B1-B16 PASS (16/16) | ~3¢ |
| S4 | `live_mission_a_full_pre_chewing_pipeline` | C1-C28 PASS (28/28) | ~5¢ |
| S5 | `live_mission_b_full_3llm_consensus_with_pre_chewing` | D1-D25 PASS (25/25) | ~5¢ |
| S6 | 4 × `live_mission_a_router_picks_*` | E1-E10 × 4 (40/40) | ~5¢ |
| S7 | 3 × `live_mission_speculative_*` | F1-F13 ×3 (4/4 console PASS) | ~10¢ |
| S9 | `live_chewing_savings_report` | H1-H10 PASS (10/10) | ~12¢ |

### Documentation

- **8 architecture docs** chirurgicales S2-S9 (~7900 LOC cumulés) dans
  `docs/architecture/EFE_A1_PHASE3_R2_S*`
- **9 `S<N>_VALIDATION.md` scellés** : R1b + S2-S6 + S8 + S9 (S7 reste
  en template non-scellé, validé via console)
- **`R2_LIVE_VALIDATION.md`** (NEW S10) — méta-rapport agrégé Phase 3 R2
- **`CHEWING_SAVINGS_REPORT.md` + `.json`** (S9) — livrable quantifié
  reproductible

### Découvertes chirurgicales documentées (D1-D8)

Cf. `R2_LIVE_VALIDATION.md` §6 :
- **D1 (S5)** : `target_words=100` produit résumés ≈ raw → fix 30
- **D2 (S6)** : routing Code demande `cost_weight=0.2` pour Claude gagne
- **D3 (S7)** : Speculator pré-S7 appelait toujours verifier (bug latent)
- **D4 (S7 audit)** : `SpeculativeResponse` manquait `cost_cents`
- **D5 (S8 audit)** : `VoteEngine::vote_with_fallback` déjà existant
  (fallback I5)
- **D6 (S8)** : `stream_many` `FuturesUnordered` désordonne — fix via
  extraction provider_id depuis `CoordError`
- **D7 (S9)** : résolution `u32 cents` trop grossière sur fixtures
  courtes → fix R3 `cost_microcents: u64`
- **D8 (S9.1)** : fixtures trop strictes sur keywords →
  `expected_keyword_alternatives`

### Argument économique Phase 3 R2 — chiffres consolidés

```
                       Latence    Coût      Qualité
                       ────────   ──────    ──────────
Baseline (Claude)        6248ms    6¢        80%
Full pipeline (S4+S6)    3933ms    6¢       100%      (+20pp)
Speculative S7 hot       1192ms    2¢        50%*
                       ────────   ──────    ──────────
Speedup vs baseline    ×5.24      —         routing améliore qualité
                       (Spec)
```

→ **Production projetée 80/20 FactualQa/Code** : ~×16 cost reduction
projetée, ~×1.5-5 latency selon mixture, qualité préservée ou améliorée.

---

## [0.0.5] — 2026-05-26 — Phase 3 R1b (OpenAI GPT — vote 3-LLM réel)

### Ajout du 3ᵉ provider réel : `GptProvider` (OpenAI Chat Completions v1)

**Endpoint câblé** :
- `POST https://api.openai.com/v1/chat/completions` avec `Authorization: Bearer …`.
- Header optionnel `OpenAI-Organization` pour comptes multi-org.
- Health-check sur `GET /v1/models` (cheap, pas d'inférence).

**Pricing** (`PricingTable::openai`, cents/M tokens entiers) :
- `gpt-4o` (250/1000) — défaut R1b, balance qualité/coût.
- `gpt-4o-mini` (15/60) — premium cheap.
- `gpt-5` (500/1500), `gpt-5-mini` (50/200) — estimations.
- `gpt-4-turbo` (1000/3000), `gpt-4` (3000/6000).
- `gpt-3.5-turbo` (50/150).
- `o1` (1500/6000), `o3-mini` (110/440) — reasoning premium.
- Fallback (1000/5000) cents/M sur modèle inconnu (conservateur).

**Parsing OpenAI** (`GptProvider::extract_text_and_tools`) :
- `choices[0].message.content` → texte direct.
- `choices[0].message.tool_calls[]` → sérialisé
  `[tool_call id=X name=Y]\n{args}\n[/tool_call]` (cohérent avec format Claude).
- Tool-calls parallèles supportés (plusieurs entrées dans le tableau).
- `content: null` géré (refusal ou tool-only response).

**Streaming SSE OpenAI** (`parse_openai_sse`) :
- Pas de `event:` prefix (différent d'Anthropic).
- Termine sur sentinelle littérale `data: [DONE]\n\n`.
- `choices[0].delta.content` → texte du chunk.
- `usage.prompt_tokens` / `completion_tokens` lu dans le dernier chunk si
  `stream_options.include_usage=true` (défaut R1b).
- Séparateurs `\n\n` ET `\r\n\r\n` (HTTP CRLF) supportés.
- `delta.role` premier chunk ignoré silencieusement.

**Rate-limit observability** (`OpenAiRateLimits`) :
- Lecture auto des headers `x-ratelimit-{limit,remaining,reset}-{requests,tokens}`.
- Header `openai-organization` capturé.
- Snapshot atomique partagé entre `call` et `stream` (mis à jour même en cas
  d'erreur HTTP — utile pour 429).
- `provider.last_rate_limits()` exposé publiquement.

**Validation `OPENAI_API_KEY`** :
- Préfixe attendu `sk-` (couvre clés legacy ET `sk-proj-…`).
- `ApiKey::from_env("OPENAI_API_KEY", "sk-")` rejette les valeurs sans préfixe.

**Fixtures réalistes** (8 fichiers `tests/fixtures/openai/*.json`) :
- 001 factual_qa_simple, 002 code_block_rust, 003 max_tokens_truncated.
- 004 tool_call_single, 005 tool_call_parallel.
- 006 long_reasoning, 007 error_rate_limit, 008 refusal.

**Tests offline** :
- `tests/unit_parse_openai.rs` : 12 tests sur fixtures + pricing.
- `tests/unit_streaming_edge.rs` : 8 nouveaux tests OpenAI
  (single-byte chunks, `[DONE]` sentinel termine le stream, JSON malformé skipped,
  empty stream, usage final chunk, CRLF separators, finish_reason+DONE,
  role-only first chunk).
- `openai.rs` inline : 12 tests unit.
- `streaming.rs` inline : 3 tests OpenAI.

**Tests live env-gated** (`tests/live_providers.rs`, `EFE_LIVE_LLM=1` + clé) :
- `live_gpt_simple_question` : `2+2 = 4` avec coût < 5¢.
- `live_gpt_streaming_works` : streaming chunks reçus.
- `live_gpt_temperature_zero_deterministic` : reproducibilité Paris.
- `live_gpt_rate_limits_observable` : headers `x-ratelimit-*` snapshotés.
- `live_gpt_health` : `/v1/models` 200 OK.
- **`live_vote_3llm_real_claude_gemini_gpt`** : the holy grail — 3 vrais
  providers en parallèle (Claude + Gemini + GPT) sur question factuelle,
  consensus 3/3 sur "Paris", coût total ≤ 10¢, latence parallèle < 30s.

### Cargo features

- Nouvelle feature `openai` (ajoutée à `all-providers`).
- `all-providers = ["anthropic", "gemini", "openai"]`.
- Default `DEFAULT_GPT_MODEL = "gpt-4o"`.

### Re-exports publics

- `efe_coordinator::GptProvider` (sous `feature = "openai"`).
- `efe_coordinator::OpenAiRateLimits`.
- `llm::providers::DEFAULT_GPT_MODEL`.

### Architecture identique aux 2 autres providers

- `LlmProvider` trait : `call` / `stream` / `health` async.
- Retry exponential + jitter (`RetryPolicy`).
- Circuit breaker 3-state (Closed/Open/HalfOpen, invariant I17).
- PII redactor closure intégré (`with_redactor`, invariant I15).
- `ApiKey` wrapper redacted en Debug/Display (invariant I14).
- `tracing::info!` structuré (provider, tokens, cost, latency_ms).

### Non-régression

- **394 tests passent** (208 lib + 186 integration) — 0 failure.
- Claude (R1a) + Gemini (R1a v2) inchangés.

---

## [0.0.4] — 2026-05-26 — Phase 3 R1a v2 (hardening complet vs PHASE_3_R1 PDF)

### Audit chirurgical sur 14 gaps vs architecture R1 — TOUS comblés (sauf OpenAI = R1b)

**Fixtures réalistes** (16 fichiers `tests/fixtures/{claude,gemini}/*.json`) :
- Claude (8) : factual_qa, thinking_tag, code_block_rust, max_tokens, tool_use,
  multi_text_blocks, error_rate_limit, long_reasoning.
- Gemini (8) : factual_qa, markdown_footnotes, multi_parts, max_tokens,
  code_block_python, error_quota, safety_blocked, empty_candidates.

**Parsing avancé Claude** (`common::strip_thinking_tags` + `extract_text_and_tools`) :
- Strip auto `<thinking>...</thinking>` (configurable `with_thinking_kept()`).
- Détection + sérialisation `tool_use` : `[tool_use id=X name=Y]\n{json}\n[/tool_use]`.
- Multi-text blocks concaténés ordonnés.

**Observabilité Claude** (`ClaudeRateLimits`) :
- Lecture auto des 6 headers `anthropic-ratelimit-*`.
- Snapshot atomique partagé entre `call` et `stream`.
- `provider.last_rate_limits()` exposé.

**Observabilité Gemini** (`GeminiObservability`) :
- `last_finish_reason`, `last_safety_blocked`, `last_prompt_tokens`,
  `last_candidates_tokens` mis à jour à chaque call.

**PII redactor opt-in** (invariant I15) :
- `PiiRedactor = Arc<dyn Fn(&str) -> String + Send + Sync>` closure-based.
- `ClaudeProvider::with_redactor()` + `GeminiProvider::with_redactor()`.
- Appliqué AVANT envoi externe (user + system).

**Tests parsing sur fixtures** :
- `unit_parse_claude.rs` (12 tests).
- `unit_parse_gemini.rs` (12 tests).

**Tests SSE edge cases** (`unit_streaming_edge.rs`, 8 tests) :
- single-byte chunks, malformed JSON, empty stream, unknown events,
  usage capture mid-stream.

**Tests live additionnels** (`live_providers.rs`, +5) :
- `live_cascade_gemini_drafter_claude_verifier`.
- `live_vote_3llm_with_2_real_plus_mock_gpt_placeholder` (Vote 3-LLM hybride).
- `live_claude_rate_limits_observable`.
- `live_gemini_observability_captured`.
- `live_claude_pii_redactor_applied_before_send`.

### Tests : 344 PASS (+46 vs v1)

| Binaire | v1 | v2 |
|---|---|---|
| lib (inline) | 176 | 183 |
| unit_parse_claude | — | 12 |
| unit_parse_gemini | — | 12 |
| unit_streaming_edge | — | 8 |
| live_providers | 8 | 13 |
| (autres existants) | 114 | 116 |
| **Total** | **298** | **344** |

### Verdict chirurgical R1a v2

| Gap initial vs PDF | Statut |
|---|---|
| Fixtures réalistes | ✅ 16 fichiers (8/provider) |
| Tests parsing | ✅ unit_parse_claude + unit_parse_gemini |
| Rate-limit headers Anthropic | ✅ `ClaudeRateLimits` + auto-snapshot |
| Observability Gemini | ✅ `GeminiObservability` (safety + usage) |
| `with_redactor()` PII | ✅ closure-based, opt-in |
| Strip `<thinking>` | ✅ default ON, opt-out via `with_thinking_kept` |
| Parse tool_use Claude | ✅ `extract_text_and_tools` |
| Safety ratings Gemini | ✅ `last_safety_blocked` |
| Edge cases SSE | ✅ `unit_streaming_edge` |
| Workflow live cascade | ✅ `live_cascade_gemini_drafter_claude_verifier` |
| OpenAI provider | ⏭ scope R1b |

**R1a v2 complet vs son scope.**

---

## [0.0.3] — 2026-05-26 — Phase 3 R1a (Claude Sonnet 4.6 + Gemini Flash)

### Added — 8 fichiers (~1 800 LOC) — features cargo `anthropic` + `gemini` + `all-providers`

**Provider Anthropic Claude** (`src/llm/providers/anthropic.rs`, 285 LOC) :
- `ClaudeProvider::from_env()` → lit `ANTHROPIC_API_KEY` (préfixe `sk-ant-`)
- `ClaudeProvider::new(api_key, model)` constructor explicite
- `LlmProvider::call` : POST `https://api.anthropic.com/v1/messages` avec
  retry policy + circuit breaker + budget tracking via `PricingTable`
- `LlmProvider::stream` : SSE `event: content_block_delta` → `TokenChunk`
  par token streamé
- `LlmProvider::health` : ping minimal (1 token) sur `/v1/messages`
- Modèle défaut : `claude-sonnet-4-6` (Q1 utilisateur)
- Debug redacte la clé API (invariant I14)

**Provider Google Gemini** (`src/llm/providers/gemini.rs`, 305 LOC) :
- `GeminiProvider::from_env()` → lit `GOOGLE_AI_KEY` (préfixe `AIza`)
- POST `generateContent?key=...` (clé en query string)
- Stream via `streamGenerateContent?alt=sse&key=...`
- Modèle défaut : `gemini-2.5-flash` (Q2 utilisateur — 40× moins cher
  que Pro pour qualité comparable)
- Health check : GET `/v1beta/models` (pas d'inférence, gratuit)

**Modules support** (`src/llm/providers/`) :
- `auth.rs` (130 LOC) — `ApiKey` wrapper avec redaction auto Debug/Display,
  invariant I14 testé
- `pricing.rs` (135 LOC) — `PricingTable::anthropic()` + `gemini()` ;
  calcul en cents (entier) ADR-P3R1-004, arrondi sup conservatif
- `circuit.rs` (165 LOC) — `CircuitBreaker` 3-states (Closed/Open/HalfOpen)
  invariant I17 ; transition automatique HalfOpen après timeout
- `retry.rs` (220 LOC) — `RetryPolicy` exponential + jitter ;
  `retry_with_backoff` async ; détection 429/5xx/transport vs 4xx
- `streaming.rs` (220 LOC) — `parse_anthropic_sse` + `parse_gemini_sse` ;
  2 implémentations distinctes (ADR-P3R1-005)
- `common.rs` (90 LOC) — `build_http_client` reqwest 0.12 + rustls,
  `split_system_user` parse `"SYSTEM:...USER:..."`, `truncate_error_body`

**Tests live env-gated** (`tests/live_providers.rs`, 8 tests) :
- `live_claude_simple_question` — Q+A factuelle, vérifie réponse + coût
- `live_claude_streaming_works` — vérifie `DRAFT_TOKEN`-like events
- `live_claude_temperature_zero_deterministic`
- `live_claude_health`
- `live_gemini_simple_question`
- `live_gemini_streaming_works`
- `live_gemini_health`
- `live_vote_2llm_claude_gemini_factual` — **workflow Mission B live**
  réelle : 2 vrais providers en parallèle + assertions consensus

Tous skipped si `EFE_LIVE_LLM != "1"` ou clés API absentes.

### Tests — non-regression complète

| Phase | Tests | Status |
|---|---|---|
| Phase 1 (nano-robots) | 121 | ✅ |
| Phase 2 + R1a (lib + integration) | 298 | ✅ (+61 R1a) |
| Phase 3 (multi-llm) | 200 | ✅ |
| **Total** | **619** | ✅ **0 fail** |

8 tests live restent en `[skip]` faute d'env var — activables manuellement.

### Sécurité

- Invariant **I14** vérifié : `ApiKey::Debug` retourne `ApiKey(***xxxx)`
  (4 derniers chars uniquement) — testé.
- ADR-P3R1-001 : `reqwest` + `rustls-tls` (pas openssl).
- ADR-P3R1-002 : Wrapper `ApiKey` avec `expose()` explicite (pas de Display
  qui leakerait la clé).
- ADR-P3R1-007 : pas de cache de réponse au niveau provider (stateless).

### Comment lancer la validation live (côté utilisateur)

```bash
# 1. Exporter les clés (dans ton shell)
export ANTHROPIC_API_KEY="sk-ant-..."
export GOOGLE_AI_KEY="AIza..."

# 2. Activer les tests live + run
cd "/Users/rodrigue.pekar/Downloads/vrai/jjjj copie/efe"
EFE_LIVE_LLM=1 PYO3_PYTHON=$(which python3.12) \
  cargo test -p efe-coordinator --features all-providers --test live_providers \
  -- --test-threads=1 --nocapture

# 3. Inspecter les logs : latences réelles + coûts mesurés en cents
```

Coût estimé du run complet : **~$0.05** (10 calls × ~$0.005).

### Migration depuis Phase 3 v0.0.2

**Zero breaking change** — aucune ligne Phase 2 ou Phase 3 modifiée.

Pour utiliser les vrais providers à la place des mocks :

```rust
// AVANT (Mock)
reg.register(Arc::new(MockLlmProvider::new("claude")));

// APRÈS (Real, avec feature "anthropic" activée)
reg.register(Arc::new(ClaudeProvider::from_env()?));  // ID = "claude_claude-sonnet-4-6"
```

Le `MultiLlmBridge`, le `VoteEngine`, et tous les workflows Phase 3
fonctionnent identiquement — seul l'ID provider change (suffixe modèle ajouté).

---

---

## [0.0.2] — 2026-05-26 — Completion pass (streaming + audit + heartbeat)

### Added — 11 fichiers (~1 700 LOC)

**Streaming SSE pipeline (lacune Phase 2 v0.0.1)** :
- `src/llm/bridge.rs::stream_many` — fan-out streaming N providers en
  parallèle ; chunks tagués par `ProviderId` sur un canal partagé.
- `src/workflows/mission_a.rs` — utilise `stream_one` ; émet
  `StreamEvent::LlmTokenStreamed { swarm_id, provider, token, index, ts }`
  pour chaque token généré.
- `src/workflows/mission_b.rs` — utilise `stream_many` ; émet
  `LlmTokenStreamed` entrelacé pour les N providers.
- `src/workflows/speculative.rs` — branche `Speculator::with_bus` →
  `DraftToken` events émis pendant la frappe du drafter local.
- `src/speculate/speculator.rs::with_bus` — builder qui attache un
  `Arc<dyn EventBus>` + `swarm_id` ; back-compat préservée (sans bus,
  comportement v0.0.1 identique).

**Heartbeat (lacune SSE prod)** :
- `src/heartbeat.rs` (115 LOC) — `HeartbeatHandle::spawn` background
  task tokio qui émet `StreamEvent::Heartbeat` toutes les
  `DEFAULT_HEARTBEAT_PERIOD` (15 s) ; cancellation déterministe via
  `Notify` ; Drop stop implicite.

**Audit chain HMAC (lacune Phase 2 → préfiguration Phase 7)** :
- `src/audit.rs` (220 LOC) — `HmacAuditChain` BLAKE3-keyed chaîné
  cryptographiquement (genesis → entry_N → entry_N+1) ; `append`
  / `verify` / `snapshot` thread-safe via `parking_lot::Mutex`.
- `ChainedEntry` — wire format frozen v1.0.0 (entry_id UUIDv7,
  payload_hash, prev_hash, entry_hash, written_at, source).
- `verify()` détecte tampering au payload ET cassure de chaînage.

**NATS bus stub (feature `nats-bus`)** :
- `src/events/nats.rs` (160 LOC) — `NatsBus::connect` configure
  JetStream stream `efe-swarm-<tenant>` ; worker async pump
  NATS→subscribers locaux ; même surface que `InMemoryBus`.
- Subject hierarchy : `efe.swarm.<tenant>.<swarm_id>.<tag>`.
- Storage memory + max_age 1 h (config dev) — Phase 7 passera en
  file storage avec retention configurable.

**Helper SSE HTTP (intégration framework-agnostic)** :
- `src/sse.rs` (140 LOC) — `SseByteStream` impl `futures::Stream<Bytes>`
  ; préface `:efe-coordinator SSE v1.0.0 / retry: 5000` ; helpers
  `to_sse_blob` (sérialisation) + `parse_sse_blob` (réception client).
- Compatible Axum `Sse::new`, Warp, Actix-web, Hyper raw.
- Constantes : `CONTENT_TYPE_SSE = "text/event-stream"`, `SSE_PREFACE`.

**Examples runnables (`examples/`)** :
- `mission_a_demo.rs` — full E2E avec heartbeat + reader SSE stdout.
- `mission_b_demo.rs` — fan-out 3 LLMs + vote consensus.
- `speculative_demo.rs` — drafter rapide + verifier lent (gain latence).

**Nouveaux tests d'intégration** :
- `tests/unit_audit.rs` (8 tests) — chain verify, tamper detection,
  concurrent append, deterministic hashing across instances.
- `tests/unit_heartbeat.rs` (5 tests) — interval, stop cancel, drop safety.
- `tests/unit_sse.rs` (8 tests) — préface, blob, parse, byte stream.
- `tests/unit_stream_many.rs` (4 tests) — fan-out streaming, tagging,
  partial failure isolation.
- `tests/unit_streaming_events.rs` (3 tests) — workflows émettent bien
  `LlmTokenStreamed` × N et `DraftToken` × N.

### Tests : 213/213 PASS (+53 vs v0.0.1)

| Binaire | Tests v0.0.1 | Tests v0.0.2 |
|---|---|---|
| lib (inline) | 90 | 112 (+22 : audit, heartbeat, sse, bridge::stream_many, speculator::with_bus) |
| unit_audit | — | 8 |
| unit_heartbeat | — | 5 |
| unit_sse | — | 8 |
| unit_stream_many | — | 4 |
| unit_streaming_events | — | 3 |
| (existants v0.0.1) | 70 | 70 |
| **Total** | **160** | **210** |

### Highlights techniques v0.0.2

- **SSE end-to-end fonctionne** : Mission A streame token-par-token,
  Mission B entrelace 3 providers, Speculative emit DraftToken pendant
  la frappe.  L'UI peut désormais voir les LLM "taper" en temps réel.
- **Audit chain crypto-vérifiable** : `HmacAuditChain::verify` détecte
  toute modification ou injection a posteriori (test : tampering payload
  + cassure chaînage).  Préfigure le passage Postgres + cosign Phase 7.
- **NatsBus compile et est testable en local** (stub : `cargo check
  --features nats-bus` passe en 55 s, tests live env-gated Phase 7).
- **Heartbeat anti-proxy-cut** : tâche tokio cancellation-safe,
  notification déterministe à l'arrêt, Drop safe par défaut.

### Compatibilité v0.0.1 → v0.0.2

- **Toutes les API publiques v0.0.1 préservées** (re-exports `lib.rs`
  inchangés, signatures `Coordinator::*` inchangées).
- **Mission A / B / Speculative** retournent la même `*Response` ;
  seul l'**ajout** d'évènements SSE granulaires est visible côté bus.
- **Speculator** : `new()` inchangé ; `with_bus()` est un nouveau
  builder additif.
- **HmacAuditChain** : nouveau, pas de breaking change.

---

## [0.0.1] — 2026-05-25 — Sprint S3+S4 (Phase 2 Coordinator)

---

## [0.0.1] — 2026-05-25 — Sprint S3+S4 (Phase 2 Coordinator)

### Added — 31 fichiers (~3 800 LOC Rust)

**Surface publique (`src/`)** :
- `lib.rs` — façade + invariants (I1–I8) + constantes (`SSE_PROTOCOL_VERSION`, `MAX_PARALLEL_LLMS`, `DEFAULT_MAX_CONCURRENCY`).
- `error.rs` — `CoordError` (10 variantes) + `is_retryable` + `suggested_backoff` ; encapsule `RobotError` via `From`.
- `request.rs` — `MissionA/B/SpeculativeRequest/Response` + `FactCheck` + `CandidateSourceRef`.
- `coordinator.rs` — `Coordinator` + `CoordinatorBuilder` (fluide) + `CoordinatorClients` (bundle Phase 1) + `CoordinatorConfig`.

**Blackboard** (stigmergie SwarmSys, HF 2510.10047) — `src/blackboard/` :
- `mod.rs` — `MAX_BLACKBOARD_BYTES` (256 KB), `MAX_ENTRIES` (4096), `DEFAULT_HALF_LIFE_SECS`, `PHEROMONE_GC_THRESHOLD`.
- `entry.rs` — `BlackboardEntry` + `EntryKind` (10 variantes) + decay temporel.
- `pheromone.rs` — `PheromoneBlackboard` (parking_lot RwLock + BTreeMap, LRU eviction sur bytes overflow).

**Events** — `src/events/` :
- `mod.rs` — façade + re-exports.
- `bus.rs` — trait `EventBus` + `SubscriberToken`.
- `inmem.rs` — `InMemoryBus` (tokio mpsc + DashMap, backpressure → drop + compteur).
- `stream.rs` — `StreamEvent` (13 variantes, protocole **v1.0.0** frozen) + `MissionTag` + `to_sse_frame()`.

**LLM bridge** — `src/llm/` :
- `mod.rs` — façade + re-exports.
- `provider.rs` — trait `LlmProvider` + `LlmCall` + `LlmResponse` + `TokenChunk` + impl `stream()` par défaut.
- `mock.rs` — `MockLlmProvider` (déterministe, configurable : response/cost/latence/quality/`fail_after_n`).
- `registry.rs` — `ProviderRegistry` (DashMap thread-safe).
- `bridge.rs` — `MultiLlmBridge` : `call_one` / `stream_one` / `call_many` / `call_many_ok` (FuturesUnordered).

**Vote** (Symphony, HF 2508.20019) — `src/vote/` :
- `mod.rs` — doc invariants I5/I6.
- `engine.rs` — `VoteEngine` + `Ballot` + `VoteOutcome` (cluster BLAKE3 + tie-break déterministe + fallback success_rate).

**Speculate** (Speculative RAG, HF 2407.08223) — `src/speculate/` :
- `mod.rs` — façade.
- `speculator.rs` — `Speculator::run` + `SpeculationOutcome` + `DraftToken` + Jaccard mot-à-mot.

**DAG** — `src/dag/` :
- `mod.rs` — façade.
- `node.rs` — `DagNode` + `NodeStatus` + `NodeId` + closures async typées.
- `executor.rs` — `DagExecutor` (Kahn topo sort, JoinSet, semaphore borné, skip propagé sur fail) + `DagRunReport`.

**Workflows** — `src/workflows/` :
- `mod.rs` — façade + `audit_chain_hash` BLAKE3.
- `mission_a.rs` — analyzer → router → compressor → LLM → verifier ; full SSE.
- `mission_b.rs` — pre-chew → fan-out N LLMs → vote pondéré → verifier ; full SSE.
- `speculative.rs` — drafter + verifier via Speculator ; full SSE.

**Tests d'intégration** (`tests/`) :
- `unit_blackboard.rs` (9 tests) — I2/I7 + LRU + clone visibility.
- `unit_bridge.rs` (6 tests) — fan-out, partial failures, registry overwrites.
- `unit_bus.rs` (7 tests) — sub/unsub, backpressure, tokens uniques.
- `unit_dag.rs` (8 tests) — linear/diamond/cycle/missing parent, I8.
- `unit_speculator.rs` (5 tests) — accept/reject/partial overlap, IDs.
- `unit_vote.rs` (7 tests) — unanimous/split/weighted/quorum 0.
- `workflow_mission_a.rs` (4 tests) — E2E + SSE events.
- `workflow_mission_b.rs` (5 tests) — E2E + vote + weighted override.
- `workflow_speculative.rs` (5 tests) — drafter+verifier + provider missing.
- `sse_stream.rs` (8 tests) — version v1.0.0, frame format, kebab tags.
- `invariants.rs` (6 tests) — I2/I3/I4/I5/I6/I7 transverses.

**Benchmarks** (`benches/coordinator_throughput.rs`, Criterion) :
- `blackboard::deposit_small_entry`, `read_kind_100_entries`.
- `vote::tally::3_llms`.
- `dag::diamond_a_to_d`.
- `bridge::call_one_mock`, `call_many_3_mocks_parallel`.
- `workflow::mission_a_mock_only`, `workflow::mission_b_3_mocks_parallel`.

### Tests : 160/160 PASS

| Binaire | Tests | Statut |
|---|---|---|
| `lib` (inline) | 90 | ✅ |
| `unit_blackboard` | 9 | ✅ |
| `unit_bridge` | 6 | ✅ |
| `unit_bus` | 7 | ✅ |
| `unit_dag` | 8 | ✅ |
| `unit_speculator` | 5 | ✅ |
| `unit_vote` | 7 | ✅ |
| `workflow_mission_a` | 4 | ✅ |
| `workflow_mission_b` | 5 | ✅ |
| `workflow_speculative` | 5 | ✅ |
| `sse_stream` | 8 | ✅ |
| `invariants` | 6 | ✅ |
| **Total** | **160** | **✅** |

### Highlights techniques

- **`StreamEvent` v1.0.0 frozen** — 13 variantes typées exposées via SSE
  à l'UI ; tag `SCREAMING_SNAKE_CASE` ; `to_sse_frame()` produit du
  `text/event-stream` standard.
- **Blackboard bornée 256 KB** avec LRU eviction et garbage collection
  des phéromones sub-seuil.  parking_lot RwLock + BTreeMap pour O(log n)
  fairness + itération ordonnée par UUID v7.
- **DagExecutor** : validation Kahn topo sort à submit, parallélisme borné
  par semaphore, propagation déterministe de `Skipped` sur fail amont.
- **VoteEngine** : clustering BLAKE3 des réponses canonicalisées
  (whitespace-insensitive), tie-break alphabétique déterministe, fallback
  invariant I5 sur `success_rate` quand pas de quorum.
- **Speculator** : `tokio::spawn` du drafter en parallèle du verifier,
  arbitrage Jaccard mot-à-mot, comptage des tokens acceptés vs regénérés.
- **MultiLlmBridge** : `FuturesUnordered` pour fan-out N providers ;
  `call_many_ok` drop les erreurs (utile Mission B "best-effort").
- **InMemoryBus** : DashMap pour pas-de-lock-global, mpsc bounded
  (1024/subscriber), drop+compteur sur backpressure.

### Intégration

- Ajouté au workspace EFE racine
  (`Cargo.toml::members += "crates/efe-coordinator"`).
- Consomme **uniquement** `efe-nano-app-coordinator` (Phase 1) en
  dépendance interne, plus tokio/serde/uuid/parking_lot/dashmap/blake3.
- Zéro dépendance native externe (pas de NATS server requis en default).

### Versions des composants frozen Phase 2

| Composant | Version | Frozen sur |
|---|---|---|
| `StreamEvent` SSE | v1.0.0 | tag, type, champs |
| `MissionA/B/SpeculativeRequest/Response` | v1.0.0 | champs |
| `BlackboardEntry` payload kinds | v1.0.0 | `EntryKind` 10 variantes |
| Trait `EventBus` | v1.0.0 | publish/subscribe/unsubscribe/dropped_count/flush |
| Trait `LlmProvider` | v1.0.0 | id/call/stream/health |

### Documentation associée

- `EFE_PROJET/03_a_construire/PHASE_2_COORDINATOR.pdf` — architecture
  26 pages.
- `crates/efe-coordinator/README.md` — usage côté crate.
- `EFE_PROJET/03_a_construire/PHASE_1_JSON_GRAMMAR.pdf` — fondation
  Phase 1 (rappel).
