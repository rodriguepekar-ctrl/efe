# Runbook : d18_pod_unable_to_acquire_lease

> **Severity** : P1 (degraded — pod stuck in startup)
> **ETA to resolution** : 2 min
> **Sprint origin** : D18 (multi-replica + leader election)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

- Alert : `D18ObsLeaseAcquireFail`
- Pod stuck in `Pending` or `Running` but not `Ready`
- Logs : `failed to acquire lease efe-leader`

## 2. Why this matters

- Pod can't participate as leader → degraded multi-replica behavior
- Cascade : if all pods fail, leader-only operations stuck

## 3. Diagnostics

```bash
# Check Lease object
kubectl get lease -n efe-production efe-leader -o yaml

# Check current leader
kubectl describe lease -n efe-production efe-leader | grep holderIdentity

# Check pod permissions
kubectl auth can-i update leases -n efe-production --as=system:serviceaccount:efe-production:efe-server
```

## 4. Mitigation

### Quick (< 2 min)
1. Delete the lease object (forces re-acquisition) :
   ```bash
   kubectl delete lease efe-leader -n efe-production
   ```
2. Restart the affected pod :
   ```bash
   kubectl delete pod <stuck-pod> -n efe-production
   ```

### Longer
3. Verify RBAC permissions for ServiceAccount
4. Check coordination.k8s.io API accessibility

## 5. Resolution criteria

- All pods `Ready`
- Lease holder updates regularly
- 0 lease acquisition failures for 5 min

## 6. Post-incident actions

- [ ] If pattern recurs, investigate K8s API server load
- [ ] Update D18 architecture if needed

## 7. Related runbooks

- `d18_replicas_below_minimum.md`
- `general_incident_playbook.md`
