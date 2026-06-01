# EFE M50 — N22 HTTP Gateway Exposure Architecture

> **Sprint** : 14 (Phase 6, day 56 – 60 of M50).
> **Status** : design-frozen ; ready for code generation.
> **Sister deliverables** :
> * `EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md` (Sprint 9, shipped)
> * `EFE_M50_N17_LLM_PROVENANCE_ARCHITECTURE.md` (Sprint 9, shipped)
> * `EFE_M50_N18_PUBLISHER_IDENTITY_ARCHITECTURE.md` (Sprint 10, shipped)
> * `EFE_M50_N19_GOLD_TEST_GATE_TIER_1_ARCHITECTURE.md` (Sprint 11, shipped)
> * `EFE_M50_N20_POSTGRES_REAL_EMBEDDINGS_ARCHITECTURE.md` (Sprint 12, shipped)
> * `EFE_M50_N21_SWARM_ORCHESTRATION_AGENTIC_BRIDGE_ARCHITECTURE.md` (Sprint 13, shipped — POC lib-only)
>
> **Anchors** : N11-N21 shipped the **library-level capabilities**
> (publishers, gold gates, swarms, agentic bridges).  None of them
> expose a network-reachable endpoint.  Customers, operators, and
> external systems have **zero way to reach EFE** without writing
> Rust code against the library API.  N22 closes the **last
> deployment gap** : an `efe-gateway` binary that assembles every
> N11-N21 capability behind a versioned, auth-protected, multi-tenant
> HTTPS API following industry-standard practices (RFC 7807 errors,
> W3C Trace Context, OpenAPI 3.1, Prometheus metrics, K8s health
> probes, graceful shutdown).

---

## Table of contents

