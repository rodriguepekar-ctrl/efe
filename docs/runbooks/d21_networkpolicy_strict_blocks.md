# Runbook : d21_networkpolicy_strict_blocks

> **Severity** : P1 (legitimate traffic blocked)
> **ETA to resolution** : 30 min
> **Sprint origin** : M12 D21 (NetworkPolicy strict)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

- Customer reports : "Cannot reach efe-server from my namespace"
- Connection timeouts on gRPC
- `kubectl logs efe-test-networkpolicy` shows wget timeout (D21 test pod)

## 2. Why this matters

- Legitimate tenant traffic blocked → customer outage
- Compliance : SOC 2 CC6.6 + ISO A.8.20 imply correct enforcement

## 3. Diagnostics

```bash
# Inspect NetworkPolicy strict
kubectl get networkpolicy -n efe-production efe-server-strict -o yaml

# Check pod labels match tenantSelector
kubectl get pods --show-labels -A | grep <customer-namespace>

# Run connectivity test from customer pod
kubectl exec -n <customer-ns> <customer-pod> -- \
    wget --timeout=5 -qO- http://efe-server.efe-production.svc:50051/

# Test from monitoring ns (should pass)
kubectl exec -n monitoring prometheus-0 -- \
    wget --timeout=5 -qO- http://efe-server.efe-production.svc:9090/metrics
```

## 4. Mitigation

### Quick (< 5 min)
1. Verify customer pods have label `efe.io/tenant=<tenant-id>` matching `tenantSelector`
2. If misconfigured, update labels :
   ```bash
   kubectl label pod <pod> efe.io/tenant=<tenant-id> -n <customer-ns>
   ```
3. If `cidrAllowList` needed, update Helm values

### Longer
4. If customer NetworkPolicy lib needs updating, coordinate with customer
5. Disable strict NP temporarily (LAST RESORT) :
   ```bash
   helm upgrade efe charts/efe \
       --set networkPolicyStrict.enabled=false --reuse-values
   ```
   **MUST** re-enable post-fix + post-mortem.

## 5. Resolution criteria

- Customer connectivity restored
- D21 test pod still passes (denial works)
- Monitoring scrape still working

## 6. Post-incident actions

- [ ] Update D21 architecture if pattern
- [ ] Customer onboarding doc improved

## 7. Related runbooks

- `d22_mtls_cert_renewal_failed.md`
- `d11_cross_tenant_data_leak.md`
