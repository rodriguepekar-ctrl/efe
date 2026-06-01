# How-to — observability setup (D13)

> Sprint **D13** (v0.1.13). Sets up the 6 Grafana dashboards, the
> 7 recording rules, and the 10 alerts on a Helm-managed EFE
> deployment. Assumes Prometheus + Grafana are already running ;
> Loki + Tempo are optional.

This page is one of four D13 docs. For deeper material, see :

* [Cardinality management](./cardinality-management.md) — how to
  budget the per-tenant label explosion.
* [Dashboard catalog](../reference/dashboard-catalog.md) —
  what each panel shows and how to read it.
* [Observability architecture](../explanation/observability-architecture.md)
  — why the stack is shaped this way.

---

## Prerequisites

* `efe-server` ≥ v0.1.13.
* Kubernetes cluster with the [Prometheus operator][promop]
  installed (any flavour : kube-prometheus-stack, prom operator
  helm chart, …).
* Grafana ≥ 10.2 with the `kiwigrid/k8s-sidecar` enabled and
  configured to pick up ConfigMaps with the
  `grafana_dashboard=1` label.
* Optional : Loki ≥ 2.9 + promtail.
* Optional : Tempo ≥ 2.3 with OTLP/gRPC ingester.

[promop]: https://github.com/prometheus-operator/prometheus-operator

---

## 1. Enable in 3 lines (Tier 2 minimum)

In your Helm values :

```yaml
observability:
  dashboards:
    enabled: true
  recordingRules:
    enabled: true
  alerts:
    enabled: true
```

Apply with :

```bash
helm upgrade efe-acme deploy/helm/efe \
    --namespace tenant-acme \
    -f values.tier2.yaml
```

The chart will :

1. Render 6 ConfigMaps named
   `efe-acme-dashboard-01-operator-overview`, …,
   `…-06-capacity-planning`, each carrying
   `grafana_dashboard=1`.
2. Create one PrometheusRule resource bundling the 7 recording
   rules + 10 alerts.
3. Wire the recording rules to the EFE namespace so the
   `efe:*` aggregates are computed every 30 s.

---

## 2. Verify the dashboards loaded

After 30-60 seconds (sidecar polling cycle), open Grafana →
Dashboards → folder `EFE`. You should see the 6 dashboards :

| UID | Title |
|-----|-------|
| `efe-d13-01-operator-overview` | EFE — 01 Operator Overview |
| `efe-d13-02-tenant-drill-down` | EFE — 02 Tenant Drill-Down |
| `efe-d13-03-catalog-version-distribution` | EFE — 03 Catalog Version Distribution |
| `efe-d13-04-cost-attribution` | EFE — 04 Cost Attribution |
| `efe-d13-05-audit-stream` | EFE — 05 Audit Stream |
| `efe-d13-06-capacity-planning` | EFE — 06 Capacity Planning |

If a dashboard is missing :

```bash
# Check that the ConfigMap was created
kubectl get cm -l grafana_dashboard=1 -n tenant-acme

# Tail the sidecar logs for picks
kubectl logs -l app.kubernetes.io/name=grafana -c k8s-sidecar | grep dashboard

# Verify the label selector matches what the sidecar looks for
kubectl exec -n grafana deploy/grafana -c k8s-sidecar -- env | grep LABEL
```

