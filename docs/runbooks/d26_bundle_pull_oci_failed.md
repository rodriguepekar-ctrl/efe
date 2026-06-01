# Runbook : d26_bundle_pull_oci_failed

> **Severity** : P1 (cannot pull bundle for new pods)
> **ETA to resolution** : 5 min
> **Sprint origin** : M13 D26
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

- Alert : `M13ObsBundlePullFailed`
- New pods stuck in `Init:0/1` (oras-pull init container)
- Cannot pull from `oci://registry.efe.ai/bundles/<slug>:<v>`

## 2. Why this matters

- New pod scheduling blocked
- HPA scale-up impossible
- Cold-start indefinite

## 3. Diagnostics

```bash
# Init container logs
kubectl describe pod <stuck-pod> -n efe-production
kubectl logs -c bundle-pull <stuck-pod> -n efe-production

# Test pull manually
oras pull oci://registry.efe.ai/bundles/<slug>:<v> -o /tmp/test/

# Verify OCI registry reachability
curl -fsSL https://registry.efe.ai/v2/

# Check IRSA / Workload Identity
kubectl exec -n efe-production efe-server-0 -- \
    aws sts get-caller-identity
```

## 4. Mitigation

### Quick (< 5 min)
1. Verify registry reachability + auth
2. Try alternate registry (GHCR backup) :
   ```bash
   helm upgrade efe charts/efe \
       --set bundles.ociPull.registry=ghcr.io/efe-project/bundles-private --reuse-values
   ```
3. Pre-warm bundle in PVC for faster scale-up

### Longer
4. If registry outage : fallback to K8s Secret-mounted bundle
5. Coordinate with registry provider (GitHub support / etc.)

## 5. Resolution criteria

- New pods successfully pull bundle
- HPA scale-up working
- 0 init container failures for 30 min

## 6. Post-incident actions

- [ ] Implement fallback registry permanent if outage repeats
- [ ] Customer notification if scale-up was blocked

## 7. Related runbooks

- `d16_bundle_load_timeout.md`
- `d16_bundle_signature_invalid.md`
- `general_incident_playbook.md`
