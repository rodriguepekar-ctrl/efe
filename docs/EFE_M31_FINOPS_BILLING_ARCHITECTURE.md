# EFE M31 — FinOps + Cost Governance + Stripe Billing

**Status** : Architecture · `EXPERIMENTAL` · sprint scope ≈ 4 weeks
**Hard dependencies** : M27 (Redis quota counters), M28 (NATS events),
M29 (audit chain), M30 (multi-modal metering targets).
**Soft dependencies** : Stripe Enterprise account, AWS SES (budget
emails), AWS SNS (high-criticality alerts).
**Activation** : OFF by default. Opt-in via
`efe-stripe.enabled=true` + SQL migration 0025 + Stripe API key
populated in the `efe-stripe-secrets` Kubernetes Secret.

---

## 1. Why M31 now ?

Up to M30 the EFE platform is **technically commercial-ready** but
not **commercially commercial-ready**. Two gaps :

1. **No per-tenant cost attribution.** Every classify, agent run,
   webhook delivery, multimodal inference is paid for by EFE
   (compute + LLM tokens + GPU minutes), but there is no per-tenant
   bookkeeping. We literally cannot tell whether tenant *X* is
   profitable.
2. **No invoicing pipeline.** Closing a $200k/year enterprise deal
   today requires hand-rolled spreadsheets + ad-hoc Stripe invoice
   creation — error-prone, slow, and impossible to audit.

M31 closes both gaps by introducing :

- A **real-time metering plane** that records every billable event
  to a Redis-backed counter + PG mirror for durability.
- A **billing plane** that aggregates metering data into hourly +
  daily rollups, projects monthly cost, and emits Stripe invoices
  at month-end.
- A **budget alert plane** that lets customers set hard / soft caps
  and receive email / webhook notifications before they overrun.

The whole plane is **opt-in per tenant** so existing pilot deals
that pre-date M31 keep their flat-rate pricing — no surprise bills.

---

## 2. Scope summary

### In scope (M31)

| Capability | Description |
|---|---|
| Metering | Real-time `Counter` API for every operation (classify, agent run, multimodal infer, webhook delivery, batch job, audit export). |
| Rollup | Hourly + daily aggregation into PG (`usage_rollup_hour`, `usage_rollup_day`). |
| Stripe sync | One Stripe `Customer` per tenant, one `Subscription` per pricing plan, monthly `Invoice` with usage-based line items. |
| Pricing plans | Free / Starter / Pro / Enterprise with per-resource unit prices. |
| Budgets | Per-tenant soft + hard caps with `email` + `webhook` notification channels. |
| Cost projection | "If your current daily run rate continues, you will spend $X by month-end." |
| Self-service portal | `GET /v1/billing/portal` returns a Stripe Customer Portal session URL. |
| Invoice download | `GET /v1/billing/invoices/:id` returns the Stripe-hosted PDF. |
| Payment method | `PUT /v1/billing/payment_method` updates the default card / SEPA mandate. |

### Out of scope (deferred)

| Capability | Target |
|---|---|
| Multi-currency invoicing (USD/EUR/GBP) | M31.1 |
| Quote → contract → revenue recognition (Stripe Quotes API) | M32 |
| Pro-rated mid-cycle plan changes | M31.1 |
| Tax handling beyond Stripe Tax | M32 |
| Marketplace billing (partner share) | M33 |
| Prepaid credit balances | M32 |

---

## 3. Domain model

### 3.1 Vocabulary

- **Meter** : a typed billable resource. M31 ships 8 meters :
  `classify_requests`, `agent_run_steps`, `agent_run_seconds`,
  `multimodal_vision_requests`, `multimodal_audio_seconds`,
  `webhook_deliveries`, `batch_classified_rows`, `audit_export_gb`.
- **Counter** : an in-process Redis-backed integer that increments
  on every billable event. One counter per (tenant, meter, hour).
- **Rollup** : the persisted snapshot of a counter at hour end +
  day end. Source of truth for billing.
