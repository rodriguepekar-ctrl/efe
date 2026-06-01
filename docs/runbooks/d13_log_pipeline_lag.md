# d13_log_pipeline_lag — Runbook : promtail position cursor > 30s behind

> **Severity** : warning.
> **Alert** : `D13ObsLogPipelineLag`.
> **Owner** : SRE.

## 1. Alert summary

promtail's last_seen position is > 30 s behind real time. Logs
are not landing in Loki within the SLO window. SRE may be
debugging an outdated view.

## 2. First 5 minutes

1. Check promtail health :
   ```bash
   kubectl logs -l app.kubernetes.io/name=promtail | tail -100
   ```
2. Check Loki ingester saturation :
   ```promql
   sum(rate(loki_ingester_streams_created_total[5m]))
   ```
3. Check disk space on promtail / Loki nodes :
   ```bash
   kubectl describe nodes | grep -A2 ephemeral-storage
   ```

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| Loki ingester rate limit | promtail backpressure | Raise Loki ingestion rate limit |
| promtail queue full | OOM-killed pods | Scale promtail replicas |
| Network partition | Connection timeouts | Check ingress |
| Unbounded log volume from one pod | Single tenant flooding | Per-tenant rate limit at promtail |

## 4. Mitigation

- Restart promtail :
  `kubectl rollout restart ds/promtail`.
- Raise Loki ingestion limit if rate-limited.

## 5. Resolution

- Add a per-tenant rate limit to the promtail config example.
- If a single tenant is flooding : reach out to them.

## 6. Escalation

If observability lag > 5 min : escalate (we're effectively
flying blind).
