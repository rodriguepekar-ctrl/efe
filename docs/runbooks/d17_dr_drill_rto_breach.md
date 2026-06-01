# Runbook : d17_dr_drill_rto_breach

> **Severity** : P2 (drill failure, not customer-impacting)
> **ETA to resolution** : 1 week
> **Sprint origin** : D17 + D40
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead + Compliance lead

---

## 1. Symptom

- Alert : `D17ObsDrillRtoBreach`
- Quarterly drill outcome.json shows `within_rto: false`
- DR drill duration > target (30 min weekly / 45 min quarterly)

## 2. Why this matters

- Compliance : SOC 2 A1.3 + ISO 27001 A.5.30 + RGPD Art. 32(1)(d) require regular DR testing
- Drill RTO breach signals real RTO is also at risk
- 4 drills/year × any breach → annual report includes finding

## 3. Diagnostics

```bash
# Read latest drill outcome
aws s3 cp s3://efe-compliance-evidence/dr-drill-quarterly/<YYYY>-Q<N>/outcome.json /tmp/
jq '.duration_secs, .rto_target_secs, .within_rto, .verifiers' /tmp/outcome.json

# Identify slowest verifier
jq '.verifiers[] | select(.duration_secs > 600)' /tmp/outcome.json

# Check drill cluster events
kubectl get events -n efe-dr-drill-quarterly --sort-by='.lastTimestamp' | tail -50
```

## 4. Mitigation

### Investigate root cause
1. Restore phase slow ? → Postgres pgBackRest config tuning
2. Helm install slow ? → reduce hook timeouts
3. Hello-World slow ? → diagnose embedder load

### Apply fix
4. Update D17 architecture if needed
5. Re-run drill manually to confirm fix

## 5. Resolution criteria

- Next drill `within_rto: true`
- All verifiers pass
- Compliance report updated

## 6. Post-incident actions

- [ ] Update DR drill targets if architecture changed
- [ ] Document root cause in compliance log
- [ ] Notify compliance officer for annual report

## 7. Related runbooks

- `d17_backup_failed.md`
- `d40_dr_drill_quarterly_failed.md`
