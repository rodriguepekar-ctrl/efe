# Runbook : General Secret Rotation Playbook

> **Severity** : Scheduled (quarterly / annual / ad-hoc)
> **ETA to resolution** : varies by secret type
> **Sprint origin** : M14 D42
> **Last updated** : 2026-04-30
> **Owner** : Security lead

---

## 1. Symptom

Triggered by :
- Scheduled rotation (per HANDOFF_OPS calendar)
- Suspected compromise (incident response)
- Personnel departure (offboarding)

## 2. Why this matters

- Limits blast radius of compromise
- Compliance : ISO A.5.17 + NIST SC-12

## 3. Generic procedure

### Phase 1 — Generate new secret
1. Use HSM-backed KMS where possible
2. Use crypto-strong randomness (no manual passwords)
3. Document key_id + valid_from in tracking system

### Phase 2 — Provision new secret in target system
4. Add new secret alongside old (dual-key period)
5. Verify new secret works in staging
6. Roll out to production

### Phase 3 — Cutover
7. Switch production to new secret
8. Verify all consumers can authenticate

### Phase 4 — Grace period
9. Old + new both valid for 30-90 days
10. Monitor for unexpected use of old secret

### Phase 5 — Retire old secret
11. Remove old secret from trust store
12. Crypto-shred from KMS

### Phase 6 — Audit log update
13. Log rotation event in HMAC chain
14. Update HANDOFF_OPS rotation calendar

## 4. Per-secret-type runbook references

- Bundle privkey : `m13_signing_key_rotation.md`
- TLS cert : `d22_mtls_cert_renewal_failed.md`
- HMAC keys : `d14bis_audit_chain_broken.md`
- OIDC issuer : `d15_oidc_jwks_stale.md`
- Cloud IAM : (per cloud provider docs)
- GH PAT : (GH security center)

## 5. Resolution criteria

- New secret active everywhere it should be
- Old secret retired + crypto-shred
- 0 unexpected use of old secret in monitoring
- Compliance log updated

## 6. Post-incident actions

- [ ] Verify next rotation scheduled
- [ ] Update inventory if secret added/removed

## 7. Related runbooks

- `general_incident_playbook.md`
- All sprint-specific rotation runbooks
