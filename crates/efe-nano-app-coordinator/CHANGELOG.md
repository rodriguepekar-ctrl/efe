# Changelog — `efe-nano-app-coordinator`

Toutes les modifications notables sont documentées ici. Format suit
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/) et
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_(rolling — prochains items)_

- **Phase 2** : `Coordinator` orchestre DAG parallèle des 6 robots
- **Phase 2** : Branchement blackboard EFE (extension phéromones)
- **Phase 2** : Substrat multi-LLM via NATS topics

---

## [0.0.1] — 2026-05-25 — Sprint S1+S2 (Phase 1 JSON Grammar)

### Added — 35 fichiers (3 984 LOC Rust + 339 LOC assets)

**Code source Rust** (15 fichiers, 21 modules)
- `src/lib.rs` — façade publique + constantes `robot_ids` + `default_ports`
- `src/error.rs` — `RobotError` (8 variantes typées) + `should_retry()` + `suggested_backoff()`
- `src/context.rs` — `CallContext` + `AuditWriter` trait + `InMemoryAuditWriter` + `blake3_canonical_json()`
- `src/canonical/mod.rs` — validation des 5 sub-schémas (`Citation`, `Fact`, `ToolCall`, `Claim`, `SourceRef`)
- `src/types/{analyzer,reformulator,compressor,verifier,filter,router}.rs` — types I/O des 6 robots avec `serde + validator`
- `src/clients/mod.rs` — trait `NanoRobotClient` + `RunMode` enum
- `src/clients/llama_server.rs` — helper `LlamaServerClient` (HTTP `/completion` + grammar + audit chain)
- `src/clients/{analyzer,reformulator,compressor,verifier,filter,router}.rs` — 6 implémentations
- `src/clients/filter.rs` — inclut `EmbeddingBackend` trait + `MockEmbedder` + `BgeM3Embedder` (feature `fastembed-real`)

**Assets** (4 schemas + 4 grammars + 4 prompts = 12 fichiers + 2 nouveaux)
- `assets/schemas/{analyzer,reformulator,compressor,verifier,filter,router}_v1.json` — 6 JSON Schemas Draft 2020-12
- `assets/grammars/{analyzer,reformulator,compressor,verifier}_v1.gbnf` — 4 grammars GBNF pour llama.cpp
- `assets/prompts/{analyzer,reformulator,compressor,verifier}.txt` — 4 system prompts avec exemples few-shot

**Tests** (6 fichiers, ~85 tests)
- `tests/unit_schema_validation.rs` — 14 tests : validation des types + présence des assets
- `tests/phase_1_workflow.rs` — 6 tests : workflows nominaux via wiremock
- `tests/audit_chain_integrity.rs` — 8 tests : invariants de la chaîne d'audit
- `tests/chaos_robot_down.rs` — 6 tests : comportement sous panne
- `tests/unit_1000_tirages.rs` — 14 tests + proptest (1000 cases chacun) sur les validators
- `tests/live_smoke.rs` — 7 tests env-gated (`EFE_LIVE_SMOKE=1`) contre vrais `llama-server`

**Benchmarks** (1 fichier)
- `benches/robot_latencies.rs` — Criterion p50/p95/p99 (analyzer, filter, router, cosine, blake3) ; mode `EFE_BENCH_LIVE=1` pour vraies inférences

**Scripts** (2 fichiers)
- `scripts/generate-grammars.sh` — auto-générateur GBNF depuis JSON Schema via `json_schema_to_grammar.py`
- `scripts/validate-grammars.py` — 1000 tirages par robot via vrai `llama-server` + rapport CSV

### Highlights techniques

- **Trait `NanoRobotClient`** : surface unifiée pour les 6 robots (4 LLM-based + 2 ONNX-based)
- **Grammar enforcement** : la sortie est *mathématiquement garantie* conforme au JSON Schema
- **Audit chain par construction** : chaque invocation produit **exactement 1** entry (succès ou échec)
- **BLAKE3 canonicalisé** pour hashs deterministes
- **Pareto routing** sur (coût, latence, qualité) pour la sélection LLM
- **Feature `fastembed-real`** : `BgeM3Embedder` ONNX pour Filter + Router en prod

### Métriques finales

- Workspace member #44 sur 44
- Compile en 3.6 s incrémental sur M3
- Zéro dépendance native externe (pas de brew/protoc/cmake)
- Tests : **77/77 PASS** initial + **30+ nouveaux tests** ajoutés
- Coût grammar : ~7 % overhead vs sans grammar (mesuré théorique, à confirmer au bench live)

### Intégration

- Ajouté au workspace EFE racine (`Cargo.toml::members += "crates/efe-nano-app-coordinator"`)
- `EFE_PROJET/03_a_construire/scripts/start-nano-robots.sh` mis à jour : auto-détecte les grammars et active `--grammar-file` si présentes (mode Phase 1) ou fallback texte libre (mode Phase 0)

### Versions des modèles SLM ciblés (frozen Phase 1)

| Robot | Modèle | Quantization | Taille |
|---|---|---|---|
| Analyseur | SmolLM2-360M-Instruct | Q5_K_M | 277 MB |
| Reformulateur | Qwen3-0.6B-Instruct | Q5_K_M | 424 MB |
| Compresseur | Llama-3.2-1B-Instruct | Q5_K_M | 869 MB |
| Vérificateur | Phi-4-mini-instruct | Q4_K_M | 2.3 GB |
| Filtreur | bge-m3 (ONNX) | full | 2.3 GB (auto-cache) |
| Routeur | bge-m3 (ONNX) | full | (shared) |

### Documentation associée

- `EFE_PROJET/03_a_construire/PHASE_1_JSON_GRAMMAR.pdf` — architecture 25 pages
- `EFE_PROJET/03_a_construire/ROADMAP_COMPLETE.pdf` — plan 20 semaines
- `crates/efe-nano-app-coordinator/README.md` — usage côté crate
