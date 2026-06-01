# Lettre D — Intégration dans un projet Python existant · Architecture détaillée

> **Statut** : Draft v1.0.0 — Document de référence pour l'adoption de niveau D du Mode 1 EFE.
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-19
> **Dépendances doc** : `LETTER_A_ARCHITECTURE.md`, `LETTER_B_ARCHITECTURE.md`, `LETTER_C_ARCHITECTURE.md`, `MODE1_ARCHITECTURE.md`, `M11_ARCHITECTURE.md`.
> **Portée** : l'intégration d'EFE Mode 1 **à l'intérieur** du processus d'une application Python existante (FastAPI, Django, Flask, Celery, Jupyter, Dagster, …) en tant que bibliothèque embarquée, sous la discipline des frameworks hôtes et non d'un binaire EFE autonome.

---

## Table des matières

1. Positionnement et contrats
2. État actuel (AS-IS) — inventaire de la surface réutilisable
3. Cas d'usage canoniques et anti-usages
4. Vue d'ensemble architecturale — EFE embarqué dans un process hôte
5. Surface publique de la Lettre D — la façade
6. Cycle de vie et hooks de framework
7. Thread-safety, concurrence et asyncio
8. Intégrations spécifiques par framework
9. Résolution de configuration dans un hôte
10. Taxonomie d'erreurs et mappage HTTP
11. Observabilité — logging, métriques, tracing
12. Budget de performance
13. Sécurité et gestion des secrets
14. Stratégie de tests dans un projet hôte
15. Packaging et déploiement
16. Versioning et compatibilité
17. Ergonomie développeur — intégration en 5 minutes
18. Dépendances inter-modules et inter-frameworks
19. Plan d'implémentation
20. Matrice de risques
21. Critères Go / No-Go
22. Annexes

---

## §1. Positionnement et contrats

### §1.1 Définition opérationnelle de D

La **Lettre D — Intégration dans un projet Python existant** satisfait la propriété :

> *Une application Python existante importe `efe_tool`, construit une façade EFE au démarrage du framework, l'utilise depuis ses routes / tâches / handlers / notebooks sans rien connaître de gRPC, et libère ses ressources proprement au shutdown — le tout sans nouveau processus, sans nouveau binaire, et sans que la latence d'EFE ne dégrade l'expérience utilisateur au-delà d'un budget borné.*

Cette propriété implique **quatre corollaires structurants** qui différencient D de tout ce qui précède :

- **C1 — Un seul processus, partagé.** Contrairement à A (nouveau process par run), B (REPL séparé), C (script éphémère), D fait cohabiter EFE avec le code métier dans **le même interpréteur Python**. Import-time effects, locks globaux, pool de threads : tout est mutualisé.
- **C2 — Cycle de vie piloté par l'hôte.** Boot, shutdown, hot-reload, drain, graceful restart : c'est le framework qui décide. EFE doit exposer des hooks (`preload`, `close`, `healthcheck`) mais **ne décide jamais** du moment où ils sont appelés.
- **C3 — Concurrence réelle.** L'hôte traite plusieurs requêtes en parallèle : threads (Flask/gunicorn-sync), asyncio (FastAPI/uvicorn), greenlets (gevent/eventlet), workers (Celery). EFE doit être appelable depuis chacun **sans refactor côté consommateur**.
- **C4 — L'hôte est le premier observateur.** Les logs, les métriques, les traces, les erreurs HTTP passent par le système d'observabilité de l'hôte. EFE ne crée ni son propre logger, ni son propre endpoint Prometheus dans ce contexte ; il s'insère dans celui qui existe.

### §1.2 Place dans la trajectoire A → E

| Lettre | Durée de vie | Consommateur | Concurrence | Cycle de vie piloté par |
|---|---|---|---|---|
| A | une invocation | pipeline shell | non (un seul appel) | OS (fork / exit) |
| B | session humaine | humain direct | non | REPL lui-même |
| C | un script | LLM + humain | non | script `main()` |
| **D** | **indéfinie** | **application hôte** | **oui** | **framework (FastAPI, Django, …)** |
| E | indéfinie | clients hétérogènes | oui | orchestrateur (Kubernetes) |

D est **le premier palier où EFE ne contrôle pas sa propre durée de vie**. Un incident de déploiement de l'app hôte (rollback, reload `gunicorn --reload`, scale-down Kubernetes) doit être géré élégamment par la façade.

### §1.3 Contrats externes

D expose **six surfaces contractuelles**, chacune versionnée indépendamment :

