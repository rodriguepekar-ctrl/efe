# EFE — Operator runbook index

This directory contains **on-call runbooks** referenced from the
`runbook:` annotation on every Prometheus alert rule shipped with
the Helm chart.

When PagerDuty pages an on-call, the alert payload includes a
runbook URL pointing to a file in this directory. **Format** :

* Symptom (what fired)
* Impact (what's broken from a user / business standpoint)
* Triage immediate (5-min checks)
* Common causes (sorted by likelihood)
* Mitigation (concrete commands)
* Escalation (when to page whom)

## Index

### Sprint D17 — Backup + Disaster Recovery

| Runbook | Alert | Severity |
|---------|-------|:--------:|
| [D17-1](D17-1.md) | `D17BackupStaleFull` / `D17BackupStaleDiff` | 🔴 critical / 🟠 high |
| [D17-2](D17-2.md) | `D17BackupFailureRate` | 🟠 high |
| [D17-3](D17-3.md) | `D17BackupRpoViolation` | 🔴 critical |
| [D17-4](D17-4.md) | `D17DrDrillStale` | 🔴 critical |
| [D17-5](D17-5.md) | `D17DrRtoViolation` | 🟠 high |
| [D17-7](D17-7.md) | `D17BackupS3Unreachable` | 🔴 critical |

(D17-6 reserved for a future cipher-passphrase-missing alert ;
no runbook needed yet.)

## Conventions

* Every runbook lists a **single primary command** to copy-paste
  during triage. On-call shouldn't need to invent commands at 3 AM.
* Escalation paths reference job titles, not specific people, so
  the runbook stays correct after team turnover.
* `kubectl --namespace efe ...` is the assumed prefix ; substitute
  for your cluster's actual namespace if different.

## Cross-reference

* [D17 architecture](../bench/d17_backup_dr_architecture.md)
* [D17 audit report](../audits/d17_backup_dr_audit_report.md)
* [D17 operator howto](../docs/howto/backup-and-restore.md)
