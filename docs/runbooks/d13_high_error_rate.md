# d13_high_error_rate — Runbook : high error rate per tenant

> **Severity** : critical (page on-call).
> **Alert** : `D13ObsHighErrorRate`.
> **Owner** : SRE on-call rotation.
> **Updated** : 2026-04-27.
> **Related dashboards** : `01_operator_overview`,
> `02_tenant_drill_down`.

## 1. Alert summary

The error rate (5 m window) for tenant `$tenant` exceeded **5 %**
for 10 minutes. The dominant gRPC code is shown in the
"Error rate per RPC" panel.

## 2. First 5 minutes — triage

1. Open dashboard
   `02_tenant_drill_down?var-tenant=<tenant>`.
2. Identify the dominant error code in the "Error breakdown by
   code" panel.
3. Check Loki for stack traces : pivot via the `trace_id`
   exemplar visible on the latency heatmap.
4. Check upstream dependencies :

   ```bash
   kubectl get pods -n efe-system | grep -v Running
   kubectl get pods -n upstream-* | grep -v Running
   ```

5. Check rate-limit drops :

   ```promql
   sum(rate(efe_ratelimit_drops_total{tenant="<tenant>"}[5m]))
   ```

   If elevated, the tenant is being throttled — that's a
   legitimate response, not an EFE bug.

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| Embedder pool overload | INTERNAL errors, p99 spike | Scale embedder pool replicas |
| JWT misconfig at tenant | UNAUTHENTICATED dominant | Check tenant token config |
| Catalog corruption | INVALID_ARGUMENT spike | `efe-admin catalog get $key` to verify |
| Rate limit too aggressive | RESOURCE_EXHAUSTED | Loosen tenant bucket (cf. D11) |
| Network partition | UNAVAILABLE | Check istio/nginx logs |
| Schema regression | INVALID_ARGUMENT after deploy | Roll back the most recent release |

## 4. Mitigation (immediate)

- If embedder overloaded :
  ```bash
  kubectl scale deploy/embedder --replicas=$(($current * 2))
  ```
- If catalog corruption : roll back to last known-good version
  via `efe-admin catalog versions --name X` then re-pin.
- If rate limit too aggressive :
  ```bash
  efe-admin ratelimit set --tenant <tenant> --capacity 1000
  ```

## 5. Resolution (durable)

- File a ticket with the dashboard screenshot.
- Add a regression test covering the failure mode.
- Update this runbook if you found a new root cause.

## 6. Escalation

If unresolved at T+30 min : page tier-2 SRE via Pagerduty
`@efe-oncall-tier-2`.

## 7. Postmortem

Use template `docs/operations/postmortem-template.md`.