- **Surface d'import** : `from efe_tool.integrations import EfeFacade, EfeConfig, …`. Stabilité = SemVer de `efe-tool`.
- **Surface de configuration** : la dataclass `EfeConfig` (champs + defaults). Un champ renommé = version majeure.
- **Surface d'API façade** : `EfeFacade.classify()`, `.classify_async()`, `.preload()`, `.close()`, `.healthcheck()`. Idem SemVer strict.
- **Surface des types de retour** : `Verdict` (dataclass publique), pas les `ClassifyResult` internes.
- **Surface d'erreurs** : la famille `EfeFacadeError` avec mapping HTTP documenté. L'hôte peut choisir de la laisser remonter ou de la catcher.
- **Surface des hooks de framework** : les modules `efe_tool.integrations.fastapi`, `.django`, etc. Versionnés avec efe-tool ; chaque framework a son propre mineur (si on doit bumper l'API Django, pas besoin de tout le monde).

### §1.4 Principes invariants de D

- **P1 — Un seul `EfeFacade` par process.** Pas de construction par requête. L'instance est créée au boot du framework et réutilisée.
- **P2 — Preload obligatoire.** L'embedder (~1-2 s au cold-start) est chargé avant la première requête utilisateur. Un `preload()` manqué = timeout sur le premier user, inacceptable.
- **P3 — Thread-safe à la lecture, lock-free en hot-path.** `classify()` peut être appelée depuis N threads simultanément sans coordination externe.
- **P4 — Fermeture propre.** `close()` attend la fin des appels en vol, ferme le canal gRPC, libère la mémoire modèle, sans exception.
- **P5 — Erreurs typées, pas de `Exception` nu.** L'hôte reçoit des sous-classes d'`EfeFacadeError` avec `suggested_http_status` et `hint`.
- **P6 — Configuration une fois figée.** `EfeConfig` est frozen après `EfeFacade.__init__`. Un change runtime passe par `facade.reload_catalog()` explicite, pas par mutation.
- **P7 — Pas d'effet de bord à l'import.** `import efe_tool.integrations.fastapi` ne doit **jamais** ouvrir un canal gRPC, charger un modèle, lire une variable d'env. Uniquement des définitions.
- **P8 — Aucune fuite du protocole sous-jacent.** L'hôte ne voit jamais `grpc.RpcError`, `anthropic.APIError` ni `sentence_transformers.SentenceTransformer`. Toujours `EfeFacadeError` + `Verdict`.

### §1.5 Différences contractuelles avec C

| | C (script) | **D (lib)** |
|---|---|---|
| Fichier consommateur | 1 script isolé | intégré dans une app existante |
| `run_conversation_with_tool` utilisé ? | oui (bridge Claude) | parfois, mais **pas central** |
| Coût LLM par défaut | oui (chaque run) | **optionnel** (la classification EFE seule suffit souvent) |
| Cycle de vie | `main() → exit` | `startup → requests → shutdown` |
| Concurrence | séquentielle | parallèle (async/threads) |
| Logging | stderr (humain) | **logger hôte** (JSON structuré) |
| Metrics | non | **obligatoire** (Prometheus etc.) |
| Tests | `pytest` CLI | `pytest` **dans la suite de l'hôte** |

La différence la plus structurante : **D n'exige pas un LLM**. L'hôte peut consommer EFE uniquement pour la classification (sans Claude, sans scenario) et router lui-même selon le label. La Lettre D est donc le plus flexible palier d'intégration.

---

## §2. État actuel (AS-IS)

### §2.1 Ce qui existe (réutilisable)

| Module | Rôle | Utilisation en D |
|---|---|---|
| `efe_tool.EfeClient` | Client gRPC thread-safe (channel + stub) | Base de la façade |
| `efe_tool.AnchorCatalog` | Catalog compilable, cache `.compiled.json` | Chargé au boot, partagé |
| `efe_tool.LocalSTEmbedder` | Embedder avec singleton lazy + preload explicite | À preload au startup |
| `efe_tool.cli.errors` | Taxonomie `CliError` | Base des erreurs façade |
| `efe_tool.bridges.*` | Bridges LLM (C) | Optionnel en D |
| `efe_tool.scenarios` | Scénarios packagés (C) | Optionnel en D |

### §2.2 Ce qui manque — le gap D

**Aucune** des briques ci-dessus ne couvre l'intégration en hôte au niveau que D exige. Les gaps concrets :

- **G1 — Pas de façade de haut niveau.** Les consommateurs doivent manipuler `EfeClient`, `LocalSTEmbedder`, `AnchorCatalog` explicitement. Chaque projet hôte réimplémente le même boilerplate (init, catalog loading, fermeture).
- **G2 — Pas de `Verdict` public.** `ClassifyResult` a des champs internes (`text_hash`, `sub_scores`, `extra.zone`) non pertinents côté hôte.
- **G3 — Pas d'intégrations framework.** Aucun hook FastAPI, Django, Flask, Celery, Jupyter n'est fourni. Chaque équipe écrit son propre adaptateur.
- **G4 — Pas de méthode `classify_async`.** L'hôte async doit faire `asyncio.get_event_loop().run_in_executor(...)` manuellement.
- **G5 — Pas de metrics d'intégration.** Pas d'exporteurs Prometheus prêts, pas de spans OpenTelemetry pré-câblés.
- **G6 — Pas de politique HTTP error mapping.** Chaque hôte invente son propre mapping `EfeError → status`.
- **G7 — Pas de tests partagés.** Chaque hôte mocke EFE différemment ; pas de fixture canonique `pytest`.
- **G8 — Pas d'exemples de bout en bout.** Un dev qui découvre D trouve seulement `demo_agent_claude.py` (qui est C).

### §2.3 Synthèse

La Lettre D est **80 % infrastructurelle, 20 % applicative**. L'infrastructure sous-jacente (gRPC, catalog, embedder) existe et est stable. Ce qui reste à construire, c'est la **couche d'intégration** : façade, types publics, adaptateurs framework, helpers d'observabilité, fixtures de test.

---

## §3. Cas d'usage canoniques et anti-usages

### §3.1 UC-D1 — FastAPI : router un message de support

**Contexte.** Un service HTTP reçoit des messages de support sur `POST /tickets` et doit les router vers `fraud` / `bug` / `question` / `billing` pour dispatch automatique.

```python
# app/main.py
from fastapi import FastAPI, Depends
from efe_tool.integrations.fastapi import efe_lifespan, get_efe

app = FastAPI(lifespan=efe_lifespan)

@app.post("/tickets")
async def route_ticket(body: TicketIn, efe = Depends(get_efe)):
    verdict = await efe.classify_async(body.message, catalog="support_triage")
    if verdict.reject:
        return {"status": "needs_clarification", "reason": "low_confidence"}
    return {"status": "routed", "queue": verdict.label}
```

**Attentes.**
- L'embedder est preloaded pendant le startup de FastAPI (latence first-request < latence Nième-request).
- `get_efe` dépendance injectable, test-friendly.
- `classify_async` respecte la boucle event loop (pas de blocage).
- Fermeture propre à `SIGTERM` (canal gRPC close, pool thread drain).

### §3.2 UC-D2 — Django admin action : batch classification

**Contexte.** Un admin Django veut batch-classifier 200 tickets sélectionnés dans le changelist. L'action tourne de manière synchrone dans la requête HTTP (avec progress bar).

```python
# tickets/admin.py
from django.contrib import admin
from django.contrib import messages
from efe_tool.integrations.django import get_efe

@admin.action(description="Classify via EFE")
def classify_selected(modeladmin, request, queryset):
    facade = get_efe()
    for ticket in queryset:
        verdict = facade.classify(ticket.text, catalog="support_triage")
        ticket.predicted_label = verdict.label
        ticket.predicted_confidence = verdict.confidence
        ticket.save(update_fields=["predicted_label", "predicted_confidence"])
    messages.success(request, f"{queryset.count()} tickets classified.")
```

**Attentes.**
- `get_efe()` retourne le singleton chargé par `AppConfig.ready()`.
- Les appels sont **synchrones**, bloquent la requête (acceptable pour admin).
- Les erreurs EFE ne cassent **pas** l'action ; elles sont loggées et le ticket reste non classifié.

### §3.3 UC-D3 — Celery worker : classification asynchrone décorrélée

**Contexte.** Un pipeline asynchrone : l'app web enqueue un message, un worker Celery le prend, le classifie, écrit en DB, et trigger downstream.

```python
# tasks.py
from celery import shared_task
from efe_tool.integrations.celery import get_efe

@shared_task(bind=True, max_retries=3)
def classify_and_route(self, ticket_id: int) -> dict:
    ticket = Ticket.objects.get(pk=ticket_id)
    try:
        verdict = get_efe().classify(ticket.text, catalog="support_triage")
    except EfeTransientError as exc:
        raise self.retry(countdown=5, exc=exc)
    return {"label": verdict.label, "confidence": verdict.confidence}
```

**Attentes.**
- `get_efe()` initialisé dans le signal `worker_ready` (par worker, pas par task).
- Retry géré par Celery pour les erreurs transitoires ; les erreurs définitives remontent.
- Le modèle `sentence-transformers` est chargé **une fois par worker**, pas par task.

### §3.4 UC-D4 — Jupyter notebook : exploration data

**Contexte.** Un data scientist explore un dataset de messages, compare le label EFE au label humain, analyse les désaccords.

```python
import pandas as pd
from efe_tool.integrations.notebook import efe_notebook_singleton

efe = efe_notebook_singleton()

df = pd.read_csv("messages.csv")
df["predicted"] = df["text"].map(
    lambda t: efe.classify(t, catalog="support_triage").label
)
confusion = pd.crosstab(df["label_human"], df["predicted"])
```

**Attentes.**
- Un seul singleton par notebook kernel (réutilisé entre cellules).
- `atexit` pour fermer proprement.
- Idempotent : re-exécuter la cellule ne recharge pas le modèle.

### §3.5 UC-D5 — Dagster / Airflow : op dans un pipeline

**Contexte.** Un pipeline Dagster a un op qui classifie une batch de 10k messages toutes les nuits.

```python
from dagster import op, resource
from efe_tool.integrations.dagster import efe_resource

@op(required_resource_keys={"efe"})
def classify_batch(context, messages: list[str]) -> list[dict]:
    facade = context.resources.efe
    return [
        facade.classify(m, catalog="support_triage").as_dict()
        for m in messages
    ]

@job(resource_defs={"efe": efe_resource})
def nightly_classification(): classify_batch()
```

**Attentes.**
- Resource définition = une construction par exécution de job.
- Logging intégré via le logger Dagster.
- Erreurs définitives font failer l'op ; retry piloté par Dagster.

### §3.6 Anti-usages explicites

- **Construction par requête HTTP.** `EfeFacade()` dans chaque handler = recharge du modèle à chaque fois = latence inacceptable. Violation de P1.
- **Import à effet de bord.** Charger le modèle au module-load de l'app = augmente le temps de boot + casse `python -c "import myapp"` en CI.
- **Bypasser la façade.** Utiliser `EfeClient` directement depuis le code applicatif. Perd la typage `Verdict`, la taxonomie d'erreurs, les metrics.
- **Faire du LLM par défaut.** Un scénario C dans chaque route HTTP = coût explose. D expose le LLM en option, classification seule par défaut.
- **Partager un singleton entre processes via pickle/fork.** gRPC channel n'est **pas** safe à travers `fork()`. Chaque worker process doit construire sa propre façade après fork.

---

## §4. Vue d'ensemble architecturale

### §4.1 Diagramme — EFE embarqué dans un process hôte

```
┌──────────────────────────────────────────────────────────────────────┐
│                   PROCESS hôte (FastAPI, Django, …)                  │
│                                                                      │
│  ┌────────────────────────┐           ┌───────────────────────┐      │
│  │     Host framework     │           │    EfeFacade          │      │
│  │  (routes / views /     │──classify▶│    (singleton)        │      │
│  │   tasks / handlers)    │◀─Verdict──│                       │      │
│  └────────────┬───────────┘           └───┬───────────────────┘      │
│               │                           │                          │
│               │ (startup hook)            │                          │
│               ├──preload()───────────────▶│                          │
│               │                           ├─▶ LocalSTEmbedder (1.5s) │
│               │                           ├─▶ AnchorCatalog (cache)  │
│               │                           └─▶ EfeClient (gRPC stub)  │
│               │                                           │          │
│               │ (shutdown hook)                           │          │
│               └──close()──────────────────────────────────┤          │
└───────────────────────────────────────────────────────────│──────────┘
                                                            │
                                                            ▼ gRPC
                                                    ┌──────────────┐
                                                    │  efe-server  │
                                                    └──────────────┘
```

### §4.2 Boundaries

- **Boundary 1 — Host ↔ Facade.** L'hôte connaît `EfeFacade`, `Verdict`, `EfeFacadeError`. Jamais `EfeClient`, `AnchorCatalog`, `ClassifyResult`.
- **Boundary 2 — Facade ↔ EfeClient.** La façade connaît les internes de `efe_tool.client`. Mais l'hôte non.
- **Boundary 3 — Facade ↔ framework adapter.** Les adaptateurs (FastAPI, Django, …) vivent dans `efe_tool.integrations.<name>` et connaissent SEULEMENT les primitives de ce framework + `EfeFacade`. Jamais l'inverse.
- **Boundary 4 — EfeClient ↔ gRPC.** Inchangé depuis M11.
- **Boundary 5 — Host observability ↔ Facade.** La façade émet via `logging.getLogger("efe_tool.integrations")` et incrémente des `Counter`/`Histogram` *optionnels* (si `prometheus_client` est installé). Jamais de forcing.

### §4.3 Invariants architecturaux

- **I1.** Exactement UN `EfeFacade` par process Python. Toute construction supplémentaire lève `EfeFacadeInitError` avec hint.
- **I2.** Toute méthode publique de `EfeFacade` est sûre à appeler depuis N threads simultanés.
- **I3.** Un appel `facade.classify(...)` ne peut ni bloquer indéfiniment ni lever une exception non typée (sauf `EfeFacadeError` famille).
- **I4.** `facade.close()` peut être appelé plusieurs fois (idempotent). Après close, tout appel `classify` renvoie `EfeFacadeClosedError`.
- **I5.** Aucun adaptateur framework ne `import efe_tool.integrations.*` transitivement un autre framework. FastAPI n'importe pas Django.

---

## §5. Surface publique de la Lettre D

### §5.1 Les dataclasses publiques

```python
@dataclass(frozen=True)
class EfeConfig:
    endpoint: str = "127.0.0.1:50051"
    catalogs: tuple[str, ...] = ()          # noms à preloader
    catalog_search_paths: tuple[Path, ...] = ()
    embedder_model: str = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"
    request_timeout_s: float = 10.0
    classify_budget_ms: int = 5_000
    max_rounds: int = 16
    tls: bool = False
    tls_ca: Optional[str] = None
    jwt: Optional[str] = None
    tenant: Optional[str] = None
    cache_size: int = 10_000
    cache_ttl_s: float = 3600.0
    preload_classify_probe: bool = True    # au preload, faire un classify bidon pour warm
    metrics_enabled: bool = True            # si prometheus_client disponible
    tracing_enabled: bool = True            # si opentelemetry disponible
    strict_thread_local_seed: bool = False  # reproductibilité stricte par thread

    @classmethod
    def from_env(cls) -> "EfeConfig": ...
    @classmethod
    def from_dict(cls, d: dict) -> "EfeConfig": ...

@dataclass(frozen=True)
class Verdict:
    label: str
    confidence: float
    reject: bool
    rounds: int
    alignment_delta: float
    catalog: str
    elapsed_ms: int
    zone: str                               # "trusted" | "warn" | "reject"

    def as_dict(self) -> dict: ...
    @property
    def is_trusted(self) -> bool: ...
```

### §5.2 La façade

```python
class EfeFacade:
    """Singleton d'accès EFE pour une application hôte.

    Cycle de vie:
        boot  → EfeFacade(config) + facade.preload()
        work  → facade.classify(text, catalog=...)   (sync, thread-safe)
                facade.classify_async(...)            (async)
        shutdown → facade.close()
    """
    def __init__(self, config: EfeConfig) -> None: ...
    @classmethod
    def from_env(cls) -> "EfeFacade": ...

    # --- lifecycle ---
    def preload(self) -> None:
        """Bloque jusqu'à ce que l'embedder et les catalogs soient chargés."""
    def close(self, timeout_s: float = 30.0) -> None:
        """Drain in-flight calls + close gRPC channel. Idempotent."""
    def healthcheck(self) -> HealthStatus:
        """Retourne SERVING / NOT_SERVING + diagnostics."""
    def reload_catalog(self, name: str) -> None:
        """Recharge un catalog sans redémarrer la façade."""

    # --- hot-path ---
    def classify(self, text: str, *, catalog: str,
                 timeout_s: float | None = None) -> Verdict:
        """Synchronous classify. Thread-safe. Raises EfeFacadeError."""
    async def classify_async(self, text: str, *, catalog: str,
                             timeout_s: float | None = None) -> Verdict:
        """Async variant; runs the gRPC call in a thread pool."""

    # --- diagnostics ---
    @property
    def is_ready(self) -> bool: ...
    @property
    def catalogs(self) -> list[str]: ...
    def stats(self) -> FacadeStats: ...
```

### §5.3 Les erreurs publiques

```python
class EfeFacadeError(Exception):
    """Base de toutes les erreurs qu'un hôte reçoit."""
    suggested_http_status: int = 500
    hint: Optional[str] = None

class EfeFacadeConfigError(EfeFacadeError):
    suggested_http_status = 500  # dev issue

class EfeFacadeInitError(EfeFacadeError):
    suggested_http_status = 500

class EfeFacadeClosedError(EfeFacadeError):
    suggested_http_status = 503

class EfeFacadeTransientError(EfeFacadeError):
    """Retryable: network blip, server overload."""
    suggested_http_status = 503

class EfeFacadeTimeoutError(EfeFacadeTransientError):
    suggested_http_status = 504

class EfeFacadeCatalogError(EfeFacadeError):
    suggested_http_status = 500  # ops issue

class EfeFacadeUsageError(EfeFacadeError):
    """The caller passed invalid input (empty text, unknown catalog)."""
    suggested_http_status = 422

class EfeFacadeRejectedError(EfeFacadeError):
    """Only raised when `strict=True` in the call."""
    suggested_http_status = 422
```

Chaque erreur transporte `hint`, `context: dict`, et (quand applicable) `cause` — la chaîne est compatible avec `raise ... from ...`.

### §5.4 Helpers de framework

| Module | Exports |
|---|---|
| `efe_tool.integrations.fastapi` | `efe_lifespan`, `get_efe`, `EfeDep` |
| `efe_tool.integrations.django` | `EfeAppConfig`, `get_efe`, `EfeMiddleware` (opt) |
| `efe_tool.integrations.flask` | `init_efe`, `get_efe` |
| `efe_tool.integrations.celery` | `setup_efe_for_worker`, `get_efe` |
| `efe_tool.integrations.notebook` | `efe_notebook_singleton`, `reset_for_test` |
| `efe_tool.integrations.dagster` | `efe_resource` (ResourceDefinition) |
| `efe_tool.integrations.testing` | `FakeEfeFacade`, `InMemoryVerdict`, fixtures pytest |

Chaque module **n'importe pas** le framework au chargement (lazy via `__getattr__`) ; ainsi `from efe_tool.integrations.fastapi import efe_lifespan` n'impose `fastapi` que si l'utilisateur y touche.

---

## §6. Cycle de vie et hooks de framework

### §6.1 Les trois phases canoniques

```
phase 1 — BOOT
   ↓
   construct EfeFacade(config)
   ↓
   facade.preload()     ← bloquant : ~1.5 s cold, ~100 ms warm
   ↓
phase 2 — RUNTIME
   ↓
   facade.classify(...)  ← hot path, p95 < 2 s
   ↓
phase 3 — SHUTDOWN
   ↓
   facade.close(timeout_s=30)
   ↓
   process exit
```

### §6.2 Mapping par framework

| Framework | Boot hook | Shutdown hook |
|---|---|---|
| FastAPI (moderne) | `lifespan` context manager | idem |
| FastAPI (legacy) | `@app.on_event("startup")` | `@app.on_event("shutdown")` |
| Django | `AppConfig.ready()` | signal `django.core.signals.got_request_exception` + SIGTERM |
| Flask | application factory + `before_first_request` (deprecated) OU `create_app()` once | `@app.teardown_appcontext` + SIGTERM |
| Celery | signal `celery.signals.worker_ready` | signal `worker_shutdown` |
| Jupyter | module-level singleton | `atexit.register` |
| Dagster | `resource` initialization | resource `cleanup` |
| Airflow | DAG-level callable | — |

### §6.3 `preload()` — le point critique

Sans `preload()` explicite, la première requête utilisateur attend :

- Import `torch` (~80 ms)
- Import `sentence_transformers` (~200 ms)
- Load modèle depuis le cache HuggingFace (~800 ms sur M3 Pro)
- Vérification fingerprint catalog (~20 ms)
- Un éventuel classify probe (~800 ms si `preload_classify_probe=True`)

Total worst-case : **2-3 secondes**. Cela dégrade la latence de la première requête (p99 explose sur les 2 premières secondes du process).

**Règle stricte** : `preload()` est **mandatoire** dans le hook boot du framework. C'est bloquant, c'est voulu. `is_ready` doit être `True` avant que le traffic arrive.

### §6.4 `close()` — sémantique de drain

```python
def close(self, timeout_s: float = 30.0) -> None:
    """Drain in-flight classify calls, close gRPC channel, free model memory.

    1. Mark self as "closing" (new calls → EfeFacadeClosedError).
    2. Wait up to timeout_s for in-flight calls to complete.
    3. Close the gRPC channel (forces pending RPCs to cancel).
    4. Drop reference to the embedder model.
    5. Set is_ready = False.
    6. All subsequent method calls → EfeFacadeClosedError.
    """
```

**Idempotence** : la méthode peut être appelée plusieurs fois sans erreur (cas : Django `AppConfig.ready()` + signal SIGTERM).

### §6.5 `healthcheck()` — pour readinessProbe

```python
@dataclass(frozen=True)
class HealthStatus:
    status: str                  # "SERVING" | "NOT_SERVING" | "NOT_READY"
    latency_ms: int
    checks: dict[str, bool]      # {"embedder_loaded": True, "grpc_reachable": True, ...}
    reason: Optional[str] = None
```

L'hôte expose ce résultat dans son endpoint `/healthz` ou `/readyz`.

### §6.6 `reload_catalog(name)` — hot reload

Recharge un catalog depuis son fichier source sans redémarrer le processus. Utile après un edit YAML en prod (rare mais possible). Thread-safe : les appels en cours voient la version ancienne, les nouveaux appels voient la nouvelle. Implémentation via `copy-on-write` du dict de catalogs.

---

## §7. Thread-safety, concurrence et asyncio

### §7.1 Matrice des concurrences par framework

| Framework | Modèle dominant | Implication pour D |
|---|---|---|
| FastAPI + uvicorn | asyncio avec threadpool default | `classify_async` ou `classify` depuis `run_in_threadpool` |
| Flask + gunicorn (sync) | multi-thread par worker | `classify` sync, thread-safe OBLIGATOIRE |
| Flask + gunicorn (gevent) | greenlet | monkey-patch safe, `classify` sync |
| Django + gunicorn-sync | multi-thread | `classify` sync |
| Django + uvicorn ASGI | asyncio | `classify_async` |
| Celery | multi-process (prefork) OU threads (eventlet/gevent) | Singleton après fork (§6.2) |
| Jupyter | single-thread interactive | `classify` sync |

### §7.2 Garanties de `EfeFacade`

- **Toutes** les méthodes publiques sont **sûres à appeler depuis N threads** simultanés.
- La méthode `classify()` ne prend **aucun lock utilisateur** sur le hot path. Le seul verrou est **interne** à `EfeClient` : un `threading.RLock` sur la LRU cache, contention négligeable.
- Les méthodes de configuration (`reload_catalog`, `close`) prennent un `threading.RWLock` côté façade (lecture partagée côté hot path, écriture exclusive côté reload).

### §7.3 Implémentation de `classify_async`

```python
async def classify_async(self, text: str, *, catalog: str,
                         timeout_s: float | None = None) -> Verdict:
    loop = asyncio.get_running_loop()
    return await loop.run_in_executor(
        self._executor,     # ThreadPoolExecutor, taille == cpu_count * 2 par défaut
        functools.partial(self.classify, text, catalog=catalog, timeout_s=timeout_s),
    )
```

**Choix MVP** : sync gRPC sous le capot + thread pool. Raison : la stub gRPC sync existe déjà dans M11 et est éprouvée.

**Évolution V1.1** : support de la stub `grpc.aio` native — supprimerait le thread pool. Reporté par simplicité.

### §7.4 Fork / multi-process

gRPC **n'est pas sûr à travers `fork()`**. Si l'hôte utilise `multiprocessing` ou gunicorn-prefork, il FAUT :

- Construire la façade APRÈS le fork (dans `worker_ready` ou `after_fork`).
- Ne **pas** partager la façade via un `Manager` inter-process.

Chaque worker process a sa propre façade, son propre channel gRPC, son propre embedder en mémoire. Coût mémoire : ~500 MB × N workers. C'est une contrainte forte à documenter.

### §7.5 Greenlets (gevent / eventlet)

Le monkey-patching de gevent modifie les primitives de thread. `EfeClient` utilise `threading.RLock` qui est patché par gevent → compatible. gRPC avec gevent : nécessite `grpc.experimental.gevent.init_gevent()` **avant** l'import de `grpc`. À documenter dans `integrations/celery.py` (cas le plus fréquent).

---

## §8. Intégrations spécifiques par framework

### §8.1 FastAPI (§23.B pour snippet complet)

Hooks modernes via `lifespan`. Dependency injection via `Depends`. Support asyncio natif.

```python
# efe_tool/integrations/fastapi.py
from contextlib import asynccontextmanager

@asynccontextmanager
async def efe_lifespan(app):
    """Context manager compatible with FastAPI's ``lifespan=`` arg."""
    facade = EfeFacade.from_env()
    facade.preload()         # bloquant, acceptable ici
    app.state.efe = facade
    try:
        yield
    finally:
        facade.close(timeout_s=30)

def get_efe(request: Request) -> EfeFacade:
    return request.app.state.efe
```

### §8.2 Django

`AppConfig.ready()` pour le boot. Le shutdown propre en Django est plus subtil — on utilise `atexit` en complément, et on expose un `management command` `efe_shutdown` pour du déploiement ordonné.

### §8.3 Flask

Application factory pattern obligatoire (sinon fork + Flask + gRPC = désastre).

```python
def create_app(config=None):
    app = Flask(__name__)
    app.config.from_object(config)
    from efe_tool.integrations.flask import init_efe
    init_efe(app)       # ajoute app.extensions["efe"] + teardown
    return app
```

### §8.4 Celery

Worker-wide singleton via `worker_ready`.

```python
from celery.signals import worker_ready, worker_shutdown
from efe_tool.integrations.celery import setup_efe_for_worker, shutdown_efe

@worker_ready.connect
def _on_ready(**_):
    setup_efe_for_worker()
    # preload() est appelé DANS setup_efe_for_worker

@worker_shutdown.connect
def _on_shutdown(**_):
    shutdown_efe()
```

### §8.5 Jupyter / notebooks

Un singleton global protégé par `threading.Lock`. `atexit` registered pour le close au kernel shutdown.

### §8.6 Dagster

ResourceDefinition pattern, avec `init_fn` qui construit et `teardown_fn` qui ferme.

### §8.7 Airflow

Pas de singleton au sens strict ; un hook `get_efe()` qui reconstruit par DAG ou par task selon la durée. Documenté comme "non idéal" — D est mieux en Celery/FastAPI qu'en Airflow.

### §8.8 Test adapters — `efe_tool.integrations.testing`

```python
class FakeEfeFacade:
    """A drop-in for EfeFacade with scripted responses.

    Use in unit tests when the real facade would pull torch + model.
    """
    def __init__(self, responses: dict[str, list[Verdict]] | None = None): ...
    def classify(self, text, *, catalog): ...
    async def classify_async(self, text, *, catalog): ...
    def preload(self): ...
    def close(self, timeout_s=30): ...
    def healthcheck(self): ...

# pytest fixture
@pytest.fixture
def efe_facade() -> EfeFacade:
    return FakeEfeFacade()
```

Disponible pour chaque framework : `client.dependency_overrides[get_efe] = lambda: fake_facade()`.

---

## §9. Résolution de configuration dans un hôte

### §9.1 Trois couches possibles

1. **Env vars** (défaut) — `EFE_*`, lues par `EfeConfig.from_env()`.
2. **Config du framework hôte** — `settings.EFE_*` (Django), `app.config["EFE_*"]` (Flask), `BaseSettings` (FastAPI/pydantic).
3. **Secret manager** — Vault, AWS Secrets Manager, GCP Secret Manager (pour `ANTHROPIC_API_KEY`, certificats mTLS).

### §9.2 Règle de précédence

```
Explicit kwargs at EfeFacade() construction     (highest)
        ▼
Framework-specific config (settings.EFE_*)
        ▼
Env vars (EFE_*)
        ▼
Defaults in EfeConfig                           (lowest)
```

### §9.3 Intégration Django settings

```python
# settings.py
EFE = {
    "ENDPOINT": os.environ.get("EFE_ENDPOINT", "127.0.0.1:50051"),
    "CATALOGS": ["support_triage", "billing_escalation"],
    "TLS": os.environ.get("EFE_TLS", "0") == "1",
    "CLASSIFY_BUDGET_MS": 3_000,
}
```

`efe_tool.integrations.django.get_config()` reads from `settings.EFE`.

### §9.4 Intégration Pydantic (FastAPI)

```python
from pydantic_settings import BaseSettings

class AppSettings(BaseSettings):
    efe_endpoint: str = "127.0.0.1:50051"
    efe_catalogs: list[str] = []
    efe_tls: bool = False
    class Config: env_prefix = "MYAPP_"
```

### §9.5 Secret rotation

Le cas le plus subtil. `JWT` dans `EfeConfig` est read-once à la construction. Si le token tourne, il faut :

- Construire un **nouveau** `EfeFacade` avec le nouveau config.
- `close()` l'ancien.
- Swap atomique (avec RWLock côté façade).

API post-MVP : `facade.rotate_credentials(new_jwt)`. Non-MVP.

### §9.6 Validation au boot

`EfeConfig.__post_init__` valide :

- `endpoint` a la forme `host:port`.
- `classify_budget_ms > 0`.
- `request_timeout_s >= classify_budget_ms / 1000 + 0.5`.
- Si `tls=True`, `tls_ca` est soit None (utilise le CA système) soit un chemin existant.
- Les noms de catalogs ne contiennent que `[a-z0-9_-]`.

Tout échec de validation lève `EfeFacadeConfigError` avec `hint`.

---

## §10. Taxonomie d'erreurs et mappage HTTP

### §10.1 Table canonique

| Exception | HTTP status | Retryable | Cas typique |
|---|---|---|---|
| `EfeFacadeConfigError` | 500 | non | Config invalide au boot — bug dev |
| `EfeFacadeInitError` | 500 | non | `preload()` a échoué (modèle introuvable) |
| `EfeFacadeClosedError` | 503 | non (mais SIGTERM) | close() appelé puis classify() |
| `EfeFacadeCatalogError` | 500 | non | Catalog YAML corrompu en prod — ops issue |
| `EfeFacadeUsageError` | 422 | non | Texte vide, catalog inconnu |
| `EfeFacadeTransientError` | 503 | **oui** | gRPC UNAVAILABLE, serveur overloaded |
| `EfeFacadeTimeoutError` | 504 | **oui** | Swarm n'a pas convergé |
| `EfeFacadeRejectedError` | 422 | non | Seul si `strict=True` |

### §10.2 Mapping HTTP explicite

```python
def http_status_for(exc: EfeFacadeError) -> int:
    return exc.suggested_http_status
```

**Invariant** : chaque `EfeFacadeError` expose `suggested_http_status`. L'hôte peut choisir :

- **Adapter direct** : `raise HTTPException(status_code=http_status_for(e), detail=e.hint)`.
- **Adapter custom** : mapper selon sa propre taxonomie d'API (ex. tout transient → 200 avec queue fallback).

### §10.3 Middleware FastAPI inclus

```python
# efe_tool/integrations/fastapi/errors.py
from fastapi import Request
from fastapi.responses import JSONResponse

def register_efe_exception_handlers(app):
    @app.exception_handler(EfeFacadeError)
    async def _handler(request: Request, exc: EfeFacadeError):
        return JSONResponse(
            status_code=exc.suggested_http_status,
            content={"error": exc.error_code, "hint": exc.hint,
                     "detail": str(exc)},
        )
```

### §10.4 Django — middleware équivalent

Middleware class qui attrape `EfeFacadeError` et renvoie `JsonResponse` avec le status approprié.

### §10.5 Retry policy

La façade **n'effectue aucun retry automatique**. L'hôte décide :

- FastAPI + tenacity : decorator `@retry_if_transient`.
- Celery : `self.retry(countdown=5, exc=exc)`.

Raison : dans un contexte async multi-requête, un retry aveugle amplifie le load sur EFE. Le retry est *politique métier*.

---

## §11. Observabilité

### §11.1 Logging via `logging`

```python
import logging
_log = logging.getLogger("efe_tool.integrations")
```

Le logger hôte (Django, FastAPI) propage selon sa configuration. En contexte JSON logger (python-json-logger, structlog), les kwargs sont sérialisés.

### §11.2 Événements logués par la façade

- `efe_facade_init` (endpoint, model, catalogs)
- `efe_facade_preload_start`
- `efe_facade_preload_done` (elapsed_ms, classify_probe_ok)
- `efe_facade_classify_start` (catalog, text_len, text_hash)
- `efe_facade_classify_done` (label, confidence, rounds, elapsed_ms)
- `efe_facade_classify_failed` (exception_type, elapsed_ms)
- `efe_facade_reload_catalog` (name, old_fingerprint, new_fingerprint)
- `efe_facade_close` (in_flight_count, wait_ms)

Tous à niveau `INFO` sauf échecs (`WARNING`/`ERROR`).

### §11.3 Metrics Prometheus

Si `prometheus_client` est installé et `config.metrics_enabled=True`, la façade enregistre :

```python
efe_classify_total{catalog,label,outcome}          # Counter
efe_classify_latency_seconds{catalog}              # Histogram
efe_classify_errors_total{catalog,error_code}      # Counter
efe_facade_in_flight{catalog}                      # Gauge
efe_facade_preload_seconds                         # Histogram (observes once)
```

Si le package n'est pas installé, aucun code metrics n'est exécuté (pas de hard dep).

### §11.4 Tracing OpenTelemetry

Si `opentelemetry-api` est installé et `config.tracing_enabled=True`, chaque `classify` ouvre un span `efe.classify` avec attributes :

- `efe.catalog`
- `efe.text_length`
- `efe.label`
- `efe.confidence`
- `efe.rounds`
- `efe.elapsed_ms`

Le span est enfant de celui du request handler hôte (propagation via `contextvars`). Les erreurs ajoutent `span.record_exception()`.

### §11.5 Request ID propagation

L'hôte a typiquement un `X-Request-ID` ou un `correlation_id` dans ses logs. La façade l'accepte en paramètre optionnel :

```python
verdict = facade.classify(text, catalog="x", request_id="req-abc-123")
```

Ce `request_id` est propagé :

- dans le metadata gRPC (header `x-efe-request-id`)
- dans chaque ligne de log associée
- dans les span attributes

### §11.6 Contextvars — pattern recommandé

Pour éviter de passer `request_id` partout, la façade lit un contextvar optionnel :

```python
from contextvars import ContextVar
request_id_var: ContextVar[str | None] = ContextVar("efe_request_id", default=None)
```

L'hôte set ce var dans son middleware ; la façade lit implicitement.

---

## §12. Budget de performance

### §12.1 Ventilation des coûts

| Phase | Cold-start (une fois) | Per-request |
|---|---|---|
| Import torch | 80 ms | — |
| Import sentence_transformers | 200 ms | — |
| Load model (disk cache) | 800 ms | — |
| EfeClient init | 20 ms | — |
| Catalog load (cache hit) | 30 ms | — |
| **Total preload()** | **~1.1 s** | — |
| Prepare request | — | < 1 ms |
| embedder.encode() | — | 30-80 ms (CPU) |
| gRPC roundtrip | — | 10-30 ms |
| Swarm consensus | — | 300-1800 ms |
| Client-side decode | — | < 5 ms |
| **Total classify()** | — | **~350-1900 ms (p50-p95)** |

### §12.2 Cibles

- Cold-start : **< 2 s** (budget preload).
- Per-request p50 : **< 500 ms**.
- Per-request p95 : **< 2 s**.
- Per-request p99 : **< 3 s**.

### §12.3 Facteurs d'optimisation

- **Catalog preload** au boot (plus de disk read au premier call).
- **Classify probe** facultatif au preload (warm le modèle + le serveur).
- **LRU cache** dans `EfeClient` — si même texte classifié 2× dans TTL, saute le RPC.
- **Model sharing**: si 2 façades avec même embedder dans le même process (rare), partage automatique via singleton interne `LocalSTEmbedder`.

### §12.4 Piège : GIL et thread pool

Le GIL Python **ne bloque pas** les appels gRPC C-extension, mais **bloque** l'encoding CPU de l'embedder. Dans FastAPI async avec N threads, si `encode()` dure 80 ms, la concurrence réelle est au plus N × (1000/80) = 12×N req/s par embedder.

**Mitigation** : passer au gRPC async en V1.1, ou déplacer l'embedder dans un sidecar process (E).

### §12.5 Pré-chauffage du serveur

`preload()` peut optionnellement envoyer un classify bidon au serveur (`preload_classify_probe=True`). Cela préchauffe le SwarmController côté serveur et le cache OS sur le catalog compilé.

---

## §13. Sécurité et gestion des secrets

### §13.1 ANTHROPIC_API_KEY (si le LLM-bridge est utilisé dans D)

Jamais en dur dans le code. Jamais dans `EfeConfig` au sens littéral. Chargé via :

- `os.environ["ANTHROPIC_API_KEY"]`
- Secret manager injecté par le framework (Kubernetes Secret, Docker Secret, Vault)
- Django `settings.ANTHROPIC_API_KEY` (qui lit env)

### §13.2 mTLS vers `efe-server`

Si `tls=True`, `tls_ca` est un chemin vers un bundle PEM. Le chemin est dans la config, **pas** le contenu. Le fichier est monté via le framework (Docker volume, Kubernetes secret mount).

### §13.3 Redaction dans les logs

Par défaut, la façade **ne log JAMAIS** le texte classifié. Elle log :

- `text_length`
- `text_hash` (SHA-256 premier 16 hex chars)

Pour debug, flag `config.log_text_sample=True` qui log les 24 premiers caractères avec prefix `<<sample: …>>`. Jamais en prod.

### §13.4 Rate limiting upstream

D n'implémente pas de rate-limiting. C'est à l'hôte (middleware FastAPI, Django-ratelimit, nginx) de borner la charge avant d'atteindre EFE.

### §13.5 Input validation

La façade valide au-delà du texte non-vide :

- `len(text) <= 100_000` (même limite que Lettre A).
- `catalog in config.catalogs` (rejet précoce `EfeFacadeUsageError`).

Extensions optionnelles (non-MVP) :

- Détection de patterns sensibles (numéros de carte, SSN).
- Détection de langue (si le catalog est monolingue).

### §13.6 Surface d'attaque

- La façade n'expose **aucun endpoint réseau** (c'est l'hôte qui fait ça).
- Le canal gRPC est sortant, jamais entrant.
- Aucune désérialisation arbitraire (YAML via `safe_load`, JSON via stdlib).

---

## §14. Stratégie de tests

### §14.1 Matrice de tests dans un projet hôte

| Niveau | Acteur | But | Outillage D fourni |
|---|---|---|---|
| Unit | mocks pur | logique métier, sans EFE | `FakeEfeFacade` |
| Contract | mock façade | valider que handlers respectent le contrat Verdict | fixtures `pytest` |
| Intégration | EFE + serveur | tout le stack | docker-compose avec efe-server |
| E2E | app complète | user journey | selenium / playwright + smoke |

### §14.2 `FakeEfeFacade`

Drop-in pour `EfeFacade`, paramétrable par un dict `{catalog: [Verdict, Verdict, ...]}`. Pop-par-appel.

```python
fake = FakeEfeFacade(responses={
    "support_triage": [
        Verdict(label="fraud", confidence=0.85, ...),
        Verdict(label="bug", confidence=0.90, ...),
    ],
})
```

### §14.3 Fixtures pytest

```python
# efe_tool/integrations/testing/__init__.py
@pytest.fixture
def efe_facade() -> "EfeFacade":
    return FakeEfeFacade()

@pytest.fixture(autouse=True)
def _patch_get_efe(monkeypatch, efe_facade):
    """Route every get_efe() to the fake in tests."""
    monkeypatch.setattr("efe_tool.integrations.fastapi.get_efe",
                        lambda *a, **kw: efe_facade)
```

### §14.4 Intégration docker-compose

```yaml
# tests/docker-compose.yml
services:
  efe-server:
    image: efe/server:latest
    ports: ["50051:50051"]
  efe-test:
    image: python:3.12
    depends_on: [efe-server]
    command: pytest tests/integration
    environment:
      EFE_ENDPOINT: efe-server:50051
```

### §14.5 Contract test sur `Verdict`

Un test snapshot garantit que le schéma du `Verdict.as_dict()` n'a pas bougé entre releases. Toute rupture → changelog.

### §14.6 Load tests

Hors scope MVP mais recommandé post-intégration. Locust ou k6 contre le endpoint hôte qui wrappe classify.

---

## §15. Packaging et déploiement

### §15.1 Dépendance dans le projet hôte

```toml
# pyproject.toml (host project)
[project]
dependencies = [
    "efe-tool==1.2.*",     # pin strict, bump mineur controllé
    # ...autres deps hôte
]

[project.optional-dependencies]
dev = [
    "pytest",
    "pytest-asyncio",
]
```

### §15.2 Extras de `efe-tool`

L'hôte installe :

- `efe-tool` (de base)
- **Pas** les extras, sauf si besoin de C (`efe-tool[claude]`).

### §15.3 Déploiement multi-service

Pattern typique : hôte + efe-server côte à côte (sidecar Kubernetes).

```yaml
# k8s/deployment.yaml
spec:
  containers:
  - name: app
    image: my-app:1.0
    env:
    - name: EFE_ENDPOINT
      value: "127.0.0.1:50051"
  - name: efe-server
    image: efe/server:1.2.0
    ports:
    - containerPort: 50051
```

Deux containers dans le même pod. Latence réseau négligeable (unix socket ou loopback).

### §15.4 Image Docker optimisée

Multi-stage build pour l'hôte :

```dockerfile
FROM python:3.12-slim AS builder
RUN pip install -U pip
COPY pyproject.toml poetry.lock .
RUN pip install --prefix=/install .

FROM python:3.12-slim
COPY --from=builder /install /usr/local
COPY app/ /app/
# Pré-télécharger le modèle pour éviter un cold-start réseau au premier boot
RUN python -c "from sentence_transformers import SentenceTransformer; \
               SentenceTransformer('sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2')"
```

### §15.5 Shutdown propre

Kubernetes envoie SIGTERM, puis SIGKILL après `terminationGracePeriodSeconds` (30s par défaut). La façade doit close() en < 30s. On recommande `terminationGracePeriodSeconds: 45` pour laisser marge.

---

## §16. Versioning et compatibilité

### §16.1 SemVer strict de `efe-tool`

- **Majeur** : breaking change de `EfeFacade`, `Verdict`, `EfeConfig`.
- **Mineur** : ajout de champ optionnel (backward-compat), nouvelle intégration framework.
- **Patch** : bugfix, perf.

### §16.2 Garanties inter-version

L'hôte en `efe-tool 1.2.0` doit pouvoir passer à `1.2.5` sans changement de code. Passage à `1.3.0` : potentiels nouveaux champs dans `Verdict` (ignorés si ignorés).

### §16.3 Déprécations graduelles

3 versions mineures minimum entre annonce et suppression.

Exemple projeté : le champ `alignment_delta` rarement utilisé pourrait être retiré en V2. Cycle :

- V1.5 : marquage `deprecated` dans la docstring.
- V1.6 : `DeprecationWarning` runtime.
- V2.0 : retrait.

### §16.4 Compatibilité inter-frameworks

Les adaptateurs framework sont versionnés avec efe-tool mais fragmentés :

- Un bump d'API Django ne casse pas FastAPI.
- Si on arrête de supporter Django 3.x, c'est un mineur (pas un majeur efe-tool).

---

## §17. Ergonomie développeur — intégration en 5 minutes

### §17.1 Objectif

Un dev qui connaît FastAPI doit intégrer EFE en 5 minutes. Un dev Django en 10 minutes.

### §17.2 Template FastAPI (5 minutes)

```python
# app/main.py
from fastapi import FastAPI, Depends
from pydantic import BaseModel
from efe_tool.integrations.fastapi import efe_lifespan, get_efe
from efe_tool.integrations.fastapi import register_efe_exception_handlers

app = FastAPI(lifespan=efe_lifespan)
register_efe_exception_handlers(app)

class TicketIn(BaseModel):
    message: str

@app.post("/tickets")
async def route_ticket(body: TicketIn, efe = Depends(get_efe)):
    v = await efe.classify_async(body.message, catalog="support_triage")
    return {"label": v.label, "confidence": v.confidence, "reject": v.reject}
```

Configuration : env vars `EFE_ENDPOINT`, `EFE_CATALOGS=support_triage`.

Lancement : `uvicorn app.main:app`. Le preload s'exécute au startup event.

### §17.3 Template Django (10 minutes)

```python
# apps/efe_integration/apps.py
from django.apps import AppConfig
from efe_tool.integrations.django import EfeAppConfig

class EfeIntegrationConfig(EfeAppConfig):
    name = "apps.efe_integration"

# views.py
from django.http import JsonResponse
from efe_tool.integrations.django import get_efe

def classify_view(request):
    text = request.POST["message"]
    verdict = get_efe().classify(text, catalog="support_triage")
    return JsonResponse(verdict.as_dict())
```

Settings :

```python
INSTALLED_APPS = [..., "apps.efe_integration"]
EFE = {"ENDPOINT": "127.0.0.1:50051", "CATALOGS": ["support_triage"]}
```

### §17.4 Template Celery (10 minutes)

Voir §8.4.

### §17.5 Debug recipe

- Logs vides ? → `logging.basicConfig(level=logging.INFO)` ou config Django/FastAPI.
- Latence anormale ? → vérifier `facade.healthcheck().checks["embedder_loaded"]` + metric `efe_facade_preload_seconds`.
- Erreur 503 systématique ? → `efe-server` pas démarré ou `EFE_ENDPOINT` faux.

---

## §18. Dépendances

### §18.1 Hard deps (déjà dans `efe-tool`)

- `grpcio`, `grpcio-tools`, `protobuf` (M11)
- `pyyaml`, `numpy` (M1/M2)
- `sentence-transformers` (M2)

### §18.2 Soft deps (par framework)

Aucun framework n'est une hard dep d'`efe-tool`. L'utilisateur installe ce qu'il utilise :

```toml
# Dans le pyproject.toml DE L'HÔTE
dependencies = [
    "efe-tool==1.2.*",
    "fastapi>=0.100",        # si FastAPI
    # django>=4.2             # si Django
    # flask>=2.3              # si Flask
    # celery>=5.3             # si Celery
]
```

Les modules `efe_tool.integrations.*` vérifient la présence du framework à l'import du module (pas au load d'`efe_tool`).

