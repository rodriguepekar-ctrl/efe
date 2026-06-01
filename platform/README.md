# efe-platform — Bot + Swarm unified orchestration layer

> **Objet** — Package Python de niveau plateforme qui assemble le **bot LLM**
> (orchestrateur Plan / Execute / Verify avec tool-use) et **l'essaim EFE**
> (noyau Rust exposé en gRPC) en une seule surface homogène, consommable
> depuis une API HTTP, une CLI, un notebook, ou embarquée dans un hôte
> (FastAPI, Celery, Dagster).
>
> **Version du document** : 1.0 · **Date** : 2026-04-19 · **Niveau de
> précision** : fichier-par-fichier.

---

## Table des matières

1. [Positionnement](#1-positionnement)
2. [Topologie de la plateforme](#2-topologie-de-la-plateforme)
3. [Installation](#3-installation)
4. [Quickstart — 60 secondes](#4-quickstart--60-secondes)
5. [Arborescence du code](#5-arborescence-du-code)
6. [Le bot en détail](#6-le-bot-en-détail)
7. [L'essaim en détail](#7-lessaim-en-détail)
8. [Plan d'observabilité](#8-plan-dobservabilité)
9. [Budget, coût, sécurité](#9-budget-coût-sécurité)
10. [API HTTP](#10-api-http)
11. [Tests et qualité](#11-tests-et-qualité)

---

## 1. Positionnement

Le repo EFE contient déjà :

- `crates/efe-server` — serveur gRPC Rust exposant `SwarmService` (essaim)
  et `AdminService` (cônes d'alignement).
- `clients/python/efe_tool` — SDK Python de bas niveau : CLI, REPL,
  bridges LLM unitaires, intégrations framework.

`efe-platform` est **la couche au-dessus** : il ne réécrit rien, il
**compose** ces briques et fournit :

| Capacité | Fichier clé |
|---|---|
| Orchestration LLM avec tool-use typé | `src/efe_platform/bot/agent.py` |
| Boucle Plan → Execute → Verify | `src/efe_platform/bot/planner.py` |
| Client gRPC async + résilience | `src/efe_platform/swarm/client.py` |
| Construction validée de `SwarmSpec` | `src/efe_platform/swarm/spec_builder.py` |
| Gestion des cônes d'alignement | `src/efe_platform/swarm/cone.py` |
| Analyse de verdict (`CONVERGED`, `DIVERGENCE`…) | `src/efe_platform/swarm/verdict.py` |
| Décodage texte/centroid | `src/efe_platform/swarm/dissolution.py` |
| Audit JSONL immutable | `src/efe_platform/telemetry/audit.py` |
| Tracing OpenTelemetry | `src/efe_platform/telemetry/tracing.py` |
| Métriques Prometheus | `src/efe_platform/telemetry/metrics.py` |
| API HTTP FastAPI | `src/efe_platform/api/http_app.py` |

---

## 2. Topologie de la plateforme

```
 ┌───────────────────────────────────────────────────────────────────┐
 │                   efe-platform (this package)                     │
 │                                                                   │
 │   ┌────────────────┐      ┌─────────────────────┐                 │
 │   │    Bot LLM     │      │  Essaim (SwarmClient)│                 │
 │   │  Plan/Exec/Ver │◄────►│  async gRPC bridge  │                 │
 │   │  tool-use      │      │  retries + deadlines│                 │
 │   └───────┬────────┘      └──────────┬──────────┘                 │
 │           │                          │                            │
 │   ┌───────▼────────┐      ┌──────────▼──────────┐                 │
 │   │  Memory + PEV  │      │  SpecBuilder + Cone │                 │
 │   │  Budget + Audit│      │  Verdict + Decode   │                 │
 │   └───────┬────────┘      └──────────┬──────────┘                 │
 │           │                          │                            │
 │           └────── Telemetry ─────────┘                            │
 │            (tracing · metrics · audit JSONL)                      │
 │                          │                                        │
 │                   ┌──────▼──────┐                                 │
 │                   │  HTTP API   │  FastAPI + SSE + OpenAPI        │
 │                   └─────────────┘                                 │
 └───────────────────────────────────────────────────────────────────┘
            │                                     │
   ┌────────▼───────────┐               ┌─────────▼──────────┐
   │ LLM provider       │               │ efe-server (Rust)  │
   │ (Anthropic/OpenAI) │               │ SwarmService/Admin │
   └────────────────────┘               └────────────────────┘
```

---

## 3. Installation

```bash
# Depuis la racine du monorepo EFE
cd platform
pip install -e ".[llm,api,dev]"

# Ou sans extras optionnels
pip install -e .
```

L'extra `[client]` ré-exporte le package `efe-tool` quand on veut
consommer aussi la CLI/REPL/scenario layer. Les trois `[llm]`, `[api]`,
`[client]` sont additifs.

---

## 4. Quickstart — 60 secondes

```python
import asyncio
from efe_platform.bot import Bot, BotConfig
from efe_platform.swarm import SwarmClient, SwarmEndpoint

async def main() -> None:
    async with SwarmClient(SwarmEndpoint("localhost:50051")) as swarm:
        bot = Bot(BotConfig.from_env(), swarm=swarm)
        reply = await bot.answer(
            "Classe ce ticket: 'Impossible de réinitialiser mon mot de passe.'",
            tenant="acme",
        )
        print(reply.final_text)
        for step in reply.trace:
            print("-", step.kind, step.summary)

asyncio.run(main())
```

La variable `EFE_PLATFORM_LLM_PROVIDER=claude|openai|mock` choisit le
backend (par défaut `mock` pour les tests offline).

---

## 5. Arborescence du code

```
platform/
├── pyproject.toml
├── README.md
├── src/efe_platform/
│   ├── __init__.py
│   ├── version.py
│   ├── errors.py           # hiérarchie d'exceptions typées
│   ├── config.py           # settings pydantic hiérarchiques
│   ├── cli.py              # CLI `efe-platform`
│   ├── bot/
│   │   ├── __init__.py
│   │   ├── agent.py        # Bot principal (Plan/Execute/Verify)
│   │   ├── planner.py      # boucle PEV déterministe
│   │   ├── tool_registry.py# enregistrement + dispatch des outils
│   │   ├── memory.py       # mémoire de session bornée
│   │   ├── budget.py       # comptage tokens/USD + enforce
│   │   ├── policy.py       # règles de décision et guardrails
│   │   └── prompts.py      # templates système figés
│   ├── swarm/
│   │   ├── __init__.py
│   │   ├── client.py       # SwarmClient async (gRPC)
│   │   ├── spec_builder.py # construction validée de SwarmSpec
│   │   ├── cone.py         # AlignmentCone helpers
│   │   ├── dissolution.py  # décodage centroid → texte/raw
│   │   ├── verdict.py      # interprétation Verdict enum
│   │   └── protocol.py     # dataclasses wire-shape neutres
│   ├── catalog/
│   │   ├── __init__.py
│   │   ├── loader.py       # chargement YAML → modèle
│   │   └── models.py       # entités (Task, Anchor, Cone ref)
│   ├── telemetry/
│   │   ├── __init__.py
│   │   ├── tracing.py      # spans OpenTelemetry
│   │   ├── metrics.py      # compteurs/histo RED
│   │   └── audit.py        # audit JSONL immutable append-only
│   └── api/
│       ├── __init__.py
│       ├── http_app.py     # FastAPI app + lifespan
│       ├── schemas.py      # pydantic I/O models
│       └── dependencies.py # DI helpers
├── tests/
│   ├── conftest.py
│   ├── test_spec_builder.py
│   ├── test_tool_registry.py
│   ├── test_memory.py
│   ├── test_verdict.py
│   ├── test_api_http.py
│   └── test_audit.py
└── examples/
    ├── hello_swarm.py
    ├── audited_routing.py
    └── batch_bench.py
```

---

## 6. Le bot en détail

Le bot est un **orchestrateur déterministe avec tool-use**. Il ne fait
pas de free-form chain-of-thought textuelle : sa politique est un
automate `Plan → Execute → Verify` où :

1. **Plan** — le LLM reçoit la requête utilisateur, la description
   figée des outils EFE (`swarm.classify`, `swarm.describe`,
   `swarm.register_cone`), et produit un plan minimal (1..N tool-uses).
2. **Execute** — chaque tool-use est exécuté via le `ToolRegistry`, qui
   route vers un `SwarmClient` ; les résultats reviennent typés.
3. **Verify** — le LLM reçoit les résultats et produit (a) une
   paraphrase utilisateur, (b) un `VerdictLabel` machine-lisible, (c) un
   niveau de confiance.

Le tout est contrôlé par un `Budget` (tokens, USD, wall-clock) et
tracé par `AuditLog`.

---

## 7. L'essaim en détail

`SwarmClient` est un wrapper async autour des stubs gRPC. Ses
responsabilités :

- **Construire des `ExecuteRequest` valides** via `SpecBuilder`.
- **Gérer deadlines, retries idempotents** (UNAVAILABLE / DEADLINE_EXCEEDED).
- **Propager le `request_id` + tenant** dans les metadata.
- **Décoder** les `ExecuteResponse` en objets pythoniques stables
  (`SwarmOutcome`).
- **Analyser le verdict** (`CONVERGED`, `DIVERGENCE`, `MODE_COLLAPSE`,
  `CONTINUE`) via `verdict.analyze()`.
- **Décoder** le `DecodedOutput` via `dissolution.decode()`.

---

## 8. Plan d'observabilité

| Surface | Émetteur | Où atterrit |
|---|---|---|
| Traces | `telemetry.tracing` | Collector OTLP |
| Métriques | `telemetry.metrics` | Prometheus `/metrics` |
| Audit JSONL | `telemetry.audit` | Disque append-only |
| Logs structurés | `structlog` | stdout JSON |

Chaque requête produit un span racine `efe.platform.request` avec les
sous-spans `bot.plan`, `bot.execute.<tool>`, `bot.verify` — plus un
span enfant côté essaim (`swarm.execute`).

---

## 9. Budget, coût, sécurité

- **Budget** — le bot refuse tout plan qui excède un plafond en tokens
  LLM, un plafond USD et une deadline wall-clock.
- **Tenant** — propagé de bout en bout ; chaque metric/log/audit
  l'inclut en label.
- **Redaction** — les prompts utilisateur sont facultativement
  hash-redacted dans l'audit (`audit.redact_prompts=true`).

---

## 10. API HTTP

FastAPI app dans `api/http_app.py` — `efe-platform serve` expose :

- `POST /v1/answer` — bot synchrone, payload `{prompt, tenant}`.
- `POST /v1/answer/stream` — stream SSE avec événements `plan`, `tool_use`,
  `tool_result`, `final`.
- `GET /v1/swarm/healthz` — status du serveur EFE en amont.
- `GET /v1/catalog/cones` — liste des cônes enregistrés.
- `GET /metrics` — Prometheus text format.
- `GET /healthz` — liveness plateforme.

---

## 11. Tests et qualité

- `pytest -q` — ~40 tests unitaires + intégration.
- `mypy --strict src/efe_platform` — typage total.
- `ruff check src/ tests/` — lint strict (ANN, B, SIM, N, RUF).
- Les tests n'appellent **jamais** un LLM réel : le `MockProvider` est
  utilisé systématiquement. Les tests gRPC utilisent un fake in-process.

```bash
pytest -q
mypy --strict src/efe_platform
ruff check src/ tests/
```