- **Pricing plan** : a row in `pricing_plan` carrying a unit price
  per meter. Tenants are associated to one plan via
  `tenant_billing.plan_id`.
- **Subscription** : a Stripe `Subscription` object mirrored into
  `billing_subscription` for offline access.
- **Invoice** : a Stripe `Invoice` object mirrored into
  `billing_invoice`. Line items denormalized into
  `billing_invoice_item` for analytics.
- **Budget** : a row in `budget_alert` describing a soft / hard
  cap + notification channels.

### 3.2 ER summary

```
tenant_feature                              ← M22+
  └── tenant_billing (1:1)                  ← M31 (plan + customer_id)
        ├── billing_subscription (0..N)
        │     └── billing_invoice (0..N)
        │           └── billing_invoice_item (0..N)
        ├── budget_alert (0..N)
        ├── payment_method (0..N)
        └── pricing_plan (N:1)              ← shared catalog

meter_event (append-only log, 30d retention)
  └── usage_rollup_hour
        └── usage_rollup_day
              └── billing_invoice_item.usage_summary (JSON)
```

### 3.3 Pricing plan example

| Plan | Monthly base | classify_req | agent_run_sec | multimodal_vision | …  |
|---|---:|---:|---:|---:|---|
| `free` | $0 | 0–10 k free, then refuse | 0 | 0 | …|
| `starter` | $49 | $0.001 / req over 50 k | $0.01 / sec | $0.05 / req | …|
| `pro` | $499 | $0.0005 / req over 500 k | $0.005 / sec | $0.025 / req | …|
| `enterprise` | quote | volume-discounted by SE | quote | quote | …|

The actual numbers are in `bench/m31_pricing_calibration.py` ; the
table above is illustrative.

---

## 4. Components

### 4.1 `efe-metering` crate

Tiny Rust crate (~ 1 500 LOC) whose only job is to expose :

```rust
pub async fn meter_inc(tenant: &str, meter: Meter, n: u64) -> Result<()>;
pub async fn meter_inc_with_attrs(tenant: &str, meter: Meter, n: u64, attrs: Attrs) -> Result<()>;
```

Backed by a Redis `HINCRBY` against `m31:counter:{tenant}:{yyyymmddhh}`
with a TTL of 48 h. Buffered in-memory via a small `MeterBuffer` that
flushes every 5 s or every 1024 events (whichever first) to amortize
Redis round-trips. The buffer is **lossy on crash** by design — we
mirror to NATS `meter.bumped` events for durable replay.

The crate re-uses `efe_cache::RedisClient` (M27) so the connection
pool is shared with the quota counters.

### 4.2 `efe-billing` crate

Slightly larger (~ 4 000 LOC) and split into 5 modules :

| Module | Responsibility |
|---|---|
| `stripe_client.rs` | Thin async client over `https://api.stripe.com/v1`. Customer + Subscription + Invoice + Webhook signature verification. |
| `pricing.rs` | In-memory pricing-plan cache + `compute_line_items(rollup, plan)` pure function. |
| `invoice_builder.rs` | Walks `usage_rollup_day` for a billing period, applies the plan, produces Stripe invoice items. |
| `reconciliation.rs` | Cron-driven reconciler that compares the local mirror against Stripe and flags drift. |
| `budget.rs` | Evaluates `budget_alert` rules against the latest rollup ; fires NATS `budget.exceeded` events. |

### 4.3 v1 REST handlers

| Endpoint | Method | Description |
|---|---|---|
| `/v1/usage/now` | GET | Real-time meter values (last 60 min). |
| `/v1/usage/daily` | GET | Daily rollups for the current month. |
| `/v1/usage/projection` | GET | Month-end cost projection based on run rate. |
| `/v1/budgets` | GET / POST | List / create budget alert. |
| `/v1/budgets/:id` | PUT / DELETE | Update / delete budget. |
| `/v1/billing/portal` | POST | Stripe Customer Portal session. |
| `/v1/billing/plan` | GET / PUT | Current plan + plan switch. |
| `/v1/billing/subscriptions` | GET | List subscriptions. |
| `/v1/billing/invoices` | GET | List invoices. |
| `/v1/billing/invoices/:id` | GET | Get invoice + PDF URL. |
| `/v1/billing/payment_method` | PUT | Update default payment method. |
| `/v1/billing/webhook` | POST | Stripe webhook endpoint. |

