-- 0017_agent_run_rename_template_id.sql
-- M26 — rename `agent_run.template_id` → `template_slug` for semantic accuracy.
-- The column has always stored a slug (text) ; the misleading name dates back
-- to M25 where the foreign-key intent was deferred. We provide a SELECT-only
-- backward-compatibility view `agent_run_v25` for one release window.

BEGIN;

-- 1. Rename the column.
ALTER TABLE agent_run RENAME COLUMN template_id TO template_slug;

-- 2. Rename the matching column on agent_audit if any (none, but defensive).

-- 3. Recreate indexes that referenced the old name.
DROP INDEX IF EXISTS agent_run_active_idx;
CREATE INDEX agent_run_active_idx
    ON agent_run(tenant_id, status, created_at)
    WHERE status NOT IN ('COMPLETED', 'FAILED', 'TERMINATED', 'TIMED_OUT', 'COST_CAPPED');

-- 4. Backward-compat view (read-only). DO NOT INSERT into this view —
--    M25 callers must update to use `template_slug` directly within the
--    M26 → M27 migration window.
CREATE OR REPLACE VIEW agent_run_v25 AS
SELECT
    id,
    tenant_id,
    template_slug AS template_id,    -- alias for M25 callers
    profile,
    provider,
    model,
    status,
    created_by,
    created_at,
    started_at,
    completed_at,
    timeout_seconds,
    max_cost_cents,
    current_cost_cents,
    current_step,
    max_steps,
    vm_id,
    initial_prompt_json,
    tool_whitelist_json,
    final_output_json,
    failure_reason,
    dpia_addendum_hash,
    k8s_pod_name,
    paused_at,
    resumed_at
FROM agent_run;

COMMENT ON VIEW agent_run_v25 IS
    'M25 backward-compat view aliasing template_slug → template_id. '
    'Read-only ; deprecated in M27.';

-- 5. Schema migration tracker.
INSERT INTO schema_migration (id, applied_at, description)
VALUES (
    '0017_agent_run_rename_template_id',
    NOW(),
    'M26 — rename agent_run.template_id → template_slug + back-compat view'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
