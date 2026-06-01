# Security model

> Sprint D11 — partner-facing threat model. Audience: a security
> engineer doing a vendor review.

EFE's security model has three layers, addressed top to bottom :

  1. **Identity** — who is calling.
  2. **Isolation** — what they can see / mutate.
  3. **Integrity** — guarantees on the audit trail.

---

## 1. Identity

### JWT-based auth

Every authenticated RPC carries a Bearer JWT in the
`authorization` gRPC metadata. The server validates :

* HMAC-HS256 signature against one of the IDP's currently-active
  signing keys (multi-key validation supports zero-downtime
  rotation, cf. [`operations/key_rotation`](../../operations/key_rotation.md)).
* `aud` claim matches `efe.v1`.
* `exp` claim is in the future (with 60 s clock skew tolerance).
* Required custom claims present : `tenant_id`, `scopes`.

### Scopes

The `scopes` claim is a list of strings controlling which RPCs
the principal can invoke :

| Scope | Grants |
|-------|--------|
| `classify:read` | `Execute`, `ExecuteStreaming`, `DescribeSpec` |
| `classify:write` | (reserved — same as read currently) |
| `admin:cone:read` | `GetCone`, `ListVersions`, `ListCatalogs` |
| `admin:cone:write` | `RegisterCone`, `EvolveCone`, `DeleteCone`, `BreachHistory` |

### Anonymous fallback (DEV ONLY)

When `config.auth.enabled=false`, the server treats every request
as `tenant_id="anonymous"`. **This is for local development
only** and is enforced disabled in the Tier 2 chart preset.

---

## 2. Isolation — the 12-vector threat model

| # | Vector | Mitigation |
|--:|--------|-----------|
| 1 | JWT absent | UNAUTHENTICATED |
| 2 | JWT signed by attacker | UNAUTHENTICATED (signature mismatch) |
| 3 | JWT.tenant_id ≠ request.tenant | PERMISSION_DENIED via `assert_tenant_match` |
| 4 | Catalog list across tenants | server filters by JWT's tenant_id |
| 5 | Cone register with foreign tenant | PERMISSION_DENIED |
| 6 | Tenant A overflows global rate limit | per-tenant token bucket — B unaffected |
| 7 | Idempotency cache collision | cache keyed on `(tenant_id, request_id)` |
| 8 | Audit log emits before tenant binding | tracing span MUST carry `tenant_id` (test-pinned) |
| 9 | Audit log spans cross tenants | span scoped to single request lifecycle |
| 10 | Anonymous fallback in prod | Tier 2 chart enforces `auth.enabled=true` |
| 11 | Per-tenant LRU cache leak | LRU bounded `max_tenants=10 000` |
| 12 | Metrics cardinality DoS | ServiceMonitor `metricRelabeling` allowlist option |

The full HAZOP analysis is in
[`docs/bench/d11_per_tenant_isolation_architecture.md`](https://github.com/efe-project/efe/blob/main/docs/bench/d11_per_tenant_isolation_architecture.md) §4.

### What "tenant isolation" guarantees

* **No cross-tenant read.** A JWT for tenant A cannot retrieve
  any catalog, cone, or response keyed to tenant B. Server-enforced.
* **No cross-tenant write.** RegisterCone / EvolveCone /
  DeleteCone with a foreign tenant fails fast (PERMISSION_DENIED).
* **No cross-tenant rate-limit leakage.** Each tenant has its own
  token bucket ; one tenant's saturation doesn't penalise another.
* **No cross-tenant audit confusion.** Every audit log line
  carries the `tenant_id` that was authoritative at request time.

### What it does NOT guarantee

* **Side-channel resistance** (timing, CPU). EFE shares CPU and
  RAM in Tier 1 ; for paranoid isolation use Tier 2 (k8s
  ResourceQuota) or Tier 3 (cluster-per-tenant).
* **Network confidentiality** in the absence of TLS. Tier 1's
  `insecure=true` is dev only ; production deployments MUST use
  TLS termination (cert-manager + ingress / service mesh).
* **JWT replay protection across clusters.** Mitigation: short
  TTL (≤ 1 h) on tokens, idempotency cache window of 10 s on
  retries.

---

## 3. Integrity — audit trail

Every business RPC produces an audit log line carrying :

* `tenant_id` — authoritative tenant.
* `sub` — JWT subject (user/service-account id).
* `request_id` — server-side stamped UUID.
* `method` — gRPC method full name.
* `code` — gRPC status code.
* `duration_ms` — wallclock.

PII (`raw_prompt`) is **NEVER** in the audit log by default.
Setting `include_raw_prompt=True` on the SDK is allowed for dev /
debugging and MUST not be enabled in production.

The PII redaction pipeline (D8) hits the audit logger BEFORE
emission. See [Operations / observability](../../operations/multi-tenant-observability.md).

### What goes in audit

* request_id, tenant_id, sub, method, code, duration_ms
* status (OK / failed)
* round_count (for Mode-1 traffic)

### What does NOT go in audit

* raw_prompt (unless explicit opt-in, never in prod)
* response payloads
* embedding vectors

---

## Compliance posture

* **GDPR** — EFE stores no PII at rest by default. Audit logs are
  the only persistence and they're PII-free under the default
  policy. Data residency is the operator's responsibility (choose
  a cluster region appropriate for your tenants).
* **SOC2** — D11 ships the audit trail + multi-tenant boundary.
  Type 1 readiness ; Type 2 needs 6+ months of operation.
* **ISO 27001** — out of scope for D11 ; the MVP ships the
  technical controls, the operator wraps the management system.

---

## See also

* [How-to / Multi-tenant deployment](../howto/multi-tenant.md)
* [Reference / Tenant claims](../reference/tenant-claims.md)
* [Operations / Key rotation](../../operations/key_rotation.md)
