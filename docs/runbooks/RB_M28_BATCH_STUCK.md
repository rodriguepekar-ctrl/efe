# RB_M28_BATCH_STUCK — Batch job not progressing

**Severity** : P2 (job > 4h with no rows processed).
**Trigger** : `M28BatchJobStuck`.

## 1. Identify
```sql
SELECT id, tenant_id, status, rows_processed, rows_total, started_at,
       now() - started_at AS dur
  FROM batch_job
 WHERE status = 'running' AND now() - started_at > INTERVAL '4 hours'
 ORDER BY started_at DESC LIMIT 10;
```

## 2. Common causes

### 2.1 Worker pod crashed mid-job
NATS work-queue `ack_wait = 10m` will redeliver. But if the cancel
flag is set, the worker won't restart.

```bash
kubectl -n efe-stack get pods -l app=efe-batch-worker
kubectl -n efe-stack logs <worker-pod> --tail=200
```

Restart the worker if needed.

### 2.2 S3 misconfig
Customer rotated their bucket policy mid-job. Worker logs will show
403 Forbidden on PUT.

```sql
UPDATE batch_job SET status='failed',
       error_message='S3 PUT denied during job (likely permission change)',
       completed_at = NOW()
 WHERE id = '<job_id>';
```
Notify customer.

### 2.3 LLM provider rate limit
M21 LLMRouter returning 429s repeatedly. Worker should retry but
may stall if `max_cost_usd` is binding.

## 3. Resume vs cancel

### Resume :
```bash
nats consumer info BATCH_JOBS efe-batch-worker
# Re-deliver the message manually if stuck :
nats stream get BATCH_JOBS <seq> | nats pub efe.batch.jobs.queue --file -
```

### Cancel :
```bash
curl -X DELETE https://api.efe.ai/v1/batch/<job_id> \
  -H "Authorization: Bearer <admin_token>"
```

## 4. Post-incident
- Verify the customer's webhook (if `callback_webhook_id` was set)
  received either `batch.completed` or `batch.failed`.
- Update batch worker to surface stuck-job heartbeats every 5 min so
  this alert can detect more quickly.
