# Customer email — Audit export sink returning 403

**Subject** : Your EFE audit export sink is failing — credentials may
have rotated

**To** : `{customer.contact_email}`
**CC** : `{customer.security_contact}`, `dpo@{customer.domain}`
**From** : `support@efe.ai`

---

Hi `{customer.name}`,

Our audit-export worker has been receiving HTTP 403 responses when
attempting to push your audit chain to your registered sink :

| Field | Value |
|---|---|
| Sink ID | `{sink_id}` |
| Sink kind | `{sink_kind}` (`s3` / `gcs` / `azure` / `webhook`) |
| Destination | `{sink_url}` |
| First failure observed | `{first_failure_at}` |
| Consecutive failures | `{consecutive_failures}` |

The most likely cause is a credentials / IAM-policy rotation on your
side that no longer permits writes from our worker. Common cases :

### If you use AWS S3
- Confirm that the IAM principal `arn:aws:iam::123456789012:role/efe-audit-worker`
  (or the principal we provided at registration) still has
  `s3:PutObject` on `{sink_bucket}/{sink_prefix}/*`.
- Check for explicit Deny rules that may have been added.

### If you use GCS
- Verify the HMAC key delivered at registration is still active.

### If you use Azure Blob
- Verify the SAS token has not expired (we receive `403
  AuthorizationFailure`).

### If you use a webhook
- Verify your endpoint accepts our `PUT` ; we're sending
  `Content-Type: application/x-parquet`.

---

**Your audit data is not lost.** While the sink is degraded, every
event is still recorded in our durable NATS stream and the PG mirror
(`customer_event_log` retention 30 days). Once you fix the
credentials, run the following to replay the missed window :

```
curl -X POST https://api.efe.ai/v1/audit/replay \
  -H "Authorization: Bearer <your-token>" \
  -d '{"sink_id":"{sink_id}","since":"{first_failure_at}"}'
```

We'll continue to retry hourly until either you fix the destination
or 7 days elapse. After 7 days, the sink is auto-disabled and any
missed events will need to be replayed manually within the 30-day
event log retention window.

---

**Need help ?** Reply to this email or ping us at
`support@efe.ai`. For urgent compliance impact, escalate to
`dpo@efe.ai`.

Best,
EFE Customer Success
`{ticket_id}`
