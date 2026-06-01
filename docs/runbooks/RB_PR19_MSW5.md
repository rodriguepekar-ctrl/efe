# RB_PR19_MSW5 — `ControllerNotServing`

| Field | Value |
|---|---|
| **Alert** | `ControllerNotServing` |
| **Severity** | critical (page) |
| **Dashboards** | [D8 Runtime](https://grafana.efe.internal/d/efe-msw-runtime) · [D1 Overview](https://grafana.efe.internal/d/efe-msw-overview) |
| **PR ref** | [§ 26](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#26-alert-a5--controllernotserving) |
| **Inhibits** | A1, A2, A3, A6, A7, A8 (all downstream) |

## Summary

The meta-swarm controller pod is unreachable to Prometheus for ≥ 1 minute.
Every iterate call fails. **This is the highest-priority alert in the
PR #19 catalogue.**

## Symptom

- PagerDuty page within 30 seconds of detection.
- **D1 P1** "Controller status" = DOWN (red).
- All other meta-swarm alerts are inhibited (don't be alarmed by the silence).
- Downstream EFE-Server `EFEHighErrorRate` will fire shortly.

## Diagnose

```bash
# Quick triage
kubectl -n efe get pods -l app.kubernetes.io/component=controller -o wide

# If pods exist but not Ready
kubectl -n efe describe pod <controller-pod>
kubectl -n efe logs <controller-pod> --tail=300

# If pod is in CrashLoopBackOff
kubectl -n efe logs <controller-pod> --previous --tail=300
```

Possible states :
1. **Pod gone** (replicas=0) → someone scaled it to 0. Check audit log.
2. **CrashLoopBackOff** → application-level panic. Look at last log lines.
3. **Pending** → scheduling issue (taints, node pressure, PVC bound).
4. **OOMKilled** → see D8 P6.
5. **Pod healthy but scrape failing** → ServiceMonitor / metrics endpoint
   broken, NOT a controller-down condition — investigate separately.

## Mitigate

| State | Mitigation |
|---|---|
| Replicas=0 | `kubectl -n efe scale deployment efe-meta-swarm-controller --replicas=2` |
| CrashLoopBackOff (recent deploy) | `helm rollback efe-meta-swarm <previous-revision>` |
| OOMKilled | Increase memory limit immediately ; investigate leak post-mitigation |
| Node pressure | Cordon the saturated node, let scheduler relocate |
| ConfigMap missing | Re-apply the chart manifest ConfigMap |

## Escalate

| Time | Action |
|---|---|
| Page received | Acknowledge in PagerDuty within 2 m |
| + 5 m unmitigated | Page meta-swarm dev lead |
| + 10 m unmitigated | Page engineering manager + open `#meta-swarm-warroom` + activate incident commander |
| + 20 m unmitigated | Page CTO — this is a customer-impacting outage |

## Postmortem

**Mandatory** — A5 firing means every meta-swarm consumer was failing.

- Use `docs/templates/POSTMORTEM_TEMPLATE.md`.
- Required : measured downtime, customer-facing impact assessment,
  prevention actions for the specific root cause.
- File a SEV-1 incident report within 24 h.
