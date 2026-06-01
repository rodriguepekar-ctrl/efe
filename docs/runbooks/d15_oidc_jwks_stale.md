# Runbook : d15_oidc_jwks_stale

> **Severity** : P1 (auth degraded)
> **ETA to resolution** : 15 min
> **Sprint origin** : D15 (OIDC)
> **Last updated** : 2026-04-30
> **Owner** : Security lead

---

## 1. Symptom

- Alert : `D15ObsJwksStale > 1h`
- Customer report : intermittent auth failures
- gRPC `Unauthenticated` errors elevated
- JWKS cache age exceeds `gracePeriod` window

## 2. Why this matters

- OIDC issuer unreachable → JWT validation fails-closed after grace
- Customer auth blocked when grace expires
- Cascade : new pods can't validate JWTs

## 3. Diagnostics

```bash
# Check JWKS cache age + last fetch
kubectl exec -n efe-production efe-server-0 -- /efe-admin jwks-status

# Check egress to OIDC issuer
kubectl exec -n efe-production efe-server-0 -- \
    curl -fsSL https://<issuer-url>/.well-known/jwks.json

# Check NetworkPolicy egress
kubectl get networkpolicy -n efe-production efe-strict -o yaml | \
    grep -A 5 oidc

# Check recent auth failures
kubectl logs -n efe-production -l app=efe-server --since 1h | grep "jwks"
```

## 4. Mitigation

### Quick (< 5 min)
1. Verify OIDC issuer is reachable from pods
2. Check NetworkPolicy egress allows issuer host
3. Force JWKS refresh :
   ```bash
   kubectl exec -n efe-production efe-server-0 -- /efe-admin jwks-refresh
   ```

### Longer (15-30 min)
4. Switch to backup OIDC issuer (if multi-issuer config)
5. Adjust `graceStaleS` to extend grace window temporarily
6. Contact OIDC provider support if outage on their side

## 5. Resolution criteria

- JWKS fresh (age < 5 min)
- Auth success rate > 99 %
- 0 P1 incidents related to auth for 1h

## 6. Post-incident actions

- [ ] If issuer outage repeats, evaluate multi-issuer fallback
- [ ] Update D15 SLO monitoring threshold
- [ ] Customer notification if auth was blocked

## 7. Related runbooks

- `d13_auth_failure_spike.md`
- `m13_signing_key_rotation.md`
- `general_incident_playbook.md`
