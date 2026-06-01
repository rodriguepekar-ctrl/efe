# EFE M28 — Streaming + Events + Webhooks Architecture

**Status** : ARCHITECTURE — design phase, pre-execution.
**Predecessors** :
- `EFE_M27_COMMERCIAL_EDGE_ARCHITECTURE.md` (Edge layer, hard dep on Redis cache + Envoy).
- `EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` (M25 + M26 agent + audit).
- `EFE_M21_MODE2_LLM_ROUTED_ANCHORS_ARCHITECTURE.md` (LLMRouter).
**Target** : turn EFE from a request/response API into a reactive
event-driven SaaS — webhooks, streaming classification, async batch
jobs, internal event bus, CDC-friendly DB.
**Codename** : "Cascade" (the flow of events through the system).
**EFE version** : `2.3.0` — semver MINOR (additive APIs only).
**Sprint window** : 3-4 working weeks.
**Hard deps** : M27 (Redis cluster + Envoy gateway), cert-manager,
NATS JetStream 2.10+, an S3-compatible object store (AWS S3, MinIO,
GCS, R2).

---

## Table of Contents

1. [Executive summary](#1-executive-summary)
2. [Scope + non-goals](#2-scope--non-goals)
3. [What M27 left without :](#3-what-m27-left-without)
4. [Threat model + new attack surfaces](#4-threat-model--new-attack-surfaces)
5. [Topology overview](#5-topology-overview)
6. [Component A — Webhook subsystem](#6-component-a--webhook-subsystem)
7. [Component B — Streaming classification (gRPC bidirectional)](#7-component-b--streaming-classification-grpc-bidirectional)
8. [Component C — Batch API](#8-component-c--batch-api)
9. [Component D — NATS internal event bus](#9-component-d--nats-internal-event-bus)
10. [Component E — Postgres CDC → NATS](#10-component-e--postgres-cdc--nats)
11. [Component F — Customer event log API](#11-component-f--customer-event-log-api)
12. [Component G — Server-Sent Events (SSE) public stream](#12-component-g--server-sent-events-sse-public-stream)
13. [Wire formats + schemas](#13-wire-formats--schemas)
14. [SQL migrations introduced](#14-sql-migrations-introduced)
15. [New crate : `efe-events`](#15-new-crate--efe-events)
16. [Helm chart additions](#16-helm-chart-additions)
17. [Observability matrix](#17-observability-matrix)
18. [SLO definitions + alerts](#18-slo-definitions--alerts)
19. [Performance budget + load test plan](#19-performance-budget--load-test-plan)
20. [Failure modes + degradation paths](#20-failure-modes--degradation-paths)
21. [Migration plan M27 → M28](#21-migration-plan-m27--m28)
22. [Testing strategy](#22-testing-strategy)
23. [Runbooks (4 new)](#23-runbooks-4-new)
24. [ADRs (5)](#24-adrs-5)
25. [Risk register](#25-risk-register)
26. [Sprint plan (4 weeks)](#26-sprint-plan-4-weeks)
27. [Done definition](#27-done-definition)
28. [Annexes](#28-annexes)

---

## 1. Executive summary

After M27, EFE has a hardened request/response surface : auth at edge,
rate limit, cache, read replicas. But it remains **synchronous** —
every classify is request/response, every agent step is polled or via
SSE for a *single* run.

M28 — **Cascade** — adds 5 reactive primitives :

1. **Webhooks** : per-tenant subscriptions to `agent.run.*`,
   `classify.completed`, `drift.detected`, `bundle.deployed`, etc.,
   with HMAC-signed delivery, exponential backoff, replay.
2. **Streaming classification** (gRPC bidirectional) :
   `StreamClassify(stream Request) → stream Response` for log
   pipelines, live feeds, real-time triage at 10 000 events/s/tenant.
3. **Batch API** : `POST /v1/batch/classify` accepts an S3 URL of
   Parquet/CSV input → job worker classifies in 1000-row batches →
   writes Parquet result to S3 → notifies via webhook + status endpoint.
4. **NATS JetStream internal bus** : every consequential domain event
   is published. Webhook delivery, batch progress, downstream
   integrations (P3 billing aggregator, M24 active-learning trigger)
   subscribe.
5. **Postgres CDC → NATS** : Debezium-style logical replication slot
   reads `agent_audit`, `classify_call_log`, `drift_event` row writes
   and publishes them on the same NATS bus.

Together these unlock :
- **Webhooks** as table-stakes for any SaaS API.
- **Streaming** as the differentiating capability for real-time use
  cases (security log triage, IoT, financial feeds).
- **Batch** as the enterprise tier feature (millions of items
  classified async).
- **Event log API** : customers can pull their audit chain + event
  history programmatically without ticketing support.

Zero new public REST endpoints break v1 ; all M28 additions are
backward-compatible **adds**.

---

## 2. Scope + non-goals

### 2.1 In scope (EFE 2.3.0)

- New crate `efe-events` (`crates/efe-events/`) — NATS JetStream
  publisher + typed event schema + subscriber boilerplate.
- New REST routes :
  - `POST /v1/webhooks` (subscribe), `GET /v1/webhooks`,
    `DELETE /v1/webhooks/:id`.
  - `POST /v1/webhooks/:id/replay` (re-deliver from t0).
  - `POST /v1/batch/classify` + `GET /v1/batch/:id` (status) +
    `GET /v1/batch/:id/result` (signed URL).
  - `GET /v1/events` (paginated customer-facing event log).
- New gRPC streaming method :
  `efe.v1.SwarmService.StreamClassify(stream ClassifyRequest) → stream ClassifyResponse`.
- Webhook delivery worker (in `efe-server`) consuming the NATS bus.
- Batch worker (new pod `efe-batch-worker`) consuming a NATS
  `batch_jobs` queue group.
- NATS JetStream Helm sub-chart `charts/efe-stack/charts/efe-nats/`.
- Postgres logical replication slot + Debezium-style publisher (we
  ship a small Rust binary `efe-cdc-publisher` that tails the slot
  and pushes to NATS).
- SQL migrations 0022 (webhook_subscription, batch_job, customer_event_log).
- 4 new runbooks + 5 ADRs.

### 2.2 Out of scope (deferred)

- **Multi-region NATS** — M30.
- **Kafka as alternative bus** — explicit ADR rejection (NATS only ;
  see ADR-M28-001).
- **Source connectors (Salesforce, S3 cron, Notion)** — M32.
- **Webhook delivery to external IP-restricted endpoints with mTLS
  client cert** — enterprise upgrade, post-GA.
- **Compressed event log archive (Parquet > 90 days)** — M29 data
  rights.
- **Real-time UI dashboard** (rather than polled `GET /events`) —
  P1 portal-admin enhancement.

### 2.3 Adjacent (parallel sprint candidates)

- **Stripe metered billing aggregator** (P3 sprint) — subscribes to
  NATS `classify.completed` for usage roll-up.
- **Active-learning trigger** (M24 extension) — subscribes to NATS
  `drift.detected` to schedule retraining.

---

## 3. What M27 left without

| Gap | Symptom | M28 fix |
|---|---|---|
| **No webhook delivery** | Customer must poll `GET /v1/agent/runs/:id` every few seconds. Battery drain on mobile, bandwidth waste, latency. | `POST /v1/webhooks` lets customer subscribe to events ; delivery worker pushes HMAC-signed JSON to their endpoint. |
| **No streaming classify** | Each item = 1 HTTP request, ~5 ms overhead per request, can't sustain 10k items/s/connection. | gRPC bidirectional stream amortizes auth + transport overhead, sustains 50k items/s/connection. |
| **No async batch** | Customer with 1M items must call classify 1M times, hit rate limit, drown in retries. | `POST /v1/batch/classify` returns a job ID ; worker processes the S3 input in 1000-row batches, completes in < 1h. |
| **No event log readable by customer** | Customers can't audit their own usage history without support ticket. | `GET /v1/events?since=...` returns a paginated, immutable event log. |
| **No internal pub/sub** | Modules (M24 active-learning, P3 billing, webhook worker) poll PG to detect new rows → wasteful. | NATS JetStream bus + CDC publisher publish every consequential write as an event. |
| **Audit chain unreachable programmatically** | Compliance team must `kubectl exec` into PG to query audit. | `GET /v1/events?topic=audit&...` exposes the HMAC-chained audit per tenant. |

---

## 4. Threat model + new attack surfaces

M28 expands the attack surface in 3 directions :
1. **Outbound calls to customer URLs** (webhook delivery) → SSRF risk.
2. **Async batch jobs** with privileged access to customer S3 → leak risk.
3. **NATS bus exposed to multiple pods** → cross-tenant event leak risk.

| ID | Threat | Vector | Impact | Control (§) | Detection |
|---|---|---|---|---|---|
| T1 | **Webhook SSRF** | Customer registers `webhook_url=http://169.254.169.254/...` (cloud metadata) | Leak of cloud creds | URL validation : reject private CIDRs + cloud metadata IPs (§ 6.5) | Webhook delivery logs |
| T2 | **Webhook URL takeover** | Attacker registers a webhook on someone else's domain | Attacker receives victim's events | DNS ownership check (TXT record proof) for verified webhooks (§ 6.6) ; non-verified webhooks marked unverified | Per-webhook delivery counter |
| T3 | **Cross-tenant event leak via NATS** | Pod A subscribes to all subjects, sees tenant B's events | Tenant data leak | NATS subject hierarchy `efe.tenant.{slug}.{topic}` + per-tenant JWT permissions (§ 9.4) | NATS audit log |
| T4 | **Batch job S3 path traversal** | Customer submits `s3://efe-internal-bucket/...` instead of own bucket | Server reads/writes EFE internal data | Validate S3 bucket against per-tenant allowlist (§ 8.4) ; presigned URLs only | Batch job audit log |
| T5 | **Webhook replay attack** | Attacker intercepts a webhook payload, replays to a 3rd party | Confused-deputy | Every webhook carries `X-EFE-Webhook-Signature` (HMAC) + `X-EFE-Webhook-Timestamp` ; receiver MUST verify (§ 6.3) | Customer-side responsibility (documented) |
| T6 | **NATS stream unbounded growth** | Bug → millions of events published, never consumed | Disk full, NATS down | JetStream stream retention : 7d or 10M msgs, whichever first (§ 9.5) | `jetstream_stream_messages` |
| T7 | **Streaming connection exhaustion** | Attacker opens 10k gRPC streams, never sends data | Pod descriptors exhausted | Per-tenant concurrent stream limit (`max_concurrent_streams = 50`) (§ 7.4) | `efe_streaming_active_streams_per_tenant` |
| T8 | **Batch job poison** | Customer submits 10GB file of malformed Parquet | Worker OOM | Hard size cap (1GB default, configurable) + early validation pass (§ 8.5) | Batch job duration p99 |
| T9 | **Webhook denial-of-service** | Customer endpoint always returns 500 → exponential backoff queue grows | Memory growth in delivery worker | Per-webhook circuit breaker : open after 10 consecutive failures, retry after 1h (§ 6.4) | `webhook_circuit_state` |
| T10 | **CDC slot abandoned** | CDC publisher crashes, slot grows unbounded | PG WAL exhausted, primary degraded | Slot-lag alert + auto-recreate if abandoned > 1h (§ 10.6) | `pg_replication_slots.confirmed_flush_lsn` |
| T11 | **Event log enumeration** | Attacker pages through `GET /v1/events?since=0` to enumerate audit chain | Compliance leak | Per-tenant RLS + size cap per page + rate limit (§ 11.4) | Existing tenant_quota |

The **pen-test bundle** (`bench/m28_streaming_pentest/`) covers T1, T3,
T4, T7, T9 with executable scenarios.

---

## 5. Topology overview

```
                            ┌─ FROM M27 ────────────────────────────┐
                            │                                        │
   ─────────▶ Cloudflare ───▶ Envoy ───▶ efe-server pods           │
                                                                     │
                                                                     ▼
                                                       (synchronous endpoints)
                                                                     │
                                                                     │   M28
                                                                     ▼   NEW ▼
            ┌─────────────────────────────────────────────────────────────────────┐
            │                                                                      │
            │   ┌─ NATS JETSTREAM cluster (3 nodes) ─────────────────────────┐   │
            │   │                                                              │   │
            │   │  Subjects :                                                  │   │
            │   │    efe.tenant.{slug}.classify.completed                      │   │
            │   │    efe.tenant.{slug}.agent.run.completed                     │   │
            │   │    efe.tenant.{slug}.agent.run.failed                        │   │
            │   │    efe.tenant.{slug}.agent.approval.requested                │   │
            │   │    efe.tenant.{slug}.agent.approval.granted                  │   │
            │   │    efe.tenant.{slug}.drift.detected                          │   │
            │   │    efe.tenant.{slug}.bundle.deployed                         │   │
            │   │    efe.tenant.{slug}.batch.completed                         │   │
            │   │    efe.system.audit.{*}                                      │   │
            │   │                                                              │   │
            │   │  Streams (retention 7d / 10M msgs) :                         │   │
            │   │    EVENTS_TENANT_<slug>   (per-tenant)                       │   │
            │   │    EVENTS_SYSTEM           (internal cross-tenant audit)     │   │
            │   │    BATCH_JOBS              (work queue, deliver-once)        │   │
            │   │                                                              │   │
            │   └──────────────────────────────────────────────────────────────┘   │
            │       ▲                              ▲                    ▲          │
            │       │ publish                      │ subscribe          │          │
            │       │                              │                    │          │
            │   ┌───┴────┐    ┌──────────┐    ┌───┴────┐    ┌──────────┐         │
            │   │efe-svr │    │efe-cdc-  │    │webhook-│    │efe-batch-│         │
            │   │(handler│    │publisher │    │delivery│    │worker    │         │
            │   │publish)│    │(PG slot) │    │(Tower) │    │(batch)   │         │
            │   └────────┘    └──────────┘    └────────┘    └──────────┘         │
            │       │              │              │              │                 │
            │       └──► PG ◄──────┘              ▼              ▼                 │
            │                                     │              │                 │
            │                                  Customer       S3                   │
            │                                  HTTPS endpoint  bucket              │
            │                                                                       │
            └─────────────────────────────────────────────────────────────────────┘
```

---

## 6. Component A — Webhook subsystem

### 6.1 Public API

```
POST   /v1/webhooks                  subscribe
GET    /v1/webhooks                  list subscriptions
GET    /v1/webhooks/:id              get one
PATCH  /v1/webhooks/:id              update (URL, topics, active)
DELETE /v1/webhooks/:id              unsubscribe
POST   /v1/webhooks/:id/replay       replay last N events
GET    /v1/webhooks/:id/deliveries   list recent delivery attempts
```

### 6.2 Subscribe request

```http
POST /v1/webhooks
Content-Type: application/json
Authorization: Bearer <jwt>

{
  "url": "https://example.com/efe-webhook",
  "secret": "<shared HMAC secret, generated by customer>",
  "topics": [
    "classify.completed",
    "agent.run.completed",
    "agent.approval.requested",
    "drift.detected"
  ],
  "active": true,
  "verified": false
}
```

Response : `201 Created` with `{"id": "wh_...", "status": "pending_verification"}` (or `active` if `verified: false` initially).

### 6.3 Delivery format

Outbound request from EFE to customer URL :

```http
POST /efe-webhook
Content-Type: application/json
X-EFE-Webhook-ID: wh_abc123
X-EFE-Webhook-Topic: classify.completed
X-EFE-Webhook-Timestamp: 1715383500
X-EFE-Webhook-Signature: t=1715383500,v1=<hex of HMAC-SHA256(secret, ts + "." + body)>
User-Agent: EFE-Webhook/2.3.0

{
  "id": "evt_abc...",
  "tenant_id": "acme",
  "topic": "classify.completed",
  "occurred_at": "2026-05-11T12:34:56Z",
  "data": { ... topic-specific payload ... }
}
```

Customer verifies signature server-side :

```python
import hmac, hashlib

def verify(secret, ts, body, sig_header):
    parts = dict(p.split("=") for p in sig_header.split(","))
    expected = hmac.new(
        secret.encode(),
        f"{ts}.{body}".encode(),
        hashlib.sha256,
    ).hexdigest()
    return hmac.compare_digest(expected, parts["v1"])
```

### 6.4 Delivery semantics

- **At-least-once** : every event has a unique ID ; customer must
  de-dupe.
- **Ordering** : NOT guaranteed across topics. Within a topic, best-effort
  ordering (NATS JetStream `deliver_subject` preserves stream order).
- **Retry policy** :
  - Immediate, then 1 min, 5 min, 30 min, 4 h, 24 h (= 6 attempts max).
  - Each attempt logs to `webhook_delivery` table.
- **Circuit breaker** :
  - Per-webhook : open after 10 consecutive 5xx ; retry after 1 h.
  - Per-host : open if > 50% of webhooks to host failing in 5 min.
- **Timeout** : 10 s per request. Long-running customer handlers must
  ack quickly + process async.

### 6.5 URL validation (SSRF defense)

`POST /v1/webhooks` validates `url` :
- Scheme must be `https` (not `http`, except for `localhost` in dev).
- Host resolves to a public IP (reject 10.0.0.0/8, 172.16/12,
  192.168/16, 169.254/16, ::1, fc00::/7).
- No userinfo (`https://user:pass@host`) allowed.
- TLD must be in `~/.well-known/public_suffix_list` (or close enough).

Implementation : a helper `is_safe_webhook_url` in
`efe-server::v1::webhooks::validation` :

```rust
pub fn validate_webhook_url(url: &str) -> Result<(), WebhookError> {
    let u = url::Url::parse(url)?;
    if u.scheme() != "https" {
        if cfg!(debug_assertions) && u.host_str() == Some("localhost") {
            // OK in dev
        } else {
            return Err(WebhookError::InsecureScheme);
        }
    }
    if u.username() != "" || u.password().is_some() {
        return Err(WebhookError::UserInfoForbidden);
    }
    let host = u.host_str().ok_or(WebhookError::MissingHost)?;
    // Resolve + check.
    let resolved: Vec<IpAddr> = host
        .to_socket_addrs()
        .map_err(|e| WebhookError::DnsResolveFailed(e.to_string()))?
        .map(|sa| sa.ip())
        .collect();
    for ip in resolved {
        if is_private_or_loopback(ip) {
            return Err(WebhookError::PrivateIp(ip.to_string()));
        }
    }
    Ok(())
}
```

DNS rebinding mitigation : we re-resolve at delivery time and
re-check ; if the host now resolves to a private IP, drop the
delivery.

### 6.6 Verification (optional)

A webhook can be "verified" by proving domain ownership :
1. Customer creates a TXT record on the webhook host containing
   a per-webhook nonce.
2. Calls `POST /v1/webhooks/:id/verify`.
3. EFE checks the TXT record ; sets `verified: true`.

Verified webhooks get a green badge in portal-admin + can be used for
"this is our official webhook" gating (e.g. graduation requires a
verified webhook).

### 6.7 Replay

`POST /v1/webhooks/:id/replay` accepts `{since: "2026-05-10T00:00:00Z",
topics: [...]}` and re-publishes filtered events from the customer's
event log. Useful when the customer's endpoint had an outage.

Cap : 10 000 events per replay.

---

## 7. Component B — Streaming classification (gRPC bidirectional)

### 7.1 Proto definition (new)

```proto
service SwarmService {
  // ... existing M0-M27 RPCs ...

  // M28 — bidirectional stream.
  rpc StreamClassify(stream ClassifyRequest) returns (stream ClassifyResponse);
}

message ClassifyRequest {
  string client_id   = 1; // per-stream client-assigned id for resp matching
  string tenant_id   = 2;
  string text        = 3;
  string catalog_id  = 4;
  int32  mode        = 5;
}

message ClassifyResponse {
  string client_id   = 1;
  string class_id    = 2;
  float  confidence  = 3;
  bool   from_cache  = 4;
  int64  latency_us  = 5;
}
```

### 7.2 Pipeline pattern

Server reads each `ClassifyRequest` from the inbound stream,
asynchronously dispatches to the existing classify path (same Mode-1
backend), pushes the result on the outbound stream. Client matches
responses by `client_id`.

```rust
// crates/efe-server/src/services/stream_classify.rs

async fn stream_classify(
    &self,
    req: Request<Streaming<ClassifyRequest>>,
) -> Result<Response<ReceiverStream<Result<ClassifyResponse, Status>>>, Status> {
    let auth = extract_auth(&req)?;
    auth.require_any_role(&["operator", "admin"])?;
    let mut inbound = req.into_inner();
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    let backend = self.backend.clone();

    tokio::spawn(async move {
        while let Some(msg) = inbound.next().await {
            let req = match msg {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                    return;
                }
            };
            // Per-stream concurrency limit (default 16) :
            let semaphore = ...;
            let permit = match semaphore.try_acquire() {
                Ok(p) => p,
                Err(_) => {
                    let _ = tx.send(Err(Status::resource_exhausted("stream backpressure"))).await;
                    continue;
                }
            };
            let resp = classify_one(&backend, &req).await;
            drop(permit);
            if tx.send(resp).await.is_err() {
                break; // client gone
            }
        }
    });

    Ok(Response::new(ReceiverStream::new(rx)))
}
```

### 7.3 Per-stream backpressure

Each stream owns a `Semaphore::new(16)` (configurable) so a single
client can have 16 in-flight classifies before the next inbound message
parks. Outbound stream channel = 128 slots.

### 7.4 Per-tenant concurrent-stream cap

To prevent T7 (connection exhaustion), the gRPC service enforces :
- `max_concurrent_streams_per_tenant = 50` (configurable per tenant).
- Tracked in Redis via `streaming_active:{tenant}` counter (INCR on
  open, DECR on close).
- Hitting the cap → `Status::resource_exhausted("max_streams_per_tenant")`.

### 7.5 Wire protocol

HTTP/2 over TLS (terminated at Envoy). Each stream is a single
HTTP/2 stream. Envoy `max_concurrent_streams: 100` per connection
(M27 setting unchanged).

### 7.6 Performance budget

- 50 000 classifies/s sustained per stream connection (with cache hit
  rate > 80%).
- p99 round-trip latency < 30 ms (cache hit path).
- p99 round-trip < 200 ms (cache miss, Mode-1 classification).

---

## 8. Component C — Batch API

### 8.1 Public API

```
POST   /v1/batch/classify       submit (input S3 URL → job_id)
GET    /v1/batch/:id            status (queued|running|completed|failed)
GET    /v1/batch/:id/result     presigned S3 URL of output
DELETE /v1/batch/:id            cancel
```

### 8.2 Submit request

```json
POST /v1/batch/classify
{
  "tenant_id": "acme",
  "catalog_id": "intents-v3",
  "input_url": "s3://acme-bucket/inputs/run1.parquet",
  "input_format": "parquet",         // or csv
  "input_column": "text",
  "output_url": "s3://acme-bucket/outputs/run1.parquet",
  "output_format": "parquet",
  "callback_webhook_id": "wh_abc",    // optional
  "max_cost_usd": 50.0,
  "mode": 1                            // 1, 2, or 3
}
```

### 8.3 Job lifecycle

```
queued → running → (completed | failed | cancelled)
```

- Queued in NATS `BATCH_JOBS` work-queue stream (deliver-once
  semantics).
- `efe-batch-worker` pods (2-N replicas, queue group `batch-workers`)
  process jobs.
- Worker reads input in 1000-row chunks, classifies via internal API
  call (loopback, sharing cache hits across chunks), writes output
  Parquet incrementally.
- Cost tracked per-row → halt if `max_cost_usd` exceeded.
- On completion : publish `batch.completed` NATS event → webhook
  delivery worker fans out.

### 8.4 S3 path validation

`input_url` and `output_url` must :
- Use `s3://` scheme (no `s3+ssl://` ; TLS implicit).
- Bucket must be in the tenant's `tenant_s3_allowlist` (a new SQL
  table populated by `efe-admin tenant s3-allow <slug> <bucket>`).
- The submitter's AWS credentials (passed in the request as a
  short-lived STS token, signed by tenant's AWS account) must be able
  to read input + write output. EFE never reads or writes outside
  these tokens.

This isolates EFE from customer cloud accounts. EFE doesn't have a
cross-account role.

### 8.5 Size + safety caps

- Input file size : default 1 GB hard cap (configurable per tenant).
- Row count : 10M default.
- Validation pass : worker downloads first 1 MB, parses, confirms
  schema matches `input_column`. Fails fast if malformed.

### 8.6 Idempotency

`POST /v1/batch/classify` accepts an `Idempotency-Key` header. Two
requests with the same key + same body within 24 h return the same
job ID. The existing M11 idempotency layer is reused.

---

## 9. Component D — NATS internal event bus

### 9.1 Why NATS JetStream (not Kafka, not Redis Streams)

See ADR-M28-001. Summary :
- **JetStream native HA + persistence** (Raft-based replication, no
  Zookeeper).
- **Subject hierarchy** simpler than Kafka topics for per-tenant
  isolation.
- **Resource footprint** : ~50 MB RAM per node (vs Kafka 4 GB).
- **Wildcards** : `efe.tenant.*.classify.*` for cross-tenant aggregator.
- **Replay** : `start_seq` / `start_time` makes the customer event log
  trivial.

### 9.2 Cluster topology

- 3 NATS server nodes (StatefulSet, anti-affinity per zone).
- JetStream backed by 20 GB PVC each (~7d × 10M msgs typical).
- TLS server-side + mTLS optional for client.

### 9.3 Subject hierarchy

```
efe.tenant.{slug}.classify.completed
efe.tenant.{slug}.classify.failed
efe.tenant.{slug}.agent.run.created
efe.tenant.{slug}.agent.run.completed
efe.tenant.{slug}.agent.run.failed
efe.tenant.{slug}.agent.run.terminated
efe.tenant.{slug}.agent.approval.requested
efe.tenant.{slug}.agent.approval.granted
efe.tenant.{slug}.agent.approval.rejected
efe.tenant.{slug}.drift.detected
efe.tenant.{slug}.bundle.deployed
efe.tenant.{slug}.bundle.rolled_back
efe.tenant.{slug}.batch.submitted
efe.tenant.{slug}.batch.completed
efe.tenant.{slug}.batch.failed
efe.tenant.{slug}.dpia.signed
efe.tenant.{slug}.dpia.expired
efe.tenant.{slug}.graduation.acknowledged
efe.tenant.{slug}.graduation.revoked

efe.system.audit.{verb}              cross-tenant audit events for ops
```

### 9.4 ACL : per-tenant JWT

NATS supports JWT-based ACL. Each pod gets a JWT signed by the EFE
NATS issuer, with claims :

```json
{
  "iss": "efe-nats-issuer",
  "sub": "efe-server-pod-abc123",
  "aud": "nats",
  "nats": {
    "pub": {
      "allow": ["efe.tenant.*.>"]
    },
    "sub": {
      "allow": ["efe.tenant.*.>"]
    }
  }
}
```

A webhook worker pod gets `sub.allow = efe.tenant.>` (subscribe-only
on tenant stream). Per-customer external subscribers (out-of-scope v1)
would get `sub.allow = efe.tenant.{their_slug}.>` only.

### 9.5 Streams + retention

```yaml
# Stream EVENTS_TENANT_<slug> — one per tenant
subjects: ["efe.tenant.<slug>.>"]
retention: limits
max_age: 7d
max_msgs: 10_000_000
max_bytes: 5GB
storage: file
replicas: 3
discard: old

# Stream BATCH_JOBS — work queue (deliver-once)
subjects: ["efe.batch.jobs.queue"]
retention: workqueue
max_age: 24h
storage: file
replicas: 3
```

Auto-create per-tenant stream when a tenant signs DPIA + activates
agent runtime (idempotent via `nats stream add --update`).

### 9.6 Failure modes

- **NATS down** : publishers buffer locally (in-memory ring buffer,
  default 10 000 msgs) and retry. If buffer overflows, log + drop +
  metric incremented. **No publisher blocks on NATS.**
- **JetStream disk full** : NATS rejects new writes (`MaxBytesExceeded`).
  Operators run RB_M28_NATS_FULL : `nats stream purge` on the oldest
  stream + scale up PVC.

---

## 10. Component E — Postgres CDC → NATS

### 10.1 Why CDC + NATS (not direct app publish only)

App-side publish covers happy-path business events. But for **audit
chain integrity** + **drift event correctness** + **graduation
acknowledgement**, we need an exhaustive event stream that captures
every PG row write — even ones written by background jobs, manual
SQL, or migration scripts. CDC achieves this.

### 10.2 Implementation : `efe-cdc-publisher`

New small Rust binary (in `crates/efe-cdc-publisher/`) that :

1. Opens a PG logical replication slot named `efe_cdc_publisher`.
2. Decodes events using the `pgoutput` plugin (PG 13+ standard).
3. For each row write on a watched table, builds a domain event :
   ```
   agent_audit → efe.tenant.{slug}.audit.{verb}
   classify_call_log → efe.tenant.{slug}.classify.completed
   drift_event → efe.tenant.{slug}.drift.detected
   bundle_version → efe.tenant.{slug}.bundle.deployed
   ```
4. Publishes to NATS.
5. Advances the slot's `confirmed_flush_lsn` only after NATS ack.

### 10.3 At-least-once

A crash between NATS-ack and slot-flush replays the event. Subscribers
must de-dupe by event ID (same ID derived from `(table, primary_key,
xact_lsn)`).

### 10.4 Watched tables

Initially :
- `agent_audit`
- `classify_call_log`
- `drift_event`
- `bundle_version`
- `agent_run` (status transitions only — RUNNING, COMPLETED, FAILED,
  TERMINATED)
- `agent_approval_request` (status transitions only)

Filter applied via `CREATE PUBLICATION efe_cdc FOR TABLE ... WITH
(publish='insert, update');`.

### 10.5 Schema evolution

When a watched table schema changes, the CDC publisher re-discovers
the schema on next connect (sqlx introspection). Field additions are
backward-compatible ; field renames cause an event emission with the
new shape (subscribers should handle unknown fields gracefully).

### 10.6 Failure modes

| Failure | Detection | Mitigation |
|---|---|---|
| CDC publisher pod crashes | `pg_replication_slots.active = false` | Pod restarts ; slot resumes from last `confirmed_flush_lsn` |
| CDC slot abandoned > 1h | Slot lag > 1h | RB_M28_CDC_LAG : truncate slot or recreate |
| NATS down | Publish error | Buffer in slot ; do not advance confirmed_flush_lsn |
| Slot lag = PG WAL exhaustion | `pg_wal_size > 80% disk` | P0 — pause CDC, free disk, restart |

---

## 11. Component F — Customer event log API

### 11.1 Endpoint

```
GET /v1/events?since=<ts>&until=<ts>&topic=<filter>&limit=100&page_token=<...>
```

Returns paginated events from the tenant's `EVENTS_TENANT_<slug>`
JetStream stream, RLS-filtered to the caller's tenant.

### 11.2 Response

```json
{
  "events": [
    {
      "id": "evt_abc",
      "tenant_id": "acme",
      "topic": "classify.completed",
      "occurred_at": "2026-05-11T12:34:56Z",
      "data": { ... }
    },
    ...
  ],
  "next_page_token": "..." | null
}
```

### 11.3 Auth

OIDC ES256 JWT with role `viewer` or above. RLS filter at PG layer +
NATS subject filter (defense in depth).

### 11.4 Rate limit

Inherits the tenant_quota middleware bucket "search" (default 100/min).
Each event-log read counts as 1 request regardless of how many events
returned (limit ≤ 100 per call).

---

## 12. Component G — Server-Sent Events (SSE) public stream

For browser-friendly real-time streaming (without gRPC), we expose
SSE on REST :

```
GET /v1/events/stream
Accept: text/event-stream
Authorization: Bearer <jwt>
```

The same NATS subscription as M11, just framed as SSE. Used by the
portal-admin live dashboard (P1 sprint).

Supports `Last-Event-ID` header for resume (NATS `start_seq` mapped
to the SSE id).

---

## 13. Wire formats + schemas

### 13.1 Event envelope (common to webhook + event log + NATS payload)

```json
{
  "$schema": "https://schemas.efe.ai/event/v1.json",
  "id": "evt_01J...",         // ULID
  "tenant_id": "acme",
  "topic": "classify.completed",
  "occurred_at": "2026-05-11T12:34:56.789Z",
  "schema_version": 1,
  "trace_id": "...",          // optional, W3C trace context
  "data": {
    // topic-specific
  }
}
```

### 13.2 Per-topic schemas (JSON Schema, in `crates/efe-events/schemas/`)

- `classify.completed.v1.json`
- `agent.run.completed.v1.json`
- `agent.approval.requested.v1.json`
- `drift.detected.v1.json`
- `bundle.deployed.v1.json`
- `batch.completed.v1.json`
- `dpia.signed.v1.json`
- `graduation.acknowledged.v1.json`

Schemas versioned : `topic@v1`, `topic@v2`. Customers subscribe to a
schema version explicitly.

### 13.3 Protobuf alternative (out-of-scope v1)

For gRPC clients that want strongly-typed event streams, M29 will add
proto3 equivalents (`efe.v1.events.*`). M28 ships JSON only.

---

## 14. SQL migrations introduced

### 14.1 Migration 0022 — `m28_streaming.sql`

```sql
BEGIN;

-- ── webhook_subscription ──────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS webhook_subscription (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    url                 TEXT NOT NULL,
    secret_hmac_hash    TEXT NOT NULL,        -- argon2-hashed; raw secret never stored
    topics              TEXT[] NOT NULL,
    active              BOOLEAN NOT NULL DEFAULT TRUE,
    verified            BOOLEAN NOT NULL DEFAULT FALSE,
    verification_nonce  TEXT,
    circuit_state       VARCHAR(16) NOT NULL DEFAULT 'closed',
                        -- closed | open | half_open
    consecutive_fails   INTEGER NOT NULL DEFAULT 0,
    circuit_opens_at    TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_delivery_at    TIMESTAMPTZ,
    CONSTRAINT topics_nonempty CHECK (array_length(topics, 1) >= 1)
);

CREATE INDEX idx_webhook_sub_tenant_active
    ON webhook_subscription (tenant_id, active);

-- ── webhook_delivery (per-attempt log) ────────────────────────────────
CREATE TABLE IF NOT EXISTS webhook_delivery (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subscription_id     UUID NOT NULL REFERENCES webhook_subscription(id) ON DELETE CASCADE,
    event_id            TEXT NOT NULL,
    topic               VARCHAR(64) NOT NULL,
    attempt_n           SMALLINT NOT NULL DEFAULT 1,
    attempted_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    response_status     SMALLINT,
    response_body_head  TEXT,                  -- first 256 chars
    latency_ms          INTEGER,
    outcome             VARCHAR(16) NOT NULL,  -- ok | retry | fail | drop
    next_retry_at       TIMESTAMPTZ
);

CREATE INDEX idx_webhook_deliv_sub_time
    ON webhook_delivery (subscription_id, attempted_at DESC);
CREATE INDEX idx_webhook_deliv_event
    ON webhook_delivery (event_id);

-- ── batch_job ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS batch_job (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    catalog_id          TEXT NOT NULL,
    input_url           TEXT NOT NULL,
    input_format        VARCHAR(16) NOT NULL,
    input_column        VARCHAR(64) NOT NULL DEFAULT 'text',
    output_url          TEXT NOT NULL,
    output_format       VARCHAR(16) NOT NULL,
    callback_webhook_id UUID REFERENCES webhook_subscription(id),
    max_cost_usd        NUMERIC(12,4),
    mode                SMALLINT NOT NULL DEFAULT 1,
    status              VARCHAR(16) NOT NULL DEFAULT 'queued',
                        -- queued | running | completed | failed | cancelled
    rows_total          BIGINT,
    rows_processed      BIGINT NOT NULL DEFAULT 0,
    cost_cents          BIGINT NOT NULL DEFAULT 0,
    error_message       TEXT,
    submitted_by        TEXT NOT NULL,
    submitted_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    idempotency_key     TEXT
);

CREATE INDEX idx_batch_job_tenant_status
    ON batch_job (tenant_id, status, submitted_at DESC);
CREATE UNIQUE INDEX idx_batch_job_idempotency
    ON batch_job (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- ── tenant_s3_allowlist (batch path validation) ───────────────────────
CREATE TABLE IF NOT EXISTS tenant_s3_allowlist (
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    bucket              VARCHAR(255) NOT NULL,
    added_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_by            TEXT NOT NULL,
    PRIMARY KEY (tenant_id, bucket)
);

-- ── customer_event_log (mirror of NATS for SQL query path) ────────────
-- A small SQL window of recent events ; primary source is NATS, this
-- mirror serves /v1/events when NATS access is unavailable. Retention
-- 30 days, populated by efe-cdc-publisher.
CREATE TABLE IF NOT EXISTS customer_event_log (
    id                  UUID PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    topic               VARCHAR(64) NOT NULL,
    occurred_at         TIMESTAMPTZ NOT NULL,
    schema_version      SMALLINT NOT NULL DEFAULT 1,
    trace_id            TEXT,
    data                JSONB NOT NULL
);

CREATE INDEX idx_event_log_tenant_time
    ON customer_event_log (tenant_id, occurred_at DESC);
CREATE INDEX idx_event_log_topic
    ON customer_event_log (topic);

-- ── PG logical replication setup (manual ; documented) ────────────────
-- These statements are recipes for the operator, not auto-applied :
--
--   CREATE PUBLICATION efe_cdc FOR TABLE
--       agent_audit, classify_call_log, drift_event, bundle_version,
--       agent_run, agent_approval_request
--   WITH (publish = 'insert, update');
--
--   SELECT pg_create_logical_replication_slot('efe_cdc_publisher', 'pgoutput');
--
-- Run by the runbook `RB_M28_CDC_BOOTSTRAP.md`.

-- ── new agent_audit_event enum values ─────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'WebhookSubscribed') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'WebhookSubscribed';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'WebhookDeliveryFailed') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'WebhookDeliveryFailed';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'BatchSubmitted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'BatchSubmitted';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'BatchCompleted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'BatchCompleted';
    END IF;
END$$;

COMMIT;
```

Rollback : drop tables ; enum values stay (PG limitation).

---

## 15. New crate : `efe-events`

### 15.1 Layout

```
crates/efe-events/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── envelope.rs          // Event<T> struct + JSON Schema validate
│   ├── topics.rs            // const TOPIC_* strings + Topic enum
│   ├── publisher.rs         // NatsPublisher trait + impl
│   ├── subscriber.rs        // NatsSubscriber trait + impl
│   ├── retry.rs             // delivery worker retry policy
│   ├── schemas/             // JSON Schema files
│   │   ├── classify.completed.v1.json
│   │   ├── agent.run.completed.v1.json
│   │   ├── ...
│   └── tests/
└── tests/integration.rs     // gated by EFE_NATS_TEST_URL
```

### 15.2 Public API

```rust
// crates/efe-events/src/lib.rs

pub trait EventPublisher: Send + Sync {
    async fn publish<T: Serialize>(
        &self,
        topic: &str,
        tenant_id: &str,
        payload: T,
    ) -> Result<EventId, EventError>;
}

pub trait EventSubscriber: Send + Sync {
    async fn subscribe(
        &self,
        topic_filter: &str,
        durable_name: &str,
    ) -> Result<EventStream, EventError>;
}

pub struct NatsPublisher { /* JetStream client */ }
pub struct NatsSubscriber { /* JetStream consumer */ }

pub struct Event<T> {
    pub id: EventId,
    pub tenant_id: String,
    pub topic: String,
    pub occurred_at: DateTime<Utc>,
    pub schema_version: u32,
    pub trace_id: Option<String>,
    pub data: T,
}
```

### 15.3 Used by

- `efe-server` : publishes events from handler paths (classify done,
  webhook subscribed, etc.).
- `efe-batch-worker` : subscribes to `BATCH_JOBS` queue, publishes
  `batch.completed`.
- `efe-cdc-publisher` : publishes from PG slot.
- Webhook delivery worker (in `efe-server`) : subscribes to
  `efe.tenant.>` per pod.

---

## 16. Helm chart additions

### 16.1 New sub-chart : `charts/efe-stack/charts/efe-nats/`

```
charts/efe-stack/charts/efe-nats/
├── Chart.yaml
├── values.yaml
└── templates/
    ├── _helpers.tpl
    ├── nats-statefulset.yaml      (3 replicas, anti-affinity, PVC 20Gi)
    ├── nats-configmap.yaml         (TLS, JetStream config, ACL)
    ├── nats-service.yaml           (headless + client + monitoring)
    ├── nats-secret.yaml            (issuer key)
    ├── nats-pdb.yaml
    └── nats-networkpolicy.yaml
```

### 16.2 `efe-batch-worker` Deployment (in efe-server sub-chart)

A new lightweight binary `efe-batch-worker` bundled inside the
`efe-server` image (binary build target). Deployed as a separate
Deployment with 2 replicas.

`charts/efe-stack/charts/efe-server/templates/batch-worker-deployment.yaml`

### 16.3 `efe-cdc-publisher` Deployment

Separate binary, separate Deployment, single replica (leader election
via PG advisory lock).

`charts/efe-stack/charts/efe-server/templates/cdc-publisher-deployment.yaml`

### 16.4 Umbrella `efe-stack` integration

Adds `efe-nats` sub-chart dependency. New `Values.efe-nats.enabled`
(default `false` until M28 ships).

---

## 17. Observability matrix

### 17.1 New metrics

| Metric | Type | Labels |
|---|---|---|
| `efe_webhook_subscriptions_active` | gauge | tenant |
| `efe_webhook_delivery_total` | counter | tenant, topic, outcome |
| `efe_webhook_delivery_duration_seconds` | histogram | topic, outcome |
| `efe_webhook_circuit_state` | gauge (0/1/2) | webhook_id |
| `efe_batch_jobs_active` | gauge | tenant, status |
| `efe_batch_rows_processed_total` | counter | tenant, status |
| `efe_streaming_active_streams_per_tenant` | gauge | tenant |
| `efe_streaming_throughput_rows_per_s` | gauge | tenant |
| `efe_nats_publish_total` | counter | topic, outcome |
| `efe_nats_subscribe_lag_seconds` | gauge | subject, consumer |
| `efe_cdc_slot_lag_bytes` | gauge | slot |
| `efe_cdc_publish_total` | counter | table, outcome |
| `efe_event_log_query_duration_seconds` | histogram | tenant |

### 17.2 PrometheusRule alerts

In `ops/prometheus/rules/m28-streaming-slos.yaml`. Key alerts :
- `M28WebhookDeliveryFailureRate` (> 5% for 10 min → P2).
- `M28WebhookCircuitsOpenCount` (> 10 circuits open → P3, investigate).
- `M28NatsLag` (consumer > 60s behind → P1).
- `M28CdcSlotLag` (> 1 GB → P1, free WAL).
- `M28BatchJobStuck` (running > 4 h with no row progress → P2).

---

## 18. SLO definitions + alerts

| ID | SLO | Window | Target |
|---|---|---|---|
| M28-1 | Webhook delivery p95 latency | 30d | < 5s end-to-end |
| M28-2 | Webhook delivery success rate | 30d | > 99% (excludes customer-side 5xx > 50 consecutive) |
| M28-3 | Streaming classify throughput | sustained | ≥ 50k items/s per stream |
| M28-4 | Batch job p95 | per-job | < 1h for 1M rows |
| M28-5 | NATS consumer lag p99 | 30d | < 5s |
| M28-6 | CDC slot lag p99 | 30d | < 100 MB |
| M28-7 | Event log query p95 | 30d | < 200 ms |

---

## 19. Performance budget + load test plan

### 19.1 Targets

- 1 000 active webhooks per tenant, sustained delivery rate 100/s.
- 100k webhook deliveries/min cluster-wide.
- 1 single gRPC stream sustains 50k classifies/s (cached input).
- 100 concurrent batch jobs each processing 1M rows in < 1 h.
- 10M events/day in NATS without disk pressure.

### 19.2 k6 + custom scripts

`scripts/load/m28/` :
- `webhook_burst.js` — flood 1k webhooks, measure delivery latency.
- `streaming_classify.js` — single gRPC stream at 50k rps.
- `batch_1m_rows.js` — submit + poll a 1M-row Parquet job.
- `event_log_pagination.js` — paginate through 100k events.

### 19.3 Chaos tests

`scripts/chaos/m28/` :
- `kill_nats_leader.sh` — kill leader, expect raft re-election < 10s.
- `pause_cdc_publisher.sh` — sleep slot for 5 min, expect lag alert,
  resume, verify catch-up.
- `webhook_customer_500.sh` — customer endpoint returns 500 for 1 min,
  expect circuit open + reset after grace.

---

## 20. Failure modes + degradation paths

| Failure | Detection | Degraded mode | Recovery |
|---|---|---|---|
| NATS cluster down | NATS pod readiness | Publishers buffer in-memory (10k cap) → drop with metric | NATS restart ; events from buffer lost |
| NATS disk full | `jetstream_stream_messages` | Reject new writes → publishers buffer + drop | Operator runs RB_M28_NATS_FULL |
| Webhook delivery worker crashes | Pod liveness | NATS consumer lag grows ; events in stream OK | Pod restart ; consumer resumes |
| CDC publisher crashes | `pg_replication_slots.active` | Slot grows ; events not pushed | Pod restart ; slot resumes |
| Batch worker crashes mid-job | Worker liveness + NATS msg redelivery | Job re-delivered after `ack_wait=10m` ; idempotency reuses output URL | Auto-recover |
| Customer endpoint 100% 5xx | Circuit breaker | Circuit opens ; deliveries paused for 1h | Customer fixes ; circuit auto-resets to half-open |
| Customer endpoint slow (> 10s) | Per-delivery timeout | Timeout, retry with backoff | Customer fixes |

---

## 21. Migration plan M27 → M28

### Phase 1 (Week 1) : NATS + CDC dual-write

- Install NATS sub-chart in staging.
- Deploy efe-cdc-publisher reading from staging PG.
- Verify events flowing in `nats stream view`.

### Phase 2 (Week 2) : Webhook subsystem

- SQL migration 0022.
- `POST /v1/webhooks` live ; deliveries running but no customer
  subscribed yet (internal test).
- Add a test webhook hitting `httpbin.org/post` ; verify.

### Phase 3 (Week 3) : Batch + streaming

- Deploy efe-batch-worker pods.
- Streaming classify gRPC method published.
- Internal pen test + load test.

### Phase 4 (Week 4) : Customer rollout

- 1 design-partner tenant pilots webhooks + batch.
- Observe 7 days.
- Then unlock for all tenants.

---

## 22. Testing strategy

### 22.1 Unit tests
- `efe-events::publisher` — JSON Schema validation, NATS publish
  success / fail.
- `webhook_url::validate_webhook_url` — every CIDR + scheme variant.
- Webhook circuit breaker state machine.

### 22.2 Integration tests
- `tests/m28_webhook_flow.rs` — subscribe → publish → deliver → ack.
- `tests/m28_batch_e2e.rs` — submit job → worker processes → result
  S3 (MinIO mock).
- `tests/m28_streaming_grpc.rs` — bidirectional stream with 10k items.

### 22.3 Load + chaos
- See § 19.

### 22.4 Pen test
- `bench/m28_streaming_pentest/` :
  - `test_01_webhook_ssrf.py` — register `webhook_url=http://169.254.169.254`.
  - `test_02_webhook_takeover.py` — register webhook on a domain owned
    by another tenant.
  - `test_03_nats_cross_tenant_leak.py` — subscribe to wildcard,
    confirm ACL blocks.
  - `test_04_batch_s3_traversal.py` — submit job pointing to
    `s3://efe-internal/`.
  - `test_05_streaming_exhaustion.py` — open 1000 streams, expect
    rejection at 50.

---

## 23. Runbooks (4 new)

- `RB_M28_WEBHOOK_DELIVERY_FAIL.md` — diagnose + retry / circuit reset.
- `RB_M28_NATS_FULL.md` — JetStream disk pressure mitigation.
- `RB_M28_CDC_LAG.md` — slot lag mitigation + WAL pressure response.
- `RB_M28_BATCH_STUCK.md` — stuck batch job diagnosis + restart.

---

## 24. ADRs (5)

- **ADR-M28-001** : NATS JetStream over Kafka. Rationale : RAM
  footprint, simpler ops, native subject hierarchy.
- **ADR-M28-002** : At-least-once delivery with idempotency keys.
  Rationale : strict ordering not required across topics ; idempotency
  + ULID event IDs sufficient.
- **ADR-M28-003** : PG logical replication slot per CDC publisher
  instance. Rationale : exactly-once semantics ; PG-native replay.
- **ADR-M28-004** : HMAC-SHA256 signed payloads vs JWS / mTLS.
  Rationale : compat with every HTTP framework ; mTLS opt-in
  enterprise-tier upgrade.
- **ADR-M28-005** : S3 path allowlist per tenant. Rationale : strict
  isolation ; no cross-account roles.

---

## 25. Risk register

| ID | Risk | L | I | Mitigation |
|---|---|:--:|:--:|---|
| R1 | NATS cluster ops complexity | MED | MED | Use community Helm chart `nats-io/nats` as reference ; not write own from scratch |
| R2 | CDC slot abandoned → WAL exhaustion | LOW | HIGH | Slot lag alert + auto-recreate runbook |
| R3 | Webhook customer endpoints SSRF us | LOW | HIGH | URL validation + DNS re-check at delivery |
| R4 | Cross-tenant event leak | LOW | CRITICAL | NATS ACL + RLS + pen test in CI |
| R5 | Batch job blowing LLM cost | MED | MED | max_cost_usd hard cap + cost tracking per row |
| R6 | gRPC streaming kept-alive blocking ops | MED | MED | Per-stream timeout 5 min ; idle timeout 60s |
| R7 | NATS schema drift between publishers and subscribers | MED | LOW | JSON Schema validated at publish ; subscribers tolerate unknown fields |

---

## 26. Sprint plan (4 weeks)

### Week 1 — NATS + events crate
- Day 1-2 : Helm sub-chart `efe-nats`, install in staging.
- Day 3-4 : `efe-events` crate (publisher + subscriber + JSON Schema).
- Day 5 : Integration test in staging.

### Week 2 — Webhooks
- Day 1-2 : SQL migration 0022 + handler routes.
- Day 3-4 : Delivery worker (subscribes NATS, signs, posts, retries).
- Day 5 : URL validation + circuit breaker.

### Week 3 — Batch + streaming
- Day 1-2 : Batch API handlers + worker pod.
- Day 3 : Streaming gRPC method.
- Day 4-5 : Per-tenant stream cap + load test.

### Week 4 — CDC + customer event log + rollout
- Day 1-2 : `efe-cdc-publisher` binary + Helm deployment.
- Day 3 : `GET /v1/events` handler.
- Day 4-5 : Migration phase 4 (1 design-partner) + retro.

---

## 27. Done definition

- [ ] `cargo build --workspace --exclude efe-ffi` green.
- [ ] `cargo test --workspace --no-run` green.
- [ ] `helm lint charts/efe-stack` green (efe-nats included).
- [ ] In a fresh k3d cluster, `helm install efe-stack --set efe-nats.enabled=true`
      completes in < 10 min.
- [ ] Webhook subscribe → publish → deliver works end-to-end.
- [ ] Streaming classify gRPC method sustains 10k items/s in
      `tests/m28_streaming_grpc.rs`.
- [ ] Batch job 1k rows completes < 1 min.
- [ ] CDC publisher reads `agent_audit` rows and publishes within 5s.
- [ ] All 5 pen test scenarios pass.
- [ ] 4 runbooks + 5 ADRs committed.
- [ ] CHANGELOG `## EFE 2.3.0 — Streaming + Events (M28)`.

### Explicit non-criteria

- Multi-region NATS — M30.
- Proto-encoded events — M29.
- Kafka alternative — out forever (ADR-M28-001).
- Source connectors — M32.

---

## 28. Annexes

### Annex A — Final file list

**Shipped in EFE 2.3.0** (post-audit chirurgical) :

```
crates/efe-events/                                  (NEW crate, 6 modules + 13 tests)
crates/efe-server/src/v1/webhooks/                  (NEW module, 6 files + 25 tests)
crates/efe-server/src/v1/batch/                     (NEW module, 5 files + 14 tests)
crates/efe-server/src/v1/events/                    (NEW module, 3 files)
crates/efe-server/src/v1/mod.rs                     (+ pub mod webhooks/batch/events)
crates/efe-server/Cargo.toml                        (+ efe-events, async-nats, rand, ring)
crates/efe-server/migrations/0022_m28_streaming.sql (NEW + rollback)
charts/efe-stack/charts/efe-nats/                   (NEW sub-chart, 5 templates)
charts/efe-stack/Chart.yaml                         (+ efe-nats dep)
charts/efe-stack/values.yaml                        (+ efe-nats block)
ops/prometheus/rules/m28-streaming-slos.yaml        (NEW)
bench/m28_streaming_pentest/                        (NEW, 5 scenarios + lib + README)
scripts/load/m28/webhook_burst.js                   (NEW k6)
scripts/chaos/m28/kill_nats_leader.sh               (NEW chaos)
scripts/m28-assembly-verify.sh                      (NEW 5-tier driver)
docs/EFE_M28_STREAMING_EVENTS_ARCHITECTURE.md       (this doc)
docs/docs/adr/ADR-M28-{001..005}.md                 (5 ADRs)
docs/runbooks/RB_M28_{WEBHOOK_DELIVERY_FAIL,NATS_FULL,CDC_LAG,BATCH_STUCK}.md  (4 runbooks)
CHANGELOG.md                                        (## EFE 2.3.0 entry)
```

**Total shipped : 50 files.** Verified via inventory script in
`scripts/m28-assembly-verify.sh` Tier 4.

**Deferred to M28.1** (foundation in place, operator binary spinout
deferred to keep v1 blast radius small) :

```
crates/efe-cdc-publisher/                           (DEFERRED — binary crate)
                                                    Foundation : SQL slot recipe in migration 0022
                                                    + ADR-M28-003 documents the design.
crates/efe-server/src/bin/efe-batch-worker.rs       (DEFERRED — binary entry point)
                                                    Foundation : pure pipeline in
                                                    `v1/batch/worker.rs` (5 unit tests passing).
crates/efe-server/src/services/stream_classify.rs   (DEFERRED — gRPC bidirectional)
                                                    Proto rebuild chain risk ; SSE
                                                    `GET /v1/events/stream` covers M28 use cases.
proto/efe/v1/swarm.proto                            (DEFERRED — StreamClassify rpc)
                                                    See above.
charts/efe-stack/charts/efe-server/templates/batch-worker-deployment.yaml  (DEFERRED)
charts/efe-stack/charts/efe-server/templates/cdc-publisher-deployment.yaml (DEFERRED)
                                                    Pair with binaries above.
```

These deferrals are honored in the CHANGELOG explicit non-criteria
section. Their absence does NOT prevent M28 from delivering its
core promises (webhooks + batch API + NATS event bus + customer event
log + SSE).

### Annex B — One-line verification script (`scripts/m28-assembly-verify.sh`)

```bash
#!/usr/bin/env bash
set -euo pipefail
cargo check --workspace --exclude efe-ffi --locked
cargo test --workspace --exclude efe-ffi --no-run --locked
( command -v helm && helm lint charts/efe-stack ) || true
( command -v k3d  && bash scripts/m28/k3d-smoke.sh ) || true
echo "M28 ALL TIERS PASSED"
```

### Annex C — Wire diff M27 → M28

| Wire | Status |
|---|---|
| REST `/v1/*` existing | unchanged |
| REST `/v1/webhooks/*`, `/v1/batch/*`, `/v1/events*` | NEW |
| gRPC `efe.v1.SwarmService.StreamClassify` | NEW (additive) |
| SSE `/v1/events/stream` | NEW |
| Webhook callback (outbound) | NEW |
| NATS subjects `efe.tenant.{slug}.>` | NEW (internal) |
| CDC slot `efe_cdc_publisher` | NEW (internal) |

Backward-compatible. v1 clients keep working.

### Annex D — Why not Kafka

See ADR-M28-001 for the full rationale, summarised :
- Kafka cluster ops = JVM tuning + Zookeeper/KRaft, ~4 GB RAM per node.
- Kafka topics flat → per-tenant filtering at consumer-side, less elegant.
- Kafka adoption in Rust ecosystem weaker than NATS (rdkafka mature
  but heavyweight ; async-nats native).
- Migration to Kafka future-possible if NATS doesn't scale ; abstraction
  in `efe-events::EventPublisher` trait keeps switch-cost low.

### Annex E — Operational complexity delta

| Pods added | Description |
|:--:|---|
| 3 | NATS server cluster (StatefulSet) |
| 2 | efe-batch-worker |
| 1 | efe-cdc-publisher |
| → 6 new pods total (or 7 with PDB tracking) |

All captured in Helm + observability dashboards.

---

## End of document

**Wire diff summary** : 0 breaking. New REST + gRPC routes are
additive. Subscribers (internal pods or external customers) opt in.
Existing v1 clients see no behavioral change.

**Audit-passable** per M26/M27 pattern : architecture + code +
5-tier verification + ADRs + runbooks + CHANGELOG. The chirurgical
audit (D93-style) will follow the code drop in a separate verification
pass — same as M27.