All endpoints are tenant-scoped + bearer-authed except the webhook
which is HMAC-verified using the Stripe signing secret.

### 4.4 PG schema

7 new tables (cf. SQL migration 0025), 1 new column on
`tenant_feature` (`billing_enabled`), 3 audit enum extensions
(`BillingPlanChanged`, `BudgetExceeded`, `InvoiceFinalized`).

### 4.5 Helm `efe-stripe` sub-chart

- Deployment hosting the **Stripe Webhook Receiver** (a tiny axum
  service that validates the signature, decodes the event,
  publishes to NATS `billing.stripe.event`).
- Network policy : ingress only from Cloudflare IPs.
- Service Monitor for Prometheus.
- Secret reference `efe-stripe-secrets` (API key, signing secret).

### 4.6 Terraform

- `terraform/m31_billing/stripe.tf` — Stripe products, prices, tax
  rates created via the Stripe Terraform provider.
- `terraform/m31_billing/budget_sns.tf` — SNS topic for high-severity
  budget alerts (page on-call).
- `terraform/m31_billing/ses.tf` — SES configured set for budget
  emails (re-used from M28).

---

## 5. Data flow — happy path

```
[REST request /v1/classify]
   ↓
  efe-server::v1::classify  ─── meter_inc("classify_requests", 1) ──┐
   ↓                                                                │
  efe-server response                                               │
                                                                   ▼
                                                       efe-metering::flush()
                                                                   ↓
                                                          Redis HINCRBY +
                                                          NATS meter.bumped
                                                                   ↓
                                                       ┌──── hourly cron ───┐
                                                       │   rollup_hour      │
                                                       │   ↓ PG insert      │
                                                       │   delete Redis key │
                                                       └────────────────────┘
                                                                   ↓
                                                       ┌──── daily cron ────┐
                                                       │   rollup_day       │
                                                       │   ↓ PG insert      │
                                                       │   budget eval      │
                                                       └────────────────────┘
                                                                   ↓
                                                       ┌──── month-end ─────┐
                                                       │  invoice_builder   │
                                                       │  ↓ Stripe API     │
                                                       │  finalize_invoice  │
                                                       └────────────────────┘
                                                                   ↓
                                                       ┌── Stripe webhook ──┐
                                                       │   sig verify       │
                                                       │   NATS publish     │
                                                       │   PG mirror update │
                                                       └────────────────────┘
```

---

## 6. Failure modes

| Failure | Mitigation |
|---|---|
| Redis cluster down | `meter_inc` returns `Err(MeteringError::CacheUnavailable)` ; caller ignores (fail-open). NATS `meter.bumped` event still published so durability is preserved. |
| NATS down | `meter_inc` continues to bump Redis ; replay from PG mirror on recovery. Loss window = last hourly rollup. |
| PG down (rollup) | Rollup cron retries with exponential backoff. Redis counters keep accumulating until TTL (48 h). Beyond 48 h we accept the lossiness — caller is paged. |
| Stripe API down | Invoice creation defers ; cron retries with backoff. Customer-visible : invoice marked `pending_stripe` in PG mirror ; portal shows "Invoice will be available soon". |
| Stripe webhook lost | Reconciliation cron compares local mirror to Stripe `Invoice.list` API every 6 h and patches the mirror. |
| Budget false-positive | Tenant can dismiss + suppress for 24 h via the portal ; suppression mirrored to PG. |
| Negative metering value | `meter_inc(_, _, n)` panics if `n` would cause overflow ; clamped to `MAX_PER_CALL = 100_000` to avoid pathological inputs. |

