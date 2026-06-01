# `@efe/sdk` (TypeScript SDK) — Architecture détaillée de l'extension M10

> **Statut** : Draft v1.0.0 — Document de référence pour la
> **création from-scratch** du module **M10 = `clients/typescript/@efe/sdk/`**.
> **Greenfield** : ce module n'existe PAS encore (✓ vérifié AS-IS au
> 2026-04-29 ; `clients/` ne contient que `python/` + `go/`).
> **Auteur** : rodrigue.pekar@icloud.com
> **Date** : 2026-04-29.
>
> **Dépendances doc** :
>   * `docs/PRODUCT_COMPLETION_ARCHITECTURE.md` §6.4 — sprint D24
>     dans Φ-B, et §15 — surface SDK multi-runtime.
>   * `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` — pattern de
>     référence pour le SDK Python (M8) ; le TS SDK suit la même
>     surface conceptuelle (`Verdict`, `EfeClient`, `Classify`)
>     adaptée aux idiomes TypeScript (Promise, options object,
>     classes d'erreur).
>   * `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md` — pattern de
>     référence pour le SDK Go (M9), notamment la chaîne
>     d'interceptors gRPC + le mode mock bufconn (équivalent
>     mock-server in-memory en TS).
>   * `LETTER_E_ARCHITECTURE.md` — service backend single-tenant que
>     le SDK consomme (via gRPC, post-D23).
>   * `MODE1_ARCHITECTURE.md` — contrat Mode-1 (la classification que
>     ce SDK appelle).
>   * `protos/efe/v1/*.proto` — wire contract (efe.proto + types.proto
>     + catalog.proto + admin.proto + mode2.proto + mode3.proto).
>
> **Portée** : **1 sprint atomique D24** (Φ-B), 10 jours
> wallclock. Création complète d'un package npm publishable :
> client gRPC Node `@grpc/grpc-js` + client grpc-web browser
> (subset RPCs idempotents only) + types TS auto-générés depuis
> protos via `@bufbuild/protobuf-es` + OTel auto-instrumentation
> (universel) + retry exponentiel (Node) / no-retry browser
> (idempotent + caller responsibility) + auth JWT Bearer (universel)
> + mTLS Node-only + 3 exemples framework (React + Vue + Express)
> + tests Vitest unit/integration + Playwright e2e browser +
> publication npm scoped private puis public à GA.
>
> **Cible GA** : `@efe/sdk@v1.0.0` packagé publiquement sur npm
> registry ; aligné semver-wise avec le produit (D24 = v0.3.0
> privé via npm scoped `--access=restricted`, GA = v1.0.0 public
> `--access=public`).

---

## Table des matières

§0. Méta, glossaire, conventions
§1. Mission de l'extension M10 (Φ-B)
§2. Position dans la topologie EFE post-M9
§3. État AS-IS au 2026-04-29 (greenfield)
§4. Définition opérationnelle de M10 v1.0 GA
§5. Principes directeurs (idiomes TypeScript)
§6. Invariants structurels (layout package npm)
§7. Vue d'ensemble — sprint D24 unique
§8. Surface publique cible (API TS publishable)
§9. **Sprint D24 — package npm entier**
§10. Sub-système 1 — `EfeClient` (Node) + `EfeWebClient` (browser)
§11. Sub-système 2 — Type generation pipeline (`@bufbuild/protobuf-es` + `buf` CLI)
§12. Sub-système 3 — Errors + retry + idempotence policy
§13. Sub-système 4 — OpenTelemetry auto-instrumentation
§14. Sub-système 5 — Prometheus metrics (Node) + browser metrics opt-in
§15. Sub-système 6 — Auth (JWT Bearer + mTLS Node-only)
§16. Sub-système 7 — 3 framework examples (React + Vue + Express)
§17. Sub-système 8 — Mock infrastructure (Vitest fakes + Playwright fixtures)
§18. Stratégie de tests (Vitest unit + Playwright e2e)
§19. Browser limitations + grpc-web subset enumeration
§20. Bundle size budget + tree-shaking + dual ESM/CJS exports
§21. SLO mapping (cold start + bundle size + latency overhead)
§22. Risques transverses + mitigations
§23. Volumétrie code attendue
§24. Plan d'implémentation J1-J10
§25. Versioning + publication npm (private → public à GA)
§26. Compliance + sécurité (CSP, supply-chain, redaction)
§27. Annexes — `package.json` canonique
§28. Annexes — surface API TS complète (`.d.ts`)
§29. Annexes — `buf.yaml` + `buf.gen.yaml` + scripts
§30. Annexes — RACI + budget + Gantt
§31. Mises à jour du document

---

## §0. Méta, glossaire, conventions

### §0.1 Conventions

* **AS-IS** = état du repo à fin D23 (post-Go SDK livré, audit pass 1
  appliqué, v0.3.0 publishable privé).
* **TO-BE** = état cible défini par le sprint D24.
* Le sprint D24 est **dans la phase Φ-B** (Lettre E + multi-runtime
  SDKs). D21 (Lettre E + AdminService) et D22 (mTLS + JWT scopes) +
  D23 (Go SDK) sont des prérequis cohérents — D24 réutilise la
  même `efe.v1` proto + chaîne d'interceptors logique + même 10
  sentinels d'erreur (cross-language consistency).
* Les défauts résiduels suivent **DT-T.x** (Defect Trace, TS SDK).
* **Package npm** = `@efe/sdk` à publier sur le npm registry. Pendant
  le développement (avant publication), le path filesystem est
  `clients/typescript/@efe/sdk/`. Le scope `@efe` est privé pendant
  Φ-B (`npm publish --access=restricted`) puis public à GA
  (`npm publish --access=public`).
* **Symboles `EfeClient`, `EfeWebClient`, `Verdict`,
  `ClientOptions`** sont les noms publics canoniques (PascalCase
  TS). Les méthodes en camelCase (`classify`, `healthcheck`,
  `close`).
* **Verdict TypeScript** = même 10 fields que Python/Go SDKs, mêmes
  noms de propriété (camelCase TS vs snake_case wire JSON via
  `bufbuild/protobuf-es` qui génère les deux).
* `[ref:fichier:ligne]` = pointeur dans le code source.

### §0.2 Glossaire