The sidecar default label is `grafana_dashboard=1` ; some
helm charts use `grafana_dashboards`. If yours differs, set
the right label on the ConfigMaps (impossible via values.yaml
directly — you'd patch the Helm template).

---

## 3. (Optional) Wire Loki for log queries

The 6 dashboards each have a final row that queries Loki for
the structured log streams. Without Loki, those panels show
"datasource not configured" but the rest of the dashboard works.

To enable :

```yaml
observability:
  loki:
    enabled: true
    endpoint: "http://loki.observability.svc.cluster.local:3100/loki/api/v1/push"
    promtailExample: true
```

Then deploy promtail (or your existing log shipper) using the
example config rendered as a ConfigMap :

```bash
kubectl get cm efe-acme-promtail-example \
    -n tenant-acme \
    -o jsonpath='{.data.promtail-config\.yaml}' \
    > my-promtail-config.yaml

# Customize the loki endpoint, redact patterns, etc.
vim my-promtail-config.yaml

# Apply via your promtail Helm chart of choice
helm upgrade promtail grafana/promtail \
    -f my-promtail-config.yaml
```

The 5 streams expected by the dashboards :

* `request` — one event per gRPC call.
* `audit` — one event per catalog/cone mutation (D12).
* `classify` — one event per Classify call (no PII).
* `evolution` — one event per EvolveCatalog call.
* `security` — auth/authz failures.

JSON-Schemas are committed under
`deploy/observability/log-schemas/`.

---

## 4. (Optional) Wire Tempo for traces + exemplars

Traces are head-sampled at 5 % by default + always-sampled on
errors. To enable :

```yaml
observability:
  otel:
    enabled: true
    sampleRate: 0.05
    forceSampleOnError: true
  tempo:
    enabled: true
    endpoint: "http://tempo-distributor.observability.svc.cluster.local:4317"
```

The server initialises OpenTelemetry SDK at startup, sends
spans via OTLP/gRPC, and includes `traceID` exemplars in the
Prometheus histograms. Click a histogram bucket in dashboard 01
→ green dot = exemplar → opens the trace waterfall in Tempo.

---

## 5. Multi-tenant Grafana setup (Tier 2 partner)

When deploying for a partner who needs to give their own SREs
access to dashboards :

```yaml
observability:
  multiTenancy:
    enabled: true
```

This switches the chart to assume Mimir / Cortex / Thanos
behind the Grafana Prometheus datasource and threads the
tenant id via the `X-Scope-OrgID` header (Mimir / Cortex
multi-tenant model).

The default Grafana org for the tenant uses the dashboards
with the `$tenant` variable hardcoded to its own id —
cross-tenant queries are architecturally impossible (the
backend filters at query time via the org id header).

For full RBAC details, see [observability architecture
§ Multi-tenant data plane](../explanation/observability-architecture.md#multi-tenant-data-plane).

---

## 6. Sanity check — `helm test`

The chart ships a Helm test that hits the Grafana API for each
of the 6 dashboard UIDs and asserts HTTP 200 :

```bash
helm test efe-acme -n tenant-acme

# Expected output :
# OK : efe-d13-01-operator-overview
# OK : efe-d13-02-tenant-drill-down
# OK : efe-d13-03-catalog-version-distribution
# OK : efe-d13-04-cost-attribution
# OK : efe-d13-05-audit-stream
# OK : efe-d13-06-capacity-planning
# All 6 D13 dashboards loaded successfully.
```

If the test fails, see § 2 above for triage.

---

## 7. Common errors

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| "no data" on every panel | recording rules disabled | Enable `observability.recordingRules.enabled` |
| ConfigMap created but Grafana doesn't see it | sidecar label mismatch | Check `GF_SIDECAR_DASHBOARDS_LABEL` env var |
| Loki panels show "datasource error" | Loki endpoint wrong | Verify `observability.loki.endpoint` resolves from inside the cluster |
| Trace exemplars missing | OTel SDK off | Set `observability.otel.enabled = true` AND Prometheus runs with `--enable-feature=exemplar-storage` |
| Cross-tenant data leakage | Mimir multi-tenant not enabled | See § 5 + architecture § 12 |
| Alerts firing immediately | thresholds too tight | Override per-alert via `observability.alerts.overrides` |

---

## 8. What's next

* Browse [Dashboard catalog](../reference/dashboard-catalog.md) to
  understand what each panel measures.
* Read [Cardinality management](./cardinality-management.md) before
  scaling past 100 tenants.
* For incident response, every alert has a `runbook_url`
  pointing to `docs/runbooks/d13_*.md` — bookmark them.