---

## 7. Security & compliance

- **PCI scope** : we never store full PAN. Stripe handles card data ;
  we only store `payment_method.last4` + `brand`.
- **GDPR** : meter events are per-tenant aggregates ; no PII. Audit
  trail logs operator who changed a plan or dismissed a budget alert.
- **SOX** : invoice line items are immutable once `status =
  finalized` ; mutations create a new revision row.
- **Webhook replay protection** : Stripe signing secret + 5-minute
  timestamp tolerance + idempotency key on the event id.

---

## 8. Performance budget

| Operation | Budget | Mechanism |
|---|---:|---|
| `meter_inc` p99 (fast path, buffered) | < 50 µs | In-process atomic add |
| `meter_inc` p99 (flush, Redis HINCRBY) | < 2 ms | Redis Cluster + connection pool |
| `usage/now` GET p99 | < 50 ms | Redis multi-get + JSON encode |
| `usage/daily` GET p99 | < 100 ms | PG SELECT on indexed (tenant, day) |
| `billing/invoices` GET p99 | < 200 ms | PG mirror, no Stripe round-trip |
| Hourly rollup wall-clock | < 30 s | Streamed Redis SCAN + bulk PG insert |
| Monthly invoice generation (per tenant) | < 5 s | Reads usage_rollup_day + 1 Stripe API call |
| Reconciliation per tenant | < 1 s | Stripe `Invoice.list?customer={id}&limit=10` |

---

## 9. Observability

| Metric | Type | Labels | Purpose |
|---|---|---|---|
| `efe_meter_inc_total` | counter | meter | Per-meter increment count |
| `efe_meter_inc_duration_seconds` | histogram | outcome | Latency of `meter_inc` calls |
| `efe_meter_flush_batch_size` | histogram | — | Buffered batch size at flush |
| `efe_meter_redis_errors_total` | counter | kind | Redis errors (transport, encode, capacity) |
| `efe_rollup_lag_seconds` | gauge | period | Time since last successful rollup |
| `efe_invoice_status_total` | counter | status | Invoice lifecycle counts |
| `efe_stripe_api_duration_seconds` | histogram | endpoint, outcome | Stripe API latency |
| `efe_stripe_webhook_received_total` | counter | event_type, outcome | Webhook intake |
| `efe_budget_exceeded_total` | counter | severity | Budget alerts fired |

Cardinality budget : ≤ 32 series per histogram. Cf. ADR-M31-005.

---

## 10. SLOs

| SLO | Target |
|---|---|
| Metering availability | 99.95 % (`meter_inc` fail-open OK) |
| Rollup completion | 99.9 % within 30 min of period end |
| Invoice generation | 99 % within 24 h of period end |
| Stripe webhook ingest | 99.9 % within 60 s |
| Budget alert delivery | 99 % within 5 min of threshold crossed |

---

## 11. Threat model

| Threat | Mitigation |
|---|---|
| Tenant inflates own meter to test pricing | Meter writes server-side only ; no client mutation API. |
| Compromised tenant key floods meter | Standard M27 ratelimit + audit on `Bearer`. |
| Replay-attack on Stripe webhook | Stripe-Signature header verification + 5 min tolerance. |
| Budget alert spam | Per-tenant debounce : ≥ 30 min between identical alerts. |
| Invoice tampering | `billing_invoice.status` transitions write to HMAC audit chain. |
| Stripe API key leak | Stored in K8s Secret + IAM-restricted KMS ; rotated every 90 days. |
| Cost projection abuse (forecasting attacker queries) | Read endpoints rate-limited at 60 RPS / tenant. |

---

## 12. Rollout

| Phase | Duration | Description |
|---|---|---|
| α (internal) | 1 week | Enable for `efe-internal` tenant. Validate every meter increments. |
| β (pilot) | 2 weeks | Enable for 3 friendly pilot tenants. Compare hand-rolled spreadsheets to invoices. |
| γ (GA EXPERIMENTAL) | 4 weeks | Open to enterprise tenants on opt-in basis. Marketing announcement deferred. |
| GA stable | M31.1 | After 3 successful billing cycles + 0 SEV-1 incidents. |

