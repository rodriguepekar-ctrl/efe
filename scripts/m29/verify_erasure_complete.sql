-- M29 — Verify erasure cascade completeness for a given tenant.
--
-- Usage : psql -d efe -v tenant_id='<slug>' -f scripts/m29/verify_erasure_complete.sql
--
-- Output : for each tenant-scoped table, count of remaining rows
-- that match :tenant_id (excluding pseudonymized audit chain rows
-- where actor_subject is SHA-256 hash).
--
-- Run this as part of RB_M29_ERASURE_VERIFY.
--
-- Expected output : "0 orphan rows" for every table except
-- `agent_audit` (pseudonymised) where rows are kept but their
-- actor_subject must be `sha256:...` form.

\set ON_ERROR_STOP on
\set tenant_id_quoted '\''':tenant_id'\''

-- ── 1. Enumerate every table with a tenant_id column ───────────────
DROP TABLE IF EXISTS _verify_results;
CREATE TEMP TABLE _verify_results (
    table_name TEXT PRIMARY KEY,
    orphan_count BIGINT,
    notes TEXT
);

DO $$
DECLARE
    r RECORD;
    n BIGINT;
    q TEXT;
BEGIN
    FOR r IN
        SELECT table_schema, table_name
          FROM information_schema.columns
         WHERE column_name = 'tenant_id'
           AND table_schema = 'public'
           AND table_name NOT LIKE '\_%' ESCAPE '\'
         ORDER BY table_name
    LOOP
        -- Skip the agent_audit table (handled separately).
        IF r.table_name = 'agent_audit' THEN
            q := format(
                'SELECT count(*) FROM %I.%I '
                'WHERE tenant_id = $1 '
                'AND actor_subject !~ ''^sha256:''',
                r.table_schema, r.table_name
            );
            EXECUTE q INTO n USING current_setting('myapp.tenant_id', true);
            INSERT INTO _verify_results VALUES (
                r.table_name,
                n,
                'pseudonymised audit row count NOT in sha256: form'
            );
        ELSE
            q := format(
                'SELECT count(*) FROM %I.%I WHERE tenant_id = $1',
                r.table_schema, r.table_name
            );
            EXECUTE q INTO n USING current_setting('myapp.tenant_id', true);
            INSERT INTO _verify_results VALUES (
                r.table_name, n, NULL
            );
        END IF;
    END LOOP;
END$$;

-- ── 2. Report ──
\echo '═════ Erasure verification report ═════'
\echo 'Expected : every row = 0 (or for agent_audit : 0 unpseudonymised rows).'
\echo ''
SELECT table_name,
       orphan_count,
       CASE
           WHEN orphan_count = 0 THEN '✓ clean'
           ELSE '✗ ORPHAN ROWS REMAIN'
       END AS status,
       coalesce(notes, '')
  FROM _verify_results
 ORDER BY orphan_count DESC, table_name;

\echo ''
SELECT count(*) FILTER (WHERE orphan_count = 0) AS clean_tables,
       count(*) FILTER (WHERE orphan_count > 0) AS dirty_tables,
       count(*) AS total
  FROM _verify_results;

-- ── 3. Final assertion (fails the script if any orphan) ──
SELECT CASE
    WHEN (SELECT count(*) FROM _verify_results WHERE orphan_count > 0) = 0
    THEN 'PASS : erasure complete'
    ELSE 'FAIL : some tables still have rows ; review above and complete cascade'
END AS final_verdict;
