# RB_PR19_MSW10 — `SLOBudgetBurnSlow6x`

| Field | Value |
|---|---|
| **Alert** | `SLOBudgetBurnSlow6x` |
| **Severity** | warning (ticket) |
| **SLO** | `iterate-availability` (99.9 % / 30d) |
| **Burn pattern** | Google SRE 2-window slow-burn (1h AND 6h > 6×) |
| **Budget exhaustion** | ~ 5 days at sustained burn |
| **Dashboards** | [D1 Overview](https://grafana.efe.internal/d/efe-msw-overview) · [D2 Iterate](https://grafana.efe.internal/d/efe-msw-iterate) |
| **PR ref** | [§ 31](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#31-alert-a10--slobudgetburnslow6x-google-sre-slow-burn) |

## Summary

The 30-day error budget is being consumed 6× faster than allowed.
Sustained, the budget exhausts within a working week. Not paging-worthy
in isolation, but **must be investigated during business hours**.

## Symptom

- Slack ticket in `#meta-swarm-alerts`.
- **D1 P5** "SLO budget remaining" trending down (still > 0, not critical).
- **D1 P9** "SLO burn rate" line crossing the orange (6×) band.

## Diagnose

Identical diagnostic surface to RB_PR19_MSW9 / RB_PR19_MSW1 — the only
difference is the time horizon :

1. **D2 P7** "Iterate failures by failure_reason" — what's the slow,
   constant background of failures ?
2. **D2 P1** — is one manifest version contributing disproportionately ?
   Canary-vs-stable comparison.
3. **D3 P3** "Step error rate" — is one participant consistently slow ?

Slow burns usually indicate either :
- A defect with low reproduction rate (e.g. specific input shape).
- A degraded but not failing dependency (e.g. slow database).
- A recent canary that's still rolling out with small but persistent errors.

## Mitigate

The slow burn gives you days, not minutes. Don't paste-and-pray.

| Pattern | Action |
|---|---|
| Canary worse than stable | Pause canary rollout : `helm upgrade --reuse-values --set canary.weight=0` |
| Single participant accumulating errors | Investigate during business hours ; restart if benign |
| Background defect (specific input) | Capture exemplar traces (D9), file ticket, fix in next release |
| External dependency degraded | Coordinate with the dependency owner |

## Escalate

| Time | Action |
|---|---|
| Sustained > 24 h | Page meta-swarm dev lead |
| Burn-rate trending toward 14.4× | Escalate immediately — A9 will fire shortly |

## Postmortem

Required when a slow-burn alert directly precedes a fast-burn (A9). The
postmortem should explain why the slow signal was missed and propose
detection improvements.
