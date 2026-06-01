# RB_PR19_MSW2 — `IterateLatencyP99Critical`

| Field | Value |
|---|---|
| **Alert** | `IterateLatencyP99Critical` |
| **Severity** | critical (page) |
| **SLO** | `iterate-latency-p95` (95 % ≤ 1 s / 30d) — A2 is the p99 hard ceiling |
| **Dashboards** | [D2 Iterate](https://grafana.efe.internal/d/efe-msw-iterate) |
| **PR ref** | [§ 23](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#23-alert-a2--iteratelatencyp99critical) |

## Summary

Iterate p99 latency has exceeded 5 s for ≥ 5 minutes. User-visible. Possible
cause : single slow participant dominating the controller p99, or controller-
side saturation.

## Symptom

- PagerDuty page.
- **D1 P8** ("Iterate latency p50/p95/p99") red line crosses 5 s threshold.
- **D2 P1** ("Latency by manifest version") shows divergence if a canary
  is mid-rollout.

## Diagnose

1. **D2 P8** "p99 with exemplars" — click a red exemplar → opens trace in D9.
   Identify the longest span in the trace tree :
   - Long `efe.meta_swarm.step` for participant X ⇒ pivot to D3 with
     var-participant_id=X.
   - Long `efe.meta_swarm.connect_participants` ⇒ network / TLS issue,
     pivot to D6.
   - Long `efe.meta_swarm.inner_loop` ⇒ algorithm-level issue (manifest
     was tuned for harder convergence).
2. **D3 P5** "Step latency p99" per participant — sort descending,
   identify the slowest.
3. **D8 P1** "CPU" — controller > 80 % usage strongly correlates with
   queueing delays at p99.

## Mitigate

| Root cause | Mitigation |
|---|---|
| Single slow participant | Scale up that participant's replica or kill the pod ; the meta-swarm gracefully tolerates the brief loss |
| Controller CPU saturation | Bump cpu request via `kubectl edit deployment efe-meta-swarm-controller` |
| Recent manifest tuning issue | Roll back manifest (cf. RB_PR19_MSW8) |
| Network partition | Check `kubectl get networkpolicy -n efe` for an over-restrictive policy |

## Escalate

| Time | Action |
|---|---|
| + 10 m unmitigated | Page meta-swarm dev lead |
| + 30 m unmitigated | Open `#meta-swarm-warroom` |

## Postmortem

Required when alert open > 30 m. Use `docs/templates/POSTMORTEM_TEMPLATE.md`.
Required artifact : the slow trace ID + screenshot of D2 P1.
