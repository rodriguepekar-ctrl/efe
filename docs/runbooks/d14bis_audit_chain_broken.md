# Runbook : d14bis_audit_chain_broken

> **Severity** : P0 (audit log integrity violation)
> **ETA to resolution** : Immediate forensic preservation
> **Sprint origin** : D14-bis (HMAC tamper-evident audit)
> **Last updated** : 2026-04-30
> **Owner** : Security lead + DPO

---

## 1. Symptom

- Alert : `D14ObsAuditChainBroken`
- HMAC chain validation failure on audit log row
- Inconsistent chain hash sequence

## 2. Why this matters

- **Possible tampering of audit logs**
- Compliance : NIST AU-9 + ISO A.5.28 + SOC 2 CC4.2 violation
- May indicate insider threat or breach
- **POTENTIAL BREACH** — escalate immediately

## 3. Diagnostics

```bash
# Identify break point
kubectl exec -n efe-pg efe-pg-0 -- psql -c "
SELECT * FROM audit_events
WHERE NOT EXISTS (
    SELECT 1 FROM audit_events e2
    WHERE e2.id = audit_events.id - 1
        AND audit_events.prev_hmac = encode(hmac(e2.payload, current_hmac_key, 'sha256'), 'hex')
)
ORDER BY id LIMIT 10;
"

# Check HMAC key state
kubectl get secret efe-audit-hmac-keys -o yaml
```

## 4. Mitigation

### Immediate (forensic preservation)

1. **STOP further writes** to audit chain :
   ```bash
   kubectl scale deployment efe-server --replicas=0
   ```
2. **Snapshot state** for forensics :
   ```bash
   kubectl exec -n efe-pg efe-pg-0 -- pg_dump audit_events > /tmp/forensic-$(date +%s).sql
   ```
3. **Notify** : security@efe.ai + dpo@efe.ai + ciso@efe.ai

### Investigation
4. Determine cause :
   - HMAC key rotation issue ?
   - Database corruption ?
   - **Tampering** ? (insider / external breach)

### If tampering confirmed
5. Activate `general_breach_notification.md` procedure
6. GDPR Art. 33 notification within 72h
7. Engage forensics team

### Recovery
8. If non-tampering : repair chain by restoring from backup point
9. If tampering : full breach investigation + customer notification

## 5. Resolution criteria

- Root cause identified
- Chain restored or breach declared
- Post-incident review completed
- Customer + regulator notifications (if breach)

## 6. Post-incident actions

- [ ] Update threat model with new attack vector
- [ ] Add anti-regression detection
- [ ] HMAC key rotation
- [ ] Customer compliance report update
- [ ] Possible regulator follow-up

## 7. Related runbooks

- `general_breach_notification.md`
- `general_incident_playbook.md`
- `general_secret_rotation_playbook.md`
