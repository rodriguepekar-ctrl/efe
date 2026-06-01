-- Rollback for 0017.

BEGIN;

DROP VIEW IF EXISTS agent_run_v25;

DROP INDEX IF EXISTS agent_run_active_idx;
ALTER TABLE agent_run RENAME COLUMN template_slug TO template_id;
CREATE INDEX agent_run_active_idx
    ON agent_run(tenant_id, status, created_at)
    WHERE status NOT IN ('COMPLETED', 'FAILED', 'TERMINATED', 'TIMED_OUT', 'COST_CAPPED');

UPDATE schema_migration
SET description = description || ' [ROLLED BACK ' || NOW()::TEXT || ']'
WHERE id = '0017_agent_run_rename_template_id';

COMMIT;
