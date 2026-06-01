-- ===========================================================================
-- M30 — Rollback for 0024_m30_multimodal_region.sql.
--
-- Drops every artifact created by 0024. Does NOT remove the enum
-- values added to `agent_audit_event` because PG <13 cannot drop
-- enum values without re-creating the type ; we leave them in
-- place (forward-compatible — they're additive).
-- ===========================================================================

BEGIN;

DROP VIEW IF EXISTS v_region_active;

ALTER TABLE tenant_feature
    DROP COLUMN IF EXISTS preferred_region,
    DROP COLUMN IF EXISTS multimodal_enabled;

-- Drop the FK first (the ALTER added in 0024 § 6.bis) — otherwise
-- DROP TABLE order matters. Using IF EXISTS so the rollback is
-- idempotent.
ALTER TABLE region_failover_event
    DROP CONSTRAINT IF EXISTS fk_region_failover_event_drill_run;
DROP TABLE IF EXISTS dr_drill_run;
DROP TABLE IF EXISTS region_failover_event;
DROP TABLE IF EXISTS region_health_history;
DROP INDEX IF EXISTS idx_region_one_active;
DROP TABLE IF EXISTS region;
DROP INDEX IF EXISTS idx_mm_audit_outcome_modality;
DROP INDEX IF EXISTS idx_mm_audit_tenant_created;
DROP TABLE IF EXISTS multimodal_inference_audit;
DROP TABLE IF EXISTS tenant_modality_policy;

COMMIT;
