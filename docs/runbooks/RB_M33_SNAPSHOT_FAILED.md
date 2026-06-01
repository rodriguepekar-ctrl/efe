# RB_M33_SNAPSHOT_FAILED — KG snapshot upload failed

**Severity** : P2 (durability at risk).
**Trigger** : `M33SnapshotFailing` (custom alert).

## 1. Diagnose
```bash
kubectl -n efe-system get jobs -l component=snapshot --sort-by=.metadata.creationTimestamp | tail
kubectl -n efe-system logs job/<last> --tail=200
```

## 2. Common causes
- Same set as RB_M32_SNAPSHOT_FAILED (S3 creds, KMS, bucket full).

## 3. Mitigation
1. Manual snapshot trigger : `scripts/m33/snapshot_runner.sh`.
2. Verify IAM / KMS rotation history.

## 4. Customer impact
- **Live KG unaffected** — index runs from RAM.
- **Pod restart risk** : if no recent snapshot, replay from PG
  mirror takes ~ 30 s for 1 M edges.
