# RB_PR19_MSW6 — `OTelExporterFailing`

| Field | Value |
|---|---|
| **Alert** | `OTelExporterFailing` |
| **Severity** | warning (ticket) |
| **Dashboards** | [D5 OTel Pipeline](https://grafana.efe.internal/d/efe-msw-otel) |
| **PR ref** | [§ 27](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#27-alert-a6--otelexporterfailing) |

## Summary

The OpenTelemetry SDK is dropping more than 10 % of spans at the
BatchSpanProcessor → exporter seam over 10 minutes. Trace-based debugging
becomes unreliable above 10 % drop rate.

## Symptom

- Slack ticket in `#meta-swarm-alerts`.
- **D5 P3** "Drop ratio" is red (> 10 %).
- **D5 P2** "Spans dropped by reason" shows a dominant reason.
- Exemplar clickthrough from D2 P8 returns "trace not found" for recent
  traces.

## Diagnose

Identify the dominant drop reason from **D5 P2** :

| Reason | Likely cause | First check |
|---|---|---|
| `queue_full` | BatchSpanProcessor backpressure (collector slow) | D5 P5 export duration |
| `export_error` | Collector unreachable, TLS bad, quota exceeded | D5 P4 collector up + collector logs |
| `timeout` | Collector accepted requests but doesn't respond | Collector logs + cluster networking |
| `shutdown` | Process drain during deploy — benign if transient | Cross-reference deploy annotations |

```bash
# Inspect the collector DaemonSet
kubectl -n observability logs -l app=otel-collector --tail=200

# Check collector resource usage
kubectl -n observability top pods -l app=otel-collector
```

## Mitigate

| Cause | Action |
|---|---|
| `queue_full` due to bursty traffic | Lower `tracing.otel.samplerArg` from 0.01 to 0.005 (1% → 0.5%) |
| `queue_full` persistent | Scale up collector (`kubectl -n observability scale daemonset otel-collector --replicas=+1` on selected nodes) |
| `export_error` (TLS) | Inspect `tracing.otel.tls.caBundleConfigMap` ; rotate CA |
| `export_error` (network) | Check NetworkPolicy + Service DNS |
| `timeout` | Increase collector resources ; investigate slow OTLP receivers |
| `shutdown` (transient) | No action — benign |

## Escalate

| Time | Action |
|---|---|
| Persistent > 1 h | Page meta-swarm-sre lead |
| Coincides with A1/A2 firing | The lost spans hamper triage — page collector owner immediately |

## Postmortem

Not required unless the failure correlates with a customer incident
where loss of traces hampered debugging.
