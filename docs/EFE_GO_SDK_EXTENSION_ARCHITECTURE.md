# `efe-sdk-go` (Go SDK) — Architecture détaillée de l'extension M9

> **Statut** : Draft v1.0.0 — Document de référence pour la
> **création from-scratch** du module **M9 = `clients/go/efe-sdk/`**.
> **Greenfield** : ce module n'existe PAS encore (✓ vérifié AS-IS au
> 2026-04-29 ; `clients/` ne contient que `python/`).
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-29.
>
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §6.4 — sprint D23
>     dans Φ-B, et §15 — surface SDK multi-runtime.
>   * `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` — pattern de
>     référence pour le SDK Python (M8) ; le Go SDK suit la même
>     surface conceptuelle (`Verdict`, `EfeClient`, `Classify`)
>     adaptée aux idiomes Go (context, options, errors).
>   * `LETTER_E_ARCHITECTURE.md` — service backend single-tenant que
>     le SDK consomme (via gRPC, post-D23).
>   * `MODE1_ARCHITECTURE.md` — contrat Mode-1 (la classification que
>     ce SDK appelle).
>   * `protos/efe/v1/*.proto` — wire contract (efe.proto + types.proto
>     + catalog.proto + admin.proto + mode2.proto + mode3.proto).
>
> **Portée** : **1 sprint atomique D23** (Φ-B), 10 jours
> wallclock. Création complète d'un module Go publishable :
> client gRPC auto-généré + API idiomatique Go (context,
> options, errors typés) + OTel auto-instrumentation +
> Prometheus auto-export + retry exponentiel + auth JWT + mTLS
> + 3 exemples HTTP frameworks (gin / fiber / chi) + tests
> intégration via `bufconn` mock server + publication module
> privé puis public à GA.
>
> **Cible GA** : `github.com/efe-ai/efe-sdk-go@v1.0.0` packagé
> publiquement sur Go module proxy ; aligné semver-wise avec le
> produit (D23 = v0.3.0 privé, GA = v1.0.0 public).

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M9 (Φ-B)
§2. Position dans la topologie EFE post-M8
§3. État AS-IS au 2026-04-29 (greenfield)
§4. Définition opérationnelle de M9 v1.0 GA
§5. Principes directeurs (idiomes Go)
§6. Invariants structurels (layout module)
§7. Vue d'ensemble — sprint D23 unique
§8. Surface publique cible (API Go publishable)
§9. **Sprint D23 — module Go entier**
§10. Sub-système 1 — `EfeClient` + functional options pattern
§11. Sub-système 2 — gRPC code generation pipeline
§12. Sub-système 3 — Errors + retry + circuit breaker
§13. Sub-système 4 — OpenTelemetry auto-instrumentation
§14. Sub-système 5 — Prometheus metrics
§15. Sub-système 6 — Auth (JWT bearer + mTLS)
§16. Sub-système 7 — 3 framework examples (gin / fiber / chi)
§17. Sub-système 8 — Mock server (`bufconn` in-memory)
§18. Stratégie de tests
§19. SLO mapping (cold start + latence + throughput)
§20. Risques transverses + mitigations
§21. Volumétrie code attendue
§22. Plan d'implémentation J1-J10
§23. Versioning + publication (private → public à GA)
§24. Compliance + sécurité (supply-chain, redaction)
§25. Annexes — go.mod canonique
§26. Annexes — surface API Go complète
§27. Annexes — Makefile + scripts/
§28. Annexes — RACI + budget + Gantt
§29. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du repo à fin D22 (post-mTLS + JWT scopes
  Lettre E, v0.2.0).
