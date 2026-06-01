# Runbook : d40_dr_drill_quarterly_failed

> **Severity** : P2 (drill failure, compliance reporting impact)
> **ETA to resolution** : Within quarter (next drill)
> **Sprint origin** : M12 D40 (quarterly DR drill)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead + Compliance lead

---

## 1. Symptom

- Alert : `D40DrillQuarterlyFailed`
- outcome.json shows `success_overall: false` or any verifier `ok: false`
- Drill exit code != 0

## 2. Why this matters

- SOC 2 CC9.1 + ISO A.5.30 + GDPR Art. 32(1)(d) require successful quarterly drill
- 1 failure → annual report includes finding
- 2+ failures → audit could fail

## 3. Diagnostics

```bash
# Get latest outcome.json
aws s3 ls s3://efe-compliance-evidence/dr-drill-quarterly/ --recursive | sort | tail -1
aws s3 cp s3://efe-compliance-evidence/dr-drill-quarterly/<latest>/outcome.json /tmp/

# Identify failed verifier
jq '.verifiers[] | select(.ok == false)' /tmp/outcome.json

# Read drill logs
aws s3 ls s3://efe-compliance-evidence/dr-drill-quarterly/<latest>/logs/
aws s3 cp s3://efe-compliance-evidence/dr-drill-quarterly/<latest>/logs/<failed-step>.log /tmp/
```

## 4. Mitigation

1. Identify the failed step (restore / Hello-World / verifier)
2. Reproduce locally if possible
3. Apply fix to relevant code (D17 backup script / Hello-World / etc.)
4. Re-run drill manually :
   ```bash
   kubectl create job --from=cronjob/efe-dr-drill-quarterly \
       efe-dr-drill-manual-$(date +%s) -n efe-dr-drill-quarterly
   ```
5. Verify outcome before next scheduled drill

## 5. Resolution criteria

- Re-run drill : `success_overall: true` + all verifiers pass
- Compliance report updated with explanation + fix

## 6. Post-incident actions

- [ ] Document root cause + fix in compliance report
- [ ] Add anti-regression pin test
- [ ] Notify CISO + DPO for annual report
- [ ] If pattern : full DR architecture review

## 7. Related runbooks

- `d17_backup_failed.md`
- `d17_dr_drill_rto_breach.md`
- `general_incident_playbook.md`
