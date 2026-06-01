-- 0011_m22_mode3_review_queue.sql — M22 Phase B + D
--
-- Per-tenant pending Mode-3 reviews. Each row represents a
-- classification that hit max_turns without converging AND was
-- routed to human review (because catalog.stakes_tier='high' AND
-- tenant_feature_mode3.human_review_required=true).
--
-- review_token : UUID v4, 122-bit entropy, used as opaque pointer
--                in mcp-server tool calls.
-- expires_at   : default 7 days post-creation. Cron job daily at
--                02:00 UTC marks `expired` rows.
-- audit_id     : FK to llm_call_audit(id) — preserves full
--                concept_stack for right-to-explanation (GDPR Art. 22).
--
-- Compliance gate (separate trigger on tenant_feature_mode3) blocks
-- enabling Mode-3 with human_review_required=false on high-stakes
-- catalogs.

CREATE TABLE IF NOT EXISTS mode3_review_queue (
    review_token        TEXT PRIMARY KEY,
    tenant_id           TEXT NOT NULL,
    audit_id            BIGINT NOT NULL REFERENCES llm_call_audit(id),
    catalog             TEXT NOT NULL,
    bundle_version      TEXT NOT NULL,
    sentence_hash       TEXT NOT NULL,
    proposed_label      TEXT NOT NULL,
    proposed_confidence REAL NOT NULL
                        CHECK (proposed_confidence BETWEEN 0 AND 1),
    concept_stack       JSONB NOT NULL,
    status              TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
    requested_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at          TIMESTAMPTZ NOT NULL,
    reviewed_at         TIMESTAMPTZ,
    reviewed_by         TEXT,
    review_notes        TEXT,
    final_label         TEXT,
    CHECK (
        (status = 'pending' AND reviewed_at IS NULL AND reviewed_by IS NULL)
        OR (status IN ('approved', 'rejected') AND reviewed_at IS NOT NULL AND reviewed_by IS NOT NULL)
        OR status = 'expired'
    )
);

CREATE INDEX IF NOT EXISTS ix_mode3_review_queue_status_expires
    ON mode3_review_queue(status, expires_at)
    WHERE status = 'pending';

CREATE INDEX IF NOT EXISTS ix_mode3_review_queue_tenant_status
    ON mode3_review_queue(tenant_id, status, requested_at DESC);

CREATE INDEX IF NOT EXISTS ix_mode3_review_queue_audit_id
    ON mode3_review_queue(audit_id);

-- Per-tenant Mode-3 feature flags + compliance attestation.
CREATE TABLE IF NOT EXISTS tenant_feature_mode3 (
    tenant_id              TEXT PRIMARY KEY,
    enabled                BOOLEAN NOT NULL DEFAULT FALSE,
    human_review_required  BOOLEAN NOT NULL DEFAULT FALSE,
    default_max_turns      INTEGER NOT NULL DEFAULT 3
                           CHECK (default_max_turns BETWEEN 1 AND 5),
    max_max_turns          INTEGER NOT NULL DEFAULT 5
                           CHECK (max_max_turns BETWEEN 1 AND 5),
    default_threshold      REAL NOT NULL DEFAULT 0.85
                           CHECK (default_threshold BETWEEN 0 AND 1),
    review_ttl_days        INTEGER NOT NULL DEFAULT 7
                           CHECK (review_ttl_days BETWEEN 1 AND 365),
    ai_act_art14_acknowledged_at TIMESTAMPTZ,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (default_max_turns <= max_max_turns)
);

-- Compliance gate trigger : prevent enabling Mode-3 on high-stakes
-- catalogs without human_review_required.
CREATE OR REPLACE FUNCTION mode3_compliance_gate()
RETURNS TRIGGER AS $$
DECLARE
    has_high_stakes BOOLEAN;
BEGIN
    IF NEW.enabled = TRUE THEN
        -- Check that AI Act Art. 14 has been acknowledged.
        IF NEW.ai_act_art14_acknowledged_at IS NULL THEN
            RAISE EXCEPTION
              'tenant_feature_mode3: enabling Mode-3 requires ai_act_art14_acknowledged_at to be set (EU AI Act Art. 14 high-risk human oversight)';
        END IF;
        -- Check that for any high-stakes Mode-3-enabled catalog the
        -- tenant uses, human_review_required is true.
        -- Table is `catalogs` (plural) per migration 0001_init.sql.
        -- Tenant column is `tenant` (NOT `tenant_id`) per 0001 schema.
        SELECT EXISTS (
            SELECT 1 FROM catalogs c
             WHERE c.tenant = NEW.tenant_id
               AND c.mode3_enabled = TRUE
               AND c.stakes_tier = 'high'
        ) INTO has_high_stakes;
        IF has_high_stakes AND NEW.human_review_required = FALSE THEN
            RAISE EXCEPTION
              'tenant_feature_mode3: high-stakes catalog requires human_review_required=true (GDPR Art. 22(3) + AI Act Art. 14)';
        END IF;
    END IF;
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS tenant_feature_mode3_gate ON tenant_feature_mode3;
CREATE TRIGGER tenant_feature_mode3_gate
    BEFORE INSERT OR UPDATE ON tenant_feature_mode3
    FOR EACH ROW EXECUTE FUNCTION mode3_compliance_gate();

COMMENT ON TABLE mode3_review_queue IS
    'M22 — Mode-3 pending human reviews. UUID v4 token, 7-day TTL.';
COMMENT ON TABLE tenant_feature_mode3 IS
    'M22 — Per-tenant Mode-3 config + AI Act Art. 14 attestation.';
COMMENT ON COLUMN tenant_feature_mode3.human_review_required IS
    'HARD GATE for high-stakes catalogs (Postgres trigger enforced).';
