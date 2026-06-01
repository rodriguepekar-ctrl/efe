# M10 — API Layer (gRPC server + Python SDK)
## Architecture complète, ultra-détaillée — v1.0

---

**Document** : spécification d'architecture (contrat d'implémentation)
**Modules** :
- `efe-server` crate (binaire `efe-server`, serveur gRPC tonic)
- `efe-ffi` crate (bindings PyO3 pour usage in-process)
- `python/efe` paquet (SDK client gRPC async)
- `protos/efe/v1/` (descripteurs proto versionnés)

**Sprints ciblés** : Sprint 7 (proto + serveur unary MVP) → Sprint 8 (auth + rate limit + streaming) → Sprint 9 (SDK Python + PyO3 FFI + load testing)

**Dépend de** :
- M1 Core Runtime (`SwarmController`, `Request`, `Response`, `DecodeMode`, `TraceContext`, `Verdict`)
- M2 Memory Pool (budget `ResourceBudget`, erreurs `BudgetExhausted`)
- M9 Alignment Registry (admin API pour cones, breach reports)
- M11 Telemetry (traces OpenTelemetry, métriques Prometheus, logs JSON)

**Consommé par** :
- Clients externes (Python SDK, gRPC reflection, toute stack gRPC-compatible : Go/Java/JS/Swift)
- M12 (packaging Docker + CI)

**Statut** : spec **gelée** pour implémentation. Toute dérive sans ADR rejetée en revue. Versioning proto suit semver strict — toute breaking change = nouveau package `v1beta2` / `v2`.

---

## Table des matières

