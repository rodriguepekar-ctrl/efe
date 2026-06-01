# Runbook : d27_bundle_reload_hook_failed

> **Severity** : P1 (bundle drift between values + running)
> **ETA to resolution** : Immediate post-upgrade
> **Sprint origin** : M12 D27 (Helm post-upgrade hook)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead (backup : ML lead)

---

## 1. Symptom

- Helm hook Job exit non-zero post-`helm upgrade`
- Logs : `❌ Reload failed after 3 attempts`
- Bundle hash drift between `values.yaml` and running pods

## 2. Why this matters

- Pods running stale bundle → wrong embeddings
- Customer-visible classification drift
- Compliance : AI Act §6 (predetermined changes — must work)

## 3. Diagnostics

```bash
# Check hook Job
kubectl get jobs -n efe-production -l efe.io/feature=bundle-reload --sort-by=.status.startTime
kubectl describe job <reload-job> -n efe-production
kubectl logs <reload-pod> -n efe-production

# Check current bundle hash on running pods
kubectl exec -n efe-production efe-server-0 -- \
    curl -fsSL http://localhost:8080/admin/bundle-info | jq

# Check expected hash from values
helm get values efe -n efe-production -o yaml | grep manifestSha256
```

## 4. Mitigation

### Quick (< 5 min)
1. Re-run hook manually :
   ```bash
   kubectl create job --from=cronjob/efe-bundle-reload \
       efe-bundle-reload-manual-$(date +%s) -n efe-production
   ```

### If admin endpoint unreachable
2. Verify NetworkPolicy egress allows admin port 8080 (M12-A8 fix)
3. Check cert + RBAC permissions

### Longer
4. If pattern recurs : rolling restart pods (safer than hot reload) :
   ```bash
   kubectl rollout restart deployment efe-server -n efe-production
   ```

## 5. Resolution criteria

- Bundle hash on all pods matches `values.bundle.manifestSha256`
- 0 reload Job failures for 1h
- Admin RPC `/admin/bundle-info` returns expected hash

## 6. Post-incident actions

- [ ] Review M12 D27 audit fix history
- [ ] Add anti-regression pin test
- [ ] Customer notification if drift caused issues

## 7. Related runbooks

- `m12_helm_upgrade_rollback.md`
- `d16_bundle_signature_invalid.md`
