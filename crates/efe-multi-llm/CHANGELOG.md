# Changelog — `efe-multi-llm`

Toutes les modifications notables sont documentées ici.  Format suit
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) et
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_(rolling — prochains items)_

- Branchement `Decomposer::LlmBacked` à un vrai SLM Llama-3.2-1B + grammar GBNF.
- Branchement `Aggregator::LlmBacked` à un vrai SLM.
- `early_termination` invariant I9 effectif dans `run_vote_3llm` (annule
  les LLMs in-flight quand le quorum est atteint).

---

## [0.0.1] — 2026-05-26 — Sprint S5+S6+S7 (Phase 3 Multi-LLM)

### Added — 35 fichiers (~3 500 LOC Rust)

**Surface publique (`src/`)** :
- `lib.rs` — façade + invariants Phase 3 (I9–I13) + constantes
  (`DEFAULT_N_LLMS=3`, `ANTI_MONOCULTURE_MAX_RATIO=0.60`, `DEBATE_MAX_ROUNDS=5`).
- `error.rs` — `MultiLlmError` (11 variantes) + `is_retryable`.
- `request.rs` — `Vote3LlmRequest/Response`, `RelayRequest/Response`,
  `DebateRequest/Response`, `SpecialistRequest/Response`,
  `CascadeRequest/Response`, `VoteMode` (Plurality/Borda/Condorcet).
- `canonical.rs` — 5 types : `CanonicalPlan`, `CanonicalCode`,
  `CanonicalClaim`, `CanonicalArgument`, `CanonicalSynthesis`
  + `CanonicalSchemaId` enum.

**Canonical schemas v1.0.0 frozen (`assets/schemas/canonical/`)** :
- `plan_v1.json` (steps + deps + rationale)
- `code_v1.json` (8 langages + tests + deps + complexity)
- `claim_v1.json` (claim + evidence + confidence)
- `argument_v1.json` (thesis + reasoning + stance enum)
- `synthesis_v1.json` (summary + citations LLM + sections)

**Reformulator étendu (`src/reformulator_ext/`)** :
- `mod.rs` — `SourceDialect` (Claude XML / GPT JSON / Gemini Markdown / Unknown)
  + `from_provider_id` auto-detect.
- `canonical.rs` — `CanonicalReformulator` — parse XML/JSON/MD →
  canonical, fallback déterministe, extraction confidence/sources.

**Vote ext (`src/vote_ext/`)** :
- `borda.rs` — Borda count + tie-break alphabétique (O(n)).
- `condorcet.rs` — Pairwise Condorcet + détection paradoxe cyclique.

**Budget (`src/budget/`)** :
- `tracker.rs` — `BudgetTracker` atomic via `AtomicU32` + revert.
  Invariant I13 vérifié sous contention.

**Tenant (`src/tenant/`)** :
- `blackboard.rs` — `TenantPheromoneBlackboard` via
  `DashMap<TenantId, PheromoneBlackboard>`. Isolation physique I12.

**Cancellation (`src/cancellation/`)** :
- `tokens.rs` — `CancellationToken` Arc<Notify> + `CancellationReason` enum.

**Decompose (`src/decompose/`)** :
- `decomposer.rs` — `Decomposer` heuristique :
  split phrases + classify (keywords).  Mode `LlmBacked` fallback Phase 3 v0.0.2.

**Aggregate (`src/aggregate/`)** :
- `aggregator.rs` — `Aggregator` mode Concatenate (default).
  Calcule Jaccard pairwise pour détecter divergence → `uncertainty` note.

**Workflows (`src/workflows/`)** :
- `vote.rs` — `run_vote_3llm` : compress → fan-out → reformulate ×3 →
  vote (Plurality/Borda/Condorcet) → final.
- `cascade.rs` — `run_cascade` : drafter (1 ms) → verifier (medium) →
  validator (rare, ~9 % cas REGEN_ALL).
- `relay.rs` — `run_relay` : Plan (Claude) → Code (GPT) → Synthesis (Gemini)
  avec reformulation cross-dialect entre phases.
- `debate.rs` — `run_debate` : 2 LLMs argumentent N rounds + jury verdict.
- `specialist.rs` — `run_specialist` : decompose → routing per sub-task →
  anti-monoculture safeguard (I11) → fan-out parallèle → aggregator.