Rollback : `efe-stripe.enabled=false` reverts to silent-mode (meters
still record but no invoice generation). Pilots are billed manually
during this period.

---

## 13. ADRs (M31-001..006)

- **M31-001** Redis-backed counters over per-event SQL inserts.
- **M31-002** Stripe over alternatives (Lago, OpenMeter, Recurly).
- **M31-003** Hourly rollups (not minute-level).
- **M31-004** Customer-portal-first self-service (no in-app billing UI).
- **M31-005** Prometheus cardinality budget for billing.
- **M31-006** Idempotent retries on Stripe Operation IDs.

---

## 14. Runbooks (RB_M31_*)

- **RB_M31_REDIS_OUTAGE** — Metering degraded, what to do.
- **RB_M31_ROLLUP_BACKLOG** — Hourly cron is behind.
- **RB_M31_STRIPE_API_DEGRADED** — Stripe 5xx storm.
- **RB_M31_INVOICE_RECONCILIATION_FAILED** — drift detected.
- **RB_M31_BUDGET_ALERT_STORM** — false-positive cascade.

---

## 15. Migration plan

1. Deploy `efe-stripe.enabled=false` to all clusters with SQL
   migration 0025 applied. Tables exist but empty.
2. Enable `meter_inc` calls in every operational path behind a
   per-tenant `tenant_feature.metering_enabled` flag.
3. Backfill `pricing_plan` rows from the marketing site.
4. Flip the master switch `efe-stripe.enabled=true`.
5. Switch the internal tenant to plan `free` to validate.
6. Pilot tenants opt in.

---

## 16. Annex A — files shipped in this milestone

```
crates/
  efe-metering/                    [NEW]
    Cargo.toml
    src/
      lib.rs
      counter.rs
      buffer.rs
      meter.rs
      storage.rs
      error.rs
      metrics.rs
  efe-billing/                     [NEW]
    Cargo.toml
    src/
      lib.rs
      stripe_client.rs
      pricing.rs
      invoice_builder.rs
      reconciliation.rs
      budget.rs
      error.rs
      metrics.rs
  efe-server/
    src/v1/
      usage/                       [NEW]
        mod.rs / handlers.rs / errors.rs / state.rs
      billing/                     [NEW]
        mod.rs / handlers.rs / errors.rs / state.rs / webhook.rs
crates/efe-server/migrations/
  0025_m31_billing.sql              [NEW]
  0025_m31_billing_rollback.sql     [NEW]
charts/efe-stack/charts/
  efe-stripe/                       [NEW]
    Chart.yaml / values.yaml / templates/_helpers.tpl / templates/receiver.yaml
terraform/m31_billing/              [NEW]
  stripe.tf / budget_sns.tf / ses.tf / variables.tf / outputs.tf / README.md
deploy/observability/
  prometheus/rules.m31/alerts.yaml  [NEW]
  grafana/dashboards/m31-01-billing-overview.json  [NEW]
scripts/
  chaos/m31/                        [NEW] (Redis kill, Stripe outage sim)
  load/m31/                         [NEW] (10k RPS metering steady state)
  m31/                              [NEW] (rollup_runner.sh, invoice_runner.sh)
  m31-assembly-verify.sh            [NEW]
bench/
  m31_billing_pentest/              [NEW]
docs/
  EFE_M31_FINOPS_BILLING_ARCHITECTURE.md       [this doc]
  EFE_M31_KNOWN_GAPS.md                         [NEW]
docs/docs/adr/
  ADR-M31-001..006-*.md             [6 NEW]
docs/runbooks/
  RB_M31_*.md                       [5 NEW]
CHANGELOG.md                         [UPDATED — 2.6.0 entry]
```

Approximate net new : **65 files**.
