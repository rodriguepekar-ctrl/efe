-- Rollback for 0019.

BEGIN;

DROP POLICY IF EXISTS tenant_isolation_pen_test_finding ON pen_test_finding;
DROP POLICY IF EXISTS tenant_isolation_pen_test_run ON pen_test_run;

DROP TABLE IF EXISTS pen_test_finding CASCADE;
DROP TABLE IF EXISTS pen_test_run CASCADE;

UPDATE schema_migration
SET description = description || ' [ROLLED BACK ' || NOW()::TEXT || ']'
WHERE id = '0019_m26_pen_test_run';

COMMIT;
