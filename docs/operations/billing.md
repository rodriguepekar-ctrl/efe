# EFE — Billing aggregation runbook

> **Sprint D11.5 deliverable.** Audience: SRE / finance ops at the
> EFE provider. Computes the per-tenant usage CSV / JSON used by
> the billing pipeline.

---

## Overview

EFE emits three Prometheus counters, each labelled by `tenant` :

| Metric | Unit | What it counts |
|--------|------|---------------|
| `efe_grpc_requests_total{tenant}` | requests | every authenticated RPC |
| `efe_classify_compute_seconds_total{tenant}` | seconds | wallclock fusion compute |
| `efe_egress_bytes_total{tenant}` | bytes | gRPC response bytes shipped |

The aggregator script reads these via Prometheus' `query_range`
HTTP API, sums them over the billing window, and emits a flat
CSV / JSON ready for ingestion.

---

## Usage

### Monthly billing close

```bash
python scripts/billing/aggregate_usage.py \
  --prom-url http://prometheus.monitoring.svc:9090 \
  --start 2026-04-01T00:00:00Z \
  --end   2026-05-01T00:00:00Z \
  --output /var/billing/2026-04.csv
```

Output (CSV):

```
tenant,requests,compute_seconds,egress_bytes
acme,124300,1854.20,8732145
globex,8421,103.50,541230
contoso,55920,840.75,3925180
```

### JSON output (richer metadata)

```bash
python scripts/billing/aggregate_usage.py \
  --prom-url http://prometheus:9090 \
  --start 2026-04-01T00:00:00Z \
  --end   2026-05-01T00:00:00Z \
  --output /var/billing/2026-04.json
```

```json
{
  "schema_version": "1.0",
  "generated_at": "2026-05-01T03:00:00+00:00",
  "metadata": {
    "prom_url": "http://prometheus:9090",
    "start": "2026-04-01T00:00:00Z",
    "end": "2026-05-01T00:00:00Z",
    "window": "30d",
    "step": "1h",
    "elapsed_s": 4.73
  },
  "tenants": {
    "acme":    {"requests": 124300, "compute_seconds": 1854.20, "egress_bytes": 8732145},
    "globex":  {"requests": 8421,   "compute_seconds": 103.50,  "egress_bytes": 541230}
  }
}
```

### Restrict to known tenants only

If your Prometheus has been polluted with unknown tenant labels
(cardinality bomb attempt, mis-issued JWTs), use `--allow-tenants` :

```bash
python scripts/billing/aggregate_usage.py \
  --prom-url http://prometheus:9090 \
  --start 2026-04-01T00:00:00Z \
  --end   2026-05-01T00:00:00Z \
  --allow-tenants acme \
  --allow-tenants globex \
  --allow-tenants contoso \
  --output /var/billing/2026-04.csv
```

Same effect can be achieved upstream via the ServiceMonitor's
`tenant.allowlist` (Helm value) — that's the recommended
defense-in-depth.

---

## Scheduling

Recommended : a daily cron pulling **the previous day** (24 h
window), and a monthly close pulling the full month.

### Kubernetes CronJob example

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: efe-billing-daily
  namespace: billing
spec:
  schedule: "30 0 * * *"   # 00:30 UTC every day
  jobTemplate:
    spec:
      template:
        spec:
          restartPolicy: OnFailure
          serviceAccountName: efe-billing
          containers:
            - name: aggregate
              image: ghcr.io/efe-project/efe-bench:0.1.11
              command: ["/bin/sh", "-c"]
              args:
                - |
                  set -e
                  yesterday=$(date -u -d "yesterday" +%Y-%m-%dT00:00:00Z)
                  today=$(date -u +%Y-%m-%dT00:00:00Z)
                  python /opt/efe/scripts/billing/aggregate_usage.py \
                    --prom-url http://prometheus.monitoring:9090 \
                    --start "$yesterday" \
                    --end "$today" \
                    --output "/var/billing/daily/$(date -u -d 'yesterday' +%Y-%m-%d).json"
```

The `efe-billing` ServiceAccount needs read access to the
Prometheus query API and write access to wherever you ship the
output (S3 / GCS / NFS / managed file store).

---

## Resilience properties

* **Prometheus restart in window** — the aggregator uses
  `increase()` (not `delta()`), which Prometheus computes as the
  rate-corrected delta and is robust to scrape gaps. Cf. the
  architecture supplement §13.
* **Idempotent** — running twice produces the same output ; safe
  to retry.
* **Stateless** — no DB, no local cache. The script's only inputs
  are Prometheus + CLI args.

---

## Troubleshooting

### Aggregator returns empty

* Prometheus has no data with the `tenant` label in the window.
  Check `tenant.namespace.isolated` is true on the chart, OR
  that the soft-isolation in-process layer is emitting metrics.
* Check the ServiceMonitor `metricRelabeling` allowlist isn't
  filtering everything out.

### `_unknown` tenant in raw data

A pre-D11 deployment emitted metrics without the `tenant` label.
The aggregator drops `_unknown` automatically. To investigate,
inspect the raw Prometheus :

```promql
sum by (tenant) (efe_grpc_requests_total)
```

### Drift between aggregator total and Prometheus dashboard

* Aggregator uses `increase()` over a *fixed* window ; the
  dashboard may use `rate()` over a moving window. Different
  numbers, both correct in their context.
* SLO `D11-MT-5` budgets ≤ 1 % drift between aggregator and a
  spot-check `count_over_time` query — investigate beyond that.

---

## Related

* [How-to / Multi-tenant deployment](../docs/howto/multi-tenant.md) — overview of the per-tenant labelling that feeds this aggregator.
* [Operations / Multi-tenant observability](multi-tenant-observability.md) — the Grafana dashboard side.
* `scripts/billing/aggregate_usage.py` — source.
