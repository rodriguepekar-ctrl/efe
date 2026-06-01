-- 0014_taxonomy_curation_rollback.sql
-- Rollback migration for M23 (taxonomy curation).
--
-- WARNING : this drops all curation data. Only run on dev/test clusters
-- or after exporting taxonomy_change_audit for compliance retention.

BEGIN;

-- 1. Drop triggers + functions
DROP TRIGGER IF EXISTS trg_taxonomy_version_immutable ON taxonomy_version;
DROP TRIGGER IF EXISTS trg_proposal_updated_at ON taxonomy_change_proposal;
DROP FUNCTION IF EXISTS fn_taxonomy_version_immutable_after_candidate();
DROP FUNCTION IF EXISTS fn_set_updated_at();

-- 2. Drop view
DROP VIEW IF EXISTS v_pending_review_queue;

-- 3. Drop policies (RLS disabled implicitly when table dropped)
DROP POLICY IF EXISTS tenant_isolation_taxonomy_version ON taxonomy_version;
DROP POLICY IF EXISTS tenant_isolation_taxonomy_change_proposal ON taxonomy_change_proposal;
DROP POLICY IF EXISTS tenant_isolation_curation_review ON curation_review;
DROP POLICY IF EXISTS tenant_isolation_taxonomy_drift_event ON taxonomy_drift_event;
DROP POLICY IF EXISTS tenant_isolation_taxonomy_change_audit ON taxonomy_change_audit;
DROP POLICY IF EXISTS tenant_isolation_gold_dataset ON gold_dataset;
DROP POLICY IF EXISTS tenant_isolation_gold_example ON gold_example;
DROP POLICY IF EXISTS tenant_isolation_bundle_audit_scorecard ON bundle_audit_scorecard;

-- 4. Drop tables in reverse dependency order
DROP TABLE IF EXISTS bundle_audit_scorecard CASCADE;
DROP TABLE IF EXISTS gold_example CASCADE;
DROP TABLE IF EXISTS gold_dataset CASCADE;
DROP TABLE IF EXISTS taxonomy_change_audit CASCADE;
DROP TABLE IF EXISTS taxonomy_drift_event CASCADE;
DROP TABLE IF EXISTS curation_review CASCADE;
-- the taxonomy_version <-> proposal cycle requires manual order :
ALTER TABLE taxonomy_version DROP CONSTRAINT IF EXISTS taxonomy_version_proposal_fk;
DROP TABLE IF EXISTS taxonomy_change_proposal CASCADE;
DROP TABLE IF EXISTS taxonomy_version CASCADE;

-- 5. Drop ENUM types
DROP TYPE IF EXISTS taxonomy_version_status;
DROP TYPE IF EXISTS proposal_status;
DROP TYPE IF EXISTS proposal_kind;
DROP TYPE IF EXISTS drift_severity;
DROP TYPE IF EXISTS curation_review_decision;

-- 6. Drop tenant_feature columns
ALTER TABLE tenant_feature
  DROP COLUMN IF EXISTS m23_curation_enabled,
  DROP COLUMN IF EXISTS m23_curation_allow_internal_llm,
  DROP COLUMN IF EXISTS m23_curation_require_4_eyes,
  DROP COLUMN IF EXISTS m23_drift_detection_enabled,
  DROP COLUMN IF EXISTS m23_drift_auto_trigger_proposal,
  DROP COLUMN IF EXISTS m23_drift_kl_threshold,
  DROP COLUMN IF EXISTS m23_drift_conf_drop_threshold,
  DROP COLUMN IF EXISTS m23_drift_support_threshold_pct,
  DROP COLUMN IF EXISTS m23_canary_traffic_percent,
  DROP COLUMN IF EXISTS m23_canary_duration_hours,
  DROP COLUMN IF EXISTS m23_canary_min_samples,
  DROP COLUMN IF EXISTS m23_proposal_retention_days,
  DROP COLUMN IF EXISTS m23_curation_budget_cents_per_day;

-- 7. Mark migration as rolled back
UPDATE schema_migration
SET description = description || ' [ROLLED BACK ' || NOW()::TEXT || ']'
WHERE id = '0014_taxonomy_curation';

COMMIT;