| Terme | Définition |
|-------|------------|
| **`@efe/sdk`** | Le package npm `clients/typescript/@efe/sdk/` qui héberge le SDK TypeScript pour EFE. Distribué en ESM + CJS dual via `package.json` `exports`. AS-IS : 0 LOC (greenfield). |
| **gRPC client (Node)** | Le code généré par `buf generate` (via `@bufbuild/protoc-gen-es` + `@connectrpc/protoc-gen-connect-es`) à partir de `protos/efe/v1/*.proto`, transporté par `@grpc/grpc-js` (HTTP/2 natif Node). |
| **grpc-web client (browser)** | Subset RPC consommable depuis un navigateur via le protocole gRPC-Web (HTTP/1.1 wrapper) ; transporté par `@connectrpc/connect-web` qui parle gRPC-Web et Connect-Protocol. |
| **`EfeClient`** | La classe publique Node qui wrappe le gRPC client + options (auth, OTel, retry, mTLS). Construite via `new EfeClient({ endpoint, ...opts })`. Tous les RPCs disponibles (Classify, Healthcheck, RegisterCatalog, …). |
| **`EfeWebClient`** | La classe publique browser, **subset** de `EfeClient` exposant **uniquement les RPCs idempotents** (Classify, Healthcheck). Pas de RegisterCatalog / EvolveCatalog / DeleteCatalog (write RPCs interdits depuis browser pour des raisons de sécurité — un BFF doit servir d'intermédiaire). |
| **`Verdict`** | L'interface TS publique des résultats de classification (équivalent du `Verdict` Python `@dataclass(frozen=True)` de M8 + `Verdict` Go struct de M9). 10 fields cross-language pinnés. |
| **`ClientOptions`** | Le type `interface` TS qui paramètre `EfeClient` (auth, TLS, retry, OTel, etc.). Idiome TS canonique : objet de configuration au constructeur (vs functional options Go ou kwargs Python). |
| **Idempotent RPC** | RPC qui peut être ré-exécuté sans effet de bord (Classify ✓, Healthcheck ✓, GetCatalog ✓). Les non-idempotents (RegisterCatalog, EvolveCatalog, DeleteCatalog, AdminService.*) ne sont **PAS** exposés dans `EfeWebClient`. |
| **`buf`** | CLI moderne de management proto (`buf.build`). Remplace `protoc + protoc-gen-go-grpc` chain par une config YAML (`buf.yaml` + `buf.gen.yaml`) déclarative. Génère TS via `@bufbuild/protoc-gen-es` (types) + `@connectrpc/protoc-gen-connect-es` (RPC clients). |
| **`@bufbuild/protobuf-es`** | Lib runtime ESM-first, tree-shakeable, qui matérialise les messages proto en classes TS. Bundle size 5-10× plus petit que `google-protobuf` legacy. |
| **`@connectrpc/connect`** | Lib transport agnostique : un seul code TS produit qui parle gRPC (Node), gRPC-Web (browser), ou Connect-Protocol (HTTP/1.1+JSON). On utilise `connect-node` pour le client Node + `connect-web` pour le client browser. |
| **`MockEfeClient`** | Classe TS de test, drop-in `EfeClient` que les consumers utilisent dans Vitest. API : `setVerdict()`, `setError()`, `getCalls()`, `reset()`. |
| **In-memory mock server** | Serveur gRPC qui tourne en mémoire dans le même process Node — utilisé par les tests d'intégration Vitest pour exercer la chaîne d'interceptors complète. Pas l'équivalent du `bufconn` Go (Node n'a pas d'analogue exact) ; on utilise `@grpc/grpc-js` avec un port aléatoire ou un pipe Unix domain socket. |
| **Playwright fixtures** | Fixtures e2e qui spawn un serveur gRPC mock + un dev server Vite + un navigateur Chromium/WebKit/Firefox. Les tests browser exercent le grpc-web client end-to-end (HTTP/1.1 envoyé, réponse parsed, Verdict displayé). |
| **Tree-shakeable** | Build qui permet aux bundlers (esbuild, rollup, webpack, vite) d'éliminer le code mort à la compilation. Critique pour le bundle browser : on ne veut pas pull les sentinels Node-only quand l'app cible le browser. |
| **CSP-friendly** | Compatible avec une `Content-Security-Policy` stricte (no `eval`, no `Function()`, no inline scripts). Le SDK browser doit fonctionner sous `script-src 'self'`. |
| **CORS preflight** | Requête HTTP `OPTIONS` envoyée par le browser avant la requête gRPC-Web réelle pour vérifier que le serveur autorise les headers `authorization` + `content-type: application/grpc-web+proto`. Le serveur EFE doit retourner les bons `Access-Control-Allow-*` headers. |

### §0.3 Notation

* Les blocs `text-with-tabs` sans surlignage = texte canonique copié
  tel quel dans le code TS / package.json.
* Les blocs ```ts = TypeScript pseudo-code didactique.
* Les blocs ```bash = commandes shell exécutables.
* Tableaux pin tests = id `DT-T.<sprint>.<n>` à matérialiser dans
  `scripts/tests/test_d24_typescript_sdk_extension.py`.

---

## §1. Mission de l'extension M10 (Φ-B)

**Phrase canonique** :

> Doter EFE d'un **SDK TypeScript** publishable npm, **isomorphe**
> (Node + Browser), **type-safe end-to-end** (génération depuis proto),
> **bundle-size optimisé** (< 50 KB gz browser), **observable** (OTel
> auto), et **inclusif** (3 framework examples React + Vue + Express)
> — **en 1 sprint atomique D24** de 10 jours wallclock.

**Pourquoi maintenant (Φ-B post-M9)** :

1. **Demande consumer** — 60% des consumers EFE pre-D23 prototypaient
   en TypeScript (frontend BFF Express, dashboard React) en bricolant
   `fetch()` sur l'API REST proxy. Pas de type safety, pas de retry,
   pas d'OTel, pas de validation à la construction. **Le besoin
   d'un SDK natif TS est antérieur à D24** ; M10 le canonise.

2. **Cohérence cross-language** — M8 a livré le SDK Python (`efe-tool`
   CLI + `EfeFacade`), M9 a livré le SDK Go (`efe-sdk-go`). Sans M10,
   le 3ème écosystème majeur (TS/Node/JS) reste sur du fetch DIY,
   créant une asymétrie d'expérience pour les architects qui choisissent
   leur stack. Le pin test `test_m10_verdict_field_set_matches_m8_m9`
   anchor cette cohérence (mêmes 10 fields, mêmes JSON tags).

3. **Browser-side classification** — Use case réel : un dashboard
   Common Room qui montre des verdicts EFE inline (par exemple "ce
   ticket support sera classé `billing` à 0.91 confiance"). Sans SDK
   browser, l'app doit faire un round-trip BFF → backend → BFF, ce qui
   double la latence + complique l'implémentation. Le client grpc-web
   subset (idempotent only) résout ce besoin.

4. **Cohérence avec PRODUCT_COMPLETION** — `PRODUCT_COMPLETION_ARCHITECTURE.md`
   §6.4 liste D24 comme l'avant-dernier sprint Φ-B (avant D25 = FFI
   Python ↔ Rust + D26 = release engineering). M10 est le dernier
   SDK greenfield ; M11+ sera de la maintenance + de l'évolution.

**Non-objectifs** (explicitement OUT du sprint D24) :

* **Streaming bidi côté browser** — gRPC-Web protocol ne supporte que
  server-streaming (pas client-streaming, pas bidi). Le client browser
  exposera **uniquement les RPCs unary** (Classify + Healthcheck) en D24.
  Streaming server-side (`ClassifyStream` à la Go) reporté à D26+ pour
  le browser.
* **Module path npm renommage post-GA** — Une fois v1.0.0 publié sous
  `@efe/sdk`, le path est immuable. Le bump majeur (v2.0.0) requiert
  un nouveau scope ou un sub-path import migration.
* **Polyfills legacy browsers** — Cible : ES2022 + modules ESM. Pas de
  IE11, pas de Safari < 15, pas de Node < 18. Les consumers qui
  veulent supporter du legacy doivent transpiler eux-mêmes (Babel,
  TypeScript `target: "es5"`).
* **Edge runtime support garanti** — Cloudflare Workers / Vercel Edge /
  Deno Deploy ont des runtimes JS limités (pas de TCP socket, pas de
  HTTP/2 client). Le SDK browser fonctionne sur ces runtimes (HTTP/1.1
  fetch), le SDK Node ne fonctionne PAS. Pin test `test_d24_edge_subset`.
* **Real-time WebSocket fallback** — Pas de fallback WebSocket si
  HTTP/2 indisponible côté Node. La doc spécifie clairement que Node
  18+ = HTTP/2 disponible.

---

## §2. Position dans la topologie EFE post-M9

```
┌───────────────────────────────────────────────────────────────────────┐
│                 EFE Topology — post-M9 livré (D23)                    │
└───────────────────────────────────────────────────────────────────────┘

  Layer L4 — Consumer surface
  ┌───────────────────────────────────────────────────────────────────┐
  │  efe-tool CLI (Python)         efe-sdk-go (Go)        ??? (TS)    │
  │  EfeFacade(client_kind="auto")  EfeClient.Classify()    ↑↑↑       │
  │                                                                   │
  │   ↑ M8 livré D19/D20/D25         ↑ M9 livré D23   ←  M10 GAP ici  │
  └───────────┬───────────────────────────┬─────────────────┬─────────┘
              │ gRPC                      │ gRPC            │ gRPC + grpc-web
              ▼                           ▼                 ▼
  Layer L3 — Wire contract (efe.v1)
  ┌───────────────────────────────────────────────────────────────────┐
  │   protos/efe/v1/*.proto — types.proto, efe.proto, catalog.proto,  │
  │   admin.proto, mode2.proto, mode3.proto                           │
  └───────────────────────────────────────────────────────────────────┘
              │                          │                  │
              ▼                          ▼                  ▼
  Layer L2 — Server (Lettre E single-tenant + mTLS + JWT scopes)
  ┌───────────────────────────────────────────────────────────────────┐
  │  efe-server (Rust) — exposes :50051 gRPC + :50052 gRPC-Web         │
  │  D21 + D22 livrés                                                 │
  └───────────────────────────────────────────────────────────────────┘
              │
              ▼
  Layer L1 — Compute kernel (Rust + LibTorch + cones)
  ┌───────────────────────────────────────────────────────────────────┐
  │  efe-core (LibTorch) + efe-cones (alignment) + efe-fusion          │
  │  M3..M7 livrés (D1..D18)                                          │
  └───────────────────────────────────────────────────────────────────┘
```

**Où M10 s'insère** :

* **Layer L4 (consumer surface)** — comble le `??? (TS)` gap. Après
  M10, les 3 langages stratégiques (Python, Go, TS) sont couverts.
* **Layer L3 (wire contract)** — M10 consomme **les mêmes** `protos/efe/v1/*.proto`
  que M8/M9. Aucune extension proto requise. Les pin tests
  `test_d24_typescript_sdk_protos_in_sync` matchent les SHA-256 des
  protos contre `clients/typescript/@efe/sdk/src/_generated/PROTOS_HASH.txt`.
* **Layer L2 (server)** — Lettre E doit exposer **deux ports** post-D24 :
  * `:50051` = gRPC natif (HTTP/2) — utilisé par M8 (Python) + M9 (Go)
    + M10 Node.
  * `:50052` = gRPC-Web (HTTP/1.1) — utilisé par M10 Browser. Ce port
    requiert une couche de translation (Envoy ou native) qui doit
    exister AVANT D24. Si elle n'existe pas, on déploie un sidecar
    Envoy (`grpcweb` filter) en D24 J1.
* **Layer L1 (compute)** — pas d'impact M10.

**Couplage M10 ↔ M8/M9** : 0 dépendance fonctionnelle. Les 3 SDKs
sont indépendants ; ils partagent UNIQUEMENT le wire contract proto.
Cross-language consistency est validée par pin tests Python qui
charge les 3 SDKs et compare les surfaces.

---

## §3. État AS-IS au 2026-04-29 (greenfield)

### §3.1 Ce qui existe déjà

* `clients/python/efe_tool/` — SDK Python (M8 livré + audit pass 1
  GREEN). 0 surface TS.
* `clients/go/efe-sdk/` — SDK Go (M9 livré + audit pass 1 GREEN).
  0 surface TS.
* `protos/efe/v1/*.proto` — wire contract stable depuis D17 (modulo
  ajouts compatible-additive D18..D22).
* `docs/EFE_PYTHON_SDK_EXTENSION_ARCHITECTURE.md` (2 359 lignes) —
  référence M8.
* `docs/EFE_GO_SDK_EXTENSION_ARCHITECTURE.md` (2 257 lignes) —
  référence M9.

### §3.2 Ce qui n'existe PAS encore

* `clients/typescript/` — répertoire entier inexistant. **AS-IS = 0 fichier**.
* `package.json` racine TS — inexistant.
* `tsconfig.json` — inexistant.
* `buf.yaml` + `buf.gen.yaml` — inexistant (le repo n'a jamais utilisé
  buf jusqu'à présent ; M8/M9 utilisaient `protoc` direct).
* CI workflows `.github/workflows/sdk-typescript-test.yml` +
  `sdk-typescript-smoke.yml` — inexistants.
* Pin tests Python `scripts/tests/test_d24_typescript_sdk_extension.py` — inexistants.
* CHANGELOG entrée M10 — inexistante.

### §3.3 Capacités de l'environnement de dev

* **Node.js** : on suppose 18+ (LTS récent) installé. Si absent, le
  CI `actions/setup-node@v4` provisionne. Localement, le dev install
  via `nvm` ou direct.
* **npm** : version ≥ 10 (ships with Node 20). On utilise `pnpm`
  pour le monorepo car il gère mieux les workspaces que npm — voir
  §6.5 décision.
* **buf CLI** : à installer via `brew install bufbuild/buf/buf` ou
  équivalent par OS. Installation pré-requise au sprint J1.
* **Playwright browsers** : install via `npx playwright install
  chromium webkit firefox` (≈ 600 MB total) — installé en CI au
  premier run, caché dans `~/Library/Caches/ms-playwright/`.

### §3.4 Vérification AS-IS

Commande de vérification au 2026-04-29 :

```bash
ls -la clients/
# ATTENDU : drwxr-xr-x ... go/  python/
# (PAS de typescript/)

find clients -name 'package.json' -o -name 'tsconfig.json' 2>/dev/null
# ATTENDU : (vide)
```

→ **Greenfield confirmé**. M10 part de zéro.

---

## §4. Définition opérationnelle de M10 v1.0 GA

### §4.1 Scope fonctionnel

M10 est **GA-ready** quand toutes les capabilities suivantes sont
livrées + tests verts + pin tested :

| Capability | Surface | Test plan |
|---|---|---|
| **Construct + close client (Node)** | `new EfeClient({...})` + `client.close()` | Vitest unit |
| **Construct + close client (Browser)** | `new EfeWebClient({...})` + `client.close()` | Playwright e2e |
| **Classify (Node)** | `await client.classify(text, opts?)` → `Verdict` | Vitest integration vs in-memory mock |
| **Classify (Browser)** | `await client.classify(text, opts?)` → `Verdict` | Playwright e2e vs Envoy proxy |
| **Healthcheck (Node + Browser)** | `await client.healthcheck()` → `void` (throws on not-serving) | Vitest + Playwright |
| **JWT auth (Node + Browser)** | `withAuth({ jwt: '...' })` option | Vitest spec — header asserted on mock |
| **mTLS auth (Node only)** | `withMTLS({ certPath, keyPath, caPath })` | Vitest spec — TLS config produced |
| **Retry + backoff (Node only)** | `withRetry({ maxAttempts, backoff })` | Vitest spec — 3 attempts on Unavailable |
| **OTel auto (Node + Browser)** | `withOTel(tracerProvider)` — soft dep | Vitest — span shape |
| **Prometheus metrics (Node only)** | `withPrometheus(registry)` — 5 collectors canoniques | Vitest — names match `efe_sdk_*` |
| **3 framework examples** | React (hook), Vue (composable), Express (middleware) | Each example builds + runs |
| **Mock for unit tests** | `MockEfeClient` class + `setVerdict/setError/getCalls` | Vitest — golden path |
| **Type generation pipeline** | `pnpm gen:proto` regenerates `_generated/` | CI step |
| **Bundle size budget** | `< 50 KB gz` for browser entry | CI step `pnpm bundlesize` |
| **ESM + CJS dual** | `package.json` exports validates with both `import` + `require` | Vitest matrix Node 18/20/22 |

### §4.2 Hors scope (D24)

* **Streaming RPCs côté browser** — gRPC-Web protocol limitation
  (server-streaming OK ; client-streaming + bidi NON). Reporté D26+.
* **`ClassifyStream` API** (à la Go SDK ClassifyStream worker pool)
  — Node OK in D24 (delivered as a worker pool wrapper around
  Promise.all + concurrency limit) ; browser reporté.
* **AdminService RPCs** — write RPCs (RegisterCatalog, EvolveCatalog,
  DeleteCatalog) exposés UNIQUEMENT dans `EfeClient` (Node) ; pas
  dans `EfeWebClient` (browser). Le pattern : la browser app passe
  par un BFF qui détient `@efe/sdk` Node + scope admin ; l'app
  browser détient `@efe/sdk` browser + scope `data:read` only.
* **Hot reload connection** — pas de reconnexion automatique au
  serveur en cas de drop. Le caller catch + retry au niveau application.
* **Custom encoder/codec** — on utilise le binary proto encoding.
  Pas de JSON wire format (Connect-Protocol JSON variant) en D24
  pour minimiser le bundle browser.

### §4.3 Acceptance criteria

```text
M10 v1.0 GA = TOUS les items suivants ✓ :

[A] Surface API publique
  ✓ EfeClient class (Node) avec 4 méthodes publiques
  ✓ EfeWebClient class (Browser) avec 2 méthodes publiques
  ✓ Verdict interface — 10 fields cross-lang pinnés
  ✓ ClientOptions interface
  ✓ 10 sentinel error classes (extends Error)
  ✓ isRetryable(err) helper
  ✓ errorCode(err) helper

[B] Type generation pipeline
  ✓ buf.yaml + buf.gen.yaml committed
  ✓ pnpm gen:proto regenerates idempotently
  ✓ PROTOS_HASH.txt anti-drift pin tested

[C] Tests
  ✓ Vitest unit tests : ≥ 30 passants
  ✓ Vitest integration : ≥ 10 passants vs in-memory mock
  ✓ Playwright e2e : ≥ 5 passants (Chromium + WebKit + Firefox)
  ✓ Coverage > 80%

[D] Examples
  ✓ React example : pnpm dev compile + page load OK
  ✓ Vue example  : idem
  ✓ Express example : pnpm dev start + curl /classify OK

[E] CI/CD
  ✓ sdk-typescript-test.yml — matrix 3 Node × 3 OS
  ✓ sdk-typescript-smoke.yml — daily live vs staging
  ✓ sdk-typescript-publish.yml — auto on tag

[F] Publication
  ✓ npm publish --access=restricted @efe/sdk@0.3.0 OK (private)
  ✓ Test-install dans repo bidon : pnpm add @efe/sdk --registry=...
  ✓ ESM import + CJS require both work
  ✓ Bundle browser < 50 KB gz
```

### §4.4 SLO finaux

| ID | Cible | Mesure |
|----|-------|--------|
| `SLO-TS-1` | `new EfeClient(...)` cold start < 30 ms | Vitest microbenchmark |
| `SLO-TS-2` | `classify` p99 ≤ server p99 + 30 ms | Vitest integration |
| `SLO-TS-3` | Bundle size browser ≤ 50 KB gz | CI `bundlesize` step |
| `SLO-TS-4` | Bundle size Node ≤ 250 KB gz | CI step (Node = full proto + grpc-js) |
| `SLO-TS-5` | Tree-shake : import `{ Verdict }` only → dead-code-elim metrics | bundle visualizer |
| `SLO-TS-6` | TypeScript strict ≥ 5.4 supported | tsconfig CI |
| `SLO-TS-7` | typedoc clean on https://efe-ai.github.io/sdk-typescript/ | docs CI step |
| `SLO-TS-8` | Coverage ≥ 80% lines | vitest coverage |

---

## §5. Principes directeurs (idiomes TypeScript)

### §5.1 Five guiding principles

1. **Type safety end-to-end** — chaque RPC, chaque option, chaque
   error code est typé statiquement. `ts-strict` mode ON. `noImplicitAny`,
   `strictNullChecks`, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`.
   Aucun `any` dans la surface publique.

2. **Isomorphic by design, not by accident** — le code partagé (Verdict,
   errors, options, retry) vit dans `src/core/`. Le code Node-only
   vit dans `src/node/`. Le code browser-only vit dans `src/web/`.
   Le `package.json` `exports` route conditionnellement (`"node"` vs
   `"browser"` vs `"default"`). Pas d'import croisé : un fichier
   `src/web/*.ts` ne peut PAS `import` `src/node/*.ts` (CI step
   eslint check pin tested).

3. **Functional + immutable où possible** — Verdict est `readonly`
   (TS). Les options sont passées au constructeur, pas mutables après.
   `EfeClient` instances sont par contre mutables côté state interne
   (closed flag, in-flight counter) — comme en Go.

4. **Soft deps for observability** — OTel + Prom sont peerDependencies
   optionnelles. Si l'app les installe, l'SDK les wire automatiquement.
   Si non, no-op providers sont utilisés (zéro overhead). Pin test
   `test_d24_otel_no_op_when_unset`.

5. **Bundle size is a first-class concern** — chaque dépendance
   ajoutée doit justifier son coût. On préfère re-implémenter une
   logique de retry simple (50 LOC) plutôt qu'importer
   `axios-retry` (3 KB). Le CI step `bundlesize` bloque tout PR
   qui dépasse 50 KB gz browser.

### §5.2 Anti-patterns explicites

| Anti-pattern | Pourquoi NON | Alternative TS |
|---|---|---|
| Default exports (`export default class EfeClient`) | Renommage IDE ne propage pas, tree-shake limité | `export class EfeClient` named export |
| `any` dans la surface publique | Casse la type-safety | `unknown` + type guards, ou un union type explicite |
| Throw `string` (`throw "boom"`) | Stack trace perdue, instanceof inutile | `throw new EfeError(...)` extends Error |
| Callback API (`client.classify(text, (err, v) => {...})`) | Pas idiomatic TS moderne | `Promise<Verdict>` + `await` |
| `Function.prototype.bind` magique | Difficile à tree-shake, type unsafe | Arrow functions + closures |
| Fetch interceptors via monkey-patching `globalThis.fetch` | Casse autres consumers du fetch global | Pass un `fetch` instance via options |
| Synchronous I/O dans la chaîne client (Node `readFileSync`) | Bloque event loop | `fs.promises.readFile` async |
| `process.env.X` lu au niveau module-load | Empêche les tests, casse browser | Lu uniquement dans constructeur si fallback |

### §5.3 Idiomes TypeScript canoniques choisis

* **Constructor options object** — `new EfeClient({ endpoint: '...', auth: {...} })`.
  Pas de constructor params positionnels (devient illisible au-delà
  de 3 params).
* **`Readonly<T>`** sur la Verdict — empêche les mutations accidentelles.
* **Discriminated unions** pour les options auth :
  ```ts
  type AuthConfig =
    | { kind: 'jwt'; token: string }
    | { kind: 'mtls'; certPath: string; keyPath: string; caPath: string }
    | { kind: 'none' };
  ```
* **`Promise<T>` + `async/await`** pour toute I/O. Pas de callback.
* **`Symbol`-keyed sentinel errors** pour permettre `errors.is`-style
  detection sans casser les error chains :
  ```ts
  const ErrTimeoutSymbol = Symbol.for('@efe/sdk:ErrTimeout');
  class ErrTimeout extends Error {
    readonly [ErrTimeoutSymbol] = true;
  }
  ```
  Voir §12 pour le pattern complet.
* **AbortController + AbortSignal** pour la cancellation (équivalent
  Go context). Tous les RPCs acceptent un `signal?: AbortSignal` dans
  les options du call.

### §5.4 Decision : `pnpm` vs `npm` vs `yarn`

**Décision** : `pnpm@9` pour le monorepo workspaces.

**Rationale** :
* **pnpm** : workspaces natifs, hard-link store (gain disk + install
  speed), `pnpm overrides` pour pin transitif (sécurité supply-chain).
  C'est le défaut de la communauté TS moderne (2024+).
* **npm** : workspaces fonctionnels mais lents, pas de store partagé,
  history compatibility issues sur les hoists.
* **yarn** : `yarn berry` (v3+) introduit PnP qui casse certains
  packages ; `yarn classic` deprecated.

**Pin test** : `test_d24_uses_pnpm` vérifie `packageManager: "pnpm@9.x.x"`
dans le `package.json`.

### §5.5 Decision : `tsup` vs `tsc` vs `vite` pour build

**Décision** : `tsup` (esbuild-based) pour le bundling distribution,
`tsc --noEmit` pour le type-check.

**Rationale** :
* **`tsup`** : ESM + CJS dual output natif, dts générés, esbuild
  ultra-rapide (200ms typique), config minimale (`tsup.config.ts`).
  C'est le standard de facto pour les libs TS modernes.
* **`tsc`** : marche mais pas de bundling native (chaque fichier .ts
  → 1 fichier .js, pas idéal pour browser). Pas de dual ESM/CJS easy.
* **`vite`** : excellent pour les apps, overkill pour une lib.

`tsc` reste utilisé pour :
1. Type-check des sources avant build (`pnpm typecheck`).
2. Génération des `.d.ts` pour les `_generated/` (les protoc-gen-es
   types).

**Pin test** : `test_d24_build_uses_tsup` vérifie l'existence de
`tsup.config.ts` + le script `pnpm build`.

---

## §6. Invariants structurels (layout package npm)

### §6.1 Filesystem layout (frozen)

```
clients/typescript/@efe/sdk/
├── package.json                          ← npm manifest (publish unit)
├── tsconfig.json                         ← TS strict config
├── tsconfig.build.json                   ← extended pour build
├── tsup.config.ts                        ← bundling config (ESM + CJS + dts)
├── vitest.config.ts                      ← test runner config
├── playwright.config.ts                  ← e2e browser config
├── .npmignore                            ← exclude src/, tests/, examples/ from publish
├── .gitignore                            ← node_modules/, dist/, coverage/
├── README.md                             ← quickstart + 3 frameworks pointers
├── LICENSE                               ← Apache-2.0
├── CHANGELOG.md                          ← per-version release notes
│
├── buf.yaml                              ← buf module config (proto sources)
├── buf.gen.yaml                          ← codegen config (protoc-gen-es + connect-es)
│
├── src/                                  ← TS source (NOT shipped to npm directly)
│   │
│   ├── index.ts                          ← universal entrypoint (re-exports core)
│   ├── node.ts                           ← Node-only entrypoint
│   ├── web.ts                            ← Browser-only entrypoint
│   │
│   ├── core/                             ← shared (Node + Browser)
│   │   ├── verdict.ts                    ← Verdict + StreamItem types
│   │   ├── errors.ts                     ← 10 sentinels + helpers
│   │   ├── options.ts                    ← ClientOptions type + validators
│   │   ├── retry.ts                      ← ExponentialBackoff (universal)
│   │   ├── auth.ts                       ← JWT bearer header builder
│   │   ├── otel.ts                       ← OTel span helper (peer dep)
│   │   ├── from-rpc.ts                   ← RPC error → sentinel mapping
│   │   ├── version.ts                    ← const Version
│   │   └── _types.ts                     ← internal type aliases
│   │
│   ├── node/                             ← Node-only code
│   │   ├── client.ts                     ← EfeClient class
│   │   ├── transport.ts                  ← @grpc/grpc-js wrapper
│   │   ├── tls.ts                        ← mTLS config loader
│   │   ├── interceptors.ts               ← unary/stream interceptors chain
│   │   ├── metrics.ts                    ← Prometheus collectors
│   │   └── stream.ts                     ← classifyStream (worker pool)
│   │
│   ├── web/                              ← Browser-only code
│   │   ├── client.ts                     ← EfeWebClient class
│   │   ├── transport.ts                  ← @connectrpc/connect-web wrapper
│   │   ├── interceptors.ts               ← grpc-web interceptors (subset)
│   │   └── auth.ts                       ← browser-friendly auth (no fs)
│   │
│   ├── mock/                             ← test helpers (re-exported from /mock)
│   │   ├── client.ts                     ← MockEfeClient class
│   │   ├── verdict.ts                    ← newVerdict factory
│   │   └── server.ts                     ← in-memory gRPC mock server (Node only)
│   │
│   └── _generated/                       ← protoc-gen-es output (gitignored)
│       ├── efe/v1/types_pb.ts            ← types.proto
│       ├── efe/v1/efe_pb.ts              ← efe.proto
│       ├── efe/v1/catalog_pb.ts          ← catalog.proto
│       ├── efe/v1/admin_pb.ts            ← admin.proto
│       ├── efe/v1/efe_connect.ts         ← Connect-RPC client (Node + Browser)
│       └── PROTOS_HASH.txt               ← drift detection anchor
│
├── tests/                                ← Vitest tests
│   ├── unit/
│   │   ├── verdict.test.ts
│   │   ├── errors.test.ts
│   │   ├── options.test.ts
│   │   ├── retry.test.ts
│   │   └── auth.test.ts
│   ├── integration/
│   │   ├── classify.test.ts               ← vs in-memory mock
│   │   ├── healthcheck.test.ts
│   │   └── retry.test.ts
│   └── e2e/                               ← Playwright
│       ├── browser-classify.spec.ts
│       ├── browser-healthcheck.spec.ts
│       └── fixtures/
│           ├── envoy.yaml                 ← Envoy config gRPC-Web translator
│           └── test-server.ts             ← spawn helper
│
├── examples/                             ← 3 frameworks (ESM only, separate package.json)
│   ├── react/
│   │   ├── package.json                   ← own deps (react 18+, vite)
│   │   ├── vite.config.ts
│   │   ├── tsconfig.json
│   │   ├── index.html
│   │   └── src/
│   │       ├── main.tsx
│   │       ├── App.tsx
│   │       └── useEfeClient.ts
│   ├── vue/
│   │   ├── package.json
│   │   └── src/
│   │       ├── main.ts
│   │       ├── App.vue
│   │       └── useEfeClient.ts
│   └── express/
│       ├── package.json
│       └── src/
│           └── index.ts
│
├── scripts/                              ← bash + node scripts
│   ├── gen-proto.sh                       ← buf generate wrapper
│   ├── publish.sh                         ← npm publish (private/public modes)
│   ├── bundlesize.mjs                     ← CI bundle size check
│   ├── e2e-bootstrap.sh                   ← spawn Envoy + dev server
│   └── lint.sh                            ← biome + tsc + bundlesize
│
└── docs/                                 ← typedoc HTML output (gitignored)
```

### §6.2 Layout invariants

* **`src/core/` ne peut PAS importer `src/node/` ou `src/web/`** — pin
  test `test_d24_core_no_runtime_imports` (regex `import.*from.*['"]\.\./(node|web)/`).
* **`src/web/` ne peut PAS importer `src/node/` ni vice-versa** — pin
  test `test_d24_no_runtime_cross_imports`.
* **`src/_generated/`** est gitignored mais regeneré reproductiblement via
  `pnpm gen:proto` (qui appelle `scripts/gen-proto.sh` → `buf generate`).
  Le `PROTOS_HASH.txt` lui est **commité** comme drift anchor.
* **`tests/` n'est PAS publié** — `.npmignore` exclut.
* **`examples/` n'est PAS publié** non plus — chaque exemple est un
  package indépendant. Les consumers les copient/forkent.
* **`docs/` est gitignored** — généré par `pnpm typedoc` et publié sur
  GitHub Pages via un workflow séparé (out-of-scope D24, reporté D26).

### §6.3 `package.json` exports map (canonical)

```json
{
  "name": "@efe/sdk",
  "version": "0.3.0",
  "type": "module",
  "packageManager": "pnpm@9.7.0",
  "engines": {
    "node": ">=18"
  },
  "exports": {
    ".": {
      "browser": {
        "types": "./dist/web/index.d.ts",
        "import": "./dist/web/index.js"
      },
      "node": {
        "types": "./dist/node/index.d.ts",
        "import": "./dist/node/index.js",
        "require": "./dist/node/index.cjs"
      },
      "default": {
        "types": "./dist/web/index.d.ts",
        "import": "./dist/web/index.js"
      }
    },
    "./node": {
      "types": "./dist/node/index.d.ts",
      "import": "./dist/node/index.js",
      "require": "./dist/node/index.cjs"
    },
    "./web": {
      "types": "./dist/web/index.d.ts",
      "import": "./dist/web/index.js"
    },
    "./mock": {
      "types": "./dist/mock/index.d.ts",
      "import": "./dist/mock/index.js",
      "require": "./dist/mock/index.cjs"
    },
    "./package.json": "./package.json"
  },
  "files": [
    "dist/",
    "README.md",
    "LICENSE",
    "CHANGELOG.md"
  ]
}
```

**Lecture** :
* `import { EfeClient } from '@efe/sdk'` :
  * **Node** → `dist/node/index.js` → `EfeClient` (full surface).
  * **Browser** → `dist/web/index.js` → `EfeWebClient` (re-exporté
    aliasé en `EfeClient`).
* `import { EfeClient } from '@efe/sdk/node'` → force la version Node
  (utile dans un BFF qui veut être explicite).
* `import { EfeClient } from '@efe/sdk/web'` → force la version
  browser (utile pour les bundlers qui ne font pas de auto-detect).
* `import { MockEfeClient } from '@efe/sdk/mock'` → fixtures de test,
  shared core (n'importe pas Node ni Web).

### §6.4 `tsconfig.json` (strict mode)

```jsonc
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "outDir": "./dist",
    "rootDir": "./src",
    "declaration": true,
    "declarationMap": true,
    "sourceMap": true,

    "strict": true,
    "noImplicitAny": true,
    "strictNullChecks": true,
    "strictFunctionTypes": true,
    "strictBindCallApply": true,
    "strictPropertyInitialization": true,
    "noImplicitThis": true,
    "alwaysStrict": true,

    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedIndexedAccess": true,
    "exactOptionalPropertyTypes": true,

    "esModuleInterop": true,
    "forceConsistentCasingInFileNames": true,
    "skipLibCheck": true
  },
  "include": ["src/**/*"],
  "exclude": ["node_modules", "dist", "tests", "examples"]
}
```

**Justifications** :
* `target: ES2022` — top-level await + private fields + Error.cause.
  Node 18+ + browsers récents OK.
* `module: NodeNext` — résolution Node-style avec `.js` extension
  obligatoire dans les imports (canonical ESM).
* `noUncheckedIndexedAccess` — `arr[0]` donne `T | undefined` au lieu
  de `T`. Force le type-narrowing → pas de `undefined.method()` runtime
  errors.
* `exactOptionalPropertyTypes` — `{ foo?: string }` n'accepte PAS
  `{ foo: undefined }`. Distingue absent vs explicite undefined.
* `noUnusedLocals` + `noUnusedParameters` — fail-fast sur le dead
  code.

### §6.5 Decision : workspaces structure

**Top-level repo layout** :

```
efe/                                        ← repo root
├── clients/
│   └── typescript/
│       └── @efe/
│           └── sdk/                        ← M10 SDK package (this doc)
├── pnpm-workspace.yaml                     ← workspaces declaration
└── package.json                            ← workspace root manifest
```

**`pnpm-workspace.yaml`** (à créer en J1) :

```yaml
packages:
  - 'clients/typescript/@efe/sdk'
  - 'clients/typescript/@efe/sdk/examples/*'
```

**Pourquoi un workspace** : permet aux examples de référencer le SDK
via `"@efe/sdk": "workspace:*"` (auto-resolve to local) sans publier.
Identique à la stratégie Go `replace github.com/efe-ai/efe-sdk-go => ../..`.

---

## §7. Vue d'ensemble — sprint D24 unique

### §7.1 Récapitulatif du livrable

| Axe | Spec | Volumétrie |
|---|---|---|
| Package npm publishable | `@efe/sdk@0.3.0` private (Φ-B) | 1 package |
| Source TS | `src/core/` + `src/node/` + `src/web/` + `src/mock/` | ~3 200 LOC |
| Generated types | `src/_generated/` (buf-generated) | ~1 500 LOC (auto) |
| Tests | Vitest unit + integration + Playwright e2e | ~1 200 LOC |
| Examples | React + Vue + Express (3 sub-packages) | ~600 LOC |
| Scripts shell | gen-proto, publish, bundlesize, e2e-bootstrap, lint | ~250 LOC |
| CI workflows | sdk-typescript-test + sdk-typescript-smoke + sdk-typescript-publish | ~400 LOC YAML |
| Pin tests Python | ~32 tests anti-régression | ~700 LOC |
| Documentation | README + doc.md inline TSDoc | ~250 LOC |
| **Total** | **~8 100 LOC** (~3 200 hand-coded TS + 1 500 generated + 1 200 tests + 600 examples + 1 600 build/CI/docs) | |

### §7.2 Phasage J1 → J10

| Jour | Goal | Output |
|---|---|---|
| **J1** | Bootstrap monorepo + workspaces + tsconfig + buf init | `package.json` + `pnpm-workspace.yaml` + `buf.yaml` + `buf.gen.yaml` ; `pnpm install` OK ; `pnpm gen:proto` produit `_generated/` |
| **J2** | Core types : Verdict + 10 sentinels + ClientOptions + version | `src/core/*.ts` ; `pnpm typecheck` OK ; 5 unit tests Vitest |
| **J3** | Node client : EfeClient class + transport @grpc/grpc-js | `src/node/client.ts` + `src/node/transport.ts` ; in-memory mock → Vitest integration test "happy path" |
| **J4** | Auth + TLS + retry + interceptors (Node) | `src/node/{auth, tls, interceptors}.ts` + `src/core/retry.ts` ; tests retry + auth header |
| **J5** | OTel + Prometheus (Node) ; mock package | `src/core/otel.ts` + `src/node/metrics.ts` + `src/mock/*.ts` ; pin tests Prom names |
| **J6** | Browser client : EfeWebClient + Connect-Web transport | `src/web/{client, transport, interceptors}.ts` ; first Playwright e2e + Envoy fixture |
| **J7** | 3 framework examples : React + Vue + Express | `examples/{react, vue, express}/` ; `pnpm dev` chaque exemple OK |
| **J8** | Tests Vitest + Playwright complets ; coverage | ≥ 30 unit + ≥ 10 integration + ≥ 5 e2e ; coverage > 80% |
| **J9** | Pin tests Python + scripts/ + CI workflows | `scripts/tests/test_d24_typescript_sdk_extension.py` ; 3 workflows YAML ; `pnpm bundlesize` budget |
| **J10** | Audit pass 1 + CHANGELOG + publish dry-run | `npm pack` produces tarball ; `npm publish --dry-run` OK ; CHANGELOG entry M10 |

### §7.3 Risques sprint-level

| Risque | Probabilité | Impact | Mitigation |
|---|:---:|:---:|---|
| `buf generate` toolchain instable | M | H | Pin `buf` version dans CI ; fallback hand-coded stubs (à la M9 stratégie) |
| Bundle size > 50 KB gz | M | M | `bundlesize` CI step bloquant ; tree-shake audit J8 |
| Playwright flakiness | H | M | Retry 2x ; `test.describe.configure({ retries: 2 })` |
| Envoy gRPC-Web sidecar non disponible staging | L | H | Spin un Envoy local en CI ; document setup dans README |
| Type generation drift entre proto + .ts | M | M | `PROTOS_HASH.txt` pin + CI step `pnpm gen:proto && git diff --exit-code` |
| `@grpc/grpc-js` peerDep version mismatch | L | M | Pin major version `^1.10.0` ; tests matrix Node 18/20/22 |
| Examples build cascade fail (workspaces) | M | L | `pnpm -r build` + chaque example a son propre `tsconfig.build.json` |

---

## §8. Surface publique cible (API TS publishable)

### §8.1 Universal exports (`@efe/sdk`)

```ts
// src/index.ts — re-exports auto-routed by package.json exports

// Core types (universal)
export type { Verdict, StreamItem } from './core/verdict.js';
export type { ClientOptions, AuthConfig, RetryConfig } from './core/options.js';

// Sentinel error classes (universal)
export {
  EfeError,                     // base class
  ErrUnauthenticated,
  ErrPermissionDenied,
  ErrCatalogNotFound,
  ErrCatalogInvalid,
  ErrTimeout,
  ErrUnavailable,
  ErrRateLimited,
  ErrInvalidArgument,
  ErrInternal,
  ErrClosed,
  isRetryable,
  errorCode,
} from './core/errors.js';

// Retry (universal)
export {
  ExponentialBackoff,
  type RetryBackoff,
} from './core/retry.js';

// Version
export { Version } from './core/version.js';

// Conditional re-export — bundler resolves to web or node automatically
export { EfeClient } from './_runtime-export.js';
```

Le fichier `_runtime-export.ts` est un **stub** qui se résout
différemment selon `package.json` `exports` :
* En contexte Node : re-export de `./node/client.js::EfeClient`.
* En contexte Browser : re-export de `./web/client.js::EfeWebClient`
  (renommé `EfeClient` pour la cohérence du nom).

### §8.2 Node-specific exports (`@efe/sdk/node`)

```ts
// src/node.ts
export * from './index.js';                 // tout l'universal
export { EfeClient } from './node/client.js';
export type { ClassifyStreamOptions } from './node/stream.js';

// Node-only auth helpers
export { withMTLS } from './node/tls.js';
```

### §8.3 Browser-specific exports (`@efe/sdk/web`)

```ts
// src/web.ts
export * from './index.js';                 // tout l'universal
export { EfeWebClient as EfeClient } from './web/client.js';

// Browser-only auth helpers
export { withFetchAuth } from './web/auth.js';
```

### §8.4 Mock exports (`@efe/sdk/mock`)

```ts
// src/mock.ts
export { MockEfeClient } from './mock/client.js';
export { newVerdict } from './mock/verdict.js';
export type { VerdictOverrides } from './mock/verdict.js';

// Node-only test helper (in-memory gRPC server)
export { createMockServer } from './mock/server.js';
```

### §8.5 Total public surface

| Export | Type | Stable | Pin tested |
|---|---|---|---|
| `EfeClient` | class | yes — major bump to break | ✓ DT-T-1 |
| `EfeWebClient` | class | yes | ✓ DT-T-1 |
| `Verdict` | interface | yes — 10 fields immutables | ✓ DT-T-2 |
| `StreamItem` | interface | yes | ✓ DT-T-2 |
| `ClientOptions` | interface | yes | ✓ DT-T-3 |
| `AuthConfig` | discriminated union | yes | ✓ DT-T-3 |
| `RetryConfig` | interface | yes | ✓ DT-T-3 |
| `RetryBackoff` | interface | yes | ✓ DT-T-3 |
| `ExponentialBackoff` | class | yes | ✓ DT-T-3 |
| `EfeError` | class | yes — base for all | ✓ DT-T-4 |
| `ErrUnauthenticated` | class | yes | ✓ DT-T-4 |
| `ErrPermissionDenied` | class | yes | ✓ DT-T-4 |
| `ErrCatalogNotFound` | class | yes | ✓ DT-T-4 |
| `ErrCatalogInvalid` | class | yes | ✓ DT-T-4 |
| `ErrTimeout` | class | yes | ✓ DT-T-4 |
| `ErrUnavailable` | class | yes | ✓ DT-T-4 |
| `ErrRateLimited` | class | yes | ✓ DT-T-4 |
| `ErrInvalidArgument` | class | yes | ✓ DT-T-4 |
| `ErrInternal` | class | yes | ✓ DT-T-4 |
| `ErrClosed` | class | yes | ✓ DT-T-4 |
| `isRetryable(err)` | function | yes | ✓ DT-T-4 |
| `errorCode(err)` | function | yes | ✓ DT-T-4 |
| `Version` | const string | yes | ✓ DT-T-5 |
| `MockEfeClient` | class | yes (mock package) | ✓ DT-T-6 |
| `newVerdict(opts?)` | function | yes | ✓ DT-T-6 |
| `createMockServer(opts?)` | function | yes (Node only) | ✓ DT-T-6 |
| `withMTLS(...)` | function | yes (Node only) | ✓ DT-T-7 |
| `withFetchAuth(...)` | function | yes (Browser only) | ✓ DT-T-7 |
| `ClassifyStreamOptions` | interface | yes (Node only) | ✓ DT-T-8 |

**Total** : 28 exports publics ; 8 pin test groups DT-T-1..8.

---

## §9. Sprint D24 — package npm entier

### §9.1 Definition of Done — sprint-level

```text
Sprint D24 = DONE quand :

  ✓ pnpm install OK from clean state
  ✓ pnpm gen:proto regenerates _generated/ idempotently
  ✓ pnpm build produces dist/ avec ESM + CJS + dts
  ✓ pnpm typecheck OK (strict mode, 0 errors)
  ✓ pnpm test (Vitest unit + integration) GREEN, > 80% coverage
  ✓ pnpm test:e2e (Playwright) GREEN sur Chromium + WebKit + Firefox
  ✓ pnpm bundlesize : browser entry < 50 KB gz
  ✓ pnpm lint (biome + tsc) zero warning
  ✓ npm pack produce un tarball valide (taille < 250 KB)
  ✓ npm publish --dry-run --access=restricted OK
  ✓ Examples : React + Vue + Express tous compile + run
  ✓ Pin tests Python (~32) GREEN
  ✓ CHANGELOG.md mis à jour avec entrée [Unreleased] M10 D24
  ✓ Architecture doc (CE FICHIER) committed avant le code
  ✓ CI workflows green sur PR test : sdk-typescript-test.yml
```

### §9.2 Output artefacts cibles

```
clients/typescript/@efe/sdk/
├── (≈ 50 fichiers source créés)
└── dist/                                  ← built artefact (publié)
    ├── node/
    │   ├── index.js (ESM)
    │   ├── index.cjs (CJS)
    │   └── index.d.ts
    ├── web/
    │   ├── index.js (ESM only — browser ne require pas CJS)
    │   └── index.d.ts
    └── mock/
        ├── index.js
        ├── index.cjs
        └── index.d.ts
```

### §9.3 npm tarball preview

```bash
$ npm pack --dry-run
@efe/sdk@0.3.0
=== Tarball Contents ===
12.3kB  dist/node/index.js
8.9kB   dist/node/index.cjs
2.1kB   dist/node/index.d.ts
9.7kB   dist/web/index.js
1.8kB   dist/web/index.d.ts
4.2kB   dist/mock/index.js
3.8kB   dist/mock/index.cjs
1.2kB   dist/mock/index.d.ts
2.3kB   README.md
11.4kB  LICENSE
1.5kB   CHANGELOG.md
1.2kB   package.json
=== Tarball Details ===
total files : 12
unpacked size : 60.4 kB
shasum : ...
```

---

## §10. Sub-système 1 — `EfeClient` (Node) + `EfeWebClient` (browser)

### §10.1 EfeClient (Node) — full surface

```ts
// src/node/client.ts

import {
  ConnectError,
  createPromiseClient,
  type Interceptor,
} from '@connectrpc/connect';
import { createGrpcTransport } from '@connectrpc/connect-node';

import type { ClientOptions } from '../core/options.js';
import type { Verdict } from '../core/verdict.js';
import { ErrClosed, ErrInvalidArgument, fromConnectError } from '../core/errors.js';
import { Version } from '../core/version.js';

import { EfeService } from '../_generated/efe/v1/efe_connect.js';
import { authInterceptor } from './auth.js';
import { otelInterceptor } from '../core/otel.js';
import { retryInterceptor } from './interceptors.js';
import { promInterceptor } from './metrics.js';
import { resolveTLSCredentials } from './tls.js';

/**
 * EfeClient is the Node entry point for the EFE SDK.
 *
 * Construct via:
 *   const client = new EfeClient({ endpoint: 'staging.efe.ai:50051', ... });
 *
 * Methods:
 *   await client.classify(text, opts?) : Promise<Verdict>
 *   await client.healthcheck(opts?) : Promise<void>
 *   await client.classifyStream(texts, opts?) : AsyncIterable<StreamItem>
 *   await client.close() : Promise<void>
 *
 * Lifecycle:
 *   - constructor wires the gRPC transport (LAZY DIAL — no I/O yet).
 *   - First RPC triggers TCP+TLS handshake.
 *   - close() releases the underlying connection.
 *
 * Concurrency: instances are SAFE for concurrent use across goroutines /
 * workers. Internally uses a single `@grpc/grpc-js` channel with the
 * canonical interceptor chain.
 */
export class EfeClient {
  readonly #service: ReturnType<typeof createPromiseClient<typeof EfeService>>;
  readonly #opts: Required<ClientOptions>;
  #closed = false;

  constructor(opts: ClientOptions) {
    this.#opts = validateOptions(opts);

    const interceptors: Interceptor[] = [
      authInterceptor(this.#opts.auth),
      otelInterceptor(this.#opts.otel),
      promInterceptor(this.#opts.prometheus),
      retryInterceptor(this.#opts.retry),
    ];

    const transport = createGrpcTransport({
      baseUrl: normalizeEndpoint(this.#opts.endpoint),
      httpVersion: '2',
      nodeOptions: resolveTLSCredentials(this.#opts.auth),
      interceptors,
      // Default 16 MiB recv ; override via opts.maxReceivedMessageSize.
      readMaxBytes: this.#opts.maxReceivedMessageSize ?? 16 * 1024 * 1024,
    });

    this.#service = createPromiseClient(EfeService, transport);
  }

  /**
   * Send one classification request.
   *
   * @param text - input text (≥ 1 char after trim)
   * @param opts.signal - optional AbortSignal for cancellation
   * @param opts.timeout - per-call deadline (ms) ; falls back to client default
   * @throws ErrInvalidArgument | ErrUnauthenticated | ErrTimeout | ErrUnavailable
   *         | ErrRateLimited | ErrCatalogInvalid | ErrInternal | ErrClosed
   */
  async classify(
    text: string,
    opts: { signal?: AbortSignal; timeout?: number } = {},
  ): Promise<Verdict> {
    if (this.#closed) {
      throw new ErrClosed('cannot issue RPC after close()');
    }
    if (text.trim().length === 0) {
      throw new ErrInvalidArgument('empty text');
    }

    const start = performance.now();
    try {
      const resp = await this.#service.execute(
        { rawPrompt: text },
        {
          signal: opts.signal,
          timeoutMs: opts.timeout ?? this.#opts.deadline,
        },
      );
      return verdictFromExecuteResponse(resp, performance.now() - start);
    } catch (err) {
      if (err instanceof ConnectError) {
        throw fromConnectError(err);
      }
      throw err;
    }
  }

  /**
   * Probe the server's gRPC Health endpoint.
   *
   * @throws ErrUnavailable on NOT_SERVING / unreachable
   * @throws ErrTimeout on deadline exceeded
   */
  async healthcheck(opts: { signal?: AbortSignal; timeout?: number } = {}): Promise<void> {
    if (this.#closed) {
      throw new ErrClosed('cannot issue RPC after close()');
    }
    // ... implementation calls grpc.health.v1.Health/Check
  }

  /**
   * Concurrent batch classification with index preservation.
   *
   * Uses a worker pool (default 8) consuming texts from an AsyncIterable.
   * Yields StreamItems with the input position in `index`.
   *
   * @example
   *   for await (const item of client.classifyStream(['a', 'b', 'c'])) {
   *     if (item.error) console.error(item.index, item.error);
   *     else console.log(item.index, item.verdict.label);
   *   }
   */
  async *classifyStream(
    texts: AsyncIterable<string> | Iterable<string>,
    opts: ClassifyStreamOptions = {},
  ): AsyncIterable<StreamItem> {
    // Implementation in src/node/stream.ts (worker pool + dispatcher).
  }

  /**
   * Release the underlying gRPC connection. Idempotent.
   *
   * Subsequent classify/healthcheck calls throw ErrClosed.
   */
  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    // The underlying transport from connect-node manages the channel ;
    // we call its dispose if available.
    if (typeof (this.#service as any).dispose === 'function') {
      await (this.#service as any).dispose();
    }
  }

  /** Returns true after close() has been called. Public for testing. */
  get closed(): boolean {
    return this.#closed;
  }

  /** Redacted summary, safe for logs. */
  toString(): string {
    return `EfeClient{endpoint=${this.#opts.endpoint} closed=${this.#closed}}`;
  }
}
```

**Notes** :
* Le `#field` syntax = vrais private fields (ES2022). Inaccessibles
  même via reflection.
* `createPromiseClient` (de `@connectrpc/connect`) génère le client
  RPC à partir du service descriptor. Auto-typé, pas de cast.
* L'interceptor chain ordre = même que Go SDK : auth → otel → prom →
  retry.
* `performance.now()` vs `Date.now()` : `performance.now()` est
  monotone (immune aux NTP jumps), correct pour mesurer la latence.

### §10.2 EfeWebClient (Browser) — subset

```ts
// src/web/client.ts

import {
  ConnectError,
  createPromiseClient,
  type Interceptor,
} from '@connectrpc/connect';
import { createGrpcWebTransport } from '@connectrpc/connect-web';

import type { ClientOptions } from '../core/options.js';
import type { Verdict } from '../core/verdict.js';
import { ErrClosed, ErrInvalidArgument, fromConnectError } from '../core/errors.js';

import { EfeService } from '../_generated/efe/v1/efe_connect.js';
import { authInterceptor } from './auth.js';
import { otelInterceptor } from '../core/otel.js';

/**
 * EfeWebClient — browser entry point.
 *
 * SUBSET API : exposes only IDEMPOTENT RPCs (Classify + Healthcheck).
 * Write RPCs (RegisterCatalog, EvolveCatalog, DeleteCatalog) are
 * intentionally NOT available — they require admin scope and should
 * be performed server-side via a BFF.
 *
 * Transport : gRPC-Web over HTTP/1.1 via @connectrpc/connect-web.
 * Requires the EFE server to expose port :50052 (gRPC-Web translator)
 * with appropriate CORS headers (`access-control-allow-origin`,
 * `access-control-allow-headers: authorization,content-type,...`).
 *
 * NO RETRY by default : the browser is on an unreliable network ;
 * we surface errors to the caller, who decides retry policy
 * (e.g. via TanStack Query / SWR).
 *
 * NO mTLS : the browser uses the OS trust store + the user's TLS.
 */
export class EfeWebClient {
  readonly #service: ReturnType<typeof createPromiseClient<typeof EfeService>>;
  readonly #opts: Required<ClientOptions>;
  #closed = false;

  constructor(opts: ClientOptions) {
    this.#opts = validateOptions(opts);

    const interceptors: Interceptor[] = [
      authInterceptor(this.#opts.auth),
      otelInterceptor(this.#opts.otel),
      // No retry / no prom in browser by default.
    ];

    const transport = createGrpcWebTransport({
      baseUrl: normalizeEndpoint(this.#opts.endpoint),
      // Browser uses fetch ; consumers may inject a custom fetch
      // (for testing or MSW-mocked).
      fetch: this.#opts.fetch ?? globalThis.fetch.bind(globalThis),
      interceptors,
    });

    this.#service = createPromiseClient(EfeService, transport);
  }

  async classify(
    text: string,
    opts: { signal?: AbortSignal; timeout?: number } = {},
  ): Promise<Verdict> {
    // Same as Node ; just no retry around it.
    if (this.#closed) throw new ErrClosed('cannot issue RPC after close()');
    if (text.trim().length === 0) throw new ErrInvalidArgument('empty text');

    const start = performance.now();
    try {
      const resp = await this.#service.execute(
        { rawPrompt: text },
        {
          signal: opts.signal,
          timeoutMs: opts.timeout ?? this.#opts.deadline,
        },
      );
      return verdictFromExecuteResponse(resp, performance.now() - start);
    } catch (err) {
      if (err instanceof ConnectError) {
        throw fromConnectError(err);
      }
      throw err;
    }
  }

  async healthcheck(opts: { signal?: AbortSignal; timeout?: number } = {}): Promise<void> {
    // ...
  }

  async close(): Promise<void> {
    this.#closed = true;
    // Browser fetch transport has no persistent connection to close ;
    // the AbortController only cancels in-flight requests.
  }

  get closed(): boolean { return this.#closed; }

  toString(): string {
    return `EfeWebClient{endpoint=${this.#opts.endpoint} closed=${this.#closed}}`;
  }
}
```

**Différences clés Node vs Browser** :

| Aspect | EfeClient (Node) | EfeWebClient (Browser) |
|---|---|---|
| Transport | `@grpc/grpc-js` HTTP/2 | `@connectrpc/connect-web` gRPC-Web HTTP/1.1 |
| Port serveur | :50051 (gRPC) | :50052 (gRPC-Web translator, e.g. Envoy filter) |
| RPCs disponibles | All (Classify, Healthcheck, RegisterCatalog, EvolveCatalog, ...) | Idempotent only (Classify, Healthcheck) |
| Auth JWT | OK | OK (via `Authorization` header) |
| mTLS | OK | NO (browser doesn't expose client cert API) |
| Retry intégré | OK (configurable) | NO (caller's responsibility) |
| Prometheus métriques | OK (5 collecteurs) | NO (browser metrics opt-in via OTel only) |
| Streaming RPCs | Server-streaming + Client-streaming + Bidi | Server-streaming only (gRPC-Web limit) ; D24 uses unary only |
| Bundle impact | ~250 KB gz | ~50 KB gz |
| TLS material loading | `fs.promises.readFile()` | NO (browser CORS+TLS automatic) |

### §10.3 Construction-time validation

```ts
// src/core/options.ts

export function validateOptions(opts: ClientOptions): Required<ClientOptions> {
  if (!opts.endpoint || typeof opts.endpoint !== 'string') {
    throw new ErrInvalidArgument('endpoint required (string)');
  }
  if (opts.deadline !== undefined && opts.deadline < 0) {
    throw new ErrInvalidArgument('deadline must be ≥ 0');
  }
  if (opts.retry?.maxAttempts !== undefined && opts.retry.maxAttempts < 1) {
    throw new ErrInvalidArgument('retry.maxAttempts must be ≥ 1');
  }
  // ... etc.
  return {
    endpoint: opts.endpoint,
    deadline: opts.deadline ?? 10_000,
    auth: opts.auth ?? { kind: 'none' },
    retry: opts.retry ?? defaultRetry(),
    otel: opts.otel ?? { kind: 'noop' },
    prometheus: opts.prometheus ?? { kind: 'noop' },
    userAgent: opts.userAgent ?? `@efe/sdk/${Version}`,
    maxReceivedMessageSize: opts.maxReceivedMessageSize ?? 16 * 1024 * 1024,
    fetch: opts.fetch,
  };
}
```

### §10.4 ClientOptions interface

```ts
// src/core/options.ts

export interface ClientOptions {
  /** gRPC endpoint, e.g. 'staging.efe.ai:50051' (Node) or 'https://api.efe.ai' (Browser). */
  readonly endpoint: string;

  /** Per-call deadline in ms. Default 10000. 0 = no SDK-side deadline. */
  readonly deadline?: number;

  /** Auth strategy. Default = none (DEV ONLY). */
  readonly auth?: AuthConfig;

  /** Retry policy. Default = 3 attempts exponential backoff. (Node only — ignored in browser.) */
  readonly retry?: RetryConfig;

  /** OTel tracer provider. Default = no-op. */
  readonly otel?: OTelConfig;

  /** Prometheus registry. (Node only.) */
  readonly prometheus?: PrometheusConfig;

  /** User-Agent header. Default = `@efe/sdk/<version>`. */
  readonly userAgent?: string;

  /** Max gRPC message size (bytes). Default 16 MiB. */
  readonly maxReceivedMessageSize?: number;

  /** Custom fetch implementation. (Browser only — useful for MSW / nock-equivalent.) */
  readonly fetch?: typeof globalThis.fetch;
}

/** Auth config — discriminated union. */
export type AuthConfig =
  | { readonly kind: 'jwt'; readonly token: string }
  | { readonly kind: 'mtls'; readonly certPath: string; readonly keyPath: string; readonly caPath: string }
  | { readonly kind: 'none' };

export interface RetryConfig {
  readonly maxAttempts: number;     // ≥ 1, default 3
  readonly backoff: RetryBackoff;
}

export interface RetryBackoff {
  /** Returns delay in ms before the (attempt+1)-th try. */
  next(attempt: number): number;
}
```

---

## §11. Sub-système 2 — Type generation pipeline

### §11.1 Tooling

* **`buf` CLI** : `brew install bufbuild/buf/buf` (≥ 1.30).
* **`@bufbuild/protoc-gen-es`** : protoc plugin that generates ESM
  TS message classes (no cls inheritance, plain objects + parser).
* **`@connectrpc/protoc-gen-connect-es`** : generates the RPC client
  + service descriptors compatible with `@grpc/grpc-js` (Node) AND
  `connect-web` (Browser).

### §11.2 `buf.yaml`

```yaml
# clients/typescript/@efe/sdk/buf.yaml
version: v2
modules:
  - path: ../../../../protos
deps: []
breaking:
  use:
    - FILE
lint:
  use:
    - DEFAULT
```

Le `path: ../../../../protos` pointe vers le `protos/` racine du
repo (4 levels up depuis `clients/typescript/@efe/sdk/`).

### §11.3 `buf.gen.yaml`

```yaml
# clients/typescript/@efe/sdk/buf.gen.yaml
version: v2
managed:
  enabled: true
plugins:
  - local: protoc-gen-es
    out: src/_generated
    opt:
      - target=ts
      - import_extension=.js
  - local: protoc-gen-connect-es
    out: src/_generated
    opt:
      - target=ts
      - import_extension=.js
```

`import_extension=.js` est nécessaire pour ESM (les imports doivent
référencer `.js` même quand le fichier source est `.ts`).

### §11.4 `scripts/gen-proto.sh`

```bash
#!/usr/bin/env bash
# Sprint D24 — regenerate TS proto stubs via buf.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

# Pre-flight.
for bin in buf node pnpm; do
  command -v "$bin" >/dev/null || { echo "FATAL: $bin missing"; exit 2; }
done

# Ensure plugins are installed (workspace dep).
pnpm install --frozen-lockfile >/dev/null

# Clean.
rm -rf src/_generated/efe

# Generate.
buf generate

# Update PROTOS_HASH.txt for drift detection.
{
  echo "# Generated by scripts/gen-proto.sh — DO NOT EDIT."
  for f in ../../../../protos/efe/v1/*.proto; do
    h=$(shasum -a 256 "$f" | awk '{print $1}')
    echo "$h  $(basename "$f")"
  done
} > src/_generated/PROTOS_HASH.txt

echo "✅ buf generate complete."
```

### §11.5 Drift detection

Pin test Python `test_d24_typescript_sdk_protos_in_sync` :

```python
def test_d24_typescript_sdk_protos_in_sync():
    """PROTOS_HASH.txt matches actual SHA-256 of protos/."""
    hash_file = SDK_ROOT / 'src/_generated/PROTOS_HASH.txt'
    assert hash_file.is_file(), "regenerate via `pnpm gen:proto`"
    expected = parse_hashes(hash_file.read_text())
    for proto_name in ('types.proto', 'efe.proto', 'catalog.proto', 'admin.proto'):
        actual = sha256(REPO_ROOT / 'protos/efe/v1' / proto_name)
        assert expected[proto_name] == actual, \
            f"{proto_name} drift — re-run `pnpm gen:proto`"
```

### §11.6 Generated output structure

`buf generate` produces approximately :

```
src/_generated/
├── efe/v1/
│   ├── types_pb.ts          ← message classes (Verdict enum, SwarmSpec, ConeRef, ...)
│   ├── efe_pb.ts            ← ExecuteRequest, ExecuteResponse messages
│   ├── catalog_pb.ts        ← Catalog* messages
│   ├── admin_pb.ts          ← Admin* messages
│   └── efe_connect.ts       ← EfeService descriptor + Connect-RPC client class
└── PROTOS_HASH.txt
```

### §11.7 Decision : pourquoi `bufbuild/protobuf-es` plutôt que `ts-proto`

| Critère | `@bufbuild/protobuf-es` | `ts-proto` |
|---|---|---|
| Bundle size | ~5 KB runtime | ~15 KB runtime |
| Tree-shake | excellent (ESM-first) | OK |
| Maintained by | buf.build (full-time team) | community |
| Browser support | first-class | OK |
| Connect-Protocol | native | manual setup |
| TypeScript strict | excellent | OK |
| Field options preservation | yes | partial |
| **Decision** | **CHOSEN** | rejected |

---

## §12. Sub-système 3 — Errors + retry + idempotence policy

### §12.1 10 sentinel errors (cross-language pinned)

```ts
// src/core/errors.ts

/**
 * Base class for all SDK errors.
 *
 * `instanceof EfeError` is true for all SDK-generated errors.
 * Use `errors.is(err, ErrXxx)` (`isXxx(err)` helper) to discriminate.
 */
export class EfeError extends Error {
  readonly code: string;
  override readonly name: string;

  constructor(code: string, message: string, options?: { cause?: unknown }) {
    super(message, options);
    this.code = code;
    this.name = `Efe${code}`;
  }
}

/** gRPC code 16 (UNAUTHENTICATED). */
export class ErrUnauthenticated extends EfeError {
  constructor(message = 'unauthenticated', options?: { cause?: unknown }) {
    super('Unauthenticated', `efe: ${message}`, options);
  }
}

/** gRPC code 7 (PERMISSION_DENIED). */
export class ErrPermissionDenied extends EfeError {
  constructor(message = 'permission denied', options?: { cause?: unknown }) {
    super('PermissionDenied', `efe: ${message}`, options);
  }
}

/** gRPC code 5 (NOT_FOUND). */
export class ErrCatalogNotFound extends EfeError {
  constructor(message = 'catalog not found', options?: { cause?: unknown }) {
    super('CatalogNotFound', `efe: ${message}`, options);
  }
}

/** gRPC code 9 (FAILED_PRECONDITION). */
export class ErrCatalogInvalid extends EfeError {
  constructor(message = 'catalog invalid', options?: { cause?: unknown }) {
    super('CatalogInvalid', `efe: ${message}`, options);
  }
}

/** gRPC code 4 (DEADLINE_EXCEEDED). */
export class ErrTimeout extends EfeError {
  constructor(message = 'deadline exceeded', options?: { cause?: unknown }) {
    super('Timeout', `efe: ${message}`, options);
  }
}

/** gRPC code 14 (UNAVAILABLE). */
export class ErrUnavailable extends EfeError {
  constructor(message = 'server unavailable', options?: { cause?: unknown }) {
    super('Unavailable', `efe: ${message}`, options);
  }
}

/** gRPC code 8 (RESOURCE_EXHAUSTED). */
export class ErrRateLimited extends EfeError {
  constructor(message = 'rate limited', options?: { cause?: unknown }) {
    super('RateLimited', `efe: ${message}`, options);
  }
}

/** gRPC code 3 (INVALID_ARGUMENT). */
export class ErrInvalidArgument extends EfeError {
  constructor(message = 'invalid argument', options?: { cause?: unknown }) {
    super('InvalidArgument', `efe: ${message}`, options);
  }
}

/** gRPC code 13 (INTERNAL) or 2 (UNKNOWN). */
export class ErrInternal extends EfeError {
  constructor(message = 'server internal error', options?: { cause?: unknown }) {
    super('Internal', `efe: ${message}`, options);
  }
}

/** Local SDK error : the client was already closed. */
export class ErrClosed extends EfeError {
  constructor(message = 'client is closed', options?: { cause?: unknown }) {
    super('Closed', `efe: ${message}`, options);
  }
}
```

### §12.2 RPC error → sentinel mapping

```ts
// src/core/from-rpc.ts

import { Code as ConnectCode, ConnectError } from '@connectrpc/connect';

export function fromConnectError(err: ConnectError): EfeError {
  switch (err.code) {
    case ConnectCode.Unauthenticated:    return new ErrUnauthenticated(err.message, { cause: err });
    case ConnectCode.PermissionDenied:   return new ErrPermissionDenied(err.message, { cause: err });
    case ConnectCode.NotFound:           return new ErrCatalogNotFound(err.message, { cause: err });
    case ConnectCode.FailedPrecondition: return new ErrCatalogInvalid(err.message, { cause: err });
    case ConnectCode.DeadlineExceeded:
    case ConnectCode.Canceled:           return new ErrTimeout(err.message, { cause: err });
    case ConnectCode.Unavailable:        return new ErrUnavailable(err.message, { cause: err });
    case ConnectCode.ResourceExhausted:  return new ErrRateLimited(err.message, { cause: err });
    case ConnectCode.InvalidArgument:
    case ConnectCode.OutOfRange:         return new ErrInvalidArgument(err.message, { cause: err });
    case ConnectCode.Internal:
    case ConnectCode.Unknown:
    case ConnectCode.DataLoss:
    default:                              return new ErrInternal(err.message, { cause: err });
  }
}
```

### §12.3 isRetryable + errorCode helpers

```ts
// src/core/errors.ts (suite)

/** Returns true iff err is in the retry-eligible subset. */
export function isRetryable(err: unknown): boolean {
  if (!(err instanceof EfeError)) return false;
  return err instanceof ErrUnavailable
      || err instanceof ErrTimeout
      || err instanceof ErrRateLimited;
}

/** Returns the canonical short code (used for metrics labels). */
export function errorCode(err: unknown): string {
  if (err === undefined || err === null) return '';
  if (!(err instanceof EfeError)) return 'unknown';
  switch (err.code) {
    case 'Unauthenticated':  return 'unauthenticated';
    case 'PermissionDenied': return 'permission_denied';
    case 'CatalogNotFound':  return 'catalog_not_found';
    case 'CatalogInvalid':   return 'catalog_invalid';
    case 'Timeout':          return 'timeout';
    case 'Unavailable':      return 'unavailable';
    case 'RateLimited':      return 'rate_limited';
    case 'InvalidArgument':  return 'invalid_argument';
    case 'Internal':         return 'internal';
    case 'Closed':           return 'closed';
    default:                 return 'unknown';
  }
}
```

### §12.4 Idempotence policy (Browser subset)

```ts
// src/web/client.ts (extract)

const IDEMPOTENT_RPCS = new Set([
  '/efe.v1.SwarmService/Execute',           // = Classify
  '/grpc.health.v1.Health/Check',           // = Healthcheck
  '/efe.v1.CatalogService/GetCatalog',      // (read-only, future)
  '/efe.v1.CatalogService/ListCatalogs',    // (read-only, future)
]);

const NON_IDEMPOTENT_RPCS = new Set([
  '/efe.v1.CatalogService/RegisterCatalog',
  '/efe.v1.CatalogService/EvolveCatalog',
  '/efe.v1.CatalogService/DeleteCatalog',
  '/efe.v1.AdminService/RotateKeys',
  '/efe.v1.AdminService/UpdateConfig',
  // ... all admin / write RPCs
]);

// In EfeWebClient, only wire the IDEMPOTENT_RPCS into the public API.
// Compile-time guard via TS : EfeWebClient class doesn't expose
// `registerCatalog`, `evolveCatalog`, etc. — they're not even in the
// `.d.ts`.
```

### §12.5 Retry policy (Node)

```ts
// src/core/retry.ts

export class ExponentialBackoff implements RetryBackoff {
  readonly initialIntervalMs: number;
  readonly maxIntervalMs: number;
  readonly multiplier: number;

  constructor(opts: {
    initialIntervalMs?: number;
    maxIntervalMs?: number;
    multiplier?: number;
  } = {}) {
    this.initialIntervalMs = opts.initialIntervalMs ?? 100;
    this.maxIntervalMs = opts.maxIntervalMs ?? 30_000;
    this.multiplier = opts.multiplier ?? 2.0;
  }

  /** Full jitter (AWS canonical). */
  next(attempt: number): number {
    const base = Math.min(
      this.initialIntervalMs * Math.pow(this.multiplier, attempt - 1),
      this.maxIntervalMs,
    );
    return Math.random() * base;
  }
}

export function defaultRetry(): RetryConfig {
  return { maxAttempts: 3, backoff: new ExponentialBackoff() };
}
```

### §12.6 Retry interceptor (Node)

```ts
// src/node/interceptors.ts

import type { Interceptor } from '@connectrpc/connect';
import type { RetryConfig } from '../core/options.js';
import { fromConnectError, isRetryable } from '../core/errors.js';

export function retryInterceptor(cfg: RetryConfig): Interceptor {
  return (next) => async (req) => {
    let lastErr: unknown;
    for (let attempt = 1; attempt <= cfg.maxAttempts; attempt++) {
      try {
        return await next(req);
      } catch (err) {
        lastErr = err;
        const efeErr = err instanceof Error ? fromConnectError(err as any) : err;
        if (!isRetryable(efeErr)) throw efeErr;
        if (attempt === cfg.maxAttempts) throw efeErr;
        const delay = cfg.backoff.next(attempt);
        await sleep(delay);
      }
    }
    throw lastErr;
  };
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}
```

---

## §13. Sub-système 4 — OpenTelemetry auto-instrumentation

### §13.1 Soft dep — peer

`package.json` :

```json
{
  "peerDependencies": {
    "@opentelemetry/api": "^1.9.0",
    "@opentelemetry/sdk-trace-base": "^1.25.0"
  },
  "peerDependenciesMeta": {
    "@opentelemetry/api": { "optional": true },
    "@opentelemetry/sdk-trace-base": { "optional": true }
  }
}
```

Si l'app n'installe pas OTel, le SDK utilise un no-op tracer (zero
overhead).

### §13.2 OTel interceptor

```ts
// src/core/otel.ts

import type { Interceptor } from '@connectrpc/connect';

let TraceAPI: typeof import('@opentelemetry/api') | null = null;
try {
  // Dynamic import — fails silently if @opentelemetry/api absent.
  TraceAPI = await import('@opentelemetry/api');
} catch {
  TraceAPI = null;
}

export type OTelConfig =
  | { kind: 'noop' }
  | { kind: 'provider'; tracerProvider: import('@opentelemetry/api').TracerProvider };

export function otelInterceptor(cfg: OTelConfig): Interceptor {
  if (cfg.kind === 'noop' || !TraceAPI) {
    return (next) => async (req) => next(req);
  }
  const tracer = cfg.tracerProvider.getTracer('@efe/sdk', Version);
  return (next) => async (req) => {
    return await tracer.startActiveSpan(
      `efe/${req.method.name}`,
      {
        kind: TraceAPI.SpanKind.CLIENT,
        attributes: {
          'rpc.system': 'grpc',
          'rpc.service': 'efe.v1.SwarmService',
          'rpc.method': req.method.name,
        },
      },
      async (span) => {
        try {
          const resp = await next(req);
          span.setStatus({ code: TraceAPI.SpanStatusCode.OK });
          return resp;
        } catch (err) {
          span.recordException(err as Error);
          span.setStatus({
            code: TraceAPI.SpanStatusCode.ERROR,
            message: String(err),
          });
          throw err;
        } finally {
          span.end();
        }
      },
    );
  };
}
```

### §13.3 Browser OTel — same file, fonctionne identiquement

OTel JS API est universal — Node + Browser. Même code dans
`src/core/otel.ts`. Tracer provider injecté par l'app.

---

## §14. Sub-système 5 — Prometheus metrics (Node) + browser metrics opt-in

### §14.1 Node — 5 collectors canoniques

Mêmes noms que Go SDK (M9) :

```ts
// src/node/metrics.ts

import { Counter, Gauge, Histogram, Registry } from 'prom-client';

export const METRIC_NAMES = {
  classifyTotal:        'efe_sdk_classify_total',
  classifyLatency:      'efe_sdk_classify_latency_seconds',
  classifyErrorsTotal:  'efe_sdk_classify_errors_total',
  inFlight:             'efe_sdk_in_flight',
  retryAttemptsTotal:   'efe_sdk_retry_attempts_total',
} as const;

export class SdkMetrics {
  readonly classifyTotal: Counter<'outcome'>;
  readonly classifyLatency: Histogram<'outcome'>;
  readonly classifyErrors: Counter<'sentinel'>;
  readonly inFlight: Gauge;
  readonly retryAttempts: Counter<'attempt'>;

  constructor(registry: Registry) {
    this.classifyTotal = new Counter({
      name: METRIC_NAMES.classifyTotal,
      help: 'Total efe_sdk Classify calls, partitioned by outcome.',
      labelNames: ['outcome'],
      registers: [registry],
    });
    this.classifyLatency = new Histogram({
      name: METRIC_NAMES.classifyLatency,
      help: 'Classify wall-clock latency in seconds, partitioned by outcome.',
      labelNames: ['outcome'],
      buckets: [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10],
      registers: [registry],
    });
    this.classifyErrors = new Counter({
      name: METRIC_NAMES.classifyErrorsTotal,
      help: 'Total efe_sdk Classify errors, partitioned by sentinel.',
      labelNames: ['sentinel'],
      registers: [registry],
    });
    this.inFlight = new Gauge({
      name: METRIC_NAMES.inFlight,
      help: 'Currently in-flight Classify calls.',
      registers: [registry],
    });
    this.retryAttempts = new Counter({
      name: METRIC_NAMES.retryAttemptsTotal,
      help: 'Total retry attempts, partitioned by attempt number.',
      labelNames: ['attempt'],
      registers: [registry],
    });
  }
}
```

### §14.2 Pin test names

```python
# scripts/tests/test_d24_typescript_sdk_extension.py

CANONICAL_METRIC_NAMES_TS = (
    'efe_sdk_classify_total',
    'efe_sdk_classify_latency_seconds',
    'efe_sdk_classify_errors_total',
    'efe_sdk_in_flight',
    'efe_sdk_retry_attempts_total',
)

def test_d24_metric_names_match_go_sdk():
    """TS SDK metric names MUST match Go SDK names exactly."""
    src = (SDK_ROOT / 'src/node/metrics.ts').read_text()
    for name in CANONICAL_METRIC_NAMES_TS:
        assert name in src
```

### §14.3 Browser — pas de Prometheus

Browser n'a pas de scrape endpoint stable. Les apps qui veulent des
métriques côté browser utilisent OTel + un OTLP exporter (HTTP) qui
push à un backend collecteur. Le SDK n'embarque pas `prom-client`
dans le bundle browser.

---

## §15. Sub-système 6 — Auth (JWT Bearer + mTLS Node-only)

### §15.1 JWT Bearer — universel

```ts
// src/core/auth.ts (universal)

export function buildBearerHeader(jwt: string): string {
  if (!jwt) throw new ErrInvalidArgument('empty JWT');
  return `Bearer ${jwt}`;
}
```

```ts
// src/node/auth.ts ou src/web/auth.ts (mêmes lignes ; can be unified)

import type { Interceptor } from '@connectrpc/connect';
import type { AuthConfig } from '../core/options.js';

export function authInterceptor(cfg: AuthConfig): Interceptor {
  if (cfg.kind === 'none') return (next) => async (req) => next(req);
  if (cfg.kind === 'jwt') {
    const header = `Bearer ${cfg.token}`;
    return (next) => async (req) => {
      req.header.set('authorization', header);
      return next(req);
    };
  }
  // mTLS : auth happens at TLS handshake, not in headers.
  // The interceptor is a no-op ; the TLS config is set at transport time.
  return (next) => async (req) => next(req);
}
```

### §15.2 mTLS — Node only

```ts
// src/node/tls.ts

import { readFileSync } from 'node:fs';
import type { AuthConfig } from '../core/options.js';

export function resolveTLSCredentials(auth: AuthConfig): { rejectUnauthorized?: boolean; ca?: Buffer; cert?: Buffer; key?: Buffer } {
  if (auth.kind !== 'mtls') return {};
  return {
    cert: readFileSync(auth.certPath),
    key: readFileSync(auth.keyPath),
    ca: readFileSync(auth.caPath),
  };
}
```

### §15.3 mTLS impossible browser

Le navigateur n'expose PAS d'API JS pour configurer un client cert.
Le `<keygen>` HTML5 a été deprecated. Les TLS Client Certificates
sont gérés par l'OS / le profil utilisateur.

→ `EfeWebClient` n'a **PAS** d'option mTLS dans son `ClientOptions`
type. Le compilateur TypeScript bloque toute tentative.

---

## §16. Sub-système 7 — 3 framework examples

### §16.1 React example

```tsx
// examples/react/src/useEfeClient.ts
import { EfeClient } from '@efe/sdk/web';
import { useEffect, useMemo, useState } from 'react';

export function useEfeClient(opts: { endpoint: string; jwt: string }) {
  const client = useMemo(
    () => new EfeClient({ endpoint: opts.endpoint, auth: { kind: 'jwt', token: opts.jwt } }),
    [opts.endpoint, opts.jwt],
  );
  useEffect(() => () => { void client.close(); }, [client]);
  return client;
}

export function useClassify(client: EfeClient) {
  const [state, setState] = useState<{ loading: boolean; verdict?: Verdict; error?: Error }>({ loading: false });
  const classify = async (text: string) => {
    setState({ loading: true });
    try {
      const verdict = await client.classify(text);
      setState({ loading: false, verdict });
    } catch (error) {
      setState({ loading: false, error: error as Error });
    }
  };
  return { ...state, classify };
}
```

```tsx
// examples/react/src/App.tsx
import { useState } from 'react';
import { useEfeClient, useClassify } from './useEfeClient.js';

export function App() {
  const client = useEfeClient({
    endpoint: import.meta.env.VITE_EFE_ENDPOINT,
    jwt: import.meta.env.VITE_EFE_JWT,
  });
  const { loading, verdict, error, classify } = useClassify(client);
  const [text, setText] = useState('');

  return (
    <div>
      <textarea value={text} onChange={(e) => setText(e.target.value)} />
      <button disabled={loading || !text} onClick={() => classify(text)}>
        Classify
      </button>
      {loading && <p>Classifying…</p>}
      {error && <pre>Error: {error.message}</pre>}
      {verdict && (
        <pre>
          {JSON.stringify(verdict, null, 2)}
        </pre>
      )}
    </div>
  );
}
```

### §16.2 Vue 3 composable example

```ts
// examples/vue/src/useEfeClient.ts
import { onUnmounted, ref, type Ref } from 'vue';
import { EfeClient } from '@efe/sdk/web';
import type { Verdict } from '@efe/sdk';

export function useEfeClient(opts: { endpoint: string; jwt: string }): EfeClient {
  const client = new EfeClient({ endpoint: opts.endpoint, auth: { kind: 'jwt', token: opts.jwt } });
  onUnmounted(() => { void client.close(); });
  return client;
}

export function useClassify(client: EfeClient): {
  loading: Ref<boolean>;
  verdict: Ref<Verdict | undefined>;
  error: Ref<Error | undefined>;
  classify: (text: string) => Promise<void>;
} {
  const loading = ref(false);
  const verdict = ref<Verdict | undefined>(undefined);
  const error = ref<Error | undefined>(undefined);

  const classify = async (text: string) => {
    loading.value = true;
    error.value = undefined;
    verdict.value = undefined;
    try {
      verdict.value = await client.classify(text);
    } catch (e) {
      error.value = e as Error;
    } finally {
      loading.value = false;
    }
  };
  return { loading, verdict, error, classify };
}
```

### §16.3 Express middleware example

```ts
// examples/express/src/index.ts
import express from 'express';
import { EfeClient, errorCode, ErrUnauthenticated, ErrTimeout, ErrRateLimited } from '@efe/sdk/node';

const client = new EfeClient({
  endpoint: process.env.EFE_ENDPOINT!,
  auth: { kind: 'jwt', token: process.env.EFE_JWT_TOKEN! },
});

const app = express();
app.use(express.json());

app.post('/classify', async (req, res) => {
  const { text } = req.body;
  if (!text) return res.status(400).json({ error: 'text required' });
  try {
    const verdict = await client.classify(text, { signal: AbortSignal.timeout(10_000) });
    res.json(verdict);
  } catch (err) {
    res.status(efeErrorStatus(err)).json({ error: String(err), code: errorCode(err) });
  }
});

app.get('/healthz', async (_req, res) => {
  try {
    await client.healthcheck();
    res.json({ status: 'serving' });
  } catch (err) {
    res.status(503).json({ status: 'not_serving', error: String(err) });
  }
});

const port = parseInt(process.env.PORT ?? '3000', 10);
app.listen(port, () => console.log(`efe-express example on :${port}`));

function efeErrorStatus(err: unknown): number {
  if (err instanceof ErrUnauthenticated) return 401;
  if (err instanceof ErrTimeout) return 504;
  if (err instanceof ErrRateLimited) return 429;
  return 500;
}
```

### §16.4 Cohérence avec Python/Go SDKs

Les 3 examples TS produisent les **mêmes endpoints HTTP** que les
3 examples Go (gin/fiber/chi) et les futurs examples Python
(FastAPI/Flask) : `POST /classify` + `GET /healthz` avec mapping
identique sentinel → HTTP status. Pin test cross-language à venir
en M11+.

---

## §17. Sub-système 8 — Mock infrastructure

### §17.1 MockEfeClient — pure JS, no gRPC

```ts
// src/mock/client.ts

import type { Verdict } from '../core/verdict.js';
import { ErrClosed } from '../core/errors.js';
import { newVerdict } from './verdict.js';

/**
 * Drop-in replacement for EfeClient in unit tests. NO gRPC machinery.
 *
 * @example
 *   const mock = new MockEfeClient();
 *   mock.setVerdict(newVerdict({ label: 'fraud' }));
 *   await myHandler(mock); // myHandler: (client) => Promise<...>
 *   expect(mock.getCalls()).toEqual(['input text']);
 */
export class MockEfeClient {
  #verdict?: Verdict;
  #error?: Error;
  #calls: string[] = [];
  #closed = false;

  setVerdict(v: Verdict): void { this.#verdict = v; }
  setError(err: Error): void { this.#error = err; }
  getCalls(): readonly string[] { return [...this.#calls]; }
  getCallCount(): number { return this.#calls.length; }
  reset(): void {
    this.#verdict = undefined;
    this.#error = undefined;
    this.#calls = [];
    this.#closed = false;
  }

  async classify(text: string): Promise<Verdict> {
    if (this.#closed) throw new ErrClosed();
    this.#calls.push(text);
    if (this.#error) throw this.#error;
    if (this.#verdict) return { ...this.#verdict };
    return newVerdict();
  }

  async healthcheck(): Promise<void> {
    if (this.#closed) throw new ErrClosed();
    if (this.#error) throw this.#error;
  }

  async close(): Promise<void> { this.#closed = true; }
  get closed(): boolean { return this.#closed; }
}
```

### §17.2 newVerdict factory

```ts
// src/mock/verdict.ts

import type { Verdict } from '../core/verdict.js';

export interface VerdictOverrides {
  label?: string;
  confidence?: number;
  reject?: boolean;
  rounds?: number;
  alignmentDelta?: number;
  catalog?: string;
  zone?: string;
  swarmId?: string;
  requestId?: string;
  elapsedMs?: number;
}

const DEFAULTS: Verdict = Object.freeze({
  label: 'alpha',
  confidence: 0.85,
  reject: false,
  rounds: 4,
  alignmentDelta: 0.10,
  catalog: 'test/cat@v1',
  zone: 'trusted',
  swarmId: 'mock-swarm',
  requestId: '',
  elapsedMs: 42,
});

export function newVerdict(overrides: VerdictOverrides = {}): Verdict {
  const v = { ...DEFAULTS, ...overrides };
  // Auto-derive zone if not explicitly set.
  if (!('zone' in overrides) && !v.reject) {
    if (v.confidence >= 0.80) v.zone = 'trusted';
    else if (v.confidence >= 0.50) v.zone = 'warn';
    else v.zone = 'reject';
  }
  return Object.freeze(v);
}
```

### §17.3 In-memory mock server (Node only)

```ts
// src/mock/server.ts (Node only — uses @grpc/grpc-js)

import { ServerCredentials, type Server } from '@grpc/grpc-js';
import { ConnectRouter, createGrpcServer } from '@connectrpc/connect-node';
// ...

export interface MockServer {
  port: number;
  endpoint: string;
  onClassify(handler: (req: ExecuteRequest) => Promise<ExecuteResponse>): void;
  close(): Promise<void>;
}

export async function createMockServer(opts: { port?: number } = {}): Promise<MockServer> {
  // Spawn @grpc/grpc-js Server on a free port (or specified).
  // Register the EFE service handlers wired through to user-provided callbacks.
  // Return a handle with `close()`.
  // Implementation : ~80 LOC.
  ...
}
```

### §17.4 Playwright fixtures

```ts
// tests/e2e/fixtures/test-server.ts

import { test as base } from '@playwright/test';
import { spawn, type ChildProcess } from 'node:child_process';

export const test = base.extend<{ envoy: ChildProcess; viteServer: ChildProcess }>({
  envoy: async ({}, use) => {
    const proc = spawn('envoy', ['-c', 'tests/e2e/fixtures/envoy.yaml']);
    await waitForPort(50052, 5_000);
    await use(proc);
    proc.kill();
  },
  viteServer: async ({}, use) => {
    const proc = spawn('pnpm', ['dev'], { cwd: 'examples/react' });
    await waitForPort(5173, 10_000);
    await use(proc);
    proc.kill();
  },
});
```

---

## §18. Stratégie de tests

### §18.1 Tests pyramid

```
                          /\
                         /  \  Playwright e2e (~5 tests, browser)
                        /----\
                       /      \
                      / Vitest \  ~10 tests integration (in-mem mock)
                     /----------\
                    /            \
                   /   Vitest     \  ~30 tests unit (no I/O)
                  /----------------\
                 /                  \
                /   Pin tests Python \  ~32 tests structure
               /______________________\
```

### §18.2 Vitest unit tests (≥ 30)

| File | Tests | Focus |
|---|:---:|---|
| `tests/unit/verdict.test.ts` | 4 | Verdict immutability, isReject/isWarn/isTrusted helpers, JSON shape |
| `tests/unit/errors.test.ts` | 6 | 10 sentinels distinct, isRetryable subset, errorCode mapping, instanceof EfeError |
| `tests/unit/options.test.ts` | 8 | Options validation : empty endpoint, neg deadline, bad retry, JWT empty, etc. |
| `tests/unit/retry.test.ts` | 5 | ExponentialBackoff defaults, monotonic growth, max cap, never negative, jitter |
| `tests/unit/auth.test.ts` | 3 | buildBearerHeader, mtls config validation (Node), no-op interceptor on 'none' |
| `tests/unit/from-rpc.test.ts` | 4 | ConnectError → sentinel mapping, all 10 codes |

### §18.3 Vitest integration (≥ 10)

```ts
// tests/integration/classify.test.ts
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { EfeClient } from '@efe/sdk/node';
import { createMockServer, type MockServer } from '@efe/sdk/mock';

let server: MockServer;
let client: EfeClient;

beforeEach(async () => {
  server = await createMockServer();
  client = new EfeClient({ endpoint: server.endpoint });
});

afterEach(async () => {
  await client.close();
  await server.close();
});

describe('EfeClient.classify', () => {
  it('returns the server-side verdict', async () => {
    server.onClassify(async (req) => ({
      output: { text: 'billing' },
      confidence: 0.91,
      verdict: 'CONVERGED',
      // ...
    }));
    const verdict = await client.classify('I want a refund');
    expect(verdict.label).toBe('billing');
    expect(verdict.confidence).toBeCloseTo(0.91);
    expect(verdict.zone).toBe('trusted');
  });

  it('throws ErrInvalidArgument on empty text', async () => {
    await expect(client.classify('  ')).rejects.toThrow(/empty text/);
  });

  it('throws ErrClosed after close()', async () => {
    await client.close();
    await expect(client.classify('foo')).rejects.toThrow(/closed/);
  });

  it('retries on Unavailable (3 attempts)', async () => {
    let calls = 0;
    server.onClassify(async () => {
      calls += 1;
      if (calls < 3) throw new ConnectError(ConnectCode.Unavailable, 'transient');
      return { output: { text: 'ok' }, confidence: 0.9, ... };
    });
    const v = await client.classify('foo');
    expect(calls).toBe(3);
    expect(v.label).toBe('ok');
  });

  // ... 6 more cases
});
```

### §18.4 Playwright e2e (≥ 5)

```ts
// tests/e2e/browser-classify.spec.ts
import { expect } from '@playwright/test';
import { test } from './fixtures/test-server.js';

test.describe('Browser classify e2e', () => {
  test('happy path : classify text via grpc-web', async ({ page, envoy, viteServer }) => {
    await page.goto('http://localhost:5173/');
    await page.fill('textarea', 'I want a refund');
    await page.click('button:has-text("Classify")');
    await expect(page.locator('pre')).toContainText('"label"');
    await expect(page.locator('pre')).toContainText('billing');
  });

  test('JWT auth flows in Authorization header', async ({ page }) => {
    // ...
  });

  test('handles 401 unauthorized', async ({ page }) => {
    // ...
  });

  test('handles 503 server unavailable', async ({ page }) => {
    // ...
  });

  test('CORS preflight succeeds', async ({ page }) => {
    // ...
  });
});
```

### §18.5 Pin tests Python (≈ 32)

Section §27.4 détaille les ~32 pin tests. Sommaire :

| Section | Tests |
|---|:---:|
| §1. Module skeleton | 5 |
| §2. version.ts | 1 |
| §3. Public API surface (28 exports) | 7 |
| §4. Verdict + cross-lang pin | 3 |
| §5. Errors taxonomy | 2 |
| §6. Metrics canonical names | 1 |
| §7. Build outputs (ESM + CJS + dts) | 2 |
| §8. Examples (React + Vue + Express) | 3 |
| §9. Scripts shell | 4 |
| §10. CI workflows | 2 |
| §11. PROTOS_HASH drift | 1 |
| §12. Architecture doc presence | 1 |
| **Total** | **32** |

### §18.6 Coverage target

`pnpm test --coverage` :
* Lines : ≥ 80%
* Statements : ≥ 80%
* Functions : ≥ 80%
* Branches : ≥ 70%

`vitest.config.ts` :

```ts
export default defineConfig({
  test: {
    coverage: {
      provider: 'v8',
      reporter: ['text', 'html', 'lcov'],
      thresholds: {
        lines: 80,
        statements: 80,
        functions: 80,
        branches: 70,
      },
      exclude: ['src/_generated/**', 'tests/**', 'examples/**'],
    },
  },
});
```

---

## §19. Browser limitations + grpc-web subset enumeration

### §19.1 Limitations protocole

| Limitation | gRPC native (Node) | gRPC-Web (Browser) |
|---|---|---|
| Transport | HTTP/2 | HTTP/1.1 (HTTP/2 fallback in some impls) |
| Trailers | Native HTTP/2 trailers | Encoded in body (`grpc-status` trailer-frame) |
| Streaming | Unary + Server-stream + Client-stream + Bidi | Unary + Server-stream only |
| Metadata size | 8 KB+ typical | Limited by HTTP/1.1 headers (4-8 KB) |
| Compression | gzip, deflate | identity (compression server-side rare) |
| Timeouts | grpc-timeout header | grpc-timeout header (translated) |
| Cancellation | RST_STREAM | AbortController.abort() → fetch cancel |
| Connection pooling | HTTP/2 multiplexing | HTTP/1.1 keep-alive (less efficient) |

### §19.2 RPCs exposés dans EfeWebClient

```ts
// src/web/client.ts — public methods only

class EfeWebClient {
  classify(text: string, opts?: CallOptions): Promise<Verdict>;
  healthcheck(opts?: CallOptions): Promise<void>;
  close(): Promise<void>;
  // PAS de :
  // - registerCatalog
  // - evolveCatalog
  // - deleteCatalog
  // - rotateKeys / updateConfig (admin)
  // - classifyStream (server-streaming OK in protocol but D24 deferred)
}
```

### §19.3 CORS headers required

Le serveur EFE (Lettre E ou Envoy translator) doit retourner :

```
Access-Control-Allow-Origin: <consumer-origin>
Access-Control-Allow-Methods: POST, OPTIONS
Access-Control-Allow-Headers: authorization, content-type, x-grpc-web, x-user-agent
Access-Control-Expose-Headers: grpc-status, grpc-message
Access-Control-Allow-Credentials: false
Access-Control-Max-Age: 86400
```

### §19.4 Envoy filter config (fixture e2e)

```yaml
# tests/e2e/fixtures/envoy.yaml
admin:
  address:
    socket_address: { address: 0.0.0.0, port_value: 9901 }
static_resources:
  listeners:
    - name: listener_0
      address:
        socket_address: { address: 0.0.0.0, port_value: 50052 }
      filter_chains:
        - filters:
            - name: envoy.filters.network.http_connection_manager
              typed_config:
                "@type": type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager
                stat_prefix: ingress_http
                codec_type: AUTO
                http_filters:
                  - name: envoy.filters.http.grpc_web
                  - name: envoy.filters.http.cors
                  - name: envoy.filters.http.router
                route_config:
                  name: local_route
                  virtual_hosts:
                    - name: backend
                      domains: ["*"]
                      cors:
                        allow_origin_string_match: [{ exact: "*" }]
                        allow_methods: POST, OPTIONS
                        allow_headers: authorization,content-type,x-grpc-web,x-user-agent
                        max_age: "86400"
                      routes:
                        - match: { prefix: "/" }
                          route: { cluster: efe_grpc, timeout: 30s }
  clusters:
    - name: efe_grpc
      type: STRICT_DNS
      lb_policy: ROUND_ROBIN
      http2_protocol_options: {}
      load_assignment:
        cluster_name: efe_grpc
        endpoints:
          - lb_endpoints:
              - endpoint:
                  address:
                    socket_address: { address: 127.0.0.1, port_value: 50051 }
```

---

## §20. Bundle size budget + tree-shaking + dual ESM/CJS

### §20.1 Budget

| Cible | Bundle entry | Limit (gz) | Hard fail in CI |
|---|---|:---:|:---:|
| Browser | `dist/web/index.js` | 50 KB | ✓ |
| Node ESM | `dist/node/index.js` | 250 KB | ✓ |
| Node CJS | `dist/node/index.cjs` | 250 KB | ✓ |
| Mock | `dist/mock/index.js` | 30 KB | ✓ |

### §20.2 `bundlesize` config

```js
// scripts/bundlesize.mjs
import { gzipSize } from 'gzip-size';
import { readFileSync } from 'node:fs';

const TARGETS = [
  { path: 'dist/web/index.js', limit: 50 * 1024 },
  { path: 'dist/node/index.js', limit: 250 * 1024 },
  { path: 'dist/node/index.cjs', limit: 250 * 1024 },
  { path: 'dist/mock/index.js', limit: 30 * 1024 },
];

let failed = false;
for (const t of TARGETS) {
  const size = await gzipSize(readFileSync(t.path));
  const ok = size <= t.limit;
  console.log(`${ok ? '✅' : '❌'} ${t.path}: ${size}b gz (limit ${t.limit}b)`);
  if (!ok) failed = true;
}
if (failed) process.exit(1);
```

### §20.3 Tree-shake strategy

Toutes les exports sont **named**, pas de default exports. Side-effect-free
package via `package.json` :

```json
{
  "sideEffects": false
}
```

Permet aux bundlers d'éliminer `MockEfeClient` quand l'app n'importe
que `EfeClient`. Pin test : `import { Verdict } from '@efe/sdk'`
seul → bundle final < 5 KB gz.

### §20.4 Dual ESM + CJS strategy

```ts
// tsup.config.ts
import { defineConfig } from 'tsup';

export default defineConfig([
  {
    entry: { index: 'src/node.ts' },
    outDir: 'dist/node',
    format: ['esm', 'cjs'],
    target: 'node18',
    dts: true,
    sourcemap: true,
    clean: true,
    splitting: false,
    treeshake: true,
  },
  {
    entry: { index: 'src/web.ts' },
    outDir: 'dist/web',
    format: ['esm'],
    target: 'es2022',
    dts: true,
    sourcemap: true,
    splitting: false,
    treeshake: true,
    platform: 'browser',
  },
  {
    entry: { index: 'src/mock.ts' },
    outDir: 'dist/mock',
    format: ['esm', 'cjs'],
    target: 'node18',
    dts: true,
    sourcemap: true,
    splitting: false,
    treeshake: true,
  },
]);
```

---

## §21. SLO mapping

| SLO | Méthode mesure | Cible | Pin test |
|---|---|---|---|
| SLO-TS-1 cold start < 30 ms | Vitest microbench `new EfeClient()` | < 30 ms | DT-T-9 |
| SLO-TS-2 p99 ≤ server p99 + 30 ms | Integration test 1k iterations | < 30 ms overhead | DT-T-10 |
| SLO-TS-3 bundle browser ≤ 50 KB gz | `pnpm bundlesize` | ≤ 50 KB | DT-T-11 |
| SLO-TS-4 bundle node ≤ 250 KB gz | idem | ≤ 250 KB | DT-T-11 |
| SLO-TS-5 tree-shake | `pnpm bundlesize:treeshake` | < 5 KB pour Verdict-only import | DT-T-12 |
| SLO-TS-6 TS strict ≥ 5.4 | `pnpm typecheck` matrix | green | DT-T-13 |
| SLO-TS-7 typedoc clean | `pnpm typedoc` | 0 warning | DT-T-14 |
| SLO-TS-8 coverage ≥ 80% | Vitest | ≥ 80% lines | DT-T-15 |

---

## §22. Risques transverses + mitigations

| Risque | P | I | Mitigation |
|---|:-:|:-:|---|
| Connect-RPC TS lib breaking change pre-GA | M | H | Pin major version `^1.4.0` ; tests CI re-run on dependabot bumps |
| Browser TLS errors invisible | H | M | Documenter le besoin Envoy + serveur :50052 ; smoke test daily |
| Bundle size creep par accumulation deps | M | M | `bundlesize` CI bloquant ; chaque PR voit le delta |
| Playwright flakiness en CI | H | L | retries: 2 ; quarantine flaky tests |
| Edge runtimes non testés | L | M | Ajouter une matrice Vercel Edge en post-D24 |
| `@grpc/grpc-js` major bump (HTTP/2 quirks) | L | H | Pin major ; tests matrix Node 18/20/22 |
| Cross-lang Verdict drift Python/Go/TS | M | H | Pin test cross-lang `test_d24_verdict_field_set_matches_m8_m9` |
| Examples React/Vue versions deprecated | L | L | Pin major ; bump quarterly |

---

## §23. Volumétrie code attendue

| Composant | Hand-coded LOC | Generated LOC | Total |
|---|---:|---:|---:|
| `src/core/` (verdict, errors, options, retry, auth, otel, from-rpc, version) | 850 | — | 850 |
| `src/node/` (client, transport, tls, interceptors, metrics, stream) | 750 | — | 750 |
| `src/web/` (client, transport, interceptors, auth) | 400 | — | 400 |
| `src/mock/` (client, verdict, server) | 350 | — | 350 |
| `src/index.ts` + `src/node.ts` + `src/web.ts` + `src/mock.ts` | 80 | — | 80 |
| `src/_generated/` (protoc-gen-es + connect-es output) | — | 1 500 | 1 500 |
| Tests Vitest (unit + integration) | 950 | — | 950 |
| Tests Playwright | 250 | — | 250 |
| Examples React + Vue + Express | 600 | — | 600 |
| Scripts shell + node | 250 | — | 250 |
| CI workflows YAML | 400 | — | 400 |
| Pin tests Python | 700 | — | 700 |
| `package.json` + tsconfig + tsup + vitest + playwright + buf configs | 250 | — | 250 |
| README + CHANGELOG + LICENSE | 350 | — | 350 |
| **Total** | **~6 280 LOC hand** | **~1 500 LOC gen** | **~7 780 LOC** |

---

## §24. Plan d'implémentation J1-J10

### J1 — Bootstrap monorepo

* `pnpm-workspace.yaml` + root `package.json`.
* `clients/typescript/@efe/sdk/package.json` (squelette).
* `tsconfig.json` strict + `tsconfig.build.json`.
* `buf.yaml` + `buf.gen.yaml`.
* `.gitignore` + `.npmignore`.
* `scripts/gen-proto.sh`.
* `pnpm install` + `pnpm gen:proto` produit `_generated/` (test localement).
* `git commit` initial : "chore(m10): scaffold typescript sdk workspace".

### J2 — Core types

* `src/core/version.ts` : `export const Version = '0.3.0'`.
* `src/core/verdict.ts` : `Verdict` interface + `StreamItem`.
* `src/core/errors.ts` : 10 sentinel classes + `EfeError` base + `isRetryable` + `errorCode`.
* `src/core/options.ts` : `ClientOptions` + `AuthConfig` + `validateOptions`.
* `tests/unit/verdict.test.ts` + `errors.test.ts` + `options.test.ts`.
* `pnpm typecheck` + `pnpm test` GREEN.

### J3 — Node client + happy path

* `src/node/client.ts` : `EfeClient` class.
* `src/node/transport.ts` : wraps `createGrpcTransport` from connect-node.
* `src/mock/server.ts` : `createMockServer()` (in-memory gRPC).
* `tests/integration/classify.test.ts` happy path GREEN.

### J4 — Auth + retry + interceptors (Node)

* `src/node/auth.ts` : `authInterceptor`.
* `src/node/tls.ts` : `resolveTLSCredentials`.
* `src/core/retry.ts` : `ExponentialBackoff` + `RetryConfig`.
* `src/node/interceptors.ts` : `retryInterceptor`.
* Tests : auth header asserted, retry 3x on Unavailable, mTLS config produces TLS 1.3.

### J5 — OTel + Prometheus + mock package

* `src/core/otel.ts` : `otelInterceptor` (universal, soft-dep).
* `src/node/metrics.ts` : `SdkMetrics` + `promInterceptor`.
* `src/mock/client.ts` : `MockEfeClient`.
* `src/mock/verdict.ts` : `newVerdict`.
* `src/mock.ts` re-exports.
* Tests : 5 metric names match canonical, OTel span shape.

### J6 — Browser client + Envoy fixture

* `src/web/client.ts` : `EfeWebClient`.
* `src/web/transport.ts` : `createGrpcWebTransport` wrapper.
* `src/web/auth.ts` : browser-friendly auth (no fs).
* `src/web/interceptors.ts`.
* `src/web.ts` re-exports.
* `tests/e2e/fixtures/envoy.yaml` + `test-server.ts`.
* First Playwright e2e GREEN (Chromium).

### J7 — 3 framework examples

* `examples/react/` (Vite + React 18 + TS).
* `examples/vue/` (Vite + Vue 3 + TS).
* `examples/express/` (Node + tsx + Express).
* Chaque example `pnpm dev` lance + curl/page load OK manuellement.

### J8 — Tests massifs + coverage

* Compléter Vitest unit (target 30+).
* Compléter Vitest integration (target 10+).
* Compléter Playwright e2e (target 5+).
* Coverage > 80% lines.
* Fix any flakiness.

### J9 — Pin tests Python + scripts/ + CI workflows

* `scripts/tests/test_d24_typescript_sdk_extension.py` (~32 tests).
* `scripts/publish.sh` + `scripts/bundlesize.mjs` + `scripts/lint.sh`.
* `.github/workflows/sdk-typescript-test.yml`.
* `.github/workflows/sdk-typescript-smoke.yml`.
* `.github/workflows/sdk-typescript-publish.yml` (gated on tag).
* `pytest scripts/tests/test_d24_*.py` GREEN.

### J10 — Audit pass 1 + CHANGELOG + dry-run publish

* Audit chirurgical (à la M9) : surface, types, exports, sentinels distinctness, bundle size.
* Apply audit fixes (≈ 10 expected).
* Add audit-fix pin tests.
* `npm pack --dry-run` valide.
* `npm publish --dry-run --access=restricted` valide.
* CHANGELOG.md M10 D24 entrée [Unreleased].
* PR ouverte, CI green.

---

## §25. Versioning + publication npm

### §25.1 Semver alignment

* `0.3.0` = D24 ship (private, scope `@efe`, `npm publish --access=restricted`).
* `0.3.1..0.3.x` = audit fixes + patches privés.
* `0.4.0` = D26+ extension (streaming browser, edge runtime support).
* `1.0.0` = GA public, `npm publish --access=public`. Module path
  `@efe/sdk` immuable.

### §25.2 Private publish (Φ-B)

```bash
# Pre-flight : ensure NPM_TOKEN is set with the org-bot account
# token that has publish-restricted scope.

cd clients/typescript/@efe/sdk
pnpm install --frozen-lockfile
pnpm gen:proto
pnpm test
pnpm bundlesize
pnpm build
npm publish --access=restricted
```

### §25.3 Public publish (GA)

```bash
# Bump version.ts + package.json to 1.0.0
# Then :
npm publish --access=public
# Effect : the package is publicly available on https://www.npmjs.com/package/@efe/sdk
```

### §25.4 Consumer install

Pendant Φ-B :

```bash
# .npmrc must contain :
# @efe:registry=https://npm.pkg.github.com
# //npm.pkg.github.com/:_authToken=${NPM_TOKEN}
pnpm add @efe/sdk
```

Post-GA :

```bash
pnpm add @efe/sdk
# (no .npmrc tweak needed — public registry default)
```

---

## §26. Compliance + sécurité

### §26.1 Supply-chain

* `pnpm overrides` pin les transitives critiques.
* `pnpm audit` OK requis avant publish.
* Provenance attestation via `npm publish --provenance` (sigstore).
* `package-lock.json` (or `pnpm-lock.yaml`) committed.
* Dependabot weekly audits enabled.

### §26.2 CSP-friendly

Le SDK browser doit fonctionner sous :

```
Content-Security-Policy: default-src 'self'; script-src 'self'; connect-src 'self' https://api.efe.ai
```

→ pas de `eval`, pas de `Function()`, pas de `new Function()`. Vérification
pin test `test_d24_no_eval_in_browser_bundle`.

### §26.3 Secret redaction

Le SDK ne **jamais** log la JWT en clair. Test pin :

```python
def test_d24_jwt_never_in_logs():
    """Browser + Node code MUST NOT String.includes the jwt token in any log path."""
    # Static check : no `console.log` of `auth.token`, `opts.auth.token`, etc.
    for f in (SDK_ROOT / 'src/node/client.ts', SDK_ROOT / 'src/web/client.ts'):
        src = f.read_text()
        assert 'auth.token' not in src or 'console.log(' not in src
```

### §26.4 mTLS material handling

Sur Node, les certs/keys sont lus avec `fs.readFileSync` au constructor
time. Le buffer est passé directement au transport, jamais sérialisé,
jamais loggé. La méthode `toString()` du client redacte (renvoie
`<MTLS>` placeholder).

---

## §27. Annexes — `package.json` canonique

```json
{
  "name": "@efe/sdk",
  "version": "0.3.0",
  "description": "Official TypeScript SDK for EFE — isomorphic Node + Browser, type-safe end-to-end.",
  "license": "Apache-2.0",
  "author": "EFE team <oss@efe-ai.com>",
  "homepage": "https://github.com/efe-ai/efe",
  "repository": {
    "type": "git",
    "url": "https://github.com/efe-ai/efe.git",
    "directory": "clients/typescript/@efe/sdk"
  },
  "type": "module",
  "packageManager": "pnpm@9.7.0",
  "engines": { "node": ">=18" },
  "sideEffects": false,
  "exports": {
    ".": {
      "browser": {
        "types": "./dist/web/index.d.ts",
        "import": "./dist/web/index.js"
      },
      "node": {
        "types": "./dist/node/index.d.ts",
        "import": "./dist/node/index.js",
        "require": "./dist/node/index.cjs"
      },
      "default": {
        "types": "./dist/web/index.d.ts",
        "import": "./dist/web/index.js"
      }
    },
    "./node": {
      "types": "./dist/node/index.d.ts",
      "import": "./dist/node/index.js",
      "require": "./dist/node/index.cjs"
    },
    "./web": {
      "types": "./dist/web/index.d.ts",
      "import": "./dist/web/index.js"
    },
    "./mock": {
      "types": "./dist/mock/index.d.ts",
      "import": "./dist/mock/index.js",
      "require": "./dist/mock/index.cjs"
    },
    "./package.json": "./package.json"
  },
  "files": [
    "dist/",
    "README.md",
    "LICENSE",
    "CHANGELOG.md"
  ],
  "scripts": {
    "build": "tsup",
    "typecheck": "tsc --noEmit",
    "test": "vitest run --coverage",
    "test:watch": "vitest",
    "test:e2e": "playwright test",
    "lint": "biome check src tests",
    "lint:fix": "biome check --apply src tests",
    "gen:proto": "./scripts/gen-proto.sh",
    "bundlesize": "node scripts/bundlesize.mjs",
    "publish:dry": "pnpm build && npm publish --dry-run --access=restricted"
  },
  "dependencies": {
    "@bufbuild/protobuf": "^2.1.0",
    "@connectrpc/connect": "^1.4.0",
    "@connectrpc/connect-node": "^1.4.0",
    "@connectrpc/connect-web": "^1.4.0"
  },
  "peerDependencies": {
    "@grpc/grpc-js": "^1.10.0",
    "@opentelemetry/api": "^1.9.0",
    "prom-client": "^15.1.0"
  },
  "peerDependenciesMeta": {
    "@grpc/grpc-js": { "optional": true },
    "@opentelemetry/api": { "optional": true },
    "prom-client": { "optional": true }
  },
  "devDependencies": {
    "@biomejs/biome": "^1.8.0",
    "@bufbuild/protoc-gen-es": "^2.1.0",
    "@connectrpc/protoc-gen-connect-es": "^1.4.0",
    "@playwright/test": "^1.45.0",
    "@types/node": "^20.14.0",
    "@vitest/coverage-v8": "^1.6.0",
    "gzip-size": "^7.0.0",
    "tsup": "^8.2.0",
    "typedoc": "^0.26.0",
    "typescript": "^5.4.0",
    "vitest": "^1.6.0"
  }
}
```

---

## §28. Annexes — surface API TS complète (`.d.ts`)

```ts
// dist/node/index.d.ts (synthetic — actual file is auto-generated)

// ===== Universal (re-exported in node + web + mock) =====

export interface Verdict {
  readonly label: string;
  readonly confidence: number;
  readonly reject: boolean;
  readonly rounds: number;
  readonly alignmentDelta: number;
  readonly catalog: string;
  readonly zone: 'trusted' | 'warn' | 'reject' | string;
  readonly swarmId: string;
  readonly requestId: string;
  readonly elapsedMs: number;
}

export interface StreamItem {
  readonly index: number;
  readonly verdict?: Verdict;
  readonly error?: Error;
}

export interface ClientOptions {
  readonly endpoint: string;
  readonly deadline?: number;
  readonly auth?: AuthConfig;
  readonly retry?: RetryConfig;
  readonly otel?: OTelConfig;
  readonly prometheus?: PrometheusConfig;
  readonly userAgent?: string;
  readonly maxReceivedMessageSize?: number;
  readonly fetch?: typeof globalThis.fetch;
}

export type AuthConfig =
  | { readonly kind: 'jwt'; readonly token: string }
  | { readonly kind: 'mtls'; readonly certPath: string; readonly keyPath: string; readonly caPath: string }
  | { readonly kind: 'none' };

export interface RetryConfig {
  readonly maxAttempts: number;
  readonly backoff: RetryBackoff;
}

export interface RetryBackoff {
  next(attempt: number): number;
}

export class ExponentialBackoff implements RetryBackoff {
  constructor(opts?: {
    initialIntervalMs?: number;
    maxIntervalMs?: number;
    multiplier?: number;
  });
  next(attempt: number): number;
}

export class EfeError extends Error {
  readonly code: string;
}
export class ErrUnauthenticated extends EfeError { /* */ }
export class ErrPermissionDenied extends EfeError { /* */ }
export class ErrCatalogNotFound extends EfeError { /* */ }
export class ErrCatalogInvalid extends EfeError { /* */ }
export class ErrTimeout extends EfeError { /* */ }
export class ErrUnavailable extends EfeError { /* */ }
export class ErrRateLimited extends EfeError { /* */ }
export class ErrInvalidArgument extends EfeError { /* */ }
export class ErrInternal extends EfeError { /* */ }
export class ErrClosed extends EfeError { /* */ }

export function isRetryable(err: unknown): boolean;
export function errorCode(err: unknown): string;

export const Version: string;

// ===== Node-only =====

export class EfeClient {
  constructor(opts: ClientOptions);
  classify(text: string, opts?: CallOptions): Promise<Verdict>;
  healthcheck(opts?: CallOptions): Promise<void>;
  classifyStream(texts: AsyncIterable<string> | Iterable<string>, opts?: ClassifyStreamOptions): AsyncIterable<StreamItem>;
  close(): Promise<void>;
  readonly closed: boolean;
  toString(): string;
}

export interface CallOptions {
  readonly signal?: AbortSignal;
  readonly timeout?: number;
}

export interface ClassifyStreamOptions extends CallOptions {
  readonly concurrency?: number;
}

// ===== Browser-only (in dist/web/) =====
// EfeClient resolves to EfeWebClient in browser context.

export class EfeWebClient {
  constructor(opts: ClientOptions);
  classify(text: string, opts?: CallOptions): Promise<Verdict>;
  healthcheck(opts?: CallOptions): Promise<void>;
  close(): Promise<void>;
  readonly closed: boolean;
  toString(): string;
}

// ===== Mock =====
export class MockEfeClient {
  setVerdict(v: Verdict): void;
  setError(err: Error): void;
  getCalls(): readonly string[];
  getCallCount(): number;
  reset(): void;
  classify(text: string): Promise<Verdict>;
  healthcheck(): Promise<void>;
  close(): Promise<void>;
  readonly closed: boolean;
}

export function newVerdict(overrides?: VerdictOverrides): Verdict;
export interface VerdictOverrides { /* ... */ }

export interface MockServer {
  readonly endpoint: string;
  onClassify(handler: ClassifyHandler): void;
  close(): Promise<void>;
}
export function createMockServer(opts?: { port?: number }): Promise<MockServer>;
```

---

## §29. Annexes — `buf.yaml` + `buf.gen.yaml` + scripts

### §29.1 `buf.yaml`

```yaml
version: v2
modules:
  - path: ../../../../protos
breaking:
  use: [FILE]
lint:
  use: [DEFAULT]
```

### §29.2 `buf.gen.yaml`

```yaml
version: v2
managed:
  enabled: true
plugins:
  - local: protoc-gen-es
    out: src/_generated
    opt: [target=ts, import_extension=.js]
  - local: protoc-gen-connect-es
    out: src/_generated
    opt: [target=ts, import_extension=.js]
```

### §29.3 `scripts/publish.sh`

```bash
#!/usr/bin/env bash
# Sprint D24 — publish @efe/sdk via npm.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

PUBLISH="${PUBLISH:-private}"
VERSION=$(node -p "require('./package.json').version")

# Pre-flight.
[[ -z "$(git status --porcelain)" ]] || { echo "FATAL: dirty tree"; exit 3; }
git rev-parse "v$VERSION" >/dev/null 2>&1 && { echo "FATAL: tag already exists"; exit 4; }

# Build + test.
pnpm install --frozen-lockfile
pnpm gen:proto
pnpm typecheck
pnpm test
pnpm bundlesize
pnpm build

case "$PUBLISH" in
  private)
    npm publish --access=restricted
    git tag -s "v$VERSION" -m "@efe/sdk v$VERSION (private)"
    git push origin "v$VERSION"
    ;;
  public)
    [[ "$VERSION" =~ ^1\. ]] || { echo "FATAL: PUBLISH=public requires v1.x.x"; exit 5; }
    npm publish --access=public
    git tag -s "v$VERSION" -m "@efe/sdk v$VERSION (PUBLIC)"
    git push origin "v$VERSION"
    ;;
  *)
    echo "FATAL: unknown PUBLISH=$PUBLISH"; exit 1
    ;;
esac
```

---

## §30. Annexes — RACI + budget + Gantt

### §30.1 RACI

| Activity | R | A | C | I |
|---|:-:|:-:|:-:|:-:|
| Architecture doc (this) | rodrigue | rodrigue | — | team |
| Bootstrap monorepo + buf | rodrigue | rodrigue | — | team |
| Core types + sentinels | rodrigue | rodrigue | — | team |
| Node client | rodrigue | rodrigue | — | team |
| Browser client + Envoy | rodrigue | rodrigue | platform-eng | team |
| 3 examples | rodrigue | rodrigue | — | team |
| Pin tests Python | rodrigue | rodrigue | — | team |
| CI workflows | rodrigue | rodrigue | devops | team |
| npm scope + access | rodrigue | platform-eng | rodrigue | team |
| Publish dry-run + GA | rodrigue | rodrigue | platform-eng | team |

### §30.2 Budget temps

10 jours wallclock × 1 dev = 80 h. Dépenses (estimations) :

| Phase | Heures | Cumulé |
|---|---:|---:|
| J1 Bootstrap | 6 | 6 |
| J2 Core types | 8 | 14 |
| J3 Node client | 10 | 24 |
| J4 Auth+retry+TLS | 8 | 32 |
| J5 OTel+Prom+mock | 8 | 40 |
| J6 Browser+Envoy | 10 | 50 |
| J7 3 examples | 8 | 58 |
| J8 Tests massifs | 10 | 68 |
| J9 Pin+scripts+CI | 8 | 76 |
| J10 Audit+publish | 4 | 80 |

### §30.3 Gantt

```
J1 │█ Bootstrap
J2 │ █ Core
J3 │  █ Node client
J4 │   █ Auth+retry
J5 │    █ OTel+mock
J6 │     ██ Browser+Envoy
J7 │       █ Examples
J8 │        █ Tests
J9 │         █ Pin+CI
J10│          █ Audit+publish
```

---

## §31. Mises à jour du document

| Date | Version | Auteur | Diff |
|---|---|---|---|
| 2026-04-29 | v1.0.0 | rodrigue | Création initiale (greenfield) |

---

**Fin du document EFE_TYPESCRIPT_SDK_EXTENSION_ARCHITECTURE.md (M10 D24).**
