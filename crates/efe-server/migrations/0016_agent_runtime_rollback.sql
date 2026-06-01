-- 0016_agent_runtime_rollback.sql
-- Rollback for M25 agent runtime migration.

BEGIN;

-- 1. Triggers + functions
DROP TRIGGER IF EXISTS trg_agent_run_state_machine ON agent_run;
DROP TRIGGER IF EXISTS trg_agent_approval_state_machine ON agent_approval_request;
DROP FUNCTION IF EXISTS fn_agent_run_state_machine();
DROP FUNCTION IF EXISTS fn_agent_approval_state_machine();

-- 2. Policies
DROP POLICY IF EXISTS tenant_isolation_agent_run_template ON agent_run_template;
DROP POLICY IF EXISTS tenant_isolation_agent_run ON agent_run;
DROP POLICY IF EXISTS tenant_isolation_agent_step ON agent_step;
DROP POLICY IF EXISTS tenant_isolation_agent_approval ON agent_approval_request;
DROP POLICY IF EXISTS tenant_isolation_agent_state ON agent_state;
DROP POLICY IF EXISTS tenant_isolation_agent_memory ON agent_memory;
DROP POLICY IF EXISTS tenant_isolation_agent_audit ON agent_audit;

-- 3. Tables (reverse dep order)
DROP TABLE IF EXISTS agent_audit CASCADE;
DROP TABLE IF EXISTS agent_memory CASCADE;
DROP TABLE IF EXISTS agent_state CASCADE;
DROP TABLE IF EXISTS agent_approval_request CASCADE;
DROP TABLE IF EXISTS agent_step CASCADE;
DROP TABLE IF EXISTS agent_run CASCADE;
DROP TABLE IF EXISTS agent_run_template CASCADE;

-- 4. ENUM types
DROP TYPE IF EXISTS agent_run_status;
DROP TYPE IF EXISTS agent_approval_status;

-- 5. tenant_feature columns
ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS m25_agent_runtime_enabled,
    DROP COLUMN IF EXISTS m25_dpia_signed_at,
    DROP COLUMN IF EXISTS m25_dpia_expires_at,
    DROP COLUMN IF EXISTS m25_dpia_addendum_hash,
    DROP COLUMN IF EXISTS m25_cost_cap_per_run_usd,
    DROP COLUMN IF EXISTS m25_timeout_minutes_max,
    DROP COLUMN IF EXISTS m25_allowed_profiles,
    DROP COLUMN IF EXISTS m25_allowed_providers,
    DROP COLUMN IF EXISTS m25_memory_enabled,
    DROP COLUMN IF EXISTS m25_microvm_pool_size,
    DROP COLUMN IF EXISTS m25_max_concurrent_runs;

UPDATE schema_migration
SET description = description || ' [ROLLED BACK ' || NOW()::TEXT || ']'
WHERE id = '0016_agent_runtime';

COMMIT;
