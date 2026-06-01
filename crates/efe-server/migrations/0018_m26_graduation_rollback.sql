-- Rollback for 0018.

BEGIN;

DROP TRIGGER IF EXISTS trg_m26_graduation_guard ON tenant_feature;
DROP FUNCTION IF EXISTS fn_m26_graduation_guard();

DROP POLICY IF EXISTS tenant_isolation_ai_act_post_market ON ai_act_post_market_report;
DROP POLICY IF EXISTS tenant_isolation_dpo_review ON dpo_review;
DROP POLICY IF EXISTS tenant_isolation_compliance_pack_run ON compliance_pack_run;

DROP TABLE IF EXISTS ai_act_post_market_report CASCADE;
DROP TABLE IF EXISTS dpo_review CASCADE;
DROP TABLE IF EXISTS compliance_pack_run CASCADE;

ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS m26_monthly_budget_cents,
    DROP COLUMN IF EXISTS m26_daily_budget_cents,
    DROP COLUMN IF EXISTS m26_graduation_revoked_reason,
    DROP COLUMN IF EXISTS m26_graduation_acknowledgement_hash,
    DROP COLUMN IF EXISTS m26_graduation_revoked_at,
    DROP COLUMN IF EXISTS m26_graduation_acknowledged_at;

UPDATE schema_migration
SET description = description || ' [ROLLED BACK ' || NOW()::TEXT || ']'
WHERE id = '0018_m26_graduation';

COMMIT;
