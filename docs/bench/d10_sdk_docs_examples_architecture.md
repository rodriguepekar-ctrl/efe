# Sprint D10 — Architecture : SDK docs + quickstart + 4 examples (DX Pilot 1.0)

> **Statut** : architecture détaillée, prête à implémenter.
> **Phase** : ultra-thinking. Précision chirurgicale suprême.
> **Position** : suit D9 (Docker + Helm + load/soak ; 5 SLOs `D9-*`,
> code D9 audité et fixé sur 10 bugs dont 2 CRITIQUES). D10 est
> **le sprint qui transforme EFE de "service installable" en
> "service utilisable par un développeur partenaire en 10 minutes
> sans aide humaine"**. C'est le verrou critique avant le **Pilot
> 1.0 milestone** (premier client externe payant).
> **Pré-requis** : avoir lu
> [`d9_docker_helm_load_architecture.md`](d9_docker_helm_load_architecture.md)
> et son compagnon
> [`d9_deep_analysis_supplement.md`](d9_deep_analysis_supplement.md)
> qui établissent l'état post-D9 (image distroless < 50 MB ;
> Helm chart 13 templates ; 117 tests anti-régression D8+D9 verts ;
> SDK Python interne mature mais non-publié).

---

## Table des matières

1. [Contexte & mission](#1-contexte--mission)
2. [État post-D9 et écart DX vers Pilot 1.0](#2-état-post-d9-et-écart-dx-vers-pilot-10)
3. [Mesure du gap : modèle "10-minute test"](#3-mesure-du-gap--modèle-10-minute-test)
4. [Architecture SDK Python — surface stabilisée](#4-architecture-sdk-python--surface-stabilisée)
5. [Architecture documentation — Diátaxis + mkdocs-material](#5-architecture-documentation--diátaxis--mkdocs-material)
6. [Architecture quickstart — flow et vérifications](#6-architecture-quickstart--flow-et-vérifications)
7. [Les 4 exemples — sélection, structure, code skeleton](#7-les-4-exemples--sélection-structure-code-skeleton)
8. [Publication — PyPI (OIDC) + docs.efe.ai (GitHub Pages)](#8-publication--pypi-oidc--docsefeai-github-pages)
9. [12 décisions de conception clés](#9-12-décisions-de-conception-clés)
10. [10 risques identifiés et mitigations](#10-10-risques-identifiés-et-mitigations)
11. [Plan jour-par-jour (4 j wallclock)](#11-plan-jour-par-jour-4-j-wallclock)
12. [SLOs ajoutés post-D10](#12-slos-ajoutés-post-d10)
13. [Anti-régression & realism check](#13-anti-régression--realism-check)
14. [Intégration CI : workflow `docs.yml` + example smoke](#14-intégration-ci--workflow-docsyml--example-smoke)
15. [Critères d'acceptation (DoD)](#15-critères-dacceptation-dod)
16. [Out of scope & roadmap follow-ups](#16-out-of-scope--roadmap-follow-ups)
17. [Annexes](#17-annexes)

---

## 1. Contexte & mission

### 1.1 État post-D9

Sprint D9 a livré l'industrialisation du déploiement :

* Image Docker `ghcr.io/efe-project/efe` distroless cc-debian12,
  < 50 MB, signed-attested-scanned (cosign keyless + SBOM CycloneDX
  + Trivy CRITICAL gate).
* Helm chart `deploy/helm/efe/` avec 13 templates dont 9 amendements
  appliqués (clusterIP None toggle, tenant.id label, helm test
  grpc + semantic, etc.).
* Load harness `bench/load/` (4 scénarios ghz) + soak harness
  `bench/soak/` (24 h + détecteur OLS).
* 5 SLOs `D9-*` (4 LOAD + 1 SOAK) gated dans
  `scripts/parse_criterion_report.py`.
* 117 tests anti-régression D8+D9 verts (post-audit, 10 bugs fixés).
* Documentation opérations sous `docs/operations/` (quickstart,
  deployment, troubleshooting, load_test, key_rotation,
  chaos_runbook).

Et pourtant, **aucun développeur externe ne peut utiliser EFE en
moins de quelques heures** :

* Le SDK Python existe (`clients/python/efe_tool/`) mais n'est
  **publié nulle part** (PyPI absent, docs absentes, semver flou).
* Les exemples existent (`clients/python/examples/demo_*.py`) mais
  sont des **scripts de démo internes** non polis pour un usage
  partenaire (chemin de fichiers en dur, pas de `if __name__`,
  pas de gestion d'erreurs).
* La quickstart D9 (`docs/operations/quickstart.md`) référence
  un workflow Helm mais **les commandes ne s'enchaînent pas**
  réellement contre un Helm chart fraîchement installé sans étapes
  intermédiaires non documentées (variables d'env attendues, pip
  install introuvable car SDK pas sur PyPI).
* La doc `docs.efe.ai` est **un site mkdocs vide** (configuration
  présente, pages génériques mais pas de tutoriel ni d'API
  reference).

### 1.2 Mission D10 (énoncé chirurgical)

> 1. **Geler la surface SDK Python** comme contrat semver-versionné
>    (`v0.1.9` aligné serveur). Définir le `__all__` public,
>    documenter la politique de dépréciation, ajouter les type
>    stubs (.pyi) pour le support IDE, exposer une API async en
>    plus du sync existant. Aucune régression de compatibilité.
>
> 2. **Restructurer la documentation** selon le framework
>    [Diátaxis](https://diataxis.fr) (Tutorials / How-to /
>    Reference / Explanation). Auto-générer la référence API
>    avec `mkdocstrings`. Publier sur `docs.efe.ai` via GitHub
>    Pages, versioned via `mike` (URLs `docs.efe.ai/0.1.x/...`).
>
> 3. **Authoring 4 exemples** copy-paste-ready, testés en CI
>    contre un stub server :
>
>    * `examples/01_quickstart_classify.py` — zéro-à-classification
>      en 30 LOC.
>    * `examples/02_custom_catalog.py` — définir ses propres intents
>      et anchors, plus le cas typique partenaire.
>    * `examples/03_multilang_streaming.py` — flux concurrent
>      français + anglais, asyncio + back-pressure.
>    * `examples/04_production_client.py` — JWT + retries
>      exponentiel + OTel trace + idempotency batch + structured
>      logging.
>
> 4. **Polir la quickstart** D9 pour qu'elle marche **réellement**
>    end-to-end : install Helm chart → port-forward → `pip install
>    efe-tool` → run example 01 → 3 cas labellés OK. Cible : 10 min
>    wallclock validé par un dev externe.
>
> 5. **Publier via CI** : tag → release.yml étendu pour pousser
>    le SDK sur PyPI (OIDC trusted publisher, sans token statique)
>    et déployer la doc via `gh-pages`. Linter strict
>    (`mkdocs build --strict`) + link-checker + execution-smoke
>    des 4 exemples gatés en CI.
>
> 6. **Définir 5 SLOs `D10-*`** : 3 doc + 2 DX, avec mesures
>    automatiques en CI. Aucune SLO antérieure régressée.

### 1.3 Localisation des changements

| Fichier | Modification | Sub-step |
|---------|--------------|----------|
| `clients/python/efe_tool/__init__.py` | Stabiliser `__all__` ; ajouter `__version__` ; ajouter alias async | D10.1 |
| `clients/python/efe_tool/_async/client.py` | **Nouveau** — `AsyncEfeClient` async-aware | D10.1 |
| `clients/python/efe_tool/py.typed` | **Nouveau** marker file (PEP 561) | D10.1 |
| `clients/python/efe_tool/_internal/` | **Nouveau** — déplacer modules privés | D10.1 |
| `clients/python/pyproject.toml` | Compléter metadata (description, license, classifiers, urls) | D10.1 |
| `clients/python/CHANGELOG.md` | **Nouveau** — semver journal SDK | D10.1 |
| `docs/docs/index.md` | Refonte page d'accueil partenaire | D10.2 |
| `docs/docs/tutorial/quickstart.md` | **Nouveau** (Tutorial — Diátaxis) | D10.2 |
| `docs/docs/tutorial/concepts.md` | **Nouveau** — Mode 1, anchors, hybrid | D10.2 |
| `docs/docs/howto/install.md` | **Nouveau** (How-to) | D10.2 |
| `docs/docs/howto/auth.md` | **Nouveau** | D10.2 |
| `docs/docs/howto/custom-catalog.md` | **Nouveau** | D10.2 |
| `docs/docs/howto/observability.md` | **Nouveau** | D10.2 |
| `docs/docs/reference/api.md` | **Nouveau** — auto-généré via `mkdocstrings` | D10.2 |
| `docs/docs/reference/cli.md` | **Nouveau** — `efe-tool` CLI | D10.2 |
| `docs/docs/reference/errors.md` | **Nouveau** — gRPC code → exception | D10.2 |
| `docs/docs/explanation/why-anchors.md` | **Nouveau** — design rationale | D10.2 |
| `docs/docs/explanation/hybrid.md` | **Nouveau** — Plan B explained | D10.2 |
| `docs/mkdocs.yml` | Activer mkdocstrings ; navigation Diátaxis ; mike | D10.2 |
| `docs/requirements-docs.txt` | **Nouveau** — pinned doc deps | D10.2 |
| `examples/01_quickstart_classify.py` | **Nouveau** | D10.3 |
| `examples/02_custom_catalog.py` | **Nouveau** | D10.3 |
| `examples/03_multilang_streaming.py` | **Nouveau** | D10.3 |
| `examples/04_production_client.py` | **Nouveau** | D10.3 |
| `examples/README.md` | **Nouveau** — index | D10.3 |
| `examples/tests/test_examples_smoke.py` | **Nouveau** — exec-smoke contre stub | D10.3 |
| `docs/operations/quickstart.md` | Réécrire pour aligner sur SDK PyPI | D10.4 |
| `.github/workflows/docs.yml` | **Nouveau** — build + link-check + deploy | D10.5 |
| `.github/workflows/release.yml` | Étendre — publish PyPI (OIDC) + push docs | D10.6 |
| `bench_baselines/v0.1.9/baseline.json` | Ajouter SLOs `D10-*` | D10.7 |
| `BENCHMARKS.md § 12` | **Nouveau** | D10.7 |
| `docs/audits/d10_dx_report.md` | **Nouveau** — sprint closure skeleton | D10.7 |
| `CHANGELOG.md` | Entrée Unreleased D10 | D10.7 |

---

## 2. État post-D9 et écart DX vers Pilot 1.0

### 2.1 Ce que D9 a déjà livré pour l'opérabilité

| Aspect | État D9 | DX gap restant |
|--------|--------|----------------|
| Image Docker reproducible | ✅ | aucun |
| Helm chart installable | ✅ | aucun |
| TLS terminaison côté Service | ✅ (cert-manager) | aucun |
| Métriques Prometheus | ✅ | aucun |
| Runbook ops production | ✅ (deployment.md) | partner pilot ≠ prod |
| Quickstart 10 min | partiel | **commandes ne s'enchaînent pas** |
| SDK installable | ❌ (pas PyPI) | **bloquant** |
| Documentation API | ❌ (mkdocs vide) | **bloquant** |
| Exemples polis | ❌ (`demo_*.py` internes) | **bloquant** |
| Versioning SDK ↔ serveur | ❌ flou | risque silent breakage |

### 2.2 Persona cible — l'« ingénieur partenaire »

* Backend dev, Python 3.10+, déjà familier de gRPC ou REST.
* Cluster k8s d'évaluation à disposition (minikube ou shared).
* Connait `pip` et `helm` mais ne lit ni le code ni l'architecture
  EFE.
* Budget temps : **un après-midi (4 h max)** pour valider qu'EFE
  classifie ses 10-50 cas représentatifs avant d'engager un budget
  d'évaluation.
* Critère d'abandon : si après 30 min il n'a rien classé qui marche,
  il passe au concurrent.

### 2.3 Ce qui manque pour le Pilot 1.0

```
┌────────────────────────────────────────────────────────────┐
│  Partner engineer Day 1 walkthrough                        │
└────────────────────────────────────────────────────────────┘

  09:00  Lecture docs.efe.ai      ─────────►  ❌ vide
  09:10  helm install              ─────────►  ✅ marche (D9)
  09:13  pip install efe-tool      ─────────►  ❌ pas sur PyPI
  09:15  python examples/01_…py    ─────────►  ❌ exemples pas pour partner
  09:17  Première classify         ─────────►  ❌ bloqué
  09:20                                            ABANDONNE
```

D10 cible le **gap entre "helm install OK" et "première classify
qui marche"**. C'est ~7 minutes de friction qui distinguent un
pilot adopté d'un pilot perdu.

### 2.4 Critère "Pilot 1.0 ready"

À l'issue de D10, le critère est un **test reproductible** :

```
$ helm install efe oci://ghcr.io/efe-project/charts/efe \
    --version 0.1.9 -n efe --create-namespace \
    --set service.headless=true --set replicaCount=2
$ kubectl wait deploy/efe -n efe --for=condition=available --timeout=120s
$ kubectl port-forward -n efe svc/efe 50051:50051 &
$ pip install efe-tool==0.1.9
$ python -m efe_tool.examples.quickstart  # ou: python examples/01_quickstart_classify.py
  ✓ classified 'Someone charged my card without my consent.' → fraud (0.92)
  ✓ classified 'Where do I find a record of my past invoices?' → question (0.88)
  ✓ classified "L'application plante quand j'ouvre le menu" → bug (0.91)
```

**Si ces 5 commandes succèdent en moins de 10 minutes sur une
machine partenaire fraîche (avec minikube déjà installé), D10 est
un succès.**

---

## 3. Mesure du gap : modèle "10-minute test"

### 3.1 Pourquoi 10 minutes

Référence empirique : la « 10-minute test » de Stripe / Twilio /
Vercel — produit DX tier 1 = un dev passe d'inconnu à premier
résultat utile en moins de 10 min. Au-dessus de 10 min, le drop-out
rate dépasse 50 %.

EFE n'a pas le luxe d'être plus complexe que Stripe à intégrer.
Si la première classify prend > 10 min, on perd le partenaire.

### 3.2 Décomposition du chemin critique

```
Étape                                   Budget   Acteur
──────────────────────────────────────  ───────  ──────────
1. helm install                          120 s   helm/k8s
2. kubectl wait                          120 s   k8s scheduling
3. pip install efe-tool                   30 s   PyPI mirror
4. read example 01 (or quickstart)        60 s   partner
5. python examples/01_…py                 30 s   SDK + gRPC roundtrip × 3
                                         ─────
                                         360 s = 6 min

  + buffer (debug, network glitch)        +240 s
                                         ─────
                                         600 s = 10 min ✓
```

**Conclusion** : sur les 600 s de budget, 360 s sont des coûts
inflexibles (helm, k8s, pip). **Reste 240 s pour la lecture +
debug.** D10 doit donc :

* **Réduire la friction lecture** (quickstart minimal, exemples
  ultra-courts).
* **Réduire la friction debug** (messages d'erreur explicites,
  troubleshooting cross-référencé).
* **Préempter les bugs courants** (DNS, port-forward, version
  mismatch).

### 3.3 Métriques DX mesurables en CI

Trois métriques chiffrées que D10 cible :

| Métrique | Cible D10 | Mesure |
|----------|----------:|--------|
| Lecture quickstart → première classify | ≤ 10 min | Manuel (3 reviewers externes) |
| `python examples/01_…py` aggregate exec time | ≤ 30 s | CI automatique |
| Lignes de code utilisateur dans example 01 | ≤ 20 LOC | grep + assert en CI |

La 3ᵉ métrique (LOC) est la plus actionnable. Une référence solide :
Stripe's "Hello world" Python = **10 LOC**. EFE peut viser 15-20 LOC
(Mode 1 SwarmSpec a plus de surface qu'un PaymentIntent).

### 3.4 Le coût caché : copy-paste fidelity

Le partenaire copie-colle le code de la doc. Toute typo dans la
doc = bug chez le partenaire. Mitigation D10 :

* Tous les blocs `python` dans la doc sont **extraits depuis les
  fichiers `examples/*.py` réels** via `pymdownx.snippets`.
* Plus aucun "code dans la doc" qui n'est pas testé en CI.
* La doc ne contient JAMAIS de chemin absolu, jamais de placeholder
  `<YOUR_TOKEN>` sans variable d'env définie.

---

## 4. Architecture SDK Python — surface stabilisée

### 4.1 Surface publique freezée — `efe_tool.__init__`

```python
# clients/python/efe_tool/__init__.py
"""EFE — Ephemeral Fluid Swarm Python SDK.

Public API (semver-stable post-v0.1.9):

    Client / decoder:
        EfeClient                     — synchronous gRPC client
        AsyncEfeClient                — asyncio client (D10 NEW)
        HybridClassifier              — Plan B classifier (D8)

    Catalog:
        Catalog                       — anchor catalog model
        load_builtin_catalog          — load bundled catalogs
        compile_catalog               — text → embedded anchors

    Errors:
        EfeError                      — base exception
        EfeAuthError                  — gRPC UNAUTHENTICATED
        EfeRateLimitError             — gRPC RESOURCE_EXHAUSTED
        EfeInvalidArgumentError       — gRPC INVALID_ARGUMENT
        EfeUnavailableError           — gRPC UNAVAILABLE / DEADLINE_EXCEEDED
        EfeInternalError              — gRPC INTERNAL

    Types (transparent re-exports for IDE auto-complete):
        ExecuteRequest, ExecuteResponse, SwarmSpec, AlignmentCone,
        Anchor, PopulationEntry, ConvergenceThresholds, DecodeMode,
        ConeRef, ResourceBudget, Verdict, DecodedOutput

Underscore-prefixed modules (`_internal`, `_async`) are NOT public.
Importing them is allowed but DOES NOT respect semver.
"""

from efe_tool._version import __version__

# Sync client
from efe_tool.client import EfeClient

# Async client (D10 NEW)
from efe_tool._async.client import AsyncEfeClient

# Decoders
from efe_tool.integrations.hybrid_decoder import (
    HybridClassifier,
    HybridResult,
    raw_cosine_predict,
)

# Catalog
from efe_tool.catalog import (
    Catalog,
    load_builtin_catalog,
    compile_catalog,
)

# Errors
from efe_tool.errors import (
    EfeError,
    EfeAuthError,
    EfeRateLimitError,
    EfeInvalidArgumentError,
    EfeUnavailableError,
    EfeInternalError,
)

# Re-export proto types for IDE help (transparent — these are
# generated code and follow the protobuf semver, not ours)
from efe_tool.types import (
    ExecuteRequest, ExecuteResponse,
    SwarmSpec, AlignmentCone, Anchor,
    PopulationEntry, ConvergenceThresholds,
    DecodeMode, ConeRef, ResourceBudget,
    Verdict, DecodedOutput,
)

__all__ = [
    "__version__",
    # Clients
    "EfeClient", "AsyncEfeClient",
    # Decoders
    "HybridClassifier", "HybridResult", "raw_cosine_predict",
    # Catalog
    "Catalog", "load_builtin_catalog", "compile_catalog",
    # Errors
    "EfeError", "EfeAuthError", "EfeRateLimitError",
    "EfeInvalidArgumentError", "EfeUnavailableError",
    "EfeInternalError",
    # Proto types (re-exported for IDE — protobuf semver, not ours)
    "ExecuteRequest", "ExecuteResponse", "SwarmSpec",
    "AlignmentCone", "Anchor", "PopulationEntry",
    "ConvergenceThresholds", "DecodeMode", "ConeRef",
    "ResourceBudget", "Verdict", "DecodedOutput",
]
```

### 4.2 Politique de versioning

```
SemVer 0.1.x — minor / patch :
    * MINOR : breaking change permis (pré-1.0).
    * PATCH : compatible additif.
    * Le SDK MAJEUR/MINEUR DOIT matcher le serveur (0.1.x SDK ↔
      0.1.x serveur).

Politique de dépréciation :
    1. Une fonction "à retirer" est annotée @deprecated dès la
       version N.
    2. Émet DeprecationWarning au runtime.
    3. La fonction est supprimée à la version N+2.
    4. Le CHANGELOG note la suppression dans la section "Removed".
```

Implémentation : décorateur `@deprecated` dans
`efe_tool/_internal/deprecation.py` :

```python
import functools
import warnings


def deprecated(reason: str, since: str, removal: str):
    """Mark a public symbol as deprecated.

    The deprecation warning is shown at first call (per-process).
    """
    def decorator(fn):
        warned = False
        @functools.wraps(fn)
        def wrapper(*args, **kwargs):
            nonlocal warned
            if not warned:
                warnings.warn(
                    f"{fn.__qualname__} is deprecated since {since} "
                    f"and will be removed in {removal}: {reason}",
                    DeprecationWarning,
                    stacklevel=2,
                )
                warned = True
            return fn(*args, **kwargs)
        return wrapper
    return decorator
```

### 4.3 Async API — parité fonctionnelle

Le sync existant (`EfeClient`) reste la voie par défaut. Pour les
partenaires qui ont une boucle asyncio (FastAPI, aiohttp), on
expose `AsyncEfeClient` via `efe_tool._async.client`.

```python
# clients/python/efe_tool/_async/client.py
"""Async variant of EfeClient — built on grpc.aio."""

from __future__ import annotations

import grpc
from grpc.aio import insecure_channel, secure_channel
from efe_tool.types import ExecuteRequest, ExecuteResponse
from efe_tool._internal.proto import efe_pb2_grpc as _grpc_stubs


class AsyncEfeClient:
    """Asyncio-aware EFE client.

    Functional parity with `EfeClient` — same arguments, same
    methods, all async.

        async with AsyncEfeClient(host="127.0.0.1:50051", insecure=True) as client:
            response = await client.execute(request)
            print(response.output.text)
    """
    def __init__(
        self,
        *,
        host: str,
        insecure: bool = False,
        token: str | None = None,
        tenant: str | None = None,
        timeout_s: float = 30.0,
    ): ...

    async def __aenter__(self) -> "AsyncEfeClient": ...
    async def __aexit__(self, *exc) -> None: ...

    async def execute(
        self,
        request: ExecuteRequest,
    ) -> ExecuteResponse: ...

    async def execute_streaming(
        self,
        request: ExecuteRequest,
    ): ...    # → AsyncIterator[StreamChunk]

    async def close(self) -> None: ...
```

### 4.4 Type stubs et IDE support

* PEP 561 marker `clients/python/efe_tool/py.typed` (fichier vide).
* `pyproject.toml` :
  ```toml
  [tool.setuptools.package-data]
  efe_tool = ["py.typed", "*.pyi"]
  ```
* mypy strict mode sur `clients/python/efe_tool/` (déjà
  partiellement actif via pre-commit).

### 4.5 Erreurs — hiérarchie mappée à gRPC

```python
# clients/python/efe_tool/errors.py (extension D10)
import grpc


class EfeError(Exception):
    """Base for all EFE SDK errors. Catches everything safe-by-default."""

    def __init__(self, message: str, *,
                 grpc_code: int | None = None,
                 details: dict | None = None,
                 retryable: bool = False):
        super().__init__(message)
        self.grpc_code = grpc_code
        self.details = details or {}
        self.retryable = retryable


class EfeAuthError(EfeError):
    """gRPC UNAUTHENTICATED (16) — token absent / invalid / expired."""
    retryable = False


class EfeRateLimitError(EfeError):
    """gRPC RESOURCE_EXHAUSTED (8) — server rate limit hit."""
    retryable = True       # backoff, then retry


class EfeInvalidArgumentError(EfeError):
    """gRPC INVALID_ARGUMENT (3) — request shape invalid."""
    retryable = False      # don't retry a malformed request


class EfeUnavailableError(EfeError):
    """gRPC UNAVAILABLE (14) / DEADLINE_EXCEEDED (4) — transient."""
    retryable = True


class EfeInternalError(EfeError):
    """gRPC INTERNAL (13) / UNKNOWN (2) — server bug or panic."""
    retryable = False      # don't hammer a broken server


_CODE_MAP: dict[int, type[EfeError]] = {
    grpc.StatusCode.UNAUTHENTICATED.value[0]: EfeAuthError,
    grpc.StatusCode.RESOURCE_EXHAUSTED.value[0]: EfeRateLimitError,
    grpc.StatusCode.INVALID_ARGUMENT.value[0]: EfeInvalidArgumentError,
    grpc.StatusCode.UNAVAILABLE.value[0]: EfeUnavailableError,
    grpc.StatusCode.DEADLINE_EXCEEDED.value[0]: EfeUnavailableError,
    grpc.StatusCode.INTERNAL.value[0]: EfeInternalError,
    grpc.StatusCode.UNKNOWN.value[0]: EfeInternalError,
}


def raise_from_grpc(rpc_error: grpc.RpcError) -> "EfeError":
    """Translate a grpc.RpcError to the appropriate EfeError subclass."""
    code = rpc_error.code()
    cls = _CODE_MAP.get(code.value[0], EfeError)
    return cls(
        rpc_error.details() or str(rpc_error),
        grpc_code=code.value[0],
        details={"grpc_status": code.name},
        retryable=cls.retryable if hasattr(cls, "retryable") else False,
    )
```

### 4.6 Réorganisation interne `_internal/`

Ce qui est privé aujourd'hui mais public-par-défaut (Python n'a pas
de `pub`/`internal` natif) bouge sous `_internal/` :

```
clients/python/efe_tool/
├── __init__.py              ← public surface (only __all__)
├── _version.py               ← single source of truth for version
├── client.py                 ← public sync client
├── catalog.py                ← public catalog ops
├── types.py                  ← public re-exports of proto
├── errors.py                 ← public error hierarchy
├── py.typed                  ← PEP 561 marker (NEW)
├── integrations/
│   └── hybrid_decoder.py    ← public (D8)
├── _async/                  ← NEW
│   ├── __init__.py
│   └── client.py            ← AsyncEfeClient
├── _internal/               ← NEW (was scattered)
│   ├── __init__.py
│   ├── proto/                ← generated pb2 stubs
│   ├── grpc_channel.py       ← channel factory + auth interceptor
│   ├── retry.py              ← backoff helper
│   ├── deprecation.py        ← @deprecated
│   └── telemetry.py          ← OTel tracer wrapper
├── scenarios/
│   └── builtin/
│       ├── support_triage.yaml
│       ├── fraud_alert.yaml
│       └── billing_escalation.yaml
└── examples/                ← NEW — `python -m efe_tool.examples.X`
    ├── __init__.py
    ├── quickstart.py
    └── …
```

---

## 5. Architecture documentation — Diátaxis + mkdocs-material

### 5.1 Pourquoi Diátaxis

Le framework [Diátaxis](https://diataxis.fr) classe la doc en 4
catégories orthogonales :

| Type | Audience | Format | Goal |
|------|----------|--------|------|
| **Tutorial** | Débutant | "Apprenez par la pratique" | Compétence acquise |
| **How-to** | Praticien | "Résoudre un problème ciblé" | Tâche accomplie |
| **Reference** | Tous | "Description exhaustive" | Lookup rapide |
| **Explanation** | Curieux / architecte | "Comprendre le pourquoi" | Compréhension |

Mélanger les 4 dans un même page est la cause #1 de mauvaise doc
(mélange info-action-référence-théorie). Diátaxis impose la
séparation.

### 5.2 Arborescence cible `docs/docs/`

```
docs/docs/
├── index.md                     ← landing page (60 LOC, 3 calls-to-action)
├── tutorial/
│   ├── quickstart.md            ← "10 min from zero"
│   ├── concepts.md              ← "What is Mode 1, anchors, hybrid"
│   └── your-first-classifier.md ← guided walkthrough custom catalog
├── howto/
│   ├── install.md               ← pip install, helm install
│   ├── auth.md                  ← JWT setup
│   ├── custom-catalog.md        ← define your own intents
│   ├── observability.md         ← logs / metrics / tracing
│   ├── streaming.md             ← async + back-pressure
│   ├── retries.md               ← idempotency + backoff
│   └── language-detection.md    ← multilingual best practices
├── reference/
│   ├── api.md                   ← AUTO from docstrings (mkdocstrings)
│   ├── cli.md                   ← efe-tool CLI flags
│   ├── errors.md                ← gRPC code → exception mapping table
│   ├── catalog-schema.md        ← YAML schema for custom catalogs
│   └── env-vars.md              ← every EFE_* env var
├── explanation/
│   ├── architecture.md          ← deep dive — high level
│   ├── why-anchors.md           ← design rationale (zero-shot vs fine-tune)
│   ├── hybrid.md                ← Plan B explained (raw cosine + swarm)
│   └── tradeoffs.md             ← what EFE is NOT
├── operations/                  ← carried over from D9 (SRE-facing)
│   ├── quickstart.md            (← still used, partner-pilot variant)
│   ├── deployment.md
│   ├── troubleshooting.md
│   ├── load_test.md
│   ├── key_rotation.md
│   └── chaos_runbook.md
├── changelog.md                 ← single-source CHANGELOG.md mirror
└── contributing.md              ← link to CONTRIBUTING.md
```

### 5.3 Configuration `mkdocs.yml`

Diff principal vs l'existant :

```yaml
site_name: EFE Documentation
site_description: Ephemeral Fluid Swarm — multi-agent inference SDK
site_url: https://docs.efe.ai
repo_url: https://github.com/efe-ai/efe
edit_uri: edit/main/docs/docs/
docs_dir: docs

theme:
  name: material
  palette: [...]   # unchanged

plugins:
  - search
  - mkdocstrings:
      handlers:
        python:
          paths: [../clients/python]
          options:
            docstring_style: google
            show_signature_annotations: true
            separate_signature: true
            show_source: false
            members_order: source
            filters: ["!^_"]            # hide private symbols
  - mike:                                # versioned docs (D10 NEW)
      canonical_version: latest
      version_selector: true

nav:
  - Home: index.md
  - Tutorial:
    - Quickstart: tutorial/quickstart.md
    - Concepts: tutorial/concepts.md
    - Your first classifier: tutorial/your-first-classifier.md
  - How-to:
    - Install: howto/install.md
    - Authentication: howto/auth.md
    - Custom catalog: howto/custom-catalog.md
    - Observability: howto/observability.md
    - Streaming: howto/streaming.md
    - Retries: howto/retries.md
    - Language detection: howto/language-detection.md
  - Reference:
    - API: reference/api.md
    - CLI: reference/cli.md
    - Errors: reference/errors.md
    - Catalog schema: reference/catalog-schema.md
    - Environment variables: reference/env-vars.md
  - Explanation:
    - Architecture: explanation/architecture.md
    - Why anchors: explanation/why-anchors.md
    - Hybrid (Plan B): explanation/hybrid.md
    - Trade-offs: explanation/tradeoffs.md
  - Operations: ...
  - Changelog: changelog.md

markdown_extensions:
  - admonition
  - attr_list
  - footnotes
  - tables
  - pymdownx.details
  - pymdownx.snippets:                   # for ```--8<-- "..."``` in MD
      check_paths: true
      base_path: ["..", "../examples"]
  - pymdownx.highlight:
      anchor_linenums: true
  - pymdownx.inlinehilite
  - pymdownx.tabbed:
      alternate_style: true
  - pymdownx.tasklist:
      custom_checkbox: true
```

### 5.4 mkdocstrings — auto-référence API

Une page `docs/docs/reference/api.md` qui produit toute la
référence à partir des docstrings :

```markdown
# API Reference

## efe_tool

::: efe_tool
    options:
      show_root_heading: true
      show_root_full_path: false

## efe_tool.client

::: efe_tool.client
    options:
      show_root_heading: true
      members:
        - EfeClient

## efe_tool._async.client

::: efe_tool._async.client
    options:
      show_root_heading: true
      members:
        - AsyncEfeClient

## efe_tool.integrations.hybrid_decoder

::: efe_tool.integrations.hybrid_decoder
    options:
      show_root_heading: true
      members:
        - HybridClassifier
        - HybridResult
        - raw_cosine_predict

## efe_tool.errors

::: efe_tool.errors
    options:
      show_root_heading: true
```

Test anti-régression D10 : `tests/test_d10_api_reference_coverage.py`
parse cette page rendue + le `__all__` du package, et asserte que
chaque symbole `__all__` a une entrée dans la référence générée.

### 5.5 Versioning de la doc — `mike`

`mike` (mkdocs plugin) gère plusieurs versions concurrentes sur
la même origine GitHub Pages :

```
docs.efe.ai/                    → redirige vers /latest/
docs.efe.ai/latest/             → alias vers la dernière stable
docs.efe.ai/0.1.9/              → docs gelées de v0.1.9
docs.efe.ai/0.1.10/             → docs gelées de v0.1.10
docs.efe.ai/dev/                → trunk
```

Workflow CI :

* Push tag `v0.1.10` → `release.yml` build docs → `mike deploy 0.1.10 latest`.
* Push sur `main` → `docs.yml` build docs → `mike deploy dev`.

### 5.6 Snippets : zero-drift entre doc et code

Toutes les docs de tutorial/howto utilisent l'extension
`pymdownx.snippets` pour inclure des fragments depuis les fichiers
réels :

```markdown
## Your first classification

The complete example is `examples/01_quickstart_classify.py`. Inline:

```python
--8<-- "examples/01_quickstart_classify.py:imports"

--8<-- "examples/01_quickstart_classify.py:classify-loop"
```
```

Et dans `examples/01_quickstart_classify.py`, on annote :

```python
# --8<-- [start:imports]
from efe_tool import EfeClient, HybridClassifier, load_builtin_catalog
# --8<-- [end:imports]
...
# --8<-- [start:classify-loop]
for text in cases:
    result = decoder.predict(text)
    print(f"{text!r:60s} → {result.label} ({result.confidence:.2f})")
# --8<-- [end:classify-loop]
```

→ La doc montre toujours **le code testé**.

---

## 6. Architecture quickstart — flow et vérifications

### 6.1 Quickstart partner-pilot (NEW) vs ops-quickstart (D9)

Deux quickstarts coexistent post-D10, avec des audiences distinctes :

| Quickstart | Audience | Lieu | Goal |
|-----------|----------|------|------|
| `docs/docs/tutorial/quickstart.md` | partner dev | `docs.efe.ai/tutorial/quickstart` | "premier classify en 10 min" |
| `docs/operations/quickstart.md` | SRE / ops partner | inchangé (D9) | "déploiement HELM correct" |

### 6.2 Flow tutorial/quickstart.md

```
1. Pre-flight (30 s)             → vérifier helm, kubectl, python
2. Installer EFE (2 min)          → helm install
3. Installer le SDK (30 s)         → pip install efe-tool
4. Premier classify (60 s)         → python examples/01_…py
5. Comprendre la sortie (30 s)     → label, confidence, branch
6. Que faire ensuite ? (30 s)      → liens vers tutorial/your-first-classifier
```

Total : ~5 min. Buffer 5 min pour les imprévus = 10 min cible.

### 6.3 Pre-flight — éviter les pièges

```bash
$ python -c "import sys; assert sys.version_info >= (3, 10)" \
  && echo "Python OK" || echo "Need Python 3.10+"

$ helm version --short          # ≥ 3.13
$ kubectl version --short       # client + server ≥ 1.27
```

### 6.4 Le test de validation — script `validate_quickstart.sh`

```bash
#!/usr/bin/env bash
# docs/docs/tutorial/validate_quickstart.sh
#
# Optional script for the quickstart reader to ensure their
# environment passes the 6 quickstart steps without manual
# intervention. Used by D10 audit + by 3 external reviewers.

set -Eeuo pipefail
trap 'echo "::error::quickstart validation FAILED at line $LINENO"' ERR

START=$(date +%s)

# Step 1
echo "[1/6] preflight"
python3 -c "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)"
helm version --short >/dev/null
kubectl get nodes >/dev/null

# Step 2
echo "[2/6] helm install"
helm install efe oci://ghcr.io/efe-project/charts/efe --version 0.1.9 \
  -n efe --create-namespace --set service.headless=true --set replicaCount=2 \
  --wait --timeout 120s

# Step 3
echo "[3/6] pip install"
python3 -m pip install --quiet "efe-tool==0.1.9"

# Step 4
echo "[4/6] port-forward"
kubectl port-forward -n efe svc/efe 50051:50051 &
PF_PID=$!
trap 'kill $PF_PID 2>/dev/null || true ; helm uninstall efe -n efe || true' EXIT
sleep 3

# Step 5
echo "[5/6] first classify"
python3 examples/01_quickstart_classify.py | tee /tmp/qs.out

# Step 6 — assertions
echo "[6/6] validate output"
grep -q "fraud" /tmp/qs.out
grep -q "question" /tmp/qs.out
grep -q "bug" /tmp/qs.out

ELAPSED=$(($(date +%s) - START))
echo "✅ quickstart validated in $ELAPSED s"
test "$ELAPSED" -le 600 || { echo "::error::> 10 min budget"; exit 1; }
```

Le script est **versionné dans le repo**. La SLO `D10-DX-1`
(quickstart end-to-end ≤ 10 min) est mesurée par 3 reviewers
externes qui exécutent ce script et reportent le `ELAPSED`.

---

## 7. Les 4 exemples — sélection, structure, code skeleton

### 7.1 Critères de sélection

Les 4 exemples doivent couvrir :

* Beginner (5 min copy-paste win).
* Intermediate (custom data, real partner work).
* Advanced (concurrency, multilang).
* Production-grade (auth, retries, observability).

Chaque exemple :

* **Standalone .py** (pas Jupyter) — testable, lintable, diffable.
* **Maximum 100 LOC**.
* **Auto-contenu** (pas de catalogue externe ni de fichier joint).
* Préface en docstring expliquant le **why**.
* Bas de fichier : section **Next steps** avec liens vers la doc.

### 7.2 Example 1 — `examples/01_quickstart_classify.py`

```python
"""Sprint D10 — example 1: quickstart classifier (≤ 30 LOC).

Goal: first classification in 30 seconds. Uses the bundled
`support_triage` catalog (4 intents: fraud / billing / question /
bug). Demonstrates the canonical `HybridClassifier` happy path.

Pre-requisites:
  * efe-server reachable on 127.0.0.1:50051 (helm install + port-forward)
  * pip install efe-tool==0.1.9

Run:
  python examples/01_quickstart_classify.py
"""
from efe_tool import EfeClient, HybridClassifier, load_builtin_catalog

CASES = [
    "Someone charged my card without my consent.",
    "Where do I find a record of my past invoices?",
    "L'application plante quand j'ouvre le menu paramètres.",
    "How do I reset my two-factor authentication?",
    "I was charged twice for the same order.",
]


def main() -> None:
    catalog = load_builtin_catalog("support_triage")
    with EfeClient(host="127.0.0.1:50051", insecure=True) as client:
        decoder = HybridClassifier(client=client, catalog=catalog)
        for text in CASES:
            result = decoder.predict(text)
            print(
                f"{text[:55]:55s} → "
                f"{(result.label or 'UNKNOWN'):12s} "
                f"({result.confidence:.2f}, branch={result.branch})"
            )


if __name__ == "__main__":
    main()
```

Expected output:

```
Someone charged my card without my consent.            → fraud        (0.92, branch=agree)
Where do I find a record of my past invoices?          → question     (0.88, branch=agree)
L'application plante quand j'ouvre le menu paramètres. → bug          (0.91, branch=agree)
How do I reset my two-factor authentication?           → question     (0.86, branch=agree)
I was charged twice for the same order.                → billing      (0.93, branch=agree)
```

LOC count : ~25. Cible ≤ 30 ✅.

### 7.3 Example 2 — `examples/02_custom_catalog.py`

```python
"""Sprint D10 — example 2: custom anchor catalog.

Goal: define your own intents (e-commerce return-management) and
classify against them. Demonstrates the full anchor-design loop:

  1. Author intent labels + 2-5 example prompts each
  2. Compile to embedded anchors via `compile_catalog`
  3. Use the resulting catalog with HybridClassifier

This is the canonical "partner adapts EFE to their domain" path.

Pre-requisites:
  * efe-server reachable on 127.0.0.1:50051
  * pip install efe-tool==0.1.9

Run:
  python examples/02_custom_catalog.py
"""
from efe_tool import (
    Catalog, EfeClient, HybridClassifier, compile_catalog,
)


# 1. Define intents as (label, [example prompts]).
INTENTS = {
    "track_order": [
        "Where is my order?",
        "What's the shipping status of order ABC123?",
        "Quand vais-je recevoir mon colis ?",
    ],
    "request_refund": [
        "I'd like to return this item and get a refund.",
        "How do I get my money back for the broken product?",
        "Je veux annuler ma commande et être remboursé.",
    ],
    "product_question": [
        "What are the dimensions of this item?",
        "Is this product compatible with my model X?",
        "Quelle est la garantie sur cet article ?",
    ],
    "complaint": [
        "This is unacceptable, I want to speak to a manager.",
        "The product arrived damaged and you're not responding.",
        "Votre service client est une catastrophe.",
    ],
}


# Real customer queries to classify.
CASES = [
    "My package hasn't arrived after 2 weeks.",
    "Can I return this if I changed my mind?",
    "Does this come with batteries included?",
    "I've been waiting 3 days for a reply, this is ridiculous.",
    "Le produit est cassé, comment je le renvoie ?",
]


def main() -> None:
    print("Compiling custom catalog (5 intents × ~3 prompts)...")
    catalog: Catalog = compile_catalog(intents=INTENTS, language="multi")
    print(f"  → {len(catalog.anchors)} anchors, dim={catalog.dim_latent}")

    with EfeClient(host="127.0.0.1:50051", insecure=True) as client:
        decoder = HybridClassifier(client=client, catalog=catalog)
        print("\nClassifying customer queries:")
        for text in CASES:
            r = decoder.predict(text)
            print(f"  {text[:60]:60s} → {r.label or 'UNKNOWN':18s} ({r.confidence:.2f})")


if __name__ == "__main__":
    main()
```

LOC : ~55. Cible ≤ 100 ✅.

### 7.4 Example 3 — `examples/03_multilang_streaming.py`

```python
"""Sprint D10 — example 3: multilingual concurrent streaming.

Goal: process a stream of FR + EN messages concurrently with
back-pressure. Demonstrates:
  * AsyncEfeClient (asyncio path)
  * Multilingual anchor design (one catalog, mixed languages)
  * Bounded concurrency via asyncio.Semaphore
  * Per-message timing for SLA monitoring

Pre-requisites:
  * efe-server reachable on 127.0.0.1:50051 with
    EFE_IDEMPOTENCY_TTL_MS=0 (so concurrency stress is real,
    not cache-served — see d9_deep_analysis_supplement.md A2)
  * pip install efe-tool==0.1.9
  * Python 3.10+

Run:
  python examples/03_multilang_streaming.py
"""
import asyncio
import time
from typing import AsyncIterator

from efe_tool import AsyncEfeClient, HybridClassifier, load_builtin_catalog


async def fake_stream() -> AsyncIterator[str]:
    """A 20-message synthetic stream — half FR, half EN."""
    messages = [
        "I'd like to cancel my subscription.",
        "Pourquoi ai-je été facturé deux fois ?",
        "How do I update my payment method?",
        "Je n'arrive pas à me connecter à mon compte.",
        "The delivery is 5 days late, what's going on?",
        "Ma commande est marquée livrée mais je n'ai rien reçu.",
        "Can I extend the return window?",
        "Quelles sont les conditions de remboursement ?",
    ] * 3   # 24 messages total
    for m in messages:
        yield m
        await asyncio.sleep(0.05)   # simulated upstream pacing


async def classify_one(
    decoder: HybridClassifier,
    text: str,
    sem: asyncio.Semaphore,
) -> tuple[str, str | None, float, float]:
    async with sem:    # bound concurrency
        t0 = time.monotonic()
        r = await decoder.apredict(text)   # async variant of .predict
        elapsed_ms = (time.monotonic() - t0) * 1000
    return text, r.label, r.confidence, elapsed_ms


async def main() -> None:
    catalog = load_builtin_catalog("support_triage")
    sem = asyncio.Semaphore(8)   # ≤ 8 in-flight RPCs (back-pressure)

    async with AsyncEfeClient(host="127.0.0.1:50051", insecure=True) as client:
        decoder = HybridClassifier(client=client, catalog=catalog)

        async for text in fake_stream():
            asyncio.create_task(classify_one(decoder, text, sem))
        await asyncio.sleep(2.0)   # let in-flight tasks drain

        # Note: a real consumer would await asyncio.gather(*tasks);
        # here we just demo the back-pressure pattern.


if __name__ == "__main__":
    asyncio.run(main())
```

LOC : ~70. Cible ≤ 100 ✅.

### 7.5 Example 4 — `examples/04_production_client.py`

```python
"""Sprint D10 — example 4: production-grade client.

Goal: production wiring — auth, retries, idempotency, structured
logging, OpenTelemetry trace propagation. The "what you would
deploy in prod" template.

Demonstrates:
  * JWT token from env var
  * Exponential backoff with jitter (3 retries on UNAVAILABLE)
  * Per-request idempotency key
  * OTel trace context propagation
  * Structured logging with fields
  * Graceful shutdown on SIGTERM/SIGINT

Pre-requisites:
  * efe-server reachable on 127.0.0.1:50051 with auth ENABLED
  * EFE_TOKEN env var set
  * pip install efe-tool[otel]==0.1.9

Run:
  EFE_TOKEN=$(cat /tmp/jwt) python examples/04_production_client.py
"""
import logging
import os
import random
import signal
import sys
import time
import uuid
from contextlib import contextmanager

from efe_tool import (
    EfeClient, HybridClassifier, load_builtin_catalog,
    EfeAuthError, EfeRateLimitError, EfeUnavailableError,
)
# Optional OTel — only loaded if installed.
try:
    from opentelemetry import trace
    from opentelemetry.sdk.trace import TracerProvider
    from opentelemetry.sdk.trace.export import (
        BatchSpanProcessor, ConsoleSpanExporter,
    )
    trace.set_tracer_provider(TracerProvider())
    trace.get_tracer_provider().add_span_processor(
        BatchSpanProcessor(ConsoleSpanExporter()))
    tracer = trace.get_tracer("examples.04_production_client")
except ImportError:
    tracer = None


logging.basicConfig(
    level=logging.INFO,
    format='{"ts":"%(asctime)s","level":"%(levelname)s","msg":"%(message)s"}',
)
log = logging.getLogger(__name__)


@contextmanager
def maybe_span(name: str):
    if tracer is None:
        yield None
        return
    with tracer.start_as_current_span(name) as span:
        yield span


def classify_with_retries(
    decoder: HybridClassifier,
    text: str,
    *,
    max_attempts: int = 3,
    base_backoff_s: float = 0.2,
):
    """Classify with exponential backoff + jitter on retryable errors."""
    request_id = str(uuid.uuid4())
    for attempt in range(1, max_attempts + 1):
        with maybe_span(f"classify.attempt.{attempt}"):
            try:
                return decoder.predict(text, request_id=request_id)
            except EfeRateLimitError as exc:
                if attempt == max_attempts:
                    raise
                sleep = base_backoff_s * 2 ** (attempt - 1)
                sleep += random.uniform(0, sleep / 2)
                log.warning(
                    "rate-limited; retrying in %.2fs (attempt %d/%d)",
                    sleep, attempt, max_attempts,
                )
                time.sleep(sleep)
            except EfeUnavailableError as exc:
                if attempt == max_attempts:
                    raise
                sleep = base_backoff_s * 2 ** (attempt - 1)
                log.warning(
                    "transient error %s; retrying in %.2fs",
                    exc.grpc_code, sleep,
                )
                time.sleep(sleep)


_shutdown = False


def _on_signal(*args):
    global _shutdown
    _shutdown = True


signal.signal(signal.SIGTERM, _on_signal)
signal.signal(signal.SIGINT, _on_signal)


def main() -> int:
    token = os.environ.get("EFE_TOKEN")
    if not token:
        log.error("EFE_TOKEN env var required")
        return 2

    catalog = load_builtin_catalog("support_triage")

    with EfeClient(
        host=os.environ.get("EFE_HOST", "127.0.0.1:50051"),
        insecure=os.environ.get("EFE_INSECURE", "false") == "true",
        token=token,
        timeout_s=10.0,
    ) as client:
        decoder = HybridClassifier(client=client, catalog=catalog)
        cases = ["Someone charged my card.", "Where is my refund?"]
        while not _shutdown:
            for text in cases:
                try:
                    r = classify_with_retries(decoder, text)
                except EfeAuthError:
                    log.error("auth failed — token expired or revoked")
                    return 3
                except Exception as exc:
                    log.exception("classification failed: %s", exc)
                    continue
                log.info(
                    'classified label=%s confidence=%.2f text=%r',
                    r.label, r.confidence, text,
                )
            time.sleep(2)
        log.info("graceful shutdown")
        return 0


if __name__ == "__main__":
    sys.exit(main())
```

LOC : ~115. Légèrement au-dessus de 100, car la production a une
verbeuse irréductible (auth + retries + signaux + logs + OTel).
Acceptable — c'est l'exemple "advanced" et la LOC reflète la
complexité production réelle.

### 7.6 Tests `examples/tests/test_examples_smoke.py`

```python
"""Sprint D10 — example smoke tests.

Each example MUST `python examples/NN_…py` and exit 0 against a
stub gRPC server that returns canned responses. The smoke is run
in CI as part of `docs.yml` workflow.

Invariants checked per example:
  1. Exit code 0
  2. Stdout contains expected label substrings
  3. Wallclock < 30 s
  4. No DeprecationWarning emitted (catches regressions in
     deprecated symbols accidentally being used)
  5. No `efe_tool._internal.*` imports (would be a leak of private
     surface)

Stub server: `tests/stub_server.py` — implements `Execute` to
return a fixed ExecuteResponse based on the request_id namespace.
"""
import importlib
import os
import subprocess
import sys
import time
from pathlib import Path

import pytest

EXAMPLES_DIR = Path(__file__).resolve().parents[1]
ROOT = EXAMPLES_DIR.parent

EXAMPLES = [
    ("01_quickstart_classify.py", ["fraud", "question", "bug"]),
    ("02_custom_catalog.py", ["track_order", "request_refund"]),
    ("03_multilang_streaming.py", []),  # stream — no fixed labels expected
    ("04_production_client.py", []),    # requires EFE_TOKEN; tested separately
]


@pytest.fixture(scope="module")
def stub_server():
    proc = subprocess.Popen(
        [sys.executable, str(ROOT / "tests" / "stub_server.py")],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    # Wait for bind…
    for _ in range(30):
        if proc.poll() is not None:
            raise RuntimeError("stub server died")
        try:
            import socket
            s = socket.socket()
            s.settimeout(0.5)
            s.connect(("127.0.0.1", 50051))
            s.close()
            break
        except OSError:
            time.sleep(0.2)
    yield
    proc.terminate()
    proc.wait(timeout=5)


@pytest.mark.parametrize("script,expected_labels", EXAMPLES[:3])
def test_example_runs_and_outputs(stub_server, script, expected_labels):
    p = subprocess.run(
        [sys.executable, str(EXAMPLES_DIR / script)],
        capture_output=True, text=True, timeout=30,
    )
    assert p.returncode == 0, f"{script} failed:\n{p.stderr}"
    if expected_labels:
        for label in expected_labels:
            assert label in p.stdout, f"expected label {label!r} in stdout"


def test_example_04_requires_token():
    """Example 04 must bail with rc=2 if EFE_TOKEN not set."""
    env = {k: v for k, v in os.environ.items() if k != "EFE_TOKEN"}
    p = subprocess.run(
        [sys.executable, str(EXAMPLES_DIR / "04_production_client.py")],
        capture_output=True, text=True, timeout=10, env=env,
    )
    assert p.returncode == 2


def test_no_internal_imports_in_examples():
    """Examples MUST NOT import from efe_tool._internal — that's
    the marker of a private leak. Static check via grep."""
    import re
    pat = re.compile(r"from\s+efe_tool\._\w+|import\s+efe_tool\._")
    for f in EXAMPLES_DIR.glob("*.py"):
        text = f.read_text()
        match = pat.search(text)
        assert not match, (
            f"{f.name} imports private surface: {match.group(0)}"
        )
```

---

## 8. Publication — PyPI (OIDC) + docs.efe.ai (GitHub Pages)

### 8.1 PyPI publication via OIDC trusted publisher

Pas de token statique en secret. Utiliser PyPI's [trusted
publisher](https://docs.pypi.org/trusted-publishers/) feature :

* Sur pypi.org : déclarer le repo `efe-ai/efe`, workflow
  `release.yml`, environment `pypi` comme trusted.
* Dans `release.yml` : un job `publish-pypi` qui fait
  `pypa/gh-action-pypi-publish` — celui-ci échange l'OIDC token
  GitHub contre un token PyPI éphémère, sans aucun secret côté
  repo.

```yaml
# .github/workflows/release.yml — addition D10
  publish-pypi:
    name: Publish efe-tool to PyPI
    needs: build-sign-publish
    runs-on: ubuntu-22.04
    environment: pypi      # required for OIDC trusted publisher
    permissions:
      id-token: write
      contents: read
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }
      - name: Build sdist + wheel
        run: |
          cd clients/python
          pip install --upgrade build
          python -m build
      - name: Publish to PyPI
        uses: pypa/gh-action-pypi-publish@release/v1
        with:
          packages-dir: clients/python/dist
          # Skip the existing-version check on tag re-runs
          skip-existing: true
```

### 8.2 GitHub Pages docs deployment

```yaml
# .github/workflows/docs.yml — D10 NEW
name: docs
on:
  push:
    branches: [main]
    paths:
      - "docs/**"
      - "clients/python/efe_tool/**"
      - ".github/workflows/docs.yml"
  pull_request:
    paths:
      - "docs/**"
  workflow_dispatch:

permissions:
  contents: write          # for gh-pages push (mike)
  pages: write
  id-token: write

jobs:
  build-deploy:
    runs-on: ubuntu-22.04
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }   # mike needs full history

      - uses: actions/setup-python@v5
        with: { python-version: "3.11" }

      - name: Install docs deps
        run: pip install -r docs/requirements-docs.txt

      - name: Install efe-tool (for mkdocstrings autodoc)
        run: pip install -e clients/python

      - name: Build (strict)
        run: |
          cd docs
          mkdocs build --strict
          # `--strict` fails on warnings (broken refs, missing
          # docs, etc.)

      - name: Link checker
        run: |
          pip install linkchecker
          linkchecker --check-extern --no-warnings docs/site/

      - name: Deploy (push/main only)
        if: github.event_name == 'push'
        run: |
          cd docs
          mike deploy --push --update-aliases dev
```

Sur tag (release.yml) :
```yaml
      - name: Deploy docs (versioned)
        run: |
          cd docs
          mike deploy --push --update-aliases ${{ steps.tag.outputs.semver }} latest
          mike set-default --push latest
```

### 8.3 docs.efe.ai DNS

* Domaine `efe.ai` détenu par l'équipe.
* CNAME `docs` → `efe-ai.github.io`.
* `docs/CNAME` file dans la branche `gh-pages` (mike gère).
* HTTPS automatique via GitHub Pages (Let's Encrypt).

---

## 9. 12 décisions de conception clés

| # | Décision | Alternative rejetée | Justification |
|--:|----------|---------------------|---------------|
| 1 | mkdocs-material + mkdocstrings | sphinx-autodoc + RTD theme | Repo déjà sur mkdocs ; markdown-first vs RST ; meilleur preview live |
| 2 | Diátaxis framework | Linear "user guide" | Sépare 4 modes de lecture ; standard industrie 2024+ |
| 3 | Examples in `examples/` (root) | `clients/python/examples/` | IDE-discoverable ; convention Python idiomatic |
| 4 | 4 examples, .py only (pas Jupyter) | Jupyter notebooks | Testable en CI, lintable, diffable, version-controllable |
| 5 | PyPI publication via OIDC | API token static secret | Pas de secret à rotation ; standard PyPI 2024 |
| 6 | mike for versioned docs | Single trunk | Partner sur v0.1.9 voit la doc v0.1.9, pas le trunk |
| 7 | Async API parity (sync still default) | Async-only OR sync-only | Meet users where they are ; FastAPI users veulent async |
| 8 | py.typed marker (PEP 561) | Pas de stubs | IDE auto-complete = critical UX ; coût marginal |
| 9 | `_internal/` + `_async/` private | Public flat | Permet refactor interne sans bump major ; convention sklearn |
| 10 | gRPC code → Python exception hierarchy | grpc.RpcError naked | Catch typé ; messages humains ; retry logic standardisée |
| 11 | Snippets pymdownx (zero-drift code↔doc) | Hand-copied code blocks | Garantit que la doc montre du code testé en CI |
| 12 | examples/tests/ smoke en CI gating | No CI gate | Détecte `import efe_tool._internal` accidentels avant merge |

---

## 10. 10 risques identifiés et mitigations

| # | Risque | Sévérité | Probabilité | Mitigation |
|--:|--------|:--------:|:-----------:|------------|
| 1 | Doc drift entre code et docs | HIGH | HIGH | snippets pymdownx + `mkdocs build --strict` + test_d10_api_reference_coverage |
| 2 | Examples se cassent au minor bump | HIGH | MED | examples pinent `efe-tool==0.1.9` ; test smoke gated en CI |
| 3 | mkdocstrings extrait mauvaises signatures | MED | LOW | Pin version exacte dans requirements-docs.txt ; nightly test re-render |
| 4 | docs.efe.ai DNS pas provisionné | MED | MED | Fallback `efe-ai.github.io/efe` documenté ; CNAME quand DNS prêt |
| 5 | PyPI OIDC trusted publisher pas accepté | HIGH | LOW | Fallback : `PYPI_API_TOKEN` secret avec rotation 90j tracée |
| 6 | Partner derrière proxy corp ne peut pas pip install | HIGH | MED | Doc inclut alternate via wheel téléchargé manuellement |
| 7 | Quickstart fail car helm pas installé | LOW | HIGH | Pre-flight check + doc minikube install |
| 8 | SDK breaking changes inadvertis | HIGH | MED | `_internal/` + tests `test_d10_public_api_surface_pinned.py` |
| 9 | Examples leak données sensibles si copy-paste tel quel | HIGH | LOW | Tous les exemples utilisent données fictives + warnings dans docstring |
| 10 | Markdown link rot (sites externes) | MED | HIGH | linkchecker `--no-warnings` mode ; nightly run alerte sur 404 |

---

## 11. Plan jour-par-jour (4 j wallclock)

### D10.1 (½ j) — SDK API surface freeze
* Ajouter `_version.py`, `__all__` complet, `py.typed`.
* Migrer modules privés sous `_internal/`.
* Créer `_async/client.py` (squelette).
* Test `test_d10_public_api_surface_pinned.py` : fail si `__all__`
  change sans bump.

### D10.2 (1 j) — mkdocs setup + Diátaxis structure
* Activer `mkdocstrings` + `mike` dans mkdocs.yml.
* Créer arborescence `docs/docs/{tutorial,howto,reference,explanation}`.
* Squelettes vides pour chaque page.
* Tutorial / quickstart.md → version testable end-to-end.
* Reference / api.md → mkdocstrings auto.
* Explanation / why-anchors.md, hybrid.md → narratif (~2 KB
  chacune).

### D10.3 (1 j) — Authoring 4 exemples + smoke tests
* `examples/01_quickstart_classify.py` → 25 LOC.
* `examples/02_custom_catalog.py` → ~55 LOC.
* `examples/03_multilang_streaming.py` → ~70 LOC.
* `examples/04_production_client.py` → ~115 LOC.
* `examples/tests/test_examples_smoke.py` → 6 tests.
* `tests/stub_server.py` → minimal Mode-1 stub backend.

### D10.4 (½ j) — Quickstart polish + CI workflow `docs.yml`
* Réécrire `docs/operations/quickstart.md` pour aligner sur
  `pip install efe-tool` + linkage vers tutorial.
* `.github/workflows/docs.yml` (build + linkcheck + push gh-pages).
* `.github/workflows/release.yml` étendu : publish-pypi + mike
  deploy versioned.

### D10.5 (½ j) — Validation manuelle DX
* Script `validate_quickstart.sh` (cf. §6.4).
* 3 reviewers externes exécutent → reportent ELAPSED.
* Si > 10 min sur ≥ 1 reviewer → backlog DX-D10.X (priorisé en
  D11).

### D10.6 (½ j) — Audit + closure D10
* `docs/audits/d10_dx_report.md` (skeleton + chiffres).
* CHANGELOG.md entrée v0.1.10.
* `BENCHMARKS.md § 12` (D10 protocols).
* `bench_baselines/v0.1.10/baseline.json` placeholder avec D10
  SLOs SKIPPED (mesurés post-merge).

> **Réalité observée** : 4 j wallclock condensés. Le sprint clôt
> en J4 avec sign-off `docs/audits/d10_dx_report.md`.

---

## 12. SLOs ajoutés post-D10

### 12.1 Liste des 5 nouveaux SLOs

| ID | Métrique | Comparator | Cible | Display | Source |
|----|----------|:----------:|------:|:-------:|--------|
| `D10-DOC-1` | `docs::build_strict_warnings` | `lte` | 0 | count | `mkdocs build --strict` exit 0 |
| `D10-DOC-2` | `docs::api_coverage_ratio` | `gte` | 0.95 | ratio | `tests/test_d10_api_reference_coverage.py` |
| `D10-DOC-3` | `docs::link_check_404` | `lte` | 0 | count | `linkchecker` |
| `D10-DX-1` | `dx::quickstart_wallclock_s` | `lte` | 600 | s | `validate_quickstart.sh` (3 reviewers median) |
| `D10-DX-2` | `dx::examples_smoke_aggregate_s` | `lte` | 30 | s | `pytest examples/tests/` |

### 12.2 Format dans `bench_baselines/v0.1.10/baseline.json`

```json
{
  "version": "1.0",
  "timestamp": "PENDING_FIRST_GREEN_RUN",
  "results": {},
  "slo_compliance": {
    "SLO-D10-DOC-1": {
      "name": "Docs build strict — zero warnings",
      "target_ns": 0,
      "comparator": "lte",
      "skipped": true
    },
    "SLO-D10-DOC-2": {
      "name": "API reference symbol coverage",
      "target_ns": 950000000,
      "comparator": "gte",
      "display_as": "ratio",
      "skipped": true
    },
    "SLO-D10-DOC-3": {
      "name": "Markdown link checker — zero 404",
      "target_ns": 0,
      "comparator": "lte",
      "skipped": true
    },
    "SLO-D10-DX-1": {
      "name": "Quickstart wallclock (median 3 reviewers)",
      "target_ns": 600000000000,
      "comparator": "lte",
      "skipped": true
    },
    "SLO-D10-DX-2": {
      "name": "Example smoke aggregate runtime",
      "target_ns": 30000000000,
      "comparator": "lte",
      "skipped": true
    }
  }
}
```

### 12.3 Disjoint-key invariant (étendu)

Nouveaux préfixes ajoutés à
`bench/load/parse_ghz_report.py::ACCURACY_PREFIXES` (qui devient
`RESERVED_PREFIXES`) :

```python
RESERVED_PREFIXES = (
    "eval_accuracy::",
    "eval_latency::",
    "load_",
    "soak::",
    "docs::",   # D10 NEW
    "dx::",     # D10 NEW
)
```

Test régression `test_d10_disjoint_keys.py` :
* `dx::*` jamais émis par `parse_ghz_report.py` ni `leak_detector.py`.
* `docs::*` jamais émis hors `docs.yml` workflow.

---

## 13. Anti-régression & realism check

### 13.1 Tests SLO antérieurs (héritage D6/D7/D8/D9)

Tous les 21 SLOs déjà gatés (D4-RPC × 4, D6-ACC × 4, D6-LAT × 1,
D7-MC × 4, D8-HYB × 4, D8-LAT × 1, D9-LOAD × 4, D9-SOAK × 1)
restent **gated**. Aucun changement de seuils.

### 13.2 Tests suite Python

```bash
pytest \
  bench/load/tests/ \
  bench/soak/tests/ \
  scripts/tests/test_d{6,7,8,9}_*.py \
  scripts/tests/test_d10_*.py \
  examples/tests/ \
  clients/python/tests/
```

Total expected post-D10 : **117 (D8+D9) + 6 (D10 examples smoke) +
3 (D10 surface freeze) + 4 (D10 doc coverage) + 2 (D10 disjoint
keys) = 132 tests verts**.

### 13.3 Tests workspace Rust

```bash
cargo test --workspace
```

Aucun changement Rust en D10 → aucun risque de régression côté
serveur.

### 13.4 Vérification tag → publish round-trip

Un tag de test `v0.1.10-rc1` doit déclencher :
* Build image OK.
* Sign cosign OK.
* Trivy scan OK (0 CRITICAL).
* Publish chart Helm OK.
* Publish efe-tool sur TestPyPI (pas PyPI prod) OK.
* `mike deploy 0.1.10-rc1 --update-aliases dev` OK.

`v0.1.10` puis lance le vrai pipeline.

---

## 14. Intégration CI : workflow `docs.yml` + example smoke

### 14.1 `.github/workflows/docs.yml` (cf. §8.2)

3 jobs :
* **build-deploy** : mkdocs build strict, link check, mike deploy
  (push main only).
* **example-smoke** : python -m pytest examples/tests/ avec stub
  server.
* **api-coverage** : test_d10_api_reference_coverage.py (asserte
  que ≥ 95 % des symboles `__all__` ont une entrée dans la
  référence rendue).

### 14.2 Étendre `.github/workflows/release.yml`

* Add job `publish-pypi` après build-sign-publish.
* Add step `mike deploy <semver> --update-aliases latest` après
  `helm push`.

### 14.3 Étendre `.github/workflows/bench.yml`

* Add `bench-dx` job — execute `validate_quickstart.sh` post-merge
  vers main contre un minikube éphémère (kind action).
* Job `continue-on-error: true` pendant 3 runs verts puis flip
  fail-closed.

---

## 15. Critères d'acceptation (DoD)

### 15.1 Code

- [ ] `clients/python/efe_tool/__init__.py` exporte le `__all__`
  freezé (D10.1).
- [ ] `_async/client.py` livré + parité fonctionnelle vs sync.
- [ ] `_internal/` réorganisation OK ; aucun import privé dans
  `examples/`.
- [ ] `py.typed` marker présent ; mypy strict mode passe.
- [ ] Hiérarchie d'erreurs `EfeAuthError`, etc. + raise_from_grpc.

### 15.2 Documentation

- [ ] `docs/docs/{tutorial,howto,reference,explanation}/` — chaque
  catégorie a ≥ 2 pages.
- [ ] `mkdocs build --strict` exit 0 (D10-DOC-1).
- [ ] API reference auto-générée couvre ≥ 95 % de `__all__`
  (D10-DOC-2).
- [ ] Link check 0 broken (D10-DOC-3).
- [ ] Snippets pymdownx utilisés pour tout bloc python ≥ 3 lignes.
- [ ] mike versioning fonctionnel ; `docs.efe.ai/0.1.10/` accessible.

### 15.3 Examples

- [ ] 4 fichiers `examples/0X_*.py` livrés.
- [ ] Tous ≤ 100 LOC (sauf 04 prod-grade ≤ 120 LOC).
- [ ] `examples/tests/test_examples_smoke.py` — 6 tests verts.
- [ ] Aggregate exec < 30 s (D10-DX-2).
- [ ] Aucun import `efe_tool._internal.*`.

### 15.4 Quickstart

- [ ] `docs/docs/tutorial/quickstart.md` réécrit, lié à example 01.
- [ ] `validate_quickstart.sh` versionné dans le repo.
- [ ] 3 reviewers externes ont exécuté : tous < 10 min wallclock
  (D10-DX-1, mesure post-merge).

### 15.5 Publication

- [ ] PyPI OIDC trusted publisher configuré (PR séparée).
- [ ] Tag `v0.1.10` déclenche : push image, push chart, push
  PyPI, push docs versioned.
- [ ] `pip install efe-tool==0.1.10` retourne le wheel correct
  sur Python 3.10/3.11/3.12.
- [ ] `https://docs.efe.ai/0.1.10/` accessible avec navigation
  Diátaxis complète.

### 15.6 CI

- [ ] `.github/workflows/docs.yml` actif sur main + PR.
- [ ] `.github/workflows/release.yml` étendu (publish-pypi).
- [ ] Aucun des 21 SLO antérieurs régressé.

### 15.7 Documentation interne

- [ ] `BENCHMARKS.md § 12` documente D10 protocols.
- [ ] `bench_baselines/v0.1.10/baseline.json` créé avec D10 SLOs
  SKIPPED.
- [ ] `docs/audits/d10_dx_report.md` skeleton créé.
- [ ] `CHANGELOG.md` entrée Unreleased D10.

---

## 16. Out of scope & roadmap follow-ups

### 16.1 Hors scope D10

- ❌ **SDK Go** — Phase 3 (D15+).
- ❌ **SDK TypeScript** — Phase 3.
- ❌ **Multi-tenant doc views** (per-partner docs) — D11+.
- ❌ **Interactive playground in docs** (live sandbox) — D14+.
- ❌ **Video tutorials** (out-of-band, Loom + YouTube) — Marketing
  responsibility, not engineering.
- ❌ **Translation** (i18n des docs en français + autres) — D14+.
- ❌ **Search analytics** (Algolia DocSearch) — nice-to-have, post-D14.

### 16.2 Roadmap après D10

| Sprint | Cible | Justification |
|--------|-------|---------------|
| **D10.bis** | Reviewer feedback resolution | 3 partenaires testent, on fixe les 3 gros pain points |
| **D11** | Multi-tenant resource quotas | Phase 2 : un cluster pour N partenaires |
| **D12** | Catalog admin endpoints | Hot-reload prod sans redeploy |
| **D13** | Grafana dashboards | Production observability tier-2 |
| **D14** | Production cutover (premier client payant) | Sortie Pilot 1.0 |
| **D15** | SDK Go (officiel) | Phase 3 SaaS |

### 16.3 Métrique de succès post-D10

À 30 jours après merge D10 :

> *« Au moins 1 partenaire pilote a installé `efe-tool` via PyPI
> sur sa machine de dev, exécuté l'exemple 01 contre son cluster
> en moins de 10 minutes (vérifié par log Slack), et a écrit son
> propre exemple `02_*` adapté à son catalogue. La doc
> `docs.efe.ai/0.1.10/` reçoit ≥ 100 visites uniques externes.
> Le `mkdocs build --strict` reste vert sur 100 % des PRs main
> depuis le merge. »*

---

## 17. Annexes

### 17.1 Annexe A — `pyproject.toml` cible

```toml
[build-system]
requires = ["setuptools>=68", "wheel"]
build-backend = "setuptools.build_meta"

[project]
name = "efe-tool"
version = "0.1.10"
description = "EFE — Ephemeral Fluid Swarm Python SDK"
readme = "README.md"
license = {file = "LICENSE"}
authors = [{name = "EFE Team", email = "team@efe.ai"}]
requires-python = ">=3.10"
classifiers = [
    "Development Status :: 4 - Beta",
    "Intended Audience :: Developers",
    "License :: OSI Approved :: Apache Software License",
    "Operating System :: POSIX :: Linux",
    "Operating System :: MacOS",
    "Programming Language :: Python :: 3",
    "Programming Language :: Python :: 3.10",
    "Programming Language :: Python :: 3.11",
    "Programming Language :: Python :: 3.12",
    "Topic :: Scientific/Engineering :: Artificial Intelligence",
]
dependencies = [
    "grpcio>=1.62",
    "grpcio-tools>=1.62",
    "protobuf>=4.25",
    "numpy>=1.26",
    "sentence-transformers>=2.7",  # for embedder
    "pyyaml>=6.0",
    "click>=8.1",                  # CLI
]

[project.optional-dependencies]
otel = [
    "opentelemetry-api>=1.24",
    "opentelemetry-sdk>=1.24",
    "opentelemetry-exporter-otlp-proto-grpc>=1.24",
]
dev = [
    "pytest>=8.0",
    "pytest-asyncio>=0.23",
    "mypy>=1.8",
    "ruff>=0.4",
]

[project.urls]
Homepage = "https://efe.ai"
Documentation = "https://docs.efe.ai"
Repository = "https://github.com/efe-ai/efe"
Changelog = "https://docs.efe.ai/latest/changelog/"
Issues = "https://github.com/efe-ai/efe/issues"

[project.scripts]
efe-tool = "efe_tool.cli:main"

[tool.setuptools.packages.find]
where = ["."]
include = ["efe_tool*"]

[tool.setuptools.package-data]
efe_tool = ["py.typed", "*.pyi", "scenarios/builtin/*.yaml"]

[tool.mypy]
strict = true
ignore_missing_imports = true
files = ["efe_tool"]

[tool.ruff]
target-version = "py310"
line-length = 100
```

### 17.2 Annexe B — `docs/requirements-docs.txt`

```
mkdocs==1.6.1
mkdocs-material==9.5.42
mkdocstrings[python]==0.27.0
mike==2.1.3
linkchecker==10.5.0
pymdown-extensions==10.12
```

### 17.3 Annexe C — Cheat-sheet partner pilot DX

```
# Install
pip install efe-tool==0.1.10

# One-shot quickstart against in-cluster service
helm install efe oci://ghcr.io/efe-project/charts/efe --version 0.1.10 \
  -n efe --create-namespace --set service.headless=true
kubectl wait deploy/efe -n efe --for=condition=available --timeout=120s
kubectl port-forward -n efe svc/efe 50051:50051 &
python examples/01_quickstart_classify.py

# Custom catalog
$EDITOR examples/02_custom_catalog.py     # adapt INTENTS to your domain
python examples/02_custom_catalog.py

# Production wiring
EFE_TOKEN=$(cat /tmp/jwt) python examples/04_production_client.py

# Docs
open https://docs.efe.ai/0.1.10/
```

### 17.4 Annexe D — Diátaxis quick reference

| Si tu cherches… | Va voir… |
|----------------|----------|
| « comment je commence ? » | `tutorial/quickstart.md` |
| « comment je fais X ? » | `howto/X.md` |
| « quels sont les arguments de Y ? » | `reference/api.md` |
| « pourquoi EFE marche comme ça ? » | `explanation/architecture.md` |

### 17.5 Annexe E — Checklist d'exécution architect → impl

```
□ D10.1.1  efe_tool/_version.py + __all__ + py.typed
□ D10.1.2  _internal/ migration des modules privés
□ D10.1.3  _async/client.py (squelette + tests parity)
□ D10.1.4  errors.py — hiérarchie + raise_from_grpc
□ D10.1.5  pyproject.toml metadata + classifiers
□ D10.1.6  test_d10_public_api_surface_pinned.py

□ D10.2.1  mkdocs.yml — mkdocstrings, mike, snippets
□ D10.2.2  docs/docs/{tutorial,howto,reference,explanation}/
□ D10.2.3  Tutorial × 3 pages (quickstart, concepts, your-first-classifier)
□ D10.2.4  How-to × 7 pages
□ D10.2.5  Reference × 5 pages (api auto-mkdocstrings)
□ D10.2.6  Explanation × 4 pages
□ D10.2.7  test_d10_api_reference_coverage.py

□ D10.3.1  examples/01_quickstart_classify.py + LOC≤30
□ D10.3.2  examples/02_custom_catalog.py + LOC≤100
□ D10.3.3  examples/03_multilang_streaming.py + LOC≤100
□ D10.3.4  examples/04_production_client.py + LOC≤120
□ D10.3.5  examples/README.md (index)
□ D10.3.6  examples/tests/test_examples_smoke.py (6 tests)
□ D10.3.7  tests/stub_server.py (Mode-1 stub)

□ D10.4.1  docs/docs/tutorial/quickstart.md (réécriture)
□ D10.4.2  docs/operations/quickstart.md (mise à jour)
□ D10.4.3  validate_quickstart.sh

□ D10.5.1  .github/workflows/docs.yml
□ D10.5.2  .github/workflows/release.yml — publish-pypi job
□ D10.5.3  .github/workflows/release.yml — mike deploy step
□ D10.5.4  3 reviewers externes (manual)

□ D10.6.1  bench_baselines/v0.1.10/baseline.json (D10 SLOs SKIPPED)
□ D10.6.2  BENCHMARKS.md § 12
□ D10.6.3  docs/audits/d10_dx_report.md (skeleton)
□ D10.6.4  CHANGELOG.md Unreleased D10

□ Validation finale
  □ helm install efe → port-forward → pip install → python examples/01_…
  □ < 10 min wallclock (3 reviewers)
  □ docs.efe.ai/0.1.10/ accessible (post-tag)
  □ All 132 tests PASS
  □ All 21 SLOs antérieurs gated
  □ release.yml dry-run sur tag v0.1.10-rc1
```

### 17.6 Annexe F — Pourquoi 4 exemples (et pas 3 ou 5)

3 exemples laissent un trou dans la couverture intermédiaire :

| Couverture | 1 ex. | 2 ex. | 3 ex. | **4 ex.** | 5 ex. |
|-----------|:-----:|:-----:|:-----:|:---------:|:-----:|
| Beginner copy-paste | ✓ | ✓ | ✓ | ✓ | ✓ |
| Custom data | | ✓ | ✓ | ✓ | ✓ |
| Async / concurrent | | | ✓ | ✓ | ✓ |
| Production-grade | | | | ✓ | ✓ |
| Streaming RPC | | | | | ✓ |
| LOC budget acceptable | | | ✓ | ✓ | ✗ (trop) |
| CI smoke time < 30 s | ✓ | ✓ | ✓ | ✓ | ✗ |

5ᵉ exemple "streaming RPC" est intéressant mais Mode 1 actuel ne
streame pas vraiment (D10 = Letter B follow-up). On ajoutera en
D10.bis ou D11.

### 17.7 Annexe G — Frontière D10 ↔ D11+

D10 livre :
* SDK Python publié ✅
* Docs Diátaxis publiés ✅
* 4 exemples ✅
* Quickstart end-to-end ✅

D10 NE livre PAS :
* SDK Go / TS (D15+)
* Translation FR (D14+)
* Sandbox interactive (D14+)
* Multi-tenant doc views (D11)
* Mode 1 streaming (Letter B follow-up, D11)

---

**Fin de l'architecture D10.** Le sprinter peut implémenter en
suivant Annexe E, en revenant à §4 pour la surface SDK, §5 pour
la doc, §7 pour les exemples, §8 pour la publication.

> **Approbation requise avant implémentation** :
> - [ ] Lead DX (revue Diátaxis structure + 4 examples sélection)
> - [ ] Lead doc (revue mkdocs.yml + mike workflow)
> - [ ] Lead sécu (revue PyPI OIDC trusted publisher)
> - [ ] Owner produit (revue quickstart partner persona)
