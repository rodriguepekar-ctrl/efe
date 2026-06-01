# RB_M29_AUDIT_EXPORT_FAIL — Audit export sink failing

**Severity** : P2 (degraded) | P1 (multi-tenant impact).
**Trigger** : `M29AuditExportSinkDegraded`.

## 1. Identify failing sinks
```sql
SELECT id, tenant_id, kind, url, status, last_failure_at, last_failure_reason
  FROM audit_export_sink
 WHERE status = 'degraded'
   AND last_failure_at > NOW() - INTERVAL '1 hour'
 ORDER BY last_failure_at DESC LIMIT 20;
```

## 2. Common causes

### 2.1 S3 403 — credentials rotated
- Customer rotated their bucket policy.
- Email the customer (template in `docs/templates/sink_403.md`) ask
  them to add the EFE worker IAM principal back.
- Sink remains degraded until next successful push.

### 2.2 S3 NoSuchBucket
- Customer deleted the bucket.
- Disable the sink :
  ```sql
  UPDATE audit_export_sink SET status='disabled' WHERE id = '<id>';
  ```

### 2.3 GCS / Azure auth expired
- Refresh creds via portal-admin OR ask customer to push fresh
  credentials via `PATCH /v1/audit/export-sink/:id`.

### 2.4 Webhook 5xx
- Customer's endpoint down. Circuit breaker (same logic as
  webhook delivery) opens after 10 consecutive fails.

## 3. Manual replay
After fix, replay the missed window :
```bash
curl -X POST https://api.efe.ai/v1/audit/replay \
  -H "Authorization: Bearer <admin>" \
  -d '{"sink_id":"<id>","since":"2026-05-11T00:00:00Z"}'
```

## 4. Re-mark sink active
```sql
UPDATE audit_export_sink SET status='active', last_failure_reason=NULL
 WHERE id = '<id>';
```

## 5. Post-incident
- If failure pattern is widespread (cloud-provider outage), bulk-replay.
- If a single tenant : log + customer comms.
