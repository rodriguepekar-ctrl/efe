# EFE — Multi-tenant observability

> **Sprint D11.3 deliverable.** Audience: SRE running EFE for ≥ 2
> tenants. Walks through metrics, alerts, dashboards, log
> attribution.

---

## What's labelled `tenant` ?

After D11, every business metric carries a `tenant` label :

| Metric | Type | Notes |
|--------|------|-------|
| `efe_grpc_requests_total` | counter | also `method, code` |
| `efe_grpc_request_duration_seconds_bucket` | histogram | also `method` |
| `efe_classify_compute_seconds_total` | counter | new in D11 — billing source |
| `efe_egress_bytes_total` | counter | new in D11 — billing source |
| `efe_ratelimit_drops_total` | counter | per-tenant bucket attribution |
| `efe_admin_register_cone_total` | counter | per-tenant cone storm detection |
| `efe_swarm_confidence_bucket` | histogram | quality drift per tenant |
| `efe_policy_rejections_total` | counter | per-tenant alignment breach |

The pre-auth `efe_grpc_in_flight` gauge is intentionally
NOT tenant-labelled — it counts requests before the auth
interceptor binds the tenant.

---

## Importing the Grafana dashboard

```bash
# Direct import via Grafana CLI
GRAFANA_TOKEN=... \
  curl -X POST -H "Authorization: Bearer $GRAFANA_TOKEN" \
       -H "Content-Type: application/json" \
       -d @deploy/observability/grafana/per-tenant-dashboard.json \
       https://grafana.your-corp.com/api/dashboards/db
```

Or via the Grafana UI : Dashboards → Import → upload the JSON.

The dashboard ships :

* Request rate / error rate / p99 latency / compute / egress —
  all per tenant.
* Rate-limit drops per tenant.
* Tenant count cardinality stat (turns red at > 1000 — the
  D11-MT-7 SLO threshold).

The `tenant` template variable lets you focus on a single tenant
or compare a subset.

---

## Prometheus alerting

The Helm chart's `prometheusrule.yaml` ships 4 per-tenant alerts
when `tenant.namespace.isolated=true` :

| Alert | Severity | Condition |
|-------|----------|-----------|
| `EfeTenantHighErrorRate` | warning | per-tenant error % > 5 % for 5 min |
| `EfeTenantRateLimitSaturated` | info | rate-limit drops > 1/s for 10 min |
| `EfeTenantConeRegisterStorm` | warning | > 100 cone registrations / hr |
| `EfeTenantCardinalityExplosion` | critical | > 1000 distinct tenant labels for 10 min |

Tune the thresholds via Helm values
(`prometheusRule.thresholds.tenantErrorRate`, etc.).

---

## Log attribution

EFE emits structured JSON logs. Every business log line carries
`tenant_id` :

```json
{
  "ts": "2026-04-27T14:20:01Z",
  "level": "INFO",
  "msg": "classified",
  "tenant_id": "acme",
  "label": "fraud",
  "confidence": 0.92,
  "rounds": 7,
  "request_id": "abc-123"
}
```

### Loki / Cloud Logging label patterns

For Loki :

```
{namespace="efe", app="efe-server"} | json | tenant_id="acme"
```

For Cloud Logging :

```
resource.type="k8s_container"
resource.labels.namespace_name=~"efe.*"
jsonPayload.tenant_id="acme"
```

For Datadog :

```
service:efe-server @tenant_id:acme
```

### Per-tenant retention policies

Some tenants may have shorter retention requirements (GDPR,
contracts). Use Loki's `retention_period_per_tenant` or the
equivalent in your aggregator. Document in the partner contract,
not in this code.

---

## Tracing

OpenTelemetry traces propagate the `tenant_id` as a span
attribute. Every server-side span carries `efe.tenant_id =
<tenant>`.

In Jaeger / Tempo, filter by tag :

```
service.name="efe-server" efe.tenant_id="acme"
```

---

## Cost attribution → billing

The two new D11 metrics (`efe_classify_compute_seconds_total` and
`efe_egress_bytes_total`) feed the billing aggregator. See
[`operations/billing`](billing.md) for the workflow.

---

## Cardinality safeguards

EFE post-D11 adds ~70 series per tenant on top of the ~3 000
baseline. For 1 000 tenants : 73 000 series — comfortably below
the Prometheus 1-instance default budget of 1 M.

If you anticipate > 1 000 tenants on a single Prometheus :

1. Set `tenant.allowlist` on the Helm chart — drops metrics for
   unknown tenants at scrape time.
2. Federate Prometheus by tenant group (one Prometheus per N
   tenants).
3. Move to a long-term store with native multi-tenancy (Mimir,
   Cortex, Thanos with the multitenancy header).

The `EfeTenantCardinalityExplosion` alert fires at > 1 000 ;
when triaged, choose one of the options above.

---

## Troubleshooting

### Dashboard shows no data

* Confirm the chart is on 0.1.11+ (earlier versions don't have
  `efe_classify_compute_seconds_total`).
* Confirm the ServiceMonitor's `tenant` relabeling rule is
  active (Tier 2 only — Tier 1 stamps `tenant` from JWT in the
  server-side recorder).
* Spot-check a raw query :

```promql
sum by (tenant) (rate(efe_grpc_requests_total[5m]))
```

If this returns `_unknown` only, the tenant label isn't
propagating from the auth layer to the metrics recorder. Open a
support issue with logs.

### A specific tenant shows 0 data

* The tenant has 0 traffic in the window.
* The tenant's JWT issuer mints a slightly different `tenant_id`
  (typo, prefix). Reconcile with the IDP.
* A `metricRelabeling` allowlist drops the tenant. Check
  `tenant.allowlist`.

### Cardinality alert flapping

Some legit tenants are churning (creation/deletion of test
tenants). Either :

* Use a stable tenant_id range (don't recycle).
* Tune `prometheusRule.thresholds.tenantCardinalityCap` higher.
* Move to per-tenant federation.

---

## See also

* [Operations / Billing](billing.md)
* [How-to / Multi-tenant deployment](../docs/howto/multi-tenant.md)
* [Explanation / Security model](../docs/explanation/security-model.md)
* `deploy/observability/grafana/per-tenant-dashboard.json` — the dashboard JSON.
