-- 0012_m22_extend_llm_call_audit.sql — M22 Phase C
--
-- Extend M21's llm_call_audit table with Mode-3 specific columns :
--
--   - concept_stack_jsonb : full reasoning trace (denormalised
--     for fast retrieval via efe_get_concept_stack tool).
--     Populated ONLY on the FINAL audit row of a Mode-3 call ;
--     intermediate per-turn rows have NULL.
--   - total_turns : number of turns in this Mode-3 call (1 for
--     legacy Mode-2 audit rows, ≥ 1 for Mode-3).
--   - converged : true if `swarm_confidence >= threshold` reached
--     within max_turns ; false if max_turns hit without
--     convergence.
--
-- All columns are nullable / defaulted to preserve compat with
-- existing M21 audit rows.

ALTER TABLE llm_call_audit
ADD COLUMN IF NOT EXISTS concept_stack_jsonb JSONB,
ADD COLUMN IF NOT EXISTS total_turns INTEGER NOT NULL DEFAULT 1
                       CHECK (total_turns >= 1 AND total_turns <= 10),
ADD COLUMN IF NOT EXISTS converged BOOLEAN NOT NULL DEFAULT TRUE;

-- GIN index on concept_stack_jsonb enables fast JSONB containment
-- queries (e.g. find all rows where concept_stack contained a
-- specific anchor in any turn). Used by DPO export procedure.
CREATE INDEX IF NOT EXISTS ix_llm_call_audit_concept_stack
    ON llm_call_audit USING GIN (concept_stack_jsonb)
    WHERE concept_stack_jsonb IS NOT NULL;

-- Mode column extends to include MODE_3.
-- Existing CHECK was: mode IN ('MODE_2', 'MODE_AUTO_ESC')
-- We need to drop and re-add to include MODE_3.
ALTER TABLE llm_call_audit DROP CONSTRAINT IF EXISTS llm_call_audit_mode_check;
ALTER TABLE llm_call_audit ADD CONSTRAINT llm_call_audit_mode_check
    CHECK (mode IN ('MODE_2', 'MODE_AUTO_ESC', 'MODE_3'));

COMMENT ON COLUMN llm_call_audit.concept_stack_jsonb IS
    'M22 — Full Mode-3 reasoning trace. Populated only on FINAL audit row of a Mode-3 call. NULL for Mode-2 rows.';
COMMENT ON COLUMN llm_call_audit.total_turns IS
    'M22 — Number of turns in this classify call. 1 = single Mode-2 ; 2-5 = Mode-3.';
COMMENT ON COLUMN llm_call_audit.converged IS
    'M22 — true if swarm conf >= threshold reached within max_turns. false = max_turns hit, may be queued for review.';

-- Add catalogs.stakes_tier + mode3_enabled columns (compliance gate
-- depends on these via mode3_compliance_gate trigger).
-- Table is `catalogs` (plural) per migration 0001_init.sql convention.
ALTER TABLE catalogs
ADD COLUMN IF NOT EXISTS stakes_tier TEXT NOT NULL DEFAULT 'low'
                       CHECK (stakes_tier IN ('low', 'medium', 'high')),
ADD COLUMN IF NOT EXISTS mode3_enabled BOOLEAN NOT NULL DEFAULT FALSE,
ADD COLUMN IF NOT EXISTS mode3_default_threshold REAL NOT NULL DEFAULT 0.85
                       CHECK (mode3_default_threshold BETWEEN 0 AND 1);

-- Tenant column is `tenant` per 0001_init.sql schema (NOT tenant_id).
CREATE INDEX IF NOT EXISTS ix_catalogs_stakes_tier
    ON catalogs(tenant, stakes_tier)
    WHERE mode3_enabled = TRUE;

COMMENT ON COLUMN catalogs.stakes_tier IS
    'M22 — Risk classification (low/medium/high). high → human_review_required=true HARD GATE.';
COMMENT ON COLUMN catalogs.mode3_enabled IS
    'M22 — Per-catalog opt-in for Mode-3 multi-turn reasoning.';
