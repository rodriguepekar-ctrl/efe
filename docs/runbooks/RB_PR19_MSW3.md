# RB_PR19_MSW3 — `ParticipantDown`

| Field | Value |
|---|---|
| **Alert** | `ParticipantDown` |
| **Severity** | warning (ticket) |
| **Dashboards** | [D3 Participant](https://grafana.efe.internal/d/efe-msw-participant) |
| **PR ref** | [§ 24](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#24-alert-a3--participantdown) |

## Summary

The controller cannot reach participant `<participant_id>` for ≥ 2 minutes.
Meta-swarm tolerates losing one participant — multi-participant loss
escalates to A1 (`IterateFailureRateHigh`).

## Symptom

- Slack ticket in `#meta-swarm-alerts` with the participant_id.
- **D3 P1** "Participant up" = DOWN.
- D1 P2 "Healthy participants" drops below 100 %.

## Diagnose

1. Identify the pod :
   ```bash
   kubectl -n efe get pods -l app.kubernetes.io/component=participant \
     -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{.status.phase}{"\t"}{.status.containerStatuses[0].ready}{"\n"}{end}'
   ```
2. Inspect logs :
   ```bash
   kubectl -n efe logs <pod> --tail=300
   kubectl -n efe describe pod <pod>
   ```
3. **D3 P4** "TLS handshake success rate" — sustained 0 % suggests mTLS
   certificate trust issue (pivot to D6).

## Mitigate

| Cause | Action |
|---|---|
| Pod CrashLoopBackOff | `kubectl -n efe describe pod <pod>` → fix root cause (image pull, configmap, OOM) |
| TLS misconfig | See RB_PR19_MSW4 + D6 |
| Network policy block | `kubectl -n efe get networkpolicy` |
| Node failure | `kubectl drain` the bad node ; let the scheduler relocate |
| Truly gone (Node deleted) | Scale up : `kubectl -n efe scale deployment efe-meta-swarm-participant --replicas=+1` |

## Escalate

| Time | Action |
|---|---|
| + 30 m unmitigated | Page meta-swarm dev lead |
| Multi-participant loss | Treat as A1 — see RB_PR19_MSW1 |

## Postmortem

Required when 2+ participants down concurrently OR alert open > 1 h.
