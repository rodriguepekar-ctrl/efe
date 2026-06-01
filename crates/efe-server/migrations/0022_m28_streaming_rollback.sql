-- M28 — rollback. enum values cannot be dropped (PG limitation).

BEGIN;

DROP FUNCTION IF EXISTS fn_cleanup_event_log_old();

DROP TABLE IF EXISTS customer_event_log;
DROP TABLE IF EXISTS tenant_s3_allowlist;
DROP TABLE IF EXISTS batch_job;
DROP TABLE IF EXISTS webhook_delivery;
DROP TABLE IF EXISTS webhook_subscription;

-- NB : agent_audit_event values ('WebhookSubscribed',
-- 'WebhookDeliveryFailed', 'BatchSubmitted', 'BatchCompleted') stay.

COMMIT;
