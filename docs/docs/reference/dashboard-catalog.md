# Reference — D13 dashboard catalog

> Sprint **D13** (v0.1.13). Reference document of the 6 Grafana
> dashboards shipped by the Helm chart : their UIDs, panels,
> queries, and drill-down semantics. Suitable for printout / wall
> reference during incidents.

---

## Inventory

| # | UID | Persona | Default time range | Refresh |
|---|-----|---------|--------------------|---------|
| 01 | `efe-d13-01-operator-overview` | SRE on-call | last 3 h | 30 s |
| 02 | `efe-d13-02-tenant-drill-down` | Support T1/T2 | last 24 h | 1 min |
| 03 | `efe-d13-03-catalog-version-distribution` | Catalog curator | last 7 d | 1 min |
| 04 | `efe-d13-04-cost-attribution` | Finance / FinOps | month-to-date | 5 min |
| 05 | `efe-d13-05-audit-stream` | Security analyst | last 24 h | 1 min |
| 06 | `efe-d13-06-capacity-planning` | Capacity SRE | last 30 d | 5 min |

---

## 01 — Operator Overview

**JTBD** : "Le service est-il en santé ? Si non, où est le
problème ?" — réponse en < 10 s.

| Row | Panel | Query (PromQL) | Use |
|-----|-------|----------------|-----|
| 0 | SLO compliance | `1 - (sum(rate(efe_grpc_requests_total{code!="OK"}[5m])) / sum(rate(efe_grpc_requests_total[5m])))` | Glance check |
| 1 | Latency p99 | `histogram_quantile(0.99, sum(rate(efe_grpc_request_duration_seconds_bucket[1m])) by (le))` | Stat |
| 1 | Error rate | `sum(rate(efe_grpc_requests_total{code!="OK"}[5m])) / sum(rate(efe_grpc_requests_total[5m]))` | Stat |
| 1 | Throughput | `sum(rate(efe_grpc_requests_total[1m]))` | Stat |
| 1 | Saturation (CPU) | `max(rate(container_cpu_usage_seconds_total{namespace=~"efe.*\|tenant-.+"}[5m])) / on() max(kube_pod_container_resource_limits{resource="cpu"})` | Stat |
| 2 | Latency heatmap | `sum(rate(efe_grpc_request_duration_seconds_bucket{tenant=~"$tenant"}[5m])) by (le)` | Heatmap with exemplars |
| 2 | Error rate per RPC | `sum by (code) (rate(efe_grpc_requests_total{code!="OK", tenant=~"$tenant"}[5m]))` | Stacked timeseries |
| 3 | Top 5 noisy tenants | `topk(5, efe:rpc_error_rate:5m)` | Table |
| 3 | Active D13 alerts | (Alertmanager) | Alert list |
| 4 | Recent errors (Loki) | `{stream="request", level="ERROR", tenant_id=~"$tenant"}` | Logs panel |

**Drill-down** :
- Click any tenant cell → dashboard 02 with the variable preset.
- Click any heatmap exemplar → trace view in Tempo.

---

## 02 — Tenant Drill-Down

**JTBD** : "Le client X dit que c'est lent / cassé. Que se passe-
t-il pour CE tenant ?" — réponse en < 30 s.

**Variable required** : `$tenant` (single-select, no All).

Key panels :

* Active catalogs : `count(count by (name) (efe_catalog_versions_per_name{tenant="$tenant"}))`.
* RPS by RPC : `sum by (method) (rate(efe_grpc_requests_total{tenant="$tenant"}[5m]))`.
* Error rate by code : stacked.
* Latency p50/p95/p99 by RPC.
* Confidence distribution (heatmap).
* Idempotency hits.
* Logs panel : `{tenant_id="$tenant", stream="$stream"}`.

---

## 03 — Catalog Version Distribution

**JTBD** : "Quelles versions de mes catalogues sont actives ? Y
a-t-il des tenants encore sur des versions vétustes ?"

**Variables** : `$tenant` (multi), `$catalog_name` (multi).

Key panels :

* Version chain depth (table, color-coded).
* Evolve op rate stacked by op_tag.
* Recompiles 7d / Removes 7d (stat with severity).
* Audit log tail (Loki, evolution stream).

---

## 04 — Cost Attribution

**JTBD** : "Combien chaque tenant nous coûte sur le mois en
cours ?"

