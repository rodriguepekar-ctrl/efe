# RB_PR19_MSW8 — `ManifestReloadFailureSpike`

| Field | Value |
|---|---|
| **Alert** | `ManifestReloadFailureSpike` |
| **Severity** | warning (ticket) |
| **Dashboards** | [D4 Reload](https://grafana.efe.internal/d/efe-msw-reload) |
| **PR ref** | [§ 29](../EFE_A1_PHASE2_PR19_ARCHITECTURE.md#29-alert-a8--manifestreloadfailurespike) |

## Summary

≥ 3 manifest reload failures in the last hour. Operators are repeatedly
attempting to apply an invalid manifest, OR the chart's auto-SIGHUP loop
is flapping.

## Symptom

- Slack ticket.
- **D4 P3** "Reloads errored (24h)" stat is red.
- **D4 P5** (legacy PR #12 failure-reason bar chart) shows the dominant
  reason.

## Diagnose

```bash
# What failed ?
kubectl -n efe logs -l app.kubernetes.io/component=controller --tail=300 \
  | grep -i 'reload\|manifest'

# Is the manifest CM well-formed ?
kubectl -n efe get configmap efe-meta-swarm-manifest -o yaml | yq '.data["manifest.yaml"]' \
  | yq -e '.iteration.alpha_default'  # arbitrary path — yq fails on bad YAML
```

Failure reasons (closed taxonomy from PR #12) :
- `io` — file not found / permission error on the path the controller watches.
- `validation` — manifest schema invalid (numeric out of range, missing field).
- `incompatible` — passed schema but reload-time check rejected the manifest
  (e.g. participant dimension changed without scale-out).

## Mitigate

### Roll back to the previous manifest

```bash
# Find the previous reload_gen via the controller log
kubectl -n efe logs -l app.kubernetes.io/component=controller --tail=1000 \
  | grep "manifest reloaded" | tail -5

# History snapshots (PR #14 chart hook captures these on every reload)
kubectl -n efe get cm -l app.kubernetes.io/component=controller \
  -l efe.manifest-snapshot=true \
  --sort-by=.metadata.creationTimestamp

# Restore the previous snapshot
kubectl -n efe apply -f <prev-snapshot.yaml>
```

### Fix a validation error in place

```bash
kubectl -n efe edit configmap efe-meta-swarm-manifest
# Fix the validation error
# Controller's inotify watcher picks up the change ; reload_gen increments
```

### Stop a SIGHUP flap loop

If the same external trigger keeps sending SIGHUPs (cron job, GitOps
controller), disable the trigger first :
```bash
kubectl -n flux suspend kustomization meta-swarm-manifest-sync
```

## Escalate

| Time | Action |
|---|---|
| Sustained > 30 m | Page meta-swarm dev lead |
| GitOps controller flap | Page platform-engineering lead |

## Postmortem

Required when a failed reload caused customer-visible regression (rare —
reload is transactional, but the flap could exhaust controller CPU).
