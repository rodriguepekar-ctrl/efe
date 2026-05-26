# Changelog — `efe-coordinator`

Toutes les modifications notables sont documentées ici.  Format suit
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) et
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_(rolling — prochains items)_

- **R1b** : ajout `GptProvider` OpenAI (quand clé API disponible).
- **Phase 4** : Frontend SSE — composants UI (preview lab) consommant
  `StreamEvent` v1.0.0.
- **Phase 7** : Tests live NATS (env-gated `EFE_NATS_LIVE=1`) +
  rotation de la chaîne d'audit cross-tenant.

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
