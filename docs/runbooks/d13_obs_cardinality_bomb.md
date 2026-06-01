# d13_obs_cardinality_bomb — Runbook : Prometheus cardinality > 50k

> **Severity** : warning (escalates to critical if Prom OOMs).
> **Alert** : `D13ObsCardinalityBomb`.
> **Owner** : SRE / platform team.
> **Related dashboards** : `06_capacity_planning`.

## 1. Alert summary

The total cardinality of `efe_*` metrics exceeded 50 000
series for 15 min. Risk : Prom memory pressure → OOM →
observability outage.

## 2. First 5 minutes — triage

1. Dashboard 06 — view "Top 20 high-cardinality metric
   families".
2. Identify the offending family. Most often : a metric
   acquired a new high-cardinality label recently.
3. Check Helm release history :
   ```bash
   helm history efe-acme -n tenant-acme | head -10
   ```
   Did the latest release add a label to a relabel rule ?

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| Tenant explosion | `tenant` label with > 1000 distinct values | Onboarding spike — scale Mimir |
| Free-text leaked into label | One family with millions of series | `labeldrop` rule in ServiceMonitor |
| `request_id` accidentally exposed | Unbounded growth | Drop label, route to traces |
| Per-trace metric attribute | Compounding tenant × span_id | Move to spans |

## 4. Mitigation

- If a free-text label leaked, drop it immediately :
  ```bash
  kubectl edit servicemonitor efe-acme-server
  # Add a metricRelabelings entry :
  #   - source_labels: [bad_label]
  #     action: labeldrop
  ```
- If tenant count exploded — review onboarding pipeline ;
  scale Mimir / Cortex.

## 5. Resolution

- Add a regression test for the label allowlist
  (`scripts/tests/test_d13_metric_label_allowlist.py`).
- Document the label inventory in
  `docs/howto/cardinality-management.md`.

## 6. Escalation

If Prom node OOMs : page platform team. Prom OOM = 30 min
of observability outage.

## 7. Postmortem

Mandatory — cardinality bombs are operational antipatterns
that compound over time if unaddressed.
