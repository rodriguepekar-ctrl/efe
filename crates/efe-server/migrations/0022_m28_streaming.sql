-- M28 — Streaming + Events + Webhooks schema.
-- Adds : webhook_subscription, webhook_delivery, batch_job,
-- tenant_s3_allowlist, customer_event_log + 4 audit enum values.
--
-- Hard dep : migration 0016 (agent_audit_event enum).

BEGIN;

-- ── 1. webhook_subscription ────────────────────────────────────────
CREATE TABLE IF NOT EXISTS webhook_subscription (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    url                 TEXT NOT NULL,
    secret_hmac_hash    TEXT NOT NULL,         -- SHA256 hex of raw secret (never store raw)
    topics              TEXT[] NOT NULL,
    active              BOOLEAN NOT NULL DEFAULT TRUE,
    verified            BOOLEAN NOT NULL DEFAULT FALSE,
    verification_nonce  TEXT,
    circuit_state       VARCHAR(16) NOT NULL DEFAULT 'closed',
    consecutive_fails   INTEGER NOT NULL DEFAULT 0,
    circuit_opens_at    TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_delivery_at    TIMESTAMPTZ,
    CONSTRAINT topics_nonempty CHECK (array_length(topics, 1) >= 1),
    CONSTRAINT circuit_state_valid CHECK (circuit_state IN ('closed','open','half_open'))
);

COMMENT ON TABLE webhook_subscription IS
    'M28 — Per-tenant webhook subscription. Delivery worker fans out NATS '
    'events to subscribed URLs with HMAC-signed payload.';

CREATE INDEX IF NOT EXISTS idx_webhook_sub_tenant_active
    ON webhook_subscription (tenant_id, active);
CREATE INDEX IF NOT EXISTS idx_webhook_sub_circuit_open
    ON webhook_subscription (circuit_state) WHERE circuit_state != 'closed';

-- ── 2. webhook_delivery (per-attempt log) ──────────────────────────
CREATE TABLE IF NOT EXISTS webhook_delivery (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subscription_id     UUID NOT NULL REFERENCES webhook_subscription(id) ON DELETE CASCADE,
    event_id            TEXT NOT NULL,
    topic               VARCHAR(64) NOT NULL,
    attempt_n           SMALLINT NOT NULL DEFAULT 1,
    attempted_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    response_status     SMALLINT,
    response_body_head  TEXT,
    latency_ms          INTEGER,
    outcome             VARCHAR(16) NOT NULL,
    next_retry_at       TIMESTAMPTZ,
    CONSTRAINT outcome_valid CHECK (outcome IN ('ok','fail','retry','drop'))
);

COMMENT ON TABLE webhook_delivery IS
    'M28 — Per-delivery-attempt audit log. Surfaces in '
    'GET /v1/webhooks/:id/deliveries for customer self-service.';

CREATE INDEX IF NOT EXISTS idx_webhook_deliv_sub_time
    ON webhook_delivery (subscription_id, attempted_at DESC);
CREATE INDEX IF NOT EXISTS idx_webhook_deliv_event
    ON webhook_delivery (event_id);
CREATE INDEX IF NOT EXISTS idx_webhook_deliv_retry
    ON webhook_delivery (next_retry_at) WHERE next_retry_at IS NOT NULL;

-- ── 3. batch_job ──────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS batch_job (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    catalog_id          TEXT NOT NULL,
    input_url           TEXT NOT NULL,
    input_format        VARCHAR(16) NOT NULL DEFAULT 'parquet',
    input_column        VARCHAR(64) NOT NULL DEFAULT 'text',
    output_url          TEXT NOT NULL,
    output_format       VARCHAR(16) NOT NULL DEFAULT 'parquet',
    callback_webhook_id UUID REFERENCES webhook_subscription(id) ON DELETE SET NULL,
    max_cost_usd        NUMERIC(12,4),
    mode                SMALLINT NOT NULL DEFAULT 1,
    status              VARCHAR(16) NOT NULL DEFAULT 'queued',
    rows_total          BIGINT,
    rows_processed      BIGINT NOT NULL DEFAULT 0,
    cost_cents          BIGINT NOT NULL DEFAULT 0,
    error_message       TEXT,
    submitted_by        TEXT NOT NULL,
    submitted_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    idempotency_key     TEXT,
    CONSTRAINT batch_status_valid CHECK (
        status IN ('queued','running','completed','failed','cancelled')
    ),
    CONSTRAINT batch_format_valid CHECK (
        input_format IN ('parquet','csv') AND output_format IN ('parquet','csv')
    ),
    CONSTRAINT batch_mode_valid CHECK (mode IN (1, 2, 3))
);

