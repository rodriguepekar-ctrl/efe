# Runbook : d22_mtls_cert_renewal_failed

> **Severity** : P1 (cert nearing expiry)
> **ETA to resolution** : 2 hours
> **Sprint origin** : M12 D22 (mTLS via cert-manager)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead (backup : Security lead)

---

## 1. Symptom

- Alert : Certificate not Ready > 1h
- Alert : `cert-manager_certificate_expiration_timestamp_seconds < (now() + 7d)`
- helm test `efe-test-mtls-handshake` fails

## 2. Why this matters

- TLS handshake fails → all efe-server connections refused
- Cascade : full outage if cert expires
- Compliance : SOC 2 CC6.7 + ISO A.8.20 + GDPR Art. 32(1)(a)

## 3. Diagnostics

```bash
# Certificate state
kubectl get certificate -n efe-production efe-mtls -o yaml | yq '.status'

# Renewal time
kubectl get certificate efe-mtls -o yaml | yq '.status.renewalTime'

# cert-manager logs
kubectl describe certificate efe-mtls -n efe-production | tail -20

# Issuer state
kubectl get clusterissuer <issuerName> -o yaml | yq '.status'

# Certificate validity
kubectl get secret efe-mtls-tls -o yaml | yq '.data."tls.crt"' | \
    base64 -d | openssl x509 -noout -dates -subject -issuer
```

## 4. Mitigation

### Quick (< 15 min)
1. Force renewal :
   ```bash
   kubectl annotate certificate efe-mtls -n efe-production \
       cert-manager.io/issue-temporary-certificate="true" \
       --overwrite
   ```
2. Switch to fallback Issuer if primary broken :
   ```bash
   helm upgrade efe charts/efe \
       --set mtls.issuerRef.name=fallback-ca-issuer --reuse-values
   ```

### Longer (15-120 min)
3. Manual cert provisioning via cmctl :
   ```bash
   cmctl create cert efe-mtls-temp --duration 720h
   ```
4. Disable mTLS temporarily (LAST RESORT, customer notification) :
   ```bash
   helm upgrade efe charts/efe \
       --set mtls.enabled=false --reuse-values
   ```

## 5. Resolution criteria

- Certificate `Ready=True`
- Cert expiration > 60 days
- helm test `efe-test-mtls-handshake` passes
- 0 customer reports for 1h

## 6. Post-incident actions

- [ ] Add `D22ObsCertExpiringSoon` Prometheus alert (if not present)
- [ ] If issuer was root cause : update HANDOFF_OPS sub-processor list
- [ ] Customer notification if mTLS was disabled

## 7. Related runbooks

- `d15_oidc_jwks_stale.md`
- `m13_signing_key_rotation.md`
