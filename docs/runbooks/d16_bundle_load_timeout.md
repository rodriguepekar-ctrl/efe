# Runbook : d16_bundle_load_timeout

> **Severity** : P1 (degraded — pods slow to start)
> **ETA to resolution** : 5 min
> **Sprint origin** : D16
> **Last updated** : 2026-04-30
> **Owner** : ML lead (backup : DevOps lead)

---

## 1. Symptom

- Alert : `D16ObsBundleLoadDuration > 60s`
- Pods slow to become Ready
- Cold-start spikes during scale-up

## 2. Why this matters

- Cold-start latency violates SLO M11-RPC-3
- Customer-visible during HPA scale-up bursts

## 3. Diagnostics

```bash
# Check bundle size
kubectl exec -n efe-production efe-server-0 -- \
    du -sh /etc/efe/bundles/active/

# Check disk I/O on PVC
kubectl exec -n efe-production efe-server-0 -- \
    iostat -x 1 5

# Check ONNX runtime init time
kubectl logs -n efe-production efe-server-0 | grep "BUNDLE_LOAD"
```

## 4. Mitigation

### Quick (< 5 min)
1. If bundle ≥ 500 MiB : switch to ReadOnlyMany PVC sharing
2. Increase `loadTimeoutS` temporarily
3. Pre-warm bundle pull init container

### Longer
4. Optimize bundle size (better quantization)
5. Use FP16 vs FP32 for large models

## 5. Resolution criteria

- `BundleLoadDuration` P95 < 30s
- Cold-start P95 < 5s

## 6. Post-incident actions

- [ ] Update bundle release notes if optimization needed
- [ ] Tune `loadTimeoutS` per bundle size

## 7. Related runbooks

- `d18_pod_unable_to_acquire_lease.md`
- `d26_bundle_pull_oci_failed.md`
