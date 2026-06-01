# RB_PR19_MSW1 — `IterateFailureRateHigh`

| Field | Value |
|---|---|
| **Alert** | `IterateFailureRateHigh` |
| **Severity** | critical (page) |
| **First responder** | meta-swarm-sre on-call |
| **Escalation** | meta-swarm dev lead → meta-swarm engineering manager |
| **SLO** | `iterate-availability` (99.9 % / 30d) |
| **Inhibited by** | `ControllerNotServing`, `SLOBudgetBurnFast14x` |
| **Dashboards** | [D1 Overview](https://grafana.efe.internal/d/efe-msw-overview) · [D2 Iterate](https://grafana.efe.internal/d/efe-msw-iterate) |
| **PR reference** | [PR #19 § 22](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#22-alert-a1--iteratefailureratehigh) |
| **STABILITY tier** | Alert name — STABILITY_CONTRACT v1.1 |

## Summary

The iterate RPC is failing for more than 5 % of requests over the last 5
minutes. This is the user-facing symptom alert ; investigate immediately.

## Symptom

- PagerDuty page on alert `IterateFailureRateHigh`.
- Grafana **D1 Overview** P3 ("Iterate success rate (5m)") drops below 95 % and
  the stat panel turns red/orange.
- Grafana **D2 Iterate** P2 ("Throughput by result") shows a visible red band
  (errors) above the previously dominant green band (ok).
- Downstream consumers (efe-server, efe-edge) may surface their own elevated
  error rates on `EFEHighErrorRate` (legacy alert).

## Diagnose

### Step 1 — Identify the dominant failure_reason

Open **D2 Iterate** → P7 ("Iterate failures by failure_reason"). Look at the
top of the stacked area :

| Top reason | Likely root cause | Next dashboard |
|---|---|---|
| `safety_violation` | Newly deployed manifest loosened safety geometry, or a learner is misbehaving | [D7 Safety](https://grafana.efe.internal/d/efe-msw-safety) |
| `participant_down` | One or more participants unreachable | [D3 Participant](https://grafana.efe.internal/d/efe-msw-participant) — find which ID |
| `deadline_exceeded` | Saturation (CPU / FDs / network) | [D8 Runtime](https://grafana.efe.internal/d/efe-msw-runtime) |
| `dim_mismatch` | Manifest reload mismatch between controller and participants | [D4 Reload](https://grafana.efe.internal/d/efe-msw-reload) |
| `internal` | Likely a panic or assertion failure ; needs a trace | [D9 Traces](https://grafana.efe.internal/d/efe-msw-traces) |
| `timeout` | Network partition or slow participant | D3 + D8 |
| `other` | Investigate via traces directly | D9 |

### Step 2 — Confirm the recent deploy / reload

On **D1** read :
- **P4** "Manifest reload gen" — did it change in the last 30 m ? Compare
  to the timeline annotations (blue markers).
- **P11** "Build info" — verify the running version + git_sha match the
  intended deploy.

If a manifest reload happened minutes before the alert, the manifest is
the prime suspect.

### Step 3 — Grab a slow / failing trace

On **D2 Iterate** P8 ("p99 with exemplars"), click any **red exemplar dot**
near the alert onset time. Grafana opens **D9** with the trace pre-filled.
Inspect the span tree :

- Failed at `efe.meta_swarm.iterate` root → controller-side rejection
  (often `failure_reason=safety_violation` or `dim_mismatch`).
- Failed at `efe.meta_swarm.step` → a specific participant returned an
  error. The `efe.participant.id` attribute names it.
- Failed at `efe.meta_swarm.connect_participants` → discovery / mTLS
  handshake failure ; pivot to [D6 TLS](https://grafana.efe.internal/d/efe-msw-tls).

### Step 4 — Capture diagnostics for the postmortem

```bash
# Save the trace ID for the postmortem ticket
TRACE_ID="<from D9 URL>"

# Snapshot the relevant dashboards (link them in the ticket)
kubectl -n efe logs -l app.kubernetes.io/component=controller --tail=500 \
  > "/tmp/msw1-${TRACE_ID}-controller.log"
kubectl -n efe get configmap efe-meta-swarm-manifest -o yaml \
  > "/tmp/msw1-${TRACE_ID}-manifest.yaml"
```

## Mitigate

The mitigation depends on the dominant `failure_reason` identified in Step 1.

### Path A — `failure_reason=safety_violation` is dominant

This means the contractive iteration is producing states that fall outside
the manifest's declared safety geometry. Almost always : a manifest
loosened the geometry, or a participant is mis-trained.

1. **Roll back the manifest** if the issue started within ~ 5 m of a reload :
   ```bash
   # Find the previous reload_gen
   kubectl -n efe get cm efe-meta-swarm-manifest-history -o yaml | head -100
   # Restore (operator-specific procedure ; see RB_PR19_MSW8 for details)
   kubectl -n efe apply -f /etc/efe/manifests/manifest-prev.yaml
   kubectl -n efe rollout restart deployment efe-meta-swarm-controller
   ```
2. **If rollback isn't possible**, raise the safety margin in-place :
   ```bash
   # Edit safety.box.margin (or safety.poly.margin, .hull.margin)
   kubectl -n efe edit configmap efe-meta-swarm-manifest
   # The controller picks up the change on its inotify watcher
   ```
3. **Verify mitigation** : D1 P3 (success rate) should recover to > 95 %
   within ~ 5 m.

### Path B — `failure_reason=participant_down` is dominant

A participant became unreachable. The meta-swarm tolerates losing one
participant — but the failure rate spike means either multiple participants
are down or the loss correlates with a hot-spotted shard.

1. **Identify which** : D3 → set var-participant_id to "All", look at the
   "Step throughput" panel for which row went to zero.
2. **Scale up** :
   ```bash
   kubectl -n efe scale deployment efe-meta-swarm-participant --replicas=+1
   ```
3. **Investigate the down participant's pod** :
   ```bash
   kubectl -n efe describe pod <participant-pod>
   kubectl -n efe logs <participant-pod> --tail=200
   ```

### Path C — `failure_reason=deadline_exceeded` (saturation)

1. Check D8 P1 (CPU) and D8 P2 (RSS). If either is at 90 %+ of the limit,
   increase the controller resource request :
   ```bash
   kubectl -n efe edit deployment efe-meta-swarm-controller
   # Bump cpu request from 2 to 4 cores
   ```
2. Verify the HPA is enabled and reacting :
   ```bash
   kubectl -n efe describe hpa efe-meta-swarm-controller
   ```

### Path D — `failure_reason=internal` (panic / assertion)

This is a defect. The mitigation is to reduce traffic exposure :

1. Scale down canary weight if a canary roll-out is in progress :
   ```bash
   helm upgrade efe-meta-swarm charts/efe-stack/charts/efe-meta-swarm \
     --reuse-values --set canary.weight=10
   ```
2. Capture the trace (Step 3 above) — it carries the panic location in
   `efe.error.message`.
3. File an incident ticket using `docs/templates/POSTMORTEM_TEMPLATE.md`.

## Escalate

| Time | Action |
|---|---|
| Page received | Acknowledge in PagerDuty within 5 m |
| + 15 m unmitigated | Page meta-swarm dev lead via PagerDuty escalation policy `meta-swarm-l2` |
| + 30 m unmitigated | Page meta-swarm engineering manager + post in `#meta-swarm-warroom` Slack |
| + 1 h unmitigated | Page CTO + activate incident commander rotation |

**Trace evidence of a panic / assertion** : page **both** security and
meta-swarm dev simultaneously — a panic may be a vulnerability surface.

## Postmortem

Required when alert is open > 30 m or root cause is non-trivial.

- **Template** : `docs/templates/POSTMORTEM_TEMPLATE.md` (Blameless format).
- **Required artifacts** :
  - Trace ID from Step 3.
  - The controller log snippet (`/tmp/msw1-${TRACE_ID}-controller.log`).
  - The manifest snapshot (`/tmp/msw1-${TRACE_ID}-manifest.yaml`).
  - Grafana dashboard URL with the alert time range pre-filled.
- **Past incidents** : (none yet — PR #19 first roll-out)
- **SLO impact** : compute via D1 P5 (SLO budget remaining) before and
  after the incident window — record the consumed fraction.