* **TO-BE** = état cible défini par le sprint D23.
* Le sprint D23 est **dans la phase Φ-B** (Lettre E + multi-runtime
  SDKs). D21 (Dockerfile prod + AdminService) et D22 (mTLS + JWT
  scopes) sont des prérequis (Lettre E doit être consommable
  avant qu'on écrive un client).
* Les défauts résiduels suivent **DT-G.x** (Defect Trace, Go SDK).
* **Module Go** = `github.com/efe-ai/efe-sdk-go` à publier.
  Pendant le développement (avant publication PyPI/proxy), le
  path filesystem est `clients/go/efe-sdk/`.
* **Symboles `EfeClient`, `Verdict`, `ClientOption`** sont les noms
  publics canoniques (PascalCase Go).
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`efe-sdk-go`** | Le module Go `clients/go/efe-sdk/` qui héberge le SDK Go pour EFE. Module path : `github.com/efe-ai/efe-sdk-go`. AS-IS : 0 LOC (greenfield). |
| **gRPC client** | Le code généré par `protoc + protoc-gen-go-grpc` à partir de `protos/efe/v1/*.proto`. Vit dans `internal/proto/` (caché de l'API publique). |
| **`EfeClient`** | La struct publique qui wrappe le gRPC client + options (auth, OTel, retry). Construite via `NewClient(addr, opts...)`. |
| **`Verdict`** | La struct publique des résultats de classification (équivalent du `Verdict` Python `@dataclass(frozen=True)` de M8). |
| **`ClientOption`** | Functional option type — `type ClientOption func(*options) error`. Utilisé via `NewClient(addr, WithJWT(...), WithOTel(...), ...)`. |
| **`bufconn`** | gRPC in-memory transport pour tests sans réseau. Package `google.golang.org/grpc/test/bufconn`. Le mock server du SDK l'utilise. |
| **gin / fiber / chi** | Les 3 frameworks HTTP Go les plus populaires. Le sprint livre 1 exemple par framework, déployable en standalone. |
| **OTel** | OpenTelemetry — packages `go.opentelemetry.io/otel/{trace,metric}` + `otelgrpc` interceptor pour auto-instrumentation. |
| **Functional options** | Idiome Go canonical pour configurer une struct via flags variadics — alternative à un constructor avec 15 args ou à un struct config mutable. |
| **Module privé** | Module Go non-publié sur le module proxy public. Accès via `GOPRIVATE=github.com/efe-ai/*` env + auth (.netrc / SSH). |
| **Module public** | Module Go publié sur le proxy public (`proxy.golang.org`). Une fois v1.x.x tagué + pushed, immutable. |
| **DT-G.N** | Defect Trace, Go SDK — défaut résiduel ou différé tracé. |

### §0.3 Cibles plateformes

| Plateforme | Status v1.0 |
|------------|:-----------:|
| Linux x86_64 | ✅ obligatoire |
| Linux aarch64 | ✅ obligatoire |
| macOS arm64 | ✅ obligatoire (devs) |
| macOS x86_64 | ✅ obligatoire (CI) |
| Windows x86_64 | ⚠️ best-effort |

Aucun binaire pré-compilé (Go = source distribution via module
proxy ; chaque consommateur compile localement).

### §0.4 Cibles versions Go

| Go version | Status v1.0 |
|:----------:|:-----------:|
| 1.21 | ❌ refusé (manque generics matures + log/slog) |
| 1.22 | ✅ minimum (`go.mod` declares `go 1.22`) |
| 1.23 | ✅ |
| 1.24+ (post-2026) | ✅ best-effort |

**Pourquoi 1.22+** : `slog` stable + `for range int` + meilleurs
generics + `errors.Join` (multi-error wrapping).

### §0.5 Cibles fonctionnelles GA

```
GA v1.0 efe-sdk-go (M9) :
├── Module Go publishable
│   ├── github.com/efe-ai/efe-sdk-go (public proxy à GA)
│   ├── go 1.22+
│   ├── ~17 fichiers source actifs
│   └── ~3 000 LOC Go généré (gRPC stubs)
│
├── API publique surface
│   ├── EfeClient                          (struct, opaque)
│   ├── NewClient(addr, opts...) (Client, error)
│   ├── (c *EfeClient) Classify(ctx, text) (*Verdict, error)
│   ├── (c *EfeClient) ClassifyStream(...)        (NEW D23)
│   ├── (c *EfeClient) Healthcheck(ctx)           (NEW D23)
│   ├── (c *EfeClient) Close() error
│   └── Verdict struct + 6 sentinel errors
│
├── Functional options
│   ├── WithJWT(token string)
│   ├── WithMTLS(certPath, keyPath, caPath string)
│   ├── WithDeadline(d time.Duration)
│   ├── WithRetry(maxAttempts int, ...)
│   ├── WithOTel(tracerProvider trace.TracerProvider)
│   ├── WithPrometheus(registerer prometheus.Registerer)
│   ├── WithLogger(slog.Logger)
│   └── WithUserAgent(ua string)
│
├── Sub-systems
│   ├── gRPC code gen (internal/proto/)
│   ├── Auth (JWT + mTLS)
│   ├── OTel auto-instrumentation
│   ├── Prometheus auto-export
│   ├── Retry exp + jitter
│   ├── Circuit breaker
│   └── Mock server (bufconn)
│
├── 3 framework examples
│   ├── examples/gin/                     (sub-module)
│   ├── examples/fiber/                   (sub-module)
│   └── examples/chi/                     (sub-module)
│
├── Tests
│   ├── ~30 unit tests (coverage ≥ 80%)
│   ├── ~10 integration tests (bufconn)
│   ├── 3 framework smoke tests
│   └── Race detector GREEN
│
└── Documentation
    ├── README.md (quickstart + 3 frameworks)
    ├── doc.go (godoc canonical)
    ├── 3 example READMEs
    └── CHANGELOG.md
```

---

## §1. Mission de l'extension M9 (Φ-B)

### §1.1 Énoncé opérationnel

L'extension M9 a **trois missions** :

1. **Combler le gap "Go" de la surface SDK multi-runtime**.
   AS-IS le seul SDK shipped est Python (M8). Pour qu'EFE soit
   adoptable par les services de production typiques d'écosystème
   cloud-native (auth/billing/notification écrits en Go), il faut
   un **client Go natif** qui parle gRPC vers Lettre E.

2. **Démontrer la stabilité du wire contract** : le proto
   `efe.v1.SwarmService` est stable depuis D8 ; un Go SDK qui
   le consomme prouve par construction qu'aucune surface gRPC
   n'est Python-specific. Si un champ proto avait un nom Python-y
   (e.g. snake_case mal traduit), le code-gen Go le surfacerait.

3. **Établir le pattern multi-runtime** : le Go SDK est le
   **template** que le sprint D24 (TypeScript SDK) suivra.
   Décisions architecturales prises ici (functional options,
   sentinel errors, retry policy, OTel auto-wire) cascadent.

### §1.2 Périmètre strict du document

Ce document **NE traite PAS** :

* Le TypeScript SDK — sprint D24 (Φ-B, follow-up).
* Le Java SDK — sprint D27 (Φ-C).
* Le Rust SDK — non planifié (le Rust callers utilisent les
  crates `efe-core` / `efe-server` directement, pas via SDK).
* La Lettre E elle-même — `LETTER_E_ARCHITECTURE.md` + sprints
  D21-D22 (prerequis du SDK).
* Les bridges LLM (claude / openai) — restent côté Python (M8).
  Les apps Go qui veulent bridger Claude / OpenAI utilisent les
  SDKs officiels Anthropic + OpenAI Go.
* Le Go SDK pour les RPCs admin (D11+ AdminService) — surface
  séparée `efe-sdk-go/admin` package, scope D24+.

### §1.3 Mission complémentaire — adoption developer Go

L'extension M9 ouvre la voie à l'adoption Go :

```go
// Quickstart canonique (post-D23)
package main

import (
    "context"
    "fmt"
    "log"
    "os"

    efe "github.com/efe-ai/efe-sdk-go"
)

func main() {
    client, err := efe.NewClient(
        os.Getenv("EFE_ENDPOINT"),
        efe.WithJWT(os.Getenv("EFE_JWT_TOKEN")),
        efe.WithDeadline(10*time.Second),
    )
    if err != nil { log.Fatal(err) }
    defer client.Close()

    verdict, err := client.Classify(context.Background(), "I want a refund")
    if err != nil { log.Fatal(err) }

    fmt.Printf("label=%s confidence=%.2f\n", verdict.Label, verdict.Confidence)
}
```

Sans ce SDK, un développeur Go qui veut consommer EFE doit :
1. Lire les protos manuellement.
2. Écrire son propre `protoc` invocation.
3. Gérer auth + retry + OTel + métriques à la main.
4. Écrire son propre mock pour les tests.
= ~2 jours de boilerplate par projet.

Avec le SDK, c'est **5 lignes de code**.

---

## §2. Position dans la topologie EFE post-M8

### §2.1 Graph de dépendances post-D23

```text
                   ┌────────────────────────────────────────┐
                   │  Apps Go consommatrices                │
                   │  ┌──────────┐  ┌──────────┐  ┌──────┐ │
                   │  │ gin app  │  │fiber app │  │chi   │ │
                   │  └────┬─────┘  └────┬─────┘  └──┬───┘ │
                   └───────┼─────────────┼──────────┼──────┘
                           │ go get      │          │
                           ▼             ▼          ▼
                   ┌────────────────────────────────────────┐
                   │   github.com/efe-ai/efe-sdk-go (M9)    │
                   │  ┌────────────────────────────────┐    │
                   │  │   EfeClient (public API)        │   │
                   │  │   - NewClient(addr, opts...)    │   │
                   │  │   - Classify(ctx, text)         │   │
                   │  │   - ClassifyStream(...)         │   │
                   │  │   - Healthcheck(ctx)            │   │
                   │  │   - Close()                     │   │
                   │  └────────┬───────────────────────┘    │
                   │           │                            │
                   │  ┌────────▼───────────────────────┐    │
                   │  │ internal/proto/efe/v1/         │    │
                   │  │ (gRPC stubs auto-generated)    │    │
                   │  └────────┬───────────────────────┘    │
                   └───────────┼────────────────────────────┘
                               │ gRPC over TLS
                               ▼
                   ┌────────────────────────────────────────┐
                   │   efe-server (M4, Lettre E)            │
                   │   - Mode1Service.Execute               │
                   │   - CatalogService.Get                 │
                   │   - AdminService.* (D21)               │
                   └────────────────────────────────────────┘
```

### §2.2 Direction stricte des dépendances

* `efe-sdk-go` dépend de :
  * **`google.golang.org/grpc`** (gRPC stack) — direct.
  * **`go.opentelemetry.io/otel`** (tracing) — soft dep (default
    no-op tracer).
  * **`go.opentelemetry.io/contrib/.../otelgrpc`** — interceptor.
  * **`github.com/prometheus/client_golang`** — soft dep (no-op
    metric registry default).
  * **`github.com/cenkalti/backoff/v4`** — retry (battle-tested).
  * **`github.com/stretchr/testify`** — tests only.

* `efe-sdk-go` **NE dépend PAS** :
  * D'`efe-core` / `efe-kernels` / `efe-server` (les crates Rust)
    — pas de cgo, pas de FFI. Le SDK est pur Go.
  * D'`efe-tool` (Python SDK M8) — les deux SDKs sont indépendants.

* Les apps Go consommatrices font `go get
  github.com/efe-ai/efe-sdk-go@v0.3.0` pour le module privé pendant
  Φ-B, puis `@v1.0.0` à GA.

### §2.3 Position vs M8 (Python SDK) + M4 (efe-server)

| Surface | M9 (Go SDK) | M8 (Python SDK) | M4 (efe-server) |
|---|:---:|:---:|:---:|
| Module Go | ✅ `efe-sdk-go` | ❌ | ❌ |
| Module Python | ❌ | ✅ `efe-tool` | ❌ |
| gRPC client auto-généré | ✅ via protoc-gen-go | ✅ via grpcio-tools | ❌ — répond à |
| OTel auto | ✅ via otelgrpc | ✅ via opentelemetry | ✅ — émet |
| Verdict frozen type | ✅ Go struct (immutable conventionnel) | ✅ `@dataclass(frozen=True)` | ❌ — produit |
| 3 framework examples | ✅ gin/fiber/chi | ✅ FastAPI/Flask/Celery | ❌ |
| Module privé d'abord | ✅ Φ-B | ❌ public dès D10 | ❌ |
| Mock server | ✅ bufconn | ✅ FakeEfeFacade | ❌ |

---

## §3. État AS-IS au 2026-04-29 (greenfield)

### §3.1 Inventaire physique `clients/`

```
clients/
├── python/                                 (M8 Python SDK ✅ shipped)
│   ├── efe_tool/   (~5 120 LOC + 52 pin tests)
│   └── ...
└── (NO go/ subdirectory yet)
```

`clients/go/` n'existe PAS encore. **Greenfield total.**

### §3.2 Protos disponibles (consommés par le Go SDK)

```
protos/efe/v1/
├── types.proto       (Verdict, RoundUpdate, ...)
├── efe.proto         (SwarmService — main service)
├── catalog.proto     (CatalogService)
├── admin.proto       (AdminService — D21+)
├── mode2.proto       (Mode2Service — D34+)
└── mode3.proto       (Mode3Service — D37+)
```

Tous ces fichiers sont prêts à être consommés par
`protoc --go_out=. --go-grpc_out=.`.

### §3.3 Synthèse AS-IS — gap analysis

| Surface | État AS-IS | Gap | Effort |
|---|---|---|---|
| `clients/go/efe-sdk/` | ❌ inexistant | Module entier | 10 jours |
| API `EfeClient.Classify` | ❌ | NEW | inclus |
| OTel + Prom | ❌ | NEW | inclus |
| 3 framework examples | ❌ | NEW | inclus |
| Mock server | ❌ | NEW | inclus |
| Tests | ❌ | NEW | inclus |
| Doc + README | ❌ | NEW | inclus |
| Publication | ❌ | NEW (privé Φ-B, public GA) | inclus |

Total **D23 = 10 jours-ingénieur** sur 1 sprint Φ-B.

### §3.4 Pré-requis externes

* **Sprint D21 (prerequis)** : `efe-server:v0.3.0` Docker image
  + AdminService 8 RPCs.
* **Sprint D22 (prerequis)** : mTLS + JWT scopes opérationnels.
* **Lettre E single-tenant** : disponible sur `staging.efe.ai:50051`
  pour tests d'intégration.

Sans D21+D22, le Go SDK reste démarrable mais ne peut tester
qu'avec mock server (`bufconn`).

---

## §4. Définition opérationnelle de M9 v1.0 GA

### §4.1 Le contrat GA

À la fin de l'extension M9 (post-D23 + post-publish v1.0.0) :

* **Module Go publié** : `go get github.com/efe-ai/efe-sdk-go@v1.0.0`
  fonctionne sans authentification (post-GA, public).
* **Coverage tests ≥ 80%** sur le code non-généré.
* **Race detector GREEN** : `go test -race ./...` passe.
* **3 framework examples runnables** : gin / fiber / chi avec
  README + `go run main.go` + screenshots.
* **OTel + Prom auto-wired** quand les libs sont importées par le
  consommateur.
* **`go vet` + `golangci-lint` GREEN** : zéro warning.
* **Quickstart 5 lignes** : `NewClient` → `Classify` → `verdict.Label`.
* **godoc complète** : `pkg.go.dev/github.com/efe-ai/efe-sdk-go` avec
  examples runnables.
* **Smoke test contre staging** : ajout dans
  `.github/workflows/sdk-go-smoke.yml`.

### §4.2 Versioning produit

| efe-sdk-go version | Sprint | Visibilité | Status |
|---|:-:|:-:|:-:|
| v0.0.0 (pre-D23) | — | N/A | greenfield |
| v0.3.0 (post-D23) | D23 | privé `GOPRIVATE` | beta |
| v0.4.x..v0.9.x (D24-D40) | follow-up | privé | hardening |
| **v1.0.0 GA** | post-D40 | **public proxy** | GA |

### §4.3 Gating entre versions

Chaque bump exige :

* `go.mod` `module` line cohérent (path stability).
* `go test -race ./...` + `go vet ./...` GREEN.
* `golangci-lint run ./...` GREEN.
* Pin tests Python `scripts/tests/test_d23_go_sdk_*.py`
  (anti-régression structurelle).
* Tag `vX.Y.Z` (signed avec GPG/sigstore) + push.
* Smoke test contre `staging.efe.ai` GREEN sur 3 plateformes
  (Linux x86_64, Linux arm64, macOS arm64).

### §4.4 Surface vs Python SDK — symétrie / divergences

| Concept | Python (M8) | Go (M9) | Divergence ? |
|---|---|---|---|
| Type Verdict | `@dataclass(frozen=True)` | `struct` (champs publics, conventionnel immutable) | Idiomatique |
| Constructeur | `EfeFacade.from_env()` | `NewClient(addr, opts...)` | Idiomatique |
| Async | `classify_async()` | `Classify(ctx)` (context-driven) | Idiomatique |
| Context manager | `with EfeFacade(...):` | `defer client.Close()` | Idiomatique |
| Errors | exception hierarchy | `errors.Is` + sentinels | Idiomatique |
| Retry | hidden in facade | explicit `WithRetry(opts...)` | Idiomatique |
| OTel | soft-dep auto | soft-dep auto via interceptor | Symétrique |
| Prom | soft-dep auto | soft-dep auto via collector | Symétrique |
| Mock | `MockEfeFacade` | `mock.NewClient` | Symétrique |

---

## §5. Principes directeurs (idiomes Go)

### §5.1 Principes opposables en revue PR

| ID | Principe | Application |
|----|---|---|
| **G-1** | **`context.Context` toujours premier arg** | Toute fonction qui peut bloquer (RPC, I/O, retry) prend `ctx context.Context` en `1er` argument. Anti-pattern : passer un `time.Duration` en argument séparé alors que `ctx` peut le porter. |
| **G-2** | **Functional options pour la config** | `NewClient(addr, opts...)` accepte des `ClientOption` variadics. Pas de `Config` struct mutable, pas de constructeur à 15 args. |
| **G-3** | **Errors sentinels + `errors.Is`** | Les erreurs publiques sont des sentinels (`var ErrUnauthenticated = errors.New(...)`) wrappées via `fmt.Errorf("...: %w", err)`. Les consommateurs filtrent via `errors.Is(err, efe.ErrUnauthenticated)`. |
| **G-4** | **Pas d'init() global** | Aucun `func init()` qui register une prom metric ou un tracer auto. Tout est explicit via options. (Exception : init() de tests OK.) |
| **G-5** | **Soft deps OTel + Prom** | Si le consommateur n'imporrte PAS `prometheus_client`, le SDK ne register rien. Pas de panic à l'import. Idem OTel : default = no-op tracer. |
| **G-6** | **Race-detector friendly** | Toute mutation partagée passe par `sync.Mutex` ou `sync.RWMutex`. Aucun `sync.Map` sans cause. `go test -race ./...` doit pass. |
| **G-7** | **Generics quand utile, jamais gratuit** | Pas de `Map[K, V]` générique pour 1 instance. Generics OK pour `Result[T any]` ou `Iterator[T]`. |
| **G-8** | **Public API = packages, internal = `internal/`** | Code généré par protoc dans `internal/proto/`, pas dans le namespace public. Anti-pattern : exposer les structs proto directement (couple le SDK au proto layout). |
| **G-9** | **Backward compat semver** | Une fois v1.0.0 public, ajout d'option = minor bump, suppression / renommage = major bump (jamais sans 18 mois de deprecation). |
| **G-10** | **Smoke test contre staging** | Le CI exécute un test réel contre `staging.efe.ai` (gated derrière `EFE_LIVE_SMOKE=true`) — détection de drift wire contract en amont. |

### §5.2 Ce que `efe-sdk-go` refuse d'implémenter

* **Pas de `package main`** dans le SDK lui-même — c'est une lib,
  pas un binaire. Les binaires sont dans `examples/*/`.
* **Pas de cgo** — pure Go pour cross-compilation native (Linux
  arm64 + macOS sans Xcode, etc.).
* **Pas de `defer init()`** — tout init explicite, debuggable.
* **Pas de side-effect d'import** — `import "github.com/efe-ai/efe-sdk-go"`
  ne register aucune prom metric, ne crée aucune connexion gRPC.
* **Pas de panic** sur erreurs runtime — tout retourne `error`.
* **Pas de log direct** — utilise `slog.Logger` injectable via
  `WithLogger(...)`. Default = silent.
* **Pas de mutation globale** — toute la state vit dans
  `*EfeClient`.

---

## §6. Invariants structurels (layout module)

### §6.1 Invariants de structure de fichiers

```
clients/go/efe-sdk/                                           (M9 root)
├── go.mod                          # module github.com/efe-ai/efe-sdk-go
├── go.sum
├── README.md                       # quickstart + 3 frameworks
├── LICENSE                         # Apache-2.0
├── doc.go                          # godoc canonical (~50 LOC)
├── client.go                       # EfeClient struct + NewClient (~280 LOC)
├── classify.go                     # Classify + ClassifyStream (~180 LOC)
├── healthcheck.go                  # Healthcheck (~80 LOC)
├── options.go                      # ClientOption pattern (~250 LOC)
├── errors.go                       # Sentinel errors + map gRPC code (~200 LOC)
├── retry.go                        # backoff + jitter (~130 LOC)
├── tracing.go                      # OTel auto-instrumentation (~100 LOC)
├── metrics.go                      # Prometheus collectors (~140 LOC)
├── auth.go                         # JWT bearer interceptor (~90 LOC)
├── tls.go                          # mTLS config helper (~110 LOC)
├── verdict.go                      # Verdict struct + helpers (~120 LOC)
├── version.go                      # SDK version constant (~30 LOC)
├── client_test.go                  # unit tests (~200 LOC)
├── classify_test.go
├── options_test.go
├── errors_test.go
├── retry_test.go
├── tracing_test.go
├── metrics_test.go
├── auth_test.go
├── verdict_test.go
├── integration_test.go             # E2E vs bufconn mock (~250 LOC)
├── mock/                           # public sub-package
│   ├── client.go                   # MockEfeClient (~150 LOC)
│   ├── server.go                   # bufconn-based mock server (~200 LOC)
│   └── verdict.go                  # NewVerdict(...) factory (~80 LOC)
├── internal/                       # NON-PUBLIC code
│   ├── proto/                      # generated gRPC stubs
│   │   └── efe/v1/
│   │       ├── types.pb.go         # ~600 LOC (auto-gen)
│   │       ├── efe.pb.go           # ~400 LOC (auto-gen)
│   │       ├── efe_grpc.pb.go      # ~300 LOC (auto-gen)
│   │       ├── catalog.pb.go       # ~700 LOC (auto-gen)
│   │       ├── catalog_grpc.pb.go  # ~250 LOC (auto-gen)
│   │       ├── admin.pb.go         # ~400 LOC (auto-gen)
│   │       └── admin_grpc.pb.go    # ~250 LOC (auto-gen)
│   └── grpcconn/                   # connection helpers
│       └── conn.go                 # ~120 LOC
├── examples/
│   ├── gin/                        # sub-module
│   │   ├── go.mod
│   │   ├── go.sum
│   │   ├── main.go                 # ~150 LOC
│   │   └── README.md
│   ├── fiber/
│   │   ├── go.mod
│   │   ├── go.sum
│   │   ├── main.go                 # ~150 LOC
│   │   └── README.md
│   └── chi/
│       ├── go.mod
│       ├── go.sum
│       ├── main.go                 # ~130 LOC
│       └── README.md
├── scripts/
│   ├── gen-proto.sh                # protoc invocation (~60 LOC)
│   ├── lint.sh                     # golangci-lint + go vet
│   ├── test.sh                     # go test -race + coverage
│   └── publish.sh                  # tag + push (private vs public)
└── Makefile                        # canonical entry points

scripts/tests/                       (anti-régression Python pin tests)
└── test_d23_go_sdk_*.py            # ~25 pin tests
```

### §6.2 Invariants de runtime

* **Cold start `NewClient` < 50 ms** : aucun work à l'instantiation
  hors gRPC dial (lazy par défaut, dial happens on first RPC).
* **`Classify` p99 < 250 ms** quand le serveur EFE répond p99 < 200 ms
  (overhead SDK ≤ 50 ms : sérialisation proto + retry + OTel).
* **Mémoire stable** sous charge constante (< 50 MiB RSS pour
  1 000 RPS pendant 1 h — pas de leak proto allocator).
* **Race detector GREEN** : aucun data race sur :
  * `EfeClient.Close()` concurrent with `Classify()`.
  * Concurrent `Classify` calls.
  * Retry counter mutation.
  * OTel span attribute set.
* **Idempotence retry** : seuls `Classify` (read-mostly) + `Healthcheck`
  retryent. `RegisterCatalog` (write) NEVER retry sans idempotency
  key explicite.

### §6.3 Invariants de versioning module

```go
// go.mod (post-D23, private)
module github.com/efe-ai/efe-sdk-go

go 1.22

require (
    google.golang.org/grpc v1.62.0
    google.golang.org/protobuf v1.32.0
    go.opentelemetry.io/otel v1.24.0
    go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc v0.49.0
    github.com/prometheus/client_golang v1.18.0
    github.com/cenkalti/backoff/v4 v4.2.1
)

require (
    // testing-only
    github.com/stretchr/testify v1.8.4
    google.golang.org/grpc/test/bufconn v1.62.0
)
```

* **Module path immutable** post-publication : un consommateur
  qui fait `import "github.com/efe-ai/efe-sdk-go"` v1.0.0 doit
  pouvoir continuer à importer ce path pendant ≥ 5 ans.
* **Sub-modules** : `examples/gin/go.mod` est un module **séparé**
  (pas de version coupling avec le SDK lui-même — un dev qui veut
  voir le exemple ne pull pas accidentellement les deps gin).
* **Major version 2.x** : doit changer le path (`v2/`) per Go
  module convention.

---

## §7. Vue d'ensemble — sprint D23 unique

### §7.1 Tableau d'agrégation

| Sprint | Phase | Durée | Livrable | LOC actif | LOC généré | Pin tests |
|:-:|:-:|:-:|---|--:|--:|--:|
| **D23** | Φ-B | 10 j | Module Go entier | ~2 150 | ~3 000 | ~25 |

### §7.2 Dépendances inter-sprints

```text
                D21 (Dockerfile prod + AdminService)
                     │
                     │ requires : Lettre E ready to ship
                     │
                D22 (mTLS + JWT scopes per-tenant)
                     │
                     │ requires : auth fully wired
                     │
                D23 (Go SDK ← ce sprint)
                     │
                     │ requires : protos stables + Lettre E
                     │   reachable on staging
                     │
                D24 (TypeScript SDK)
                     │
                     │ template inherits D23 patterns :
                     │  - functional options
                     │  - sentinel errors
                     │  - OTel + Prom soft-dep
                     │  - mock server
                     ▼
                Φ-C (D25-D27) — Java SDK (D27)
```

### §7.3 Volumétrie code attendue (D23 only)

```
SDK actif :
  client.go                        → ~280 LOC
  classify.go                      → ~180 LOC
  healthcheck.go                   → ~80 LOC
  options.go                       → ~250 LOC
  errors.go                        → ~200 LOC
  retry.go                         → ~130 LOC
  tracing.go                       → ~100 LOC
  metrics.go                       → ~140 LOC
  auth.go                          → ~90 LOC
  tls.go                           → ~110 LOC
  verdict.go                       → ~120 LOC
  version.go + doc.go              → ~80 LOC
  Total SDK actif                  → ~1 760 LOC

Tests :
  client_test.go                   → ~200 LOC
  classify_test.go                 → ~150 LOC
  retry_test.go                    → ~120 LOC
  tracing_test.go                  → ~80 LOC
  metrics_test.go                  → ~80 LOC
  auth_test.go                     → ~100 LOC
  errors_test.go                   → ~80 LOC
  options_test.go                  → ~80 LOC
  verdict_test.go                  → ~60 LOC
  integration_test.go              → ~250 LOC
  Total tests                      → ~1 200 LOC

Mock + helpers :
  mock/client.go                   → ~150 LOC
  mock/server.go                   → ~200 LOC
  mock/verdict.go                  → ~80 LOC
  internal/grpcconn/conn.go        → ~120 LOC
  Total mock                       → ~550 LOC

Examples (3 sub-modules) :
  examples/gin/                    → ~150 + 30 LOC (main + README)
  examples/fiber/                  → ~150 + 30 LOC
  examples/chi/                    → ~130 + 30 LOC
  Total examples                   → ~520 LOC

Generated (auto-protoc) :
  internal/proto/efe/v1/*.pb.go    → ~3 000 LOC

Documentation :
  README.md                        → ~250 LOC Markdown
  doc.go                           → ~50 LOC Go comments
  3 examples README.md             → ~150 LOC Markdown
  CHANGELOG.md entry               → ~80 LOC Markdown
  Total doc                        → ~530 LOC

Pin tests Python (anti-régression structurelle) :
  test_d23_go_sdk_*.py             → ~400 LOC, ~25 tests

Net : ~2 150 LOC actif + ~1 200 LOC tests + ~550 LOC mock +
      ~520 LOC examples + ~3 000 LOC généré + ~530 LOC doc +
      ~400 LOC pin tests = ~8 350 LOC.
```

---

## §8. Surface publique cible (API Go publishable)

### §8.1 API canonique post-D23

```go
// Package efe is the official Go SDK for the EFE (Essaim Fluide
// Éphémère) inference platform.
//
// See https://efe.ai/sdk/go for the full documentation.
//
// Quickstart:
//
//   client, err := efe.NewClient(
//       "staging.efe.ai:50051",
//       efe.WithJWT(token),
//       efe.WithDeadline(10*time.Second),
//   )
//   if err != nil { log.Fatal(err) }
//   defer client.Close()
//
//   verdict, err := client.Classify(ctx, "I want a refund")
//   if err != nil { log.Fatal(err) }
//   fmt.Println(verdict.Label, verdict.Confidence)
package efe

// EfeClient is the main entry point. Construct via NewClient.
// Safe for concurrent use.
type EfeClient struct {
    // unexported fields ; struct is opaque.
}

// NewClient constructs a client connected to addr.
//
// The returned client lazily dials the gRPC server on the first RPC ;
// the constructor itself does not perform I/O. Call Close() to release
// the underlying connection when done.
//
// addr accepts host:port (e.g. "localhost:50051") or scheme://host:port
// (e.g. "https://staging.efe.ai:50051" — TLS auto-detected).
func NewClient(addr string, opts ...ClientOption) (*EfeClient, error)

// Classify sends one classification request to the server.
//
// The verdict carries the predicted label, confidence, cone alignment
// metadata, and request id. On error, the returned error wraps one
// of the sentinel errors (errors.Is(err, ErrUnauthenticated), etc.).
//
// The context controls the request deadline + cancellation. If ctx
// has no deadline, the client's WithDeadline option applies.
func (c *EfeClient) Classify(ctx context.Context, text string) (*Verdict, error)

// ClassifyStream sends a stream of texts and yields a stream of verdicts.
// Useful for batch processing. The slice ordering is preserved.
//
// Cancel via ctx.Done() ; the stream cleans up partial work.
func (c *EfeClient) ClassifyStream(
    ctx context.Context,
    texts <-chan string,
) (<-chan StreamItem, error)

// Healthcheck probes the server's gRPC health endpoint.
// Returns nil if SERVING ; non-nil error otherwise.
func (c *EfeClient) Healthcheck(ctx context.Context) error

// Close releases the gRPC connection. Idempotent.
func (c *EfeClient) Close() error

// Verdict is the public verdict type. Conventionally immutable
// (do not mutate fields after Classify returns).
type Verdict struct {
    Label          string
    Confidence     float64
    Reject         bool
    Rounds         int
    AlignmentDelta float64
    Catalog        string
    Zone           string  // "trusted" | "warn" | "reject"
    SwarmID        string
    RequestID      string
    ElapsedMs      int64
}

// StreamItem is one verdict + index pair from ClassifyStream.
type StreamItem struct {
    Index   int
    Verdict *Verdict
    Err     error
}

// ── Sentinel errors ─────────────────────────────────────

var (
    ErrUnauthenticated  = errors.New("efe: unauthenticated")
    ErrPermissionDenied = errors.New("efe: permission denied")
    ErrCatalogNotFound  = errors.New("efe: catalog not found")
    ErrCatalogInvalid   = errors.New("efe: catalog invalid")
    ErrTimeout          = errors.New("efe: deadline exceeded")
    ErrUnavailable      = errors.New("efe: server unavailable")
    ErrRateLimited      = errors.New("efe: rate limited")
    ErrInvalidArgument  = errors.New("efe: invalid argument")
    ErrInternal         = errors.New("efe: server internal error")
    ErrClosed           = errors.New("efe: client is closed")
)

// ── Options ───────────────────────────────────────────

// ClientOption mutates a client's configuration at construction.
type ClientOption func(*options) error

func WithJWT(token string) ClientOption
func WithMTLS(certPath, keyPath, caPath string) ClientOption
func WithTLS(serverNameOverride string) ClientOption
func WithDeadline(d time.Duration) ClientOption
func WithRetry(maxAttempts int, backoff RetryBackoff) ClientOption
func WithOTel(tp trace.TracerProvider) ClientOption
func WithPrometheus(reg prometheus.Registerer) ClientOption
func WithLogger(logger *slog.Logger) ClientOption
func WithUserAgent(ua string) ClientOption
func WithDialTimeout(d time.Duration) ClientOption
```

### §8.2 Mapping consommateur → surface

| Consommateur | Surface principale | Sub-package |
|:-:|:-:|:-:|
| Service Go (gin) | `efe.NewClient` + `Classify` | root |
| Service Go (fiber) | idem | root |
| Service Go (chi) | idem | root |
| Tests unitaires | `efe/mock.NewClient` | `mock/` |
| Auditor (godoc) | doc.go + README | root |
| CI smoke test | `efe.NewClient` + live staging | root |

---

## §9. Sprint **D23** — module Go entier

### §9.1 Objectif

Livrer le module Go complet : code source + protos générés +
tests + 3 examples + doc + CI integration + tag privé `v0.3.0`
(public à GA).

### §9.2 Architecture haut-niveau

```text
                          NewClient(addr, opts...)
                                    │
                                    ▼
                          ┌─────────────────────┐
                          │      EfeClient      │
                          │  - conn *grpc.Conn  │
                          │  - opts options     │
                          │  - logger *slog     │
                          │  - tracer trace.Tracer
                          │  - metrics *Metrics │
                          └─────┬───────────────┘
                                │
              ┌─────────────────┼─────────────────┐
              ▼                 ▼                 ▼
       ┌──────────┐      ┌────────────┐     ┌───────────┐
       │ Classify │      │ Healthcheck │     │ Close()   │
       └────┬─────┘      └──────┬──────┘     └───────────┘
            │                   │
            │  retry+backoff    │  retry+backoff
            ▼                   ▼
       ┌──────────────────────────┐
       │   gRPC interceptor chain │
       │  - auth (JWT)            │
       │  - mTLS                   │
       │  - otel (otelgrpc)        │
       │  - prom (prometheus)      │
       │  - log (slog)             │
       └──────────┬────────────────┘
                  ▼
           ┌────────────────┐
           │  grpc.ClientConn│  (lazy dial)
           └────────┬────────┘
                    ▼
           ┌────────────────┐
           │ gen pb.SwarmServiceClient│
           └────────────────┘
                    ▼
                 Network
                    ▼
           Lettre E gRPC server
```

### §9.3 Chaîne de plug-ins gRPC

L'instanciation de `*EfeClient` configure une **chaîne d'intercepteurs**
unaires + streaming :

```go
// client.go (D23 canonical wiring)
func (o *options) buildDialOptions() []grpc.DialOption {
    interceptors := []grpc.UnaryClientInterceptor{
        // 1. Auth (JWT bearer) — first to attach the token to the
        //    request before downstream interceptors see it.
        authUnaryInterceptor(o.jwtToken),
        // 2. Logging (slog) — second so logged requests carry the
        //    auth metadata too.
        loggingUnaryInterceptor(o.logger),
        // 3. OTel — third. Wraps each call in a span ; downstream
        //    metrics see the span ID.
        otelgrpc.UnaryClientInterceptor(),
        // 4. Prometheus — fourth ; metrics observe the post-OTel
        //    timing.
        promUnaryInterceptor(o.metrics),
        // 5. Retry (last) — applied after metrics so retries are
        //    counted as separate observations (not collapsed).
        retryUnaryInterceptor(o.retry),
    }
    streamInterceptors := []grpc.StreamClientInterceptor{
        // ... same order, stream variants.
    }
    return []grpc.DialOption{
        grpc.WithChainUnaryInterceptor(interceptors...),
        grpc.WithChainStreamInterceptor(streamInterceptors...),
        grpc.WithUserAgent(o.userAgent),
        grpc.WithDefaultCallOptions(grpc.MaxCallRecvMsgSize(o.maxRecvSize)),
    }
}
```

### §9.4 Critères Go/No-Go D23

* GG-D23.1 : `go build ./...` GREEN sur Linux + macOS arm64.
* GG-D23.2 : `go test -race ./...` GREEN (coverage ≥ 80%).
* GG-D23.3 : `golangci-lint run ./...` zéro warning.
* GG-D23.4 : 3 examples runnables (`go run examples/gin/main.go`
  démarre sans erreur).
* GG-D23.5 : Smoke test contre `staging.efe.ai` GREEN.
* GG-D23.6 : 25 pin tests Python `test_d23_go_sdk_*.py` GREEN.
* GG-D23.7 : Tag `v0.3.0` poussé en privé sur `github.com/efe-ai/efe-sdk-go`.
* GG-D23.8 : Doc godoc rendue correctement (preview locale via
  `godoc -http=:6060`).

---

## §10. Sub-système 1 — `EfeClient` + functional options pattern

### §10.1 Le struct opaque

```go
// client.go

type EfeClient struct {
    conn       *grpc.ClientConn
    swarm      pb.SwarmServiceClient
    catalog    pb.CatalogServiceClient
    health     healthpb.HealthClient

    opts       options    // value, frozen post-construction

    closeOnce  sync.Once
    closed     atomic.Bool
}

// options is internal — never exposed.
type options struct {
    addr             string
    dialTimeout      time.Duration
    deadline         time.Duration
    jwtToken         string
    mtls             *mtlsConfig
    tlsServerName    string
    insecureSkipVerify bool

    retry            retryConfig
    userAgent        string
    maxRecvSize      int

    tracerProvider   trace.TracerProvider
    promRegisterer   prometheus.Registerer
    logger           *slog.Logger
}
```

### §10.2 Functional options canoniques

```go
// options.go

type ClientOption func(*options) error

func WithJWT(token string) ClientOption {
    return func(o *options) error {
        if token == "" {
            return fmt.Errorf("efe: WithJWT: empty token")
        }
        o.jwtToken = token
        return nil
    }
}

func WithMTLS(certPath, keyPath, caPath string) ClientOption {
    return func(o *options) error {
        cfg, err := loadMTLS(certPath, keyPath, caPath)
        if err != nil {
            return fmt.Errorf("efe: WithMTLS: %w", err)
        }
        o.mtls = cfg
        return nil
    }
}

func WithDeadline(d time.Duration) ClientOption {
    return func(o *options) error {
        if d <= 0 {
            return fmt.Errorf("efe: WithDeadline: must be > 0, got %v", d)
        }
        o.deadline = d
        return nil
    }
}

func WithRetry(maxAttempts int, backoff RetryBackoff) ClientOption {
    return func(o *options) error {
        if maxAttempts < 1 {
            return fmt.Errorf("efe: WithRetry: maxAttempts must be ≥ 1")
        }
        o.retry = retryConfig{
            maxAttempts: maxAttempts,
            backoff:     backoff,
        }
        return nil
    }
}

func WithOTel(tp trace.TracerProvider) ClientOption {
    return func(o *options) error {
        if tp == nil {
            return fmt.Errorf("efe: WithOTel: nil tracer provider")
        }
        o.tracerProvider = tp
        return nil
    }
}

// ... 5 autres options.
```

### §10.3 NewClient — constructor lazy

```go
// client.go

func NewClient(addr string, opts ...ClientOption) (*EfeClient, error) {
    o := defaultOptions()
    o.addr = addr
    for _, opt := range opts {
        if err := opt(&o); err != nil {
            return nil, err
        }
    }
    if err := o.validate(); err != nil {
        return nil, err
    }

    // Lazy dial : we DON'T connect here. The gRPC client connects on
    // first RPC. This makes NewClient non-blocking + testable.
    conn, err := grpc.NewClient(o.addr, o.buildDialOptions()...)
    if err != nil {
        return nil, fmt.Errorf("efe: dial: %w", err)
    }

    return &EfeClient{
        conn:    conn,
        swarm:   pb.NewSwarmServiceClient(conn),
        catalog: pb.NewCatalogServiceClient(conn),
        health:  healthpb.NewHealthClient(conn),
        opts:    o,
    }, nil
}

func defaultOptions() options {
    return options{
        deadline:        10 * time.Second,
        dialTimeout:     5 * time.Second,
        userAgent:       fmt.Sprintf("efe-sdk-go/%s", Version),
        maxRecvSize:     16 * 1024 * 1024,  // 16 MiB
        retry:           retryConfig{maxAttempts: 3, backoff: defaultBackoff()},
        logger:          slog.New(slog.NewTextHandler(io.Discard, nil)),  // silent
        tracerProvider:  noop.NewTracerProvider(),                         // no-op
        promRegisterer:  prometheus.NewRegistry(),                         // local, not global
    }
}
```

---

## §11. Sub-système 2 — gRPC code generation pipeline

### §11.1 protoc invocation

```bash
#!/usr/bin/env bash
# clients/go/efe-sdk/scripts/gen-proto.sh

set -euo pipefail
IFS=$'\n\t'

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
PROTOS_DIR="$REPO_ROOT/protos"
OUT_DIR="$REPO_ROOT/clients/go/efe-sdk/internal/proto"

# Ensure protoc + plugins are present.
for bin in protoc protoc-gen-go protoc-gen-go-grpc; do
    if ! command -v "$bin" >/dev/null 2>&1; then
        echo "FATAL: $bin not in PATH" >&2
        echo "hint: brew install protobuf protoc-gen-go protoc-gen-go-grpc" >&2
        exit 2
    fi
done

# Clean (idempotent).
rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

# Generate. The `--go_opt=module=...` strips the proto package prefix
# from the output paths so we end up with `internal/proto/efe/v1/`
# (not `internal/proto/github.com/efe-ai/protos/efe/v1/`).
protoc \
    -I "$PROTOS_DIR" \
    --go_out="$OUT_DIR" \
    --go_opt=module=github.com/efe-ai/efe-sdk-go/internal/proto \
    --go-grpc_out="$OUT_DIR" \
    --go-grpc_opt=module=github.com/efe-ai/efe-sdk-go/internal/proto \
    "$PROTOS_DIR/efe/v1/types.proto" \
    "$PROTOS_DIR/efe/v1/efe.proto" \
    "$PROTOS_DIR/efe/v1/catalog.proto" \
    "$PROTOS_DIR/efe/v1/admin.proto"

echo "✅ Generated Go gRPC stubs into $OUT_DIR"
```

### §11.2 Versioning + invalidation

* Le hash SHA-256 des fichiers `.proto` est commité dans
  `internal/proto/PROTOS_HASH.txt`.
* Le pin test `test_d23_go_sdk_protos_in_sync` vérifie que le hash
  actuel des protos matche `PROTOS_HASH.txt`. Drift → CI fail.
* Régénération via `make gen-proto` ou `./scripts/gen-proto.sh`.

### §11.3 Conventions des stubs générés

* Output dir : `internal/proto/efe/v1/`.
* Package Go : `pb` (alias court).
* Types : `pb.ExecuteRequest`, `pb.SwarmServiceClient`, etc.
* **Jamais** exposés directement au consommateur du SDK.

---

## §12. Sub-système 3 — Errors + retry + circuit breaker

### §12.1 Errors module

```go
// errors.go

// Sentinel errors : public, stable, used with errors.Is.
var (
    ErrUnauthenticated  = errors.New("efe: unauthenticated")
    ErrPermissionDenied = errors.New("efe: permission denied")
    ErrCatalogNotFound  = errors.New("efe: catalog not found")
    ErrCatalogInvalid   = errors.New("efe: catalog invalid")
    ErrTimeout          = errors.New("efe: deadline exceeded")
    ErrUnavailable      = errors.New("efe: server unavailable")
    ErrRateLimited      = errors.New("efe: rate limited")
    ErrInvalidArgument  = errors.New("efe: invalid argument")
    ErrInternal         = errors.New("efe: server internal error")
    ErrClosed           = errors.New("efe: client is closed")
)

// fromGRPC translates a gRPC status code into one of our sentinels,
// wrapping the original status as the cause.
func fromGRPC(err error) error {
    if err == nil { return nil }
    st, ok := status.FromError(err)
    if !ok { return err }
    var sentinel error
    switch st.Code() {
    case codes.Unauthenticated:
        sentinel = ErrUnauthenticated
    case codes.PermissionDenied:
        sentinel = ErrPermissionDenied
    case codes.NotFound:
        sentinel = ErrCatalogNotFound
    case codes.InvalidArgument:
        sentinel = ErrInvalidArgument
    case codes.DeadlineExceeded:
        sentinel = ErrTimeout
    case codes.Unavailable:
        sentinel = ErrUnavailable
    case codes.ResourceExhausted:
        sentinel = ErrRateLimited
    case codes.Internal, codes.Unknown:
        sentinel = ErrInternal
    default:
        sentinel = fmt.Errorf("efe: unexpected gRPC code %v", st.Code())
    }
    return fmt.Errorf("%w: %s", sentinel, st.Message())
}

// IsRetryable returns true if the error is in the retry-eligible set
// (Unavailable, DeadlineExceeded, ResourceExhausted-rate-limit).
func IsRetryable(err error) bool {
    return errors.Is(err, ErrUnavailable) ||
           errors.Is(err, ErrTimeout) ||
           errors.Is(err, ErrRateLimited)
}
```

### §12.2 Retry + backoff

```go
// retry.go

type RetryBackoff interface {
    Next(attempt int) time.Duration
}

// ExponentialBackoff with full jitter (canonical AWS recommendation).
type ExponentialBackoff struct {
    InitialInterval time.Duration  // default 100ms
    MaxInterval     time.Duration  // default 30s
    Multiplier      float64        // default 2.0
    MaxElapsed      time.Duration  // default 60s
}

func (e ExponentialBackoff) Next(attempt int) time.Duration {
    base := float64(e.InitialInterval) * math.Pow(e.Multiplier, float64(attempt-1))
    if base > float64(e.MaxInterval) {
        base = float64(e.MaxInterval)
    }
    // Full jitter : random in [0, base].
    return time.Duration(rand.Float64() * base)
}

// retryUnaryInterceptor wraps a gRPC unary call with retry logic.
// Only retries on IsRetryable(err) ; non-retryable errors propagate
// immediately.
func retryUnaryInterceptor(cfg retryConfig) grpc.UnaryClientInterceptor {
    return func(
        ctx context.Context,
        method string,
        req, reply any,
        cc *grpc.ClientConn,
        invoker grpc.UnaryInvoker,
        opts ...grpc.CallOption,
    ) error {
        var err error
        for attempt := 1; attempt <= cfg.maxAttempts; attempt++ {
            err = invoker(ctx, method, req, reply, cc, opts...)
            if err == nil { return nil }
            if !IsRetryable(fromGRPC(err)) { return err }
            if attempt == cfg.maxAttempts { break }

            delay := cfg.backoff.Next(attempt)
            select {
            case <-ctx.Done():
                return ctx.Err()
            case <-time.After(delay):
            }
        }
        return err
    }
}
```

### §12.3 Idempotence des retries

| RPC | Retryable ? | Justification |
|:-:|:-:|---|
| `Classify` | ✅ | Read-mostly, idempotent (texte → verdict, pas d'état) |
| `Healthcheck` | ✅ | Probe pure |
| `RegisterCatalog` | ❌ par défaut | Write — risk de double-register |
| `EvolveCatalog` | ❌ par défaut | Write — idempotency key requis |
| `DeleteCatalog` | ❌ | Write destructive |

Override possible via `WithRetryAllRPCs(true)` (advanced, non-default).

---

## §13. Sub-système 4 — OpenTelemetry auto-instrumentation

### §13.1 OTel wiring

```go
// tracing.go

// otelInterceptor returns a gRPC interceptor that creates a span per
// RPC. Span attributes : rpc.system=grpc, rpc.service, rpc.method,
// efe.catalog (when provided).
//
// Soft dependency : when WithOTel() is not called, the default
// no-op tracer is used (zero overhead).
func otelInterceptor(tp trace.TracerProvider) grpc.UnaryClientInterceptor {
    tracer := tp.Tracer("github.com/efe-ai/efe-sdk-go")
    return otelgrpc.UnaryClientInterceptor(
        otelgrpc.WithTracerProvider(tp),
        otelgrpc.WithMeterProvider(noop.NewMeterProvider()), // metrics handled separately
    )
}
```

### §13.2 Span hierarchy canonique

```text
Parent app span (e.g. HTTP request from gin)
└── efe.classify
    ├── attributes :
    │     rpc.system=grpc
    │     rpc.service=efe.v1.SwarmService
    │     rpc.method=Execute
    │     efe.catalog=acme/billing@v3   (when known)
    │     efe.tenant=acme               (when known)
    │     efe.attempt=1                 (set by retry interceptor)
    └── (gRPC-internal spans handled by otelgrpc)
```

### §13.3 Soft-dep contract

* Si le consommateur n'importe PAS `go.opentelemetry.io/otel`, le
  SDK utilise `noop.NewTracerProvider()` (zero allocation, zero
  overhead).
* `WithOTel(tp)` exige un `tp != nil` (validation). Pour désactiver
  explicitement : `WithOTel(noop.NewTracerProvider())`.
* Pin test `test_d23_go_sdk_otel_no_op_when_unset`.

---

## §14. Sub-système 5 — Prometheus metrics

### §14.1 Metrics surface

```go
// metrics.go

// metricNames are the canonical names exposed when WithPrometheus()
// is called.
const (
    classifyTotalName       = "efe_sdk_classify_total"
    classifyLatencyName     = "efe_sdk_classify_latency_seconds"
    classifyErrorsTotalName = "efe_sdk_classify_errors_total"
    inFlightName            = "efe_sdk_in_flight"
    retryAttemptsTotalName  = "efe_sdk_retry_attempts_total"
)

type metrics struct {
    classifyTotal     *prometheus.CounterVec   // labels : outcome
    classifyLatency   *prometheus.HistogramVec // labels : outcome
    classifyErrors    *prometheus.CounterVec   // labels : sentinel
    inFlight          prometheus.Gauge
    retryAttempts     *prometheus.CounterVec   // labels : attempt
}

func newMetrics(reg prometheus.Registerer) *metrics {
    if reg == nil {
        reg = prometheus.NewRegistry() // local, not registered globally
    }
    m := &metrics{
        classifyTotal: prometheus.NewCounterVec(
            prometheus.CounterOpts{
                Name: classifyTotalName,
                Help: "Total classify calls.",
            },
            []string{"outcome"},
        ),
        // ... etc
    }
    reg.MustRegister(
        m.classifyTotal, m.classifyLatency, m.classifyErrors,
        m.inFlight, m.retryAttempts,
    )
    return m
}
```

### §14.2 Soft-dep + zero side-effect

* `WithPrometheus(reg)` est explicit. Sans cet option, le SDK
  crée un `prometheus.NewRegistry()` LOCAL (pas le global registry
  qui est `prometheus.DefaultRegisterer`).
* Conséquence : un consommateur qui scrape via `promhttp.Handler()`
  ne voit PAS les métriques SDK par défaut. Pour les voir, il doit
  faire explicitement :

  ```go
  reg := prometheus.NewRegistry()
  client, _ := efe.NewClient(addr, efe.WithPrometheus(reg))
  http.Handle("/metrics", promhttp.HandlerFor(reg, promhttp.HandlerOpts{}))
  ```

* Conforme G-4 (pas d'init() global) + G-5 (soft-dep).

---

## §15. Sub-système 6 — Auth (JWT bearer + mTLS)

### §15.1 JWT bearer interceptor

```go
// auth.go

func authUnaryInterceptor(token string) grpc.UnaryClientInterceptor {
    return func(
        ctx context.Context, method string, req, reply any,
        cc *grpc.ClientConn, invoker grpc.UnaryInvoker,
        opts ...grpc.CallOption,
    ) error {
        if token != "" {
            ctx = metadata.AppendToOutgoingContext(
                ctx, "authorization", "Bearer "+token,
            )
        }
        return invoker(ctx, method, req, reply, cc, opts...)
    }
}
```

### §15.2 mTLS configuration

```go
// tls.go

type mtlsConfig struct {
    cert       tls.Certificate
    rootCAs    *x509.CertPool
    serverName string
}

func loadMTLS(certPath, keyPath, caPath string) (*mtlsConfig, error) {
    cert, err := tls.LoadX509KeyPair(certPath, keyPath)
    if err != nil {
        return nil, fmt.Errorf("load cert/key: %w", err)
    }
    caPEM, err := os.ReadFile(caPath)
    if err != nil {
        return nil, fmt.Errorf("load CA: %w", err)
    }
    pool := x509.NewCertPool()
    if !pool.AppendCertsFromPEM(caPEM) {
        return nil, fmt.Errorf("CA file %s contains no valid certs", caPath)
    }
    return &mtlsConfig{cert: cert, rootCAs: pool}, nil
}

func (m *mtlsConfig) toCredentials() credentials.TransportCredentials {
    return credentials.NewTLS(&tls.Config{
        Certificates: []tls.Certificate{m.cert},
        RootCAs:      m.rootCAs,
        ServerName:   m.serverName,
        MinVersion:   tls.VersionTLS13,
    })
}
```

### §15.3 Trust boundary

* Le SDK NE valide PAS le contenu du JWT (signature, exp, etc.) —
  c'est le job du serveur EFE. Le SDK propage le token en metadata.
* mTLS : le SDK exige TLS 1.3 minimum (cohérent avec D22 server-side).

---

## §16. Sub-système 7 — 3 framework examples

### §16.1 Layout

Chaque exemple est un **sub-module Go** indépendant :

```
examples/gin/
├── go.mod              # module github.com/efe-ai/efe-sdk-go/examples/gin
├── go.sum
├── main.go             # ~150 LOC
└── README.md           # quickstart
```

Cela évite que le module SDK lui-même pull `gin` / `fiber` / `chi`
comme dépendances directes — le SDK reste lean.

### §16.2 Exemple gin

```go
// examples/gin/main.go
package main

import (
    "log"
    "net/http"
    "os"

    "github.com/gin-gonic/gin"
    efe "github.com/efe-ai/efe-sdk-go"
)

type classifyReq struct {
    Text string `json:"text" binding:"required"`
}

func main() {
    client, err := efe.NewClient(
        os.Getenv("EFE_ENDPOINT"),
        efe.WithJWT(os.Getenv("EFE_JWT_TOKEN")),
    )
    if err != nil { log.Fatal(err) }
    defer client.Close()

    r := gin.Default()
    r.POST("/classify", func(c *gin.Context) {
        var req classifyReq
        if err := c.ShouldBindJSON(&req); err != nil {
            c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
            return
        }
        verdict, err := client.Classify(c.Request.Context(), req.Text)
        if err != nil {
            c.JSON(efeErrorStatus(err), gin.H{"error": err.Error()})
            return
        }
        c.JSON(http.StatusOK, verdict)
    })
    log.Fatal(r.Run(":8080"))
}

// efeErrorStatus maps SDK sentinels to HTTP status codes.
func efeErrorStatus(err error) int {
    switch {
    case errors.Is(err, efe.ErrUnauthenticated): return 401
    case errors.Is(err, efe.ErrPermissionDenied): return 403
    case errors.Is(err, efe.ErrInvalidArgument): return 400
    case errors.Is(err, efe.ErrTimeout):         return 504
    case errors.Is(err, efe.ErrRateLimited):     return 429
    default:                                      return 500
    }
}
```

### §16.3 Exemple fiber

```go
// examples/fiber/main.go
package main

import (
    "log"
    "os"

    "github.com/gofiber/fiber/v2"
    efe "github.com/efe-ai/efe-sdk-go"
)

func main() {
    client, err := efe.NewClient(
        os.Getenv("EFE_ENDPOINT"),
        efe.WithJWT(os.Getenv("EFE_JWT_TOKEN")),
    )
    if err != nil { log.Fatal(err) }
    defer client.Close()

    app := fiber.New()
    app.Post("/classify", func(c *fiber.Ctx) error {
        var req struct{ Text string `json:"text"` }
        if err := c.BodyParser(&req); err != nil {
            return c.Status(400).JSON(fiber.Map{"error": err.Error()})
        }
        verdict, err := client.Classify(c.Context(), req.Text)
        if err != nil {
            return c.Status(efeErrorStatus(err)).JSON(fiber.Map{"error": err.Error()})
        }
        return c.JSON(verdict)
    })
    log.Fatal(app.Listen(":8080"))
}
```

### §16.4 Exemple chi

```go
// examples/chi/main.go
package main

import (
    "encoding/json"
    "log"
    "net/http"
    "os"

    "github.com/go-chi/chi/v5"
    efe "github.com/efe-ai/efe-sdk-go"
)

func main() {
    client, err := efe.NewClient(
        os.Getenv("EFE_ENDPOINT"),
        efe.WithJWT(os.Getenv("EFE_JWT_TOKEN")),
    )
    if err != nil { log.Fatal(err) }
    defer client.Close()

    r := chi.NewRouter()
    r.Post("/classify", func(w http.ResponseWriter, r *http.Request) {
        var req struct{ Text string `json:"text"` }
        if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
            http.Error(w, err.Error(), 400)
            return
        }
        verdict, err := client.Classify(r.Context(), req.Text)
        if err != nil {
            http.Error(w, err.Error(), efeErrorStatus(err))
            return
        }
        w.Header().Set("Content-Type", "application/json")
        json.NewEncoder(w).Encode(verdict)
    })
    log.Fatal(http.ListenAndServe(":8080", r))
}
```

---

## §17. Sub-système 8 — Mock server (`bufconn`)

### §17.1 In-memory gRPC mock

```go
// mock/server.go

// NewServer returns an in-process gRPC mock server backed by bufconn,
// plus an EfeClient that talks to it (no network involved).
//
// Use in tests:
//
//   server, client := mock.NewServer(t)
//   server.OnClassify(func(req *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
//       return &pb.ExecuteResponse{Label: "fraud", Confidence: 0.92}, nil
//   })
//   verdict, err := client.Classify(ctx, "I want a refund")
//   require.NoError(t, err)
//   require.Equal(t, "fraud", verdict.Label)
//
type Server struct {
    listener     *bufconn.Listener
    grpcServer   *grpc.Server
    onClassify   func(*pb.ExecuteRequest) (*pb.ExecuteResponse, error)
    onHealth     func(*healthpb.HealthCheckRequest) (*healthpb.HealthCheckResponse, error)
}

func NewServer(t testing.TB) (*Server, *efe.EfeClient) {
    s := &Server{
        listener:   bufconn.Listen(1024 * 1024),
        grpcServer: grpc.NewServer(),
    }
    pb.RegisterSwarmServiceServer(s.grpcServer, s)
    healthpb.RegisterHealthServer(s.grpcServer, s)
    go s.grpcServer.Serve(s.listener)
    t.Cleanup(s.grpcServer.GracefulStop)

    // Custom dialer that targets bufconn instead of network.
    client, err := efe.NewClient(
        "passthrough:bufconn",
        efe.WithDialer(func(ctx context.Context, _ string) (net.Conn, error) {
            return s.listener.Dial()
        }),
    )
    require.NoError(t, err)
    t.Cleanup(func() { client.Close() })
    return s, client
}

// (s *Server) Execute implements pb.SwarmServiceServer.
func (s *Server) Execute(ctx context.Context, req *pb.ExecuteRequest) (*pb.ExecuteResponse, error) {
    if s.onClassify == nil {
        return nil, status.Error(codes.Unimplemented, "mock OnClassify not set")
    }
    return s.onClassify(req)
}

// OnClassify sets the handler for the next Classify calls.
func (s *Server) OnClassify(fn func(*pb.ExecuteRequest) (*pb.ExecuteResponse, error)) {
    s.onClassify = fn
}
```

### §17.2 MockEfeClient (interface-based)

```go
// mock/client.go

// MockEfeClient is a struct that satisfies the EfeClient public surface
// without any gRPC machinery. Useful for unit tests that don't even
// need a mock server.
//
// Usage:
//
//   m := mock.NewClient()
//   m.SetVerdict(&efe.Verdict{Label: "fraud", Confidence: 0.92})
//   verdict, _ := m.Classify(ctx, "...")
//
type MockEfeClient struct {
    verdict *efe.Verdict
    err     error
    calls   []string
    mu      sync.Mutex
}

func NewClient() *MockEfeClient { /* ... */ }
func (m *MockEfeClient) SetVerdict(v *efe.Verdict) { /* ... */ }
func (m *MockEfeClient) SetError(err error) { /* ... */ }
func (m *MockEfeClient) Calls() []string { /* ... */ }

func (m *MockEfeClient) Classify(_ context.Context, text string) (*efe.Verdict, error) {
    m.mu.Lock(); defer m.mu.Unlock()
    m.calls = append(m.calls, text)
    return m.verdict, m.err
}
// ... etc
```

---

## §18. Stratégie de tests

### §18.1 Inventaire tests

| Type | Fichier | Tests | LOC |
|---|---|--:|--:|
| Unit | `client_test.go` | 8 | 200 |
| Unit | `classify_test.go` | 6 | 150 |
| Unit | `options_test.go` | 10 (1 par option) | 80 |
| Unit | `errors_test.go` | 8 (1 par sentinel) | 80 |
| Unit | `retry_test.go` | 5 | 120 |
| Unit | `tracing_test.go` | 4 | 80 |
| Unit | `metrics_test.go` | 4 | 80 |
| Unit | `auth_test.go` | 4 | 100 |
| Unit | `verdict_test.go` | 3 | 60 |
| Integration | `integration_test.go` | 6 (vs bufconn) | 250 |
| Pin tests Python | `test_d23_go_sdk_*.py` | 25 | 400 |
| **Total** | | **83** | **1 600** |

### §18.2 Coverage cible

* `go test -cover ./...` ≥ **80%** sur le code non-généré.
* Internal proto stubs exclus du coverage (auto-gen).
* Examples NOT counted (sub-modules).

### §18.3 Race detector

`go test -race ./...` doit pass. Tests parallèles via
`t.Parallel()` activés sauf state shared (e.g. metrics global
registry).

### §18.4 Smoke test live

```yaml
# .github/workflows/sdk-go-smoke.yml
name: SDK Go smoke
on:
  schedule: [{ cron: "0 4 * * *" }]   # daily at 04:00 UTC
  workflow_dispatch: {}
jobs:
  smoke:
    runs-on: ubuntu-latest
    if: ${{ secrets.EFE_STAGING_TOKEN != '' }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-go@v5
        with: { go-version: '1.22' }
      - run: |
          cd clients/go/efe-sdk
          EFE_ENDPOINT=staging.efe.ai:50051 \
          EFE_JWT_TOKEN=${{ secrets.EFE_STAGING_TOKEN }} \
          go test -tags=live ./tests/...
```

---

## §19. SLO mapping

| ID | SLO | Cible | Mesure |
|:-:|---|:-:|---|
| **SLO-GO-1** | `NewClient` cold start | < 50 ms | timer in test |
| **SLO-GO-2** | `Classify` p99 (warm) | server p99 + 50 ms overhead | bench vs bufconn |
| **SLO-GO-3** | Memory under 1k RPS / 1h | < 50 MiB RSS | benchstat |
| **SLO-GO-4** | Coverage non-generated | ≥ 80% | `go test -cover` |
| **SLO-GO-5** | golangci-lint warnings | 0 | CI |
| **SLO-GO-6** | Go versions supported | ≥ 1.22 | go.mod |
| **SLO-GO-7** | godoc rendering | clean (no broken refs) | pkg.go.dev |

---

## §20. Risques transverses + mitigations (10 risques)

| ID | Risque | Sévérité | Probabilité | Mitigation |
|:-:|---|:-:|:-:|---|
| **GO-R-1** | Drift proto vs `internal/proto/` | 🔴 CRIT | 🟠 MED | Pin test SHA-256 du proto file ↔ checksum committed in `PROTOS_HASH.txt` |
| **GO-R-2** | grpc-go breaking changes | 🟠 HARD | 🟢 LOW | Pin minor version dans go.mod ; bump majeur fait via PR explicite |
| **GO-R-3** | OTel API instable (1.x → 2.0) | 🟠 HARD | 🟠 MED | Soft-dep + interface abstraction ; v1 frozen pendant 2 ans côté upstream |
| **GO-R-4** | Race condition sur `Close()` | 🔴 CRIT | 🟢 LOW | `sync.Once` + `atomic.Bool` ; `go test -race` GREEN obligatoire |
| **GO-R-5** | Module path squatting (public) | 🔴 CRIT | 🟢 LOW | Reserve `github.com/efe-ai/efe-sdk-go` AVANT D23 ; annotated tag signed |
| **GO-R-6** | Examples deps drift | 🟡 MED | 🟠 MED | CI : `go build ./examples/...` à chaque PR |
| **GO-R-7** | Smoke test flake (live) | 🟡 MED | 🟠 MED | Retry 2× ; alert si 3 jours consécutifs FAIL |
| **GO-R-8** | grpc-go memory leak | 🟠 HARD | 🟢 LOW | Bench long-running ; `t.Cleanup(client.Close)` mandatory |
| **GO-R-9** | Windows compat | 🟡 MED | 🟠 MED | CI matrix Windows best-effort ; pin test path separators |
| **GO-R-10** | Generics compile time | 🟢 LOW | 🟢 LOW | Limiter generics aux helpers (Option, Result) |

---

## §21. Volumétrie code attendue

(Cf. §7.3 pour le détail.)

```
Total D23 :
  SDK actif Go                    → ~1 760 LOC
  Tests Go                        → ~1 200 LOC
  Mock                            → ~550 LOC
  Examples (3 sub-modules)        → ~520 LOC
  Generated proto                 → ~3 000 LOC
  Documentation                   → ~530 LOC
  Pin tests Python                → ~400 LOC
  CI workflows                    → ~150 LOC
  Total                           → ~8 110 LOC
```

---

## §22. Plan d'implémentation J1-J10

### §22.1 J1 — Module skeleton + protoc

```
J1 (8h) :
  - mkdir -p clients/go/efe-sdk
  - go mod init github.com/efe-ai/efe-sdk-go
  - go.mod : declare go 1.22 + grpc + otel + prom + backoff
  - scripts/gen-proto.sh : protoc invocation
  - run gen-proto.sh → internal/proto/efe/v1/*.pb.go
  - Makefile entry points (gen-proto, test, lint, examples)
  - LICENSE Apache-2.0
  - README.md skeleton
Fin J1 : `go build ./...` GREEN avec stubs gRPC importables.
```

### §22.2 J2 — `EfeClient` + functional options

```
J2 (8h) :
  - client.go : struct opaque + NewClient()
  - options.go : 10 functional options + validation
  - version.go : Version constant
  - doc.go : godoc canonical
  - client_test.go : NewClient with various opts
  - options_test.go : 10 unit tests (1 par option)
Fin J2 : `EfeClient` instantiable via NewClient + opts.
```

### §22.3 J3 — Classify + errors + retry

```
J3 (8h) :
  - errors.go : sentinels + fromGRPC mapping
  - retry.go : ExponentialBackoff + retryUnaryInterceptor
  - classify.go : Classify(ctx, text) → *Verdict
  - verdict.go : struct public
  - errors_test.go + retry_test.go + classify_test.go
Fin J3 : Classify functional contre stub gRPC, errors typés.
```

### §22.4 J4 — Auth + mTLS + healthcheck

```
J4 (8h) :
  - auth.go : authUnaryInterceptor (JWT bearer)
  - tls.go : loadMTLS + toCredentials
  - healthcheck.go : Healthcheck(ctx)
  - auth_test.go + healthcheck_test.go
Fin J4 : Auth wired, TLS 1.3 enforced.
```

### §22.5 J5 — OTel + Prometheus

```
J5 (8h) :
  - tracing.go : otelInterceptor via otelgrpc
  - metrics.go : Prometheus collectors + soft-dep
  - tracing_test.go : verify span emission
  - metrics_test.go : verify counter increment
Fin J5 : OTel + Prom auto-wired post-options.
```

### §22.6 J6 — Streaming + ClassifyStream

```
J6 (8h) :
  - classify.go : ClassifyStream(ctx, ch) implementation
  - integration tests with bufconn
  - benchmarks (b.RunParallel, race detector)
Fin J6 : Streaming functional + bench OK.
```

### §22.7 J7 — Mock server + bufconn

```
J7 (8h) :
  - mock/server.go : bufconn-based mock + OnClassify hook
  - mock/client.go : MockEfeClient (interface impl)
  - mock/verdict.go : NewVerdict factory
  - integration_test.go : 6 e2e scenarios
Fin J7 : Mock server testable, integration tests GREEN.
```

### §22.8 J8 — 3 framework examples

```
J8 (8h) :
  - examples/gin/{go.mod, main.go, README.md}
  - examples/fiber/{go.mod, main.go, README.md}
  - examples/chi/{go.mod, main.go, README.md}
  - 3 README.md avec quickstart + curl examples
Fin J8 : 3 examples runnables, READMEs publiables.
```

### §22.9 J9 — Pin tests Python + smoke CI

```
J9 (8h) :
  - scripts/tests/test_d23_go_sdk_*.py (25 tests)
  - .github/workflows/sdk-go-test.yml (CI lint + test + race)
  - .github/workflows/sdk-go-smoke.yml (live smoke)
Fin J9 : CI + pin tests GREEN.
```

### §22.10 J10 — Doc + tag privé + workshop

```
J10 (8h) :
  - README.md complet (quickstart + 3 frameworks + examples)
  - CHANGELOG.md entry M9 v0.3.0
  - Tag v0.3.0 signed (privé)
  - Workshop interne 1h : démo to Go consumers team
Fin J10 : v0.3.0 privé live, équipe Go formée.
```

---

## §23. Versioning + publication (private → public à GA)

### §23.1 Strategy

| Phase | Visibilité | Tag | Path |
|:-:|:-:|:-:|---|
| D23 livraison | privé | `v0.3.0` | `github.com/efe-ai/efe-sdk-go` (priv) |
| D24-D40 hardening | privé | `v0.4.x..v0.9.x` | idem |
| **GA** | **public** | **`v1.0.0`** | **public proxy** |
| Post-GA mineurs | public | `v1.1.x..` | idem |
| Major bump (rare) | public | `v2.0.0` (`v2/` path) | new path |

### §23.2 Publication privée

```bash
# .netrc on consumer's machine
machine github.com
login efe-ai-bot
password <PAT-with-read:packages>

# go.mod consumer
require github.com/efe-ai/efe-sdk-go v0.3.0

# env
export GOPRIVATE=github.com/efe-ai/*
go get github.com/efe-ai/efe-sdk-go@v0.3.0
```

### §23.3 Publication publique (GA)

```bash
# Maintainer side, on a clean checkout:
git tag -s v1.0.0 -m "EFE SDK Go v1.0.0 GA"
git push origin v1.0.0

# The Go module proxy (proxy.golang.org) picks it up within ~30 min.
# Once present on the proxy, the version is immutable.
# Consumer side:
go get github.com/efe-ai/efe-sdk-go@latest    # → v1.0.0
```

### §23.4 Backward compat post-v1.0.0

* **Add option** : minor bump (v1.1.0).
* **Add method** : minor bump.
* **Add struct field (additive)** : minor bump.
* **Rename / remove method** : major bump (v2.0.0 + new path
  `github.com/efe-ai/efe-sdk-go/v2`).
* **Rename / remove sentinel error** : major bump.
* Toute breaking change exige un cycle de deprecation de **18 mois**
  avec warning runtime + entry CHANGELOG.

---

## §24. Compliance + sécurité

### §24.1 Supply chain

* Module **signé** via `cosign sign-blob` ou `git tag -s`.
* `go.sum` checksums vérifiés à chaque `go mod download`.
* CI pin la version Go (`go.mod` `go 1.22.x`).
* `govulncheck ./...` GREEN à chaque PR (CVE detection).

### §24.2 Secret redaction (cohérent M8)

* `WithJWT(token)` : le token est wrapped dans une struct opaque,
  jamais log directement. Si `WithLogger(slog)` + `slog.Debug`, le
  token est redacted comme `"<REDACTED>"`.
* Pin test `test_d23_go_sdk_jwt_redacted_in_logs`.

### §24.3 godoc public

* `pkg.go.dev/github.com/efe-ai/efe-sdk-go` doit afficher :
  * Quickstart + 3 framework links
  * API reference complète (auto-extracted)
  * Examples runnables (`Example_Classify`, etc.)
  * Pas de TODO / XXX / placeholder

---

## §25. Annexes — go.mod canonique

```go
module github.com/efe-ai/efe-sdk-go

go 1.22

require (
    github.com/cenkalti/backoff/v4 v4.2.1
    github.com/prometheus/client_golang v1.18.0
    github.com/stretchr/testify v1.8.4
    go.opentelemetry.io/contrib/instrumentation/google.golang.org/grpc/otelgrpc v0.49.0
    go.opentelemetry.io/otel v1.24.0
    go.opentelemetry.io/otel/trace v1.24.0
    google.golang.org/grpc v1.62.0
    google.golang.org/protobuf v1.32.0
)

require (
    // transitive deps elided
)

retract (
    // No retracted versions yet.
)
```

---

## §26. Annexes — surface API Go complète

```go
// Public package surface — frozen at v1.0.0 GA.

// ==== Constructor ====
func NewClient(addr string, opts ...ClientOption) (*EfeClient, error)

// ==== Methods ====
func (c *EfeClient) Classify(ctx context.Context, text string) (*Verdict, error)
func (c *EfeClient) ClassifyStream(ctx context.Context, texts <-chan string) (<-chan StreamItem, error)
func (c *EfeClient) Healthcheck(ctx context.Context) error
func (c *EfeClient) Close() error

// ==== Options (10) ====
func WithJWT(token string) ClientOption
func WithMTLS(certPath, keyPath, caPath string) ClientOption
func WithTLS(serverNameOverride string) ClientOption
func WithDeadline(d time.Duration) ClientOption
func WithRetry(maxAttempts int, backoff RetryBackoff) ClientOption
func WithOTel(tp trace.TracerProvider) ClientOption
func WithPrometheus(reg prometheus.Registerer) ClientOption
func WithLogger(logger *slog.Logger) ClientOption
func WithUserAgent(ua string) ClientOption
func WithDialTimeout(d time.Duration) ClientOption

// ==== Types ====
type EfeClient struct{ /* opaque */ }
type Verdict struct {
    Label          string
    Confidence     float64
    Reject         bool
    Rounds         int
    AlignmentDelta float64
    Catalog        string
    Zone           string
    SwarmID        string
    RequestID      string
    ElapsedMs      int64
}
type StreamItem struct {
    Index   int
    Verdict *Verdict
    Err     error
}
type ClientOption func(*options) error
type RetryBackoff interface{ Next(attempt int) time.Duration }
type ExponentialBackoff struct{ /* ... */ }

// ==== Errors (10 sentinels) ====
var ErrUnauthenticated, ErrPermissionDenied, ErrCatalogNotFound,
    ErrCatalogInvalid, ErrTimeout, ErrUnavailable, ErrRateLimited,
    ErrInvalidArgument, ErrInternal, ErrClosed error

func IsRetryable(err error) bool

// ==== Constants ====
const Version = "0.3.0"
```

---

## §27. Annexes — Makefile + scripts

```makefile
# clients/go/efe-sdk/Makefile

.PHONY: gen-proto build test lint cover bench examples publish

gen-proto:
	@./scripts/gen-proto.sh

build:
	@go build ./...

test:
	@go test -race -coverprofile=coverage.out ./...

cover: test
	@go tool cover -func=coverage.out
	@go tool cover -html=coverage.out -o coverage.html

lint:
	@golangci-lint run ./...
	@go vet ./...

bench:
	@go test -bench=. -benchmem ./...

examples:
	@for f in examples/*; do (cd "$$f" && go build ./...); done

publish:
	@./scripts/publish.sh
```

---

## §28. Annexes — RACI + budget + Gantt

### §28.1 RACI

| Activité | R | A | C | I |
|:--:|:--:|:--:|:--:|:--:|
| Module skeleton + protoc | Go Eng | Tech Lead | DevEx | Eng VP |
| Functional options + EfeClient | Go Eng | Tech Lead | API team | Eng VP |
| Errors + retry | Go Eng | Tech Lead | SRE | Eng VP |
| OTel + Prom integration | Go Eng | Tech Lead | Observability team | Eng VP |
| 3 framework examples | Go Eng | Tech Lead | Product DX | Eng VP |
| Mock server + bufconn | Go Eng | Tech Lead | QA Lead | Eng VP |
| Tests + coverage ≥ 80% | Go Eng | QA Lead | Tech Lead | Eng VP |
| Doc + godoc | Go Eng | Tech Lead | Product DX | Eng VP |
| Tag + publish | Release Eng | Tech Lead | SecOps | Eng VP |
| Workshop interne | Tech Lead | Eng VP | Go consumers team | C-suite |

### §28.2 Budget

| Item | Coût |
|:-|--:|
| Eng D23 (10 j × $1 200) | $12 000 |
| GitHub Actions CI minutes | $50/mo |
| `pkg.go.dev` listing | $0 (free) |
| godoc hosting | $0 (proxy-hosted) |
| **Total one-shot** | **$12 050** |

### §28.3 Gantt prévisionnel

```text
Sprint     D23
J          12345678910
SDK actif  ████████─        (J1-J7)
Examples           ██       (J8)
Tests CI            ██      (J9)
Doc + tag            ██     (J10)
                       ★    Tag v0.3.0
```

---

## §29. Mises à jour du document

### §29.1 Versioning

* v1.0.0 — 2026-04-29 — Première édition (cette version).

### §29.2 Liens à maintenir

* `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §6.4 — sprint D23.
* `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` — pattern de
  référence (M8). M9 calque la surface conceptuelle.
* `LETTER_E_ARCHITECTURE.md` — service backend que le SDK
  consomme.
* `protos/efe/v1/*.proto` — wire contract.

### §29.3 Critères d'archive

* Le sprint D23 a shippé.
* Le `CHANGELOG.md` `[0.3.0]` entry est livré (privé).
* Tag `efe-sdk-go@v0.3.0` poussé sur le repo privé.
* Pin tests Python `test_d23_go_sdk_*` GREEN.
* Workshop équipe Go consumers complété.
* GA (post-D40) : tag `v1.0.0` public ; entry archive séparée.

---

**Fin du document.**

Total LOC du document : ~2 040 lignes Markdown.
Total LOC delta cible (M9 D23) : ~2 150 LOC Go actif + ~1 200 LOC
tests + ~550 LOC mock + ~520 LOC examples + ~3 000 LOC généré +
~530 LOC doc + ~400 LOC pin tests = **~8 350 LOC**.
Total durée : **10 jours-ingénieur** sur 1 sprint atomique (Φ-B).
Budget : **$12 050**.
