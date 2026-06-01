# How-to — cardinality management (D13)

> Sprint **D13** (v0.1.13). Practical guide for keeping the
> Prometheus cardinality of `efe_*` metrics under control as
> tenant count grows.

For the architectural background, see [observability
architecture § Gestion de la cardinalité](../explanation/observability-architecture.md).

---

## TL;DR

* The default budget is **30 000 active series for `efe_*`**.
* The panic threshold is **50 000** — above this, the
  `D13ObsCardinalityBomb` alert fires.
* The two main growth axes are `tenant` (1 → 1000) and
  `name` × `version` (1 → 1000 catalogs).
* The architecture caps **per-tenant** at 100 catalogs × 1000
  versions × 256 classes (D12 quotas).
* When approaching budget, **add recording rules** rather than
  shrinking labels.

---

## 1. Why bother

A naive per-tenant label on every metric blows up fast :

* 12 base metrics × 100 tenants × 4 status codes ≈ 4 800 series.
* + `efe_classify_confidence` × 100 tenants × 10 catalogs × 8
  buckets ≈ 8 000 series.
* + `efe_audit_events_total` × 100 × 3 actions × 7 op_tags
  ≈ 2 100 series.
* + `efe_alert_fires_total` × 16 alerts × 3 severities × 100
  tenants ≈ 4 800 series.
* **Sub-total at 100 tenants ≈ 20 000 series.**

At 1000 tenants the same calculation gives **~ 200 000 series** —
above what a 16 GB / 4-core Prometheus node can sustain
comfortably. Time to think about Mimir / Cortex / Thanos.

---

## 2. Inspect your current cardinality

```promql
# Top 20 metric families by series count
topk(20, count by (__name__) ({__name__=~"efe_.+"}))

# Total series count
count(count by (__name__, job) ({__name__=~"efe_.+"}))
```

Both queries are panels in dashboard `06_capacity_planning`.

---

## 3. Three patterns to keep cardinality bounded

### 3.1 Recording rules with `topk`

Instead of querying all tenants in a dashboard panel :

```promql
# BAD : creates one series per tenant at render
sum(rate(efe_grpc_requests_total[5m])) by (tenant)
```

Use the pre-computed top-20 :

```promql
# GOOD : 20 series, computed every 30 s by Prometheus
efe:top20_tenants_by_rps:5m
```

Or fold the long tail into an `_other_` aggregate :

```promql
# Top 10 by name, rest aggregated
topk(10, sum by (tenant) (rate(efe_grpc_requests_total[5m])))
or on()
label_replace(
  sum without (tenant) (
    rate(efe_grpc_requests_total[5m])
    unless on(tenant) topk(10, sum by (tenant) (rate(efe_grpc_requests_total[5m])))
  ),
  "tenant", "_other_", "", ""
)
```

### 3.2 Hash-bucket the long tail

For metrics where you want anonymous breakdown of the long tail,
add a `tenant_bucket` label via ServiceMonitor relabeling :

```yaml
- source_labels: [tenant]
  target_label: tenant_bucket
  regex: "tenant-(.+)"
  replacement: "${1}"
  action: hashmod
  modulus: 100
```

`tenant_bucket` is a constant 100-cardinality label whatever the
real tenant count. Useful for heatmaps that don't need names.

### 3.3 Drop high-cardinality labels you don't query

If a metric has a `request_id` label (it shouldn't), drop it via
metricRelabeling :

```yaml
- source_labels: [__name__]
  regex: "efe_.+"
  action: keep
- source_labels: [request_id]
  action: labeldrop
```

The cardinality bomb almost always comes from a label
accidentally promoted to a Prometheus label when it should have
stayed in the trace span attributes.

---

## 4. The D11 allowlist defense

D11 already enforces a metric-label allowlist via ServiceMonitor
metricRelabelings. Verify it's active :

```bash
kubectl get servicemonitor efe-acme-server -n tenant-acme \
    -o jsonpath='{.spec.endpoints[0].metricRelabelings}'
```

The expected pattern keeps only the well-known labels
(`tenant`, `method`, `code`, `class`, `op_tag`, …). Any
attacker-injected label (e.g. via a malformed gRPC metadata
header that leaks into a custom label) gets stripped.

Test :

```bash
pytest scripts/tests/test_d11_metric_label_allowlist.py
pytest scripts/tests/test_d13_metric_label_allowlist.py
```

---

## 5. Quotas you control

| Source | Quota | Default | Tunable via |
|--------|-------|---------|-------------|
| Catalogs per tenant | `MAX_NAMES_PER_TENANT` | 100 | `EFE_CATALOG_MAX_NAMES_PER_TENANT` |
| Versions per catalog | `MAX_VERSIONS_PER_NAME` | 1000 | `EFE_CATALOG_MAX_VERSIONS_PER_NAME` |
| Classes per catalog | `MAX_CLASSES_PER_CATALOG` | 256 | `EFE_CATALOG_MAX_CLASSES_PER_CATALOG` |
| Sentences per class | `MAX_SENTENCES_PER_CLASS` | 64 | `EFE_CATALOG_MAX_SENTENCES_PER_CLASS` |
| Active tenants | (cardinality probe) | 1000 | SLO D11-MT-7 |

Lowering any of these reduces the worst-case series count
proportionally.

---

## 6. When to scale to Mimir / Cortex / Thanos

Heuristic :

* **< 100 tenants** : single Prometheus 16 GB node is fine.
* **100-500 tenants** : enable retention via remote-write to
  Mimir / Thanos for long-window queries ; keep local Prom for
  hot data.
* **500+ tenants** : full Mimir / Cortex multi-tenant deployment
  with `X-Scope-OrgID` per partner.

EFE's Helm chart supports the latter via
`observability.multiTenancy.enabled = true`. See
[observability-setup § 5](./observability-setup.md#5-multi-tenant-grafana-setup-tier-2-partner).

---

## 7. Alert that fires when something is off

The chart ships `D13ObsCardinalityBomb` :

```yaml
expr: count(count by (__name__, job) ({__name__=~"efe_.+"})) > 50000
for: 15m
```

When this fires, run the runbook
`docs/runbooks/d13_obs_cardinality_bomb.md` :

1. Identify the offending family via dashboard 06's
   "Top 20 high-cardinality metric families" panel.
2. Check whether the family acquired a new label recently
   (compare against the previous Helm release).
3. Add a `labeldrop` rule to ServiceMonitor relabel.
4. Roll back the offending change.

---

## 8. Pitfalls to avoid

| Anti-pattern | Why it bites | Fix |
|---|---|---|
| `request_id` as a label | Unbounded — explodes immediately | Keep in spans, never in metrics |
| `error_message` as a label | High cardinality on free-text | Use error code label only |
| `class_name` AND `tenant` AND `catalog_name` together | Multiplicative explosion | Pick at most 2 ; aggregate the third |
| Per-trace label propagation to metrics | Defeats Prometheus model | Use exemplars instead |
| `customer_email` or any PII as a label | Regulatory + cardinality double-whammy | Hash via `tenant_bucket` |
