-- 0015_active_learning_rollback.sql
-- Rollback for M24 active learning migration. Drops all M24 tables
-- and removes column additions. Tenant-feature data lost.

BEGIN;

-- 1. Drop views first (depend on tables)
DROP VIEW IF EXISTS v_gold_examples_holdout;
DROP VIEW IF EXISTS v_gold_examples_training;

-- 2. Drop triggers + functions
DROP TRIGGER IF EXISTS trg_gold_label_queue_state_machine ON gold_label_queue;
DROP TRIGGER IF EXISTS trg_retune_run_state_machine ON bundle_retune_run;
DROP FUNCTION IF EXISTS fn_gold_label_queue_state_machine();
DROP FUNCTION IF EXISTS fn_retune_run_state_machine();

-- 3. Drop policies
DROP POLICY IF EXISTS tenant_isolation_gold_label_queue ON gold_label_queue;
DROP POLICY IF EXISTS tenant_isolation_gold_label_proposal ON gold_label_proposal;
DROP POLICY IF EXISTS tenant_isolation_gold_label_review ON gold_label_review;
DROP POLICY IF EXISTS tenant_isolation_bundle_retune_run ON bundle_retune_run;
DROP POLICY IF EXISTS tenant_isolation_bundle_holdout_split ON bundle_holdout_split;
DROP POLICY IF EXISTS tenant_isolation_bundle_rebuild_request ON bundle_rebuild_request;
DROP POLICY IF EXISTS tenant_isolation_learning_audit ON learning_audit;

-- 4. Drop tables in reverse dep order
DROP TABLE IF EXISTS learning_audit CASCADE;
DROP TABLE IF EXISTS bundle_rebuild_request CASCADE;
DROP TABLE IF EXISTS bundle_holdout_split CASCADE;
DROP TABLE IF EXISTS bundle_retune_run CASCADE;
DROP TABLE IF EXISTS gold_label_review CASCADE;
DROP TABLE IF EXISTS gold_label_proposal CASCADE;
DROP TABLE IF EXISTS gold_label_queue CASCADE;

-- 5. Drop ENUM types
DROP TYPE IF EXISTS gold_label_queue_status;
DROP TYPE IF EXISTS gold_label_review_decision;
DROP TYPE IF EXISTS retune_run_status;
DROP TYPE IF EXISTS stakes_tier;

-- 6. Drop tenant_feature M24 columns
ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS m24_active_learning_enabled,
    DROP COLUMN IF EXISTS m24_auto_promotion_enabled,
    DROP COLUMN IF EXISTS m24_human_review_required,
    DROP COLUMN IF EXISTS m24_dpo_consent_signed_at,
    DROP COLUMN IF EXISTS m24_dpo_consent_expires_at,
    DROP COLUMN IF EXISTS m24_uncertainty_threshold,
    DROP COLUMN IF EXISTS m24_auto_confirm_threshold,
    DROP COLUMN IF EXISTS m24_queue_max_size,
    DROP COLUMN IF EXISTS m24_gold_label_threshold,
    DROP COLUMN IF EXISTS m24_retune_interval_days,
    DROP COLUMN IF EXISTS m24_anchor_weight_alpha,
    DROP COLUMN IF EXISTS m24_max_centroid_drift,
    DROP COLUMN IF EXISTS m24_promotion_threshold_pp,
    DROP COLUMN IF EXISTS m24_active_learning_budget_cents_per_day,
    DROP COLUMN IF EXISTS m24_review_sla_hours,
    DROP COLUMN IF EXISTS m24_notify_after_hours;

-- 7. Drop catalog M24 columns
ALTER TABLE catalog
    DROP COLUMN IF EXISTS m24_al_enabled,
    DROP COLUMN IF EXISTS m24_stakes_tier,
    DROP COLUMN IF EXISTS m24_drift_threshold_pp;

-- 8. Mark migration as rolled back
UPDATE schema_migration
SET description = description || ' [ROLLED BACK ' || NOW()::TEXT || ']'
WHERE id = '0015_active_learning';

COMMIT;
