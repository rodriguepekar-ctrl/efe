-- M27 Edge — rollback. Not fully reversible (the `QuotaExceeded` enum
-- value cannot be dropped from `agent_audit_event` without recreating
-- the enum and rewriting every dependent column). We accept leaving
-- the value behind ; the rest of the schema reverts cleanly.

BEGIN;

DROP VIEW IF EXISTS v_tenant_monthly_usage;
DROP FUNCTION IF EXISTS fn_cleanup_expired_revoked_jwt();

DROP TABLE IF EXISTS tenant_ip_allowlist;
DROP TABLE IF EXISTS revoked_jwt;
DROP TABLE IF EXISTS tenant_daily_usage;

ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS tier,
    DROP COLUMN IF EXISTS pii_strict,
    DROP COLUMN IF EXISTS llm_tokens_quota_per_day,
    DROP COLUMN IF EXISTS search_quota_per_min,
    DROP COLUMN IF EXISTS embed_quota_per_min,
    DROP COLUMN IF EXISTS agent_run_quota_per_hour,
    DROP COLUMN IF EXISTS classify_quota_per_min;

-- Note : agent_audit_event enum value 'QuotaExceeded' is intentionally
-- NOT removed. PostgreSQL doesn't support DROP VALUE on enums without
-- a full enum recreation, which would cascade-rewrite every dependent
-- audit row. Acceptable trade-off for a rollback path.

COMMIT;
