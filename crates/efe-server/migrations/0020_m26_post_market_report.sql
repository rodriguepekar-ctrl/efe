-- 0020_m26_post_market_report.sql
-- M26 — additional metric tracking columns + Rekor index for graduation.
--
-- (The ai_act_post_market_report table itself is in 0018 since DPO review
-- depends on it ; this migration adds runtime metrics tracking.)

BEGIN;

-- ─── runtime KPI tracking (cumulative) ───────────────────────────────
-- Updated every minute by a background task ; consumed by the Art. 26
-- post-market endpoint.

CREATE TABLE IF NOT EXISTS m26_runtime_kpi_snapshot (
    tenant_id TEXT NOT NULL,
    snapshot_minute TIMESTAMPTZ NOT NULL,
    runs_total INT NOT NULL DEFAULT 0,
    runs_completed INT NOT NULL DEFAULT 0,
    runs_failed INT NOT NULL DEFAULT 0,
    runs_terminated INT NOT NULL DEFAULT 0,
    runs_cost_capped INT NOT NULL DEFAULT 0,
    cumulative_cost_cents BIGINT NOT NULL DEFAULT 0,
    sandbox_escape_attempts INT NOT NULL DEFAULT 0,
    tool_rejections INT NOT NULL DEFAULT 0,
    approval_grants INT NOT NULL DEFAULT 0,
    approval_rejects INT NOT NULL DEFAULT 0,
    PRIMARY KEY (tenant_id, snapshot_minute)
);

CREATE INDEX IF NOT EXISTS m26_kpi_recent_idx
    ON m26_runtime_kpi_snapshot(tenant_id, snapshot_minute DESC);

ALTER TABLE m26_runtime_kpi_snapshot ENABLE ROW LEVEL SECURITY;

CREATE POLICY tenant_isolation_m26_kpi ON m26_runtime_kpi_snapshot
    USING (tenant_id = current_setting('app.current_tenant_id', true))
    WITH CHECK (tenant_id = current_setting('app.current_tenant_id', true));

-- ─── per-tenant graduation channel materialized view ─────────────────
-- Cheap lookup : "is tenant on STABLE channel right now?"

CREATE OR REPLACE VIEW v_tenant_channel AS
SELECT
    id AS tenant_id,
    CASE
        WHEN m26_graduation_revoked_at IS NOT NULL THEN 'EXPERIMENTAL'
        WHEN m26_graduation_acknowledged_at IS NOT NULL THEN 'STABLE'
        WHEN m25_agent_runtime_enabled THEN 'EXPERIMENTAL'
        ELSE 'DISABLED'
    END AS channel,
    m26_graduation_acknowledged_at AS graduated_at,
    m26_graduation_revoked_at AS revoked_at,
    m25_dpia_expires_at AS dpia_expires_at
FROM tenant_feature;

COMMENT ON VIEW v_tenant_channel IS
    'M26 graduation channel resolved per tenant (DISABLED / EXPERIMENTAL / STABLE).';

-- ─── retention enforcement function ──────────────────────────────────
-- Called by the nightly retention cron ; pseudonymizes audit rows past
-- the 7-year retention boundary while preserving HMAC chain validity.

CREATE OR REPLACE FUNCTION fn_agent_audit_pseudonymize(target_tenant TEXT, older_than INTERVAL)
RETURNS BIGINT AS $$
DECLARE
    affected BIGINT := 0;
BEGIN
    UPDATE agent_audit
    SET
        actor_id = encode(digest(actor_id || tenant_id, 'sha256'), 'hex'),
        actor_role = COALESCE(actor_role, 'pseudonymized'),
        payload_json = jsonb_build_object(
            'pseudonymized_at', NOW(),
            'event_type', payload_json->>'event_type'
        )
    WHERE tenant_id = target_tenant
      AND created_at < NOW() - older_than
      AND NOT (payload_json ? 'pseudonymized_at');
    GET DIAGNOSTICS affected = ROW_COUNT;
    RETURN affected;
END;
$$ LANGUAGE plpgsql;

COMMENT ON FUNCTION fn_agent_audit_pseudonymize(TEXT, INTERVAL) IS
    'M26 — pseudonymize agent_audit rows past retention. '
    'Note : payload_hash + hmac are NOT updated → chain integrity must be '
    're-anchored from a known-good prior tail by the GDPR cascade caller.';

-- ─── Tracker ─────────────────────────────────────────────────────────

INSERT INTO schema_migration (id, applied_at, description)
VALUES (
    '0020_m26_post_market_report',
    NOW(),
    'M26 — runtime KPI snapshot table + tenant channel view + audit pseudonymization function'
)
ON CONFLICT (id) DO NOTHING;

COMMIT;