1. [Mission et positionnement](#1-mission-et-positionnement)
2. [Scope MVP vs hors-périmètre](#2-scope-mvp-vs-hors-périmètre)
3. [Vue d'ensemble architecturale](#3-vue-densemble-architecturale)
4. [Descripteur proto : `efe.v1`](#4-descripteur-proto--efev1)
5. [Couches du serveur Rust `efe-server`](#5-couches-du-serveur-rust-efe-server)
6. [Mapping proto ↔ domain (M1) : règles exactes](#6-mapping-proto--domain-m1--règles-exactes)
7. [Intercepteurs (tower layers)](#7-intercepteurs-tower-layers)
8. [Authentification — JWT HS256 + scopes](#8-authentification--jwt-hs256--scopes)
9. [Rate limiting — token bucket par tenant](#9-rate-limiting--token-bucket-par-tenant)
10. [Streaming API — server-streaming tokens](#10-streaming-api--server-streaming-tokens)
11. [Admin API — gestion des cônes M9](#11-admin-api--gestion-des-cônes-m9)
12. [Graceful shutdown et backpressure](#12-graceful-shutdown-et-backpressure)
13. [SDK Python — `python/efe/`](#13-sdk-python--pythonefe)
14. [FFI PyO3 — `efe-ffi`](#14-ffi-pyo3--efe-ffi)
15. [Observabilité end-to-end](#15-observabilité-end-to-end)
16. [Modèle de sécurité et threat model](#16-modèle-de-sécurité-et-threat-model)
17. [Budget de performance et SLO](#17-budget-de-performance-et-slo)
18. [Stratégie de test](#18-stratégie-de-test)
19. [Dépendances inter-modules](#19-dépendances-inter-modules)
20. [Plan de sprint M10](#20-plan-de-sprint-m10)
21. [Plan de migration et rollout](#21-plan-de-migration-et-rollout)
22. [Matrice de risques et mitigations](#22-matrice-de-risques-et-mitigations)
23. [Critères d'acceptation](#23-critères-dacceptation)
24. [Annexes](#24-annexes)

---

## 1. Mission et positionnement

### 1.1 Mission fonctionnelle

M10 est la **surface publique** d'EFE. Il transforme le `SwarmController`
interne (M1) en un **service réseau gRPC versionné, authentifié,
rate-limité, observable et déployable** consommable par n'importe quel
client gRPC (Python, Go, Java, etc.), tout en livrant un **SDK Python de
première classe** qui masque la plomberie gRPC derrière une API idiomatique.

En une phrase : *M10 est le pont entre le monde externe (HTTP/2, JWT, JSON
payloads) et le cœur Rust (tenseurs bf16, Arc<dyn Trait>, tokio). Il garantit
qu'aucune implémentation interne ne fuite, qu'aucune requête non-autorisée
n'atteint le contrôleur, et qu'aucun rayon de breach n'arrive côté client
sans un span OpenTelemetry complet.*

### 1.2 Pourquoi M10 est critique

Sans M10, les blocages suivants persistent :

1. **Pas de déploiement possible.** Le contrôleur est seulement accessible
   depuis un binaire embarqué — impossible d'exposer EFE comme service.
2. **Pas de multi-tenant.** Sans identité dans la requête, impossible de
   segmenter les quotas ou les logs.
3. **Pas de SDK utilisateur.** Un client Python doit aujourd'hui compiler
   Rust → friction insurmontable en prod.
4. **Pas d'évolution protocolaire maîtrisée.** Chaque changement de type
   `SwarmSpec` casse silencieusement tout caller.
5. **Pas d'observabilité de bout en bout.** Sans interceptor OTel, les
   spans s'arrêtent à la frontière du processus.
6. **Pas de rate limiting.** Un tenant peut saturer le contrôleur et
   étouffer les autres.

M10 résout les six via un modèle **transport → middleware → service →
bridge → domain** bien stratifié, chaque couche testable en isolation.

### 1.3 Contraintes et garanties

| # | Contrainte | Justification |
|---|-----------|---------------|
| C1 | **gRPC unary + server-streaming** | Streaming indispensable pour les tokens LM ; unary suffit pour metadata / admin. |
| C2 | **Proto versioning strict** | `efe.v1` = contrat stable. Changements compatibles = ajout de champ optionnel. Break = `v2`. |
| C3 | **Send + Sync sur tous les handlers** | tonic impose la sync/clonabilité des services sur l'exécuteur tokio. |
| C4 | **Opt-in sur M9** | Le serveur peut tourner sans registre alignment (cône inline obligatoire dans spec). |
| C5 | **Auth bloquante** | JWT invalide = 401 AVANT tout accès au contrôleur. |
| C6 | **Rate limit par tenant, pas par IP** | Multi-tenant juste ; tolère les clients derrière NAT. |
| C7 | **Graceful shutdown** | `SIGTERM` draine les requêtes en vol avec un budget deadline, pas de kill sec. |
| C8 | **SDK Python = async-first** | Cohérent avec `asyncio` qui domine le Python moderne. Wrapper sync optionnel. |
| C9 | **Zéro copie sur le centroïde** | `Response.Raw(Vec<f32>)` est passé par value, mais les hot paths internes évitent toute clone dans le serveur. |
| C10 | **Échecs typés** | `EfeError` → `tonic::Status` via mapping déterministe (codes gRPC explicites), jamais `Status::internal(format!("{e}"))` opaque. |

### 1.4 Position dans le pipeline

```
          ┌──────────────────────────────────────────────────────┐
          │              Client externe (gRPC)                    │
          │   Python SDK │ Go gRPC │ grpcurl │ Envoy sidecar     │
          └──────────────────────┬───────────────────────────────┘
                                 │ HTTP/2 + TLS + JWT Bearer
                                 ▼
          ┌──────────────────────────────────────────────────────┐
          │              M10  —  efe-server                      │
          │                                                      │
          │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌─────────┐  │
          │  │ Tracing  │>│  Auth    │>│  Rate    │>│ Service │  │
          │  │ Layer    │ │Interceptor│ │  Limit   │ │ Handler │  │
          │  └──────────┘ └──────────┘ └──────────┘ └────┬────┘  │
          │                                              │        │
          │  ┌─────────────────────────────────────────┐ │        │
          │  │         Bridge (proto ↔ domain)          │<┘        │
          │  └───────────────────┬─────────────────────┘          │
          └──────────────────────┼───────────────────────────────┘
                                 │
                                 ▼  Request / Response
          ┌──────────────────────────────────────────────────────┐
          │              M1  —  SwarmController                  │
          └──────────────────────────────────────────────────────┘
```

### 1.5 Garanties offertes aux clients et aux modules avals

1. **Idempotence d'`Execute`** — une requête avec `request_id` identique
   répétée dans les 10 s retourne la même réponse (cache LRU 10 k entries).
   Pas un replay cryptographique — un optimiseur de retry client.
2. **Causalité OTel** — tout span client → serveur → M1 → M9 est chaîné
   via `traceparent`.
3. **Codes d'erreur gRPC stables** — le mapping `EfeError` →
   `Status::Code` est documenté (tableau §6.4) et figé pour `v1`.
4. **Quotas découplés** — l'épuisement du rate-limit émet `RESOURCE_EXHAUSTED`
   (code 8) avec `retry_after_ms` dans metadata, pas de silencieux 503.
5. **Zero PII dans les logs** — le `raw_prompt` n'est **jamais** loggué
   par défaut ; une feature-flag `--log-prompts` est requise (et déclenche
   un warning de démarrage).

---

## 2. Scope MVP vs hors-périmètre

### 2.1 IN — livré dans le MVP

| Fonctionnalité | Sprint | Justification |
|---|---|---|
| Proto `efe.v1` : `Execute`, `ExecuteStreaming`, `HealthCheck`, `DescribeSpec` | S7 | Surface minimum pour client + healthz. |
| Proto `efe.v1` admin : `RegisterCone`, `EvolveCone`, `GetCone`, `ListCones` | S8 | Expose M9 aux admins. |
| `efe-server` avec tonic + prost + tokio | S7 | Implémentation de référence. |
| TLS optionnel (server cert/key depuis fichier) | S7 | Prod-ready ; désactivable en dev. |
| JWT HS256 authentication | S8 | Plus simple que mTLS pour MVP ; rotation clé supportée. |
| Rate limiting token bucket par tenant | S8 | Protection multi-tenant. |
| Health check (`grpc.health.v1.Health`) | S7 | Kubernetes readiness / liveness. |
| Server reflection (tonic-reflection) | S7 | Debug avec grpcurl. |
| OpenTelemetry trace propagation | S8 | Continuité des traces client ↔ serveur. |
| Prometheus metrics endpoint (HTTP sidecar port) | S9 | `/metrics` scrapable. |
| Graceful shutdown SIGTERM | S7 | Drain in-flight avec deadline configurable. |
| SDK Python : `Client`, `AsyncClient`, `execute`, `execute_streaming`, `admin_*` | S9 | 100 % API surface couverte. |
| PyO3 FFI : `efe_ffi` expose `SwarmController` embarqué pour single-process | S9 | Use case : notebook dev, scripts batch. |
| Idempotence cache (10 k entries TTL 10 s) | S9 | Smooth retry clients. |
| Integration tests client Python ↔ serveur Rust | S9 | E2E coverage. |

### 2.2 OUT — explicitement hors MVP

| Fonctionnalité | Motif | Sprint cible post-MVP |
|---|---|---|
| mTLS (auth par cert client) | Complexité de PKI ; JWT HS256 suffit MVP. | P1 (S13). |
| OAuth 2.0 / OIDC | Requiert IdP externe ; JWT HS256 est un secret partagé. | P1. |
| Bidirectional streaming | Pas de use case MVP (client envoie 1 requête, reçoit N tokens). | P2. |
| Rate limiting distribué (Redis backend) | `in-memory` suffit pour single-node. | P1. |
| Quotas par API key avec tiers (free/pro/enterprise) | MVP = un seul tier. | P1. |
| SDK Node.js / Java / Go | Priorité Python ; gRPC reflection suffit pour les autres. | P2. |
| Chargement dynamique de nouveaux templates (M8) via API | Redémarrage serveur OK au MVP. | P2. |
| Circuit breaker par backend | Tolérance aux pannes M1 ; non bloquant. | P2. |
| Gzip compression | Économie BW marginale sur centroïdes bf16. | P2. |
| gRPC-Web (support navigateur) | Pas d'UI web MVP. | P3. |

### 2.3 Frontières claires avec M9 et M11

- **M10 appelle M9** pour l'admin API des cônes (register / evolve / list /
  breach_history). Il **n'implémente pas** la logique métier — simple
  pass-through avec traduction proto ↔ domain.
- **M10 n'appelle pas M9** sur le chemin `Execute` standard : le cône est
  fourni inline dans le proto `SwarmSpec` (ou résolu par `from_registry`
  côté client avant l'appel).
- **M10 n'émet pas de métriques métier** — c'est M11 qui instrumente.
  M10 instrumente uniquement les métriques *gRPC* (latence p50/p95/p99,
  error rate, RPS) via tower layers.
- **M10 propage** `traceparent` vers M1 (`Request.trace_context`) qui le
  transmet à M11 pour les spans internes.

---

## 3. Vue d'ensemble architecturale

### 3.1 Arborescence des fichiers

```
protos/efe/v1/
├── efe.proto                 # Service principal + types de requête
├── types.proto               # Types partagés (SwarmSpec, Response, ...)
├── admin.proto               # Service admin (cônes M9, breach history)
└── buf.yaml                  # Linting proto (buf)

crates/efe-server/
├── Cargo.toml                # + tonic, prost, tower, jsonwebtoken
├── build.rs                  # Invoque tonic-build sur les .proto
└── src/
    ├── main.rs               # entry point (tokio runtime, build services)
    ├── lib.rs                # re-exports publics pour tests intégration
    ├── config.rs             # ServerConfig, Config loading via figment
    ├── services/
    │   ├── mod.rs
    │   ├── swarm.rs          # impl SwarmService pour gRPC
    │   ├── admin.rs          # impl AdminService (cônes)
    │   └── health.rs         # impl grpc.health.v1.Health
    ├── bridge/
    │   ├── mod.rs
    │   ├── spec.rs           # proto SwarmSpec → domain::SwarmSpec
    │   ├── request.rs        # proto ExecuteRequest → domain::Request
    │   ├── response.rs       # domain::Response → proto ExecuteResponse
    │   ├── cone.rs           # proto Cone ↔ types::AlignmentCone
    │   └── error.rs          # EfeError → tonic::Status (mapping)
    ├── interceptors/
    │   ├── mod.rs
    │   ├── auth.rs           # AuthInterceptor (JWT)
    │   ├── ratelimit.rs      # RateLimitLayer (tower)
    │   ├── tracing.rs        # TracingLayer (OTel propagation)
    │   └── metrics.rs        # MetricsLayer (Prometheus)
    ├── auth/
    │   ├── mod.rs
    │   ├── jwt.rs            # JwtValidator (HS256)
    │   ├── token_store.rs    # TokenStore (rotation clés)
    │   └── scopes.rs         # Scope enum + required_scope_for
    ├── ratelimit/
    │   ├── mod.rs
    │   ├── bucket.rs         # TokenBucket impl
    │   └── registry.rs       # BucketRegistry (per-tenant)
    ├── idempotency/
    │   ├── mod.rs
    │   └── cache.rs          # LRU cache (10k × 10s TTL)
    ├── shutdown.rs           # Graceful shutdown (SIGTERM handler)
    └── runtime.rs            # Server builder + bootstrap

crates/efe-ffi/
├── Cargo.toml                # + pyo3, pyo3-asyncio
└── src/
    ├── lib.rs                # #[pymodule]
    ├── py_controller.rs      # PyClass wrapping SwarmController
    ├── py_request.rs         # PyRequest / PyResponse bindings
    ├── py_spec.rs            # PySwarmSpec builder
    └── conversions.rs        # bf16 ↔ numpy, Vec ↔ list, etc.

python/efe/
├── __init__.py               # re-exports publics
├── client.py                 # AsyncClient + SyncClient
├── auth.py                   # JwtTokenProvider
├── errors.py                 # ClientError, ServerError hierarchy
├── proto/                    # généré par buf generate (fichiers _pb2.py, _pb2_grpc.py)
├── streaming.py              # StreamingResponse iterator
├── types.py                  # SwarmSpec dataclass, DecodeMode enum
└── _ffi.py                   # chargement du module efe_ffi PyO3

tests/integration/
├── test_grpc_unary.py        # Python client ↔ Rust server (unary)
├── test_grpc_streaming.py    # streaming
├── test_auth.py              # JWT valid / invalid / expired
├── test_ratelimit.py         # burst / throttle
└── test_admin.py             # cone lifecycle via API
```

### 3.2 Graphe de dépendances interne

```
                        ┌─────────────────┐
                        │   main.rs       │
                        └────────┬────────┘
                                 │
                        ┌────────▼────────┐
                        │  runtime.rs     │ (builder)
                        └─┬──┬──┬──┬──┬───┘
                          │  │  │  │  │
                          ▼  ▼  ▼  ▼  ▼
      ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
      │ config   │ │services/*│ │intercept │ │ auth/    │ │ratelimit/│
      │          │ │          │ │ors/*     │ │          │ │          │
      └──────────┘ └─┬────────┘ └──────────┘ └──────────┘ └──────────┘
                     │
                     ▼
                ┌──────────┐
                │bridge/*  │ ── dépend de efe-core types
                └──────────┘
                     │
                     ▼
                 [efe-core]
```

### 3.3 Exécution type d'une requête `Execute`

```
1. Client Python : client.execute(spec, prompt, token="eyJ...")
2. grpc.aio Channel → POST /efe.v1.SwarmService/Execute
3. Serveur : tokio::spawn pour la requête.
4. TracingLayer : extract traceparent → span.enter().
5. AuthInterceptor : validate JWT → inject tenant_id + scopes dans extensions.
6. RateLimitLayer : token_bucket[tenant_id].try_acquire(1) ou 429.
7. MetricsLayer : histogram start, counter.inc("efe_requests_total").
8. SwarmService::execute :
   a. bridge::request::from_proto() → domain::Request
   b. idempotency cache check → si hit, return cached Response
   c. self.controller.execute(req).await → domain::Response
   d. bridge::response::to_proto() → ExecuteResponse
   e. cache.insert(req_id, resp) si configuré
9. Error mapping (si Err) : bridge::error::to_status() → tonic::Status
10. MetricsLayer : histogram end.
11. Response sérialisée HTTP/2 → client.
12. Client : parse response, return typed dataclass.
```

---

## 4. Descripteur proto : `efe.v1`

### 4.1 Structure de package

```
efe.v1          # surface publique
efe.v1.admin    # admin API (séparé pour scope JWT distinct)
```

### 4.2 `protos/efe/v1/efe.proto`

```protobuf
syntax = "proto3";
package efe.v1;

import "google/protobuf/duration.proto";
import "google/protobuf/empty.proto";
import "google/protobuf/timestamp.proto";
import "efe/v1/types.proto";

service SwarmService {
  // Unary inference: submit a spec, get a single response.
  rpc Execute(ExecuteRequest) returns (ExecuteResponse);

  // Server-streaming inference: receive tokens as they are decoded.
  rpc ExecuteStreaming(ExecuteRequest) returns (stream StreamChunk);

  // Describe a spec's shape without running inference.
  // Useful for clients validating their payload before submission.
  rpc DescribeSpec(DescribeSpecRequest) returns (DescribeSpecResponse);
}

message ExecuteRequest {
  // Correlation id. Client-supplied; if empty, server generates one.
  string request_id = 1;
  // Raw user prompt (optional; audit only).
  string raw_prompt = 2;
  // Full swarm specification.
  SwarmSpec spec = 3;
  // Decoding mode.
  DecodeMode decode_mode = 4;
  // Optional absolute deadline for the inference.
  google.protobuf.Timestamp deadline = 5;
}

message ExecuteResponse {
  string swarm_id = 1;
  string request_id = 2;
  DecodedOutput output = 3;
  float confidence = 4;
  Verdict verdict = 5;
  uint32 rounds_executed = 6;
  float total_ms = 7;
  float alignment_delta = 8;
}

// Streaming chunk sent as the LM head produces tokens.
message StreamChunk {
  oneof payload {
    // A slice of text tokens.
    string text_delta = 1;
    // Sent once at the end with full metadata.
    ExecuteResponse final = 2;
    // Non-fatal warning (e.g., approaching budget, soft breach).
    ServerEvent event = 3;
  }
}

message DescribeSpecRequest {
  SwarmSpec spec = 1;
}

message DescribeSpecResponse {
  // Whether the spec is admissible (validate() passes).
  bool valid = 1;
  // Human-readable description of errors (if invalid).
  repeated string errors = 2;
  // Estimated HBM bytes used at peak.
  uint64 hbm_bytes_estimate = 3;
  // Estimated rounds with current config.
  uint32 rounds_expected = 4;
}

message ServerEvent {
  string kind = 1;     // "soft_breach", "budget_80pct", ...
  string message = 2;
  map<string, string> labels = 3;
}
```

### 4.3 `protos/efe/v1/types.proto`

```protobuf
syntax = "proto3";
package efe.v1;

// Complete swarm specification matching efe_core::types::SwarmSpec.
message SwarmSpec {
  // 32-bit or 64-bit identifier. Server overrides if empty.
  string swarm_id = 1;
  // Problem embedding (length must equal dim_latent).
  repeated float problem_embedding = 2;
  uint32 dim_latent = 3;
  // Population mix: repeated (template_code, count) entries.
  repeated PopulationEntry population = 4;
  uint32 max_rounds = 5;
  ConvergenceThresholds convergence = 6;
  // Either an inline cone or a registry reference.
  oneof cone_source {
    AlignmentCone alignment_cone = 7;
    ConeRef alignment_cone_ref = 8;
  }
  ResourceBudget budget = 9;
}

message PopulationEntry {
  string template_code = 1;  // "na-sem", "na-log", "na-crit"
  uint32 count = 2;
}

message ConvergenceThresholds {
  float variance_target = 1;
  float min_improvement = 2;
  uint32 patience = 3;
}

message AlignmentCone {
  repeated Anchor anchors = 1;
  float max_projection_distance = 2;
}

message Anchor {
  repeated float values = 1;
}

message ConeRef {
  string tenant = 1;
  string task = 2;
  uint32 version = 3;
}

message ResourceBudget {
  uint64 hbm_bytes_cap = 1;
  float energy_joules_cap = 2;
  google.protobuf.Duration wall_time_cap = 3;
}

message DecodeMode {
  oneof mode {
    TextMode text = 1;
    PassThroughMode pass_through = 2;
  }
}

message TextMode {
  uint32 max_tokens = 1;
}

message PassThroughMode {}

message DecodedOutput {
  oneof payload {
    string text = 1;
    RawCentroid raw = 2;
  }
}

message RawCentroid {
  repeated float values = 1;
}

enum Verdict {
  VERDICT_UNSPECIFIED = 0;
  VERDICT_CONTINUE = 1;
  VERDICT_CONVERGED = 2;
  VERDICT_DIVERGED = 3;
  VERDICT_ANOMALY = 4;
  VERDICT_STALLED = 5;
}
```

### 4.4 `protos/efe/v1/admin.proto`

```protobuf
syntax = "proto3";
package efe.v1.admin;

import "google/protobuf/empty.proto";
import "google/protobuf/timestamp.proto";
import "efe/v1/types.proto";

service AdminService {
  rpc RegisterCone(RegisterConeRequest) returns (ConeHandle);
  rpc EvolveCone(EvolveConeRequest) returns (ConeHandle);
  rpc GetCone(GetConeRequest) returns (CanonicalCone);
  rpc ListVersions(ListVersionsRequest) returns (ListVersionsResponse);
  rpc DeleteCone(DeleteConeRequest) returns (google.protobuf.Empty);
  rpc BreachHistory(BreachHistoryRequest) returns (BreachHistoryResponse);
}

message RegisterConeRequest {
  string tenant = 1;
  string task = 2;
  uint32 version = 3;
  efe.v1.AlignmentCone cone = 4;
  string created_by = 5;
  string description = 6;
  map<string, string> labels = 7;
}

message ConeHandle {
  string key = 1;  // "tenant/task@vN"
  CanonicalCone cone = 2;
}

message CanonicalCone {
  efe.v1.AlignmentCone cone = 1;
  ConeMetadata metadata = 2;
}

message ConeMetadata {
  google.protobuf.Timestamp created_at = 1;
  string created_by = 2;
  string description = 3;
  string parent_key = 4;   // empty if root
  string evolution_op = 5; // empty if root
  map<string, string> labels = 6;
}

message EvolveConeRequest {
  string from_key = 1;  // "tenant/task@vN"
  ConeEvolutionOp op = 2;
  // Optional override; if unset, parent labels/creator are preserved.
  ConeMetadata metadata_override = 3;
}

message ConeEvolutionOp {
  oneof op {
    efe.v1.Anchor append_anchor = 1;
    uint32 remove_anchor_idx = 2;
    ReplaceAnchorOp replace_anchor = 3;
    float update_radius = 4;
    float scale_anchors = 5;
    string union_with_key = 6;  // "tenant/task@vN"
  }
}

message ReplaceAnchorOp {
  uint32 idx = 1;
  efe.v1.Anchor anchor = 2;
}

message GetConeRequest {
  string key = 1;
}

message ListVersionsRequest {
  string tenant = 1;
  string task = 2;
}

message ListVersionsResponse {
  repeated string keys = 1;  // sorted ascending
}

message DeleteConeRequest {
  string key = 1;
  bool force = 2;
}

message BreachHistoryRequest {
  uint32 limit = 1;           // max entries returned
  string tenant = 2;          // optional filter
  string task = 3;            // optional filter
  float min_magnitude = 4;    // optional
}

message BreachHistoryResponse {
  repeated BreachReport reports = 1;
}

message BreachReport {
  string cone_key = 1;
  string swarm_id = 2;
  float observed_delta = 3;
  float cone_radius = 4;
  google.protobuf.Timestamp timestamp = 5;
  string context = 6;         // display form of BreachContext
}
```

### 4.5 Règles de versioning

| Changement | Compatible ? | Action requise |
|---|---|---|
| Ajout d'un champ optionnel dans un message | ✅ | aucune |
| Ajout d'une méthode RPC dans un service | ✅ | aucune |
| Ajout d'un variant dans une `oneof` | ⚠️ | les clients v1 voient `Unknown` — documenter |
| Renommage d'un champ (même tag) | ⚠️ | compat binaire ok, source incompat — bump mineur |
| Suppression d'un champ | ❌ | bump majeur (`v2`) |
| Changement du type d'un champ | ❌ | bump majeur |
| Ajout d'un enum value | ✅ | documenter — clients v1 reçoivent `UNSPECIFIED` |

Linting via `buf lint` en CI ; `buf breaking` contre le dernier release tag.

---

## 5. Couches du serveur Rust `efe-server`

### 5.1 Pipeline tower

```
                 tonic::transport::Server
                          │
                          ▼
              ┌─────────────────────────┐
              │  TracingLayer (OTel)    │
              └─────────┬───────────────┘
                        │
                        ▼
              ┌─────────────────────────┐
              │  MetricsLayer (Prom)    │
              └─────────┬───────────────┘
                        │
                        ▼
              ┌─────────────────────────┐
              │  AuthInterceptor (JWT)  │
              └─────────┬───────────────┘
                        │
                        ▼
              ┌─────────────────────────┐
              │  RateLimitLayer         │
              └─────────┬───────────────┘
                        │
                        ▼
              ┌─────────────────────────┐
              │  Concurrency limit      │  (tower::limit)
              └─────────┬───────────────┘
                        │
                        ▼
              ┌─────────────────────────┐
              │  Timeout layer          │  (per-call default 30s)
              └─────────┬───────────────┘
                        │
                        ▼
              ┌─────────────────────────┐
              │  SwarmService handler   │
              │  ├─ idempotency cache   │
              │  └─ bridge + controller │
              └─────────────────────────┘
```

### 5.2 `runtime.rs` — builder

```rust
pub struct Server {
    config: ServerConfig,
    controller: Arc<dyn SwarmController>,
    registry: Option<Arc<dyn AlignmentRegistry>>,
    token_store: Arc<TokenStore>,
    rate_limiter: Arc<BucketRegistry>,
    idempotency: Arc<IdempotencyCache>,
    metrics_handle: PrometheusHandle,
    otel_tracer: Arc<dyn Tracer>,
}

impl Server {
    pub fn builder() -> ServerBuilder { ... }

    pub async fn serve(self, addr: SocketAddr, shutdown: Shutdown)
        -> anyhow::Result<()>
    {
        let swarm_svc = SwarmServiceImpl::new(
            self.controller.clone(),
            self.idempotency.clone(),
        );
        let admin_svc = self.registry.as_ref().map(|r| {
            AdminServiceImpl::new(r.clone())
        });
        let health_svc = HealthServiceImpl::new();

        let reflection = tonic_reflection::server::Builder::configure()
            .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
            .build_v1alpha()?;

        let mut server = tonic::transport::Server::builder()
            .layer(TracingLayer::new(self.otel_tracer.clone()))
            .layer(MetricsLayer::new(self.metrics_handle.clone()))
            .layer(AuthLayer::new(self.token_store.clone()))
            .layer(RateLimitLayer::new(self.rate_limiter.clone()))
            .concurrency_limit_per_connection(
                self.config.max_concurrent_streams)
            .timeout(self.config.request_timeout);

        server = server.add_service(
            SwarmServiceServer::new(swarm_svc));
        if let Some(admin) = admin_svc {
            server = server.add_service(
                AdminServiceServer::new(admin));
        }
        server = server
            .add_service(health_svc)
            .add_service(reflection);

        server.serve_with_shutdown(addr, shutdown.signal()).await?;
        Ok(())
    }
}
```

### 5.3 `services/swarm.rs` — handler principal

```rust
pub struct SwarmServiceImpl {
    controller: Arc<dyn SwarmController>,
    idempotency: Arc<IdempotencyCache>,
}

#[async_trait::async_trait]
impl SwarmService for SwarmServiceImpl {
    async fn execute(
        &self,
        req: Request<ExecuteRequest>,
    ) -> Result<Response<ExecuteResponse>, Status> {
        let span = tracing::Span::current();
        let tenant = extract_tenant(&req)?;
        let inner = req.into_inner();
        let req_id = ensure_request_id(&inner);

        // Idempotency : short-circuit if recent identical req_id.
        if let Some(cached) = self.idempotency.get(&req_id).await {
            span.record("cache_hit", true);
            return Ok(Response::new(cached));
        }

        // Bridge proto → domain (ne propage PAS EfeError directement).
        let domain_req = bridge::request::from_proto(inner, tenant)
            .map_err(bridge::error::to_status)?;

        // Exécute.
        let domain_resp = self.controller.execute(domain_req).await
            .map_err(bridge::error::to_status)?;

        // Bridge domain → proto.
        let proto_resp = bridge::response::to_proto(domain_resp);

        // Cache store.
        self.idempotency.insert(req_id, proto_resp.clone()).await;

        Ok(Response::new(proto_resp))
    }

    type ExecuteStreamingStream = ReceiverStream<Result<StreamChunk, Status>>;

    async fn execute_streaming(
        &self,
        req: Request<ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStreamingStream>, Status> {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let ctrl = self.controller.clone();
        tokio::spawn(async move {
            // Streaming logic — see §10.
            run_streaming(ctrl, req, tx).await;
        });
        Ok(Response::new(ReceiverStream::new(rx)))
    }

    async fn describe_spec(&self, req: Request<DescribeSpecRequest>)
        -> Result<Response<DescribeSpecResponse>, Status>
    {
        let spec_proto = req.into_inner().spec
            .ok_or_else(|| Status::invalid_argument("spec is required"))?;
        match bridge::spec::from_proto(spec_proto) {
            Ok(spec) => {
                let hbm = estimate_hbm(&spec);
                let rounds = spec.max_rounds.get();
                Ok(Response::new(DescribeSpecResponse {
                    valid: true, errors: vec![],
                    hbm_bytes_estimate: hbm,
                    rounds_expected: rounds,
                }))
            }
            Err(e) => Ok(Response::new(DescribeSpecResponse {
                valid: false,
                errors: vec![e.to_string()],
                hbm_bytes_estimate: 0,
                rounds_expected: 0,
            })),
        }
    }
}
```

### 5.4 `services/admin.rs` — handler pour M9

```rust
pub struct AdminServiceImpl {
    registry: Arc<dyn AlignmentRegistry>,
}

#[async_trait::async_trait]
impl AdminService for AdminServiceImpl {
    async fn register_cone(&self, req: Request<RegisterConeRequest>)
        -> Result<Response<ConeHandle>, Status>
    {
        let inner = req.into_inner();
        let key = ConeKey::new(&inner.tenant, &inner.task, inner.version)
            .map_err(bridge::error::to_status)?;
        let cone = bridge::cone::from_proto(inner.cone
            .ok_or_else(|| Status::invalid_argument("cone required"))?)
            .map_err(bridge::error::to_status)?;
        let mut md = ConeMetadata::new(inner.created_by, inner.description)
            .map_err(bridge::error::to_status)?;
        for (k, v) in inner.labels {
            md = md.with_label(k, v).map_err(bridge::error::to_status)?;
        }
        let canonical = CanonicalCone::with_metadata(cone, md);
        let handle = self.registry.register(key, canonical).await
            .map_err(bridge::error::to_status)?;
        Ok(Response::new(bridge::cone::handle_to_proto(handle)))
    }

    async fn evolve_cone(&self, req: Request<EvolveConeRequest>)
        -> Result<Response<ConeHandle>, Status> { ... }
    async fn get_cone(&self, req: Request<GetConeRequest>)
        -> Result<Response<CanonicalCone>, Status> { ... }
    async fn list_versions(&self, req: Request<ListVersionsRequest>)
        -> Result<Response<ListVersionsResponse>, Status> { ... }
    async fn delete_cone(&self, req: Request<DeleteConeRequest>)
        -> Result<Response<Empty>, Status> { ... }
    async fn breach_history(&self, req: Request<BreachHistoryRequest>)
        -> Result<Response<BreachHistoryResponse>, Status> { ... }
}
```

### 5.5 `services/health.rs`

Implémente `grpc.health.v1.Health` avec l'état dérivé de :
- `SERVING` si controller + registry sont `Healthy`
- `NOT_SERVING` pendant `shutdown.is_triggered()`
- `UNKNOWN` au démarrage avant bootstrap complet

---

## 6. Mapping proto ↔ domain (M1) : règles exactes

### 6.1 `bridge::spec`

Construit un `SwarmSpec` depuis un `proto::SwarmSpec`.

```rust
pub fn from_proto(proto: ProtoSwarmSpec) -> Result<SwarmSpec> {
    let mut builder = SwarmSpec::builder()
        .problem_embedding(proto.problem_embedding)
        .dim_latent(LatentDim::new(proto.dim_latent)?);

    let entries: Vec<(TemplateCode, u32)> = proto.population.into_iter()
        .map(|e| {
            let code = TemplateCode::from_str(&e.template_code)
                .ok_or_else(|| EfeError::InvalidConfig(format!(
                    "unknown template_code {}", e.template_code)))?;
            Ok((code, e.count))
        })
        .collect::<Result<_>>()?;
    builder = builder.population(PopulationMix::new(entries)?);

    if proto.max_rounds > 0 {
        builder = builder.max_rounds(proto.max_rounds)?;
    }
    if let Some(c) = proto.convergence {
        builder = builder.convergence(ConvergenceThresholds {
            variance_target: c.variance_target,
            min_improvement: c.min_improvement,
            patience: c.patience,
        });
    }
    let cone = match proto.cone_source {
        Some(proto::swarm_spec::ConeSource::AlignmentCone(c)) => {
            crate::bridge::cone::from_proto(c)?
        }
        Some(proto::swarm_spec::ConeSource::AlignmentConeRef(_)) => {
            return Err(EfeError::InvalidConfig(
                "server-side cone resolution from registry not yet in unary path; use client-side from_registry".into(),
            ));
        }
        None => return Err(EfeError::InvalidConfig(
            "cone_source required".into())),
    };
    builder = builder.alignment_cone(cone);
    if let Some(b) = proto.budget {
        builder = builder.budget(bridge::budget::from_proto(b)?);
    }
    builder.build()
}
```

### 6.2 `bridge::request`

```rust
pub fn from_proto(proto: ProtoExecuteRequest, tenant_id: String)
    -> Result<Request>
{
    let spec = crate::bridge::spec::from_proto(proto.spec
        .ok_or_else(|| EfeError::InvalidConfig("spec required".into()))?)?;
    let decode_mode = crate::bridge::decode::from_proto(proto.decode_mode)?;
    let deadline = proto.deadline.map(|ts| {
        let t = SystemTime::UNIX_EPOCH
            + Duration::new(ts.seconds as u64, ts.nanos as u32);
        Instant::now() + t.duration_since(SystemTime::now()).unwrap_or_default()
    });
    let trace_context = /* extracted from metadata in interceptor */ None;
    Ok(Request {
        id: if proto.request_id.is_empty() {
            RequestId::new()
        } else {
            RequestId::parse(&proto.request_id)?
        },
        tenant_id,
        spec,
        decode_mode,
        raw_prompt: (!proto.raw_prompt.is_empty()).then_some(proto.raw_prompt),
        deadline,
        trace_context,
    })
}
```

### 6.3 `bridge::response`

```rust
pub fn to_proto(resp: Response) -> ExecuteResponse {
    ExecuteResponse {
        swarm_id: resp.swarm_id.as_uuid().to_string(),
        request_id: resp.request_id.as_str().to_string(),
        output: Some(bridge::decode::output_to_proto(resp.output)),
        confidence: resp.confidence,
        verdict: bridge::verdict::to_proto(resp.verdict) as i32,
        rounds_executed: resp.rounds_executed,
        total_ms: resp.total_ms,
        alignment_delta: resp.alignment_delta,
    }
}
```

### 6.4 `bridge::error` — mapping `EfeError` → `tonic::Status`

| Variant `EfeError` | gRPC Code | Message client |
|---|---|---|
| `InvalidConfig(s)` | `INVALID_ARGUMENT` (3) | `s` |
| `AlignmentBreach { delta, max }` | `FAILED_PRECONDITION` (9) | formatted |
| `Internal(s)` | `INTERNAL` (13) | `"internal error"` (ne pas fuiter le détail) |
| `HbmAllocFailed { requested }` | `RESOURCE_EXHAUSTED` (8) | formatted |
| `ModeCollapse { round }` | `FAILED_PRECONDITION` (9) | formatted |
| `NumericalExplosion { round }` | `INTERNAL` (13) | `"numerical explosion"` |
| `Divergence { .. }` | `FAILED_PRECONDITION` (9) | formatted |
| `ConvergenceTimeout { .. }` | `DEADLINE_EXCEEDED` (4) | formatted |
| `BudgetExhausted { kind }` | `RESOURCE_EXHAUSTED` (8) | `"budget exhausted: {kind}"` |

Implémentation :

```rust
pub fn to_status(e: EfeError) -> tonic::Status {
    use tonic::Code;
    let (code, msg) = match e {
        EfeError::InvalidConfig(s) => (Code::InvalidArgument, s),
        EfeError::AlignmentBreach { delta, max } => (
            Code::FailedPrecondition,
            format!("alignment breach: delta={delta:.6} > max={max:.6}"),
        ),
        EfeError::Internal(_) => (
            Code::Internal,
            "internal error".to_string(), // <- NEVER leak the static msg
        ),
        EfeError::HbmAllocFailed { requested } => (
            Code::ResourceExhausted,
            format!("HBM allocation failed: {requested} bytes"),
        ),
        EfeError::ModeCollapse { round } => (
            Code::FailedPrecondition,
            format!("mode collapse at round {round}"),
        ),
        EfeError::NumericalExplosion { round: _ } => (
            Code::Internal,
            "numerical explosion".to_string(),
        ),
        EfeError::Divergence { rounds, reason } => (
            Code::FailedPrecondition,
            format!("divergence after {rounds} rounds: {reason}"),
        ),
        EfeError::ConvergenceTimeout { rounds, sigma2 } => (
            Code::DeadlineExceeded,
            format!("timeout after {rounds} rounds, sigma2={sigma2}"),
        ),
        EfeError::BudgetExhausted { kind } => (
            Code::ResourceExhausted,
            format!("budget exhausted: {kind}"),
        ),
        // Fallback for non_exhaustive extension.
        _ => (Code::Unknown, "unknown EfeError variant".to_string()),
    };
    tonic::Status::new(code, msg)
}
```

**Règle inviolable** : `EfeError::Internal(&'static str)` ne fuite **jamais**
le message static dans le corps de `Status` — seulement logged côté serveur.
Évite l'exposition de code paths internes à un attaquant.

### 6.5 Verdict mapping

```rust
pub fn to_proto(v: Verdict) -> proto::Verdict {
    match v {
        Verdict::Continue => proto::Verdict::Continue,
        Verdict::Converged { .. } => proto::Verdict::Converged,
        Verdict::Diverged { .. } => proto::Verdict::Diverged,
        Verdict::Anomaly { .. } => proto::Verdict::Anomaly,
        Verdict::Stalled { .. } => proto::Verdict::Stalled,
    }
}
```

Les détails internes (raison, round, etc.) sont émis via logs / spans,
pas dans le proto (stabilité).

---

## 7. Intercepteurs (tower layers)

### 7.1 `TracingLayer`

Objectif : extraire `traceparent` depuis gRPC metadata, ouvrir un span
lié, le stocker dans les extensions de la requête pour que le handler
puisse le propager à `SwarmController`.

```rust
impl<S> Service<Request<Body>> for TracingMiddleware<S>
where S: Service<Request<Body>, Response = Response<BoxBody>>, ...
{
    fn call(&mut self, mut req: Request<Body>) -> Self::Future {
        let traceparent = req.headers().get("traceparent")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let span = tracing::info_span!(
            "grpc.request",
            method = %req.uri().path(),
            traceparent = ?traceparent,
        );
        if let Some(tp) = traceparent {
            req.extensions_mut().insert(TraceContext {
                traceparent: tp,
                tracestate: None,
            });
        }
        let fut = self.inner.call(req);
        async move { fut.instrument(span).await }.boxed()
    }
}
```

### 7.2 `MetricsLayer`

Émet les métriques gRPC standard :
- `efe_grpc_requests_total{method, code}` (counter)
- `efe_grpc_request_duration_seconds{method}` (histogram)
- `efe_grpc_requests_in_flight{method}` (gauge)

### 7.3 `AuthInterceptor`

Voir §8.

### 7.4 `RateLimitLayer`

Voir §9.

---

## 8. Authentification — JWT HS256 + scopes

### 8.1 Format du token

```json
{
  "alg": "HS256",
  "typ": "JWT"
}
.
{
  "iss": "efe.internal",
  "sub": "service-acct-123",
  "aud": "efe.v1",
  "exp": 1714500000,
  "iat": 1714493000,
  "tenant_id": "acme",
  "scopes": ["swarm:execute", "admin:cone:read"]
}
```

### 8.2 `JwtValidator`

```rust
pub struct JwtValidator {
    keys: Arc<RwLock<Vec<DecodingKey>>>,
    validation: Validation,
}

impl JwtValidator {
    pub fn new(secrets: Vec<Vec<u8>>) -> Self { ... }

    pub fn rotate_keys(&self, new: Vec<Vec<u8>>) { ... }

    pub fn validate(&self, token: &str) -> Result<Claims> {
        let keys = self.keys.read().unwrap();
        for key in keys.iter() {
            match decode::<Claims>(token, key, &self.validation) {
                Ok(data) => return Ok(data.claims),
                Err(_) => continue,
            }
        }
        Err(EfeError::InvalidConfig("invalid token".into()))
    }
}
```

### 8.3 Scope enforcement

Chaque RPC déclare son scope requis :

| RPC | Scope required |
|---|---|
| `SwarmService.Execute` | `swarm:execute` |
| `SwarmService.ExecuteStreaming` | `swarm:execute` |
| `SwarmService.DescribeSpec` | `swarm:describe` |
| `AdminService.RegisterCone` | `admin:cone:write` |
| `AdminService.EvolveCone` | `admin:cone:write` |
| `AdminService.GetCone` | `admin:cone:read` |
| `AdminService.ListVersions` | `admin:cone:read` |
| `AdminService.DeleteCone` | `admin:cone:delete` |
| `AdminService.BreachHistory` | `admin:breach:read` |
| `Health.Check` | — (public) |

Implémentation dans `AuthLayer` :

```rust
fn required_scope(path: &str) -> Option<&'static str> {
    match path {
        "/efe.v1.SwarmService/Execute" => Some("swarm:execute"),
        "/efe.v1.SwarmService/ExecuteStreaming" => Some("swarm:execute"),
        "/efe.v1.SwarmService/DescribeSpec" => Some("swarm:describe"),
        "/efe.v1.admin.AdminService/RegisterCone" => Some("admin:cone:write"),
        /* ... */
        "/grpc.health.v1.Health/Check" => None,
        _ => Some("__denied__"),  // default deny
    }
}

async fn authenticate(&self, req: &Request<Body>) -> Result<Claims, Status> {
    let scope_needed = required_scope(req.uri().path());
    if scope_needed.is_none() { return Ok(anonymous_claims()); }

    let token = extract_bearer(req.headers())
        .ok_or_else(|| Status::unauthenticated("missing bearer token"))?;
    let claims = self.validator.validate(&token)
        .map_err(|_| Status::unauthenticated("invalid token"))?;

    let needed = scope_needed.unwrap();
    if !claims.scopes.iter().any(|s| s == needed) {
        return Err(Status::permission_denied(format!(
            "missing scope: {needed}"
        )));
    }
    if claims.aud != "efe.v1" {
        return Err(Status::unauthenticated("wrong audience"));
    }
    Ok(claims)
}
```

### 8.4 Rotation de clé

Le `TokenStore` accepte jusqu'à 2 clés actives simultanément (current + previous)
pour supporter une rotation zéro-downtime :

1. Admin pousse nouvelle clé via signal SIGHUP ou config reload.
2. `rotate_keys([new, current])` — les deux valident.
3. Après 2×TTL (2×15min = 30min), `rotate_keys([new])` retire la vieille.

### 8.5 Échecs d'auth — codes précis

| Cas | Status code |
|---|---|
| Header `authorization` absent | `UNAUTHENTICATED` (16) |
| Header mal formé (pas `Bearer xxx`) | `UNAUTHENTICATED` |
| JWT signature invalide | `UNAUTHENTICATED` |
| JWT expiré | `UNAUTHENTICATED` + detail `{"reason":"expired"}` |
| Audience incorrecte | `UNAUTHENTICATED` |
| Scope manquant | `PERMISSION_DENIED` (7) |

---

## 9. Rate limiting — token bucket par tenant

### 9.1 `TokenBucket`

```rust
pub struct TokenBucket {
    capacity: u32,
    refill_rate_per_sec: f32,
    state: Mutex<BucketState>,
}

struct BucketState {
    tokens: f32,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn try_acquire(&self, n: u32) -> Result<(), TryAcquireError> {
        let mut st = self.state.lock().unwrap();
        let now = Instant::now();
        let elapsed = now.duration_since(st.last_refill).as_secs_f32();
        st.tokens = (st.tokens + elapsed * self.refill_rate_per_sec)
            .min(self.capacity as f32);
        st.last_refill = now;
        if st.tokens >= n as f32 {
            st.tokens -= n as f32;
            Ok(())
        } else {
            let missing = n as f32 - st.tokens;
            let retry_after_ms =
                (missing / self.refill_rate_per_sec * 1000.0) as u64;
            Err(TryAcquireError { retry_after_ms })
        }
    }
}
```

### 9.2 `BucketRegistry`

```rust
pub struct BucketRegistry {
    buckets: DashMap<String, Arc<TokenBucket>>,
    default_cfg: BucketConfig,
}

impl BucketRegistry {
    pub fn get_or_create(&self, tenant: &str) -> Arc<TokenBucket> {
        self.buckets
            .entry(tenant.to_string())
            .or_insert_with(|| Arc::new(TokenBucket::new(&self.default_cfg)))
            .clone()
    }
}
```

`dashmap` pour concurrent insert sans contention globale.

### 9.3 `RateLimitLayer`

```rust
impl<S> Service<Request<Body>> for RateLimitMiddleware<S>
where S: Service<Request<Body>, Response = Response<BoxBody>>, ...
{
    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let tenant = req.extensions().get::<Claims>()
            .map(|c| c.tenant_id.clone())
            .unwrap_or_else(|| "anonymous".into());
        let bucket = self.registry.get_or_create(&tenant);
        match bucket.try_acquire(1) {
            Ok(()) => {
                let fut = self.inner.call(req);
                Box::pin(fut)
            }
            Err(e) => {
                let mut status = Status::resource_exhausted(
                    format!("rate limit: retry after {}ms", e.retry_after_ms));
                status.metadata_mut().insert(
                    "retry-after-ms",
                    e.retry_after_ms.to_string().parse().unwrap(),
                );
                Box::pin(async move { Ok(status.to_http()) })
            }
        }
    }
}
```

### 9.4 Configuration

```toml
[api.ratelimit]
default_capacity = 200       # bucket size
default_refill_rate = 100    # tokens/second (= 100 req/s soutenu)
# Overrides per-tenant :
overrides = { "acme" = { capacity = 1000, refill_rate = 500 } }
```

### 9.5 Limites
- `in-memory` seulement → multi-instance nécessite sticky routing ou
  migration vers Redis (post-MVP).
- L'anonymous bucket existe pour les endpoints publics (Health) afin
  d'éviter un flood.

---

## 10. Streaming API — server-streaming tokens

### 10.1 Flux

```
Client           Server                M1 Controller       Dissolver
  │                │                         │                   │
  │──Execute──────▶│                         │                   │
  │◀STREAM BEGIN──│                         │                   │
  │                │──execute_streaming─────▶│                   │
  │                │                         │──dissolve─────────▶│
  │                │                         │                   │
  │                │                         │            token 1 │
  │◀ StreamChunk──│◀── token_delta ─────────│◀──────────────────│
  │  text_delta    │                         │                   │
  │                │                         │            token 2 │
  │◀ StreamChunk──│◀── token_delta ─────────│◀──────────────────│
  │                │                         │         ...       │
  │                │                         │       (final)     │
  │◀ StreamChunk──│◀── final(resp) ─────────│                   │
  │  final         │                         │                   │
  │                │                         │                   │
  │ STREAM END    │                          │                   │
```

### 10.2 Contract

Le serveur envoie :
- N × `StreamChunk { text_delta: "..." }` pendant la décodage
- Optionnellement 1 × `StreamChunk { event: ServerEvent { kind: "soft_breach", ... } }`
- 1 × `StreamChunk { final: ExecuteResponse { ... } }` à la fin

Puis termine le stream avec `OK` (code 0) ou `Status::error(...)` si échec.

### 10.3 Implémentation

Le `Dissolver` expose un hook `decode_streaming` :

```rust
pub trait StreamingDissolver: Dissolver {
    async fn dissolve_streaming(
        &self,
        state: &SwarmState,
        tx: mpsc::Sender<TextToken>,
    ) -> Result<Response>;
}
```

Le serveur wrappe :

```rust
async fn run_streaming(
    ctrl: Arc<dyn SwarmController>,
    req: Request<ExecuteRequest>,
    tx: mpsc::Sender<Result<StreamChunk, Status>>,
) {
    let tenant = extract_tenant(&req);
    let inner = req.into_inner();
    let domain_req = match bridge::request::from_proto(inner, tenant) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Err(bridge::error::to_status(e))).await;
            return;
        }
    };
    let (tok_tx, mut tok_rx) = mpsc::channel::<TextToken>(32);
    let ctrl_fut = ctrl.execute_streaming(domain_req, tok_tx);
    tokio::pin!(ctrl_fut);
    loop {
        tokio::select! {
            Some(token) = tok_rx.recv() => {
                let chunk = StreamChunk {
                    payload: Some(stream_chunk::Payload::TextDelta(token.text)),
                };
                if tx.send(Ok(chunk)).await.is_err() { return; }
            }
            result = &mut ctrl_fut => {
                match result {
                    Ok(resp) => {
                        let final_chunk = StreamChunk {
                            payload: Some(stream_chunk::Payload::Final(
                                bridge::response::to_proto(resp))),
                        };
                        let _ = tx.send(Ok(final_chunk)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(bridge::error::to_status(e))).await;
                    }
                }
                return;
            }
        }
    }
}
```

### 10.4 Client disconnect

Si le client fait `cancel()`, `tx.send().await` retourne `Err`, le spawn
abandonne la tâche du controller (via `tokio::select!` avec un cancellation
token optionnel). Pas de leak.

### 10.5 Backpressure

Le channel a une capacité de 32 tokens. Si le client est lent, le
dissolver bloque sur `tok_tx.send().await`, ce qui propage la backpressure
jusqu'aux kernels. Acceptable pour MVP.

---

## 11. Admin API — gestion des cônes M9

### 11.1 Pass-through léger

Chaque méthode `AdminService.*` fait :

1. Auth (scope admin:*).
2. Traduction proto → domain (via `bridge::cone`).
3. Appel direct à `self.registry.<method>().await`.
4. Traduction domain → proto.
5. Mapping erreur via `bridge::error`.

Aucune logique métier ajoutée — M10 reste mince.

### 11.2 Exemple : `BreachHistory`

```rust
async fn breach_history(&self, req: Request<BreachHistoryRequest>)
    -> Result<Response<BreachHistoryResponse>, Status>
{
    let inner = req.into_inner();
    let mut filter = BreachFilter::none();
    if !inner.tenant.is_empty() {
        filter = filter.with_tenant(inner.tenant);
    }
    if !inner.task.is_empty() {
        filter = filter.with_task(inner.task);
    }
    if inner.min_magnitude > 0.0 {
        filter = filter.with_min_magnitude(inner.min_magnitude);
    }
    let limit = if inner.limit == 0 { 100 } else { inner.limit as usize };
    let reports = self.registry
        .breach_history(limit, Some(filter)).await
        .map_err(bridge::error::to_status)?;
    let proto_reports = reports.into_iter()
        .map(bridge::cone::breach_to_proto)
        .collect();
    Ok(Response::new(BreachHistoryResponse { reports: proto_reports }))
}
```

---

## 12. Graceful shutdown et backpressure

### 12.1 Signal handling

```rust
pub struct Shutdown {
    triggered: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl Shutdown {
    pub async fn signal(&self) {
        let sigterm = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate())
            .expect("valid SIGTERM");
        tokio::pin!(sigterm);
        tokio::select! {
            _ = sigterm.recv() => {},
            _ = self.notify.notified() => {},
        }
        self.triggered.store(true, Ordering::SeqCst);
    }
}
```

### 12.2 Drain behavior

`tonic::transport::Server::serve_with_shutdown` accepte le future de shutdown.
Lorsqu'il complète :

1. Le listener TCP rejette les nouvelles connexions.
2. Les streams ouverts poursuivent jusqu'à la deadline (`config.shutdown_grace_period`).
3. Après deadline, les streams restants sont abandonnés (client reçoit
   `CANCELLED`).

### 12.3 Health check pendant shutdown

Le `HealthServiceImpl` écoute le flag `shutdown.triggered` et retourne
`NOT_SERVING` dès que SIGTERM est reçu — permettant au load balancer
Kubernetes de retirer le pod avant qu'il ne devienne non-responsive.

### 12.4 Config

```toml
[api.shutdown]
grace_period_seconds = 30    # max drain time
force_abort_after_seconds = 45
```

---

## 13. SDK Python — `python/efe/`

### 13.1 Public API

```python
# python/efe/__init__.py
from .client import AsyncClient, SyncClient
from .types import (
    SwarmSpec, AlignmentCone, PopulationMix, TemplateCode,
    DecodeMode, DecodedOutput, Verdict, Response,
)
from .errors import ClientError, ServerError, AuthError, RateLimitError
from .streaming import StreamingResponse

__all__ = [
    "AsyncClient", "SyncClient",
    "SwarmSpec", "AlignmentCone", "PopulationMix", "TemplateCode",
    "DecodeMode", "DecodedOutput", "Verdict", "Response",
    "ClientError", "ServerError", "AuthError", "RateLimitError",
    "StreamingResponse",
]
```

### 13.2 `client.py` — cœur

```python
from __future__ import annotations
import asyncio
from dataclasses import dataclass
from typing import AsyncIterator, Optional

import grpc

from .proto import efe_pb2, efe_pb2_grpc
from .types import SwarmSpec, DecodeMode, Response
from .errors import map_grpc_error
from .streaming import StreamingResponse


class AsyncClient:
    def __init__(
        self,
        endpoint: str,
        token: Optional[str] = None,
        timeout_s: float = 30.0,
        tls: bool = True,
    ):
        creds = (
            grpc.aio.ssl_channel_credentials() if tls
            else grpc.aio.insecure_channel_credentials()
        )
        self._channel = grpc.aio.secure_channel(endpoint, creds) if tls \
            else grpc.aio.insecure_channel(endpoint)
        self._stub = efe_pb2_grpc.SwarmServiceStub(self._channel)
        self._token = token
        self._timeout = timeout_s

    def _metadata(self) -> list[tuple[str, str]]:
        md = []
        if self._token:
            md.append(("authorization", f"Bearer {self._token}"))
        return md

    async def execute(
        self,
        spec: SwarmSpec,
        *,
        raw_prompt: str = "",
        request_id: Optional[str] = None,
        decode_mode: DecodeMode = DecodeMode.text(256),
    ) -> Response:
        req = efe_pb2.ExecuteRequest(
            request_id=request_id or "",
            raw_prompt=raw_prompt,
            spec=spec.to_proto(),
            decode_mode=decode_mode.to_proto(),
        )
        try:
            proto_resp = await self._stub.Execute(
                req, metadata=self._metadata(), timeout=self._timeout)
        except grpc.aio.AioRpcError as e:
            raise map_grpc_error(e) from None
        return Response.from_proto(proto_resp)

    async def execute_streaming(
        self,
        spec: SwarmSpec,
        **kwargs,
    ) -> AsyncIterator[StreamingResponse]:
        req = efe_pb2.ExecuteRequest(
            spec=spec.to_proto(),
            decode_mode=kwargs.get("decode_mode",
                DecodeMode.text(256)).to_proto(),
            raw_prompt=kwargs.get("raw_prompt", ""),
            request_id=kwargs.get("request_id", ""),
        )
        stream = self._stub.ExecuteStreaming(
            req, metadata=self._metadata())
        async for chunk in stream:
            yield StreamingResponse.from_proto(chunk)

    async def close(self):
        await self._channel.close()

    async def __aenter__(self):
        return self
    async def __aexit__(self, *_):
        await self.close()


class SyncClient:
    """Thin wrapper around AsyncClient for non-async callers."""
    def __init__(self, *args, **kwargs):
        self._async = AsyncClient(*args, **kwargs)
        self._loop = asyncio.new_event_loop()

    def execute(self, spec, **kwargs):
        return self._loop.run_until_complete(
            self._async.execute(spec, **kwargs))

    def close(self):
        self._loop.run_until_complete(self._async.close())
        self._loop.close()
```

### 13.3 `types.py` — dataclasses idiomatiques

```python
from dataclasses import dataclass, field
from enum import Enum
from typing import List, Optional, Union
from . import proto

class TemplateCode(Enum):
    NA_SEM = "na-sem"
    NA_LOG = "na-log"
    NA_CRIT = "na-crit"

@dataclass
class PopulationEntry:
    template: TemplateCode
    count: int

@dataclass
class PopulationMix:
    entries: List[PopulationEntry]

@dataclass
class AlignmentCone:
    anchors: List[List[float]]
    max_projection_distance: float

    def to_proto(self) -> proto.AlignmentCone:
        return proto.AlignmentCone(
            anchors=[proto.Anchor(values=a) for a in self.anchors],
            max_projection_distance=self.max_projection_distance,
        )

@dataclass
class SwarmSpec:
    problem_embedding: List[float]
    dim_latent: int
    population: PopulationMix
    max_rounds: int
    alignment_cone: AlignmentCone
    # ... other fields ...

    def to_proto(self) -> proto.SwarmSpec:
        return proto.SwarmSpec(
            problem_embedding=self.problem_embedding,
            dim_latent=self.dim_latent,
            population=[
                proto.PopulationEntry(
                    template_code=e.template.value,
                    count=e.count,
                )
                for e in self.population.entries
            ],
            max_rounds=self.max_rounds,
            alignment_cone=self.alignment_cone.to_proto(),
        )

    @classmethod
    def from_proto(cls, p: proto.SwarmSpec) -> "SwarmSpec":
        return cls(
            problem_embedding=list(p.problem_embedding),
            dim_latent=p.dim_latent,
            population=PopulationMix([
                PopulationEntry(
                    template=TemplateCode(e.template_code),
                    count=e.count,
                )
                for e in p.population
            ]),
            max_rounds=p.max_rounds,
            alignment_cone=AlignmentCone(
                anchors=[list(a.values) for a in p.alignment_cone.anchors],
                max_projection_distance=p.alignment_cone.max_projection_distance,
            ),
        )
```

### 13.4 `errors.py`

```python
import grpc

class ClientError(Exception):
    """Client-side error (network, serialization, ...)."""

class ServerError(Exception):
    """Server returned a gRPC error other than auth/ratelimit."""
    def __init__(self, code: grpc.StatusCode, message: str):
        super().__init__(f"[{code.name}] {message}")
        self.code = code
        self.message = message

class AuthError(ServerError):
    """UNAUTHENTICATED or PERMISSION_DENIED."""

class RateLimitError(ServerError):
    """RESOURCE_EXHAUSTED; carries retry_after_ms."""
    def __init__(self, message: str, retry_after_ms: int):
        super().__init__(grpc.StatusCode.RESOURCE_EXHAUSTED, message)
        self.retry_after_ms = retry_after_ms

def map_grpc_error(e: grpc.aio.AioRpcError) -> Exception:
    code = e.code()
    msg = e.details() or str(e)
    if code in (grpc.StatusCode.UNAUTHENTICATED,
                grpc.StatusCode.PERMISSION_DENIED):
        return AuthError(code, msg)
    if code == grpc.StatusCode.RESOURCE_EXHAUSTED:
        retry = 0
        for k, v in (e.initial_metadata() or []):
            if k == "retry-after-ms":
                try: retry = int(v)
                except ValueError: pass
        return RateLimitError(msg, retry)
    return ServerError(code, msg)
```

### 13.5 Packaging

```toml
# pyproject.toml
[project]
name = "efe"
version = "0.0.1"
requires-python = ">=3.12"
dependencies = [
    "grpcio >= 1.60",
    "grpcio-tools >= 1.60",
    "protobuf >= 5.0",
]

[project.optional-dependencies]
embedded = ["efe-ffi >= 0.0.1"]  # wheels built with maturin

[build-system]
requires = ["maturin >= 1.6"]
build-backend = "maturin"
```

### 13.6 Génération code proto

```bash
buf generate --template buf.gen.python.yaml --output python/efe/proto
```

---

## 14. FFI PyO3 — `efe-ffi`

### 14.1 Use case

Pour les notebooks / scripts batch / pipelines MLops où on veut
éviter le coût réseau gRPC, `efe-ffi` expose une API Python qui
embarque `SwarmController` en direct, via PyO3.

Zéro trip réseau. Latence ~0.5 ms overhead Python → Rust.

### 14.2 Module Python

```python
# Installation :
# pip install efe[embedded]

from efe_ffi import PyController, PySwarmSpec, PyResponse

controller = PyController.default()  # uses stubs at MVP
spec = PySwarmSpec.builder() \
    .problem_embedding([0.0]*128) \
    .dim_latent(128) \
    .population_uniform("na-sem", 4) \
    .max_rounds(4) \
    .alignment_cone([[1.0]*128], 0.5) \
    .build()

resp: PyResponse = controller.execute(spec, prompt="hello")
print(resp.output_text)
```

### 14.3 Implémentation `py_controller.rs`

```rust
use pyo3::prelude::*;
use pyo3_asyncio::tokio::future_into_py;
use std::sync::Arc;

#[pyclass]
pub struct PyController {
    inner: Arc<dyn efe_core::SwarmController>,
}

#[pymethods]
impl PyController {
    #[staticmethod]
    pub fn default() -> PyResult<Self> {
        // MVP stub: uses NullStrategy everywhere. Real strategies
        // loaded via a separate module (efe-runtime) post-MVP.
        let controller = efe_core::SwarmController::builder()
            // ... .build() ...
            ;
        Ok(Self { inner: Arc::new(controller) })
    }

    pub fn execute<'py>(
        &self,
        py: Python<'py>,
        spec: PySwarmSpec,
    ) -> PyResult<&'py PyAny> {
        let inner = self.inner.clone();
        let domain_req = spec.into_domain_request()?;
        future_into_py(py, async move {
            let resp = inner.execute(domain_req).await
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                    e.to_string()))?;
            Python::with_gil(|py| Py::new(py, PyResponse::from_domain(resp)))
        })
    }
}

#[pymodule]
fn efe_ffi(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<PyController>()?;
    m.add_class::<PySwarmSpec>()?;
    m.add_class::<PyResponse>()?;
    m.add("__version__", efe_core::VERSION)?;
    Ok(())
}
```

### 14.4 Conversions bf16 / numpy

Le type `Vec<f32>` côté Rust ↔ `numpy.ndarray[dtype=f32]` côté Python.
Implémenté via `numpy` crate :

```rust
use numpy::{IntoPyArray, PyArray1, PyReadonlyArray1};

#[pymethods]
impl PyResponse {
    pub fn centroid<'py>(&self, py: Python<'py>)
        -> Option<&'py PyArray1<f32>>
    {
        match &self.inner.output {
            efe_core::DecodedOutput::Raw(v) =>
                Some(v.clone().into_pyarray(py)),
            _ => None,
        }
    }
}
```

---

## 15. Observabilité end-to-end

### 15.1 Propagation des traces

```
Client Python (otel instrumented)
  span "client.execute"
  └─ traceparent: 00-abc123...-def456-01
          │
          │ gRPC metadata
          ▼
Rust server (TracingLayer)
  span "grpc.request" (extract traceparent)
  └─ span "swarm.execute" (SwarmService handler)
          │
          ▼
M1 controller
  span "arbiter.run" (opens via TraceContext)
          │
          ▼
M5/M6/M7/M8 spans (via TelemetryCollector)
```

### 15.2 Métriques Prometheus (port sidecar 9090)

| Metric | Type | Labels | Description |
|---|---|---|---|
| `efe_grpc_requests_total` | counter | `method,code,tenant` | total RPCs |
| `efe_grpc_request_duration_seconds` | histogram | `method,tenant` | E2E latency |
| `efe_grpc_requests_in_flight` | gauge | `method` | concurrency |
| `efe_auth_failures_total` | counter | `reason` | unauth/forbidden |
| `efe_ratelimit_drops_total` | counter | `tenant` | 429 fires |
| `efe_idempotency_hits_total` | counter | | cache short-circuits |
| `efe_shutdown_in_progress` | gauge | | 0 or 1 |

### 15.3 Logs structurés JSON

Format exemple :
```json
{
  "ts": "2026-05-14T12:34:56.789Z",
  "level": "INFO",
  "event": "grpc.request.complete",
  "method": "/efe.v1.SwarmService/Execute",
  "tenant": "acme",
  "request_id": "01HWZR...",
  "trace_id": "abc123",
  "span_id": "def456",
  "duration_ms": 142.3,
  "status_code": "OK"
}
```

Aucun log de `raw_prompt` par défaut (opt-in `--log-prompts` +
warning au démarrage).

---

## 16. Modèle de sécurité et threat model

### 16.1 Menaces identifiées

| # | Menace | Vecteur | Mitigation |
|---|---|---|---|
| T1 | Token vol/forge | Attaquant copie/forge un JWT | HS256 secret côté serveur ; rotation 15min ; `exp` strict. |
| T2 | Replay JWT expiré | Attaquant rejoue un vieux token | Validation `exp` ; TTL court. |
| T3 | DDoS par RPS | Flood d'Execute | Rate limit + concurrency limit + timeout. |
| T4 | DDoS par payload | Prompts 1 GB | Limite `tonic::transport::Server::max_frame_size = 4 MiB`. |
| T5 | Fuite info via erreur | `Status::Internal(e.to_string())` | Mapping strict §6.4 ; `Internal` ne fuite jamais. |
| T6 | Injection logs | Prompt avec `\n` + ANSI | Logs JSON-escaped ; pas de `raw_prompt` par défaut. |
| T7 | SSRF / admin escalation | Scope admin donné par erreur | Scopes granulaires + default-deny. |
| T8 | Memory exhaustion (idempotency) | Client pousse 10⁹ request_id uniques | Cache LRU capped 10 k entries ; TTL 10 s. |
| T9 | PII in breach reports | Reason contient email/nom | `BREACH_REASON_MAX_LEN = 256` + audit manuel. |
| T10 | Man-in-the-middle | Pas de TLS | TLS obligatoire en prod (config flag refusé en prod). |

### 16.2 Non-threats (acceptés MVP)

- **Replay attack authentifié** : un JWT valide peut être rejoué jusqu'à
  `exp`. Idempotency cache mitige partiellement. mTLS post-MVP.
- **Latéral cross-tenant dans logs** : un opérateur humain voit tous les
  logs. ACL logs post-MVP.
- **Admin API brute-force** : pas de lockout sur échec d'auth. Acceptable
  car rate limit s'applique.

### 16.3 Zone de confiance

```
┌─ zone publique ─────────┐  ┌─ zone privée ──────────────────────┐
│                          │  │                                     │
│   Client externe          │  │  efe-server ── local ──> M1-M9     │
│   (mobile / web / CLI)   │  │                                     │
│         │                │  │  loopback / socket                  │
│   TLS + JWT              │  │                                     │
└───────────┬──────────────┘  └────────────────────────────────────┘
            │                            ▲
            │────────── gRPC ────────────┘
```

- TLS termine au serveur.
- M1-M9 tournent dans le même processus que `efe-server` (MVP). Pas de
  RPC inter-process.
- M11 (OTel collector) peut tourner en sidecar ; communication via
  loopback OTLP/gRPC.

---

## 17. Budget de performance et SLO

### 17.1 SLO cibles

| Métrique | Target MVP | Stretch |
|---|---|---|
| p50 latence `Execute` (N=512, D=256) | < 100 ms | < 80 ms |
| p95 latence `Execute` | < 180 ms | < 140 ms |
| p99 latence `Execute` | < 250 ms | < 200 ms |
| p99 latence `DescribeSpec` | < 5 ms | < 2 ms |
| Throughput `Execute` sur 1 GPU A100 | ≥ 500 rps | ≥ 800 rps |
| Availability | 99.5 % | 99.9 % |
| Time-to-first-byte streaming | < 50 ms | < 30 ms |

### 17.2 Overhead attribué à M10

- Serialization proto : < 0.5 ms pour N=512, D=256.
- JWT validation : < 0.1 ms (in-memory key).
- Rate limit check : < 0.01 ms (DashMap + atomic).
- Tracing layer : < 0.2 ms overhead.
- **Total M10 overhead** : **< 1 ms** du budget p99.

### 17.3 Benchmarks criterion

```rust
#[bench] fn bench_bridge_spec_from_proto();
#[bench] fn bench_bridge_response_to_proto();
#[bench] fn bench_auth_jwt_validate();
#[bench] fn bench_ratelimit_acquire();
#[bench] fn bench_idempotency_get_miss();
#[bench] fn bench_idempotency_get_hit();
```

### 17.4 Load testing

Outils :
- `ghz` pour load gRPC (RPS, concurrency, histogram).
- `k6` (avec plugin gRPC) pour scénarios complexes.

Cible : 500 rps soutenus 10 min sans dégradation p99.

---

## 18. Stratégie de test

### 18.1 Pyramide

```
        ┌──────────────┐  Load / chaos (ghz, k6)
        │              │
        │ Integration  │  python ↔ rust (docker-compose)
        │              │
        ├──────────────┤
        │              │
        │  Unit (Rust) │  bridge, auth, ratelimit, idempotency
        │              │
        ├──────────────┤
        │              │
        │ Unit (Py SDK)│  pytest, types, error mapping
        │              │
        └──────────────┘
```

### 18.2 Tests unit Rust (estimation)

| Fichier | Tests |
|---|---|
| `bridge/spec.rs` | 12 (happy, dim mismatch, bad template, ...) |
| `bridge/request.rs` | 8 |
| `bridge/response.rs` | 6 |
| `bridge/cone.rs` | 10 (AlignmentCone + Metadata + EvolutionOp) |
| `bridge/error.rs` | 12 (un par variant EfeError → Status) |
| `auth/jwt.rs` | 10 (valid, expired, wrong audience, bad sig, rotation) |
| `auth/scopes.rs` | 8 |
| `ratelimit/bucket.rs` | 12 (acquire, refill, overflow, concurrent) |
| `ratelimit/registry.rs` | 6 |
| `idempotency/cache.rs` | 10 (hit, miss, eviction, TTL, concurrent) |
| `services/swarm.rs` | 12 (via mock controller) |
| `services/admin.rs` | 10 |

**Total unit Rust : ~115 tests**

### 18.3 Tests intégration Python ↔ Rust (`tests/integration/`)

1. `test_unary_execute_happy_path`
2. `test_unary_execute_with_invalid_spec_returns_422_equivalent`
3. `test_streaming_yields_tokens_then_final`
4. `test_streaming_cancel_propagates`
5. `test_describe_spec_valid`
6. `test_describe_spec_invalid_reports_errors`
7. `test_auth_missing_token_unauthenticated`
8. `test_auth_expired_token_unauthenticated`
9. `test_auth_wrong_scope_permission_denied`
10. `test_ratelimit_burst_then_429`
11. `test_ratelimit_returns_retry_after`
12. `test_idempotency_same_request_id_short_circuits`
13. `test_admin_register_and_get_cone`
14. `test_admin_evolve_chain`
15. `test_health_check_serving_then_not_serving_on_shutdown`
16. `test_graceful_shutdown_drains_inflight`
17. `test_tls_connection_valid_cert`
18. `test_trace_context_propagated_through`
19. `test_breach_history_filter_by_tenant`
20. `test_reflection_enumerates_services`

### 18.4 Tests chaos

- Tuer le serveur mid-stream → client voit `UNAVAILABLE`.
- Partition réseau (tc / iptables) → deadline exceeded.
- Lenteur backend (mock sleep 10s) → timeout propre.

### 18.5 Tests FFI PyO3

- `pytest` sur `efe_ffi` import + `PyController.default()` + execute.
- Vérifie numpy ↔ Vec<f32> roundtrip.
- Vérifie release GIL pendant `execute` (concurrence Python).

### 18.6 Coverage cible

- Rust : ≥ 85 % ligne.
- Python : ≥ 80 % ligne (moins strict car beaucoup de code généré).

---

## 19. Dépendances inter-modules

### 19.1 Sortantes

| De | Vers | Nature |
|---|---|---|
| efe-server | M1 (`SwarmController`, `Request`, `Response`) | handler principal |
| efe-server | M9 (`AlignmentRegistry`, types) | admin API |
| efe-server | M11 (OTel, Prometheus) | observabilité |
| efe-server | externes : `tonic`, `prost`, `tower`, `jsonwebtoken`, `dashmap`, `jwt` | infra |
| efe-ffi | M1 | embedding |
| efe-ffi | externes : `pyo3`, `pyo3-asyncio`, `numpy` | bindings |
| python/efe | efe-server (via gRPC) | client |
| python/efe | externes : `grpcio`, `protobuf` | infra |

### 19.2 Entrantes

Aucune — M10 est une feuille de l'arbre (pas d'autre module ne dépend
de lui). M12 le packagent dans le Docker final.

---

## 20. Plan de sprint M10

### 20.1 Sprint 7 — Proto + serveur unary MVP

**Durée** : 2 semaines.

| Jour | Livrable |
|---|---|
| J1 | Protos `efe.v1.SwarmService` + `types.proto` + `admin.proto` + `buf.yaml` |
| J2 | `efe-server` avec `tonic-build`, squelette `runtime.rs`, health check |
| J3-J4 | `bridge/spec.rs` + `bridge/request.rs` + `bridge/response.rs` + 26 tests |
| J5 | `bridge/error.rs` (12 tests) + `bridge/cone.rs` (10 tests) |
| J6 | `services/swarm.rs::execute` (unary) + tests sur stub controller |
| J7-J8 | Integration test Python ↔ Rust (happy path + invalid spec) |
| J9 | Graceful shutdown + SIGTERM handler |
| J10 | Tonic reflection + `DescribeSpec` + doc |

**Gate sortie S7** : 40+ tests passent, `grpcurl` peut exécuter un
round-trip sur `DescribeSpec`, CI verte.

### 20.2 Sprint 8 — Auth + ratelimit + streaming

| Jour | Livrable |
|---|---|
| J1 | `auth/jwt.rs` + `TokenStore` + tests (10) |
| J2 | `auth/scopes.rs` + mapping RPC → scope (8 tests) |
| J3 | `AuthLayer` tower + integration test |
| J4 | `ratelimit/bucket.rs` + `BucketRegistry` (18 tests) |
| J5 | `RateLimitLayer` tower + integration tests (3) |
| J6-J7 | `services/admin.rs` — 6 RPCs pour M9 (10 tests) |
| J8-J9 | `execute_streaming` — implémentation + test |
| J10 | Integration test streaming + cancel |

### 20.3 Sprint 9 — SDK Python + FFI + load testing

| Jour | Livrable |
|---|---|
| J1 | `python/efe/types.py` + génération proto via `buf generate` |
| J2 | `python/efe/client.py` (AsyncClient + SyncClient) |
| J3 | `python/efe/streaming.py` + `python/efe/errors.py` |
| J4 | Integration tests Python (20 scénarios) |
| J5 | `efe-ffi` — `PyController` + `PyRequest/Response` |
| J6 | numpy interop + tests FFI |
| J7 | TLS support + mutual cert tests (MVP only server cert) |
| J8 | Idempotency cache + tests |
| J9 | Load testing ghz + tuning |
| J10 | Dashboards Grafana + runbook ops |

### 20.4 Jalons

- **J-M10.1 (fin S7)** : Server unary up + Python `grpcurl`-able.
- **J-M10.2 (fin S8)** : Full auth + ratelimit + streaming + admin.
- **J-M10.3 (fin S9)** : SDK Python v0.0.1 publishable + 500 rps soutenu.

---

## 21. Plan de migration et rollout

### 21.1 Phase A : Dev internal (S7 fin)

- Déployer `efe-server` sur runner CI.
- Python test suite pointe sur `localhost:50051`.
- Pas de TLS, pas d'auth (feature flag `--no-auth-dev`).

### 21.2 Phase B : Staging (S8 fin)

- Déploiement en cluster staging (Kubernetes minikube).
- TLS avec cert Let's Encrypt staging.
- Auth JWT avec secret distribué via sealed-secret.
- Smoke test : 10 rps × 1h.

### 21.3 Phase C : Production interne (S9 fin)

- Déploiement sur 1 pod single-replica dans cluster prod.
- 1 client pilote (équipe plateforme).
- 10 rps soutenu ; scaling horizontal après validation.
- Monitoring : dashboards publiés, alertes Pagerduty configurées.

### 21.4 Rollback

- Feature flag `api_v1_enabled` dans config.
- En cas d'incident, désactiver via ConfigMap hot-reload.
- Le client SDK accepte un fallback `local mode` via `efe-ffi`.

---

## 22. Matrice de risques et mitigations

| # | Risque | Proba | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Proto breaking change non détecté | Moyenne | Élevé | `buf breaking` en CI contre tag précédent. |
| R2 | Fuite de `EfeError::Internal` msg | Faible | Moyen | `bridge::error` force mapping explicite + tests. |
| R3 | JWT secret exposé dans logs | Faible | Critique | `token_store` implémente `Debug` manuel (`***`). |
| R4 | DDoS par gros payload | Moyenne | Moyen | `max_frame_size = 4MiB`. |
| R5 | Rate limit contourné via anonymous | Faible | Moyen | anonymous bucket partagé pour public endpoints. |
| R6 | Overhead gRPC > budget | Moyenne | Moyen | Benchmarks + profile avant release. |
| R7 | Streaming memleak (client slow) | Faible | Moyen | Channel bounded + timeout. |
| R8 | TLS cert expire non détecté | Faible | Élevé | Cert-manager + alerte 30j avant expiration. |
| R9 | SDK Python API incompat v1 → v2 | Moyenne | Moyen | Strict semver + deprecation warnings. |
| R10 | PyO3 FFI crash → process kill | Moyenne | Élevé | Test FFI dans subprocess ; bindings sans unsafe. |
| R11 | `tonic-build` incompat toolchain | Faible | Moyen | Pin versions workspace ; CI matrix. |
| R12 | Shutdown non-gracieux kill in-flight | Moyenne | Moyen | Test chaos + deadline configurable. |

### 22.1 Risques résiduels acceptés

- **Single-node rate limit** : ok MVP, migration Redis post-MVP.
- **HS256 JWT** : pas de signature asymétrique ; acceptable secret partagé.
- **Python SDK uniquement** : autres langages via raw gRPC.

---

## 23. Critères d'acceptation

### 23.1 Fonctionnels

- **CA-F1** : `Execute` unary latence p99 < 250 ms sur N=512 D=256 A100.
- **CA-F2** : `ExecuteStreaming` TTFB p99 < 50 ms.
- **CA-F3** : `DescribeSpec` latence p99 < 5 ms.
- **CA-F4** : 500 rps soutenus sur 10 min sans dégradation p99.
- **CA-F5** : Auth : 100 % JWT invalides rejetés avec `UNAUTHENTICATED`.
- **CA-F6** : Rate limit : dépassement → `RESOURCE_EXHAUSTED` + `retry-after-ms`.
- **CA-F7** : Shutdown graceful : toutes les requêtes in-flight complètent
  sous 30s, aucune nouvelle acceptée après SIGTERM.
- **CA-F8** : Mapping erreur : chaque variant `EfeError` a un code gRPC
  déterministe (testé).
- **CA-F9** : Idempotency : `request_id` dupliqué dans 10 s retourne la
  même réponse (cache hit).
- **CA-F10** : Admin API : tous les opés M9 accessibles (6 RPCs).
- **CA-F11** : SDK Python : `AsyncClient.execute(spec)` retourne un
  `Response` dataclass équivalent au domain.
- **CA-F12** : FFI : `efe_ffi.PyController().execute(spec)` fonctionne
  sans latence réseau.

### 23.2 Non-fonctionnels

- **CA-NF1** : Zéro `unsafe` dans `efe-server`. `efe-ffi` limité aux
  macros PyO3.
- **CA-NF2** : Clippy pedantic -D warnings sur `efe-server`.
- **CA-NF3** : Coverage ≥ 85 % sur `efe-server`, ≥ 80 % sur `python/efe`.
- **CA-NF4** : `buf lint` passe.
- **CA-NF5** : `cargo doc --no-deps` sans warning.
- **CA-NF6** : Bench CI : aucune régression > 10 % sur les benchmarks
  listés §17.3.
- **CA-NF7** : Zero PII dans logs par défaut (vérifié par grep en CI).

### 23.3 Documentation

- **CA-D1** : ADR-010 "API versioning strategy".
- **CA-D2** : ADR-011 "JWT HS256 vs mTLS tradeoff".
- **CA-D3** : Runbook `docs/runbooks/m10-api-ops.md` (rotation clé,
  rollback, incident response).
- **CA-D4** : README `python/efe/README.md` avec quickstart.
- **CA-D5** : `docs/api/grpc.md` avec référence proto complète.

---

## 24. Annexes

### 24.1 Annexe A — Example end-to-end

#### A.1 Serveur Rust

```rust
// src/main.rs
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cfg = ServerConfig::from_env()?;
    let controller = build_controller().await?;
    let registry = build_registry().await?;
    let server = Server::builder()
        .config(cfg)
        .controller(Arc::new(controller))
        .registry(Some(Arc::new(registry)))
        .build()?;
    let shutdown = Shutdown::new();
    server.serve(cfg.listen_addr, shutdown).await?;
    Ok(())
}
```

#### A.2 Client Python

```python
import asyncio
from efe import AsyncClient, SwarmSpec, AlignmentCone, PopulationMix, \
    PopulationEntry, TemplateCode

async def main():
    async with AsyncClient("grpc+tls://api.efe.ai:443", token="eyJ...") as c:
        spec = SwarmSpec(
            problem_embedding=[0.0]*128,
            dim_latent=128,
            population=PopulationMix([
                PopulationEntry(TemplateCode.NA_SEM, 4)
            ]),
            max_rounds=4,
            alignment_cone=AlignmentCone(
                anchors=[[1.0]*128], max_projection_distance=0.5),
        )
        resp = await c.execute(spec, raw_prompt="hello, world")
        print(resp.output.text)
        print(f"rounds={resp.rounds_executed} conf={resp.confidence}")

asyncio.run(main())
```

### 24.2 Annexe B — Glossaire

| Terme | Définition |
|---|---|
| gRPC | Protocol RPC basé HTTP/2 + Protobuf. |
| tonic | Crate Rust gRPC. |
| tower | Abstraction de couches / middleware. |
| JWT | JSON Web Token, format de claim signé. |
| HS256 | HMAC-SHA256, algo JWT à secret partagé. |
| PyO3 | Bindings Rust ↔ Python. |
| OTel | OpenTelemetry, framework observabilité. |
| Bucket | Token bucket, algo de rate limiting. |
| TTFB | Time To First Byte. |
| DescribeSpec | RPC qui valide un spec sans exécution. |
| Idempotency cache | Cache keyed par request_id. |

### 24.3 Annexe C — Config TOML complète

```toml
[api.server]
listen_addr = "0.0.0.0:50051"
metrics_port = 9090
max_concurrent_streams = 1024
max_frame_size_bytes = 4_194_304
request_timeout_seconds = 30

[api.tls]
enabled = true
cert_path = "/etc/efe/tls/server.crt"
key_path = "/etc/efe/tls/server.key"

[api.auth]
enabled = true
jwt_secret_env = "EFE_JWT_SECRET"
jwt_audience = "efe.v1"
allow_insecure_dev = false

[api.ratelimit]
enabled = true
default_capacity = 200
default_refill_rate = 100
anonymous_capacity = 50
anonymous_refill_rate = 10

[api.idempotency]
enabled = true
cache_capacity = 10000
ttl_seconds = 10

[api.shutdown]
grace_period_seconds = 30
force_abort_after_seconds = 45

[api.logging]
format = "json"
level = "info"
log_raw_prompts = false  # security-sensitive, default off
```

### 24.4 Annexe D — Mapping rapide EfeError → Status

| `EfeError` variant | gRPC Code | Retryable ? |
|---|---|---|
| `InvalidConfig` | `INVALID_ARGUMENT` | non |
| `AlignmentBreach` | `FAILED_PRECONDITION` | non |
| `Internal` | `INTERNAL` | oui (avec backoff) |
| `HbmAllocFailed` | `RESOURCE_EXHAUSTED` | oui |
| `ModeCollapse` | `FAILED_PRECONDITION` | oui (nouveau seed) |
| `NumericalExplosion` | `INTERNAL` | non |
| `Divergence` | `FAILED_PRECONDITION` | non |
| `ConvergenceTimeout` | `DEADLINE_EXCEEDED` | oui (budget plus long) |
| `BudgetExhausted` | `RESOURCE_EXHAUSTED` | oui (budget plus long) |

### 24.5 Annexe E — FAQ

**Q1. Pourquoi gRPC et pas REST ?**
Streaming natif, proto typing fort, interop multilingue, moins de boilerplate.

**Q2. Pourquoi HS256 et pas RS256/ES256 ?**
Simplicité MVP (un secret partagé), perf meilleur. Post-MVP bascule vers
asymétrique.

**Q3. Pourquoi pas de session (stateful) ?**
Stateless scale mieux ; idempotency cache 10s suffit pour retry.

**Q4. Pourquoi `PyO3` si on a déjà gRPC ?**
Pour les notebooks / scripts où la latence réseau (~5 ms) est trop élevée.

**Q5. Est-ce que M10 peut tourner sans M9 ?**
Oui — si `registry = None`, l'AdminService n'est pas enregistré. `Execute`
fonctionne avec cones inline.

**Q6. Comment tester le serveur sans Python ?**
`grpcurl -import-path ./protos -proto efe/v1/efe.proto -d '{"spec": ...}' \
  localhost:50051 efe.v1.SwarmService/Execute`

**Q7. Peut-on changer le codage de `Verdict` sans casser ?**
Ajouter une valeur = OK (clients v1 verront `UNSPECIFIED`). Renommer ou
supprimer = breaking.

**Q8. Comment gérer un client qui envoie un prompt de 100 MB ?**
`max_frame_size = 4 MiB` rejette à la couche HTTP/2 avec
`RESOURCE_EXHAUSTED` avant même d'atteindre le handler.

**Q9. Que se passe-t-il si le controller panique ?**
tokio propagate le panic à la task, qui est catch par le runtime ; le
client reçoit `INTERNAL`. Le serveur reste up (panic=unwind, pas abort).

**Q10. Y a-t-il un endpoint WebSocket ?**
Non au MVP. gRPC-Web post-MVP si besoin UI navigateur.

### 24.6 Annexe F — Liste des fichiers à créer

**À créer**

Rust :
- `protos/efe/v1/efe.proto`
- `protos/efe/v1/types.proto`
- `protos/efe/v1/admin.proto`
- `protos/buf.yaml`, `protos/buf.gen.yaml`
- `crates/efe-server/build.rs`
- `crates/efe-server/src/lib.rs`
- `crates/efe-server/src/config.rs`
- `crates/efe-server/src/runtime.rs`
- `crates/efe-server/src/shutdown.rs`
- `crates/efe-server/src/services/{mod,swarm,admin,health}.rs`
- `crates/efe-server/src/bridge/{mod,spec,request,response,cone,error,decode,verdict,budget}.rs`
- `crates/efe-server/src/interceptors/{mod,auth,ratelimit,tracing,metrics}.rs`
- `crates/efe-server/src/auth/{mod,jwt,token_store,scopes}.rs`
- `crates/efe-server/src/ratelimit/{mod,bucket,registry}.rs`
- `crates/efe-server/src/idempotency/{mod,cache}.rs`
- `crates/efe-server/tests/{integration_unary,integration_streaming,integration_auth,integration_admin}.rs`
- `crates/efe-ffi/src/{lib,py_controller,py_request,py_spec,conversions}.rs`

Python :
- `python/efe/__init__.py`
- `python/efe/{client,auth,errors,streaming,types}.py`
- `python/efe/proto/__init__.py`  (généré)
- `python/pyproject.toml`
- `tests/integration/{test_grpc_unary,test_grpc_streaming,test_auth,test_ratelimit,test_admin}.py`

Ops :
- `docker/prod/Dockerfile` — ajoute `efe-server` binary
- `ops/grafana/m10-api.json` — dashboards gRPC
- `ops/prometheus/alerts-m10.yaml` — alertes P1/P2

Docs :
- `docs/docs/adr/ADR-010-api-versioning.md`
- `docs/docs/adr/ADR-011-jwt-choice.md`
- `docs/docs/api/grpc.md` — référence exhaustive
- `docs/runbooks/m10-api-ops.md`

**À modifier**
- `crates/efe-server/Cargo.toml` : ajouter `tonic`, `prost`, `tower`, `jsonwebtoken`, `dashmap`, `tonic-reflection`, `tonic-health`, `tokio-stream`.
- `crates/efe-ffi/Cargo.toml` : ajouter `pyo3`, `pyo3-asyncio`, `numpy`.
- `Cargo.toml` workspace : ajouter ces deps comme workspace.deps.
- `.github/workflows/ci.yml` : ajouter jobs `buf lint`, `buf breaking`,
  `pytest integration`.

**Inchangés**
- `efe-core` (M1-M9) — M10 ne modifie aucun type existant.

### 24.7 Annexe G — Checklist de revue PR M10

- [ ] `cargo fmt --check` OK
- [ ] `cargo clippy -- -D warnings` (pedantic sur `efe-server`)
- [ ] `cargo test` — nouveaux tests ajoutés
- [ ] `buf lint` OK
- [ ] `buf breaking` OK contre tag précédent
- [ ] `pytest python/` OK
- [ ] Integration tests OK (`docker-compose up -d && pytest tests/integration/`)
- [ ] Tout nouveau RPC a un scope assigné dans `auth/scopes.rs`
- [ ] Tout nouveau type proto est documenté
- [ ] `cargo doc --no-deps` sans warning
- [ ] ADR mis à jour si décision architecturale
- [ ] CHANGELOG.md mis à jour
- [ ] Load test `ghz` exécuté et p99 < seuil
- [ ] Dashboards Grafana synchronisés avec nouvelles métriques

---

**Fin du document M10_ARCHITECTURE.md — v1.0.**

Ce document est la **référence d'implémentation** du module M10. Toute
dérive significative (nouveau RPC, changement de scope, nouveau variant
proto) nécessite un ADR + mise à jour du présent document. La spec est
**gelée pour Sprint 7** à la date de signature.

| Rôle | Nom | Date |
|---|---|---|
| Auteur | Core Runtime Team | 2026-04-19 |
| Reviewer | Platform Security Lead | _pending_ |
| Approbateur | Platform Director | _pending_ |
