# Runbook : m12_helm_upgrade_rollback

> **Severity** : Manual trigger
> **ETA to resolution** : 5 min
> **Sprint origin** : M12 cross-cutting
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

When to use :
- Recent `helm upgrade` caused regression
- Customer reports degraded service post-upgrade
- Pin tests / Polaris reveal issue post-deploy

## 2. Why this matters

Fast rollback prevents prolonged customer impact.

## 3. Diagnostics

```bash
# List recent revisions
helm history efe -n efe-production --max=10

# Show current values
helm get values efe -n efe-production

# Show current manifest
helm get manifest efe -n efe-production
```

## 4. Mitigation

### Quick rollback (< 5 min)
```bash
# Identify target revision (the one BEFORE the bad upgrade)
helm history efe -n efe-production

# Roll back
helm rollback efe <REV> -n efe-production --atomic --timeout 5m

# Verify rollback succeeded
helm test efe -n efe-production --timeout 5m
```

### If rollback also fails
1. Check K8s events : `kubectl get events -n efe-production --sort-by='.lastTimestamp'`
2. Force pod restart if stuck :
   ```bash
   kubectl rollout restart deployment efe-server -n efe-production
   ```
3. Worst case : `kubectl scale deployment efe-server --replicas=0` then back to 3

## 5. Resolution criteria

- Service back to pre-upgrade state
- helm test passes
- No customer reports for 15 min
- SLO P99 back below 200 ms

## 6. Post-incident actions

- [ ] Open issue with root cause of failed upgrade
- [ ] Add anti-regression pin test
- [ ] Don't re-upgrade until root cause fixed

## 7. Related runbooks

- `d27_bundle_reload_hook_failed.md`
- `general_incident_playbook.md`
