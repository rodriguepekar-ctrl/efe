# Runbook : d15_tenant_onboarding_failed

> **Severity** : P2 (new tenant blocked)
> **ETA to resolution** : 1 hour
> **Sprint origin** : D15
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

Customer support ticket : "Cannot complete tenant onboarding."

## 2. Why this matters

New customer onboarding blocked → revenue impact + customer satisfaction risk.

## 3. Diagnostics

```bash
# Run efe-onboard preflight
kubectl exec -n efe-production efe-server-0 -- \
    efe-onboard preflight --tenant-id <new-tenant>

# Check OIDC issuer config
kubectl get secret -n efe-production oidc-issuers -o yaml | base64 -d

# Check tenant exists in catalog
kubectl exec -n efe-pg efe-pg-0 -- \
    psql -c "SELECT * FROM tenants WHERE tenant_id='<new-tenant>'"
```

## 4. Mitigation

1. Verify OIDC issuer config matches customer's provider
2. Verify tenant row created (`Register` RPC)
3. Verify NetworkPolicy allows customer's IP range
4. Test JWT manually via curl

## 5. Resolution criteria

- Customer can authenticate + perform first Encode

## 6. Post-incident actions

- [ ] Update onboarding documentation if pattern emerges
- [ ] Add tenant onboarding test to staging suite

## 7. Related runbooks

- `d15_oidc_jwks_stale.md`
- `d11_cross_tenant_data_leak.md`