COMMENT ON TABLE batch_job IS
    'M28 — Async classify batch job. Worker pulls from NATS '
    'BATCH_JOBS workqueue, processes input Parquet/CSV from S3, '
    'writes result to S3 output_url.';

CREATE INDEX IF NOT EXISTS idx_batch_job_tenant_status
    ON batch_job (tenant_id, status, submitted_at DESC);
CREATE UNIQUE INDEX IF NOT EXISTS idx_batch_job_idempotency
    ON batch_job (tenant_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- ── 4. tenant_s3_allowlist (batch path validation) ────────────────
CREATE TABLE IF NOT EXISTS tenant_s3_allowlist (
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    bucket              VARCHAR(63) NOT NULL,
    added_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    added_by            TEXT NOT NULL,
    note                TEXT,
    PRIMARY KEY (tenant_id, bucket),
    CONSTRAINT bucket_name_valid CHECK (bucket ~ '^[a-z0-9][a-z0-9-]*[a-z0-9]$')
);

COMMENT ON TABLE tenant_s3_allowlist IS
    'M28 — Per-tenant allow-list of S3 buckets accepted by '
    'POST /v1/batch/classify input_url and output_url.';

-- ── 5. customer_event_log (mirror of NATS, 30d retention) ─────────
CREATE TABLE IF NOT EXISTS customer_event_log (
    id                  UUID PRIMARY KEY,
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    topic               VARCHAR(64) NOT NULL,
    occurred_at         TIMESTAMPTZ NOT NULL,
    schema_version      SMALLINT NOT NULL DEFAULT 1,
    trace_id            TEXT,
    data                JSONB NOT NULL,
    inserted_at         TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

COMMENT ON TABLE customer_event_log IS
    'M28 — DB mirror of recent NATS events for the customer event '
    'log API (GET /v1/events). Populated by efe-cdc-publisher + '
    'directly by handlers. 30 day rolling retention.';

CREATE INDEX IF NOT EXISTS idx_event_log_tenant_time
    ON customer_event_log (tenant_id, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_event_log_topic
    ON customer_event_log (tenant_id, topic, occurred_at DESC);

-- ── 6. Retention cleanup function ─────────────────────────────────
CREATE OR REPLACE FUNCTION fn_cleanup_event_log_old()
RETURNS INTEGER
LANGUAGE plpgsql
AS $$
DECLARE
    deleted INTEGER;
BEGIN
    DELETE FROM customer_event_log
    WHERE occurred_at < NOW() - INTERVAL '30 days';
    GET DIAGNOSTICS deleted = ROW_COUNT;
    RETURN deleted;
END;
$$;

COMMENT ON FUNCTION fn_cleanup_event_log_old() IS
    'M28 — Hourly cleanup : delete event log entries older than 30 '
    'days. Schedule via pg_cron or external job. Returns deleted row count.';

-- ── 7. New agent_audit_event enum values ──────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'WebhookSubscribed') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'WebhookSubscribed';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'WebhookDeliveryFailed') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'WebhookDeliveryFailed';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'BatchSubmitted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'BatchSubmitted';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'BatchCompleted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'BatchCompleted';
    END IF;
END$$;

-- ── 8. Logical replication scaffolding (operator-applied) ─────────
-- The following are advisory comments ; the operator runs them
-- manually as part of RB_M28_CDC_BOOTSTRAP.md, because they require
-- superuser + are non-transactional :
--
--   CREATE PUBLICATION efe_cdc FOR TABLE
--       agent_audit, classify_call_log, drift_event, bundle_version,
--       agent_run, agent_approval_request
--   WITH (publish = 'insert, update');
--
--   SELECT pg_create_logical_replication_slot('efe_cdc_publisher', 'pgoutput');

COMMIT;
