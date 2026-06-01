# RB_M32_SNAPSHOT_FAILED — Vector snapshot CronJob failing

**Severity** : P2 ; P1 if no successful snapshot in 24h.
**Trigger** : `M32SnapshotFailing` or `M32SnapshotMissing`.

## 1. Diagnose
```bash
kubectl -n efe-system get cronjob/efe-stack-vector-snapshot
kubectl -n efe-system get jobs -l component=snapshot --sort-by=.metadata.creationTimestamp | tail
kubectl -n efe-system logs job/<last-job> --tail=200
```

## 2. Common causes

### 2.1 S3 credentials revoked
- IAM role rotation broke the snapshotter's AssumeRole.
- Mitigation : update IRSA annotation on the service account.

### 2.2 KMS key rotated
- Bucket-level encryption uses a KMS key that was rotated/disabled.
- Mitigation : verify `kms_key_arn` in Terraform and re-apply.

### 2.3 S3 bucket full / quota
- Lifecycle rules failed to transition older versions to Glacier ⇒
  bucket size grows unbounded.
- Mitigation : force lifecycle eval, raise the AWS Service Quota.

### 2.4 Bincode encode error (M32 ships JSON, but M32.1 swap)
- An invalid JSON `payload` could break serialization.
- Mitigation : log offending doc id, manually patch the row.

## 3. Customer impact
- **Live searches unaffected** — index runs from RAM.
- **Pod restart risk** : the latest snapshot may be > 24h old →
  RPO breach. Customers with strict SOC 2 BCDR receive notification.

## 4. Mitigation
1. Force a manual snapshot run :
   ```bash
   scripts/m32/snapshot_runner.sh
   ```
2. If it succeeds, the CronJob is healthy again ; root-cause why
   prior runs failed.

## 5. Post-incident
- File P2 incident.
- If KMS / IAM was the cause : add the rotation step to the
  runbook in `terraform/m30_multiregion/`.
