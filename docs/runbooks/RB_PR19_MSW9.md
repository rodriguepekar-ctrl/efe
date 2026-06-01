# RB_PR19_MSW9 — `SLOBudgetBurnFast14x`

| Field | Value |
|---|---|
| **Alert** | `SLOBudgetBurnFast14x` |
| **Severity** | critical (page) |
| **First responder** | meta-swarm-sre on-call |
| **Escalation** | meta-swarm dev lead → engineering manager |
| **SLO** | `iterate-availability` (99.9 % / 30d) |
| **Burn pattern** | Google SRE 2-window fast-burn (5m AND 1h windows > 14.4×) |
| **Budget exhaustion** | ~ 2 days at sustained burn |
| **Inhibits** | `IterateFailureRateHigh` (same root cause) |
| **Dashboards** | [D1 Overview § SLO panels](https://grafana.efe.internal/d/efe-msw-overview) · [D2 Iterate](https://grafana.efe.internal/d/efe-msw-iterate) |
| **PR reference** | [PR #19 § 30](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#30-alert-a9--slobudgetburnfast14x-google-sre-2-window-fast-burn) |

## Summary

The 30-day iterate-availability error budget is being consumed 14.4× faster
than the allowed rate. Sustained, the entire monthly budget is gone in ~ 2 days.
Both the 5-minute and 1-hour windows have exceeded the threshold, ruling out
a transient blip.

## Symptom

- PagerDuty page on `SLOBudgetBurnFast14x` (severity=critical, labels :
  `slo_id=iterate-availability`, `burn_rate=fast`).
- Grafana **D1 Overview** P5 ("SLO budget remaining") drops below 90 %
  rapidly ; P9 ("SLO burn rate — availability") shows the red band (14.4×)
  being crossed.
- `IterateFailureRateHigh` is suppressed by inhibition rule — DO NOT expect
  to see it in the active alerts list.

## Diagnose

### Step 1 — Compute the actual burn

On **D1 P9**, read the current burn-rate value. It's already normalised to
the error budget : a value of 14.4 means 14.4× the allowed rate ; a value
of 30 means the budget exhausts in < 1 day if sustained.

If burn rate ≥ 30 → **incident severity SEV-1** ; trigger the full incident
process. If 14.4 ≤ burn rate < 30 → **SEV-2** ; first responder mitigates,
no full incident.

### Step 2 — Reduce to A1's diagnostic surface

This alert is the SLO-aware variant of `IterateFailureRateHigh`. The
diagnostic surface is identical : open **D2 Iterate** P7 and identify the
dominant `failure_reason` (cf. [RB_PR19_MSW1](RB_PR19_MSW1.md#diagnose) Step 1).

### Step 3 — Check correlated deploys

The 2-window pattern requires the burn to be sustained ≥ 5 m AND visible
in the 1 h window. A sudden burn always points to one of :

1. A manifest reload in the past hour → cross-reference **D4 Reload** P8
   (Recent reload events). Suspect the most recent.
2. A binary deploy in the past hour → cross-reference **D1 P11** (Build
   info) annotations on **D1 P7** (Throughput).
3. A participant fleet event (mass restart, autoscaling event) →
   **D3 Participant** + **D8 Runtime P5** (Available replicas).

## Mitigate

### Path A — Confirm root cause is a recent change

1. **If deploy in last 1 h** :
   ```bash
   helm rollback efe-meta-swarm <previous-revision> --wait --timeout 3m
   ```
   Verify D1 P5 (budget) starts recovering within 5 m.

2. **If manifest reload in last 1 h** :
   ```bash
   # See RB_PR19_MSW8 for the rollback procedure
   ```

3. **If neither** : escalate immediately (Step "Escalate" below). The
   burn without a recent change indicates an external dependency
   degradation (network, etcd, kube-apiserver) or a hardware fault.

### Path B — Reduce blast radius without rolling back

If rollback isn't safe (e.g. data-loss risk), **reduce traffic** :

1. Scale down replicas to limit the failure surface :
   ```bash
   kubectl -n efe scale deployment efe-meta-swarm-controller --replicas=1
   ```
2. Reduce canary weight to 0 :
   ```bash
   helm upgrade efe-meta-swarm charts/efe-stack/charts/efe-meta-swarm \
     --reuse-values --set canary.weight=0
   ```
3. Verify burn-rate trends down on D1 P9.

### Path C — Burn rate ≥ 30 (SEV-1)

Activate the incident commander rotation. Page :
- Meta-swarm engineering manager
- Platform engineering lead
- CTO (mandatory for SEV-1)

Follow the full incident process in `docs/INCIDENT_RESPONSE.md`.

## Escalate

The SLO budget alert is by definition customer-impactful. Standard
escalation tree applies, **shortened by 50 %** from RB_PR19_MSW1 :

| Time | Action |
|---|---|
| Page received | Acknowledge in PagerDuty within 3 m |
| + 10 m unmitigated | Page meta-swarm dev lead |
| + 20 m unmitigated | Page engineering manager + open `#meta-swarm-warroom` |
| + 30 m unmitigated | Page CTO + activate incident commander |

## Postmortem

**Mandatory** for every fast-burn alert — the SLO is a customer contract.

- **Template** : `docs/templates/POSTMORTEM_TEMPLATE.md` (Blameless format).
- **Required sections** :
  1. Burn-rate timeline (screenshot of D1 P9 covering the full incident).
  2. Time-to-detect (alert fired at vs root-cause-introduced at).
  3. Time-to-mitigate.
  4. Budget consumed (compute from D1 P5 delta).
  5. Action items to prevent recurrence + their owners + due dates.
- **SLO budget reconciliation** : if more than 25 % of monthly budget
  was consumed in a single incident, schedule a follow-up SRE review
  within 7 days to evaluate whether the target needs revision.
- **Past incidents** : (none yet — PR #19 first roll-out)
