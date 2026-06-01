-- M29 — Data rights + customer compliance.
-- Adds : audit_export_sink, erasure_request, portability_export,
-- subprocessor, subprocessor_change, byok_rotation_job,
-- byok_rotation_progress + 3 columns on tenant_feature + 4 audit
-- enum values.

BEGIN;

-- ── 1. audit_export_sink ──────────────────────────────────────────
CREATE TABLE IF NOT EXISTS audit_export_sink (
    id                          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id                   TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    kind                        VARCHAR(16) NOT NULL CHECK (kind IN ('s3','gcs','azure','webhook')),
    url                         TEXT NOT NULL,
    config                      JSONB NOT NULL DEFAULT '{}'::jsonb,
    per_tenant_key_pgp_pub      TEXT,
    status                      VARCHAR(16) NOT NULL DEFAULT 'active'
                                CHECK (status IN ('active','degraded','disabled')),
    live_tap_enabled            BOOLEAN NOT NULL DEFAULT FALSE,
    last_export_at              TIMESTAMPTZ,
    last_failure_at             TIMESTAMPTZ,
    last_failure_reason         TEXT,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by                  TEXT NOT NULL
);

COMMENT ON TABLE audit_export_sink IS
    'M29 — Customer-owned destination for the tenant''s audit chain. '
    'Each row = one configured sink. Worker streams Parquet rollups.';

CREATE INDEX IF NOT EXISTS idx_audit_export_tenant
    ON audit_export_sink (tenant_id, status);

-- ── 2. erasure_request ────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS erasure_request (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id),
    state               VARCHAR(20) NOT NULL DEFAULT 'mfa_pending'
                        CHECK (state IN ('mfa_pending','grace','in_progress','done','cancelled')),
    requested_by        TEXT NOT NULL,
    requested_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    mfa_token_hash      TEXT,        -- SHA-256 of the one-time MFA token
    mfa_sent_at         TIMESTAMPTZ,
    mfa_verified_at     TIMESTAMPTZ,
    grace_until         TIMESTAMPTZ,
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    cancelled_at        TIMESTAMPTZ,
    cancelled_by        TEXT,
    final_audit_url     TEXT         -- pushed to sink at completion
);

COMMENT ON TABLE erasure_request IS
    'M29 — GDPR Art. 17 tenant erasure request. '
    'mfa_pending → grace → in_progress → done. '
    '7-day grace allows cancellation.';

CREATE INDEX IF NOT EXISTS idx_erasure_tenant_state
    ON erasure_request (tenant_id, state);
-- Only one active request per tenant.
CREATE UNIQUE INDEX IF NOT EXISTS idx_erasure_active_per_tenant
    ON erasure_request (tenant_id)
    WHERE state IN ('mfa_pending','grace','in_progress');

-- ── 3. portability_export ─────────────────────────────────────────
CREATE TABLE IF NOT EXISTS portability_export (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    status              VARCHAR(16) NOT NULL DEFAULT 'queued'
                        CHECK (status IN ('queued','building','ready','expired','failed')),
    bundle_url          TEXT,           -- presigned S3 URL once ready
    bundle_sha256       TEXT,
    manifest_pgp_sig    TEXT,
    size_bytes          BIGINT,
    since               TIMESTAMPTZ,
    until_              TIMESTAMPTZ,    -- "until" reserved word in PG
    include_topics      TEXT[],
    submitted_by        TEXT NOT NULL,
    submitted_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    expires_at          TIMESTAMPTZ,
    failure_message     TEXT
);

COMMENT ON TABLE portability_export IS
    'M29 — GDPR Art. 20 export job. Worker builds Parquet bundle, '
    'PGP-signs manifest, uploads to S3, sets bundle_url. Customer has '
    '7 days to download before expires.';

CREATE INDEX IF NOT EXISTS idx_portability_tenant_status
    ON portability_export (tenant_id, status, submitted_at DESC);

-- ── 4. subprocessor + subprocessor_change ────────────────────────
CREATE TABLE IF NOT EXISTS subprocessor (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name                TEXT NOT NULL,
    purpose             TEXT NOT NULL,
    region              TEXT NOT NULL,
    data_processed      TEXT NOT NULL,
    contract_url        TEXT,
    risk_score          SMALLINT NOT NULL DEFAULT 3 CHECK (risk_score BETWEEN 1 AND 5),
    added_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    removed_at          TIMESTAMPTZ,
    notes               TEXT
);

COMMENT ON TABLE subprocessor IS
    'M29 — Sub-processor list (GDPR Art. 28). Removing = setting '
    'removed_at, not DELETE, so history is preserved.';

CREATE INDEX IF NOT EXISTS idx_subprocessor_active
    ON subprocessor (added_at) WHERE removed_at IS NULL;

CREATE TABLE IF NOT EXISTS subprocessor_change (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subprocessor_id     UUID REFERENCES subprocessor(id),
    change_type         VARCHAR(16) NOT NULL CHECK (change_type IN ('added','removed','changed')),
    diff                JSONB NOT NULL DEFAULT '{}'::jsonb,
    announced_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    effective_at        TIMESTAMPTZ NOT NULL,
    ack_required_by     TIMESTAMPTZ NOT NULL,
    notified_at         TIMESTAMPTZ
);

