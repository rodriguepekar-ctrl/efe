# Runbook : d16_bundle_signature_invalid

> **Severity** : P0 (boot fail — refuses to start)
> **ETA to resolution** : 30 min
> **Sprint origin** : D16 (bundle loader) + M13
> **Last updated** : 2026-04-30
> **Owner** : Security lead (backup : ML lead)

---

## 1. Symptom

- efe-server pods refuse to start
- Pod logs : `BUNDLE_SIG_INVALID — manifest.json.sig does not match trust store`
- `kubectl get pods` shows `CrashLoopBackOff`

## 2. Why this matters

- Pods cannot start → outage
- Compliance impact : SOC 2 CC6.1 + bundle integrity check by design
- Possible supply chain attack indicator

## 3. Diagnostics

```bash
# Check signature manually
oras pull oci://registry.efe.ai/bundles/<slug>:<v> -o /tmp/bundle/
python3 scripts/release/verify_pulled_bundle.py \
    --bundle-dir /tmp/bundle/ \
    --pubkey-id efe-prod-bundle-2026q1

# Check trust store
kubectl get secret -n efe-production efe-bundle-pubkeys -o yaml | base64 -d

# Check if bundle was tampered or trust store stale
sha256sum /tmp/bundle/manifest.json
# Compare with announced value in https://efe.ai/sbom/index.json
```

## 4. Mitigation

### Quick (< 5 min)
1. Verify signature locally
2. If tampered : revert to previous bundle version :
   ```bash
   helm rollback efe <prev-rev>
   ```
3. Pin to known-good bundle :
   ```bash
   helm upgrade efe charts/efe \
       --set bundle.version=<known-good> --reuse-values
   ```

### Longer (15-30 min)
4. If trust store stale (key rotation pending), update :
   ```bash
   kubectl create secret generic efe-bundle-pubkeys \
       --from-file=pubkeys.json=bundles/pubkeys.json \
       --dry-run=client -o yaml | kubectl apply -f -
   ```

### If suspected supply chain attack
5. **Stop all deployments** + **incident response per `general_breach_notification.md`**
6. Cosign verify against Rekor transparency log
7. Compare local SBOM vs https://efe.ai/sbom/<digest>.json
8. Notify security@efe.ai + start forensic preservation

## 5. Resolution criteria

- All efe-server pods `Ready`
- Signature verifies on all pods
- 0 boot failures for 30 min
- (If breach) regulator notification sent

## 6. Post-incident actions

- [ ] If supply chain attack confirmed : full breach notification
- [ ] Rotate signing keys
- [ ] Update threat model
- [ ] Customer notification

## 7. Related runbooks

- `m13_signing_key_rotation.md`
- `general_breach_notification.md`
- `d22_mtls_cert_renewal_failed.md`
