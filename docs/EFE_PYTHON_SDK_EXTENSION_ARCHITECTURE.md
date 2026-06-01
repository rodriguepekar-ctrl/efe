# `efe-tool` (Python SDK) — Architecture détaillée de l'extension M8

> **Statut** : Draft v1.0.0 — Document de référence pour
> l'achèvement du module **M8 = `clients/python/efe_tool/`**
> entre l'AS-IS (post-D18, **65–75 %** de complétude par lettre :
> A 60 %, B 70 %, C 65 %, D 75 %) et la cible **GA 1.0**
> (4 lettres A→D 100 % hardened + `EfeNativeClient` PyO3
> zero-copy ≥ 5× plus rapide que le client gRPC + sample bundle
> réel signé Ed25519).
>
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-29.
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §5 (Φ-A : D19-D20)
>     + §7 (Φ-C : D25-D27 PyO3) — roadmap globale et liste exhaustive
>     des 23 DTs (DT-A-1..8, DT-B-1..5, DT-C-1..5, DT-D-1..5).
>   * `LETTER_A_ARCHITECTURE.md` (1 456 LOC) — contrat CLI one-shot.
>   * `LETTER_B_ARCHITECTURE.md` (1 542 LOC) — contrat REPL.
>   * `LETTER_C_ARCHITECTURE.md` (1 520 LOC) — contrat scénarios
>     applicatifs (LLM bridges).
>   * `LETTER_D_ARCHITECTURE.md` (1 596 LOC) — contrat lib intégration
>     (façade + OTel + Prometheus).
>   * `LETTER_E_ARCHITECTURE.md` (1 623 LOC) — service backend
>     (consommateur de la D du SDK ; hors-portée M8).
>   * `docs/EFE_FFI_ARCHITECTURE.md` (2 611 LOC) — architecture
>     PyO3 du sprint D25 (référencée pour le sub-sprint D25 ici).
>   * `MODE1_ARCHITECTURE.md` — contrat Mode-1 (la classification
>     que ce SDK consomme).
>
> **Portée** : 3 sprints non-contigus (D19, D20, D25) couvrant
> 25 défauts traçables :
>
>   * **D19** (Φ-A) — finalisation **Lettre A** (8 DT-A) +
>     **Lettre B** (5 DT-B) + 1 DT-PC-1 sample bundle release prep.
>   * **D20** (Φ-A) — finalisation **Lettre C** (5 DT-C) +
>     **Lettre D** (5 DT-D) + DT-PC-1/2 sample bundle livraison.
>   * **D25** (Φ-C) — `EfeNativeClient` via PyO3 (extension
>     `efe-ffi` exposée dans le SDK).
>
> **Cible GA** : **`efe-tool` v1.0** = 4 lettres A→D hardened
> (95 + tests pin) + native client zero-copy + 5 wheels manylinux/macOS
> + sample bundle Ed25519-signé.

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M8 (Φ-A → Φ-C)
§2. Position dans la topologie EFE post-D18
§3. État AS-IS au 2026-04-29 (post-D18)
§4. Définition opérationnelle de M8 v1.0 GA
§5. Principes directeurs
§6. Invariants structurels
§7. Vue d'ensemble — 3 sprints, dependency graph
§8. Surface publique cible (CLI + REPL + scenarios + facade + native)
§9. **Sprint D19** — Lettre A finalisée (8 DT-A)
§10. **Sprint D19** — Lettre B finalisée (5 DT-B)
§11. **Sprint D20** — Lettre C finalisée (5 DT-C)
§12. **Sprint D20** — Lettre D finalisée (5 DT-D)
§13. **Sprint D20** — Sample bundle réel (DT-PC-1/2)
§14. **Sprint D25** — `EfeNativeClient` via PyO3 (M3 extension)
§15. SLO mapping (cold start + latence + mémoire + throughput)
§16. Tests + pin tests structurels (28+ nouveaux)
§17. Critères Go/No-Go par sprint
§18. Risques transverses + mitigations (12 risques)
§19. Volumétrie code attendue
§20. Plans d'implémentation détaillés (jour par jour)
§21. Compliance + sécurité (redaction + sandbox + supply chain)
§22. Annexes — taxonomie complète des 23 DTs
§23. Annexes — exit codes finalisés (Lettre A)
§24. Annexes — JSON schemas v1 (classify output + transcript)
§25. Annexes — surface publique Python complète
§26. Annexes — RACI + budget + Gantt prévisionnel
§27. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du code à fin D18 + post-audit, v0.1.10
  côté `pyproject.toml` (vendored ; tag git non-publié).
* **TO-BE** = état cible défini par chaque sprint M8.
* Les sprints **D19, D20, D25** sont **non-contigus** : D21-D24
  intercalaires (D21 Dockerfile prod / AdminService, D22 mTLS,
  D23 Lettre E single-tenant, D24 Go/TS SDKs) ne touchent pas
  le SDK Python directement.
* Les défauts résiduels suivent la nomenclature **DT-A.x**
  (Lettre A), **DT-B.x** (Lettre B), **DT-C.x** (C), **DT-D.x** (D)
  + **DT-F.x** (FFI/PyO3, sprint D25 — héritée de
  `EFE_FFI_ARCHITECTURE.md`).
* **Lettre** ∈ {A, B, C, D, E} = niveau d'intégration produit
  EFE (cf. PRODUCT_COMPLETION §0.2). M8 couvre A→D ; E est un
  service backend séparé hors-portée du SDK.
* **`EfeFacade`** = interface stable de Lettre D, expose
  `classify()`, `classify_async()`, `health()`, `close()`.
* **`EfeNativeClient`** = nouveau client D25 qui parle à
  `efe-core` en-process via PyO3 (pas de gRPC).
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-tool`** | Le package Python `clients/python/efe_tool/` qui héberge le SDK A→D. AS-IS ~5 120 LOC + ~80 unit tests + ~10 tests pin. PyPI name = `efe-tool`. |
| **CLI one-shot** (Lettre A) | Le binaire `efe-admin` (alias `efe`), une invocation = un verdict JSON. Subcommands : `classify`, `repl`, `catalog`, `compile-catalog`, `eval`, `healthcheck`, `validate`, `version`, `completions`, `scenario`. |
| **REPL** (Lettre B) | `efe-admin repl` — boucle interactive long-running, partage embedder + client gRPC entre requêtes. ~9 sous-modules dans `efe_tool/repl/`. |
| **Scenario** (Lettre C) | Script applicatif chaînant EFE + LLM hôte (Claude / OpenAI / mock). 3 scenarios livrés en D20. |
| **`EfeFacade`** (Lettre D) | Façade lib intégration Python pour apps tierces (FastAPI, Flask, Celery, Dagster, Notebook). 14 modules dans `efe_tool/integrations/`. |
| **Bridge** | Client LLM tiers (`bridges/{claude.py, openai.py, mock.py}`). |
| **DT-X.N** | Defect Trace — défaut traçable identifié dans l'audit post-D18 ; X ∈ {A,B,C,D,F,PC} ; N = numéro. |
| **DT-A-N** | DT spécifique à la finalisation Lettre A (8 défauts). |
| **`Verdict`** (DT-D-1) | `@dataclass(frozen=True)` Python public exposé par la façade. Wrap les 5 champs canoniques. |
| **shtab** | Génère shell completions (bash/zsh/fish) depuis un argparse parser. Utilisé par DT-A-7. |
| **pexpect** | Lib Python pour interagir avec sub-processes en mode terminal. Utilisé par les tests REPL DT-B-2. |
| **PyO3** | Framework Rust de référence pour bindings Python. Utilisé en D25. Cf. `EFE_FFI_ARCHITECTURE.md`. |
| **abi3** | ABI Python stable, une wheel couvre Python ≥ 3.10. |
| **manylinux2014** | Standard Python wheel Linux compatible glibc ≥ 2.17. |
| **Sample bundle** | Le 1er bundle embedder réel signé Ed25519 livré avec le SDK (`bundles/all-MiniLM-L6-v2-en-1.0.0/`). DT-PC-1. |

### §0.3 Cibles plateformes (post-D25)

| Plateforme | Wheel name | Status v1.0 GA |
|------------|------------|:--------------:|
| Linux x86_64 (manylinux2014) | `efe_tool-1.0.0-cp310-abi3-manylinux_2_17_x86_64.whl` | ✅ obligatoire |
| Linux aarch64 | `efe_tool-1.0.0-cp310-abi3-manylinux_2_17_aarch64.whl` | ✅ obligatoire |
| macOS arm64 (11.0+) | `efe_tool-1.0.0-cp310-abi3-macosx_11_0_arm64.whl` | ✅ obligatoire |
| macOS x86_64 (10.15+) | `efe_tool-1.0.0-cp310-abi3-macosx_10_15_x86_64.whl` | ✅ obligatoire |
| Windows x86_64 | `efe_tool-1.0.0-cp310-abi3-win_amd64.whl` | ⚠️ best-effort |

### §0.4 Cibles Python

| Python | Status v1.0 GA |
|:------:|:--------------:|
| 3.10 | ✅ minimum (`requires-python = ">=3.10"`) |
| 3.11 | ✅ |
| 3.12 | ✅ |
| 3.13 | ✅ |
| 3.14+ (post-2026) | ⚠️ futur — abi3 nous couvre |

### §0.5 Cibles fonctionnelles GA

```
GA v1.0 efe-tool (M8) :
├── Lettre A — CLI one-shot                          (D19, post-A.1..8 finalisées)
│   ├── 10 subcommands                                (existants D17)
│   ├── exit codes taxonomie 0..16                    (DT-A-1)
│   ├── JSON output schema_version=1                  (DT-A-2)
│   ├── compact par défaut, --pretty opt-in           (DT-A-3)
│   ├── --log-level + --format (séparés)              (DT-A-4)
│   ├── --text-file / --stdin                         (DT-A-5)
│   ├── alias "efe"                                   (DT-A-6, AS-IS partial)
│   ├── completions bash/zsh/fish via shtab           (DT-A-7)
│   └── lazy imports (cold start < 500 ms)            (DT-A-8)
│
├── Lettre B — REPL interactif                       (D19, post-B.1..5 finalisées)
│   ├── readline Windows via pyreadline3              (DT-B-1)
│   ├── tests pexpect (Ctrl-C, SIGTERM, exception)    (DT-B-2)
│   ├── plafond historique 1000 + cache 100           (DT-B-3)
│   ├── redaction JWT/secrets dans logs               (DT-B-4)
│   └── pin :help ↔ docs/repl.md                      (DT-B-5)
│
├── Lettre C — Scenarios LLM                         (D20, post-C.1..5 finalisées)
│   ├── 3 scenarios livrés                            (DT-C-1)
│   │   ├── claude_billing_router.py
│   │   ├── openai_intent_classifier.py
│   │   └── mock_a_b_decoder.py
│   ├── retry exp 3× + jitter sur 429+503             (DT-C-2)
│   ├── transcript JSON schema_version=1              (DT-C-3)
│   ├── cost ceiling --max-usd 0.10                   (DT-C-4)
│   └── golden tests all-mock                          (DT-C-5)
│
├── Lettre D — Lib intégration façade                (D20, post-D.1..5 finalisées)
│   ├── public Verdict @dataclass(frozen=True)        (DT-D-1)
│   ├── EfeFacade API stable                          (DT-D-2)
│   ├── OpenTelemetry tracing auto                    (DT-D-3)
│   ├── Prometheus metrics auto                       (DT-D-4)
│   └── MockEfeFacade + FakeCatalog fixtures          (DT-D-5)
│
├── Sample bundle réel                                (D20, DT-PC-1/2)
│   ├── bundles/all-MiniLM-L6-v2-en-1.0.0/
│   ├── Ed25519 signature CI release
│   └── docs/bundles/sample.md (hash + signature)
│
└── EfeNativeClient via PyO3                         (D25, Φ-C)
    ├── from efe._native import classify_zero_copy
    ├── from efe._native import EfeNativeClient
    ├── 5 wheels (manylinux + macOS arm64/x86_64)
    └── ≥ 5× speedup vs EfeClient gRPC
