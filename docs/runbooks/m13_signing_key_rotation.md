# Runbook : m13_signing_key_rotation

> **Severity** : Quarterly (scheduled)
> **ETA to resolution** : 4 weeks (with grace period)
> **Sprint origin** : M13 cross-cutting
> **Last updated** : 2026-04-30
> **Owner** : Security lead

---

## 1. Symptom

Quarterly rotation cadence (or ad-hoc on suspected compromise).

## 2. Why this matters

- Limits blast radius if a key is compromised
- Compliance : ISO A.8.24 + NIST SC-12 + SOC 2 CC6.1
- Customer trust

## 3. Diagnostics

```bash
# Check current active key
jq '.keys[] | select(.active_for_signing == true)' bundles/pubkeys.json

# Check current GH Secret
gh secret list | grep EFE_BUNDLE_PRIVKEY_PEM

# Check Rekor entries
cosign verify --key bundles/pubkeys.json oci://registry.efe.ai/bundles/<slug>:<v>
```

## 4. Mitigation (rotation procedure)

### T+0 — Generate new key
1. Generate new keypair via KMS HSM :
   ```bash
   cosign generate-key-pair --kms aws-kms://...
   ```
2. Save new pubkey + privkey reference

### T+0 — Add new pubkey to trust store
3. PR adding `efe-prod-bundle-<NEW_QUARTER>` to `bundles/pubkeys.json`
   with `active_for_signing: false`, `valid_from: T+30d`
4. Merge after 2-reviewer approval

### T+30 days — Grace period
5. Trust store deployed everywhere
6. Both old + new keys valid

### T+30 days — Cutover
7. Update GH Secret `EFE_BUNDLE_PRIVKEY_PEM` to new privkey :
   ```bash
   gh secret set EFE_BUNDLE_PRIVKEY_PEM < new-privkey.pem
   ```
8. Update CI workflow `bundle-release.yml` `SIGNING_KEY_ID` env :
   ```yaml
   env:
     SIGNING_KEY_ID: efe-prod-bundle-<NEW_QUARTER>
   ```
9. Set `active_for_signing: true` on new key, `false` on old
10. Trigger test bundle release to verify

### T+90 days post-cutover — Retire old key
11. Old key still verifies (grace period)
12. After 90 days : remove from trust store

### T+120 days — Cleanup
13. Old privkey crypto-shred from KMS
14. Audit trail logged

## 5. Resolution criteria

- New key actively signing
- Old key in grace + verification still works
- 0 verification failures across all customer tenants
- Compliance log updated

## 6. Post-incident actions

- [ ] Notify customers of rotation (if material)
- [ ] Update HANDOFF_OPS rotation calendar
- [ ] Add quarterly reminder to next rotation

## 7. Related runbooks

- `d16_bundle_signature_invalid.md`
- `d22_mtls_cert_renewal_failed.md`
- `general_secret_rotation_playbook.md`
