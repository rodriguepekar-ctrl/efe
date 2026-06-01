-- Rollback for 0020.

BEGIN;

DROP FUNCTION IF EXISTS fn_agent_audit_pseudonymize(TEXT, INTERVAL);
DROP VIEW IF EXISTS v_tenant_channel;
DROP POLICY IF EXISTS tenant_isolation_m26_kpi ON m26_runtime_kpi_snapshot;
DROP TABLE IF EXISTS m26_runtime_kpi_snapshot CASCADE;

UPDATE schema_migration
SET description = description || ' [ROLLED BACK ' || NOW()::TEXT || ']'
WHERE id = '0020_m26_post_market_report';

COMMIT;
