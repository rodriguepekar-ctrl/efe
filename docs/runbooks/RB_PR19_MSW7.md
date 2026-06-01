# RB_PR19_MSW7 — `SafetyViolationSpike`

| Field | Value |
|---|---|
| **Alert** | `SafetyViolationSpike` |
| **Severity** | warning (ticket) |
| **Dashboards** | [D7 Safety](https://grafana.efe.internal/d/efe-msw-safety) |
| **PR ref** | [§ 28](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#28-alert-a7--safetyviolationspike) |

## Summary

The total rate of safety geometry violations (box / poly / hull) exceeds
0.1/s sustained for 5 minutes. Phase-2 algorithm should have a nominal
rate of ~ 0 — sustained 6/min indicates either a misbehaving learner or
a too-aggressive safety relaxation in the manifest.

## Symptom

- Slack ticket.
- **D7 P2** "Violation rate" stat is red.
- **D7 P3** "Top 10 participants" identifies the offender(s).
- **D7 P6** cross-reference timeseries shows iterate failures with
  `failure_reason=safety_violation` rising in lockstep.

## Diagnose

### Step 1 — Is it concentrated on one participant ?

On **D7 P3**, look at the topk(10) bar gauge.
- One participant > 50 % of total → participant misbehaving ; pivot to
  **D3** with var-participant_id = that ID.
- Spread across many participants → the manifest safety geometry is too
  tight ; or the iterate algorithm is in a regime that consistently
  brushes the boundary.

### Step 2 — Which kind ?

D7 P2 stacked by `kind` (`box`, `poly`, `hull`) :
- `box` violations dominate → axis-aligned bound exceeded ; usually a
  single dimension is out of spec.
- `poly` / `hull` → more complex geometry violation, may indicate the
  learner is jumping.

### Step 3 — Recent manifest change ?

Cross-reference with D1 annotations — did `manifest.safety.*` change
recently ?

## Mitigate

| Pattern | Action |
|---|---|
| Single participant offender | Restart its pod (`kubectl -n efe delete pod <participant>`) ; if it persists, scale to 0 and investigate |
| Manifest too tight | Roll back manifest (cf. RB_PR19_MSW8) OR raise safety margin in-place |
| Algorithm regime change | Reduce learning rate / increase contractive factor `coupling_gamma` in manifest |

## Escalate

| Time / pattern | Action |
|---|---|
| Sustained > 1 h | Page meta-swarm dev lead |
| Coincides with a manifest deploy | Roll back without further investigation, postmortem later |
| Violations crossing compliance threshold | Page compliance officer (rare — only matters for regulated tenants) |

## Postmortem

Required when violations cross 1 % of total iterate calls in a 1-hour
window (proxy for "this affected the convergence of customer workloads").