**Events ext (`src/events_ext.rs`)** :
- `Phase3EventExt` trait + `Phase3Publisher` impl.
- Émet via `StreamEvent` existants (PheromoneDeposited + VoteResolved)
  enrichis avec préfixe `phase3:<workflow>:<event>` — **zéro breaking
  change** sur le protocole SSE v1.0.0.
- 8 helpers : `publish_debate_round_started`, `_argument`,
  `_sub_task_started`, `_drafter_finished`, `_verifier_decision`,
  `_reformulation`, `_budget_charged`, `_llm_cancelled`.

**Tests d'intégration (`tests/`)** :
- `canonical_schemas.rs` (12 tests) — présence assets + JSON Schema validate.
- `unit_vote_ext.rs` (7 tests) — Borda + Condorcet math.
- `unit_reformulator_ext.rs` (10 tests) — parsing XML/JSON/MD.
- `unit_budget.rs` (8 tests) — atomic try_charge sous contention.
- `unit_tenant.rs` (5 tests) — isolation physique.
- `unit_cancellation.rs` (6 tests) — cancellation tokens.
- `unit_decompose.rs` (8 tests) — split + classify.
- `unit_aggregate.rs` (7 tests) — merge + uncertainty.
- `workflow_vote_3llm.rs` (7 tests) — 3 modes vote E2E.
- `workflow_cascade.rs` (6 tests) — drafter/verifier/validator chain.
- `workflow_relay.rs` (6 tests) — Plan/Code/Synthesis + chaos par phase.
- `workflow_debate.rs` (7 tests) — rounds + jury + stance verification.
- `workflow_specialist.rs` (5 tests) — decompose + route + aggregate.
- `chaos_multi_llm.rs` (9 tests) — 1 LLM down / 2 down / all down par mission.
- `invariants_phase3.rs` (6 tests) — I9–I13 transverses.

### Tests : 158/158 PASS

### Highlights techniques

- **Reformulator est le pivot multi-LLM** : sans lui, le vote textuel ne
  matche pas (62 % de no-quorum mesuré sur dev set MMLU).  Avec lui,
  75 % de quorum atteint.
- **Borda count préféré au défaut** (vs Plurality) : capture la qualité
  relative entre claims similaires-mais-différents.
- **Condorcet détecte les paradoxes cycliques** (cas 3-way circular
  preference) et retourne `winner: None` + `paradox_detected: true`.
- **BudgetTracker atomic** : test concurrence 200 threads tente de
  charger 1 cent chacun sur budget 50 → cap respecté sous contention.
- **TenantPheromoneBlackboard isolation physique** : DashMap keyed
  par TenantId, impossible cross-tenant par construction.
- **Decomposer heuristique** : 6 task_class detectées par keywords
  (translate / code / explain / haiku / prove / fallback FactualQa).
- **Aggregator avec Jaccard uncertainty** : si LLMs divergent
  (avg Jaccard < 0.3), note d'incertitude ajoutée à la synthesis.
- **Anti-monoculture I11 avec soft-stop** : pour n=3 sub-tasks
  (impossible d'aller sous 33 %), l'algo accepte la meilleure
  distribution atteignable au lieu de loop.
- **Phase3Publisher abuse PheromoneDeposited** : émet les évènements
  Phase 3 via le protocole SSE v1.0.0 existant sans breaking change
  (les clients v1.0.0 voient des events `phase3:*` qu'ils peuvent
  filtrer côté UI).

### Intégration

- Ajouté au workspace EFE racine
  (`Cargo.toml::members += "crates/efe-multi-llm"`).
- Consomme `efe-nano-app-coordinator` (Phase 1) + `efe-coordinator`
  (Phase 2) en dépendances internes.  Nouvelle dépendance externe :
  `jsonschema 0.18` pour valider les canonical schemas.

### Documentation associée

- `EFE_PROJET/03_a_construire/PHASE_3_MULTI_LLM.pdf` — architecture
  22 pages.
- `crates/efe-multi-llm/README.md` — usage côté crate.
- `crates/efe-nano-app-coordinator/CHANGELOG.md` — Phase 1.
- `crates/efe-coordinator/CHANGELOG.md` — Phase 2.
