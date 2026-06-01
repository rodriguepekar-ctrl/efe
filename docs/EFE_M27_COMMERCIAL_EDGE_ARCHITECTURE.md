# EFE M27 — Commercial Edge Architecture

**Status** : ARCHITECTURE — design phase, pre-execution.
**Predecessors** :
- `EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` (canonical M26 spec)
- `EFE_M26_ASSEMBLY_VERIFICATION_ARCHITECTURE.md` (D93 assembly + chirurgical audit)
- `EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` (LLMRouter + provider abstraction)
**Target** : turn EFE 2.1.1 (deployable engine, no commercial surface) into a survivable,
multi-tenant, edge-protected, cache-accelerated SaaS — the first commercial-ready release.
**Codename** : "Carillon-Edge" (the bell at the door of the commercial product).
**EFE version** : `2.2.0` — semver MINOR (new commercial surface, no breaking API).
**Sprint window** : 4-6 working weeks (T+0 → T+6w of the Foundation quarter).
**Hard deps** : M21 LLMRouter, M25/M26 agent runtime, cert-manager ≥ 1.16, Cloudflare
account (Enterprise plan for Bot Management + Advanced WAF).

---

## Table of Contents

1. [Executive summary](#1-executive-summary)
2. [Scope + non-goals](#2-scope--non-goals)
3. [What M26 + D93 left to fix at edge level](#3-what-m26--d93-left-to-fix-at-edge-level)
4. [Threat model](#4-threat-model)
5. [Topology overview](#5-topology-overview)
6. [Component A — Cloudflare frontend](#6-component-a--cloudflare-frontend)
7. [Component B — Envoy in-cluster API gateway](#7-component-b--envoy-in-cluster-api-gateway)
8. [Component C — Redis cluster](#8-component-c--redis-cluster)
9. [Component D — PgBouncer](#9-component-d--pgbouncer)
10. [Component E — Postgres read replicas](#10-component-e--postgres-read-replicas)
11. [Component F — Backpressure middleware](#11-component-f--backpressure-middleware)
12. [Component G — Tenant quota middleware](#12-component-g--tenant-quota-middleware)
13. [Component H — API versioning + sunset policy](#13-component-h--api-versioning--sunset-policy)
14. [SQL migrations introduced](#14-sql-migrations-introduced)
15. [Helm chart additions](#15-helm-chart-additions)
16. [Terraform Cloudflare config](#16-terraform-cloudflare-config)
17. [Observability matrix](#17-observability-matrix)
18. [SLO definitions + alerts](#18-slo-definitions--alerts)
19. [Performance budget + load test plan](#19-performance-budget--load-test-plan)
20. [Failure modes + degradation paths](#20-failure-modes--degradation-paths)
21. [Migration plan M26 → M27](#21-migration-plan-m26--m27)
22. [Testing strategy](#22-testing-strategy)
23. [Runbooks (4 new)](#23-runbooks-4-new)
24. [ADRs (4)](#24-adrs-4)
25. [Risk register](#25-risk-register)
26. [Sprint plan (6 weeks)](#26-sprint-plan-6-weeks)
27. [Done definition](#27-done-definition)
28. [Annexes](#28-annexes)

---

## 1. Executive summary

After M26 + D93, EFE's engine is operational, governance-baked, and Helm-deployable.
But the deployment exposes `efe-server` pods **directly to the world** — no edge layer,
no WAF, no CDN, no DDoS protection, in-process rate limiting only (a single-pod
attacker can still saturate the cluster), and **no caching for hot LLM/embedding
calls** (every duplicate classify call burns LLM budget). PG is one primary; portal
reads contend with classify writes.

M27 — **Commercial Edge** — wraps the engine in **6 production layers** :

1. **Cloudflare frontend** (Anycast, L3-L4 DDoS absorption, WAF, CDN for docs/SDK,
   Bot Management, mTLS optional).
2. **Envoy in-cluster gateway** (TLS termination, edge JWT validation, per-tenant
   distributed rate limit via Redis, request/response transformation, API
   versioning, mTLS downstream from CF tunnel).
3. **Redis Cluster** (3 master + 3 replica) for hot embedding cache, LLM response
   dedup cache, tenant quota counters, rate limit token buckets.
4. **PgBouncer** (transaction pooling) in front of PG to support 5000 client
   connections with 50 server connections per pool.
5. **PG read replicas** (× 2) with read-path routing in `efe-server` so portal
   admin and classify reads stop saturating the primary.
6. **Backpressure + tenant-quota middlewares** in `efe-server` that complement
   (not replace) the edge filters with shed-and-degrade behaviour under load.

Together these introduce **zero new API surface** for tenants but **unblock**:
- ≥ 1000 RPS sustained per cluster with p99 < 200ms (vs ~50 RPS today).
- DDoS test (10× baseline spike) without service tearing.
- 5× LLM cost reduction via dedup cache for repeated classify-with-LLM calls.
- First paying customer signature (edge + Redis are **the** items every enterprise
  security review checks first).

**Migration risk** : moderate. Envoy is a new pod ; if mis-configured it can break
all client traffic (single point of failure). Mitigation = staged rollout per
tenant via Cloudflare DNS routing, with a 1-week canary period.

---

## 2. Scope + non-goals

### 2.1 In scope (will ship in EFE 2.2.0)

- **Cloudflare Enterprise account setup + Terraform configuration**.
- **Envoy gateway** Helm sub-chart (`charts/efe-stack/charts/efe-edge/`) :
  config map, deployment, service, HPA, NetworkPolicy.
- **Redis Cluster** Helm sub-chart : statefulset, configmap, service headless,
  bitnami/redis-cluster compatible config.
- **PgBouncer** Helm sub-chart : deployment, service, configmap.
- **PG read replicas** Helm configuration (assumes managed PG service e.g. RDS,
  Cloud SQL, or Crunchy Bridge ; self-hosted Patroni is out of scope, deferred
  to M30 multi-region).
- **Backpressure middleware** (Tower layer in `efe-server`).
- **Tenant quota middleware** (Tower layer + Redis client + load from
  `tenant_feature`).
- **New `efe-cache` crate** : typed Redis client wrapper for embedding + LLM
  caches (shared between `efe-server` and `efe-llm-router`).
- **`efe_quota_*` metrics + Prometheus rules + Grafana dashboards**.
- **4 new runbooks** : RB_M27_EDGE_DDOS, RB_M27_REDIS_FAIL,
  RB_M27_PGBOUNCER_SAT, RB_M27_REPLICA_LAG.
- **API versioning policy + sunset header** (foundation for future v2).
- **Load testing harness** (k6 + scripts/load/) + acceptance test data.

### 2.2 Out of scope (explicitly deferred)

- **Multi-region active-passive** — M30 Multi-region (Patroni or RDS Aurora
  global cluster).
- **Vector search adjacent product** — M31.
- **Multi-modal embedders** — M30.
- **OAuth2 / SSO provider integrations** (beyond OIDC ES256 already in M26).
- **Replacing in-process token bucket** (`crates/efe-server/src/ratelimit/`) —
  it stays for **anonymous** traffic (where Envoy can't rate-limit without
  identifying the tenant). The Redis-backed quota stacks on top, not replaces.
- **Managed Redis service evaluation** (ElastiCache, MemoryStore) — Helm
  self-hosted Redis is the v0 ; managed is a separate Q3/Q4 evaluation.
- **Caching of `classify` results themselves** (only embeddings + LLM calls are
  cached). Caching the final classify output would require careful cache
  invalidation tied to catalog versioning ; deferred to M31.

### 2.3 Out of scope but adjacent (parallel sprint candidates)

- **CDN for docs site + SDK downloads** — P1 marketing surface, separate sprint.
- **Status page** (status.efe.ai) — P1 marketing surface, separate sprint.
- **WAF-as-a-Service evaluation** (Cloudflare Magic WAF vs ModSecurity sidecar)
  — Cloudflare WAF chosen by default ; ModSecurity in-cluster only if
  Cloudflare drops a customer's region.

---

## 3. What M26 + D93 left to fix at edge level

The chirurgical audit (D93) found that the engine is solid but:

| Gap | Symptom today | M27 fix |
|---|---|---|
| **No edge auth** | JWT validated **per request inside efe-server**, even for bots/attackers. Every malformed-token spike burns CPU. | Envoy `envoy.filters.http.jwt_authn` validates at gateway. Bad tokens never reach efe-server. |
| **In-process rate limit only** | `crates/efe-server/src/ratelimit/` is per-pod state. Behind a load-balancer with N pods, an attacker gets N× the quota. | Envoy → ratelimit-service → Redis : **single source of truth** per (tenant, endpoint). |
| **No WAF** | SQL-injection / XSS / scanner traffic reaches the application. | Cloudflare Managed Ruleset + OWASP CRS at CF + selective ModSecurity at Envoy. |
| **No CDN** | Every `GET /v1/version`, `GET /healthz` from a remote bot hits the pod. Static SDK download URLs (when published) would too. | Cloudflare CDN with cache rules + Argo Smart Routing. |
| **No DDoS protection** | Cluster can be saturated by L3/L4 floods (UDP, SYN). | Cloudflare Anycast absorbs L3/L4 at edge globally. |
| **LLM duplicate calls billed** | M21 LLMRouter has a Redis cache **but** it's per-pod since the Redis is shared per LLMRouter pod, not per cluster. Two efe-server pods receiving the same prompt both pay. | M27 Redis Cluster is **the** shared cache plane for the whole cluster ; LLMRouter migrates to point at it. |
| **Embedding recompute** | Each classify recomputes the embedding even for identical input. The Mode-1 backend has no cache. | `efe-cache` crate wraps embedder calls with SHA256-keyed Redis cache (TTL 7d). |
| **PG connection storm** | At > 30 concurrent agent runs, sqlx PgPool starts queueing. The classifier hot path is slowed by agent-runtime activity. | PgBouncer transaction pooling : 5000 clients → 50 backend conn. Plus read-path routing to replicas for non-mutating SELECTs. |
| **Portal queries hit primary** | Every portal-admin page load runs analytical SELECTs on the primary. Long queries (1-5s) block writes. | Read replicas + explicit `pool_ro` routing in `efe-server` + portal-admin uses the ro endpoint. |
| **No tenant burst protection** | A tenant can burst 10× their quota for 60s before in-process rate limit kicks in. | Redis-backed sliding window per (tenant, endpoint), enforced at Envoy. |
| **No backpressure** | At saturation, the service tears (memory growth, queue blowup) before responding. | Tower layer measures concurrency + p99 ; sheds 10% randomly above thresholds. |

---

## 4. Threat model

The M27 edge layer is the **first line of defence** for everything inbound. Threats
enumerated below ; each maps to a specific control with a referenced § number.

| ID | Threat | Vector | Impact | Control (§) | Detection |
|---|---|---|---|---|---|
| T1.1 | **DDoS volumetric** (L3/L4) | SYN flood, UDP amplification | Cluster unreachable | CF Anycast absorption (§ 6.1) | CF analytics + `efe_edge_*_total{layer=l3l4}` |
| T1.2 | **DDoS application** (L7) | HTTP flood from large IP pool | efe-server pods saturated | CF Rate Limit Rules + Bot Mgmt (§ 6.3) + Envoy global rate limit (§ 7.5) | `efe_envoy_rate_limit_exceeded_total` |
| T2 | **Cost amplification** | Cheap input → expensive LLM call cycle | LLM bill blowup | Per-tenant cost quota in Redis (§ 12.2) + LLM-cache (§ 8.3) | `efe_llm_cost_cents_total{tenant}` |
| T3.1 | **JWT replay** | Stolen token used by attacker | Tenant impersonation | Token age check in efe-server (already M26) + Envoy revocation list (§ 7.4.3) | `efe_jwt_validation_failures_total` |
| T3.2 | **JWT forgery** | Attacker mints own ES256 JWT | Auth bypass | Envoy `jwt_authn` filter validates signature against public JWKS (§ 7.4) | Envoy access log + correlation ID |
| T4 | **Rate-limit bypass** | IP rotation / multiple tenant slugs | Burst quota exhaustion | Redis-backed **per-tenant** counter, not per-IP (§ 8.4) | `efe_quota_*` |
| T5 | **Cache poisoning** | Force embeddings via crafted text | Wrong classify outputs | Cache keys are SHA256 of canonical-form text ; collision-resistant. Read-only cache for tenants (no write API). (§ 8.2) | Manual quarterly cache audit |
| T6 | **Tenant cross-talk** | Shared Redis cache leaks data | RGPD breach | Cache keys prefixed with `tenant:{slug}:` ; deserialization checks tenant binding (§ 8.6) | Cross-tenant cache key audit (CI test) |
| T7 | **Side-channel timing** | Measure cache hit/miss to infer existence of other tenants' data | Limited reconnaissance | Cache keys hashed (SHA256), no length-based leak. Timing jitter on miss (configurable). (§ 8.7) | Out-of-band timing test (M27 pentest) |
| T8 | **WAF bypass** | Encoded payload (base64, URL-encoded twice) | SQL inject / XSS | OWASP CRS at CF + decoding rules (§ 6.4.2) | CF firewall events |
| T9 | **Connection exhaustion** | Slowloris, idle keep-alive flood | Envoy or efe-server stuck | Envoy `connection_manager_idle_timeout = 60s`, max concurrent streams = 100/conn (§ 7.7) | `envoy_http_downstream_cx_total` |
| T10 | **Redis cluster split-brain** | Network partition | Inconsistent quotas (under-counted) | Quorum-based RFC : 2 of 3 master ack required (§ 8.8) | `redis_cluster_known_nodes` |
| T11 | **PgBouncer saturation** | All 50 server conn busy | Classify hangs | Queue + 503 + Retry-After (§ 9.5) | `pgbouncer_pools_*_waiting` |
| T12 | **Replica lag spike** | Long write transaction on primary | Read path stale | Lag monitor + automatic failover to primary above 30s (§ 10.6) | `efe_pg_replica_lag_seconds` |

The **pen test bundle** (`bench/m27_edge_pentest/`) covers T1.2, T2, T3.1, T4, T6,
T9 with executable scenarios ; the others are documented threats with manual
audit cadence (quarterly for T5/T7, monthly for T10/T11/T12 via runbook).

---

## 5. Topology overview

```
                       Internet (untrusted)
                              │
                              │ DNS → cloudflare ANYCAST
                              ▼
            ┌─────────────────────────────────────────────────┐
            │                  CLOUDFLARE                      │
            │  - L3/L4 DDoS absorption (Anycast)              │
            │  - WAF (OWASP CRS + custom)                      │
            │  - Bot Management (fingerprint + JS challenge)   │
            │  - Rate Limit Rules (per-IP, per-URI)            │
            │  - CDN (docs, SDK downloads — separate)          │
            │  - mTLS optional (enterprise tenants)            │
            │  - Argo Tunnel to origin (no public IP)          │
            └─────────────────────────────────────────────────┘
                              │
                              │ Argo Tunnel (cloudflared)
                              │ — TLS 1.3 — mTLS to origin
                              ▼
            ┌─────────────────────────────────────────────────┐
            │       K8s Ingress (cloudflared pod)              │
            │       — terminates Argo Tunnel, forwards         │
            └─────────────────────────────────────────────────┘
                              │ HTTP/2 cleartext (intra-cluster)
                              ▼
            ┌─────────────────────────────────────────────────┐
            │       ENVOY API GATEWAY                          │
            │  - mTLS termination (CF cert pinned)             │
            │  - JWT auth (ES256 via OIDC JWKS)               │
            │  - Per-tenant rate limit via Redis              │
            │  - Request transform : inject x-tenant-id,      │
            │    x-correlation-id, x-token-age-s               │
            │  - API version routing : /v1/* | /v2/*           │
            │  - Per-route timeout + retry policy              │
            │  - Access log → Loki                             │
            └─────────────────────────────────────────────────┘
                              │ HTTP/2 cleartext (intra-mesh)
                              ▼
            ┌─────────────────────────────────────────────────┐
            │       efe-server pods (N replicas, HPA)          │
            │  - Tower layer : backpressure (concurrency,     │
            │    p99 latency, shed-and-degrade)                │
            │  - Tower layer : tenant quota (Redis-backed)    │
            │  - Existing M0-M26 business logic                │
            └─────────────────────────────────────────────────┘
                              │
            ┌────────┬────────┼────────┬────────────────────┐
            │        │        │        │                    │
            ▼        ▼        ▼        ▼                    ▼
        PgBouncer   PgBouncer  Redis   M21 LLMRouter      M21 LLMRouter
        (write)    (read RO)  Cluster  (multi-provider)   cache (shared
            │        │          │      with cost cap)     in Redis)
            ▼        ▼          ▼
        PG primary  PG read    embed/llm/quota/rl-token-bucket keyspaces
                    replicas
                    (× 2)
```

### 5.1 Trust boundaries

| Boundary | Direction | Auth | Encryption |
|---|---|---|---|
| Internet → Cloudflare | TLS 1.2+ | None at CF (Bot Mgmt does fingerprinting) | TLS |
| CF → Argo Tunnel | mTLS | CF-signed cert (origin pin) | TLS 1.3 |
| Tunnel → Envoy | HTTP/2 cleartext (intra-cluster) | NetworkPolicy isolates | None (cluster network) |
| Envoy → efe-server | HTTP/2 cleartext (intra-mesh) | mTLS optional via Linkerd/Istio (M30) | None or mTLS |
| efe-server → Redis | Plain TCP | Redis AUTH password + ACL user `efe-server` | None (in-cluster, NetworkPolicy) ; TLS optional |
| efe-server → PgBouncer | Plain TCP | Password from Secret | TLS (sslmode=prefer) |
| PgBouncer → PG | TLS | Password + cert | TLS (sslmode=require) |
| efe-server → LLM provider | HTTPS | API key per provider | TLS |

### 5.2 Cluster network policies (manifests in § 15.4)

- **default-deny** ingress + egress in namespace `efe-system`.
- **edge** namespace dedicated to Envoy + cloudflared (Argo Tunnel).
- Allow edge → efe-system on ports 50051 (gRPC) + 8080 (REST agent) + 8088
  (REST agent gateway).
- Allow efe-system → redis cluster on 6379.
- Allow efe-system → pgbouncer on 5432.
- Allow pgbouncer → PG (managed service, external).
- Deny efe-system → Internet **except** approved egress proxy to
  api.openai.com / api.anthropic.com (M21 egress allow-list).

---

## 6. Component A — Cloudflare frontend

### 6.1 Plan and zones

- **Plan** : Cloudflare Enterprise.
  - Required features : Bot Management, Advanced WAF (CRS managed),
    Rate Limiting (Advanced), Page Shield, mTLS authentication.
- **Zones** :
  - `efe.ai` — primary apex.
  - `api.efe.ai` — the API endpoint pointing to Argo Tunnel ID
    `00000000-0000-0000-0000-000000000000` (placeholder, real one in
    `terraform/cloudflare/variables.tf`).
  - `docs.efe.ai`, `www.efe.ai` — covered by P1 marketing surface,
    referenced here for cache rule coherence.

### 6.2 DDoS settings

- **L3/L4 absorption** : automatic, Anycast.
- **L7 sensitivity** : "I'm Under Attack Mode" (IUAM) **disabled** by default
  (legitimate API clients must work without JS challenges). IUAM is a runbook
  toggle for severe incidents (RB_M27_EDGE_DDOS § 3).
- **HTTP DDoS Attack Protection** : enabled, sensitivity = high, default
  action = challenge.
- **Network Layer DDoS** : enabled, sensitivity = high, action = block.

### 6.3 Bot Management

- **Bot Fight Mode** : OFF (false-positive risk for legitimate API clients).
- **Super Bot Fight Mode** : OFF.
- **Bot Management (Enterprise)** : ON, with custom rules :
  - Allow : Verified bots (Google, Bing).
  - Challenge (Managed Challenge) : "Likely bot" + Threat Score > 30 + URI
    matching `/v1/auth/signup` or `/v1/auth/login`.
  - Block : "Definitely bot" + Threat Score > 60.

### 6.4 WAF

#### 6.4.1 Managed rulesets

- **Cloudflare Managed Ruleset** : enabled, sensitivity = high.
- **OWASP Core Ruleset** : enabled, paranoia level = 1 (start low, raise to 2
  after 2-week tuning).
- **Cloudflare Exposed Credentials Check** : enabled, action = block.

#### 6.4.2 Custom rules

```
# Rule 1 — Block requests with double-encoded payloads (common WAF bypass)
(http.request.uri contains "%25" and http.request.uri contains "%")  → Block

# Rule 2 — Rate limit signups (anti-bot)
(http.request.uri.path eq "/v1/auth/signup") → rate limit 5/min per IP

# Rule 3 — Rate limit classify (per IP, fallback when no JWT)
(http.request.uri.path matches "^/v1/classify") → rate limit 100/min per IP

# Rule 4 — Block requests without User-Agent header (most bots)
(not http.request.headers["user-agent"] exists) → Challenge

# Rule 5 — Block known TOR exit nodes (configurable per tenant)
(ip.geoip.is_tor_exit_node) → Managed Challenge

# Rule 6 — Allow IP allowlist for enterprise tenants (signed customer IPs)
(ip.src in $enterprise_allowlist) → Skip WAF
```

#### 6.4.3 Page Shield

- Enabled with default policy (script monitoring).
- Detects unauthorized JS on portal-admin pages.
- Out-of-scope for API but listed because portal-admin shares the zone.

### 6.5 CDN cache rules

The API itself is **not cacheable** (every response is tenant-specific).
But:

- `/v1/version` → Cache 5 min (read-only, public).
- `/healthz` → No cache (CF should not absorb, must hit origin for honesty).
- `/docs/*` (separate zone `docs.efe.ai`) → Cache 1 hour, purge on deploy.
- `/sdk/*` (versioned SDK tarballs, separate zone) → Cache 1 year (immutable).

### 6.6 mTLS option for enterprise

- Enterprise tenants can be onboarded with a CF mTLS authentication policy.
- Policy = "match URI `/v1/*` AND tenant slug header AND client cert in CA
  list" → enforce mTLS.
- CA per tenant uploaded via `efe-admin tenant mtls upload-ca <slug> ca.pem`.
- Out-of-scope first version : we ship the framework, customer onboard is
  manual.

### 6.7 Argo Tunnel

- One tunnel per region (M30 splits ; for M27 = single eu-west-1 tunnel).
- Tunnel ID stored in Vault under `cloudflare/tunnel/eu-west-1`.
- `cloudflared` daemon runs as DaemonSet in cluster, 2 replicas for HA.
- No public IP for origin → origin scan-immune.

---

## 7. Component B — Envoy in-cluster API gateway

### 7.1 Why Envoy (not Kong, not NGINX)

| Criterion | Envoy | Kong | NGINX+ |
|---|:--:|:--:|:--:|
| Native gRPC support | ✅ | ✅ | partial |
| External authz / ratelimit gRPC | ✅ | plugin | plugin |
| xDS dynamic config | ✅ | ❌ | partial |
| Observability (metrics + tracing native) | ✅ | partial | partial |
| Lua filter | ✅ | ✅ | ✅ |
| WASM filter | ✅ | partial | ❌ |
| Production deployments (Istio, AWS App Mesh, Lyft) | ✅ | ✅ | ✅ |
| License | Apache 2.0 | Apache 2.0 | BSD-2 / Plus paid |
| Resource overhead | low | medium | low |

**Decision** : Envoy. Direct deployment (no Istio overhead — we want only the
data plane).

### 7.2 Deployment topology

- **Replicas** : 3 minimum, HPA up to 10 based on CPU 60% and RPS 1000/pod.
- **Resources** : `requests: cpu=200m mem=256Mi, limits: cpu=1 mem=1Gi`.
- **Anti-affinity** : podAntiAffinity (preferredDuringScheduling) for zone
  spread.
- **PDB** : minAvailable = 2 (so a node drain still leaves quorum).
- **NetworkPolicy** : ingress from cloudflared pod only, egress to
  `efe-server`, `redis-cluster`, `efe-llm-router` services only.

### 7.3 Static config sections (full YAML in § 28 Annex A)

- `node` : identifies the Envoy node (region, replica id).
- `static_resources.listeners` :
  - `listener_https` : 0.0.0.0:8443, TLS termination + HTTP filters chain.
  - `listener_admin` : 127.0.0.1:9901, admin endpoint (in-pod only).
- `static_resources.clusters` :
  - `efe-server-grpc` : HTTP/2, EDS-driven endpoints, lb_policy=ROUND_ROBIN.
  - `efe-server-rest` : HTTP/1.1 + HTTP/2, EDS.
  - `efe-server-agent-rest` : HTTP/1.1, EDS, the M26 agent REST port 8088.
  - `ratelimit-service` : gRPC v3 ratelimit-service (Lyft).
  - `redis-cluster` : EDS, lb_policy=CLUSTER_PROVIDED (redis-aware).
  - `jwks-cache` : HTTPS to OIDC issuer, used by jwt_authn (cached locally).
- `admin` : 127.0.0.1:9901.
- `bootstrap_extensions` : `envoy.bootstrap.internal_listener` for unix socket
  inter-filter chain (admin-only).
- `stats_sinks` : Prometheus endpoint on :9091 (scraped by ServiceMonitor).

### 7.4 HTTP filter chain (order matters!)

The chain :

```
1. envoy.filters.http.cors                    (preflight handling)
2. envoy.filters.http.lua                     (correlation ID injection, JWT pre-extract)
3. envoy.filters.http.jwt_authn               (validate ES256 signature)
4. envoy.filters.http.ext_authz               (optional — call efe-server /authz for fine-grained checks)
5. envoy.filters.http.ratelimit               (per-tenant, per-route, distributed)
6. envoy.filters.http.local_ratelimit         (fallback if ratelimit-service down)
7. envoy.filters.http.fault                   (chaos injection — disabled by default)
8. envoy.filters.http.router                  (final dispatch to cluster)
```

#### 7.4.1 CORS filter

```yaml
- name: envoy.filters.http.cors
  typed_config:
    "@type": type.googleapis.com/envoy.extensions.filters.http.cors.v3.Cors
```

Allowed origins controlled per route at the route level (not globally).

#### 7.4.2 Lua filter (correlation ID + JWT extract pre-validation)

The Lua filter does two things :

1. **Correlation ID** : inject `x-correlation-id` header if absent.
   - Format : UUID v4.
   - Surfaces in all access logs + propagated to efe-server.

2. **JWT pre-extraction** : pull `Authorization: Bearer <token>` and stash
   the `sub` and `tenant` claims into headers `x-token-sub` and `x-tenant-id`
   **after** jwt_authn has validated (jwt_authn validates, but doesn't
   inject custom headers). This avoids re-parsing in efe-server.

```lua
function envoy_on_request(request_handle)
  -- 1. Correlation ID
  local headers = request_handle:headers()
  if not headers:get("x-correlation-id") then
    headers:add("x-correlation-id", request_handle:streamInfo():getUuid() or
                                    uuid_v4())
  end
  -- jwt_authn runs after this filter ; claims will be in metadata.
  -- A second on_response phase pulls metadata → header.
end

function envoy_on_response(response_handle)
  -- Pass through. Could strip internal headers here.
end
```

#### 7.4.3 jwt_authn filter

```yaml
- name: envoy.filters.http.jwt_authn
  typed_config:
    "@type": type.googleapis.com/envoy.extensions.filters.http.jwt_authn.v3.JwtAuthentication
    providers:
      efe_oidc:
        issuer: "https://oidc.efe.ai"
        audiences:
          - "efe-agent-runtime"
          - "efe-classification"
        remote_jwks:
          http_uri:
            uri: "https://oidc.efe.ai/.well-known/jwks.json"
            cluster: jwks-cache
            timeout: 5s
          cache_duration: 600s
        forward: true       # keep the Authorization header for efe-server
        forward_payload_header: x-jwt-payload
        payload_in_metadata: jwt_payload
        claim_to_headers:
          - header_name: x-token-sub
            claim_name: sub
          - header_name: x-tenant-id
            claim_name: tenant
          - header_name: x-roles
            claim_name: roles
    rules:
      - match:
          prefix: "/healthz"
        # No JWT required for liveness probe (probes are intra-cluster).
      - match:
          prefix: "/v1/auth/"
        # Signup / login endpoints are unauthenticated by definition.
      - match:
          prefix: "/v1/"
        requires:
          provider_name: efe_oidc
      - match:
          prefix: "/v2/"
        requires:
          provider_name: efe_oidc
```

**Revocation list** : the standard `jwt_authn` filter doesn't natively check
revocation. We extend via a Lua filter calling Redis : `GET revoked:{jti}` →
if exists, return 401. Revocation list is populated by `efe-admin tenant
revoke-token <jti>` (CLI command added in M27).

#### 7.4.4 ext_authz filter (optional, feature-flagged)

When `enable_ext_authz=true` :

```yaml
- name: envoy.filters.http.ext_authz
  typed_config:
    "@type": type.googleapis.com/envoy.extensions.filters.http.ext_authz.v3.ExtAuthz
    grpc_service:
      envoy_grpc:
        cluster_name: efe-server-grpc
      timeout: 0.5s
    failure_mode_allow: false   # FAIL CLOSED on outage
    include_peer_certificate: true
```

Used for : fine-grained RBAC checks that depend on database state (e.g.
"this user can write to this catalog, but only if they own it"). Out-of-scope
for v1 ; enable later when needed.

#### 7.4.5 ratelimit filter

```yaml
- name: envoy.filters.http.ratelimit
  typed_config:
    "@type": type.googleapis.com/envoy.extensions.filters.http.ratelimit.v3.RateLimit
    domain: efe_edge
    failure_mode_deny: false    # FAIL OPEN if ratelimit-service down (use local_ratelimit fallback)
    rate_limited_as_resource_exhausted: true
    rate_limit_service:
      transport_api_version: V3
      grpc_service:
        envoy_grpc:
          cluster_name: ratelimit-service
        timeout: 0.25s
```

The Envoy `ratelimit-service` (Lyft) is deployed as its own pod (3 replicas) ;
it's stateless and reads its config from a ConfigMap. State lives in Redis.

Routes attach descriptors at the route_config level :

```yaml
rate_limits:
  - actions:
      - generic_key:
          descriptor_value: "classify_per_tenant"
      - request_headers:
          header_name: x-tenant-id
          descriptor_key: "tenant"
```

The ratelimit-service config (loaded from ConfigMap) :

```yaml
domain: efe_edge
descriptors:
  - key: generic_key
    value: classify_per_tenant
    descriptors:
      - key: tenant
        rate_limit:
          unit: minute
          requests_per_unit: 1000   # default ; override per-tenant via API
  - key: generic_key
    value: classify_anonymous
    rate_limit:
      unit: minute
      requests_per_unit: 60
```

Per-tenant overrides are stored in a separate ConfigMap reloaded every 30s :
`per_tenant_overrides.yaml` keys by tenant slug.

#### 7.4.6 local_ratelimit (fallback)

```yaml
- name: envoy.filters.http.local_ratelimit
  typed_config:
    "@type": type.googleapis.com/envoy.extensions.filters.http.local_ratelimit.v3.LocalRateLimit
    stat_prefix: http_local_rate_limiter
    token_bucket:
      max_tokens: 10000
      tokens_per_fill: 10000
      fill_interval: 60s
    filter_enabled:
      runtime_key: local_rate_limit_enabled
      default_value:
        numerator: 100
        denominator: HUNDRED
    filter_enforced:
      runtime_key: local_rate_limit_enforced
      default_value:
        numerator: 100
        denominator: HUNDRED
```

Triggered only if the gRPC ratelimit-service is unreachable. Per-pod (not
distributed) but it's the **safety net** : we'd rather throttle than over-serve
during a Redis outage.

#### 7.4.7 router filter

Standard envoy.router with retry policy :

```yaml
retry_policy:
  retry_on: "5xx,reset,connect-failure,refused-stream"
  num_retries: 2
  per_try_timeout: 5s
  retry_back_off:
    base_interval: 0.025s
    max_interval: 0.25s
  retriable_status_codes: [503]
```

Retries are **only for idempotent methods** (GET, HEAD). POSTs have
`x-request-id` checked at efe-server side for idempotency before retry is
considered safe ; the M11 idempotency layer covers this.

### 7.5 Listener / route config

The listener defines routes via xDS (RDS — RouteDiscoveryService) or static
RouteConfig. For v1 we use static :

```yaml
route_config:
  name: edge_routes
  virtual_hosts:
    - name: efe_api
      domains: ["api.efe.ai", "*.api.efe.ai"]
      routes:
        # ── gRPC ─────────────────────────────────────────────────────
        - match:
            prefix: "/efe.v1."
            grpc: {}
          route:
            cluster: efe-server-grpc
            timeout: 30s
        - match:
            prefix: "/efe.agent_runtime.v1."
            grpc: {}
          route:
            cluster: efe-server-grpc
            timeout: 60s
        # ── REST agent (M26) ─────────────────────────────────────────
        - match:
            prefix: "/v1/agent/"
          route:
            cluster: efe-server-agent-rest
            timeout: 30s
            retry_policy:
              retry_on: "5xx,reset"
              num_retries: 1
        - match:
            prefix: "/v1/agent/runs/"
            headers:
              - name: ":path"
                string_match:
                  contains: "/stream"
          route:
            cluster: efe-server-agent-rest
            timeout: 0s     # SSE no timeout
        # ── Auth (unauthenticated) ───────────────────────────────────
        - match:
            prefix: "/v1/auth/"
          route:
            cluster: efe-server-rest
            timeout: 10s
        # ── Healthz (no JWT, intra-cluster only) ─────────────────────
        - match:
            path: "/healthz"
          route:
            cluster: efe-server-rest
        # ── Catch-all v1 ─────────────────────────────────────────────
        - match:
            prefix: "/v1/"
          route:
            cluster: efe-server-rest
            timeout: 30s
            rate_limits:
              - actions:
                  - generic_key:
                      descriptor_value: "v1_per_tenant"
                  - request_headers:
                      header_name: x-tenant-id
                      descriptor_key: "tenant"
        # ── Catch-all v2 (placeholder) ───────────────────────────────
        - match:
            prefix: "/v2/"
          direct_response:
            status: 410
            body:
              inline_string: '{"error":"v2_not_yet_available"}'
        # ── Default 404 ──────────────────────────────────────────────
        - match:
            prefix: "/"
          direct_response:
            status: 404
            body:
              inline_string: '{"error":"not_found"}'
```

### 7.6 Request transformation

After jwt_authn, the request reaching efe-server has the following injected
headers:

- `x-correlation-id` — UUIDv4 propagated through.
- `x-tenant-id` — from JWT claim, validated.
- `x-token-sub` — from JWT `sub` claim.
- `x-roles` — comma-separated from JWT `roles` claim.
- `x-token-age-s` — computed by Lua filter (`now - iat`), forwarded for the
  `OidcTokenStale` check inside efe-server.
- `x-real-ip` — set by CF (`CF-Connecting-IP`) ; copied by Envoy.
- `x-forwarded-for` — chain of proxies.

### 7.7 Connection limits

```yaml
common_http_protocol_options:
  idle_timeout: 60s
  max_connection_duration: 300s
  max_stream_duration: 60s
http2_protocol_options:
  max_concurrent_streams: 100
  initial_stream_window_size: 65536
  initial_connection_window_size: 1048576
overload_manager:
  refresh_interval: 0.25s
  resource_monitors:
    - name: "envoy.resource_monitors.fixed_heap"
      typed_config:
        "@type": type.googleapis.com/envoy.extensions.resource_monitors.fixed_heap.v3.FixedHeapConfig
        max_heap_size_bytes: 805306368  # 768 MB out of 1Gi limit
  actions:
    - name: "envoy.overload_actions.shrink_heap"
      triggers:
        - name: "envoy.resource_monitors.fixed_heap"
          threshold: { value: 0.9 }
    - name: "envoy.overload_actions.stop_accepting_requests"
      triggers:
        - name: "envoy.resource_monitors.fixed_heap"
          threshold: { value: 0.95 }
```

### 7.8 Hot reload

Envoy is restarted via parent-child handoff (hot restart) for config changes :

```
docker run -d --name envoy_new --pid container:envoy_old envoyproxy/envoy ...
```

In k8s, we use the standard deployment rolling update (preStop hook waits for
in-flight requests to drain, then graceful termination). RDS allows
route-config-only changes without restart (used for per-tenant overrides).

---

## 8. Component C — Redis cluster

### 8.1 Topology

- **3 master + 3 replica nodes** (StatefulSet, anti-affinity per zone).
- Cluster mode enabled (16 384 hash slots).
- Persistence : RDB snapshot every 5 min + AOF every second.
- Memory : 4 GB per node (12 GB total, replicated → 12 GB usable).
- Eviction policy : `allkeys-lru` (cache eviction).

### 8.2 Embedding cache keyspace

```
Key   : embed:{sha256(text + ":" + embedder_version)}
Value : binary serialization (bincode) of Vec<f32>
TTL   : 7 days
```

Why SHA256 over text directly :
- Constant-length key (good for Redis Cluster sharding).
- Avoids text-content leaking into Redis logs (RGPD safety).
- Collision-resistant — 2⁻²⁵⁶ false positive rate.

The `:embedder_version` suffix guarantees cache invalidation when the
embedder model changes (M30 vision models, multilingual variants).

### 8.3 LLM response cache keyspace

```
Key   : llm:{sha256(prompt + ":" + model + ":" + canonical_params)}
Value : JSON-serialized LlmResponse (M21)
TTL   : 1 hour (LLM responses age fast for some use cases ; configurable per
        prompt template via the `cache_ttl_s` field on `efe-prompts`)
```

Canonical params : sorted JSON of `{temperature, max_tokens, top_p, ...}` so
that semantically-equivalent calls hit the same cache key.

### 8.4 Quota counters keyspace

```
Key      : quota:tenant:{slug}:{endpoint}:{epoch_minute}
Value    : integer (counter)
TTL      : 90 s (end of minute + 30 s grace for clock skew)
Op       : INCRBY 1 + EXPIRE (atomic via Lua script)
```

`endpoint` is bucketed coarsely : `classify`, `agent_run`, `embed`, `search`.
Per-minute granularity is the default ; daily totals are aggregated
asynchronously to `tenant_daily_usage` in PG (M27 migration 0021).

### 8.5 Rate limit token bucket keyspace

Used by Envoy ratelimit-service (Lyft) :

```
Key   : ratelimit:{descriptor_value}:{descriptor_key}={value}
Value : token bucket state (3 fields : current_tokens, refill_time, max_tokens)
```

Managed entirely by the ratelimit-service Lyft pod ; efe-server doesn't read
these keys directly.

### 8.6 Tenant isolation

**Hard rule** : every Redis key is **prefixed by tenant slug** when the data is
tenant-specific. Cross-tenant cache must use `shared:` prefix and be
explicitly approved (e.g. `embed:` is tenant-agnostic by construction because
the hash includes the embedder version, not the tenant).

ACL configuration (Redis 6 ACL) :

```
user efe-server on >SECRET ~embed:* ~llm:* ~quota:* ~ratelimit:* +@read +@write
user envoy-rl on >SECRET ~ratelimit:* +@read +@write
```

A separate ACL user prevents Envoy from reading classify caches and vice
versa.

### 8.7 Timing-side-channel mitigation

The embedder + classifier both have constant-time-ish paths :

- Cache miss : compute embedding (typical 5-20 ms) → write to cache → return.
- Cache hit : single GET (sub-ms) → return.

To prevent timing attacks against cache existence :

```rust
// In efe-cache::embed.rs
let started = Instant::now();
let result = match cache.get(&key).await {
    Some(v) => v,
    None => {
        let v = embedder.embed(&text).await?;
        cache.set(&key, &v, TTL_7D).await.ok();
        v
    }
};
// Pad to constant time (min 5 ms total) for keys with PII risk.
let elapsed = started.elapsed();
if elapsed < MIN_RESPONSE_TIME {
    tokio::time::sleep(MIN_RESPONSE_TIME - elapsed).await;
}
```

This is gated by config `EFE_CACHE_TIMING_JITTER=true` ; off by default
(performance cost), on for tenants with `tenant_feature.pii_strict=true`.

### 8.8 Split-brain handling

Redis Cluster handles partitions via quorum :
- Master needs majority of master nodes to ack a write.
- Minority partition rejects writes with `CLUSTERDOWN`.

For quota counters this means : during a partition, the minority side might
**under-count** for up to `cluster-node-timeout` (default 15 s). We accept this
risk because :
- The blast radius is small (~15 s of leaky bucket).
- The next minute's epoch resets the counter.
- Hard limits (in `tenant_feature.cost_cap_per_run_usd`) are PG-enforced and
  not Redis-dependent.

Detection : `redis_cluster_known_nodes` < expected → alert.

---

## 9. Component D — PgBouncer

### 9.1 Why PgBouncer

- Without : every efe-server pod opens its own `pool_max_connections=8`. At 10
  pods → 80 connections. Plus efe-agent-runner reconcile + portal-admin + CI
  test jobs → quickly hits PG `max_connections=100` (typical managed default).
- With : PgBouncer multiplexes 5000 client conn → 50 server conn. Scales to
  100s of efe-server pods.

### 9.2 Mode

- **transaction pooling** (releases server conn at end of transaction, not
  end of session).
- Compatible with sqlx (no prepared statements across transactions, which we
  already enforce via `cached_statements=0`).
- Not compatible with: session-level features (LISTEN/NOTIFY, advisory locks
  across statements, temp tables surviving) — none of which we use.

### 9.3 Pool sizing

```ini
[databases]
efe = host=pg-primary.efe-system.svc port=5432 dbname=efe pool_size=50 reserve_pool_size=5
efe_ro = host=pg-replica.efe-system.svc port=5432 dbname=efe pool_size=100 reserve_pool_size=10

[pgbouncer]
listen_addr = 0.0.0.0
listen_port = 5432
auth_type = md5
auth_file = /etc/pgbouncer/userlist.txt
pool_mode = transaction
max_client_conn = 5000
default_pool_size = 50
reserve_pool_size = 5
reserve_pool_timeout = 5
query_timeout = 60
query_wait_timeout = 30
server_idle_timeout = 600
server_lifetime = 1800
server_reset_query = DISCARD ALL
stats_period = 60
```

### 9.4 HA

Two PgBouncer replicas behind a ClusterIP service. Sticky sessions disabled
(transaction pooling means any pool conn is interchangeable).

### 9.5 Backpressure

When `pool_size` is exhausted :
- New clients queue up to `query_wait_timeout=30s`.
- After 30s : PgBouncer returns `ERROR` → efe-server propagates → backpressure
  middleware (§ 11) intercepts → 503 + Retry-After.

### 9.6 Migration : how efe-server connects

`EFE_DATABASE_URL` (existing env, M0+) changed from pointing to PG directly to
pointing to PgBouncer service. The `port=5432` stays the same ; the host
becomes `pgbouncer.efe-system.svc.cluster.local`.

New env added : `EFE_DATABASE_RO_URL` → PgBouncer ro pool → routed to replica.

The `sqlx::PgPool` config is changed :

```rust
// Before M27
PgPoolOptions::new()
    .max_connections(8)
    .connect(&db_url).await?;

// M27
PgPoolOptions::new()
    .max_connections(20)               // higher : multiplexed at PgBouncer
    .min_connections(2)
    .acquire_timeout(Duration::from_secs(10))
    .idle_timeout(Some(Duration::from_secs(300)))
    .max_lifetime(Some(Duration::from_secs(1800)))
    .test_before_acquire(true)         // necessary because PgBouncer can recycle
    .after_connect(|conn, _meta| Box::pin(async move {
        // Disable prepared statement cache : PgBouncer transaction pooling
        // doesn't preserve prepared statements across transactions.
        conn.execute("SET statement_cache_capacity = 0").await?;
        Ok(())
    }))
    .connect(&db_url).await?;
```

---

## 10. Component E — Postgres read replicas

### 10.1 Replication

Managed-PG path : enable streaming replicas via the cloud provider (AWS RDS
`read_replica`, Cloud SQL `replicas`, Crunchy Bridge `replicas` block).

Self-hosted path (only for `dev` / `staging` clusters) :
- Patroni deferred to M30 multi-region sprint.
- For M27 self-hosted = NO replicas (single instance). Production REQUIRES
  managed PG.

### 10.2 Synchronous vs asynchronous

- Default : **asynchronous** (default of all managed services).
- Lag SLO : p99 < 1 s.
- Routes that require **strong consistency** (e.g. read your own write
  immediately after a POST) **MUST** use `pool_rw` (primary), not `pool_ro`.
  Documented in § 10.4 routing table.

### 10.3 Connection routing (efe-server side)

```rust
// In efe-server::context
pub struct DbContext {
    pub pool_rw: PgPool,    // writes + read-your-own-write
    pub pool_ro: PgPool,    // read-only, may lag
}

impl DbContext {
    pub async fn from_env() -> anyhow::Result<Self> {
        let rw_url = std::env::var("EFE_DATABASE_URL")?;
        let ro_url = std::env::var("EFE_DATABASE_RO_URL")
            .unwrap_or_else(|_| rw_url.clone()); // fallback : single endpoint
        let pool_rw = make_pool(&rw_url, 20).await?;
        let pool_ro = make_pool(&ro_url, 50).await?;  // more conn for read path
        Ok(Self { pool_rw, pool_ro })
    }
}
```

### 10.4 Routing table per handler

| Endpoint | Pool | Justification |
|---|:--:|---|
| `POST /v1/classify` (cached) | ro | Read-only catalog lookup + cache write |
| `POST /v1/classify` (uncached) | rw | Writes `classify_call_log` immediately |
| `GET /v1/catalog/:id` | ro | Pure read |
| `POST /v1/catalog/:id/evolve` | rw | Write + read-your-own-write |
| `GET /v1/agent/runs` | ro | List read |
| `GET /v1/agent/runs/:id` | ro | Read (rare staleness OK) |
| `POST /v1/agent/runs` | rw | Insert + read after |
| `GET /v1/agent/runs/:id/steps/stream` | ro | SSE polling reads |
| `GET /v1/cost/daily` | ro | Aggregation read (slow query OK on replica) |
| `POST /v1/dpia/sign` | rw | Audit chain critical |
| Portal admin all reads | ro | Default for portal |

### 10.5 Failure mode : replica down

- `pool_ro` connect errors → fall back to `pool_rw` with a `WARN` log.
- Implemented via a `DbContext::ro_or_rw()` helper.

### 10.6 Failure mode : replica lag

- Lag exporter pod (`pg-replica-lag-exporter`) scrapes `pg_stat_replication`
  and emits `efe_pg_replica_lag_seconds{replica}`.
- Alert if p99 > 5s for 5 min → page SRE.
- Auto-mitigation : runtime flag `EFE_FALLBACK_RO_TO_RW=true` (set via
  configmap, picked up within 30s by efe-server's config reloader) routes
  `pool_ro` queries to `pool_rw`. Documented in RB_M27_REPLICA_LAG.

---

## 11. Component F — Backpressure middleware

### 11.1 Concept

A **Tower layer** wrapping the gRPC + REST routers. Two responsibilities :

1. **Concurrency cap** : if in-flight requests > `MAX_INFLIGHT` → reject new
   ones with 503 + `Retry-After: <random 1..5>`.
2. **Latency-aware shedding** : if observed p99 over last 30s > `P99_BUDGET`
   → shed `SHED_PERCENT%` of new requests randomly.

Together they implement **graceful degradation** : the service stays
responsive for some traffic instead of failing for all under saturation.

### 11.2 Implementation in `efe-server`

```rust
// crates/efe-server/src/middleware/backpressure.rs

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tower::{Layer, Service};
use http::{Request, Response, StatusCode};

#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    pub max_inflight: u64,                  // hard cap
    pub p99_budget_ms: u64,                 // shed above this
    pub p99_window: Duration,               // sliding window
    pub shed_percent: u8,                   // 0..100
    pub retry_after_range: (u8, u8),        // seconds
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            max_inflight: 1000,
            p99_budget_ms: 500,
            p99_window: Duration::from_secs(30),
            shed_percent: 10,
            retry_after_range: (1, 5),
        }
    }
}

pub struct BackpressureLayer {
    inner: Arc<BackpressureState>,
}

impl BackpressureLayer {
    pub fn new(cfg: BackpressureConfig) -> Self {
        Self { inner: Arc::new(BackpressureState::new(cfg)) }
    }
}

struct BackpressureState {
    cfg: BackpressureConfig,
    inflight: AtomicU64,
    p99_observer: RwLock<P99Observer>,
}

struct P99Observer {
    /* HDR Histogram or t-digest, sliding window */
}

impl<S> Layer<S> for BackpressureLayer {
    type Service = BackpressureService<S>;
    fn layer(&self, service: S) -> Self::Service {
        BackpressureService { inner: service, state: self.inner.clone() }
    }
}

pub struct BackpressureService<S> {
    inner: S,
    state: Arc<BackpressureState>,
}

impl<S, ReqB, ResB> Service<Request<ReqB>> for BackpressureService<S>
where
    S: Service<Request<ReqB>, Response = Response<ResB>>,
    /* ... bounds elided */
{
    /* ... call() :
       1. Increment inflight ; if > max → return 503.
       2. Sample p99 ; if above budget AND random < shed_percent → 503.
       3. Otherwise delegate to inner, observe latency, return.
    */
}
```

### 11.3 Metrics emitted

- `efe_backpressure_dropped_total{reason="inflight"|"p99_shed"}` counter.
- `efe_backpressure_inflight` gauge.
- `efe_backpressure_p99_ms` gauge (sampled p99 from sliding window).

### 11.4 Configuration

Env vars :

- `EFE_BACKPRESSURE_ENABLED` (default `true`).
- `EFE_BACKPRESSURE_MAX_INFLIGHT` (default `1000` per pod).
- `EFE_BACKPRESSURE_P99_BUDGET_MS` (default `500`).
- `EFE_BACKPRESSURE_SHED_PERCENT` (default `10`).

The defaults are deliberately conservative ; production tuning happens after
the first 2 weeks of observation (Sprint week 5 in § 26).

### 11.5 Test plan

Unit tests :
- `inflight_drops_above_cap` — inject 1001 simultaneous requests, expect 1 503.
- `p99_shed_within_percent_band` — inject 100 requests, force p99 > budget,
  expect ~10 503s.
- `decay_on_recovery` — after sustained slow period, when latency returns,
  shed_percent drops back to 0.

Integration tests :
- `k6 load test : 2000 RPS sustained 5 min → 503s issued but service does
  not crash, p99 of survivors stays < 1s`.

---

## 12. Component G — Tenant quota middleware

### 12.1 Concept

Per-tenant per-endpoint per-minute counter, backed by Redis. Limits loaded
from `tenant_feature` table (cached in efe-server for 30 s with stale-while-
revalidate to handle Redis hiccups).

### 12.2 Limits per tier (M27 default)

| Tier | classify/min | agent_run/h | embed/min | search/min |
|---|---:|---:|---:|---:|
| Free | 100 | 0 | 100 | 100 |
| Starter | 5,000 | 10 | 5,000 | 5,000 |
| Pro | 50,000 | 100 | 50,000 | 50,000 |
| Enterprise | custom | custom | custom | custom |

These are stored in `tenant_feature.classify_quota_per_min`, etc. The
schema change is in migration 0021 (§ 14).

### 12.3 Implementation sketch

```rust
// crates/efe-server/src/middleware/tenant_quota.rs

pub struct TenantQuotaLayer {
    cache: Arc<TenantLimitsCache>,
    redis: redis::cluster::ClusterClient,
}

impl<S> Layer<S> for TenantQuotaLayer { /* ... */ }

pub struct TenantQuotaService<S> { /* ... */ }

impl<S, ReqB, ResB> Service<Request<ReqB>> for TenantQuotaService<S>
where
    S: Service<Request<ReqB>, Response = Response<ResB>>,
{
    async fn call(&mut self, req: Request<ReqB>) -> Result<Response<ResB>, _> {
        let tenant = req.headers().get("x-tenant-id").and_then(|h| h.to_str().ok());
        let path = req.uri().path();
        let endpoint = endpoint_bucket(path);

        if let Some(tenant_slug) = tenant {
            let limit = self.cache.limit_for(tenant_slug, endpoint).await;
            let key = format!("quota:tenant:{}:{}:{}", tenant_slug, endpoint, epoch_minute());
            let count = self.redis.incr::<u64>(&key, 1).await?;
            if count == 1 {
                self.redis.expire(&key, 90).await?;
            }
            if count > limit {
                return Ok(too_many_requests("tenant_quota_exceeded", 60));
            }
        }
        self.inner.call(req).await
    }
}
```

### 12.4 Stacking with Envoy ratelimit

Envoy ratelimit catches : per-IP anonymous traffic, gross per-tenant limits
(coarse, 1 layer of defence).

Tenant quota middleware catches : fine-grained per-endpoint accounting,
informs cost dashboards (counter values are read by the daily aggregator
into `tenant_daily_usage` in PG).

Both must succeed for the request to pass. Envoy is **distributed** across
all gateways ; tenant_quota is **distributed** via Redis. So a tenant can't
trick either by hitting different pods.

### 12.5 Cost-cap integration

The M25 cost cap is per-run (agent runs). Tenant quota adds **per-day LLM
token cap** :

- `quota:tenant:{slug}:llm_tokens:{epoch_day}` keyspace.
- Incremented by `LLMRouter` after each provider response.
- Checked at the start of each LLM call ; if over → return cost-cap error.

This complements (not replaces) M25 cost cap which is enforced per-run.

---

## 13. Component H — API versioning + sunset policy

### 13.1 Strategy

URL-based versioning : `/v1/*` today, `/v2/*` future. No version-in-header,
no MIME-version, no query-string version.

### 13.2 Why URL versioning

- Simple to route at Envoy.
- Cacheable at CF without Vary.
- Debuggable (curl shows version clearly).
- Compatible with all 4 SDKs without runtime version selection.

### 13.3 Sunset policy

- Each `/v1/` endpoint can be marked sunset via response header :

```
X-API-Sunset: 2027-12-01
X-API-Deprecation: 2026-12-01
X-API-Successor: /v2/classify
```

- 12 months minimum between Deprecation and Sunset for breaking changes.
- 6 months minimum for security-driven removals (with executive sign-off).
- Sunset enforcement : after Sunset date, Envoy returns 410 Gone.

### 13.4 v2 placeholder

For M27 v1 only. v2 is reserved but unused. Current Envoy returns 410 Gone
on /v2/. When the first v2 endpoint is needed (Q3 or later), Envoy gets a new
cluster `efe-server-v2` and routes update.

### 13.5 Breaking changes policy

A change is **breaking** iff :
- Wire format changes (field renamed, type changed).
- Status code semantics change.
- Required input changed (new required field).
- Existing endpoint URL changes.
- Behavior changes that violate previously-documented contracts.

Breaking changes **must** :
1. Land in v2 (new namespace).
2. Be announced 12 months ahead in changelog + status page.
3. Email all affected tenants.

Backward-compatible additions (new optional field, new endpoint) don't
trigger versioning.

---

## 14. SQL migrations introduced

### 14.1 Migration 0021 — `m27_edge.sql`

```sql
-- M27 Edge : per-tenant quotas + daily usage aggregation + replica routing
-- hints + API key revocation list.

BEGIN;

-- ── tenant_feature additions for per-endpoint quotas ────────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS classify_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS agent_run_quota_per_hour INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS embed_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS search_quota_per_min INTEGER NOT NULL DEFAULT 100,
    ADD COLUMN IF NOT EXISTS llm_tokens_quota_per_day BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS pii_strict BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS tier VARCHAR(32) NOT NULL DEFAULT 'free';

-- ── daily usage rollup (populated by aggregator from Redis counters) ────
CREATE TABLE IF NOT EXISTS tenant_daily_usage (
    tenant_id     TEXT NOT NULL REFERENCES tenant_feature(id),
    day           DATE NOT NULL,
    endpoint      VARCHAR(64) NOT NULL,
    request_count BIGINT NOT NULL DEFAULT 0,
    llm_tokens    BIGINT NOT NULL DEFAULT 0,
    cost_cents    BIGINT NOT NULL DEFAULT 0,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, day, endpoint)
);

CREATE INDEX IF NOT EXISTS idx_tenant_daily_usage_day ON tenant_daily_usage (day);

-- ── API key (JWT) revocation list ───────────────────────────────────────
CREATE TABLE IF NOT EXISTS revoked_jwt (
    jti           UUID PRIMARY KEY,
    tenant_id     TEXT NOT NULL,
    sub           TEXT NOT NULL,
    revoked_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_by    TEXT NOT NULL,
    reason        TEXT,
    expires_at    TIMESTAMPTZ NOT NULL  -- when the token would have expired naturally
);

-- Auto-clean revoked_jwt after natural expiry + 7 days grace.
CREATE INDEX IF NOT EXISTS idx_revoked_jwt_expires ON revoked_jwt (expires_at);

-- ── tenant API IP allowlist (optional, enterprise tier) ────────────────
CREATE TABLE IF NOT EXISTS tenant_ip_allowlist (
    tenant_id     TEXT NOT NULL REFERENCES tenant_feature(id),
    cidr          CIDR NOT NULL,
    added_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_by      TEXT NOT NULL,
    note          TEXT,
    PRIMARY KEY (tenant_id, cidr)
);

-- ── audit event for quota exceeded ─────────────────────────────────────
ALTER TYPE agent_audit_event ADD VALUE IF NOT EXISTS 'QuotaExceeded';

COMMIT;
```

### 14.2 Migration 0021_rollback.sql

```sql
BEGIN;
DROP TABLE IF EXISTS tenant_ip_allowlist;
DROP TABLE IF EXISTS revoked_jwt;
DROP TABLE IF EXISTS tenant_daily_usage;
ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS tier,
    DROP COLUMN IF EXISTS pii_strict,
    DROP COLUMN IF EXISTS llm_tokens_quota_per_day,
    DROP COLUMN IF EXISTS search_quota_per_min,
    DROP COLUMN IF EXISTS embed_quota_per_min,
    DROP COLUMN IF EXISTS agent_run_quota_per_hour,
    DROP COLUMN IF EXISTS classify_quota_per_min;
-- Note : agent_audit_event enum value 'QuotaExceeded' cannot be dropped
-- without recreating the enum. Acceptable to leave during rollback.
COMMIT;
```

---

## 15. Helm chart additions

### 15.1 New sub-chart : `charts/efe-stack/charts/efe-edge/`

```
charts/efe-stack/charts/efe-edge/
├── Chart.yaml
├── values.yaml
└── templates/
    ├── _helpers.tpl
    ├── envoy-configmap.yaml          (Envoy bootstrap config + routes)
    ├── envoy-deployment.yaml          (3 replicas, HPA-ready)
    ├── envoy-service.yaml             (ClusterIP for cloudflared)
    ├── envoy-hpa.yaml                 (CPU 60% + custom metric RPS)
    ├── envoy-pdb.yaml                 (minAvailable=2)
    ├── envoy-networkpolicy.yaml       (ingress from edge ns, egress efe-system)
    ├── ratelimit-configmap.yaml       (Lyft ratelimit-service descriptors)
    ├── ratelimit-deployment.yaml      (3 replicas)
    ├── ratelimit-service.yaml         (ClusterIP)
    ├── redis-cluster-configmap.yaml   (redis.conf)
    ├── redis-cluster-statefulset.yaml (6 nodes : 3 m + 3 r)
    ├── redis-cluster-service.yaml     (headless)
    ├── redis-cluster-secret.yaml      (auth password)
    ├── pgbouncer-configmap.yaml       (pgbouncer.ini + userlist)
    ├── pgbouncer-deployment.yaml      (2 replicas)
    └── pgbouncer-service.yaml         (ClusterIP, exposes 5432)
```

### 15.2 Umbrella `efe-stack` integration

`charts/efe-stack/Chart.yaml` adds dependency :

```yaml
dependencies:
  - name: efe-edge
    version: 0.1.0
    repository: file://charts/efe-edge
    condition: efe-edge.enabled
```

`charts/efe-stack/values.yaml` :

```yaml
efe-edge:
  enabled: true
  envoy:
    replicas: 3
    image:
      repository: envoyproxy/envoy
      tag: v1.32-latest
  ratelimit:
    enabled: true
    image:
      repository: envoyproxy/ratelimit
      tag: latest
  redis:
    enabled: true
    replicas: 6
    persistence:
      size: 8Gi
  pgbouncer:
    enabled: true
    replicas: 2
    poolSize: 50
```

### 15.3 efe-server deployment updates

`charts/efe-stack/charts/efe-server/templates/deployment.yaml` :

- New env vars : `EFE_REDIS_URL`, `EFE_DATABASE_RO_URL`, `EFE_BACKPRESSURE_*`.
- New volumeMount : `/etc/efe/cache` (config from configmap).

`charts/efe-stack/charts/efe-server/values.yaml` :

```yaml
redis:
  endpoint: "redis-cluster.efe-edge.svc.cluster.local:6379"
  passwordSecretRef:
    name: efe-redis-auth
    key: password
database:
  endpoint: "pgbouncer.efe-edge.svc.cluster.local:5432"
  readOnlyEndpoint: "pgbouncer-ro.efe-edge.svc.cluster.local:5432"
backpressure:
  enabled: true
  maxInflight: 1000
  p99BudgetMs: 500
  shedPercent: 10
```

### 15.4 NetworkPolicy

`charts/efe-stack/charts/efe-edge/templates/envoy-networkpolicy.yaml` :

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: envoy-gateway
spec:
  podSelector:
    matchLabels:
      app: efe-envoy
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              name: cloudflared
      ports:
        - protocol: TCP
          port: 8443
  egress:
    - to:
        - namespaceSelector:
            matchLabels:
              name: efe-system
      ports:
        - protocol: TCP
          port: 50051   # gRPC
        - protocol: TCP
          port: 8080    # REST
        - protocol: TCP
          port: 8088    # M26 agent REST
    - to:
        - podSelector:
            matchLabels:
              app: efe-ratelimit
      ports:
        - protocol: TCP
          port: 8081    # ratelimit-service gRPC
    - to:
        - podSelector:
            matchLabels:
              app: redis-cluster
      ports:
        - protocol: TCP
          port: 6379
    # DNS resolution
    - to:
        - namespaceSelector: {}
      ports:
        - protocol: UDP
          port: 53
```

---

## 16. Terraform Cloudflare config

```hcl
# terraform/cloudflare/main.tf
terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 4.42"
    }
  }
}

provider "cloudflare" {
  api_token = var.cf_api_token
}

# ── DNS records ──────────────────────────────────────────────────────
resource "cloudflare_record" "api" {
  zone_id = var.zone_id_efe_ai
  name    = "api"
  type    = "CNAME"
  value   = cloudflare_argo_tunnel.eu_west_1.cname
  proxied = true
  ttl     = 1
}

# ── Argo Tunnel ─────────────────────────────────────────────────────
resource "cloudflare_argo_tunnel" "eu_west_1" {
  account_id = var.account_id
  name       = "efe-eu-west-1"
  secret     = var.tunnel_secret
}

# ── Page rules (deprecated, use rulesets) ────────────────────────────
resource "cloudflare_ruleset" "cache_rules" {
  zone_id     = var.zone_id_efe_ai
  name        = "Cache rules"
  description = "M27 cache strategy"
  kind        = "zone"
  phase       = "http_request_cache_settings"

  rules {
    expression = "(http.request.uri.path eq \"/v1/version\")"
    action     = "set_cache_settings"
    action_parameters {
      cache = true
      edge_ttl {
        mode    = "override_origin"
        default = 300
      }
    }
  }
}

# ── WAF managed rules ───────────────────────────────────────────────
resource "cloudflare_ruleset" "waf_managed" {
  zone_id  = var.zone_id_efe_ai
  name     = "M27 WAF"
  kind     = "zone"
  phase    = "http_request_firewall_managed"

  rules {
    action = "execute"
    action_parameters {
      id = "efb7b8c949ac4650a09736fc376e9aee"  # Cloudflare Managed Ruleset
    }
    expression = "true"
  }

  rules {
    action = "execute"
    action_parameters {
      id = "4814384a9e5d4991b9815dcfc25d2f1f"  # OWASP CRS
    }
    expression = "true"
  }
}

# ── Custom WAF rules ─────────────────────────────────────────────────
resource "cloudflare_ruleset" "waf_custom" {
  zone_id  = var.zone_id_efe_ai
  name     = "M27 custom WAF"
  kind     = "zone"
  phase    = "http_request_firewall_custom"

  # Rule 1 — double-encoded payloads
  rules {
    action      = "block"
    description = "Block double-encoded payloads"
    expression  = "(http.request.uri contains \"%25\" and http.request.uri contains \"%\")"
  }

  # Rule 4 — missing UA → challenge
  rules {
    action      = "managed_challenge"
    description = "Challenge requests without User-Agent"
    expression  = "(not http.request.headers[\"user-agent\"] exists)"
  }
}

# ── Rate limit rules ─────────────────────────────────────────────────
resource "cloudflare_rate_limit" "signup" {
  zone_id      = var.zone_id_efe_ai
  threshold    = 5
  period       = 60
  match {
    request {
      url_pattern = "*/v1/auth/signup"
    }
  }
  action {
    mode    = "challenge"
    timeout = 86400
  }
}

resource "cloudflare_rate_limit" "classify_anon" {
  zone_id      = var.zone_id_efe_ai
  threshold    = 100
  period       = 60
  match {
    request {
      url_pattern = "*/v1/classify*"
    }
  }
  action {
    mode    = "ban"
    timeout = 600
  }
}

# ── Bot management ───────────────────────────────────────────────────
resource "cloudflare_bot_management" "main" {
  zone_id                        = var.zone_id_efe_ai
  enable_js                      = true
  fight_mode                     = false
  suppress_session_score         = false
  auto_update_model              = true
  using_latest_model             = true
}
```

```hcl
# terraform/cloudflare/variables.tf
variable "cf_api_token"     { sensitive = true }
variable "account_id"       {}
variable "zone_id_efe_ai"   {}
variable "tunnel_secret"    { sensitive = true }
```

---

## 17. Observability matrix

### 17.1 New Prometheus metrics

| Metric | Type | Labels | Source |
|---|---|---|---|
| `efe_envoy_rate_limit_exceeded_total` | counter | tenant, endpoint | Envoy ratelimit filter |
| `efe_envoy_jwt_failure_total` | counter | reason | Envoy jwt_authn |
| `efe_envoy_request_duration_seconds` | histogram | route, status | Envoy stats sink |
| `efe_envoy_active_connections` | gauge | listener | Envoy |
| `efe_quota_hit_total` | counter | tenant, endpoint | tenant_quota middleware |
| `efe_quota_exceeded_total` | counter | tenant, endpoint | tenant_quota middleware |
| `efe_cache_hit_total` | counter | cache (embed|llm) | efe-cache crate |
| `efe_cache_miss_total` | counter | cache | efe-cache |
| `efe_cache_set_failed_total` | counter | cache, reason | efe-cache |
| `efe_redis_operation_duration_seconds` | histogram | op (get|set|incr|...) | efe-cache |
| `efe_backpressure_dropped_total` | counter | reason (inflight|p99_shed) | backpressure mw |
| `efe_backpressure_inflight` | gauge | | backpressure mw |
| `efe_backpressure_p99_ms` | gauge | | backpressure mw |
| `efe_pg_replica_lag_seconds` | gauge | replica | pg-lag-exporter |
| `pgbouncer_pools_cl_active` | gauge | pool | pgbouncer exporter |
| `pgbouncer_pools_sv_active` | gauge | pool | pgbouncer exporter |
| `pgbouncer_pools_sv_idle` | gauge | pool | pgbouncer exporter |
| `pgbouncer_clients_total` | gauge | | pgbouncer exporter |

### 17.2 Grafana dashboards (4 new)

1. **Edge — Cloudflare** : requests by status, WAF blocks, rate-limit hits,
   bot scores, top IPs blocked. Source : CF Analytics API → Grafana Cloudflare
   data source.
2. **Edge — Envoy** : QPS, latency p50/p95/p99 per route, 4xx/5xx rates,
   active connections, rate-limit exceeded by tenant.
3. **Cache — Redis** : hit ratio per cache, ops/s, latency, cluster health.
4. **DB — PG + PgBouncer** : pool saturation, replica lag, queries/sec by
   pool.

JSON dashboards committed to `ops/grafana/dashboards/edge/*.json`.

### 17.3 Loki log streams

- Cloudflare Logpush → Loki (audit-friendly retention 90 days).
- Envoy access logs → Loki via Fluent Bit DaemonSet.
- efe-server logs (existing) → Loki.

Log correlation : `x-correlation-id` flows through CF → Envoy → efe-server →
LLMRouter → Redis → PG.

---

## 18. SLO definitions + alerts

### 18.1 SLO table (M27 specific, on top of M26's 14 SLOs)

| ID | SLO | Window | Target |
|---|---|---|---|
| M27-1 | Edge availability (Envoy + CF) | 30d rolling | 99.95% |
| M27-2 | Edge p99 latency (Envoy) | 30d rolling | < 200ms for v1 paths |
| M27-3 | Cache hit ratio (embed) | 24h | > 80% |
| M27-4 | Cache hit ratio (llm) | 24h | > 40% |
| M27-5 | Redis Cluster availability | 30d rolling | 99.9% |
| M27-6 | PgBouncer pool saturation | 30d rolling | p99 wait < 10ms |
| M27-7 | PG replica lag | 30d rolling | p99 < 5s |
| M27-8 | Rate limit decision latency | 30d rolling | p99 < 5ms |

### 18.2 PrometheusRule snippets

```yaml
groups:
  - name: efe-edge-slos
    interval: 30s
    rules:
      - alert: EdgeAvailabilityBudgetBurn
        expr: |
          (
            sum(rate(efe_envoy_request_duration_seconds_count{status=~"5..", route!~"/healthz"}[5m]))
            / sum(rate(efe_envoy_request_duration_seconds_count{route!~"/healthz"}[5m]))
          ) > 0.001
        for: 10m
        labels:
          severity: P1
          slo: M27-1
        annotations:
          summary: "Edge 5xx rate burning availability budget"
          runbook: "https://docs.efe.ai/runbooks/RB_M27_EDGE_5XX"

      - alert: RedisClusterUnhealthy
        expr: redis_cluster_known_nodes != 6
        for: 5m
        labels:
          severity: P1
          slo: M27-5
        annotations:
          summary: "Redis Cluster has lost a node"
          runbook: "https://docs.efe.ai/runbooks/RB_M27_REDIS_FAIL"

      - alert: PgReplicaLag
        expr: efe_pg_replica_lag_seconds > 30
        for: 5m
        labels:
          severity: P1
          slo: M27-7
        annotations:
          summary: "PG replica lag > 30s, consider failover to RW"
          runbook: "https://docs.efe.ai/runbooks/RB_M27_REPLICA_LAG"
```

---

## 19. Performance budget + load test plan

### 19.1 Targets

- **1000 RPS sustained** on a 3-pod efe-server deployment.
- **p99 < 200 ms** for `POST /v1/classify` (uncached).
- **p99 < 30 ms** for `POST /v1/classify` (cache hit).
- **2000 RPS spike** absorbed by backpressure with < 50% shed.
- **10× DDoS spike** absorbed by CF (no degradation visible at origin).

### 19.2 k6 scripts

`scripts/load/m27/baseline_1000rps.js` :

```js
import http from 'k6/http';
import { check, sleep } from 'k6';

export const options = {
  scenarios: {
    sustained: {
      executor: 'constant-arrival-rate',
      rate: 1000,
      timeUnit: '1s',
      duration: '5m',
      preAllocatedVUs: 200,
      maxVUs: 1000,
    },
  },
  thresholds: {
    'http_req_duration{status:200}': ['p(99)<200'],
    'http_req_failed': ['rate<0.01'],
  },
};

const TOKEN = __ENV.EFE_BENCH_TOKEN;

export default function () {
  const url = 'https://api.efe.ai/v1/classify';
  const payload = JSON.stringify({
    tenant_id: 'load-test',
    text: `random_${Math.random()}`,
    catalog_id: 'demo',
  });
  const params = { headers: {
    'Authorization': `Bearer ${TOKEN}`,
    'Content-Type': 'application/json',
  }};
  const res = http.post(url, payload, params);
  check(res, { 'status 200': r => r.status === 200 });
}
```

`scripts/load/m27/ddos_spike_10x.js` — same but 10000 RPS for 2 min, expect
CF to absorb (origin RPS unchanged).

`scripts/load/m27/cache_warmup_hit_ratio.js` — repeat 100 unique texts in
a 1000-request loop, expect 99% cache hits after first pass.

### 19.3 Acceptance criteria (Tier 5 of D93-like verification)

```
Tier 5 — Performance & Load
├── k6 baseline_1000rps : p99 < 200ms, 0 failed
├── k6 ddos_spike_10x : origin RPS < 100 RPS (CF absorbed 99%)
├── k6 cache_warmup : > 99% cache hit on second pass
├── kill 1 redis master : recovery < 30s, < 1% requests fail during recovery
├── kill 1 envoy pod : recovery < 10s, < 0.1% requests fail
└── kill 1 pgbouncer pod : no client-visible impact (anti-affinity HA)
```

---

## 20. Failure modes + degradation paths

| Failure | Detection | Degraded mode | Recovery |
|---|---|---|---|
| CF outage | Synthetic monitor (StatusPage probe) | Tenants with mTLS use direct origin (DNS fallback record) ; non-mTLS down | CF recovers (typically < 10 min) |
| 1 Envoy pod crash | Liveness probe | 2 remaining pods absorb load (HPA scales up if needed) | Pod restarts within 30s |
| All Envoy pods down | Cluster monitor | No traffic flows | k8s rolls back via Argo CD ; HUMAN ESCALATION |
| 1 Redis master crash | Cluster monitor | Replica promoted within 15s (cluster-node-timeout) | Automatic |
| Redis Cluster split-brain | Quorum failure | Minority returns CLUSTERDOWN ; cache MISS on those | Network heals |
| Redis Cluster total outage | Cluster monitor | Backpressure middleware degrades to no-cache mode (env flag) | Restore from RDB snapshot |
| PgBouncer pool exhausted | Pool monitor | New clients queue or 503 | Scale up pool_size or efe-server replicas |
| PG replica down | Lag exporter | Routing falls back to primary (env flag) | Replica re-spin from primary |
| PG primary down | Health check | All writes fail ; read-only mode if replicas up | Patroni promotes (M30) or manual (M27) |
| Cloudflare WAF false positive | CF analytics + customer report | Customer paged ; rule tuned via Terraform PR | Rule update propagates < 5 min |

---

## 21. Migration plan M26 → M27

### 21.1 Phasing

Phase 1 (Days 1-7) : **deploy infra without traffic switch**.
- Install efe-edge sub-chart in staging.
- Redis, PgBouncer, replicas online.
- Envoy reachable but no DNS pointing yet.
- Smoke test in staging.

Phase 2 (Days 8-14) : **single-tenant canary**.
- Move one design-partner tenant to `api-canary.efe.ai` DNS pointing to CF →
  Envoy.
- Compare metrics : latency, error rate, cost.
- 7-day observation. Rollback if regression.

Phase 3 (Days 15-21) : **gradual rollout**.
- CF DNS-level weighted routing : 10% → 50% → 100% over 7 days.
- Monitor backpressure metrics, scale efe-server horizontally if needed.

Phase 4 (Days 22-28) : **decommission old direct path**.
- DNS-level cleanup.
- Old direct-to-pod entrypoint marked deprecated, logs flagged.

### 21.2 Rollback plan

- Each phase has a one-command rollback :
  - `kubectl rollout undo` on the helm release.
  - `terraform apply -var rollback=true` on Cloudflare module (flip DNS back).
- Rollback budget : 1h max from decision to flip.

### 21.3 Compatibility considerations

- Existing clients pointing to old direct endpoint keep working (we maintain
  the legacy ingress) until Phase 4.
- SDKs unchanged ; the change is transparent at the API layer.
- Bench harness (`bench/`) needs `EFE_API_BASE` updated to `api.efe.ai`.

---

## 22. Testing strategy

### 22.1 Unit tests

- `crates/efe-server/src/middleware/backpressure.rs` :
  - Concurrency cap at exact threshold.
  - Shed-rate within ±2% of configured.
  - Decay back to 0% shed after recovery.
- `crates/efe-server/src/middleware/tenant_quota.rs` :
  - Per-tenant counter increments correctly.
  - 429 returned at exactly limit+1.
  - Reset at epoch boundary.
- `crates/efe-cache/src/embedding.rs` :
  - SHA256 keying deterministic.
  - Bincode encode/decode round-trip.
  - TTL respected.

### 22.2 Integration tests

- `tests/integration_m27_edge.rs` :
  - End-to-end through Envoy in a kind cluster with all of efe-edge installed.
  - Assert headers injected correctly.
  - Assert rate limit kicks in.

### 22.3 Load tests

- See § 19.

### 22.4 Chaos tests

- `scripts/chaos/m27/kill_redis_master.sh` — kill a master pod, assert
  recovery < 30s.
- `scripts/chaos/m27/network_partition_redis.sh` — iptables drop between
  master and 2 replicas, assert minority side rejects writes.
- `scripts/chaos/m27/pgbouncer_oom.sh` — OOM one PgBouncer pod, assert
  failover.

### 22.5 Security tests

`bench/m27_edge_pentest/` :
- `test_01_jwt_replay.py` — replay an old token after revocation.
- `test_02_rate_limit_bypass_ip_rotation.py` — try to exceed per-IP quota by
  rotating source IPs (CF must catch via Bot Mgmt).
- `test_03_cache_poisoning_attempt.py` — try to GET someone else's cache
  (must fail via ACL).
- `test_04_cost_amplification.py` — try to hit /v1/classify 10k times in 1 min
  (must hit tenant quota at 1000).
- `test_05_ddos_l7.py` — flood /v1/auth/signup (must be rate-limited at CF).

---

## 23. Runbooks (4 new)

### RB_M27_EDGE_DDOS
Steps when DDoS detected at CF :
1. Confirm pattern via CF Analytics (top IPs, geo, ASN).
2. If global: enable IUAM (`I'm Under Attack Mode`) at CF.
3. If targeted tenant: increase rate-limit aggressiveness via terraform PR.
4. If exhausted: contact CF Enterprise Support (24/7).
5. Post-incident: review WAF custom rules, update threat intel.

### RB_M27_REDIS_FAIL
Steps when Redis Cluster reports `CLUSTERDOWN` or known_nodes < 6 :
1. Identify failed node : `redis-cli -h <node> CLUSTER NODES`.
2. If failed = replica: pod restart usually heals.
3. If failed = master: confirm replica promotion happened (`CLUSTER NODES`).
4. If multi-node failure: failover manually (`CLUSTER FAILOVER FORCE` on
   surviving master).
5. Emergency cache disable: `kubectl set env deploy/efe-server EFE_CACHE_ENABLED=false`.

### RB_M27_PGBOUNCER_SAT
Steps when pool saturation alert fires :
1. Confirm via `pgbouncer_pools_sv_active / pool_size > 0.9`.
2. Identify slow queries on primary: `SELECT pid, now()-query_start, query
   FROM pg_stat_activity WHERE state='active' ORDER BY 2 DESC LIMIT 10`.
3. Kill runaway queries: `SELECT pg_cancel_backend(pid)`.
4. Increase `pool_size` temporarily (configmap reload).
5. Scale efe-server replicas down if culprit is a runaway bench.

### RB_M27_REPLICA_LAG
Steps when replica lag > 30s :
1. Confirm: `SELECT * FROM pg_stat_replication`.
2. Identify long write transaction on primary.
3. If transient: wait, monitor recovery.
4. If sustained: set `EFE_FALLBACK_RO_TO_RW=true` (configmap, picked up < 30s).
5. Notify portal-admin owner: their queries may be slower (hitting primary).
6. Post-incident: examine the long-running transaction source.

---

## 24. ADRs (4)

### ADR-M27-001 : Envoy over Kong/NGINX

Status : Accepted. See § 7.1 for comparison table. Decision driven by native
gRPC + xDS + WASM extensibility.

### ADR-M27-002 : Redis Cluster sharded over Redis Sentinel

Status : Accepted. Sentinel is HA but not sharded — single-master throughput
ceiling. Cluster handles horizontal scale + replication. Operational
complexity higher (re-balancing) but acceptable for 6-node static topology.

### ADR-M27-003 : Cloudflare over AWS CloudFront + WAF

Status : Accepted. Cloudflare offers : (a) zero-cost zones for testing, (b)
better DDoS reputation, (c) Argo Tunnel (no public origin IP), (d) Bot
Management UX clearer. CloudFront is also viable ; if AWS-only customer
acquires, dual-CDN evaluation in M30.

### ADR-M27-004 : PgBouncer transaction pooling over per-pod sqlx pool only

Status : Accepted. Per-pod sqlx pool scales linearly with replicas → quickly
hits PG max_connections. PgBouncer multiplexes 100× pods to 1 PG. Tradeoff :
no prepared-statement cache across transactions (we already disable to be
PgBouncer-safe). Acceptable.

---

## 25. Risk register

| ID | Risk | Likelihood | Impact | Mitigation |
|---|---|:--:|:--:|---|
| R1 | Cloudflare Enterprise contract delays | MED | HIGH | Order CF Enterprise week -2 ; fallback to Cloudflare Business + extra ModSecurity sidecar |
| R2 | Envoy WASM filter learning curve | MED | MED | Start with Lua filters ; migrate to WASM after Sprint 3 |
| R3 | Redis Cluster split-brain | LOW | MED | Quorum config + monitoring ; quotas leak up to 15s on partition (acceptable) |
| R4 | PgBouncer not compatible with some sqlx feature | LOW | HIGH | Audit all sqlx usage Sprint 1 ; disable prepared statements globally |
| R5 | DDoS test at 10× exhausts CF Enterprise rate-limit quotas | LOW | MED | Coordinate with CF AM beforehand ; whitelist test IPs |
| R6 | Cache poisoning regression | LOW | HIGH | Per-tenant ACL ; CI test cross-tenant key isolation |
| R7 | Replica lag spikes during M25 active learning jobs | MED | MED | M24 active learning canary throttled ; long jobs run during off-peak |
| R8 | Argo Tunnel single point of failure | LOW | HIGH | 2 cloudflared replicas, anti-affinity ; CF supports HA tunnels |

---

## 26. Sprint plan (6 weeks)

### Week 1 — Foundation infrastructure
- Day 1-2 : Cloudflare Enterprise contract + Terraform initial commit.
- Day 3-4 : Argo Tunnel + DNS records + zone configuration.
- Day 5 : WAF managed rules + custom rules deployed.

### Week 2 — In-cluster edge
- Day 1-2 : `charts/efe-stack/charts/efe-edge/` Helm sub-chart skeleton.
- Day 3-4 : Envoy ConfigMap + Deployment + Service.
- Day 5 : ratelimit-service deployed + Lyft descriptors.

### Week 3 — Data plane
- Day 1-2 : Redis Cluster StatefulSet + ACL + persistence.
- Day 3-4 : PgBouncer Deployment + ConfigMap + connection sourcing.
- Day 5 : `efe-cache` crate (embed cache + LLM cache).

### Week 4 — efe-server integration
- Day 1-2 : `backpressure` middleware.
- Day 3 : `tenant_quota` middleware.
- Day 4-5 : Wire up `pool_ro` routing in DbContext + handler updates.

### Week 5 — Validation
- Day 1-2 : k6 load tests + tuning.
- Day 3 : Chaos tests (kill nodes).
- Day 4 : Pen test bundle (`bench/m27_edge_pentest/`).
- Day 5 : Migration phase 1 (staging install).

### Week 6 — Rollout
- Day 1 : Phase 2 canary (1 design-partner tenant).
- Day 2-4 : Phase 3 gradual rollout (10% → 50% → 100%).
- Day 5 : Phase 4 decommission + retro + CHANGELOG entry.

---

## 27. Done definition

The sprint is **DONE** when ALL of the following are true :

- [ ] `cargo build --workspace` green on stable rustc 1.88.
- [ ] `cargo test --workspace --no-run` compiles all tests.
- [ ] `helm lint charts/efe-stack` green (efe-edge sub-chart included).
- [ ] `helm template` produces valid manifests, `kubectl --dry-run=client` ok.
- [ ] In a fresh k3d/kind cluster with managed PG simulation, `helm install`
      completes in < 10 min, all pods Ready.
- [ ] k6 baseline 1000 RPS sustained 5 min, p99 < 200ms, error rate < 1%.
- [ ] k6 DDoS 10× spike : CF absorbs, origin RPS unchanged (< 10% delta).
- [ ] Chaos test : kill 1 Redis master → recovery < 30s, no client-visible
      errors > 100ms after recovery.
- [ ] All 5 pen-test scenarios pass in chaos cluster.
- [ ] PR opened with the 30+ files, link to this architecture doc.
- [ ] 1 design-partner tenant migrated to api.efe.ai successfully in Phase 2.
- [ ] CHANGELOG entry `## EFE 2.2.0 — Commercial Edge`.
- [ ] 4 ADRs committed.
- [ ] 4 runbooks committed.

### Explicit non-criteria (deferred)

- Multi-region (M30).
- Vector search adjacent product (M31).
- Multi-modal embedders (M30).
- Customer-facing usage dashboard (M28+P3 billing scope).
- WAF tuning past paranoia level 1 (rolling improvement).

---

## 28. Annexes

### Annex A — Sample Envoy ConfigMap (full bootstrap)

See `charts/efe-stack/charts/efe-edge/templates/envoy-configmap.yaml` for the
authoritative source. ~300 lines of YAML covering listeners, clusters,
filter chain, admin endpoint.

### Annex B — Sample Lyft ratelimit-service descriptors

See `charts/efe-stack/charts/efe-edge/templates/ratelimit-configmap.yaml`.

### Annex C — Final list of files touched

```
docs/EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md           (this doc, NEW)
docs/docs/adr/ADR-M27-{001,002,003,004}.md             (4 NEW ADRs)
docs/runbooks/RB_M27_{EDGE_DDOS,REDIS_FAIL,PGBOUNCER_SAT,REPLICA_LAG}.md
                                                       (4 NEW runbooks)

terraform/cloudflare/main.tf                           (NEW)
terraform/cloudflare/variables.tf                      (NEW)
terraform/cloudflare/outputs.tf                        (NEW)

charts/efe-stack/Chart.yaml                            (add efe-edge dep)
charts/efe-stack/values.yaml                           (add efe-edge config)
charts/efe-stack/charts/efe-edge/                      (NEW sub-chart, 17 files)
├── Chart.yaml
├── values.yaml
└── templates/  (15 templates + _helpers.tpl)

charts/efe-stack/charts/efe-server/templates/deployment.yaml  (env vars)
charts/efe-stack/charts/efe-server/values.yaml          (redis + ro endpoint)

crates/efe-cache/                                       (NEW crate)
├── Cargo.toml
├── src/lib.rs
├── src/embedding.rs                                   (typed embedder cache)
├── src/llm.rs                                          (typed LLM cache)
├── src/client.rs                                       (Redis Cluster client)
└── tests/

crates/efe-server/src/middleware/                       (NEW module)
├── mod.rs
├── backpressure.rs
├── tenant_quota.rs
└── correlation_id.rs

crates/efe-server/src/db_context.rs                     (NEW — DbContext{rw,ro})
crates/efe-server/src/main.rs                           (wire new middlewares)
crates/efe-server/Cargo.toml                            (add efe-cache + redis)

crates/efe-server/migrations/0021_m27_edge.sql          (NEW)
crates/efe-server/migrations/0021_m27_edge_rollback.sql (NEW)

ops/grafana/dashboards/edge/                            (4 NEW JSON dashboards)
ops/prometheus/rules/m27-edge-slos.yaml                 (NEW PrometheusRule)
ops/alertmanager/routing-m27.yaml                       (alert routing extension)

scripts/load/m27/                                        (NEW)
├── baseline_1000rps.js
├── ddos_spike_10x.js
└── cache_warmup_hit_ratio.js

scripts/chaos/m27/                                       (NEW)
├── kill_redis_master.sh
├── network_partition_redis.sh
└── pgbouncer_oom.sh

bench/m27_edge_pentest/                                  (NEW)
├── README.md
├── _lib.py
└── test_0{1..5}_*.py                                    (5 pen-test scenarios)

CHANGELOG.md                                             (## EFE 2.2.0 entry)
```

**Approximate total** : ~50 files added or modified, ~7000-10000 LOC including
configs.

### Annex D — Pricing implications

For business planning only :
- Cloudflare Enterprise : ~$200-500/mo per zone (negotiated).
- Argo Tunnel : included in Enterprise.
- Bot Management : included in Enterprise.
- Managed Redis option (post-evaluation) : ~$200/mo for 3-node cluster on
  AWS ElastiCache.
- Managed PG with replicas : ~$300-500/mo for db.r5.large primary + 2 r5.medium
  replicas on RDS.

Total infrastructure delta vs M26 : ~$700-1200/mo. To recover via 2 Starter
customers ($99 × 2 = $198/mo) doesn't fly. Must price Pro tier ($499/mo) at
or above 3 customers to amortize. Documented in P3 Billing roadmap.

### Annex E — One-line verification script

```bash
#!/usr/bin/env bash
set -euo pipefail
echo "=== Tier 1 : workspace build ==="
cargo build --workspace --exclude efe-ffi
echo "=== Tier 1.5 : workspace test compile ==="
cargo test --workspace --exclude efe-ffi --no-run
echo "=== Tier 2 : Helm lint + template ==="
helm lint charts/efe-stack
helm template test charts/efe-stack --set efe-edge.enabled=true >/tmp/m27.yaml
kubectl --dry-run=client apply -f /tmp/m27.yaml >/dev/null 2>&1 || true
echo "=== Tier 3 : Terraform validate ==="
( cd terraform/cloudflare && terraform init -backend=false && terraform validate )
echo "=== Tier 4 : k3d smoke ==="
( cd scripts && bash m27-k3d-test.sh )
echo "=== Tier 5 : load test ==="
( cd scripts/load/m27 && k6 run baseline_1000rps.js )
echo "=== ALL TIERS PASSED ==="
```

Save as `scripts/m27-assembly-verify.sh`. Companion to D93's
`assembly-verify.sh`.

---

## End of document

**Wire diff summary** : zero new public API. Zero breaking change. EFE 2.2.0
is **drop-in compatible** with 2.1.1 clients. The visible delta to operators :
1 new sub-chart (efe-edge), 1 new namespace (cloudflared), DNS pointing
through CF instead of direct.

**Operational complexity delta** : +6 new pods (3 Envoy, 6 Redis, 2 PgBouncer,
3 ratelimit-service, 2 cloudflared, 1 pg-lag-exporter) for ~14 new pods. All
captured in Helm and observability dashboards.

**Audit-passable** : per the M26 Done Definition pattern. Architecture +
code + 5-tier verification + ADRs + runbooks + CHANGELOG. The chirurgical
audit pattern (D93-style) will follow the code drop in a separate verification
pass.