### §18.3 Soft deps optionnelles d'observabilité

- `prometheus_client` → metrics activés si installé.
- `opentelemetry-api` → tracing activé si installé.

Détection runtime via `importlib.util.find_spec`.

### §18.4 Anti-deps

- `anthropic` : non (c'est une dep C).
- `langchain` / `llama_index` : non.
- Frameworks de queue (dramatiq, rq) : pas MVP.

---

## §19. Plan d'implémentation

### §19.1 Sprint D-1 (2 j) — Façade et types publics

- `efe_tool.integrations.__init__` avec `EfeFacade`, `EfeConfig`, `Verdict`, `HealthStatus`.
- `EfeFacade.__init__`, `preload`, `close`, `classify`, `healthcheck`.
- `EfeFacade.from_env`, `EfeConfig.from_env`.
- Tests : 30 unit tests (mock EfeClient + embedder).
- **Gate D-1** : `from efe_tool.integrations import EfeFacade; facade = EfeFacade.from_env(); facade.preload(); v = facade.classify("hi", catalog="x"); facade.close()`.

### §19.2 Sprint D-2 (1 j) — Erreurs et HTTP mapping

- Taxonomie 8 exceptions.
- `from_cli_error(e)` adapter.
- Test : 20 tests de mapping.

### §19.3 Sprint D-3 (1 j) — Async + threading

- `classify_async` via thread pool.
- Tests concurrence : 10 appels parallèles, no race.

### §19.4 Sprint D-4 (2 j) — FastAPI

- `efe_lifespan`, `get_efe`, dependency injection.
- `register_efe_exception_handlers`.
- Fixture `FakeEfeFacade` pour tests d'hôtes FastAPI.
- Demo app `examples/fastapi_integration/`.

### §19.5 Sprint D-5 (2 j) — Django

- `EfeAppConfig.ready()`.
- `get_efe()` via app registry.
- Middleware d'erreurs.
- Management command `efe_healthcheck`.
- Demo app `examples/django_integration/`.

### §19.6 Sprint D-6 (1 j) — Flask

- `init_efe(app)` factory helper.
- `get_efe()` via `current_app.extensions`.
- Demo.

### §19.7 Sprint D-7 (1.5 j) — Celery

- `setup_efe_for_worker`, signals.
- Handling fork correct.
- Demo task.

### §19.8 Sprint D-8 (0.5 j) — Notebook, Dagster

- Minimalistes : singleton + atexit pour notebook ; ResourceDefinition pour Dagster.

### §19.9 Sprint D-9 (1.5 j) — Observabilité

- Logging events stabilisés.
- `prometheus_client` optionnel.
- `opentelemetry-api` optionnel.

### §19.10 Sprint D-10 (1 j) — Tests d'intégration end-to-end

- docker-compose avec efe-server.
- Pipeline CI qui lance les demos et vérifie.

### §19.11 Sprint D-11 (0.5 j) — Doc

- `docs/integrations/` par framework.
- Troubleshooting.

### §19.12 Total

**~14 jours-homme** pour D au niveau Go.

---

## §20. Matrice de risques

| ID | Risque | Probabilité | Impact | Mitigation |
|---|---|---|---|---|
| R-D1 | Fork gunicorn-prefork + gRPC → deadlock | Haute (si mal init) | Critique | Doc fork + signal `worker_ready` |
| R-D2 | Embedder non preload → first-req 2s | Haute | Haut | `preload()` obligatoire, monitoring |
| R-D3 | Logs leakent le texte user | Moyenne | Haut | Par défaut log seulement hash/length |
| R-D4 | ANTHROPIC_API_KEY dans logs Django | Moyenne | Critique | N'existe pas dans `EfeConfig`, vient de env |
| R-D5 | Thread pool executor saturé | Faible | Moyen | Taille configurable, metrics monitoring |
| R-D6 | GIL bloque encode → throughput limité | Certaine | Moyen | Documenter, sidecar E en V1.1 |
| R-D7 | Circular import entre adapter et efe_tool | Faible | Moyen | Lazy imports dans `__getattr__` |
| R-D8 | Config drift env vs Django settings | Moyenne | Moyen | Précédence claire + test config |
| R-D9 | `close()` avec requêtes en vol tue la réponse | Moyenne | Moyen | Drain avec timeout |
| R-D10 | Framework version incompatible | Faible | Moyen | Range de versions testé matrix CI |
| R-D11 | Hot reload YAML corrompt le catalog actif | Faible | Haut | COW pattern + fallback |
| R-D12 | Metrics `prometheus_client` collision label | Faible | Faible | Prefix `efe_` + namespace configurable |

---

## §21. Critères Go / No-Go spécifiques à D

### §21.1 Critères bloquants

- **G-D1** `EfeFacade` importable depuis `efe_tool.integrations`, sans side-effect.
- **G-D2** `facade.preload()` blocke et retourne `is_ready=True`.
- **G-D3** `facade.classify()` renvoie un `Verdict` typé, jamais `ClassifyResult`.
- **G-D4** `facade.close()` idempotent, drain correct.
- **G-D5** `classify_async` fonctionne dans FastAPI sans blocage du loop.
- **G-D6** Taxonomie d'erreurs : chaque exception `EfeFacadeError` a un `suggested_http_status`.
- **G-D7** Intégrations FastAPI, Django, Flask, Celery fonctionnelles (démo live).
- **G-D8** Fixtures `FakeEfeFacade` disponibles et documentées.
- **G-D9** `healthcheck()` exposable en readiness probe.
- **G-D10** Logging via `logging.getLogger("efe_tool.integrations")`, structuré.
- **G-D11** Tests : 100+ tests verts (façade + adapters + integrations).
- **G-D12** Doc : `docs/integrations/{fastapi,django,flask,celery}.md` + README section.

### §21.2 Nice-to-have

- Intégration Dagster fonctionnelle.
- Intégration Airflow (best effort).
- Metrics Prometheus automatiques.
- Tracing OpenTelemetry automatique.
- Support grpc-async natif (V1.1).
- Circuit breaker intégré (V1.1).
- `rotate_credentials()` (V1.1).

### §21.3 Procédure de passage Go

1. CI verte sur 5 runs consécutifs (incluant tests intégration avec efe-server mock).
2. Démo FastAPI live : POST /tickets → verdict réel, erreurs gérées.
3. Démo Django live : admin action classifie 10 tickets.
4. Test de charge : 100 req/s sur FastAPI, sans fuite mémoire.
5. Audit sécurité : pas de token en clair dans logs, headers, configs exposés.
6. Revue doc : les 4 tutoriels framework cohérents avec le code.
7. Tag `efe-tool X.(Y+1).0` + release notes.

---

## §22. Annexes

### §22.A Graphe complet des dépendances

```
efe_tool.integrations
├── _common            (EfeConfig, EfeFacade, Verdict, HealthStatus, FacadeStats, errors)
├── fastapi            (efe_lifespan, get_efe, register_efe_exception_handlers)
├── django             (EfeAppConfig, get_efe, EfeMiddleware, management commands)
├── flask              (init_efe, get_efe)
├── celery             (setup_efe_for_worker, shutdown_efe, get_efe)
├── notebook           (efe_notebook_singleton, reset_for_test)
├── dagster            (efe_resource)
├── airflow            (get_efe, minimal helper)
└── testing            (FakeEfeFacade, pytest fixtures, mocks)
```

### §22.B Snippet FastAPI complet (§17.2 développé)

```python
# app/main.py
import logging
from contextlib import asynccontextmanager

from fastapi import FastAPI, Depends, HTTPException
from pydantic import BaseModel

from efe_tool.integrations import EfeFacade
from efe_tool.integrations.fastapi import (
    efe_lifespan, get_efe, register_efe_exception_handlers,
)

logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s %(message)s")

class TicketIn(BaseModel):
    message: str
    language: str = "en"

class TicketOut(BaseModel):
    label: str
    confidence: float
    zone: str
    reject: bool
    elapsed_ms: int

app = FastAPI(lifespan=efe_lifespan, title="Support Router")
register_efe_exception_handlers(app)

@app.post("/tickets", response_model=TicketOut)
async def route_ticket(
    body: TicketIn, efe: EfeFacade = Depends(get_efe),
) -> TicketOut:
    if not body.message or len(body.message) > 10_000:
        raise HTTPException(422, "message must be 1..10000 chars")
    verdict = await efe.classify_async(body.message, catalog="support_triage")
    return TicketOut(
        label=verdict.label, confidence=verdict.confidence,
        zone=verdict.zone, reject=verdict.reject,
        elapsed_ms=verdict.elapsed_ms,
    )

@app.get("/healthz")
async def healthz(efe: EfeFacade = Depends(get_efe)):
    hs = efe.healthcheck()
    if hs.status != "SERVING":
        raise HTTPException(503, detail=hs.reason or hs.status)
    return {"status": hs.status, "latency_ms": hs.latency_ms}
```

### §22.C Snippet Django complet

```python
# apps/efe_integration/apps.py
from django.apps import AppConfig
from efe_tool.integrations.django import EfeAppConfig

class EfeIntegrationConfig(EfeAppConfig):
    name = "apps.efe_integration"

# apps/efe_integration/views.py
from django.http import JsonResponse
from django.views.decorators.http import require_POST
from efe_tool.integrations.django import get_efe
from efe_tool.integrations import EfeFacadeError

@require_POST
def classify_view(request):
    text = request.POST.get("message", "")
    try:
        verdict = get_efe().classify(text, catalog="support_triage")
    except EfeFacadeError as e:
        return JsonResponse(
            {"error": e.error_code, "hint": e.hint}, status=e.suggested_http_status,
        )
    return JsonResponse(verdict.as_dict())

# settings.py (excerpts)
INSTALLED_APPS = [
    # ...
    "apps.efe_integration",
]
EFE = {
    "ENDPOINT": "127.0.0.1:50051",
    "CATALOGS": ["support_triage"],
    "CLASSIFY_BUDGET_MS": 3_000,
    "TLS": False,
}
MIDDLEWARE = [
    # ...
    "efe_tool.integrations.django.EfeMiddleware",  # optionnel
]
```

### §22.D Snippet Celery complet

```python
# celery_app.py
from celery import Celery
from celery.signals import worker_ready, worker_shutdown
from efe_tool.integrations.celery import setup_efe_for_worker, shutdown_efe

app = Celery("myapp", broker="redis://...", backend="redis://...")

@worker_ready.connect
def _on_ready(**_):
    setup_efe_for_worker(preload=True)

@worker_shutdown.connect
def _on_shutdown(**_):
    shutdown_efe()

# tasks.py
from celery import shared_task
from efe_tool.integrations.celery import get_efe
from efe_tool.integrations import EfeFacadeTransientError

@shared_task(bind=True, max_retries=3)
def classify_ticket(self, ticket_id: int) -> dict:
    from myapp.models import Ticket
    ticket = Ticket.objects.get(pk=ticket_id)
    try:
        verdict = get_efe().classify(ticket.text, catalog="support_triage")
    except EfeFacadeTransientError as exc:
        raise self.retry(countdown=5, exc=exc)
    ticket.predicted_label = verdict.label
    ticket.predicted_confidence = verdict.confidence
    ticket.save(update_fields=["predicted_label", "predicted_confidence"])
    return verdict.as_dict()
```

### §22.E ADR — décisions d'architecture clés

- **ADR-D-001** : Façade (`EfeFacade`) comme API hôte. Raison : isolation du protocole sous-jacent, `Verdict` typé.
- **ADR-D-002** : Sync core + async wrapper via thread pool. Raison : réutilise la stub gRPC sync testée ; `grpc.aio` en V1.1.
- **ADR-D-003** : `preload()` obligatoire. Raison : éviter la première req lente.
- **ADR-D-004** : `healthcheck()` exposé pour readiness probes. Raison : intégration Kubernetes.
- **ADR-D-005** : Un `EfeFacade` par process, pas de cross-process sharing. Raison : gRPC fork-unsafe.
- **ADR-D-006** : Erreurs mappent vers HTTP via `suggested_http_status`. L'hôte reste libre. Raison : découpler business HTTP de la lib.
- **ADR-D-007** : Logging via `logging.getLogger("efe_tool.integrations")`. Raison : respect de la stack hôte.
- **ADR-D-008** : Pas de retry automatique dans la façade. Raison : politique métier variable.
- **ADR-D-009** : `EfeConfig` frozen après construction. Raison : invariants simples, pas de race condition.
- **ADR-D-010** : Adaptateurs framework dans sous-packages `efe_tool.integrations.<name>`. Chaque sous-package n'importe pas le framework au load. Raison : pas de hard dep sur tous les frameworks.
- **ADR-D-011** : `FakeEfeFacade` fourni dans `integrations.testing`. Raison : ne pas forcer chaque hôte à réinventer le mock.
- **ADR-D-012** : Metrics Prometheus optionnels (soft-dep). Raison : pas d'overhead si non-utilisé.

### §22.F Glossaire

- **Façade** — classe de haut niveau (`EfeFacade`) qui cache les détails de `EfeClient`, `AnchorCatalog`, `LocalSTEmbedder`.
- **Adapter** — module spécifique à un framework (FastAPI, Django, etc.) qui câble la façade dans les hooks du framework.
- **Preload** — chargement anticipé de l'embedder + catalogs au boot pour éviter la latence premier appel.
- **Hot-reload** — rechargement d'un catalog YAML sans redémarrer le processus.
- **Fork-unsafe** — propriété d'une ressource (ici gRPC channel) qui ne survit pas à `os.fork()`.
- **Drain** — attente des appels en vol avant fermeture définitive.
- **COW (Copy-on-Write)** — pattern où on modifie un objet via copie + swap atomique, plutôt qu'en place.
- **Contextvar** — variable de contexte thread-safe / asyncio-safe pour propager des valeurs (ex. request_id).
- **Sidecar** — container déployé dans le même pod Kubernetes qu'un autre pour fournir un service réseau local.

### §22.G Comparaison avec frameworks d'intégration connus

| Propriété | SQLAlchemy | Redis-py | Celery | **EfeFacade** |
|---|---|---|---|---|
| Facade pattern | `Session` | `Redis` | `Celery` | `EfeFacade` |
| Async variant | `AsyncSession` | `Redis.asyncio` | pas d'async | `classify_async` |
| Preload | connection pool | connection pool | — | `preload()` |
| Close | `.close()` / `.dispose()` | `.close()` | — | `.close()` |
| Framework adapters | `flask-sqlalchemy`, `fastapi-sqla` | `flask-redis` | multi-framework | `integrations.*` |
| Singleton per process | oui (engine) | oui | oui | **oui** |
| Typed errors | `SQLAlchemyError` | `RedisError` | `CeleryError` | `EfeFacadeError` |

**Positionnement** : `EfeFacade` suit les conventions établies des librairies Python modernes — singleton per-process, adapters pattern, erreurs typées, support async. Un dev FastAPI/SQLAlchemy reconnaît immédiatement les patterns.

---

**Fin du document.**

*Ce document est la source de vérité pour la Lettre D. Toute évolution passe par PR sur ce fichier avec revue d'un second mainteneur. Les ADR (§22.E) sont append-only. Chaque intégration framework est documentée dans son propre `docs/integrations/<framework>.md` après implémentation.*
