-- M29 — rollback. enum values cannot be dropped (PG limitation).

BEGIN;
DROP VIEW IF EXISTS v_subprocessor_public;
DROP TABLE IF EXISTS subprocessor_ack;
DROP TABLE IF EXISTS subprocessor_change;
DROP TABLE IF EXISTS subprocessor;
DROP TABLE IF EXISTS byok_rotation_progress;
DROP TABLE IF EXISTS byok_rotation_job;
DROP TABLE IF EXISTS portability_export;
DROP TABLE IF EXISTS erasure_request;
DROP TABLE IF EXISTS audit_export_sink;

ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS deletion_state,
    DROP COLUMN IF EXISTS requires_dpo_sign_for_deletion,
    DROP COLUMN IF EXISTS pii_policy;

-- enum values 'AuditExportConfigured', 'TenantDeletionRequested',
-- 'TenantDeleted', 'ByokRotated' stay (PG limitation).
COMMIT;
