# Runbook : d17_backup_failed

> **Severity** : P1 (RPO at risk)
> **ETA to resolution** : 30 min
> **Sprint origin** : D17 (backup pipeline)
> **Last updated** : 2026-04-30
> **Owner** : DevOps lead

---

## 1. Symptom

- Alert : `D17ObsBackupFailureRate > 0%`
- CronJob failed in `kubectl get jobs -n efe-backup`
- Most recent backup ≥ 24h old

## 2. Why this matters

- Recovery Point Objective (RPO) at risk
- Compliance : GDPR Art. 32(1)(c) restoration capability + SOC 2 A1.2
- 24h+ failure → escalates to compliance team

## 3. Diagnostics

```bash
# Check failed CronJob
kubectl get jobs -n efe-backup --sort-by=.status.startTime | tail -5
kubectl describe job <failed-job> -n efe-backup
kubectl logs <failed-job-pod> -n efe-backup --tail=100

# Check S3 backup bucket
aws s3 ls s3://efe-backup-prod/postgres-full/ --recursive | tail -5

# Check Vault token
kubectl exec -n efe-backup <backup-pod> -- vault token lookup
```

## 4. Mitigation

### Quick (< 5 min)
1. Re-run backup manually :
   ```bash
   kubectl create job --from=cronjob/efe-backup-postgres-full \
       efe-backup-manual-$(date +%s) -n efe-backup
   ```

### Longer (15-30 min)
2. If S3 IAM expired : rotate via IRSA / Workload Identity
3. If Vault token expired : refresh via `vault token renew`
4. If backup script bug : fix + re-run

## 5. Resolution criteria

- Latest successful backup < 24h old
- All 4 CronJob types running (full / diff / WAL / vault)
- Backup integrity check passes

## 6. Post-incident actions

- [ ] Verify backup is restorable (run DR drill if > 7 days drift)
- [ ] Update D17 architecture doc if pattern
- [ ] Customer notification if customer SLA impacted

## 7. Related runbooks

- `d17_dr_drill_rto_breach.md`
- `d40_dr_drill_quarterly_failed.md`
- `general_incident_playbook.md`
