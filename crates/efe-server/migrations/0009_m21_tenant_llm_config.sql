-- 0009_m21_tenant_llm_config.sql — M21 Phase C
--
-- Per-tenant LLM provider preferences + compliance gates. Defaults
-- are protective (no providers, no BAA, aggressive PII redaction).
-- Activation requires opt-in by tenant admin.
--
-- Compliance gate : `dpia_signed_at IS NULL OR ai_act_acknowledged_at
-- IS NULL` → tenant cannot activate Mode-2. Enforced at API level
-- in `efe_set_tenant_feature` (mcp tool).

CREATE TABLE IF NOT EXISTS tenant_llm_config (
    tenant_id              TEXT PRIMARY KEY,
    enabled                BOOLEAN NOT NULL DEFAULT FALSE,
    allowed_providers      TEXT[] NOT NULL DEFAULT '{}',
    primary_provider       TEXT,
    fallback_providers     TEXT[] NOT NULL DEFAULT '{}',
    optimize_for           TEXT NOT NULL DEFAULT 'balanced'
                           CHECK (optimize_for IN ('cost', 'quality', 'latency', 'balanced')),
    max_context_tokens     INTEGER NOT NULL DEFAULT 16384
                           CHECK (max_context_tokens > 0),
    require_baa            BOOLEAN NOT NULL DEFAULT FALSE,
    require_eu_residency   BOOLEAN NOT NULL DEFAULT FALSE,
    require_no_training    BOOLEAN NOT NULL DEFAULT TRUE,
    prompt_redaction_level TEXT NOT NULL DEFAULT 'aggressive'
                           CHECK (prompt_redaction_level IN ('none', 'mild', 'aggressive')),
    audit_full_prompts     BOOLEAN NOT NULL DEFAULT FALSE,
    dpia_signed_at         TIMESTAMPTZ,
    ai_act_acknowledged_at TIMESTAMPTZ,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Compliance gate : enforce at row-level that activation requires both
-- DPIA + AI Act acknowledgements. (Trigger because CHECK can't reference
-- multiple columns conditionally in a clean way.)
CREATE OR REPLACE FUNCTION tenant_llm_config_compliance_gate()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.enabled = TRUE THEN
        IF NEW.dpia_signed_at IS NULL THEN
            RAISE EXCEPTION 'tenant_llm_config: enabling Mode-2 requires dpia_signed_at to be set (GDPR Art. 35)';
        END IF;
        IF NEW.ai_act_acknowledged_at IS NULL THEN
            RAISE EXCEPTION 'tenant_llm_config: enabling Mode-2 requires ai_act_acknowledged_at to be set (EU AI Act 2024/1689)';
        END IF;
        IF cardinality(NEW.allowed_providers) = 0 THEN
            RAISE EXCEPTION 'tenant_llm_config: enabling Mode-2 requires non-empty allowed_providers (multi-vendor obligation)';
        END IF;
    END IF;
    NEW.updated_at = now();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS tenant_llm_config_gate ON tenant_llm_config;
CREATE TRIGGER tenant_llm_config_gate
    BEFORE INSERT OR UPDATE ON tenant_llm_config
    FOR EACH ROW EXECUTE FUNCTION tenant_llm_config_compliance_gate();

COMMENT ON TABLE tenant_llm_config IS
    'M21 — Per-tenant LLM provider config + compliance attestations.';
COMMENT ON COLUMN tenant_llm_config.dpia_signed_at IS
    'Timestamp when tenant signed the DPIA (GDPR Art. 35). Required for activation.';
COMMENT ON COLUMN tenant_llm_config.ai_act_acknowledged_at IS
    'Timestamp when tenant acknowledged the EU AI Act conformity assessment.';