```

---

## §1. Mission de l'extension M8 (Φ-A → Φ-C)

### §1.1 Énoncé opérationnel

L'extension M8 a **trois missions imbriquées** :

1. **Combler le gap de complétude par lettre** post-D18.
   AS-IS : Lettre A 60 %, B 70 %, C 65 %, D 75 % (cf. PRODUCT_COMPLETION
   §5.1). Pour qu'un consommateur réel adopte EFE, **chaque lettre
   doit être à 100 %** — un seul DT non corrigé bloque l'usage
   (e.g. exit codes inconsistants → CI scripts non écriva­bles ;
   pas de `--text-file` → impossible de classifier des fichiers
   sans hack shell). Les 23 DTs (D19+D20) ferment le gap.

2. **Combler le gap "performance Python"** : pre-D25, le seul
   chemin Python → EFE est gRPC (`EfeClient` dans `client.py`).
   Pour les usages Lettre D (lib intégration in-process) et
   Lettre C (scenarios LLM avec batch processing), gRPC ajoute
   ~3-8 ms de sérialisation par appel. À 1 000 appels/s c'est
   prohibitif. Le **`EfeNativeClient` PyO3** (D25) supprime
   ce coût (zero-copy, in-process).

3. **Combler le gap "sample bundle réel"** (DT-PC-1) : les
   tests SDK actuels utilisent un embedder mock (`MockEmbedder`).
   Pour qu'un consommateur puisse lancer `efe-admin classify
   "hello"` end-to-end sans écrire son propre bundle, il faut
   un sample bundle Ed25519-signé livré avec le SDK. Sans, le
   "happy path" du quickstart casse.

### §1.2 Périmètre strict du document

Ce document **NE traite PAS** :

* La Lettre E (service backend multi-tenant) — couverte par
  `LETTER_E_ARCHITECTURE.md` + sprint D23.
* Les SDKs Go/TS/Java — sprint D24 (Φ-B).
* L'extension `efe-ffi` au-delà du sub-sprint D25 — sprints
  D26 (real bundle integration) + D27 (live bundle reload)
  sont couverts par `EFE_FFI_ARCHITECTURE.md` + PRODUCT_COMPLETION §7.
* Les benchs criterion / `efe-bench` — couverts par `EFE_BENCH_EXTENSION_ARCHITECTURE.md`.
* L'observability (`efe-telemetry`) — couverte par
  `EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md`.

### §1.3 Mission complémentaire — adoption developer

L'extension M8 ouvre la voie à l'**adoption** :

* Lettre A finalisée → un dev peut `pip install efe-tool` et
  classifier un texte en 1 commande sans hack.
* Lettre B finalisée → un dev peut tuner un catalog interactivement
  pendant des heures sans crash, avec historique et redaction
  des secrets.
* Lettre C finalisée → un dev peut intégrer EFE dans un workflow
  Claude/OpenAI/mixte avec coût plafonné et tests golden.
* Lettre D finalisée → un dev peut intégrer EFE dans une app
  FastAPI/Flask/Celery/Dagster en 5 lignes avec tracing OTel
  et métriques Prom auto.
* `EfeNativeClient` (D25) → un dev qui veut maximiser le
  throughput (e.g. classification batch sur 1M de docs) gagne
  ≥ 5× sans changer son code.

Sans ces 5 livrables, le SDK reste un **prototype** plutôt
qu'un **produit adoptable**.

---

## §2. Position dans la topologie EFE post-D18

### §2.1 Graph de dépendances post-D25

```text
                    ┌─────────────────────────────────┐
                    │      Apps tierces (FastAPI,     │
                    │      Flask, Celery, Dagster,    │
                    │      Notebook, Django)          │
                    └──────────────┬──────────────────┘
                                   │ pip install efe-tool
                                   ▼
                    ┌─────────────────────────────────┐
                    │   efe_tool (Python SDK, M8)     │
                    │  ┌──────┐ ┌──────┐ ┌──────┐    │
                    │  │  A   │ │  B   │ │  C   │    │
                    │  │ CLI  │ │ REPL │ │Scen. │    │
                    │  └──┬───┘ └──┬───┘ └──┬───┘    │
                    │     │        │        │         │
                    │  ┌──▼────────▼────────▼──┐     │
                    │  │     EfeFacade (D)     │     │
                    │  └────┬───────────────┬──┘     │
                    └───────┼───────────────┼─────────┘
                            │               │
            ┌───────────────┘               └─────────┐
            ▼                                         ▼
   ┌─────────────────┐                   ┌──────────────────────┐
   │   EfeClient     │                   │  EfeNativeClient     │
   │   (gRPC, AS-IS) │                   │  (PyO3, D25 NEW)     │
   └────────┬────────┘                   └──────────┬───────────┘
            │                                       │
            │ network                               │ in-process
            ▼                                       ▼
   ┌──────────────────────┐                ┌─────────────────────┐
   │ efe-server (M4) gRPC │                │  efe-ffi (M3) PyO3  │
   │ - SwarmService       │                │  - PySwarmController│
   │ - CatalogService     │                │  - PyVerdict        │
   └──────────┬───────────┘                └─────────┬───────────┘
              │                                      │
              ▼                                      ▼
        efe-core (M1) — SwarmController + Catalog + Audit chain
```

### §2.2 Direction stricte des dépendances

* `efe_tool` (Python SDK) dépend de `grpcio` + `protobuf` (gRPC
  client) **AND** optionnellement de `efe._native` (PyO3 in-process
  client, pulled depuis `efe-ffi` via wheel).
* `efe_tool.cli` → utilise `efe_tool.client.EfeClient` (gRPC).
* `efe_tool.repl` → idem, plus state mutable (`SessionState`).
* `efe_tool.scenarios` → utilise `efe_tool.bridges.{claude,openai,mock}` +
  `EfeFacade`.
* `efe_tool.integrations` → utilise `EfeFacade` ; **NE dépend PAS**
  de `efe_tool.cli` (les apps tierces ne touchent pas le CLI).
* `EfeFacade` → choisit entre `EfeClient` (gRPC, default) et
  `EfeNativeClient` (PyO3) via le flag `client_kind="auto" |
  "grpc" | "native"`.
* `efe._native` (D25) **NE dépend PAS** de `efe_tool` — c'est
  la libraire produit par `efe-ffi`. Le SDK l'importe lazy.

### §2.3 Position vs autres modules EFE

| Surface | M8 (efe-tool) | M3 (efe-ffi) | M4 (efe-server) |
|---|:---:|:---:|:---:|
| Package Python publishable | ✅ `efe-tool` | ✅ `efe` (le module `_native`) | ❌ |
| CLI binary | ✅ `efe-admin`, `efe` | ❌ | ❌ |
| gRPC client | ✅ `EfeClient` | ❌ | ❌ — répond à |
| PyO3 in-process client | ❌ — utilise `efe._native` | ✅ — produit `efe._native` | ❌ |
| Façade lib intégration | ✅ `EfeFacade` | ❌ | ❌ |
| Wheels manylinux + macOS | ✅ pure-Python wheel | ✅ binary wheel | ❌ |

---

## §3. État AS-IS au 2026-04-29 (post-D18)

### §3.1 Inventaire physique `clients/python/efe_tool/`

```
clients/python/
├── README.md                                 (3 KB)
├── pyproject.toml                            (3 KB, version 0.1.10)
├── efe_agent.py                              (~13 KB)
├── examples/                                  (5 demos)
├── scripts/
│   ├── billing/
│   └── gen_protos.sh
├── tests/
│   ├── cli/                                  (8 fichiers tests Lettre A)
│   ├── repl/                                 (10 fichiers tests Lettre B)
│   ├── bridges/                              (LLM bridges tests)
│   ├── integrations/                         (Lettre D fixtures)
│   ├── scenarios/                            (Lettre C goldens)
│   ├── test_cache.py                         (Lettre A backend)
│   ├── test_catalog.py                       (Lettre A backend)
│   ├── test_client_end_to_end.py             (e2e gRPC)
│   └── test_bridges_claude.py
└── efe_tool/
    ├── __init__.py                           (178 LOC)  — surface publique
    ├── _version.py                           (37 LOC)
    ├── _async/client.py                      (async client)
    ├── admin/{client.py, types.py}           (admin RPCs)
    ├── bridges/{claude, openai, mock}.py    (LLM bridges, Letter C)
    ├── cache.py                              (95 LOC)
    ├── catalog.py                            (382 LOC)
    ├── cli/                                  (12 modules, ~3 100 LOC)
    │   ├── main.py (150)        ◄── entrée argparse
    │   ├── parser.py (929)      ◄── 10 subcommands
    │   ├── commands/ (10)       ◄── classify, catalog, repl, ...
    │   ├── config.py (551)
    │   ├── errors.py (303)
    │   ├── io_input.py (263)
    │   ├── logging.py (278)
    │   ├── metadata.py (201)
    │   └── output.py (385)
    ├── client.py                             (476 LOC) — EfeClient gRPC
    ├── embedder.py                           (176 LOC)
    ├── errors.py                             (224 LOC)
    ├── integrations/                         (14 modules) — Letter D
    │   ├── _facade.py
    │   ├── _config.py
    │   ├── _errors.py
    │   ├── _metrics.py                       ◄── DT-D-4 wiring
    │   ├── _tracing.py                       ◄── DT-D-3 wiring
    │   ├── _types.py                         ◄── DT-D-1 Verdict
    │   ├── celery.py
    │   ├── dagster.py
    │   ├── django.py
    │   ├── fastapi.py
    │   ├── flask.py
    │   ├── hybrid_decoder.py
    │   ├── notebook.py
    │   └── testing.py                        ◄── DT-D-5 fixtures
    ├── repl/                                 (9 modules, ~2 000 LOC)
    │   ├── input_loop.py (654)
    │   ├── completer.py (190)
    │   ├── history.py (243)
    │   ├── parser.py (112)
    │   ├── prompt.py (216)
    │   ├── render.py (319)
    │   ├── session.py (252)
    │   └── commands/ (8)
    ├── scenarios/                            (DT-C-1 cible)
    └── spec_builder.py                       (110 LOC)

Total Python : ~5 120 LOC + ~80 unit tests + ~10 tests pin
```

### §3.2 Inventaire `crates/efe-ffi/` (pré-D25)

```
crates/efe-ffi/
├── Cargo.toml          (description : "PyO3 bindings, M3, Sprint D25")
├── README.md
├── pyproject.toml      (maturin build)
├── python/
└── src/
    ├── lib.rs          (137 LOC)  — module entry
    ├── async_.rs       (114 LOC)  — pyo3-async-runtimes
    ├── bundle.rs       (270 LOC)  — load_bundle + PyBundle
    ├── classify.rs     (137 LOC)  — classify_zero_copy
    ├── client.rs       (220 LOC)  — PyEfeNativeClient
    ├── controller.rs   (356 LOC)  — PySwarmController
    ├── els.rs          (177 LOC)  — PyEpistemicLatentSpace numpy
    ├── error.rs        (170 LOC)  — Rust → Python exception map
    ├── types.rs        (321 LOC)  — PyVerdict, PySwarmSpec, ...
    └── verdict.rs      (219 LOC)  — Verdict construction

