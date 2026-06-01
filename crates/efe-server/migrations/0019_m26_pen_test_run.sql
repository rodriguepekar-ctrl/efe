-- 0019_m26_pen_test_run.sql
-- M26 — pen test execution + findings tracking.

BEGIN;

CREATE TABLE IF NOT EXISTS pen_test_run (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT,                          -- NULL for cluster-wide tests
    run_kind TEXT NOT NULL CHECK (run_kind IN ('internal_automated', 'internal_manual', 'external')),
    suite_version TEXT NOT NULL,             -- e.g. "m26-pentest-v1.0"
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    status TEXT NOT NULL DEFAULT 'IN_PROGRESS'
        CHECK (status IN ('IN_PROGRESS', 'PASSED', 'FAILED', 'ABORTED')),
    scenarios_total INT NOT NULL DEFAULT 0,
    scenarios_passed INT NOT NULL DEFAULT 0,
    scenarios_failed INT NOT NULL DEFAULT 0,
    artifact_url TEXT,
    requested_by_email TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS pen_test_run_tenant_idx
    ON pen_test_run(tenant_id, started_at DESC);

CREATE TABLE IF NOT EXISTS pen_test_finding (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id UUID NOT NULL REFERENCES pen_test_run(id) ON DELETE CASCADE,
    scenario_id TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('P0', 'P1', 'P2', 'P3', 'INFO')),
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'OPEN'
        CHECK (status IN ('OPEN', 'CONFIRMED', 'FIXED', 'ACCEPTED_RISK', 'WONT_FIX')),
    discovered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    resolved_at TIMESTAMPTZ,
    resolution_notes TEXT,
    cve_id TEXT
);

CREATE INDEX IF NOT EXISTS pen_test_finding_run_idx
    ON pen_test_finding(run_id, severity);
CREATE INDEX IF NOT EXISTS pen_test_finding_open_idx
    ON pen_test_finding(severity, discovered_at DESC)
    WHERE status IN ('OPEN', 'CONFIRMED');

-- RLS : if tenant_id set, scope ; else cluster-wide (admin only).
ALTER TABLE pen_test_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE pen_test_finding ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_pen_test_run ON pen_test_run
    USING (
        tenant_id IS NULL
        OR tenant_id = current_setting('app.current_tenant_id', true)
    );

CREATE POLICY tenant_isolation_pen_test_finding ON pen_test_finding
    USING (
        EXISTS (SELECT 1 FROM pen_test_run pr
                WHERE pr.id = pen_test_finding.run_id
                  AND (pr.tenant_id IS NULL
                       OR pr.tenant_id = current_setting('app.current_tenant_id', true)))
    );

INSERT INTO schema_migration (id, applied_at, description)
VALUES (
    '0019_m26_pen_test_run',
    NOW(),
    'M26 — pen test run + finding tables with RLS'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
