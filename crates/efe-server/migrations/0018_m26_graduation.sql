-- 0018_m26_graduation.sql
-- M26 — tenant graduation workflow (EXPERIMENTAL → STABLE).

BEGIN;

-- ─── tenant_feature M26 columns ──────────────────────────────────────

ALTER TABLE tenant_feature
    ADD COLUMN IF NOT EXISTS m26_graduation_acknowledged_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m26_graduation_revoked_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS m26_graduation_acknowledgement_hash TEXT
        CHECK (m26_graduation_acknowledgement_hash IS NULL
               OR m26_graduation_acknowledgement_hash ~ '^sha256:[a-f0-9]{64}$'),
    ADD COLUMN IF NOT EXISTS m26_graduation_revoked_reason TEXT,
    ADD COLUMN IF NOT EXISTS m26_daily_budget_cents BIGINT DEFAULT NULL,
    ADD COLUMN IF NOT EXISTS m26_monthly_budget_cents BIGINT DEFAULT NULL;

-- ─── compliance_pack_run (R-100 lineage) ─────────────────────────────
-- Records each compliance pack regen execution. M26 expects ≥ 1 OK
-- entry per tenant before graduation.

CREATE TABLE IF NOT EXISTS compliance_pack_run (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    module TEXT NOT NULL,
    quarter TEXT NOT NULL CHECK (quarter ~ '^[0-9]{4}-Q[1-4]$'),
    status TEXT NOT NULL CHECK (status IN ('OK', 'PARTIAL', 'FAILED')),
    artifact_path TEXT,
    rekor_log_index BIGINT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    notes TEXT,
    UNIQUE(tenant_id, module, quarter)
);

CREATE INDEX IF NOT EXISTS compliance_pack_tenant_idx
    ON compliance_pack_run(tenant_id, completed_at DESC);

-- ─── dpo_review (DPO quarterly review log) ───────────────────────────

CREATE TABLE IF NOT EXISTS dpo_review (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    module TEXT NOT NULL,
    quarter TEXT NOT NULL CHECK (quarter ~ '^[0-9]{4}-Q[1-4]$'),
    reviewed_by_email TEXT NOT NULL,
    reviewed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    findings TEXT,
    decision TEXT NOT NULL CHECK (decision IN ('APPROVED', 'CONDITIONAL', 'BLOCKED')),
    UNIQUE(tenant_id, module, quarter)
);

CREATE INDEX IF NOT EXISTS dpo_review_tenant_idx
    ON dpo_review(tenant_id, reviewed_at DESC);

-- ─── ai_act_post_market_report (Art. 26 quarterly export tracker) ────

CREATE TABLE IF NOT EXISTS ai_act_post_market_report (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    module TEXT NOT NULL DEFAULT 'm25_agent_runtime',
    quarter TEXT NOT NULL CHECK (quarter ~ '^[0-9]{4}-Q[1-4]$'),
    report_json_ld JSONB NOT NULL,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    submitted_by_email TEXT NOT NULL,
    countersigned_by_dpo BOOLEAN NOT NULL DEFAULT FALSE,
    countersigned_at TIMESTAMPTZ,
    UNIQUE(tenant_id, module, quarter)
);

CREATE INDEX IF NOT EXISTS ai_act_post_market_tenant_idx
    ON ai_act_post_market_report(tenant_id, submitted_at DESC);

-- ─── RLS for new tables ──────────────────────────────────────────────

ALTER TABLE compliance_pack_run ENABLE ROW LEVEL SECURITY;
ALTER TABLE dpo_review ENABLE ROW LEVEL SECURITY;
ALTER TABLE ai_act_post_market_report ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_compliance_pack_run ON compliance_pack_run
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_dpo_review ON dpo_review
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

CREATE POLICY tenant_isolation_ai_act_post_market ON ai_act_post_market_report
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

-- ─── Graduation invariant trigger ────────────────────────────────────
-- Enforce : graduation cannot be set unless DPIA is valid.

CREATE OR REPLACE FUNCTION fn_m26_graduation_guard()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.m26_graduation_acknowledged_at IS NOT NULL
       AND OLD.m26_graduation_acknowledged_at IS NULL THEN
        IF NEW.m25_dpia_signed_at IS NULL OR NEW.m25_dpia_expires_at <= NOW() THEN
            RAISE EXCEPTION 'cannot graduate tenant : DPIA not valid';
        END IF;
        IF NEW.m26_graduation_acknowledgement_hash IS NULL THEN
            RAISE EXCEPTION 'cannot graduate tenant : acknowledgement_hash required';
        END IF;
    END IF;

    -- Revoke + acknowledge are mutually exclusive on transition.
    IF NEW.m26_graduation_revoked_at IS NOT NULL
       AND OLD.m26_graduation_revoked_at IS NULL
       AND NEW.m26_graduation_acknowledged_at IS NULL THEN
        RAISE EXCEPTION 'cannot revoke graduation that was never acknowledged';
    END IF;

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS trg_m26_graduation_guard ON tenant_feature;
CREATE TRIGGER trg_m26_graduation_guard
    BEFORE UPDATE ON tenant_feature
    FOR EACH ROW
    WHEN (
        OLD.m26_graduation_acknowledged_at IS DISTINCT FROM NEW.m26_graduation_acknowledged_at
        OR OLD.m26_graduation_revoked_at IS DISTINCT FROM NEW.m26_graduation_revoked_at
    )
    EXECUTE FUNCTION fn_m26_graduation_guard();

-- ─── Tracker ─────────────────────────────────────────────────────────

INSERT INTO schema_migration (id, applied_at, description)
VALUES (
    '0018_m26_graduation',
    NOW(),
    'M26 — tenant graduation columns + compliance_pack_run + dpo_review + ai_act_post_market_report'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