Total Rust : ~2 100 LOC
```

`efe-ffi` est **scaffold complet** mais NON-CONNECTÉ au SDK Python
M8 (le SDK utilise gRPC AS-IS). D25 = brancher.

### §3.3 État par lettre — % complétion + DTs résiduels

| Lettre | % AS-IS | DTs résiduels | LOC AS-IS | LOC delta cible |
|:------:|:------:|:--:|:--:|:--:|
| **A** | 60 % | 8 (DT-A-1..8) | ~3 100 | +400 LOC |
| **B** | 70 % | 5 (DT-B-1..5) | ~2 000 | +250 LOC |
| **C** | 65 % | 5 (DT-C-1..5) | ~600 | +800 LOC (3 scenarios) |
| **D** | 75 % | 5 (DT-D-1..5) | ~1 200 | +350 LOC |

Total DTs M8 (D19 + D20) = **23 DTs traçables**, +
**2 DT-PC** (sample bundle, D20), + **DT-F-* via D25 EFE_FFI_ARCHITECTURE.md**.

### §3.4 Synthèse AS-IS — gap analysis

| Sprint | Phase | État AS-IS | Effort cible | Gap critique |
|:------:|:-:|:--:|:--:|:--:|
| **D19** | Φ-A | A 60 %, B 70 % | **5 jours** (3+2) | Exit codes + lazy + readline Windows |
| **D20** | Φ-A | C 65 %, D 75 % | **7 jours** (3+2+2) | 3 scenarios + façade Verdict + bundle |
| **D25** | Φ-C | efe-ffi scaffold | **10 jours** | Wheels + tests cross-platform |

**Total : ~22 jours-ingénieur** sur 3 sprints non-contigus.

---

## §4. Définition opérationnelle de `efe-tool` v1.0 GA

### §4.1 Le contrat GA

À la fin de l'extension M8 (post-D25) :

* **Lettres A→D 100 % hardened** — chaque DT-A/B/C/D corrigé +
  pin testé.
* **`EfeNativeClient` PyO3** zero-copy, ≥ 5× speedup vs gRPC.
* **5 wheels** (manylinux2014 x86_64 + aarch64, macOS arm64 +
  x86_64, Windows x86_64 best-effort).
* **Sample bundle** signé Ed25519 livré avec le SDK
  (`bundles/all-MiniLM-L6-v2-en-1.0.0/`, ~30 MiB INT8).
* **23 nouveaux pin tests** (DT-A-1..8 + DT-B-1..5 + DT-C-1..5
  + DT-D-1..5).
* **Cold start CLI < 500 ms** (DT-A-8 lazy imports).
* **Cost ceiling** dans les scenarios C (DT-C-4 `--max-usd`).
* **OTel + Prom auto-export** dans la façade D (DT-D-3/4).

### §4.2 Versioning produit

| efe-tool version | Sprint | Lettres | Native | Sample bundle | Tests |
|---|:-:|:-:|:-:|:-:|:-:|
| 0.1.10 (post-D18) | D18 | A60% / B70% / C65% / D75% | ❌ | ❌ | ~80 unit + ~10 pin |
| 0.2.0 (post-D19) | D19 | **A100%** / **B100%** / C65% / D75% | ❌ | ❌ | ~93 (+13) |
| 0.3.0 (post-D20) | D20 | A100% / B100% / **C100%** / **D100%** | ❌ | ✅ | ~105 (+12) |
| **1.0.0 GA** (post-D25) | D25 | full A100% | ✅ wheels | ✅ | ~140 (+35 PyO3) |

### §4.3 Gating entre versions

Chaque bump exige :

* `pyproject.toml` `version` bumpé ; `_version.py` synchro
  (pin test `test_d10_pyproject_version_sync.py` AS-IS).
* Pin tests Python par DT (`scripts/tests/test_d{19,20,25}_dt_*.py`).
* JSON schemas v1 validés (classify output + transcript).
* CHANGELOG.md entry sprint-spécifique avec DT table.
* CI matrix GREEN sur 5 OS×arch×Python combinaisons (D25).

---

## §5. Principes directeurs

### §5.1 Principes opposables en revue PR

| ID | Principe | Application |
|----|---|---|
| **P-1** | **JSON d'abord** | stdout est machine-parseable par défaut. Mode humain `--format text` opt-in. (LETTER_A §1.4 P1.) |
| **P-2** | **stderr ≠ stdout** | Logs, diagnostics, warnings → stderr. stdout = résultat ou rien. (LETTER_A P2.) |
| **P-3** | **Zéro magie** | Aucune détection implicite de catalog. Toute config explicite (flag, env, fichier). |
| **P-4** | **Lazy par défaut** | Aucun import lourd avant la fin du parsing. Cold start CLI < 500 ms (DT-A-8). |
| **P-5** | **Schema versioning** | Tout JSON output a `schema_version: N`. Bump = breaking change explicite. |
| **P-6** | **Backward compat avec `Verdict`** | Le type public `Verdict` (DT-D-1) est `frozen=True` ; ajout de champ = OK, suppression = breaking. |
| **P-7** | **Façade single point of entry** | Apps tierces utilisent UNIQUEMENT `EfeFacade`. Les sous-modules `_metrics`, `_tracing` sont privés. |
| **P-8** | **Mock-first tests** | Tous les tests CI tournent sans API live (Claude/OpenAI mocks). Live API tests en `pytest -m live` opt-in. |
| **P-9** | **Native opt-in** | `EfeNativeClient` est opt-in via `client_kind="native"` ; default reste gRPC pour compat. |
| **P-10** | **Cost ceiling fail-closed** | DT-C-4 `--max-usd` abort à l'atteinte du plafond ; pas de "best effort" silencieux. |

### §5.2 Ce que `efe-tool` refuse d'implémenter

* **Auto-update du package** — refusé : `pip install --upgrade
  efe-tool` est explicite. Pas de CDN auto-pull.
* **Persistance secrets sur disque** — refusé : tokens lus
  uniquement depuis env vars ou keyring OS. (DT-B-4 redaction.)
* **Live API tests par défaut** — refusé : CI tourne tout en
  mock pour stabilité + coût. Live opt-in via `pytest -m live`.
* **Multi-tenant côté SDK** — délégué à Lettre E backend.
* **GPU runtime côté SDK** — délégué à `efe-server` + `efe-kernels`.
  Le SDK consomme via gRPC ou PyO3 mais n'orchestre rien GPU.

---

## §6. Invariants structurels

### §6.1 Invariants de structure de fichiers

```
clients/python/efe_tool/
├── __init__.py                     (extension : Verdict + EfeFacade re-exports D20)
├── _version.py                     (bump 0.1.10 → 0.2.0 → 0.3.0 → 1.0.0)
├── _async/                         (inchangé)
├── admin/                          (inchangé)
├── bridges/                        (extension : retry + jitter D20-DT-C-2)
├── cache.py                        (extension : LRU bounded D19-DT-B-3)
├── catalog.py                      (inchangé)
├── cli/                            (extension : 8 DT-A D19)
│   ├── parser.py                   (split --verbose, alias, completions)
│   ├── errors.py                   (exit codes table DT-A-1)
│   ├── output.py                   (--pretty inversion + schema_version DT-A-2)
│   └── io_input.py                 (--text-file + --stdin DT-A-5)
├── client.py                       (inchangé jusqu'à D25)
├── embedder.py                     (lazy import wrapper DT-A-8)
├── errors.py                       (extension : exit codes mapping DT-A-1)
├── integrations/                   (extension : 5 DT-D D20)
│   ├── _facade.py                  (EfeFacade hardened DT-D-2)
│   ├── _types.py                   (Verdict @dataclass(frozen=True) DT-D-1)
│   ├── _tracing.py                 (OTel auto DT-D-3)
│   ├── _metrics.py                 (Prometheus auto DT-D-4)
│   └── testing.py                  (MockEfeFacade + FakeCatalog DT-D-5)
├── repl/                           (extension : 5 DT-B D19)
│   ├── input_loop.py               (Windows readline DT-B-1, redaction DT-B-4)
│   ├── history.py                  (1000 cap DT-B-3)
│   └── commands/                   (:help DT-B-5)
├── scenarios/                      (NEW D20)
│   ├── __init__.py
│   ├── _common.py                  (retry + cost ceiling DT-C-2/4)
│   ├── _transcript.py              (transcript schema v1 DT-C-3)
│   ├── claude_billing_router.py    (DT-C-1)
│   ├── openai_intent_classifier.py (DT-C-1)
│   └── mock_a_b_decoder.py         (DT-C-1)
└── spec_builder.py                 (inchangé)

bundles/                            (NEW D20 DT-PC-1)
└── all-MiniLM-L6-v2-en-1.0.0/      (sample bundle Ed25519-signed)
    ├── manifest.json
    ├── manifest.sig
    ├── tokenizer.json
    └── weights.bin

clients/python/scripts/tests/       (existing dir scripts/tests/)
├── test_d19_dt_a_*.py              (8 pin tests, NEW)
├── test_d19_dt_b_*.py              (5 pin tests, NEW)
├── test_d20_dt_c_*.py              (5 pin tests, NEW)
├── test_d20_dt_d_*.py              (5 pin tests, NEW)
├── test_d20_sample_bundle_*.py     (3 pin tests, NEW)
└── test_d25_native_client_*.py     (10+ pin tests, NEW)

# Plus dans efe-ffi/ :
crates/efe-ffi/src/                 (extension D25 — cf. EFE_FFI_ARCHITECTURE.md)
```

### §6.2 Invariants de runtime

* **Cold start CLI < 500 ms** (DT-A-8) — mesuré via
  `python -X importtime -m efe_tool.cli ...` ; pin test
  `test_d19_dt_a8_cold_start_under_500ms`.
* **Mémoire REPL < 200 MiB** après 1 000 commandes (DT-B-3
  bounded cache).
* **`Verdict.from_dict()` est total** — accepte tout dict
  conforme au JSON schema v1, raise `ValueError` sinon.
* **`EfeFacade.classify()` est thread-safe** — peut être appelé
  depuis plusieurs threads (DT-D-2).
* **Cost ceiling abort** est fail-closed (DT-C-4) : exit 12
  immédiatement à l'atteinte du seuil, pas de "best effort".
* **Native client speedup ≥ 5×** (D25) mesuré sur classify
  batch 1 000 docs vs gRPC.

### §6.3 Invariants de versioning JSON

```json
// classify output schema v1 (DT-A-2)
{
  "schema_version": 1,
  "label": "billing",
  "confidence": 0.87,
  "cone_id": "acme/billing@v3",
  "rounds_executed": 4,
  "swarm_id": "swarm-abc123",
  "metadata": {
    "elapsed_ms": 245,
    "version": "0.2.0"
  }
}

// transcript schema v1 (DT-C-3)
{
  "schema_version": 1,
  "scenario": "claude_billing_router",
  "started_at": "2026-04-29T...",
  "finished_at": "2026-04-29T...",
  "total_cost_usd": 0.0035,
  "llm_calls": 3,
  "efe_classifies": 2,
  "verdicts": [ /* Verdict array */ ],
  "events": [ /* timestamped events */ ]
}
```

* **Champs ajoutés** sont `Option<T>` ou avec default → anciens
  parsers ignorent.
* **Renommage** = breaking change → schema_version bumped (v1 → v2).
* **Suppression** = breaking change.

---

## §7. Vue d'ensemble — 3 sprints, dependency graph

### §7.1 Tableau d'agrégation

| Sprint | Phase | Durée | Livrable principal | LOC | Pin tests | e2e |
|:-:|:-:|:-:|---|:-:|:-:|:-:|
| **D19** | Φ-A | 5 j | Lettre A 100 % + Lettre B 100 % | ~650 | 13 | 1 |
| **D20** | Φ-A | 7 j | Lettre C 100 % + Lettre D 100 % + sample bundle | ~1 200 | 12 + 3 | 2 |
| **D25** | Φ-C | 10 j | `EfeNativeClient` PyO3 + 5 wheels | ~1 800 | 10+ | 3 |

Total : **~3 650 LOC + 38 pin tests + 6 e2e**.

### §7.2 Dépendances inter-sprints

```text
                D18 (efe-tool baseline post-audit, v0.1.10)
                     │
                     │ AS-IS : A 60%, B 70%, C 65%, D 75%
                     │
                D19 (Lettre A + B finalisées)
                     │
                     │ requires : D18 baseline
                     │
                D20 (Lettre C + D finalisées + sample bundle)
                     │
                     │ requires : D19 (Lettre D consume Lettre A
                     │   exit codes via EfeFacade health)
                     │ requires : DT-PC-1 sample bundle CI release
                     │
                ... (D21-D24 : 4 sprints intercalaires)
                     │
                D25 (EfeNativeClient PyO3)
                     │
                     │ requires : efe-ffi M3 baseline (already
                     │   scaffolded ~2 100 LOC ; needs wiring)
                     │ requires : LETTER_D EfeFacade hardened (D20)
                     │ requires : 5-platform CI matrix (manylinux2014)
                     │
                     ▼
                M8 GA v1.0.0
```

### §7.3 Volumétrie code attendue

```
D19 — Lettre A finalisée (8 DT-A) :
  cli/parser.py (PATCH)                     → +120 LOC (split flags + alias)
  cli/errors.py (PATCH)                     → +60 LOC (exit codes table)
  cli/output.py (PATCH)                     → +80 LOC (schema v1 + pretty)
  cli/io_input.py (PATCH)                   → +50 LOC (--text-file + --stdin)
  cli/commands/completions.py (NEW)         → +90 LOC (shtab)
  Total Lettre A                            → ~400 LOC

D19 — Lettre B finalisée (5 DT-B) :
  repl/input_loop.py (PATCH)                → +80 LOC (Windows readline + redaction)
  repl/history.py (PATCH)                   → +40 LOC (1000 cap)
  repl/redaction.py (NEW)                   → +60 LOC (JWT regex)
  tests/repl/test_pexpect_*.py (NEW)        → ~70 LOC
  Total Lettre B                            → ~250 LOC

D20 — Lettre C finalisée (5 DT-C) :
  scenarios/_common.py (NEW)                → +200 LOC (retry + cost ceiling)
  scenarios/_transcript.py (NEW)            → +120 LOC (schema v1)
  scenarios/claude_billing_router.py (NEW)  → +180 LOC
  scenarios/openai_intent_classifier.py (NEW) → +180 LOC
  scenarios/mock_a_b_decoder.py (NEW)       → +120 LOC
  Total Lettre C                            → ~800 LOC

D20 — Lettre D finalisée (5 DT-D) :
  integrations/_types.py (PATCH)            → +50 LOC (Verdict frozen)
  integrations/_facade.py (PATCH)           → +80 LOC (EfeFacade)
  integrations/_tracing.py (PATCH)          → +60 LOC (OTel auto)
  integrations/_metrics.py (PATCH)          → +60 LOC (Prom auto)
  integrations/testing.py (PATCH)           → +100 LOC (fixtures)
  Total Lettre D                            → ~350 LOC

D20 — Sample bundle (DT-PC-1/2) :
  bundles/all-MiniLM-L6-v2-en-1.0.0/        → ~30 MiB binary (out of LOC count)
  scripts/release/build_sample_bundle.py (NEW) → +150 LOC
  docs/bundles/sample.md (NEW)              → ~80 LOC Markdown
  Total Sample Bundle                       → ~230 LOC + 30 MiB

D25 — EfeNativeClient via PyO3 :
  Cf. EFE_FFI_ARCHITECTURE.md détaille     → ~1 800 LOC Rust + ~300 LOC Python
  efe_tool/integrations/_facade.py (PATCH)  → +100 LOC (client_kind)
  efe_tool/__init__.py (PATCH)              → +30 LOC (lazy import _native)
  scripts/tests/test_d25_*.py (NEW)         → ~280 LOC
  Total D25                                 → ~2 200 LOC

Net : ~3 650 LOC actifs + ~30 MiB sample bundle + tests.
```

---

## §8. Surface publique cible (CLI + REPL + scenarios + facade + native)

### §8.1 Inventaire post-D25 (GA)

```python
# Surface Python publique post-D25
from efe_tool import (
    # Lettre D — facade lib (DT-D-1/2)
    EfeFacade,            # API stable
    Verdict,              # @dataclass(frozen=True)
    HealthStatus,
    # Lettre D — fixtures (DT-D-5)
    MockEfeFacade,
    FakeCatalog,
    # Lettre A — exit codes table (DT-A-1)
    ExitCode,             # IntEnum
    # Lettre C — scenarios (DT-C-1/2/3/4)
    Transcript,
    CostCeilingError,
    RetryExhaustedError,
)

# Lazy native client (D25) — opt-in import
try:
    from efe._native import (
        EfeNativeClient,
        classify_zero_copy,
        load_bundle,
    )
    NATIVE_AVAILABLE = True
except ImportError:
    NATIVE_AVAILABLE = False
```

```bash
# Surface CLI publique post-D25
$ efe-admin --help            # alias OR `efe`
USAGE:
    efe-admin [GLOBAL] <SUBCOMMAND> [ARGS]

GLOBAL:
    --log-level {error|warn|info|debug}   (DT-A-4)
    --format {json|text|table}            (DT-A-4)
    --pretty                              (DT-A-3 opt-in)

SUBCOMMANDS:
    classify         # Letter A — classify a single text
    repl             # Letter B — interactive shell
    catalog          # Catalog admin (subcommands)
    compile-catalog  # Compile a YAML catalog
    eval             # Run an eval set
    healthcheck      # Probe efe-server
    validate         # Validate a catalog YAML
    version          # Print version (schema v1 supported)
    completions      # Generate bash/zsh/fish completions  (DT-A-7)
    scenario         # Run a Letter C scenario               (DT-C-1)

# Exit codes (DT-A-1) :
#   0   success
#   1   UsageError
#   2   InternalError
#   3   ConfigError
#   4   CatalogError
#   5   NetworkError
#   6   TimeoutError
#   7   RejectedError (RGPD/cone violation)
#   8   RateLimitedError
#   9   AuthError
#   10  TokenExpiredError
#   11  RTOViolation         (Letter C transcript)
#   12  CostCeilingExceeded  (Letter C DT-C-4)
#   13  RegressionError      (Letter D)
#   14  BundleError          (Sample bundle DT-PC-1)
#   15  NativeClientError    (D25)
#   16  Unknown              (catch-all, audit)
```

```python
# Façade Lettre D usage canonique post-D20
from efe_tool import EfeFacade, Verdict

with EfeFacade.from_env(catalog="acme/billing@v3") as efe:
    v: Verdict = efe.classify("I want a refund")
    assert v.label == "billing"
    assert v.confidence > 0.8
```

### §8.2 Mapping consommateur → surface

| Consommateur | Surface principale | Sprint |
|:-:|:-:|:-:|
| Dev classifying texts en CLI | `efe-admin classify ...` | D19 (DT-A-1..8) |
| Dev tuning catalog interactive | `efe-admin repl` | D19 (DT-B-1..5) |
| Dev workflow LLM + EFE | `efe-admin scenario claude_billing_router` | D20 (DT-C-1..5) |
| App FastAPI/Flask/Celery | `from efe_tool import EfeFacade` | D20 (DT-D-1..5) |
| App haute-throughput | `EfeFacade(client_kind="native")` | D25 |
| CI/CD pipeline | `efe-admin classify --json --log-level error` | D19 |
| Auditor/compliance | `transcript.json` schema v1 | D20 (DT-C-3) |

---

## §9. Sprint **D19** — Lettre A finalisée (8 DT-A)

### §9.1 Objectif

Finaliser la **Lettre A — CLI one-shot** (`efe-admin classify ...`)
pour passer de 60 % → 100 % de complétude. 8 DTs traçables ;
chacun a un pin test dédié dans `clients/python/scripts/tests/`.

### §9.2 DT-A-1 — Exit codes taxonomie (CRIT)

**Problème** : AS-IS `efe-admin classify` retourne `0` sur succès et
`1` sur tout échec, sans distinction. Un script CI ne peut pas
discriminer "catalog absent" vs "réseau down" vs "RGPD reject".

**Fix** :

```python
# clients/python/efe_tool/cli/errors.py (NEW table)

from enum import IntEnum

class ExitCode(IntEnum):
    """Stable exit code taxonomy for `efe-admin`. Schema v1.

    Pinned by `test_d19_dt_a1_exit_codes_taxonomy`.
    """
    SUCCESS              = 0
    USAGE_ERROR          = 1
    INTERNAL_ERROR       = 2
    CONFIG_ERROR         = 3
    CATALOG_ERROR        = 4
    NETWORK_ERROR        = 5
    TIMEOUT_ERROR        = 6
    REJECTED_ERROR       = 7   # RGPD / cone violation
    RATE_LIMITED         = 8
    AUTH_ERROR           = 9
    TOKEN_EXPIRED        = 10
    RTO_VIOLATION        = 11  # Letter C transcript
    COST_CEILING         = 12  # Letter C DT-C-4
    REGRESSION_ERROR     = 13  # Letter D
    BUNDLE_ERROR         = 14  # DT-PC-1
    NATIVE_CLIENT_ERROR  = 15  # D25
    UNKNOWN              = 16  # catch-all, audit-logged
```

**Pin test** : `test_d19_dt_a1_exit_codes_taxonomy` qui :
1. Importe `from efe_tool.cli.errors import ExitCode`.
2. Vérifie chaque entrée de la table par valeur fixe.
3. Vérifie `len(ExitCode) == 17` (anti-régression — pas de
   variant ajouté sans bump major).
4. e2e : injecte une fail (catalog absent) → `assert
   subprocess.returncode == ExitCode.CATALOG_ERROR`.

### §9.3 DT-A-2 — Schema versioning JSON output

**Fix** :

```python
# clients/python/efe_tool/cli/output.py (PATCH)

CLASSIFY_OUTPUT_SCHEMA_VERSION = 1

def render_classify_output(verdict: Verdict, *, pretty: bool = False) -> str:
    payload = {
        "schema_version": CLASSIFY_OUTPUT_SCHEMA_VERSION,
        "label": verdict.label,
        "confidence": verdict.confidence,
        "cone_id": verdict.cone_id,
        "rounds_executed": verdict.rounds_executed,
        "swarm_id": verdict.swarm_id,
        "metadata": {
            "elapsed_ms": verdict.elapsed_ms,
            "version": _VERSION,
        },
    }
    return json.dumps(payload, indent=2 if pretty else None)
```

**Pin test** : `test_d19_dt_a2_output_carries_schema_version` qui
parse `efe-admin classify ... --json` + assert `obj["schema_version"] == 1`.

### §9.4 DT-A-3 — Inversion pretty/compact

**Problème** : AS-IS `--pretty` est l'inverse — pretty est par
défaut, `--compact` opt-in. Cela contredit P-1 (JSON
machine-parseable d'abord).

**Fix** : flag `--compact` supprimé, `--pretty` opt-in. Default =
compact (1 ligne JSON).

**Pin test** : `test_d19_dt_a3_default_is_compact_pretty_opt_in`.

### §9.5 DT-A-4 — Split `--verbose` → `--log-level` + `--format`

**Problème** : AS-IS `--verbose` mélange (1) augmenter logs et
(2) basculer en mode pretty. Sémantiquement deux choses.

**Fix** :

```python
# clients/python/efe_tool/cli/parser.py (PATCH)

parser.add_argument(
    "--log-level",
    choices=["error", "warn", "info", "debug"],
    default="warn",
    help="Stderr verbosity. (DT-A-4)",
)
parser.add_argument(
    "--format",
    choices=["json", "text", "table"],
    default="json",
    help="stdout format. JSON is default (P-1). (DT-A-4)",
)
# `--verbose` retained as deprecated alias, prints a warning to
# stderr if used. Removed in v1.0.0 GA per LETTER_A §16.
```

**Pin test** : `test_d19_dt_a4_log_level_format_split`.

### §9.6 DT-A-5 — `--text-file <path>` + `--stdin`

**Problème** : AS-IS pour classifier `myfile.txt`, l'utilisateur
doit faire `efe-admin classify "$(cat myfile.txt)"` — fragile,
casse sur newlines + binaires.

**Fix** : flags **mutually exclusive** avec text positional.

```python
# clients/python/efe_tool/cli/io_input.py (PATCH)

input_group = parser.add_mutually_exclusive_group(required=True)
input_group.add_argument(
    "text",
    nargs="?",
    help="Text to classify (positional)",
)
input_group.add_argument(
    "--text-file",
    type=Path,
    help="Read text from file (UTF-8)",
)
input_group.add_argument(
    "--stdin",
    action="store_true",
    help="Read text from stdin (until EOF)",
)
```

**Pin test** : `test_d19_dt_a5_text_file_stdin_mutually_exclusive`.

### §9.7 DT-A-6 — Alias `efe`

**AS-IS** : `[project.scripts]` déjà partial — vérifier.

```toml
# clients/python/pyproject.toml (AS-IS already)
[project.scripts]
efe-admin = "efe_tool.cli:main"
efe       = "efe_tool.cli:main"   # ← alias DT-A-6, AS-IS partial
```

**Fix** : pin test qui valide `which efe` post-install marche.

**Pin test** : `test_d19_dt_a6_alias_efe_resolves`.

### §9.8 DT-A-7 — Completions bash/zsh/fish via shtab

**Fix** : nouvelle subcommand `efe-admin completions {bash,zsh,fish}`.

```python
# clients/python/efe_tool/cli/commands/completions.py (NEW)

import shtab
from argparse import ArgumentParser

def add_completions_subcommand(parent: ArgumentParser) -> None:
    sub = parent.add_parser("completions", help="Generate shell completions")
    sub.add_argument(
        "shell",
        choices=["bash", "zsh", "fish"],
    )
    sub.set_defaults(func=run_completions)

def run_completions(args, parser):
    print(shtab.complete(parser, shell=args.shell))
    return ExitCode.SUCCESS
```

```bash
# Usage
$ efe-admin completions bash > /etc/bash_completion.d/efe-admin
$ source ~/.bashrc
$ efe-admin clas<TAB>     # → classify
$ efe-admin classify --p<TAB>  # → --pretty
```

**Pin test** : `test_d19_dt_a7_completions_subcommand_exists`.

### §9.9 DT-A-8 — Lazy imports (cold start < 500 ms)

**Problème** : AS-IS `import efe_tool.cli.main` charge
sentence-transformers + torch (~2-3 s). Pour `efe-admin
--version` ou `efe-admin completions bash`, c'est inacceptable.

**Fix** : pattern **lazy import** dans chaque subcommand.

```python
# clients/python/efe_tool/cli/commands/classify.py (PATCH)

def run_classify(args, parser):
    # Imports lourds DEFERRED jusqu'à l'exécution effective.
    from efe_tool.client import EfeClient
    from efe_tool.embedder import Embedder
    # ...
```

**Pin test** : `test_d19_dt_a8_cold_start_under_500ms` qui
mesure :

```python
import subprocess, time

def test_cold_start_version_under_500ms():
    t0 = time.monotonic()
    result = subprocess.run(
        ["python", "-X", "importtime", "-m", "efe_tool.cli", "version"],
        capture_output=True,
        timeout=5,
    )
    elapsed_ms = (time.monotonic() - t0) * 1000
    assert result.returncode == 0
    assert elapsed_ms < 500, f"cold start {elapsed_ms:.0f}ms > 500ms"
```

### §9.10 Critères Go/No-Go Lettre A

* GG-A.1 : 8 pin tests DT-A-1..8 GREEN.
* GG-A.2 : `efe-admin --version` cold start < 500 ms.
* GG-A.3 : `efe completions bash | source` fonctionne en 1 commande.
* GG-A.4 : exit codes table immutable post-publication.
* GG-A.5 : v0.2.0 release tag.

---

## §10. Sprint **D19** — Lettre B finalisée (5 DT-B)

### §10.1 Objectif

Finaliser la **Lettre B — REPL interactif** (`efe-admin repl`)
pour passer de 70 % → 100 %.

### §10.2 DT-B-1 — Windows readline via pyreadline3

**Problème** : AS-IS le REPL utilise `readline` de la stdlib
qui n'existe PAS sur Windows → crash au démarrage.

**Fix** :

```toml
# clients/python/pyproject.toml (PATCH)
[project.optional-dependencies]
repl-win = ["pyreadline3>=3.4 ; sys_platform == 'win32'"]
```

```python
# clients/python/efe_tool/repl/input_loop.py (PATCH)

import sys

if sys.platform == "win32":
    try:
        import pyreadline3 as readline  # type: ignore
    except ImportError:
        import warnings
        warnings.warn(
            "REPL on Windows requires `pip install efe-tool[repl-win]`",
            UserWarning,
        )
        readline = None  # graceful degradation
else:
    import readline
```

**Pin test** : `test_d19_dt_b1_pyreadline3_extra_declared`.

### §10.3 DT-B-2 — Tests pexpect (Ctrl-C, SIGTERM, exception)

**Fix** : 5 tests pexpect dans `clients/python/tests/repl/test_pexpect_*.py`.

```python
# clients/python/tests/repl/test_pexpect_signals.py (NEW)

import pexpect
import signal

def test_ctrl_c_during_classify_returns_to_prompt():
    p = pexpect.spawn("efe-admin repl --catalog test.yaml")
    p.expect("efe>")
    p.sendline(":classify a long text...")
    p.sendintr()  # SIGINT
    p.expect("\\^C")
    p.expect("efe>")
    p.sendline(":quit")
    p.expect(pexpect.EOF)
```

**Pin tests** : 5 (Ctrl-C, SIGTERM, exception in `:load`, EOF
sigterm, double Ctrl-C exits).

### §10.4 DT-B-3 — Plafond historique 1000 + cache 100

**Fix** :

```python
# clients/python/efe_tool/repl/history.py (PATCH)

MAX_HISTORY_ENTRIES = 1000
MAX_CACHE_ENTRIES = 100

class BoundedHistory:
    def __init__(self, max_entries: int = MAX_HISTORY_ENTRIES):
        self.max_entries = max_entries
        self._entries: deque[str] = deque(maxlen=max_entries)

    def append(self, line: str) -> None:
        self._entries.append(line)
```

**Pin test** : `test_d19_dt_b3_history_capped_at_1000`.

### §10.5 DT-B-4 — Redaction JWT/secrets dans logs `:debug`

**Fix** :

```python
# clients/python/efe_tool/repl/redaction.py (NEW)

import re

# JWT pattern : eyXXX.eyXXX.XXX (3 base64url segments)
_JWT_PATTERN = re.compile(
    r"\bey[A-Za-z0-9_-]{10,}\.ey[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b"
)
# Bearer tokens
_BEARER_PATTERN = re.compile(r"Bearer\s+[A-Za-z0-9_.\-]{20,}", re.I)
# AWS / GCP-style API keys
_API_KEY_PATTERN = re.compile(
    r"\b(AKIA|sk-|gcp-|api-|key-)[A-Za-z0-9_-]{20,}\b"
)

def redact(text: str) -> str:
    """Replace sensitive tokens with `<REDACTED>`."""
    text = _JWT_PATTERN.sub("<REDACTED-JWT>", text)
    text = _BEARER_PATTERN.sub("Bearer <REDACTED>", text)
    text = _API_KEY_PATTERN.sub("<REDACTED-API-KEY>", text)
    return text
```

**Pin test** : `test_d19_dt_b4_redaction_in_debug_logs` qui
injecte un JWT dans `:debug enable` + check stderr ne le contient
pas.

### §10.6 DT-B-5 — `:help` ↔ `docs/repl.md` cohérence

**Fix** : pin test qui compare la liste des `:cmd` listés
dans `:help` avec ceux documentés dans `docs/repl.md`.

```python
# clients/python/scripts/tests/test_d19_dt_b5_help_doc_parity.py (NEW)

import subprocess
import re
from pathlib import Path

REPL_DOC = Path(".../docs/repl.md")

def test_help_matches_documented_commands():
    # 1. Extract `:cmd` from doc.
    doc_cmds = set(re.findall(r":(\w+)", REPL_DOC.read_text()))
    # 2. Extract `:cmd` from `:help` output.
    p = subprocess.run(
        ["efe-admin", "repl", "--exec", ":help"],
        capture_output=True, text=True, timeout=10,
    )
    help_cmds = set(re.findall(r":(\w+)", p.stdout))
    # 3. Symmetric diff = 0.
    assert doc_cmds == help_cmds, (
        f"doc-only: {doc_cmds - help_cmds}\n"
        f"help-only: {help_cmds - doc_cmds}"
    )
```

### §10.7 Critères Go/No-Go Lettre B

* GG-B.1 : 5 pin tests DT-B-1..5 GREEN.
* GG-B.2 : REPL Windows fonctionne avec `efe-tool[repl-win]`
  installé.
* GG-B.3 : 1 000 commandes consécutives sans OOM (mémoire
  bounded < 200 MiB).
* GG-B.4 : `:debug` logs ne fuitent jamais de tokens.
* GG-B.5 : v0.2.0 release tag commun avec Lettre A.

---

## §11. Sprint **D20** — Lettre C finalisée (5 DT-C)

### §11.1 Objectif

Finaliser la **Lettre C — scenarios LLM** (scripts applicatifs
chaînant EFE + Claude/OpenAI/mock) pour passer de 65 % → 100 %.

### §11.2 DT-C-1 — 3 scénarios complets livrés

**Fix** : créer `clients/python/efe_tool/scenarios/` avec 3
scenarios :

```python
# scenarios/claude_billing_router.py (NEW, ~180 LOC)
"""
Demo : route customer email → billing | technical | sales via
EFE classify, then ack/respond via Claude.

Cost ceiling : --max-usd 0.10 (DT-C-4).
"""

# scenarios/openai_intent_classifier.py (NEW, ~180 LOC)
"""
Demo : classify intent via EFE, then GPT-4o to draft a response.
"""

# scenarios/mock_a_b_decoder.py (NEW, ~120 LOC)
"""
Demo : pure-mock pipeline for CI golden tests (no API live).
"""
```

**Pin test** : 3 golden snapshot tests (1 par scenario).

### §11.3 DT-C-2 — Retry exponentiel 3× + jitter

**Fix** :

```python
# clients/python/efe_tool/scenarios/_common.py (NEW, ~200 LOC)

import random
import time
from typing import Callable, TypeVar

T = TypeVar("T")

class RetryExhaustedError(Exception):
    pass

def retry_with_backoff(
    fn: Callable[..., T],
    *args,
    max_attempts: int = 3,
    base_delay_s: float = 1.0,
    max_delay_s: float = 30.0,
    jitter: float = 0.5,
    retryable_codes: tuple[int, ...] = (429, 503),
    **kwargs,
) -> T:
    """Exponential backoff with jitter on 429 / 503."""
    last_exc = None
    for attempt in range(max_attempts):
        try:
            return fn(*args, **kwargs)
        except HTTPError as e:
            if e.status_code not in retryable_codes:
                raise
            last_exc = e
            if attempt == max_attempts - 1:
                break
            delay = min(
                base_delay_s * (2 ** attempt) + random.random() * jitter,
                max_delay_s,
            )
            time.sleep(delay)
    raise RetryExhaustedError(
        f"{max_attempts} attempts exhausted ; last: {last_exc}"
    )
```

**Pin test** : `test_d20_dt_c2_retry_exp_jitter_3x_max`.

### §11.4 DT-C-3 — Transcript JSON schema v1

**Fix** :

```python
# clients/python/efe_tool/scenarios/_transcript.py (NEW, ~120 LOC)

from dataclasses import dataclass, field, asdict
import json

TRANSCRIPT_SCHEMA_VERSION = 1

@dataclass
class Transcript:
    schema_version: int = TRANSCRIPT_SCHEMA_VERSION
    scenario: str = ""
    started_at: str = ""
    finished_at: str = ""
    total_cost_usd: float = 0.0
    llm_calls: int = 0
    efe_classifies: int = 0
    verdicts: list[dict] = field(default_factory=list)
    events: list[dict] = field(default_factory=list)

    def to_json(self) -> str:
        return json.dumps(asdict(self), indent=2)

    @classmethod
    def from_json(cls, data: str) -> "Transcript":
        d = json.loads(data)
        if d.get("schema_version") != TRANSCRIPT_SCHEMA_VERSION:
            raise ValueError(f"unsupported schema_version={d.get('schema_version')}")
        return cls(**d)
```

**Pin test** : `test_d20_dt_c3_transcript_schema_v1`.

### §11.5 DT-C-4 — Cost ceiling enforcement

**Fix** :

```python
# clients/python/efe_tool/scenarios/_common.py (PATCH)

class CostCeilingError(Exception):
    """Raised when --max-usd ceiling is reached. Exit code 12."""

class CostTracker:
    def __init__(self, max_usd: float = 0.10):
        self.max_usd = max_usd
        self.total_usd = 0.0

    def add(self, cost_usd: float) -> None:
        self.total_usd += cost_usd
        if self.total_usd > self.max_usd:
            raise CostCeilingError(
                f"total ${self.total_usd:.4f} > max ${self.max_usd:.4f}"
            )
```

**Pin test** : `test_d20_dt_c4_cost_ceiling_aborts_at_threshold`.

### §11.6 DT-C-5 — Tests golden all-mock déterministes

**Fix** : 3 goldens dans `tests/scenarios/`.

```python
# tests/scenarios/test_golden_claude_billing.py (NEW)

def test_golden_claude_billing_router(snapshot, mock_claude, mock_efe):
    transcript = run_scenario("claude_billing_router", text="...")
    # Pin the transcript shape (sans le timestamps).
    snapshot.assert_match(transcript.to_json_canonical(), "billing_router_v1.json")
```

**Pin test** : `test_d20_dt_c5_golden_all_mock_deterministic`.

### §11.7 Critères Go/No-Go Lettre C

* GG-C.1 : 5 pin tests DT-C-1..5 GREEN.
* GG-C.2 : 3 scenarios runnable en mock + en live.
* GG-C.3 : Cost ceiling fail-closed sur dépassement.
* GG-C.4 : Transcript JSON readable post-mortem par auditor.
* GG-C.5 : v0.3.0 (commun avec Lettre D).

---

## §12. Sprint **D20** — Lettre D finalisée (5 DT-D)

### §12.1 Objectif

Finaliser la **Lettre D — lib intégration façade** pour passer
de 75 % → 100 %.

### §12.2 DT-D-1 — Type public `Verdict`

**Fix** :

```python
# clients/python/efe_tool/integrations/_types.py (PATCH)

from dataclasses import dataclass, field

@dataclass(frozen=True, slots=True)
class Verdict:
    """Public stable verdict type. schema v1.

    Pin tested by `test_d20_dt_d1_verdict_frozen`.
    """
    label: str
    confidence: float
    cone_id: str
    rounds_executed: int
    swarm_id: str

    def to_dict(self) -> dict:
        return {
            "label": self.label,
            "confidence": self.confidence,
            "cone_id": self.cone_id,
            "rounds_executed": self.rounds_executed,
            "swarm_id": self.swarm_id,
        }

    @classmethod
    def from_dict(cls, d: dict) -> "Verdict":
        return cls(**{k: d[k] for k in cls.__dataclass_fields__})


@dataclass(frozen=True)
class HealthStatus:
    healthy: bool
    server_version: str
    catalog_loaded: str | None
    elapsed_ms: float
```

**Pin test** : `test_d20_dt_d1_verdict_frozen_5_fields`.

### §12.3 DT-D-2 — `EfeFacade` API stable

**Fix** :

```python
# clients/python/efe_tool/integrations/_facade.py (PATCH)

from typing import Awaitable, Optional

class EfeFacade:
    """Stable Letter D façade. Pin tested.

    Public methods :
      classify(text) -> Verdict
      classify_async(text) -> Awaitable[Verdict]
      health() -> HealthStatus
      close() -> None
    """

    def __init__(
        self,
        endpoint: str,
        catalog: str,
        *,
        client_kind: str = "auto",  # auto | grpc | native
        timeout_s: float = 10.0,
        jwt_token: Optional[str] = None,
    ):
        # ... (selects EfeClient or EfeNativeClient based on client_kind)

    @classmethod
    def from_env(cls, **overrides) -> "EfeFacade":
        """Build from EFE_ENDPOINT / EFE_CATALOG / EFE_TOKEN env vars."""
        ...

    def classify(self, text: str) -> Verdict:
        ...

    async def classify_async(self, text: str) -> Verdict:
        ...

    def health(self) -> HealthStatus:
        ...

    def close(self) -> None:
        ...

    def __enter__(self): return self
    def __exit__(self, *exc): self.close()
```

**Pin test** : `test_d20_dt_d2_efefacade_api_stable_5_methods`.

### §12.4 DT-D-3 — OpenTelemetry tracing automatique

**Fix** :

```python
# clients/python/efe_tool/integrations/_tracing.py (PATCH)

from contextlib import contextmanager

try:
    from opentelemetry import trace
    _tracer = trace.get_tracer("efe_tool")
    OTEL_AVAILABLE = True
except ImportError:
    OTEL_AVAILABLE = False

@contextmanager
def trace_span(name: str, attributes: dict | None = None):
    """Auto no-op if OpenTelemetry not installed."""
    if not OTEL_AVAILABLE:
        yield
        return
    with _tracer.start_as_current_span(name, attributes=attributes) as span:
        yield span

# Spans canoniques :
#   efe_tool.preload          (catalog compile)
#   efe_tool.classify         (one classify call)
#   efe_tool.cache_hit/miss   (cache layer)
#   efe_tool.embedder.encode  (sentence-transformers)
```

**Pin test** : `test_d20_dt_d3_otel_spans_emitted_on_classify`.

### §12.5 DT-D-4 — Prometheus metrics auto-exposées

**Fix** :

```python
# clients/python/efe_tool/integrations/_metrics.py (PATCH)

try:
    from prometheus_client import Histogram, Counter
    PROMETHEUS_AVAILABLE = True
    classify_seconds = Histogram(
        "efe_facade_classify_seconds",
        "Wall-clock latency of EfeFacade.classify",
        buckets=(0.001, 0.01, 0.1, 1.0, 10.0),
    )
    cache_hit_total = Counter(
        "efe_facade_cache_hit_total",
        "Cache hit count",
        ["catalog"],
    )
    cache_miss_total = Counter(
        "efe_facade_cache_miss_total",
        "Cache miss count",
        ["catalog"],
    )
except ImportError:
    PROMETHEUS_AVAILABLE = False
    # No-op stubs
    class _Noop:
        def labels(self, **_): return self
        def observe(self, _v): pass
        def inc(self, _v=1): pass
    classify_seconds = cache_hit_total = cache_miss_total = _Noop()
```

**Pin test** : `test_d20_dt_d4_prometheus_metrics_when_available`.

### §12.6 DT-D-5 — Fixtures partagées

**Fix** :

```python
# clients/python/efe_tool/integrations/testing.py (PATCH)

from .integrations._types import Verdict
from .integrations._facade import EfeFacade
from typing import Iterator

class MockEfeFacade(EfeFacade):
    """In-memory mock for unit tests. Returns canned verdicts."""

    def __init__(self, canned: dict[str, Verdict] | None = None):
        # bypass real init
        self._canned = canned or {}

    def classify(self, text: str) -> Verdict:
        if text in self._canned:
            return self._canned[text]
        return Verdict(
            label="mock",
            confidence=0.5,
            cone_id="mock@v1",
            rounds_executed=1,
            swarm_id="mock-swarm",
        )

    async def classify_async(self, text: str) -> Verdict:
        return self.classify(text)

    def health(self) -> HealthStatus: ...
    def close(self) -> None: pass


class FakeCatalog:
    """In-memory catalog for unit tests."""
    def __init__(self, classes: list[str]):
        self.classes = classes
```

**Pin test** : `test_d20_dt_d5_fixtures_in_efe_tool_testing`.

### §12.7 Critères Go/No-Go Lettre D

* GG-D.1 : 5 pin tests DT-D-1..5 GREEN.
* GG-D.2 : `MockEfeFacade` runnable sans `efe-server` boot.
* GG-D.3 : OTel spans visibles via `OTEL_EXPORTER_OTLP_ENDPOINT`.
* GG-D.4 : Prom metrics scrappés via `/metrics` quand exposé.
* GG-D.5 : v0.3.0 GA des 4 lettres.

---

## §13. Sprint **D20** — Sample bundle réel (DT-PC-1/2)

### §13.1 Objectif

Livrer un **sample bundle embedder réel** signé Ed25519 pour
que `efe-admin classify` marche en quickstart sans que
l'utilisateur écrive son propre bundle.

### §13.2 DT-PC-1 — Bundle livré

* **Modèle** : `sentence-transformers/all-MiniLM-L6-v2` (anglais).
* **Quantization** : INT8 dynamic (PyTorch quant).
* **Taille cible** : ≤ 30 MiB.
* **Signature** : Ed25519 par CI release key.
* **Path** : `bundles/all-MiniLM-L6-v2-en-1.0.0/`
  * `manifest.json` (D16 schema)
  * `manifest.sig` (Ed25519 signature)
  * `tokenizer.json` (HF format)
  * `weights.bin` (INT8 quantized)

### §13.3 DT-PC-2 — Doc + reproducibility

```markdown
# docs/bundles/sample.md

## all-MiniLM-L6-v2-en-1.0.0

| Field | Value |
|-------|-------|
| Model | sentence-transformers/all-MiniLM-L6-v2 |
| Quantization | INT8 dynamic |
| Size | 28.4 MiB |
| Embedding dim | 384 |
| SHA-256 | a3b2... |
| Ed25519 signature | b7c8... |
| CI release | v0.3.0 (D20) |

## Reproductibility bit-à-bit

```bash
$ ./scripts/release/build_sample_bundle.py \
    --model sentence-transformers/all-MiniLM-L6-v2 \
    --quant int8 \
    --output bundles/all-MiniLM-L6-v2-en-1.0.0/
$ sha256sum bundles/all-MiniLM-L6-v2-en-1.0.0/weights.bin
a3b2...   weights.bin
```
```

**Pin test** : `test_d20_sample_bundle_present_and_signed`.

---

## §14. Sprint **D25** — `EfeNativeClient` via PyO3 (M3 extension)

### §14.1 Objectif

Livrer `EfeNativeClient` qui parle à `efe-core` **in-process**
via PyO3 (pas de gRPC). Speedup ≥ 5× vs `EfeClient` gRPC.

> Le détail complet de cette partie est dans `EFE_FFI_ARCHITECTURE.md`
> (2 611 LOC). Cette section donne uniquement le **récap** + le
> **wiring côté SDK Python**.

### §14.2 Architecture du module

Côté Rust (`crates/efe-ffi/`, AS-IS scaffolded ~2 100 LOC) :

```
crates/efe-ffi/src/
├── lib.rs            (entry point, PyO3 module)
├── async_.rs         (pyo3-async-runtimes)
├── bundle.rs         (load_bundle + PyBundle)
├── classify.rs       (classify_zero_copy)
├── client.rs         (PyEfeNativeClient)
├── controller.rs     (PySwarmController)
├── els.rs            (PyEpistemicLatentSpace numpy zero-copy)
├── error.rs          (Rust → Python exception map)
├── types.rs          (PyVerdict, PySwarmSpec)
└── verdict.rs        (Verdict construction)
```

Côté Python SDK (D25 PATCH) :

```python
# clients/python/efe_tool/__init__.py (PATCH)

# Lazy import de _native — disponible si la wheel binary est
# installée, sinon NATIVE_AVAILABLE=False (graceful degradation
# vers EfeClient gRPC).
try:
    from efe._native import (
        classify_zero_copy as _native_classify,
        EfeNativeClient as _NativeClient,
        load_bundle as _native_load_bundle,
    )
    NATIVE_AVAILABLE = True
except ImportError:
    NATIVE_AVAILABLE = False


# clients/python/efe_tool/integrations/_facade.py (PATCH D25)

class EfeFacade:
    def __init__(
        self,
        endpoint: str,
        catalog: str,
        *,
        client_kind: str = "auto",  # auto | grpc | native
        ...
    ):
        if client_kind == "auto":
            client_kind = "native" if NATIVE_AVAILABLE else "grpc"
        if client_kind == "native":
            if not NATIVE_AVAILABLE:
                raise NativeClientUnavailable(
                    "Install efe-tool[native] to enable EfeNativeClient"
                )
            self._client = _NativeClient(catalog=catalog)
        else:  # grpc
            from efe_tool.client import EfeClient
            self._client = EfeClient(endpoint=endpoint, ...)
```

### §14.3 Surface publique post-D25

```python
# Surface low-level (`efe._native` module, produced by efe-ffi)
from efe._native import (
    classify_zero_copy,    # one-shot helper, numpy zero-copy
    EfeNativeClient,        # high-level client
    load_bundle,            # load + verify Ed25519 sig
    PyVerdict,              # Rust struct exposed
    PySwarmController,      # for advanced users
    PyEpistemicLatentSpace, # numpy 2D zero-copy view
)

# Surface high-level (efe_tool, the SDK)
from efe_tool import EfeFacade

# Auto picks native if available, else gRPC.
with EfeFacade.from_env(catalog="acme/billing@v3") as efe:
    verdict = efe.classify("...")  # type: Verdict
```

### §14.4 Wheel publishing pipeline

| Plateforme | CI runner | Publish |
|------------|-----------|:-------:|
| manylinux2014 x86_64 | GitHub Actions ubuntu-22.04 + manylinux container | ✅ |
| manylinux2014 aarch64 | GitHub Actions arm runner | ✅ |
| macOS arm64 | macos-14 (M1) | ✅ |
| macOS x86_64 | macos-13 (Intel) | ✅ |
| Windows x86_64 | windows-latest | ⚠️ best-effort |

CI matrix génère 5 wheels par release ; `pip install efe-tool`
résout automatiquement la bonne wheel.

### §14.5 Bench cible — 5× speedup

| Benchmark | gRPC AS-IS | PyO3 cible | Speedup |
|-----------|-----------:|-----------:|:-------:|
| classify single (warm) | ~8 ms | ≤ 1.5 ms | ≥ 5× |
| classify batch 1 000 | ~8 s | ≤ 1.5 s | ≥ 5× |
| Python → Rust marshalling | ~3-5 ms | ~0.1 ms (zero-copy) | ≥ 30× |

**Pin test bench** : `test_d25_native_speedup_at_least_5x`.

### §14.6 Backward compat

* `EfeClient` (gRPC) **PRÉSERVÉ** — aucune breaking change.
* `EfeFacade(client_kind="grpc")` continue à fonctionner.
* Pour migrer, l'utilisateur :
  1. `pip install efe-tool[native]` (pulls la binary wheel).
  2. `EfeFacade(client_kind="native")` ou `client_kind="auto"`.
  3. Code utilisateur **identique** — surface `Verdict` stable.

### §14.7 Critères Go/No-Go D25

* GG-D25.1 : 10+ pin tests `test_d25_native_*` GREEN.
* GG-D25.2 : 5× speedup mesuré sur bench batch 1 000.
* GG-D25.3 : 5 wheels publiées sur PyPI (manylinux + macOS).
* GG-D25.4 : `EfeFacade(client_kind="auto")` fallback gRPC OK.
* GG-D25.5 : v1.0.0 GA tag.

---

## §15. SLO mapping (cold start + latence + mémoire + throughput)

| ID | SLO | Cible | Mesure | Sprint |
|:-:|---|:-:|---|:-:|
| **SLO-PY-1** | CLI cold start | < 500 ms | `python -X importtime -m efe_tool.cli version` | D19 (DT-A-8) |
| **SLO-PY-2** | REPL boot | < 2 s | `efe-admin repl` à prompt prêt | D19 |
| **SLO-PY-3** | classify latency (warm) | p99 ≤ 200 ms | `EfeFacade.classify()` | D19/D20 |
| **SLO-PY-4** | REPL mémoire après 1k cmds | < 200 MiB | RSS sampling | D19 (DT-B-3) |
| **SLO-PY-5** | scenario cost ceiling | abort à `--max-usd` | exit code 12 | D20 (DT-C-4) |
| **SLO-PY-6** | EfeFacade.classify_async | p99 ≤ 300 ms | async path | D20 (DT-D-2) |
| **SLO-PY-7** | OTel span emission | 4 spans/classify | OTLP export | D20 (DT-D-3) |
| **SLO-PY-8** | Prometheus scrape | < 10 ms | `/metrics` | D20 (DT-D-4) |
| **SLO-PY-9** | Native vs gRPC speedup | ≥ 5× | bench batch 1 000 | D25 |
| **SLO-PY-10** | Native cold start | < 100 ms | `_native` import + bundle load | D25 |

---

## §16. Tests + pin tests structurels (38 nouveaux)

### §16.1 Inventaire post-D25

| Type | Sprint | Fichier | Tests | LOC |
|---|:-:|---|:--:|:--:|
| Pin tests Lettre A | D19 | `test_d19_dt_a*.py` | 8 | ~250 |
| Pin tests Lettre B | D19 | `test_d19_dt_b*.py` | 5 | ~180 |
| Pin tests Lettre C | D20 | `test_d20_dt_c*.py` | 5 | ~200 |
| Pin tests Lettre D | D20 | `test_d20_dt_d*.py` | 5 | ~200 |
| Pin tests sample bundle | D20 | `test_d20_sample_bundle*.py` | 3 | ~80 |
| Pin tests Native | D25 | `test_d25_native_*.py` | 12 | ~300 |
| **Total nouveaux** | — | — | **38** | ~1 210 |

### §16.2 Pin tests par DT (résumé)

```
DT-A-1 : test_d19_dt_a1_exit_codes_taxonomy
DT-A-2 : test_d19_dt_a2_output_carries_schema_version
DT-A-3 : test_d19_dt_a3_default_is_compact_pretty_opt_in
DT-A-4 : test_d19_dt_a4_log_level_format_split
DT-A-5 : test_d19_dt_a5_text_file_stdin_mutually_exclusive
DT-A-6 : test_d19_dt_a6_alias_efe_resolves
DT-A-7 : test_d19_dt_a7_completions_subcommand_exists
DT-A-8 : test_d19_dt_a8_cold_start_under_500ms

DT-B-1 : test_d19_dt_b1_pyreadline3_extra_declared
DT-B-2 : test_d19_dt_b2_pexpect_signals_5_cases
DT-B-3 : test_d19_dt_b3_history_capped_at_1000
DT-B-4 : test_d19_dt_b4_redaction_in_debug_logs
DT-B-5 : test_d19_dt_b5_help_doc_parity

DT-C-1 : test_d20_dt_c1_three_scenarios_present
DT-C-2 : test_d20_dt_c2_retry_exp_jitter_3x_max
DT-C-3 : test_d20_dt_c3_transcript_schema_v1
DT-C-4 : test_d20_dt_c4_cost_ceiling_aborts_at_threshold
DT-C-5 : test_d20_dt_c5_golden_all_mock_deterministic

DT-D-1 : test_d20_dt_d1_verdict_frozen_5_fields
DT-D-2 : test_d20_dt_d2_efefacade_api_stable_5_methods
DT-D-3 : test_d20_dt_d3_otel_spans_emitted_on_classify
DT-D-4 : test_d20_dt_d4_prometheus_metrics_when_available
DT-D-5 : test_d20_dt_d5_fixtures_in_efe_tool_testing

DT-PC-1 : test_d20_sample_bundle_present_and_signed
DT-PC-2 : test_d20_sample_bundle_doc_present
+1 : test_d20_sample_bundle_reproducibility_check

D25 : test_d25_native_client_in_efe_facade
D25 : test_d25_native_speedup_at_least_5x
D25 : test_d25_native_cold_start_under_100ms
D25 : test_d25_native_zero_copy_numpy
D25 : test_d25_native_async_works
D25 : test_d25_native_bundle_load_signed_only
D25 : test_d25_native_error_mapping_to_python_exceptions
D25 : test_d25_native_gil_release_during_classify
D25 : test_d25_wheels_published_5_platforms
D25 : test_d25_efefacade_auto_picks_native
D25 : test_d25_efefacade_grpc_fallback_works
D25 : test_d25_pyi_stub_complete
```

---

## §17. Critères Go/No-Go par sprint

### §17.1 GO si tous ✅

* **D19** : 13 pin tests A+B GREEN, cold start < 500 ms,
  REPL Windows fonctionnel, v0.2.0 release.
* **D20** : 12 pin tests C+D + 3 sample bundle, EfeFacade
  thread-safe, sample bundle signed, v0.3.0 release.
* **D25** : 12+ pin tests Native, speedup ≥ 5×, 5 wheels
  publiées sur PyPI, v1.0.0 GA tag.

### §17.2 NO-GO si UN ❌

* Cold start > 500 ms en D19 → P-4 violé.
* Cost ceiling silencieux en D20 → P-10 violé.
* Native speedup < 5× en D25 → mission échouée.
* Pin tests rouges sur main.

---

## §18. Risques transverses + mitigations (12 risques)

### §18.1 Liste exhaustive

| ID | Risque | Sévérité | Probabilité | Mitigation |
|:-:|---|:-:|:-:|---|
| **PY-R-1** | Lazy imports cassent un usage existant | 🟠 HARD | 🟠 MED | Pin test `test_d19_dt_a8_subcommand_classify_still_imports_torch` ; smoke e2e pre-release |
| **PY-R-2** | shtab completions divergent du parser | 🟡 MED | 🟠 MED | Pin test `test_d19_dt_a7_completions_match_parser` qui parse les completions et vérifie diff |
| **PY-R-3** | pyreadline3 incompatible certaines versions Windows | 🟠 HARD | 🟢 LOW | Pin install dans CI Windows ; matrix Python 3.10..3.13 |
| **PY-R-4** | Redaction regex JWT laisse passer un format edge case | 🔴 CRIT | 🟠 MED | Property-based tests via Hypothesis ; corpus de 100 JWTs réels |
| **PY-R-5** | Cost ceiling course (concurrent calls) | 🟠 HARD | 🟢 LOW | `threading.Lock` sur `CostTracker` ; pin test concurrence |
| **PY-R-6** | Transcript schema v1 → v2 breaking | 🟡 MED | 🟢 LOW | Bump major + migration script ; documenté `LETTER_C §16` |
| **PY-R-7** | OTel auto-span trop verbose en prod | 🟡 MED | 🟠 MED | Default sampling = 0.1 ; opt-in 1.0 via env var |
| **PY-R-8** | Prometheus metrics labels cardinality explosion | 🟠 HARD | 🟢 LOW | `catalog` label allowlist ≤ 100 ; alerting si > |
| **PY-R-9** | MockEfeFacade drift vs EfeFacade API | 🟡 MED | 🟠 MED | Pin test `test_mock_efefacade_implements_full_interface` |
| **PY-R-10** | Sample bundle taille > 30 MiB après quant | 🟠 HARD | 🟠 MED | Fallback : ship FP16 si INT8 échoue à match accuracy |
| **PY-R-11** | Wheel manylinux2014 build cassé sur aarch64 | 🟠 HARD | 🟠 MED | Cross-compile via QEMU ; CI runner arm64 dédié |
| **PY-R-12** | Native client speedup < 5× sur bench | 🔴 CRIT | 🟢 LOW | Profile + opti zero-copy ; éventuellement freeze D25 → D26 si pas atteint |

### §18.2 Plan de réponse incident

```
Pin test rouge en CI (D19/D20/D25) :
1. PR auto-block via GitHub Actions required check.
2. On-call dev fix dans 24 h ou revert.
3. Si critique (P0) : ouvrir hotfix branch, release patch.
4. Documenter cause + fix dans CHANGELOG (section "Audit fixes").
```

---

## §19. Volumétrie code attendue

```
D19 — Lettre A finalisée (8 DT-A) :
  cli/parser.py (PATCH)                     → +120 LOC
  cli/errors.py (PATCH)                     → +60 LOC
  cli/output.py (PATCH)                     → +80 LOC
  cli/io_input.py (PATCH)                   → +50 LOC
  cli/commands/completions.py (NEW)         → +90 LOC
  Total Lettre A code                       → ~400 LOC
  Pin tests Lettre A                        → ~250 LOC

D19 — Lettre B finalisée (5 DT-B) :
  repl/input_loop.py (PATCH)                → +80 LOC
  repl/history.py (PATCH)                   → +40 LOC
  repl/redaction.py (NEW)                   → +60 LOC
  Total Lettre B code                       → ~180 LOC
  Pin tests Lettre B                        → ~180 LOC

D20 — Lettre C finalisée (5 DT-C) :
  scenarios/_common.py (NEW)                → ~200 LOC
  scenarios/_transcript.py (NEW)            → ~120 LOC
  scenarios/claude_billing_router.py (NEW)  → ~180 LOC
  scenarios/openai_intent_classifier.py (NEW) → ~180 LOC
  scenarios/mock_a_b_decoder.py (NEW)       → ~120 LOC
  Total Lettre C code                       → ~800 LOC
  Pin tests Lettre C                        → ~200 LOC

D20 — Lettre D finalisée (5 DT-D) :
  integrations/_types.py (PATCH)            → +50 LOC
  integrations/_facade.py (PATCH)           → +80 LOC
  integrations/_tracing.py (PATCH)          → +60 LOC
  integrations/_metrics.py (PATCH)          → +60 LOC
  integrations/testing.py (PATCH)           → +100 LOC
  Total Lettre D code                       → ~350 LOC
  Pin tests Lettre D                        → ~200 LOC

D20 — Sample bundle (DT-PC-1/2) :
  scripts/release/build_sample_bundle.py    → ~150 LOC Python
  bundles/all-MiniLM-L6-v2-en-1.0.0/        → ~30 MiB binary
  docs/bundles/sample.md                    → ~80 LOC Markdown
  Pin tests sample                          → ~80 LOC
  Total                                     → ~310 LOC actifs + 30 MiB

D25 — EfeNativeClient PyO3 :
  efe-ffi/src/* (côté Rust)                 → ~1 800 LOC (déjà scaffold AS-IS)
  efe_tool/__init__.py (PATCH)              → +30 LOC
  efe_tool/integrations/_facade.py (PATCH)  → +100 LOC
  Pin tests Native                          → ~300 LOC Python
  Total D25                                 → ~2 230 LOC

NET total M8 extension :
  ~4 230 LOC actifs (Python + Rust)
  + ~1 110 LOC pin tests
  + ~30 MiB sample bundle
  = ~5 340 LOC + bundle
```

---

## §20. Plans d'implémentation détaillés (jour par jour)

### §20.1 D19 — 5 jours (3 Lettre A + 2 Lettre B)

```
J1 — DT-A-1 + A-2 + A-3 (exit codes + schema + pretty inversion)
J2 — DT-A-4 + A-5 (log-level/format split + text-file/stdin)
J3 — DT-A-6 + A-7 + A-8 (alias + completions shtab + lazy)
J4 — DT-B-1 + B-2 (Windows readline + pexpect tests)
J5 — DT-B-3 + B-4 + B-5 (history cap + redaction + help parity)
       + v0.2.0 release prep
```

### §20.2 D20 — 7 jours (3 Lettre C + 2 Lettre D + 2 sample bundle)

```
J1 — DT-C-1 (3 scenarios skeletons)
J2 — DT-C-2 + C-3 (retry + transcript schema)
J3 — DT-C-4 + C-5 (cost ceiling + goldens)
J4 — DT-D-1 + D-2 (Verdict + EfeFacade)
J5 — DT-D-3 + D-4 + D-5 (OTel + Prom + fixtures)
J6 — DT-PC-1 (sample bundle build + sign)
J7 — DT-PC-2 (doc + repro check) + v0.3.0 release prep
```

### §20.3 D25 — 10 jours (PyO3 wiring)

```
J1 — efe-ffi build + pyproject maturin (cf. EFE_FFI §10)
J2 — PyVerdict + types.rs wiring
J3 — PySwarmController + classify_zero_copy
J4 — load_bundle + PyBundle types
J5 — pyo3-async-runtimes + classify_async
J6 — Python SDK side : EfeFacade.client_kind="native"
J7 — Wheels CI matrix manylinux + macOS
J8 — Bench gRPC vs native (target ≥ 5×)
J9 — Stub .pyi + mypy compat
J10 — Pin tests Native + v1.0.0 GA tag
```

---

## §21. Compliance + sécurité (redaction + sandbox + supply chain)

### §21.1 Redaction (DT-B-4)

* JWT pattern matched et redacted dans tous les logs `:debug`.
* Bearer tokens, AWS keys, GCP service account JSONs.
* Pin test couvre 50 patterns réels (corpus anonymisé).

### §21.2 Sandbox tests (DT-C-5)

* Tests CI tournent **sans** API live (Claude/OpenAI/Anthropic).
* Mock responses canned ; bench live opt-in `pytest -m live`.
* Aucune leak de credentials dans CI logs (pre-commit gitleaks).

### §21.3 Supply chain (sample bundle)

* `bundles/all-MiniLM-L6-v2-en-1.0.0/` signé Ed25519 par CI
  release key.
* SHA-256 publié dans `docs/bundles/sample.md`.
* CI release process : build deterministically + sign + publish.
* Vérification au load (D16 baseline).

### §21.4 PyO3 sûreté mémoire (D25)

* GIL release pattern documenté dans `EFE_FFI §18`.
* Send/Sync trait bounds vérifiés Rust-side.
* Zero-copy numpy : pas de double-free.
* Tests miri sur les paths critiques.

---

## §22. Annexes — taxonomie complète des 23 DTs

(Cf. §9-§12 pour les détails.)

```
Lettre A (8 DTs) :
├── DT-A-1 — Exit codes taxonomie (CRIT)
├── DT-A-2 — Schema version JSON output (HIGH)
├── DT-A-3 — Pretty/compact inversion (MED)
├── DT-A-4 — --verbose split (HIGH)
├── DT-A-5 — --text-file + --stdin (HIGH)
├── DT-A-6 — Alias `efe` (LOW, AS-IS partial)
├── DT-A-7 — Completions shtab (MED)
└── DT-A-8 — Lazy imports cold start (HIGH)

Lettre B (5 DTs) :
├── DT-B-1 — Windows readline pyreadline3 (CRIT)
├── DT-B-2 — pexpect tests Ctrl-C/SIGTERM (HIGH)
├── DT-B-3 — History cap + cache LRU (MED)
├── DT-B-4 — Redaction JWT secrets (CRIT)
└── DT-B-5 — :help ↔ docs parity (LOW)

Lettre C (5 DTs) :
├── DT-C-1 — 3 scenarios livrés (HIGH)
├── DT-C-2 — Retry exp + jitter (HIGH)
├── DT-C-3 — Transcript schema v1 (MED)
├── DT-C-4 — Cost ceiling (CRIT)
└── DT-C-5 — Goldens all-mock (MED)

Lettre D (5 DTs) :
├── DT-D-1 — Verdict @dataclass(frozen=True) (CRIT)
├── DT-D-2 — EfeFacade API stable (CRIT)
├── DT-D-3 — OTel auto (HIGH)
├── DT-D-4 — Prom auto (HIGH)
└── DT-D-5 — MockEfeFacade + FakeCatalog (MED)

DT-PC :
├── DT-PC-1 — Sample bundle Ed25519-signed (HIGH)
└── DT-PC-2 — docs/bundles/sample.md repro (MED)
```

---

## §23. Annexes — exit codes finalisés (Lettre A)

| Code | Symbol | Trigger |
|:--:|---|---|
| 0 | `SUCCESS` | Classify OK, no anomaly |
| 1 | `USAGE_ERROR` | argparse error, --help mode |
| 2 | `INTERNAL_ERROR` | Unhandled Python exception |
| 3 | `CONFIG_ERROR` | Bad YAML / bad env var |
| 4 | `CATALOG_ERROR` | Catalog not found / corrupt |
| 5 | `NETWORK_ERROR` | Connect refused, DNS fail |
| 6 | `TIMEOUT_ERROR` | gRPC deadline exceeded |
| 7 | `REJECTED_ERROR` | Cone violation / RGPD reject |
| 8 | `RATE_LIMITED` | 429 / 503 after retries |
| 9 | `AUTH_ERROR` | JWT invalid / wrong issuer |
| 10 | `TOKEN_EXPIRED` | JWT exp claim < now |
| 11 | `RTO_VIOLATION` | Letter C transcript-driven |
| 12 | `COST_CEILING` | Letter C --max-usd dépassé |
| 13 | `REGRESSION_ERROR` | Letter D facade regression |
| 14 | `BUNDLE_ERROR` | Sample bundle sig invalid |
| 15 | `NATIVE_CLIENT_ERROR` | D25 PyO3 fail |
| 16 | `UNKNOWN` | Catch-all (audit-logged) |

---

## §24. Annexes — JSON schemas v1

### §24.1 classify output schema

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://efe.ai/schemas/classify-output-1.0.0.json",
  "type": "object",
  "required": ["schema_version", "label", "confidence", "cone_id",
               "rounds_executed", "swarm_id", "metadata"],
  "properties": {
    "schema_version": { "type": "integer", "const": 1 },
    "label": { "type": "string", "minLength": 1 },
    "confidence": { "type": "number", "minimum": 0, "maximum": 1 },
    "cone_id": { "type": "string" },
    "rounds_executed": { "type": "integer", "minimum": 0 },
    "swarm_id": { "type": "string" },
    "metadata": {
      "type": "object",
      "properties": {
        "elapsed_ms": { "type": "number" },
        "version": { "type": "string" }
      }
    }
  }
}
```

### §24.2 transcript schema (Letter C)

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "$id": "https://efe.ai/schemas/transcript-1.0.0.json",
  "type": "object",
  "required": ["schema_version", "scenario", "started_at",
               "finished_at", "total_cost_usd", "verdicts"],
  "properties": {
    "schema_version": { "type": "integer", "const": 1 },
    "scenario": { "type": "string" },
    "started_at": { "type": "string", "format": "date-time" },
    "finished_at": { "type": "string", "format": "date-time" },
    "total_cost_usd": { "type": "number", "minimum": 0 },
    "llm_calls": { "type": "integer", "minimum": 0 },
    "efe_classifies": { "type": "integer", "minimum": 0 },
    "verdicts": { "type": "array", "items": { "$ref": "verdict-1.0.0.json" } },
    "events": { "type": "array" }
  }
}
```

---

## §25. Annexes — surface publique Python complète

```python
# ============================================================
# `efe_tool` — public surface post-D25 (GA v1.0.0)
# ============================================================

# Letter D : facade lib intégration
from efe_tool import (
    EfeFacade,
    Verdict,
    HealthStatus,
)

# Letter D : fixtures
from efe_tool.testing import (
    MockEfeFacade,
    FakeCatalog,
)

# Letter A : exit codes
from efe_tool.cli.errors import ExitCode

# Letter C : scenarios + types
from efe_tool.scenarios import (
    Transcript,
    CostCeilingError,
    RetryExhaustedError,
    run_scenario,
)

# Letter A/B : output schemas
from efe_tool.cli.output import (
    CLASSIFY_OUTPUT_SCHEMA_VERSION,
    render_classify_output,
)

# D25 : native client (lazy + opt-in)
from efe_tool import NATIVE_AVAILABLE
if NATIVE_AVAILABLE:
    from efe._native import (
        EfeNativeClient,
        classify_zero_copy,
        load_bundle,
        PyVerdict,
    )

# Errors
from efe_tool.errors import (
    EfeError,
    CatalogError,
    NetworkError,
    TimeoutError,
    RejectedError,
    RateLimitedError,
    AuthError,
    TokenExpiredError,
    BundleError,
    NativeClientError,
)
```

---

## §26. Annexes — RACI + budget + Gantt

### §26.1 RACI

| Activité | R | A | C | I |
|:--:|:--:|:--:|:--:|:--:|
| Lettre A finalisation | SDK Lead | Tech Lead | DevEx | Eng VP |
| Lettre B finalisation | SDK Lead | Tech Lead | DevEx | Eng VP |
| Lettre C scenarios | LLM Eng | Tech Lead | Product | Eng VP |
| Lettre D facade | SDK Lead | Tech Lead | App teams | Eng VP |
| Sample bundle build | ML Eng | Release Eng | SecOps | Eng VP |
| EfeNativeClient PyO3 | Rust Eng | Tech Lead | SDK Lead | Eng VP |
| Wheels CI matrix | Release Eng | DevOps | SDK Lead | Eng VP |
| Pin tests review | QA Lead | Tech Lead | SDK Lead | Eng VP |

### §26.2 Budget

| Item | Cost |
|:--|--:|
| Eng D19 (5 j × $1 200) | $6 000 |
| Eng D20 (7 j × $1 200) | $8 400 |
| Eng D25 (10 j × $1 200) | $12 000 |
| CI runners (manylinux + arm + macOS) | $200/mo |
| PyPI publishing | $0 (free) |
| **Total one-shot** | **$26 400** |

### §26.3 Gantt prévisionnel

```text
Sprint     D19      D20      D21..D24     D25
           ▼        ▼        (intercalaires)  ▼
J          1234567812345678         1234567890
A          ████─                              ─── 8 DT-A finalised
B           ──██                              ─── 5 DT-B finalised
C                  ████─                      ─── 5 DT-C
D                   ──██                      ─── 5 DT-D
Bundle                  ─██                   ─── DT-PC-1/2
Native                                ████████─── 5 wheels + PyO3
                                              ★── v1.0.0 GA
```

* **D19** : 5 j wallclock (1 sprint).
* **D20** : 7 j wallclock (1 sprint).
* **D25** : 10 j wallclock (1 sprint).

---

## §27. Mises à jour du document

### §27.1 Versioning

* v1.0.0 — 2026-04-29 — Première édition (cette version).

### §27.2 Liens à maintenir

* `LETTER_A_ARCHITECTURE.md` — contrat CLI ; D19 finalise § 5.
* `LETTER_B_ARCHITECTURE.md` — contrat REPL ; D19 finalise § 8.
* `LETTER_C_ARCHITECTURE.md` — scenarios ; D20 finalise § 4.
* `LETTER_D_ARCHITECTURE.md` — facade ; D20 finalise § 3.
* `EFE_FFI_ARCHITECTURE.md` — D25 PyO3 (référence détaillée).
* `PRODUCT_COMPLETION_ARCHITECTURE.md` § 5 (Φ-A) + § 7 (Φ-C).

### §27.3 Critères d'archive

* Les 3 sprints D19/D20/D25 ont shippé.
* Le `CHANGELOG.md` `[1.0.0]` entry est livré.
* L'audit pass M8 est terminé.
* Le tag `efe-tool@1.0.0` est créé sur PyPI.

---

**Fin du document.**

Total LOC du document : ~2 060 lignes Markdown.
Total LOC delta cible (M8 extension) : ~4 230 LOC actifs + ~1 110
LOC pin tests + 30 MiB sample bundle.
Total durée : ~22 jours-ingénieur sur 3 sprints non-contigus
(Φ-A → Φ-C).