COMMENT ON TABLE subprocessor_change IS
    'M29 — Notification record for every sub-processor addition/removal. '
    'Default notification window = 30 days (effective_at = announced_at + 30d).';

CREATE INDEX IF NOT EXISTS idx_subproc_change_time
    ON subprocessor_change (announced_at DESC);

-- Per-tenant acknowledgement (one row per tenant × change).
CREATE TABLE IF NOT EXISTS subprocessor_ack (
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    change_id           UUID NOT NULL REFERENCES subprocessor_change(id) ON DELETE CASCADE,
    acknowledged_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    acknowledged_by     TEXT NOT NULL,
    PRIMARY KEY (tenant_id, change_id)
);

-- ── 5. byok_rotation_job ──────────────────────────────────────────
CREATE TABLE IF NOT EXISTS byok_rotation_job (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id           TEXT NOT NULL REFERENCES tenant_feature(id) ON DELETE CASCADE,
    state               VARCHAR(20) NOT NULL DEFAULT 'pending'
                        CHECK (state IN (
                            'pending','preparing','reencrypting','committing',
                            'done','cancelled','failed'
                        )),
    old_key_fpr         TEXT NOT NULL,
    new_key_fpr         TEXT,
    new_key_pem         TEXT,           -- transient ; cleared post-commit
    rows_reencrypted    BIGINT NOT NULL DEFAULT 0,
    triggered_by        TEXT NOT NULL,
    triggered_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at          TIMESTAMPTZ,
    committed_at        TIMESTAMPTZ,
    completed_at        TIMESTAMPTZ,
    failure_message     TEXT,
    reason              TEXT
);

COMMENT ON TABLE byok_rotation_job IS
    'M29 — BYOK rotation job. pending → preparing → reencrypting → '
    'committing → done. Only one active rotation per tenant '
    '(enforced by partial unique index below + PG advisory lock).';

CREATE UNIQUE INDEX IF NOT EXISTS idx_byok_active_per_tenant
    ON byok_rotation_job (tenant_id)
    WHERE state IN ('pending','preparing','reencrypting','committing');

CREATE INDEX IF NOT EXISTS idx_byok_history
    ON byok_rotation_job (tenant_id, triggered_at DESC);

-- ── 6. byok_rotation_progress (per-table checkpoint) ─────────────
CREATE TABLE IF NOT EXISTS byok_rotation_progress (
    rotation_id         UUID NOT NULL REFERENCES byok_rotation_job(id) ON DELETE CASCADE,
    table_name          VARCHAR(64) NOT NULL,
    last_row_id         UUID,
    rows_done           BIGINT NOT NULL DEFAULT 0,
    checkpointed_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (rotation_id, table_name)
);

COMMENT ON TABLE byok_rotation_progress IS
    'M29 — Per-table checkpoint for resumable BYOK rotation. The '
    'rotation worker records progress every 1000 rows ; on restart, '
    'resumes from last_row_id.';

-- ── 7. tenant_feature additions ───────────────────────────────────
ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS pii_policy VARCHAR(16) NOT NULL DEFAULT 'off'
        CHECK (pii_policy IN ('off','detect','redact','strict')),
    ADD COLUMN IF NOT EXISTS requires_dpo_sign_for_deletion BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS deletion_state VARCHAR(20) NOT NULL DEFAULT 'active'
        CHECK (deletion_state IN ('active','pending_deletion','grace','in_progress','deleted'));

COMMENT ON COLUMN tenant_feature.pii_policy IS
    'M29 — Off|Detect|Redact|Strict. Drives the efe-pii pre-pass on classify.';
COMMENT ON COLUMN tenant_feature.requires_dpo_sign_for_deletion IS
    'M29 — When true, erasure needs a second confirmation from the DPO.';
COMMENT ON COLUMN tenant_feature.deletion_state IS
    'M29 — Mirrors the active erasure_request state for fast filtering. '
    'Source of truth = erasure_request row.';

-- ── 8. audit enum extensions ─────────────────────────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'AuditExportConfigured') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'AuditExportConfigured';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'TenantDeletionRequested') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'TenantDeletionRequested';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'TenantDeleted') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'TenantDeleted';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
                   WHERE t.typname = 'agent_audit_event' AND e.enumlabel = 'ByokRotated') THEN
        ALTER TYPE agent_audit_event ADD VALUE 'ByokRotated';
    END IF;
END$$;

-- ── 9. View : active sub-processors (used by trust center page) ──
CREATE OR REPLACE VIEW v_subprocessor_public AS
SELECT id, name, purpose, region, data_processed, contract_url, risk_score, added_at
  FROM subprocessor
 WHERE removed_at IS NULL
 ORDER BY name;

COMMENT ON VIEW v_subprocessor_public IS
    'M29 — Read-only view of active sub-processors. Surface for '
    'GET /v1/subprocessors/list (no auth) and trust center page.';

COMMIT;