**Variable** : `$tenant` (multi).

Key panels :

* Top 10 tenants by CPU minutes : `topk(10, sum by (namespace) (sum_over_time(efe:cpu_minutes_per_tenant:1h[$__range])))`.
* Top 10 tenants by RAM GB-hours.
* CPU minutes by top-5 tenants (hourly trend).
* Total CPU minutes / RAM GBh / requests / anchors compiled (stats).
* Per-tenant raw breakdown (table).

Reconciliation : the totals here should match the output of
`scripts/billing/aggregate_usage.py` ± 1 % (tracked by SLO
D11-MT-5 cost::aggregator_drift_pct).

---

## 05 — Audit Stream

**JTBD** : "Y a-t-il des patterns suspects dans les mutations
ou les auth failures ?"

**Variables** : `$tenant`, `$action`, `$op_tag`.

Key panels :

* Audit event rate by action / by op_tag (stacked timeseries).
* Top 10 tenants by audit volume 24h.
* Auth failure rate (5m) — stat with severity.
* Audit events with empty principal (% — alert source for D13ObsAuditPrincipalUnknown).
* z-score remove_class rate vs 7d baseline (anomaly detection).
* Loki tail of `{stream="audit"}`.

---

## 06 — Capacity Planning

**JTBD** : "Quand vais-je manquer de ressources ?"

Key panels :

* Total active tenants / catalogs / versions (stats with thresholds).
* Cardinality budget : `count(count by (__name__, job) ({__name__=~"efe_.+"}))` — alert source for D13ObsCardinalityBomb.
* RPS — 30 d trend with linear forecast (`predict_linear`).
* CPU saturation per pod (stacked).
* Tenants near catalog version quota (table, color-coded).
* Top 20 high-cardinality metric families.

---

## Variables — global conventions

| Variable | Source | Notes |
|----------|--------|-------|
| `$datasource` | Prometheus datasource | Default = first available |
| `$loki_datasource` | Loki datasource | Optional ; absent = panel "no data" |
| `$tenant` | `label_values(efe_grpc_requests_total, tenant)` | Multi-select where it makes sense |
| `$rpc` | `label_values(efe_grpc_requests_total, method)` | Multi |
| `$catalog_name` | `label_values(efe_catalog_versions_per_name, name)` | Multi |
| `$stream` | static list (5 streams) | Used by 02 |
| `$action` | static list (3 actions) | Used by 05 |
| `$op_tag` | static list (7 op_tags) | Used by 05 |

---

## Recording rules used

The dashboards rely on 7 base recording rules + 5 downsamples :

| Recording rule | Used by | Cost |
|----------------|---------|------|
| `efe:rpc_p99_latency_seconds:5m` | 01, 02, 06 | ~ 3 ms |
| `efe:rpc_error_rate:5m` | 01, 02, 06 | ~ 2 ms |
| `efe:top20_tenants_by_rps:5m` | 01 | ~ 5 ms |
| `efe:catalog_evolve_rate:5m` | 03, 05 | ~ 2 ms |
| `efe:catalog_chain_depth:1m` | 03, 06 | ~ 1 ms |
| `efe:cpu_minutes_per_tenant:1h` | 04, 06 | ~ 4 ms |
| `efe:ram_gb_hours_per_tenant:1h` | 04, 06 | ~ 5 ms |

Total render budget per dashboard : ≤ 100 ms of PromQL eval +
~ 200 ms of Grafana overhead = ≤ 3 s end-to-end (SLO D13-OBS-1).

---

## Modifying a dashboard

The dashboards are committed JSON files in
`deploy/observability/grafana/dashboards/d13/`. To customise :

1. Fork the file under your own Helm chart values override.
2. Pass it through Helm's `Files.Glob` mechanism (or use a
   sidecar config).
3. Run `pytest scripts/tests/test_d13_dashboard_jsonschema.py`
   to validate.

The chart ships the **upstream** dashboards via the
`grafana_dashboard=1` ConfigMaps. Customisation is per-deploy.

---

## Color & threshold conventions

| Threshold | Color | Meaning |
|-----------|-------|---------|
| Green | OK | Within SLO |
| Yellow | Warning | Above target but not critical |
| Orange | Caution | Approaching limit |
| Red | Critical | SLO breach |

Stat panels use `colorMode: background` for at-a-glance
incident response.