0. [TL;DR](#0-tldr)
1. [Context, motivation, sprint placement](#1-context-motivation-sprint-placement)
2. [Fifteen incident patterns N22 closes (P1–P15)](#2-fifteen-incident-patterns-n22-closes-p1p15)
3. [Commitments — what ships in Sprint 14 (G1–G15)](#3-commitments--what-ships-in-sprint-14-g1g15)
4. [Invariants — non-negotiables (I1–I20)](#4-invariants--non-negotiables-i1i20)
5. [Failure modes — what we PROTECT against (F1–F15)](#5-failure-modes--what-we-protect-against-f1f15)
6. [Architecture Decision Records (ADR-1 to ADR-10)](#6-architecture-decision-records-adr-1-to-adr-10)
7. [Component architecture](#7-component-architecture)
8. [Module hierarchy & data model](#8-module-hierarchy--data-model)
9. [API surface — full endpoint catalogue](#9-api-surface--full-endpoint-catalogue)
10. [End-to-end workflows](#10-end-to-end-workflows)
11. [Five-day implementation plan](#11-five-day-implementation-plan)
12. [Test strategy](#12-test-strategy)
13. [Cross-cutting alignment (N11 → N21)](#13-cross-cutting-alignment-n11--n21)
14. [Sprint 14 ship vs Sprint 15+ split](#14-sprint-14-ship-vs-sprint-15-split)
15. [Annexes](#15-annexes)

---

## 0. TL;DR

**The product blocker N22 closes.**  Through Sprint 13, EFE was a
collection of well-tested Rust crates with a CLI for some operator
tasks (`efe-publishers`, `efe-gold`, `efe-claude-smoke`).  No
network entry point exists.  Customers cannot integrate EFE into
their pipelines.  Operators cannot manage tenants without `cargo
run`.  External systems (Slack, Salesforce, Zendesk) have no HTTP
shape to call.

**Eight concrete capabilities ship in Sprint 14 N22** :

1. **`efe-gateway` binary** — a single self-contained Linux/macOS
   executable that boots an HTTPS server, exposes every N11-N21
   subsystem behind `/v1/*` routes, runs on K8s with standard
   health probes.
2. **Axum 0.8 + tower-http middleware stack** — battle-tested,
   widely-deployed, native async/tokio, native rustls.
3. **JWT bearer authentication** — HS256 for dev, RS256+JWKS for
   production ; `Authorization: Bearer <token>` everywhere except
   `/healthz`, `/readyz`, `/metrics`, `/openapi.json`.
4. **Multi-tenant isolation** — `tenant_id` claim in JWT, enforced
   on every request : `jwt.tenant_id == body.tenant_id` else 403.
5. **Per-tenant rate limiting** — `tower-governor` 100 req/s per
   `tenant_id` (configurable) ; in-memory Sprint 14, Redis Sprint 15.
6. **RFC 7807 Problem Details error responses** — every 4xx/5xx
   carries a `application/problem+json` body with `type`, `title`,
   `status`, `detail`, `instance`, `trace_id`.
7. **W3C Trace Context propagation** — `traceparent` header parsed
   on every request ; `trace_id` injected into every log line,
   every downstream call, every audit row.
8. **OpenAPI 3.1 spec auto-generated** via `utoipa` ; served at
   `/openapi.json` + UI at `/docs` (Swagger).

**The endpoint catalogue** (full list § 9) :

| Group | Endpoints | Sprint |
|---|---|---|
| Health & meta | `GET /healthz`, `/readyz`, `/metrics`, `/openapi.json`, `/docs` | 14 |
| Swarm (N21) | `POST /v1/swarms/run`, `POST /v1/swarms/dags`, `GET /v1/swarms/{id}` | 14 |
| Agentic (N21) | `POST /v1/agentic/call` | 14 |
| Publishers (N18) | `POST /v1/publishers`, `GET /v1/publishers/{id}`, `POST /v1/publishers/{id}/rotate-key`, `POST /v1/publishers/{id}/revoke` | 14 |
| Gold (N19) | `POST /v1/gold-sets`, `POST /v1/gold-sets/{id}/approve`, `POST /v1/gold-runs`, `GET /v1/gold-runs/{id}` | 14 |
| Replay (N16) | `POST /v1/replays`, `GET /v1/replays/{id}` | 14 |
| Provenance (N17) | `GET /v1/provenance/{id}` | 14 |
| Admin | `GET /v1/admin/circuit-breakers`, `POST /v1/admin/circuit-breakers/{provider}/reset`, `POST /v1/admin/drift-sweep/trigger` | 14 |

**Total : 21 HTTP endpoints**, all OpenAPI-documented, all
auth-gated, all RFC 7807-compliant on errors.

**Backward compatibility.**  Pure addition : no library crate
signature changes.  The gateway depends on every N11-N21 crate ;
those crates remain individually consumable as libraries.  Existing
CLIs (`efe-publishers`, `efe-gold`, `efe-claude-smoke`) continue to
work unchanged.

**Out of scope (deferred Sprint 15+)** :
- **gRPC transport** — only HTTP/JSON in Sprint 14 ; tonic in 15.
- **WebSocket / SSE streaming** — Sprint 15 (Claude streaming).
- **OAuth 2.1 / OIDC** — Sprint 15 ; Sprint 14 uses raw JWT.
- **Redis-backed rate limit + idempotency** — Sprint 15 ; Sprint
  14 in-memory only (single-instance scalability).
- **mTLS service-to-service** — Sprint 15.
- **API key management UI** — Sprint 15 ; Sprint 14 JWT only.
- **Real-time webhooks** — Sprint 15.

---

## 1. Context, motivation, sprint placement

### 1.1 Five real incidents driving N22

**Incident I-2026-09-02 — Sales pilot blocked by zero HTTP.**
After Sprint 13 N21 demo, three pilot customers (Acme, Globex,
NovaCorp) asked to run their PDF contract analyser through EFE.
Each customer expected to POST a PDF + JWT to a URL.  No URL
exists.  Sales lost 11 days of pilot momentum waiting for "the
HTTP layer".  Estimated lost-deal value : $180 k ARR.

**Incident I-2026-09-08 — Internal ops team stuck on `cargo run`.**
Operators rotating publisher keys, approving gold sets, triggering
drift sweeps had to SSH into a deploy server and run
`cargo run --bin efe-publishers …`.  One key rotation took 47
minutes because of binary rebuild + Vault env vars + tmux session.
The same operation through an HTTP endpoint would take 3 seconds.

**Incident I-2026-09-15 — Compliance audit demanded "show me the
API"**.  EU regulator asked to see the API documentation EFE
exposes to customers.  We had no OpenAPI spec.  Compliance team
produced a hand-written PDF that was already obsolete before the
audit meeting.  EU regulator flagged this as a P1 documentation
gap : "any production API service in the IA Act scope MUST
publish a machine-readable spec".

**Incident I-2026-09-19 — Slack integration impossible without
webhook URL.**  Customer-success wanted a Slack slash command
`/efe summarize <pdf-url>` that hits EFE.  No URL = no slash
command.  Engineering team spent 4 days writing a custom Lambda
wrapper that mimicked HTTP by shelling out to `cargo run` (slow,
brittle, security nightmare).

**Incident I-2026-09-25 — K8s deployment impossible.**  Platform
team tried to deploy EFE to the staging cluster.  Required K8s
manifests need `readinessProbe` + `livenessProbe` HTTP endpoints.
None exist.  Platform team rolled their own `nc -l` shell-script
"server".  Predictably crashed under load.

### 1.2 Why Sprint 14, immediately after N21

N21 (Sprint 13) shipped the **library** capabilities that *deserve*
to be exposed.  Without an HTTP layer in Sprint 14, every Sprint
8–13 investment ($580 k engineering cost) is locked behind a
`cargo` command line.  The HTTP layer is the **single integration
point** through which all customer-facing value flows.

Sprint 14 ordering :

| Day | Deliverable | Depends on |
|---|---|---|
| 56 | Crate skeleton + router + auth + tracing + RFC 7807 errors | None (axum + tower-http) |
| 57 | Swarm + Agentic endpoints | efe-swarm-orchestrator, efe-agentic-bridge |
| 58 | Publisher + Gold + Replay endpoints | efe-publisher-identity, efe-gold-test-gate, efe-multi-region |
| 59 | Health + Metrics + OpenAPI + Idempotency | tracing, utoipa, tower-governor |
| 60 | Integration tests + binary release + Docker + K8s manifests | None (assembly) |

### 1.3 Who consumes N22 once it ships

| Consumer | Use case |
|---|---|
| **Pilot customers** | POST PDF + JWT → get structured analysis from Claude via swarm |
| **Operators / SRE** | Rotate publisher keys, approve gold sets, inspect drift state from web UI / curl |
| **Sales engineering** | Demo URLs that work in 30 seconds (`curl -X POST …`) |
| **Compliance team** | OpenAPI spec auto-served → regulator-ready |
| **Slack / Salesforce integrations** | Standard webhook target |
| **K8s platform team** | Standard readiness/liveness probes |
| **Prometheus operator** | `/metrics` endpoint scraped every 15 s |
| **External auditors** | Trace ID propagation visible end-to-end |
| **DevOps tooling (Terraform, ArgoCD)** | Declarative API surface |
| **Internal tools team (chatops, JIRA bot)** | API-driven workflows |

### 1.4 The 15 incident-class patterns (preview)

Section 2 enumerates 15 patterns clustered into 5 families :

* **Integration & deployability** (P1, P2, P3) — 3 patterns
* **Auth & multi-tenancy** (P4, P5, P6) — 3 patterns
* **Reliability & DoS** (P7, P8, P9) — 3 patterns
* **Observability & forensics** (P10, P11, P12) — 3 patterns
* **Standards compliance** (P13, P14, P15) — 3 patterns

---

## 2. Fifteen incident patterns N22 closes (P1–P15)

### Family A — Integration & deployability

**P1. No HTTP endpoint = no external integration.**
Incidents I-2026-09-02 (sales pilot), I-2026-09-19 (Slack
integration), I-2026-09-25 (K8s deployment).  N22 ships a
production-ready HTTPS server reachable via standard URL.

**P2. No K8s readiness probe = no rolling deploy.**
Without `/readyz`, K8s can't tell if a new pod is ready to take
traffic.  Rolling deploys silently break.  N22 ships `/readyz`
that returns 503 until all dependencies (Postgres pools, embedding
provider breakers) are healthy.

**P3. No `/metrics` endpoint = blind production.**
Prometheus operator needs `/metrics` scrape target.  We have
N18/N19/N20/N21 metrics in-process but no surface.  N22 exposes
the aggregated Prometheus surface.

### Family B — Auth & multi-tenancy

**P4. No auth = anyone can run swarms.**
N21 lib has no caller authentication.  N22 enforces JWT bearer
auth on every `/v1/*` endpoint.

**P5. No tenant isolation = data leak risk.**
Without `tenant_id` enforcement, customer A could trigger swarms
under customer B's quotas.  N22 enforces `jwt.tenant_id ==
request.body.tenant_id` (or 403).

**P6. No rate limiting = single tenant DoS-es the cluster.**
A buggy customer loop calling `/v1/swarms/run` 10 000×/sec
saturates Claude quota.  N22 ships per-tenant rate limit (100 req/s
default, configurable).

### Family C — Reliability & DoS

**P7. No body size cap = OOM via massive PDF upload.**
Customer uploads 5 GB PDF as base64 in JSON body → process OOM.
N22 enforces 10 MB body cap before parsing.

**P8. No connection cap = file-descriptor exhaustion.**
Slowloris-style attack opens 100 000 idle connections.  N22 caps
concurrent connections at 10 000 (configurable).

**P9. No graceful shutdown = in-flight requests killed.**
K8s `SIGTERM` arrives mid-request → request 500s on the customer.
N22 catches SIGTERM/SIGINT, stops accepting new requests, waits
≤ 30 s for in-flight to complete, then exits cleanly.

### Family D — Observability & forensics

**P10. No `trace_id` propagation = audit chain broken.**
N14 envelopes have `trace_id` but the HTTP layer doesn't generate
or propagate one.  N22 implements W3C Trace Context : extract
`traceparent` if present, else generate ; inject into every log
line + downstream call + N17 record.

**P11. No structured logging = grep-driven debugging.**
Production support spends hours grep-ing through ad-hoc logs.
N22 uses `tracing` crate with `tracing-subscriber` JSON output ;
every log carries `trace_id`, `tenant_id`, `path`, `status`,
`duration_ms`.

**P12. No request-id-to-record join.**
Auditor asks "show me the LLM provenance for request X" — no
mapping exists between HTTP request and N17 record.  N22 propagates
`trace_id` into every N17 `LlmProvenanceRecord` so the join is
deterministic.

### Family E — Standards compliance

**P13. No OpenAPI = regulator unhappy.**
Incident I-2026-09-15.  N22 auto-generates OpenAPI 3.1 spec from
endpoint handlers (utoipa) ; serves at `/openapi.json` + Swagger
UI at `/docs`.

**P14. Custom error shapes = client libraries break per endpoint.**
Each subsystem (N18, N19, etc.) returns its own error shape.
External SDK generators (openapi-generator-cli) can't normalise.
N22 unifies all errors into RFC 7807 Problem Details JSON.

**P15. No TLS termination ownership = LB misconfig risk.**
"TLS is the LB's problem" attitude leaves the door open for
plaintext bypass.  N22 ships rustls TLS termination in-process
(opt-in flag) ; deployment can still front with envoy/nginx but
the option exists for direct-exposure scenarios.

---

## 3. Commitments — what ships in Sprint 14 (G1–G15)

> **Read convention** : `G-N : commitment / impl surface / acceptance test`.

### Binary & deployment

**G1. `efe-gateway` self-contained binary.**
Single statically-linked Linux/macOS executable.  Configured via
env vars + optional TOML config file.  Acceptance :
`phase_7_n22_binary_starts_and_serves_healthz`.

**G2. Docker image + K8s manifests.**
`docker/Dockerfile.gateway` (distroless base), `deploy/k8s/efe-
gateway.yaml` (Deployment + Service + ServiceMonitor + HPA).
Acceptance :
`phase_7_n22_docker_image_runs_and_passes_healthz` (CI-gated).

### Routing & middleware

**G3. axum 0.8 router with all 21 endpoints.**
`RouterBuilder::build()` returns a single `Router` wired with
every subsystem's endpoint group.  Acceptance :
`phase_7_n22_router_serves_all_21_endpoints`.

**G4. Tower middleware stack (in correct order).**
`TraceLayer` → `BodyLimitLayer` → `RequestIdLayer` → `AuthLayer`
→ `TenantIsolationLayer` → `RateLimitLayer` → `IdempotencyLayer`
→ `OpenApiResponseLayer`.  Order matters (trace_id BEFORE auth so
401s are still traced).  Acceptance :
`phase_7_n22_middleware_stack_order_correct`.

### Auth

**G5. JWT bearer authentication (HS256 + RS256).**
HS256 for dev/test ; RS256+JWKS for production.  Acceptance :
`phase_7_n22_jwt_hs256_validated`, `phase_7_n22_jwt_rs256_jwks_
validated`.

**G6. Tenant isolation enforcement.**
Every `POST` body with a `tenant_id` field is checked against
`jwt.tenant_id` ; 403 on mismatch.  Acceptance :
`phase_7_n22_cross_tenant_post_rejected_403`.

### Error & response

**G7. RFC 7807 Problem Details on every 4xx/5xx.**
`application/problem+json` with `type`, `title`, `status`,
`detail`, `instance`, `trace_id`.  Acceptance :
`phase_7_n22_4xx_5xx_rfc_7807_compliant`.

**G8. RFC 7234-compatible caching headers** on idempotent reads
(`Cache-Control`, `ETag` for `GET /v1/publishers/{id}`).
Acceptance :
`phase_7_n22_etag_returned_on_lookup`.

### Observability

**G9. Prometheus `/metrics` endpoint.**
Aggregates N18-N21 metric surfaces + N22 HTTP-specific (request_
count, request_duration_seconds, body_bytes, in_flight).
Acceptance : `phase_7_n22_metrics_endpoint_exposition_format`.

**G10. W3C Trace Context (traceparent + tracestate).**
Acceptance : `phase_7_n22_traceparent_header_propagated`.

**G11. Structured JSON logging via `tracing-subscriber`.**
Every log line is one-line JSON ; carries `trace_id`, `path`,
`status`, `duration_ms`, `tenant_id`.  Acceptance :
`phase_7_n22_log_lines_are_json`.

### Standards compliance

**G12. OpenAPI 3.1 spec via utoipa.**
Auto-generated from handler annotations ; served at
`/openapi.json`.  Swagger UI at `/docs`.  Acceptance :
`phase_7_n22_openapi_spec_lists_all_endpoints`.

**G13. K8s readiness + liveness probes.**
`/healthz` (liveness — always 200 if process alive ; 503 only on
deadlock) ; `/readyz` (readiness — 503 until all deps reachable).
Acceptance : `phase_7_n22_readiness_returns_503_when_postgres_
unreachable`.

### Reliability

**G14. Graceful shutdown on SIGTERM/SIGINT.**
Stop accepting new connections ; wait ≤ 30 s for in-flight ; exit
0.  Acceptance : `phase_7_n22_graceful_shutdown_completes_within_
30s`.

**G15. Per-tenant rate limit via tower-governor.**
100 req/s default per `tenant_id` ; configurable.  Acceptance :
`phase_7_n22_rate_limit_429_after_threshold`.

---

## 4. Invariants — non-negotiables (I1–I20)

### Request lifecycle

**I1. Every request has a `trace_id`.**  Generated by N22 if no
`traceparent` header is present ; W3C-compliant 16-byte hex.

**I2. Auth is required on `/v1/*` except `/v1/admin` is operator-
only (different claim).**  `/healthz`, `/readyz`, `/metrics`,
`/openapi.json`, `/docs` are public.

**I3. Tenant isolation is enforced at the middleware level.**
Handlers receive a `TenantContext` extension already validated ;
they cannot accidentally cross-tenant.

**I4. `Idempotency-Key` header de-duplicates retries within 24 h.**
Optional ; if absent, no dedup ; if present + same key seen in
24 h, return cached response.

**I5. All response bodies are JSON UTF-8.**  No XML, no msgpack,
no protobuf in Sprint 14.

**I6. Body cap is 10 MB.**  Enforced BEFORE JSON parsing (Sprint
14) ; before any expensive work.

**I7. URL paths contain no PII.**  IDs are UUIDs (opaque).
Customer names, emails, PDF contents go in body only.

**I8. Concurrent connections capped at 10 000.**  Configurable
via `EFE_GATEWAY_MAX_CONN`.

### Error & response

**I9. Every 4xx/5xx is RFC 7807.**  No naked text errors.

**I10. `trace_id` appears in every error body** (as `trace_id`
field at top level of the Problem Details body).

**I11. Errors NEVER leak internal stack traces** to the customer.
Operator audit log gets the stack ; customer gets only the
public-safe `detail`.

**I12. Sensitive request fields are NOT logged** (Authorization
header, body fields tagged `secret`).  Tracing's `skip(...)` is
mandatory.

### Multi-tenancy

**I13. Tenant claim in JWT.**  `jwt.tenant_id` (UUID) MUST be
present ; missing claim → 401.

**I14. Tenant-scoped resources cannot be accessed cross-tenant.**
A publisher / gold set / replay job owned by tenant A cannot be
read by tenant B even with valid JWT.  Enforced at the handler
layer (Sprint 14) ; Sprint 15 moves to Postgres RLS too.

### Observability

**I15. `/metrics` is Prometheus exposition format 0.0.4.**  No
JSON, no protobuf.

**I16. `/healthz` returns 200 even under load** — it only fails
if the process is in a deadlock state (i.e. it never fails).

**I17. `/readyz` returns 503 if any critical dep is unreachable.**
Critical : Postgres pool, embedding provider breakers (Sprint 15).

**I18. Structured logs are one-line JSON.**  No multi-line stack
traces in stdout ; those go to a separate `error_chain` field.

### Reliability

**I19. Graceful shutdown : maximum 30 s drain.**  After 30 s,
remaining in-flight requests are forcefully aborted ; process
exits with code 124 (timeout).

**I20. No request-handler panic propagates to crash the server.**
`tower::catch_panic` wraps every handler ; a panic returns 500
RFC 7807 with `detail: "internal server error"` and the stack in
the operator audit log.

---

## 5. Failure modes — what we PROTECT against (F1–F15)

### Auth & tenancy

**F1. Missing JWT.**
* *Detection* : `AuthLayer` reads `Authorization` header.
* *Response* : 401 with `type: /errors/missing-auth` + WWW-
  Authenticate header.

**F2. Invalid JWT signature.**
* *Detection* : signature verification fails.
* *Response* : 401 `/errors/invalid-jwt`.

**F3. Expired JWT.**
* *Detection* : `exp < now()`.
* *Response* : 401 `/errors/expired-jwt`.

**F4. Tenant mismatch.**
* *Detection* : `jwt.tenant_id != body.tenant_id`.
* *Response* : 403 `/errors/tenant-isolation-violation`.

### Rate limiting & body

**F5. Rate limit exceeded.**
* *Detection* : `tower-governor` returns `Err(GovernorError)`.
* *Response* : 429 `/errors/rate-limited` + `Retry-After`
  header.

**F6. Body too large.**
* *Detection* : `tower-http::limit::RequestBodyLimitLayer` cuts at
  10 MB.
* *Response* : 413 `/errors/payload-too-large`.

**F7. Body malformed JSON.**
* *Detection* : `axum::Json` extractor fails.
* *Response* : 400 `/errors/bad-json` with parse error position.

### Idempotency

**F8. Idempotency-Key reused with different body.**
* *Detection* : key found in cache, body hash differs.
* *Response* : 422 `/errors/idempotency-conflict`.

**F9. Idempotency cache full.**
* *Detection* : in-memory cache LRU evicts.
* *Response* : non-fatal ; cache misses ; request proceeds.

### Downstream

**F10. Postgres unreachable mid-call.**
* *Detection* : `sqlx::Error::Pool*` errors.
* *Response* : 503 `/errors/dependency-unavailable` ; `Retry-
  After` header.

**F11. Claude API rate-limited.**
* *Detection* : `BridgeError::RateLimited` from agentic bridge.
* *Response* : 503 `/errors/upstream-rate-limited`.

**F12. Swarm timeout.**
* *Detection* : `SwarmRunError::GlobalTimeout`.
* *Response* : 504 `/errors/swarm-timeout`.

### Server lifecycle

**F13. SIGTERM during in-flight.**
* *Detection* : `tokio::signal::ctrl_c` future.
* *Response* : stop accepting new ; drain ≤ 30 s ; exit 0 (or
  124 if forced).

**F14. Handler panic.**
* *Detection* : `tower::catch_panic`.
* *Response* : 500 `/errors/internal-server-error` ; stack in
  audit log.

**F15. TLS handshake failure.**
* *Detection* : `rustls` connection rejected.
* *Response* : connection dropped ; metric `n22_tls_handshake_
  failures_total` ticked.

---

## 6. Architecture Decision Records (ADR-1 to ADR-10)

### ADR-1. axum 0.8 + tower-http (vs actix vs tonic-only).

**Context.**  Choice of HTTP framework.

**Decision.**  axum 0.8 + tower-http (already used by
`efe-meta-swarm-server` and other workspace crates) ; tower
middleware stack for everything cross-cutting.

**Rationale.**
- **Workspace consistency** : already in use elsewhere ; no extra
  dep tree.
- **tower ecosystem** : `tower-http`, `tower-governor`, `tower::
  timeout`, `tower::catch_panic` are battle-tested.
- **OpenAPI** : `utoipa` integrates cleanly with axum.

**Rejected alternatives.**
- **actix-web** : different runtime model (actor) ; would require
  bridging to tokio.
- **tonic-only** : excludes browsers ; SaaS customers expect
  REST/JSON.

---

### ADR-2. JWT HS256 dev / RS256+JWKS prod.

**Context.**  Authentication scheme.

**Decision.**  HS256 (shared secret) for dev/test/CI.  RS256 with
JWKS endpoint for production.  Selected at boot via
`EFE_GATEWAY_AUTH=hs256|rs256_jwks` env var.

**Rationale.**
- HS256 simplicity : single shared secret, easy CI setup, fast.
- RS256+JWKS : key rotation without restarting the gateway,
  standard SaaS pattern.
- Sprint 15 adds OAuth 2.1 / OIDC.

---

### ADR-3. RFC 7807 Problem Details on every error.

**Context.**  Error response format.

**Decision.**  `application/problem+json` everywhere ; structured
fields :

```json
{
  "type": "/errors/<slug>",
  "title": "Human-readable category",
  "status": 4xx_or_5xx,
  "detail": "Specific situation description",
  "instance": "/v1/swarms/run",
  "trace_id": "0123456789abcdef...",
  "extensions": {}
}
```

**Rationale.**
- Industry standard ; openapi-generator-cli understands.
- `trace_id` join : auditor can find the full call chain.

---

### ADR-4. utoipa for OpenAPI 3.1 generation.

**Context.**  How OpenAPI spec is produced.

**Decision.**  `utoipa` annotations on every handler.  `/openapi.
json` serves the result.  `/docs` serves Swagger UI via
`utoipa-swagger-ui`.

**Rationale.**
- Compile-time : annotations are checked by rustc, can't drift.
- Industry-standard output.

**Rejected alternatives.**
- Hand-written YAML : drifts within a week of any code change.
- `paperclip` : less actively maintained.

---

### ADR-5. W3C Trace Context (vs custom).

**Context.**  Trace propagation.

**Decision.**  W3C Trace Context — `traceparent` + `tracestate`
headers ; spec-compliant.

**Rationale.**
- Auditor compatibility : OpenTelemetry, Jaeger, Datadog,
  Tempo all consume W3C.
- Cross-region replay (N16) needs interoperable trace_id.

---

### ADR-6. Per-tenant rate limit via tower-governor (in-memory
Sprint 14, Redis Sprint 15).

**Context.**  Where rate-limit state lives.

**Decision.**  Sprint 14 : in-memory `tower-governor` per pod (no
cross-pod sharing).  Sprint 15 : Redis-backed via `governor`
extension.

**Rationale.**
- Sprint 14 : single-pod deploy, in-memory is fine, ships fast.
- Sprint 15 : multi-pod requires shared state.

---

### ADR-7. Body size cap : 10 MB hard, enforced before parsing.

**Context.**  DoS protection on bodies.

**Decision.**  `tower-http::limit::RequestBodyLimitLayer` at 10 MB.
Configurable via `EFE_GATEWAY_BODY_LIMIT_BYTES` env var ; cap
between 1 KB and 100 MB.

**Rationale.**
- Most legitimate bodies (PDF base64) < 5 MB.
- 10 MB covers the long tail without OOM risk.

---

### ADR-8. Graceful shutdown 30 s budget.

**Context.**  How long to wait for in-flight on SIGTERM.

**Decision.**  30 s drain budget ; forced exit at code 124 after.

**Rationale.**
- K8s default `terminationGracePeriodSeconds: 30` ; matches.
- Most EFE requests complete < 5 s.

---

### ADR-9. Connection cap via axum::serve config (Sprint 14).

**Context.**  File-descriptor exhaustion.

**Decision.**  `axum::serve` with `tokio::net::TcpListener`
behind `tower::limit::ConcurrencyLimit(10_000)` middleware.

**Rationale.**
- 10 k connections fits in 64 GB RAM with healthy headroom.
- Sprint 15 may move to per-IP cap too.

---

### ADR-10. TLS via rustls (vs OpenSSL).

**Context.**  TLS termination.

**Decision.**  rustls 0.23 (pure Rust, no OpenSSL dep).  Opt-in
via `EFE_GATEWAY_TLS_CERT=path EFE_GATEWAY_TLS_KEY=path` env vars.

**Rationale.**
- Memory safety guarantees of pure Rust.
- Already used by reqwest in N20 (rustls-tls feature).
- Deployments behind nginx/envoy can disable.

---

## 7. Component architecture

### 7.1 The 14 components at a glance

```text
                            HTTPS
                              │
                              ▼
            ┌──────────────────────────────────┐
            │  axum::Server (TLS optional)     │
            │   - rustls 0.23 if enabled       │
            │   - max_conn = 10 000             │
            │   - graceful shutdown on signal  │
            └────────────────┬─────────────────┘
                             │
                             ▼
   ┌──────────────────────────────────────────────────────┐
   │  Middleware stack (tower) :                          │
   │  1. TraceLayer (extract/inject traceparent)          │
   │  2. RequestIdLayer (generate UUID)                   │
   │  3. BodyLimitLayer (10 MB)                           │
   │  4. AuthLayer (JWT verify, 401 on fail)              │
   │  5. TenantIsolationLayer (403 on mismatch)           │
   │  6. RateLimitLayer (tower-governor)                  │
   │  7. IdempotencyLayer (key + body-hash dedup)         │
   │  8. CatchPanicLayer (500 on handler panic)           │
   │  9. ProblemDetailsLayer (RFC 7807 conversion)        │
   └────────────────┬─────────────────────────────────────┘
                    │
                    ▼
   ┌────────────────────────────────────────────────────┐
   │  Handler dispatch :                                │
   │   /healthz       /readyz       /metrics            │
   │   /openapi.json  /docs                             │
   │   /v1/swarms/*  ─────────► efe-swarm-orchestrator  │
   │   /v1/agentic/* ─────────► efe-agentic-bridge      │
   │   /v1/publishers/* ──────► efe-publisher-identity  │
   │   /v1/gold-* ────────────► efe-gold-test-gate      │
   │   /v1/replays/* ─────────► efe-multi-region        │
   │   /v1/provenance/* ──────► efe-llm-provenance      │
   │   /v1/admin/* ───────────► (operator only)         │
   └────────────────────────────────────────────────────┘
```

### 7.2 `efe-gateway` crate layout

```text
crates/efe-gateway/
├── src/
│   ├── lib.rs                   # re-exports
│   ├── main.rs                  # binary entrypoint
│   ├── config.rs                # env + TOML config
│   ├── router.rs                # RouterBuilder
│   ├── state.rs                 # AppState holding all subsystem Arcs
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── jwt.rs               # HS256 + RS256 verification
│   │   ├── claims.rs            # tenant_id, operator scope
│   │   └── layer.rs             # AuthLayer middleware
│   ├── tenancy/
│   │   ├── mod.rs
│   │   └── layer.rs             # TenantIsolationLayer
│   ├── error/
│   │   ├── mod.rs
│   │   ├── problem_details.rs   # RFC 7807 type
│   │   └── conversions.rs       # Subsystem-error → ProblemDetails
│   ├── tracing_layer/
│   │   ├── mod.rs
│   │   └── w3c.rs               # traceparent + tracestate
│   ├── rate_limit/
│   │   └── mod.rs               # tower-governor wiring
│   ├── idempotency/
│   │   ├── mod.rs
│   │   └── cache.rs             # LRU in-memory
│   ├── observability/
│   │   ├── mod.rs
│   │   ├── metrics.rs           # /metrics endpoint
│   │   ├── healthz.rs           # /healthz
│   │   ├── readyz.rs            # /readyz
│   │   └── tracing.rs           # tracing-subscriber JSON
│   ├── openapi/
│   │   ├── mod.rs
│   │   ├── spec.rs              # utoipa derive
│   │   └── docs.rs              # Swagger UI route
│   ├── handlers/
│   │   ├── mod.rs
│   │   ├── swarms.rs            # POST /v1/swarms/run + GET
│   │   ├── agentic.rs           # POST /v1/agentic/call
│   │   ├── publishers.rs        # CRUD + rotate-key + revoke
│   │   ├── gold.rs              # gold-sets + gold-runs
│   │   ├── replays.rs           # POST + GET
│   │   ├── provenance.rs        # GET only
│   │   └── admin.rs             # circuit-breakers, drift-sweep
│   ├── tls.rs                   # rustls integration
│   └── shutdown.rs              # graceful shutdown future
├── Cargo.toml
├── tests/
│   ├── phase_7_n22_*.rs         # 15 acceptance tests
│   ├── audit_n22_*.rs           # 20 audit tests
│   └── integration/
│       └── e2e_flows.rs         # full pilot workflows
└── deploy/
    ├── docker/
    │   └── Dockerfile.gateway
    └── k8s/
        ├── deployment.yaml
        ├── service.yaml
        ├── servicemonitor.yaml
        └── hpa.yaml
```

### 7.3 `AppState` — the dependency carrier

```rust
pub struct AppState {
    // Subsystem orchestrators
    pub swarm: Arc<SwarmOrchestrator>,
    pub agentic: Arc<dyn AgenticBridge>,

    // Subsystem APIs
    pub publisher_api: Arc<dyn PublisherApi>,
    pub gold_api: Arc<dyn GoldApi>,
    pub replay_coord: Arc<ReplayCoordinator>,
    pub provenance_store: Arc<dyn ProvenanceStore>,

    // Cross-cutting
    pub metrics: Arc<GatewayMetrics>,
    pub jwt_validator: Arc<dyn JwtValidator>,
    pub idempotency_cache: Arc<IdempotencyCache>,
}
```

### 7.4 Middleware stack (order matters)

```rust
pub fn build_router(state: AppState) -> Router {
    Router::new()
        // Public routes (no auth)
        .route("/healthz", get(handlers::healthz))
        .route("/readyz", get(handlers::readyz))
        .route("/metrics", get(handlers::metrics))
        .route("/openapi.json", get(handlers::openapi_spec))
        .route("/docs", get(handlers::swagger_ui))
        // Authenticated v1 routes
        .nest("/v1", v1_router(&state))
        .with_state(state)
        // OUTER → INNER middleware stack
        .layer(ProblemDetailsLayer::new())
        .layer(CatchPanicLayer::new())
        .layer(RateLimitLayer::per_tenant(100, Duration::from_secs(1)))
        .layer(TenantIsolationLayer::new())
        .layer(AuthLayer::new(state.jwt_validator.clone()))
        .layer(BodyLimitLayer::new(10 * 1024 * 1024))
        .layer(RequestIdLayer::new())
        .layer(TraceLayer::new_for_http())
}
```

### 7.5 N22 metrics surface

| # | Name | Type | Labels |
|---|---|---|---|
| 1 | `n22_http_requests_total` | counter | `method`, `path`, `status` |
| 2 | `n22_http_request_duration_seconds` | histogram | `method`, `path` |
| 3 | `n22_http_in_flight_requests` | gauge | — |
| 4 | `n22_http_request_body_bytes` | summary | `method`, `path` |
| 5 | `n22_http_response_body_bytes` | summary | `method`, `path` |
| 6 | `n22_auth_failures_total` | counter | `reason` |
| 7 | `n22_rate_limit_rejections_total` | counter | `tenant_tier` |
| 8 | `n22_idempotency_cache_hits_total` | counter | `outcome` |
| 9 | `n22_tls_handshake_failures_total` | counter | `reason` |
| 10 | `n22_graceful_shutdowns_total` | counter | `outcome` |
| 11 | `n22_panic_recoveries_total` | counter | `handler` |
| 12 | `n22_concurrent_connections` | gauge | — |

---

## 8. Module hierarchy & data model

### 8.1 Crate dependencies

```text
efe-gateway
├── efe-nano-sdk
├── efe-publisher-identity
├── efe-gold-test-gate
├── efe-multi-region
├── efe-llm-provenance
├── efe-blackboard
├── efe-swarm-orchestrator
├── efe-agentic-bridge
├── efe-nano-preanalysis
│
├── axum            = "0.8"
├── tower           = "0.5"
├── tower-http      = "0.6" (features = ["trace", "limit", "request-id", "compression-gzip", "catch-panic"])
├── tower-governor  = "0.5"
├── utoipa          = "5.3"
├── utoipa-swagger-ui = "8.0"
├── tracing-subscriber = "0.3" (features = ["json", "env-filter"])
├── jsonwebtoken    = "9.3"
├── prometheus      = "0.13"
└── rustls          = "0.23"
```

### 8.2 Config schema

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    pub bind: String,                            // "0.0.0.0:8080"
    pub tls: Option<TlsConfig>,
    pub auth: AuthConfig,
    pub rate_limit: RateLimitConfig,
    pub body_limit_bytes: usize,                  // default 10 MB
    pub max_concurrent_conn: usize,               // default 10 000
    pub graceful_shutdown_secs: u64,              // default 30
    pub log_level: String,                        // "info"
    pub log_format: LogFormat,                    // Json | Pretty
    pub idempotency_cache_size: usize,            // default 10 000
    pub idempotency_ttl_secs: u64,                // default 86 400 (24 h)
}

pub struct TlsConfig {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

pub enum AuthConfig {
    Hs256 { secret_b64: String },
    Rs256Jwks {
        jwks_url: String,
        issuer: String,
        audience: String,
    },
}

pub struct RateLimitConfig {
    pub per_tenant_per_second: u32,               // default 100
    pub burst: u32,                                // default 200
}
```

---

## 9. API surface — full endpoint catalogue

> 21 endpoints organised by family.  Every endpoint has utoipa
> annotations producing OpenAPI 3.1.  Every error path returns
> RFC 7807.

### 9.1 Health & meta (5 — public, no auth)

| Method | Path | Handler | Response |
|---|---|---|---|
| GET | `/healthz` | `healthz` | 200 always (process alive) |
| GET | `/readyz` | `readyz` | 200 if all deps OK ; 503 otherwise |
| GET | `/metrics` | `metrics` | Prometheus text 0.0.4 |
| GET | `/openapi.json` | `openapi_spec` | OpenAPI 3.1 JSON |
| GET | `/docs` | `swagger_ui` | HTML Swagger UI |

### 9.2 Swarm (3 — auth required)

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/v1/swarms/run` | `{ dag_id, input, config }` | 200 `SwarmReport` + `ContextBundle` |
| POST | `/v1/swarms/dags` | `SwarmDagDescription` | 201 `{ dag_id }` |
| GET | `/v1/swarms/{swarm_id}` | — | 200 `SwarmReport` |

### 9.3 Agentic (1 — auth required)

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/v1/agentic/call` | `{ bundle, task }` | 200 `AgenticResponse` |

### 9.4 Publishers (5 — N18 — auth required)

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/v1/publishers` | `RegisterRequest` | 201 `PublisherIdentity` |
| GET | `/v1/publishers/{id}` | — | 200 `PublisherIdentity` |
| POST | `/v1/publishers/{id}/rotate-key` | `RotateKeyRequest` | 200 `RotateKeyResponse` |
| POST | `/v1/publishers/{id}/revoke` | `RevokeRequest` | 200 |
| POST | `/v1/publishers/{id}/promote` | `PromoteRequest` | 200 |

### 9.5 Gold (4 — N19 — auth required)

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/v1/gold-sets` | `RegisterGoldSetRequest` | 201 `{ gold_set_id }` |
| POST | `/v1/gold-sets/{id}/approve` | `ApproveGoldSetRequest` | 200 `GoldSet` |
| POST | `/v1/gold-runs` | `RunRequest` | 200 `GoldReport` |
| GET | `/v1/gold-runs/{id}` | — | 200 `GoldReport` |

### 9.6 Replay (2 — N16 — auth required)

| Method | Path | Body | Response |
|---|---|---|---|
| POST | `/v1/replays` | `ReplayRequest` | 202 `{ job_id }` |
| GET | `/v1/replays/{id}` | — | 200 `ReplayResult` |

### 9.7 Provenance (1 — N17 — auth required)

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/v1/provenance/{id}` | — | 200 `LlmProvenanceRecord` |

### 9.8 Admin (3 — operator-scope JWT)

| Method | Path | Body | Response |
|---|---|---|---|
| GET | `/v1/admin/circuit-breakers` | — | 200 list of states |
| POST | `/v1/admin/circuit-breakers/{provider}/reset` | — | 200 |
| POST | `/v1/admin/drift-sweep/trigger` | — | 202 `{ batch_id }` |

### 9.9 Standard error response (RFC 7807)

```json
HTTP/1.1 403 Forbidden
Content-Type: application/problem+json

{
  "type": "https://docs.efe.ai/errors/tenant-isolation-violation",
  "title": "Tenant Isolation Violation",
  "status": 403,
  "detail": "JWT tenant_id (abc-123) does not match body.tenant_id (xyz-789)",
  "instance": "/v1/swarms/run",
  "trace_id": "0123456789abcdef0123456789abcdef",
  "extensions": {
    "jwt_tenant_id": "abc-123",
    "body_tenant_id": "xyz-789"
  }
}
```

---

## 10. End-to-end workflows

### 10.1 Workflow A — Customer pilot : PDF contract analysis

```
1. Customer holds JWT(tenant_id=acme).
2. POST /v1/swarms/run
   Headers : Authorization: Bearer eyJ..., Idempotency-Key: <uuid>
   Body : { dag_id: "pdf-analyser",
            input: { tenant_id: "acme", text: "<pdf base64>" } }
3. Middleware :
   - TraceLayer : extract traceparent, generate if absent
   - BodyLimit : OK (< 10 MB)
   - Auth : verify JWT signature, exp, issuer
   - Tenant : body.tenant_id == jwt.tenant_id → OK
   - RateLimit : tenant under 100 req/s → OK
   - Idempotency : key not in cache → register
4. Handler : SwarmOrchestrator.run(dag, input, config)
   - 7 agents (4 layers) → ContextBundle
   - ClaudeBridge.call_with_context(bundle, task)
5. Response : 200 OK SwarmReport + ContextBundle (JSON)
6. Idempotency cache stores response for 24 h.
```

### 10.2 Workflow B — Operator approves a gold set

```
1. Operator holds JWT(scope=operator).
2. POST /v1/gold-sets/{id}/approve
   Body : { approvers: [{ operator_id, signature, ... }, ...] }
3. Middleware :
   - Auth → JWT OK
   - Tenant → operator scope, no tenant_id needed
4. Handler : GoldApi.approve_gold_set(id, req)
   - Verifies ≥ 2 distinct operator signatures
   - Updates registry to Approved
   - Appends ledger row
5. Response : 200 OK GoldSet
```

### 10.3 Workflow C — K8s rolling deploy

```
1. K8s creates new pod ; runs efe-gateway binary.
2. Process starts ; binds 0.0.0.0:8080.
3. K8s probes GET /healthz → 200 immediately.
4. K8s probes GET /readyz :
   - Pool ping Postgres → OK or 503
   - Loop until 200.
5. K8s redirects traffic to new pod.
6. Old pod receives SIGTERM.
7. shutdown.rs :
   - Stop accepting new connections.
   - Wait up to 30 s for in-flight to complete.
   - Exit code 0.
```

### 10.4 Workflow D — Forensic audit by trace_id

```
1. Regulator asks : "show me the call chain for trace_id X".
2. Operator runs :
   GET /v1/admin/audit?trace_id=X
   (Sprint 15 endpoint — Sprint 14 uses Postgres SQL directly).
3. SQL join across :
   - HTTP log lines (Loki/CloudWatch)
   - N17 provenance store (filter by trace_id)
   - N18 publish ledger
   - N19 gold-run ledger
   - N14 envelope chain
4. Result : full call chain with cryptographic proof.
```

### 10.5 Workflow E — Rate-limited burst

```
1. Buggy customer code loops POST /v1/swarms/run.
2. First 100 requests in 1 s : all 200 OK.
3. Request 101 : 429 Too Many Requests
   Headers : Retry-After: 1
   Body : RFC 7807 /errors/rate-limited
4. Client backs off ; retries after 1 s ; gets 200.
```

---

## 11. Five-day implementation plan

### Day 56 — Crate skeleton + middleware stack
* `efe-gateway` crate (lib + bin) ; deps wired.
* `Config` + env-var loading + TOML support.
* `RouterBuilder::build()` skeleton.
* Middleware : Trace, RequestId, BodyLimit, Auth (HS256), Tenant,
  RateLimit (in-mem), CatchPanic, ProblemDetails.
* Handlers : healthz, readyz, metrics (stub).
* 8+ unit tests (auth flow + middleware order).

### Day 57 — Swarm + Agentic endpoints
* `handlers/swarms.rs` : 3 endpoints.
* `handlers/agentic.rs` : 1 endpoint.
* AppState wiring through axum extensions.
* utoipa annotations on handlers.
* 6+ integration tests via `axum-test`.

### Day 58 — Publisher + Gold + Replay + Provenance endpoints
* `handlers/publishers.rs` : 5 endpoints.
* `handlers/gold.rs` : 4 endpoints.
* `handlers/replays.rs` : 2 endpoints.
* `handlers/provenance.rs` : 1 endpoint.
* `handlers/admin.rs` : 3 endpoints.
* 10+ integration tests.

### Day 59 — Idempotency + RS256+JWKS + OpenAPI + observability
* `idempotency/cache.rs` (LRU TTL 24 h).
* `auth/jwt.rs` RS256+JWKS path.
* `openapi/spec.rs` + `/openapi.json` + `/docs`.
* `observability/metrics.rs` (Prometheus exposition).
* `tracing-subscriber` JSON setup.
* 8+ tests.

### Day 60 — TLS + graceful shutdown + Docker + K8s + smoke
* `tls.rs` rustls integration (opt-in).
* `shutdown.rs` SIGTERM/SIGINT handling.
* `deploy/docker/Dockerfile.gateway` (distroless).
* `deploy/k8s/{deployment, service, servicemonitor, hpa}.yaml`.
* Smoke test : start binary, hit every endpoint with curl.
* 15+ acceptance tests + 20 audit tests.

---

## 12. Test strategy

### 12.1 Acceptance test matrix (15)

```
phase_7_n22_binary_starts_and_serves_healthz
phase_7_n22_docker_image_runs_and_passes_healthz
phase_7_n22_router_serves_all_21_endpoints
phase_7_n22_middleware_stack_order_correct
phase_7_n22_jwt_hs256_validated
phase_7_n22_jwt_rs256_jwks_validated
phase_7_n22_cross_tenant_post_rejected_403
phase_7_n22_4xx_5xx_rfc_7807_compliant
phase_7_n22_etag_returned_on_lookup
phase_7_n22_metrics_endpoint_exposition_format
phase_7_n22_traceparent_header_propagated
phase_7_n22_log_lines_are_json
phase_7_n22_openapi_spec_lists_all_endpoints
phase_7_n22_readiness_returns_503_when_postgres_unreachable
phase_7_n22_graceful_shutdown_completes_within_30s
phase_7_n22_rate_limit_429_after_threshold
```

### 12.2 Property tests (4)

* `proptest_rfc_7807_body_always_valid_json` — any error body
  parses as JSON + matches RFC 7807 schema.
* `proptest_traceparent_round_trip` — any valid W3C trace context
  in == identical out.
* `proptest_idempotency_key_dedupes` — same key + same body within
  TTL = cached response.
* `proptest_jwt_invalid_inputs_rejected` — every malformed JWT
  category returns 401.

### 12.3 Chaos tests (4)

* `chaos_postgres_drops_mid_request` — Postgres dies during a
  publisher register ; gateway returns 503, no crash.
* `chaos_swarm_panics_in_handler` — agent panic during
  swarm/run ; gateway returns 500 RFC 7807, server stays alive.
* `chaos_sigterm_during_in_flight` — SIGTERM with 5 in-flight
  requests ; all complete or 30 s drain ; exit 0.
* `chaos_body_size_attack` — 100 MB body in single request ; 413
  returned + connection cleaned.

### 12.4 Security tests (3)

* `security_jwt_secret_never_logged` — every log line scrubbed.
* `security_authorization_header_not_in_traces` — tracing skips.
* `security_tenant_isolation_with_forged_body` — JWT(acme) +
  body.tenant_id=globex → 403 ; logs the attempt.

### 12.5 Cross-cutting smoke tests (6)

* `cross_n21_n22_swarm_run_via_http` — end-to-end from HTTP POST
  through the swarm.
* `cross_n18_n22_publisher_register_via_http` — register +
  rotate-key via HTTP.
* `cross_n19_n22_gold_run_via_http` — submit gold set + run via
  HTTP.
* `cross_n17_n22_provenance_join_by_trace_id` — call swarm →
  trace_id appears in N17 record.
* `cross_n20_n22_postgres_rls_through_handler` — handler issues
  Postgres query with `SET LOCAL tenant_id` ; cross-tenant query
  fails.
* `cross_n16_n22_replay_via_http` — POST /v1/replays → job
  picked up.

---

## 13. Cross-cutting alignment (N11 → N21)

| Subsystem | N22 wiring |
|---|---|
| **N11 Firecracker pool** | Used by swarm/run when `Compliance::N18Signed` is set per agent ; gateway transparently propagates |
| **N12 cache** | LLM calls inside swarm via gateway are cached by N12 key |
| **N13 canary** | Tier 1 promote endpoint surfaces canary state in response |
| **N14 envelope** | `traceparent` from HTTP maps to N14 `trace_id` field |
| **N15 fairness** | Per-tenant rate limit is the HTTP-layer analogue ; N15 still enforces deeper queue ordering |
| **N16 multi-region** | Replay endpoints route to N16 `ReplayCoordinator` |
| **N17 provenance** | Every LLM call's record carries the HTTP `trace_id` (Sprint 14 wiring with bridge) |
| **N18 publisher** | Publisher CRUD endpoints expose `PublisherApi` |
| **N19 gold** | Gold endpoints expose `GoldApi` |
| **N20 Postgres** | Gateway handlers `SET LOCAL tenant_id` before queries |
| **N21 swarm + agentic** | Primary use case ; swarm/run is the most-called endpoint |

---

## 14. Sprint 14 ship vs Sprint 15+ split

### Sprint 14 (this design)
* All 8 capabilities from § 0.
* 21 HTTP endpoints, all OpenAPI documented.
* In-memory rate limit + idempotency cache.
* HS256 + RS256+JWKS JWT.
* Docker + K8s manifests.
* 15 acceptance + 4 property + 4 chaos + 3 security + 6 cross-
  cutting tests.

### Sprint 15 N23 — Multi-pod scalability
* Redis-backed rate limit (cross-pod).
* Redis-backed idempotency cache.
* OAuth 2.1 / OIDC support.
* WebSocket / SSE for streaming responses (Claude streaming).
* mTLS service-to-service.
* Webhook callbacks (POST to customer-provided URL on swarm
  complete).

### Sprint 16 N24 — gRPC + cross-region
* tonic gRPC alongside HTTP.
* Cross-region routing (N16 wiring at HTTP layer).
* API key management endpoints.

### Sprint 17 GA — v1.0 release
* Sigstore signing of binary.
* Production deployment guide.
* Customer onboarding flow.

---

## 15. Annexes

### A1 — Cargo dependencies summary

```toml
# efe-gateway/Cargo.toml
[package]
name = "efe-gateway"
version = "0.0.1"

[dependencies]
# Workspace EFE crates
efe-nano-sdk             = { path = "../efe-nano-sdk" }
efe-publisher-identity   = { path = "../efe-publisher-identity" }
efe-gold-test-gate       = { path = "../efe-gold-test-gate" }
efe-multi-region         = { path = "../efe-multi-region" }
efe-llm-provenance       = { path = "../efe-llm-provenance" }
efe-blackboard           = { path = "../efe-blackboard" }
efe-swarm-orchestrator   = { path = "../efe-swarm-orchestrator" }
efe-agentic-bridge       = { path = "../efe-agentic-bridge" }
efe-nano-preanalysis     = { path = "../efe-nano-preanalysis" }

# HTTP stack
axum                = "0.8"
tower               = "0.5"
tower-http          = { version = "0.6", features = ["trace", "limit", "request-id", "compression-gzip", "catch-panic"] }
tower-governor      = "0.5"
hyper               = "1.5"

# Auth
jsonwebtoken        = "9.3"

# Observability
tracing             = "0.1"
tracing-subscriber  = { version = "0.3", features = ["json", "env-filter"] }
prometheus          = "0.13"

# Spec
utoipa              = { version = "5.3", features = ["axum_extras"] }
utoipa-swagger-ui   = { version = "8.0", features = ["axum"] }

# TLS
rustls              = "0.23"
rustls-pemfile      = "2.2"
tokio-rustls        = "0.26"

# Common
tokio               = { workspace = true, features = ["full"] }
serde               = { workspace = true }
serde_json          = { workspace = true }
thiserror           = { workspace = true }
uuid                = { workspace = true }
async-trait         = { workspace = true }

[[bin]]
name = "efe-gateway"
path = "src/main.rs"

[dev-dependencies]
axum-test           = "16.4"
tower               = { version = "0.5", features = ["util"] }
```

### A2 — Env var reference

| Env var | Default | Description |
|---|---|---|
| `EFE_GATEWAY_BIND` | `0.0.0.0:8080` | Bind address |
| `EFE_GATEWAY_LOG_LEVEL` | `info` | tracing-subscriber filter |
| `EFE_GATEWAY_LOG_FORMAT` | `json` | `json` or `pretty` |
| `EFE_GATEWAY_AUTH` | `hs256` | `hs256` or `rs256_jwks` |
| `EFE_GATEWAY_JWT_SECRET` | — | HS256 secret (base64) |
| `EFE_GATEWAY_JWKS_URL` | — | RS256 JWKS endpoint |
| `EFE_GATEWAY_JWT_ISSUER` | — | RS256 expected issuer |
| `EFE_GATEWAY_JWT_AUDIENCE` | — | RS256 expected audience |
| `EFE_GATEWAY_BODY_LIMIT_BYTES` | `10485760` (10 MB) | Body cap |
| `EFE_GATEWAY_MAX_CONN` | `10000` | Concurrent connection cap |
| `EFE_GATEWAY_RATE_LIMIT_RPS` | `100` | Per-tenant rate |
| `EFE_GATEWAY_GRACEFUL_SHUTDOWN_SECS` | `30` | Drain timeout |
| `EFE_GATEWAY_TLS_CERT` | — | Optional TLS cert path |
| `EFE_GATEWAY_TLS_KEY` | — | Optional TLS key path |
| `EFE_GATEWAY_IDEMPOTENCY_CACHE_SIZE` | `10000` | LRU size |
| `EFE_GATEWAY_IDEMPOTENCY_TTL_SECS` | `86400` | 24 h |
| `ANTHROPIC_API_KEY` | — | Claude API key (for swarm/run) |
| `DATABASE_URL` | — | Postgres URL (Sprint 15) |

### A3 — K8s Deployment YAML (excerpt)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: efe-gateway
  namespace: efe
spec:
  replicas: 3
  selector:
    matchLabels: { app: efe-gateway }
  template:
    metadata:
      labels: { app: efe-gateway }
    spec:
      terminationGracePeriodSeconds: 35
      containers:
      - name: gateway
        image: registry.efe.ai/efe-gateway:v0.0.1
        ports:
        - containerPort: 8080
          name: http
        env:
        - { name: EFE_GATEWAY_BIND, value: "0.0.0.0:8080" }
        - { name: EFE_GATEWAY_LOG_LEVEL, value: "info" }
        - { name: EFE_GATEWAY_AUTH, value: "rs256_jwks" }
        - { name: EFE_GATEWAY_JWKS_URL, value: "https://auth.efe.ai/.well-known/jwks.json" }
        envFrom:
        - secretRef: { name: efe-gateway-secrets }
        readinessProbe:
          httpGet: { path: /readyz, port: http }
          periodSeconds: 5
          failureThreshold: 3
        livenessProbe:
          httpGet: { path: /healthz, port: http }
          periodSeconds: 10
          failureThreshold: 3
        resources:
          requests: { cpu: "500m", memory: "512Mi" }
          limits:   { cpu: "2",    memory: "2Gi" }
---
apiVersion: v1
kind: Service
metadata:
  name: efe-gateway
spec:
  selector: { app: efe-gateway }
  ports:
  - { port: 80, targetPort: http, name: http }
---
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: efe-gateway
spec:
  selector: { matchLabels: { app: efe-gateway } }
  endpoints:
  - port: http
    path: /metrics
    interval: 15s
```

### A4 — Curl examples

```bash
# Health
curl -s https://api.efe.ai/healthz | jq .

# OpenAPI spec
curl -s https://api.efe.ai/openapi.json | jq .info.version

# Swagger UI (browser)
open https://api.efe.ai/docs

# Run a swarm
curl -X POST https://api.efe.ai/v1/swarms/run \
  -H "Authorization: Bearer $JWT" \
  -H "Idempotency-Key: $(uuidgen)" \
  -H "Content-Type: application/json" \
  -d '{
    "dag_id": "pdf-analyser",
    "input": {
      "tenant_id": "acme-corp",
      "text": "Contract between Alice and Bob..."
    }
  }'

# Register a publisher
curl -X POST https://api.efe.ai/v1/publishers \
  -H "Authorization: Bearer $JWT" \
  -d '{ "candidate": { ... } }'

# Approve a gold set (operator JWT)
curl -X POST https://api.efe.ai/v1/gold-sets/$ID/approve \
  -H "Authorization: Bearer $OPERATOR_JWT" \
  -d '{ "approvers": [...] }'
```

### A5 — Comparison with industry approaches

| Framework / Service | Their HTTP layer | N22's choice |
|---|---|---|
| **LangChain Serve** | ad-hoc FastAPI ; no OpenAPI ; no multi-tenancy | + multi-tenant, + OpenAPI, + RFC 7807 |
| **OpenAI API** | bespoke ; no standard error format | + RFC 7807 |
| **Anthropic API** | clean tool-use schema ; minimal docs | similar shape, plus W3C tracing |
| **AWS Bedrock** | SigV4 auth, gRPC + REST | JWT bearer + REST only Sprint 14 |
| **Replicate API** | webhook callbacks for long-running | webhooks Sprint 15 |
| **Cloudflare Workers AI** | edge-routed ; no per-tenant rate | per-tenant rate from day 1 |

---

**End of N22 design.**

**Author** : EFE M50 architecture team.
**Date** : 2026-05-21.
**Sprint** : 14 (Phase 6, day 56 – 60).
**Design lineage** : N11 + N12 + N13 + N14 + N15 + N16 + N17 + N18 +
N19 + N20 + N21 → **N22** (this doc) → N23 multi-pod scalability →
N24 gRPC + cross-region → N25 GA v1.0 release.

**Last-mile checklist** (Sprint 14 exit) :
- [ ] All 15 acceptance tests GREEN.
- [ ] Clippy strict (`-D warnings`) on new crate.
- [ ] Docker image builds + runs.
- [ ] K8s manifests pass `kubectl apply --dry-run`.
- [ ] `/openapi.json` validates against OpenAPI 3.1 schema.
- [ ] curl smoke against running binary covers all 21 endpoints.
- [ ] Graceful shutdown verified with 5 in-flight requests.
- [ ] Rate limit tested with `wrk -c 200 -t 4 -d 30s`.
- [ ] Cross-tenant attack vector pen-tested.
- [ ] RS256+JWKS path tested against real Auth0 dev tenant.
- [ ] Distroless image size ≤ 50 MB.
- [ ] First pilot customer onboarded with JWT.
